"""FFmpeg export command and filter_complex builder for Tempo projects."""

from __future__ import annotations

import re
import subprocess
import time
from dataclasses import dataclass
from pathlib import Path
from typing import TYPE_CHECKING

from PySide6.QtCore import QThread, Signal

from tempo.utils.ffmpeg import check_ffmpeg

if TYPE_CHECKING:
    from tempo.core.models import Clip, Project, TextClip, TransitionConfig


@dataclass
class ExportSettings:
    output_path: Path
    resolution: tuple[int, int]  # (width, height), e.g. (1920, 1080)
    crf: int = 23  # quality: 18=high, 23=medium, 28=low
    fps: float = 30.0


def _clip_input_args(clip: Clip) -> list[str]:
    """Return FFmpeg -ss -t -i arguments for a single clip."""
    source_duration = clip.source_out - clip.source_in
    return [
        "-ss",
        str(clip.source_in),
        "-t",
        str(source_duration),
        "-i",
        clip.source_path,
    ]


def _build_atempo_chain(speed: float) -> str:
    """Build chained atempo filters for speeds outside 0.5-2.0."""
    if speed == 1.0:
        return "atempo=1.0000"
    filters: list[str] = []
    remaining = speed
    while remaining > 2.0:
        filters.append("atempo=2.0")
        remaining /= 2.0
    while remaining < 0.5:
        filters.append("atempo=0.5")
        remaining *= 2.0
    filters.append(f"atempo={remaining:.4f}")
    return ",".join(filters)


def _apply_speed_filter(
    clip: Clip, video_label: str, audio_label: str | None
) -> tuple[str, str | None]:
    """Return (video_label, audio_label) after applying speed adjustment filters."""
    if clip.speed == 1.0:
        return video_label, audio_label
    v_out = f"{video_label}s"
    if audio_label is None:
        return v_out, None
    a_out = f"{audio_label}s"
    return v_out, a_out


def _apply_transition(
    clip_a_label: str,
    clip_b_label: str,
    transition: TransitionConfig,
    output_label: str,
    clip_a_duration: float,
) -> str:
    """Return filter_complex fragment applying transition between two clips."""
    t = transition.type
    d = transition.duration

    if t == "Cross Dissolve":
        offset = max(0.0, clip_a_duration - d)
        return (
            f"[{clip_a_label}][{clip_b_label}]"
            f"xfade=transition=dissolve:duration={d}:offset={offset}"
            f"[{output_label}]"
        )
    elif t == "Fade In":
        return f"[{clip_b_label}]fade=t=in:st=0:d={d}[{output_label}]"
    elif t == "Fade Out":
        offset = clip_a_duration - d
        return f"[{clip_a_label}]fade=t=out:st={offset}:d={d}[{output_label}]"
    elif t == "Cut to Black":
        offset = max(0.0, clip_a_duration - d / 2.0)
        return (
            f"[{clip_a_label}][{clip_b_label}]"
            f"xfade=transition=fade:duration={d}:offset={offset}"
            f"[{output_label}]"
        )
    elif t == "Cut to White":
        offset = max(0.0, clip_a_duration - d / 2.0)
        return (
            f"[{clip_a_label}][{clip_b_label}]"
            f"xfade=transition=fade:duration={d}:offset={offset}:"
            f"color=white[{output_label}]"
        )
    elif t == "Crossfade":
        return f"[{clip_a_label}][{clip_b_label}]acrossfade=d={d}[{output_label}]"
    else:
        return ""


def _apply_drawtext(
    text_clips: list[TextClip],
    video_label: str,
    output_label: str,
    fps: float,
) -> str:
    """Build chained drawtext filters for all text clips."""
    if not text_clips:
        return ""
    filters: list[str] = []
    current_label = video_label
    for i, tc in enumerate(text_clips):
        out = output_label if i == len(text_clips) - 1 else f"txt{i}"
        safe_text = tc.content.replace("'", "\\'").replace(":", "\\:")
        x_expr = f"(w-text_w)*{tc.position_x / 100.0:.4f}"
        y_expr = f"(h-text_h)*{tc.position_y / 100.0:.4f}"
        font_color = tc.font_color.replace("#", "0x")
        font_size = tc.font_size
        enable = f"between(t,{tc.timeline_start},{tc.timeline_end})"
        f = (
            f"[{current_label}]drawtext="
            f"text='{safe_text}':"
            f"fontsize={font_size}:"
            f"fontcolor={font_color}:"
            f"x={x_expr}:y={y_expr}:"
            f"enable='{enable}'"
            f"[{out}]"
        )
        filters.append(f)
        current_label = out
    return ";".join(filters)


def _collect_inputs(project: Project) -> list[str]:
    """Return de-duplicated list of source file paths in input order."""
    seen: set[str] = set()
    inputs: list[str] = []
    for track in project.timeline.tracks:
        for clip in track.clips:
            if clip.source_path not in seen:
                seen.add(clip.source_path)
                inputs.append(clip.source_path)
    return inputs


def _build_video_chain(
    track_clips: list[Clip],
    input_index_map: dict[str, int],
    resolution: tuple[int, int],
    fps: float,
    chain_label: str,
) -> tuple[list[str], str]:
    """Build filter_complex fragments for one video track."""
    if not track_clips:
        return [], f"{chain_label}_empty"

    w, h = resolution
    fragments: list[str] = []

    for i, clip in enumerate(track_clips):
        idx = input_index_map.get(clip.source_path, 0)
        v_tag = f"v_{chain_label}_{i}"
        speed_pts = f"/({clip.speed})" if clip.speed != 1.0 else ""
        frag = (
            f"[{idx}:v]trim=start={clip.source_in}:end={clip.source_out},"
            f"setpts=PTS-STARTPTS{speed_pts},"
            f"scale={w}:{h}:force_original_aspect_ratio=decrease,"
            f"pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,setsar=1,fps={fps}[{v_tag}]"
        )
        fragments.append(frag)

    has_transitions = any(
        (c.transition_in and c.transition_in.type != "Cut")
        or (c.transition_out and c.transition_out.type != "Cut")
        for c in track_clips
    )

    if not has_transitions or len(track_clips) == 1:
        n = len(track_clips)
        v_inputs = "".join(f"[v_{chain_label}_{i}]" for i in range(n))
        concat_label = f"{chain_label}_vconcat"
        concat_frag = f"{v_inputs}concat=n={n}:v=1:a=0[{concat_label}]"
        fragments.append(concat_frag)
        return fragments, concat_label

    # Build transition chain using xfade
    curr_label = f"v_{chain_label}_0"
    for i in range(1, len(track_clips)):
        next_label = f"v_{chain_label}_{i}"
        out_label = f"{chain_label}_x{i}"
        prev_clip = track_clips[i - 1]
        next_clip = track_clips[i]
        trans = prev_clip.transition_out or next_clip.transition_in
        if trans and trans.type != "Cut":
            dur = prev_clip.timeline_end - prev_clip.timeline_start
            xfade_frag = _apply_transition(curr_label, next_label, trans, out_label, dur)
            if xfade_frag:
                fragments.append(xfade_frag)
                curr_label = out_label
            else:
                curr_label = next_label
        else:
            curr_label = next_label

    return fragments, curr_label


def _build_audio_chain(
    track_clips: list[Clip],
    input_index_map: dict[str, int],
    chain_label: str,
) -> tuple[list[str], str | None]:
    """Build filter_complex fragments for audio streams."""
    if not track_clips:
        return [], None

    fragments: list[str] = []
    for i, clip in enumerate(track_clips):
        idx = input_index_map.get(clip.source_path, 0)
        a_tag = f"a_{chain_label}_{i}"
        atempo = f",{_build_atempo_chain(clip.speed)}" if clip.speed != 1.0 else ""
        frag = (
            f"[{idx}:a]atrim=start={clip.source_in}:end={clip.source_out},"
            f"asetpts=PTS-STARTPTS{atempo}[{a_tag}]"
        )
        fragments.append(frag)

    n = len(track_clips)
    a_inputs = "".join(f"[a_{chain_label}_{i}]" for i in range(n))
    concat_label = f"{chain_label}_aconcat"
    concat_frag = f"{a_inputs}concat=n={n}:v=0:a=1[{concat_label}]"
    fragments.append(concat_frag)
    return fragments, concat_label


def _build_text_overlay_chain(
    text_clips: list[TextClip],
    video_input_label: str,
    output_label: str,
    fps: float,
) -> list[str]:
    """Wrap _apply_drawtext() — returns filter fragments or empty list."""
    if not text_clips:
        return []
    text_filter = _apply_drawtext(text_clips, video_input_label, output_label, fps)
    return [text_filter] if text_filter else []


def validate_project_for_export(project: Project) -> list[str]:
    """Check project is exportable. Returns list of warning strings (empty = OK)."""
    warnings: list[str] = []

    ffmpeg_ok, ffprobe_ok = check_ffmpeg()
    if not (ffmpeg_ok and ffprobe_ok):
        warnings.append("FFmpeg and ffprobe are not available in PATH.")

    v1_track = next((t for t in project.timeline.tracks if t.id == "V1"), None)
    if not v1_track or not v1_track.clips:
        warnings.append("No clips found on V1 track.")

    all_clips = [c for t in project.timeline.tracks for c in t.clips]
    for clip in all_clips:
        if not Path(clip.source_path).exists():
            warnings.append(f"Missing source file: {clip.source_path}")
        if clip.timeline_end <= clip.timeline_start or clip.source_out <= clip.source_in:
            warnings.append(f"Clip '{clip.id}' has zero or negative duration.")

    return warnings


def estimate_export_duration(project: Project) -> float:
    """Estimate total output duration in seconds.

    = end time of the last clip on V1 (or longest track).
    Used for progress calculation.
    """
    all_ends = [c.timeline_end for t in project.timeline.tracks for c in t.clips]
    return max(all_ends, default=0.0)


def build_filter_complex(project: Project, settings: ExportSettings) -> str:
    """Build the FFmpeg -filter_complex string for the full project."""
    inputs = _collect_inputs(project)
    if not inputs:
        return ""
    input_index_map = {path: i for i, path in enumerate(inputs)}

    v1_track = next((t for t in project.timeline.tracks if t.id == "V1"), None)
    v1_clips = sorted(v1_track.clips, key=lambda c: c.timeline_start) if v1_track else []

    if not v1_clips:
        return ""

    v_frags, v_label = _build_video_chain(
        v1_clips, input_index_map, settings.resolution, settings.fps, "v1"
    )
    a_frags, a_label = _build_audio_chain(v1_clips, input_index_map, "a1")

    text_clips = project.timeline.text_clips
    all_frags = list(v_frags) + list(a_frags)

    if text_clips:
        txt_frags = _build_text_overlay_chain(text_clips, v_label, "vout", settings.fps)
        all_frags.extend(txt_frags)
    else:
        all_frags.append(f"[{v_label}]null[vout]")

    if a_label:
        all_frags.append(f"[{a_label}]anull[aout]")

    return ";".join(all_frags)


def build_ffmpeg_export_command(
    project: Project,
    settings: ExportSettings,
) -> list[str]:
    """Build the complete FFmpeg command as a list of strings."""
    cmd = ["ffmpeg", "-y"]
    inputs = _collect_inputs(project)
    if not inputs:
        return ["ffmpeg", "-y", "-i", str(settings.output_path)]

    for path in inputs:
        cmd.extend(["-i", path])

    filter_complex = build_filter_complex(project, settings)
    if filter_complex:
        cmd.extend(["-filter_complex", filter_complex])
        if "[vout]" in filter_complex:
            cmd.extend(["-map", "[vout]"])
        else:
            cmd.extend(["-map", "0:v"])
        if "[aout]" in filter_complex:
            cmd.extend(["-map", "[aout]"])
        else:
            cmd.extend(["-map", "0:a?"])
    else:
        cmd.extend(["-map", "0:v", "-map", "0:a?"])

    cmd.extend(
        [
            "-c:v",
            "libx264",
            "-crf",
            str(settings.crf),
            "-c:a",
            "aac",
            "-b:a",
            "192k",
            "-movflags",
            "+faststart",
            "-r",
            str(settings.fps),
            str(settings.output_path),
        ]
    )
    return cmd


class ExportWorker(QThread):
    """Runs FFmpeg export in a background thread.

    Parses FFmpeg stderr for progress and emits signals.
    """

    progress_updated = Signal(int)  # 0-100 percent
    time_updated = Signal(float, float)  # elapsed_seconds, estimated_remaining
    export_finished = Signal(Path)  # output path on success
    export_failed = Signal(str)  # error message on failure
    export_cancelled = Signal()  # user cancelled

    def __init__(
        self,
        command: list[str],
        output_path: Path,
        total_duration: float,
    ) -> None:
        super().__init__()
        self._command = command
        self._output_path = output_path
        self._total_duration = total_duration
        self._cancelled = False
        self._process: subprocess.Popen[str] | None = None

    def run(self) -> None:
        start_time = time.monotonic()
        try:
            self._process = subprocess.Popen(
                self._command,
                stderr=subprocess.PIPE,
                stdout=subprocess.DEVNULL,
                text=True,
                encoding="utf-8",
                errors="replace",
            )
            time_pattern = re.compile(r"time=(\d+):(\d+):(\d+(?:\.\d+)?)")
            if self._process.stderr is not None:
                for line in self._process.stderr:
                    if self._cancelled:
                        self._process.terminate()
                        self.export_cancelled.emit()
                        return
                    match = time_pattern.search(line)
                    if match and self._total_duration > 0:
                        h, m, s = match.groups()
                        encoded_seconds = int(h) * 3600 + int(m) * 60 + float(s)
                        pct = min(99, int(encoded_seconds / self._total_duration * 100))
                        elapsed = time.monotonic() - start_time
                        ratio = encoded_seconds / self._total_duration
                        remaining = (elapsed / ratio - elapsed) if ratio > 0 else 0.0
                        self.progress_updated.emit(pct)
                        self.time_updated.emit(elapsed, remaining)
            self._process.wait()
            if self._cancelled:
                self.export_cancelled.emit()
            elif self._process.returncode == 0:
                self.progress_updated.emit(100)
                self.export_finished.emit(self._output_path)
            else:
                self.export_failed.emit(
                    f"FFmpeg exited with code {self._process.returncode}. "
                    "Check ~/.tempo/tempo.log for details."
                )
        except Exception as e:
            self.export_failed.emit(str(e))

    def cancel(self) -> None:
        self._cancelled = True
        if self._process and self._process.poll() is None:
            self._process.terminate()
