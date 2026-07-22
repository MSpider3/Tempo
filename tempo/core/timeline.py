"""Pure Python timeline operations. No Qt imports allowed."""

from __future__ import annotations

from copy import deepcopy
from dataclasses import replace
from typing import TYPE_CHECKING

from tempo.ui.theme import TRACK_HEIGHT, TRACKS

if TYPE_CHECKING:
    from tempo.core.models import Clip, Project, TextClip, Track


def add_clip(project: Project, clip: Clip, track_id: str) -> Project:
    """Add a clip to the specified track. Returns new Project.

    Raises:
        ValueError: If track_id is invalid.
    """
    new_project = deepcopy(project)
    track = get_track(new_project, track_id)
    if track is None:
        raise ValueError(f"Track with ID '{track_id}' not found.")

    track.clips.append(clip)
    return new_project


def remove_clip(project: Project, clip_id: str) -> Project:
    """Remove clip by ID from whichever track contains it."""
    new_project = deepcopy(project)
    for track in new_project.timeline.tracks:
        for clip in track.clips:
            if clip.id == clip_id:
                track.clips.remove(clip)
                return new_project
    return new_project


def move_clip(project: Project, clip_id: str, new_start: float, new_track_id: str) -> Project:
    """Move clip to new timeline_start on new_track_id. Recalculates timeline_end."""
    # Find original clip to know its duration
    clip = get_clip(project, clip_id)
    if clip is None:
        return project

    duration = clip.timeline_end - clip.timeline_start
    new_end = new_start + duration

    # First remove the clip, then add it to the new track with updated coordinates
    project_removed = remove_clip(project, clip_id)
    new_clip = replace(clip, timeline_start=new_start, timeline_end=new_end)
    return add_clip(project_removed, new_clip, new_track_id)


def get_clip(project: Project, clip_id: str) -> Clip | None:
    """Find clip by ID across all tracks."""
    for track in project.timeline.tracks:
        for clip in track.clips:
            if clip.id == clip_id:
                return clip
    return None


def get_track(project: Project, track_id: str) -> Track | None:
    """Find track by ID."""
    for track in project.timeline.tracks:
        if track.id == track_id:
            return track
    return None


def clips_overlap(a: Clip, b: Clip) -> bool:
    """True if two clips overlap in time (exclusive end boundary)."""
    return a.timeline_start < b.timeline_end and b.timeline_start < a.timeline_end


def find_clips_at_time(project: Project, seconds: float) -> list[tuple[str, Clip]]:
    """Return (track_id, clip) pairs for all clips that contain the given time."""
    results = []
    for track in project.timeline.tracks:
        for clip in track.clips:
            if clip.timeline_start <= seconds < clip.timeline_end:
                results.append((track.id, clip))
    return results


def add_text_clip(project: Project, text_clip: TextClip) -> Project:
    """Add text clip to the TX track."""
    new_project = deepcopy(project)
    new_project.timeline.text_clips.append(text_clip)
    return new_project


def remove_text_clip(project: Project, clip_id: str) -> Project:
    """Remove text clip by ID from TX track."""
    new_project = deepcopy(project)
    for tc in new_project.timeline.text_clips:
        if tc.id == clip_id:
            new_project.timeline.text_clips.remove(tc)
            break
    return new_project


def move_text_clip(project: Project, clip_id: str, new_start: float) -> Project:
    """Move text clip to new timeline_start on TX track."""
    new_project = deepcopy(project)
    for i, tc in enumerate(new_project.timeline.text_clips):
        if tc.id == clip_id:
            duration = tc.timeline_end - tc.timeline_start
            new_tc = replace(tc, timeline_start=new_start, timeline_end=new_start + duration)
            new_project.timeline.text_clips[i] = new_tc
            break
    return new_project


def track_id_for_drop(y_in_scene: float) -> str:
    """Given a Y position in scene coordinates, return the track ID.

    Uses TRACK_HEIGHT from theme. Returns 'V1' if y is out of bounds.
    """
    index = int(y_in_scene // TRACK_HEIGHT)
    return TRACKS[index] if 0 <= index < len(TRACKS) else "V1"


def get_clip_track(project: Project, clip_id: str) -> str | None:
    """Return the track_id that contains this clip. None if not found."""
    for track in project.timeline.tracks:
        for clip in track.clips:
            if clip.id == clip_id:
                return track.id
    return None


def clips_on_track_after(project: Project, track_id: str, after_time: float) -> list[Clip]:
    """Return all clips on a track starting at/after after_time, sorted by start time."""
    track = get_track(project, track_id)
    if not track:
        return []
    results = [c for c in track.clips if c.timeline_start >= after_time]
    results.sort(key=lambda c: c.timeline_start)
    return results


def trim_clip(
    project: Project,
    clip_id: str,
    new_timeline_start: float,
    new_timeline_end: float,
    new_source_in: float,
    new_source_out: float,
) -> Project:
    """Replace a clip's timing values. Returns new Project.

    Validates: new_timeline_start < new_timeline_end,
               new_source_in < new_source_out,
               new_source_in >= 0,
               new_source_out <= media duration (not enforced here — caller's job)
    Minimum clip duration: 0.1 seconds.
    """
    if new_timeline_start >= new_timeline_end:
        raise ValueError("new_timeline_start must be less than new_timeline_end")
    if new_source_in >= new_source_out:
        raise ValueError("new_source_in must be less than new_source_out")
    if new_source_in < 0:
        raise ValueError("new_source_in cannot be negative")
    # Minimum duration check: 0.1 seconds (allow tiny floating point tolerance)
    if (new_timeline_end - new_timeline_start) < 0.09999:
        raise ValueError("Minimum clip duration is 0.1 seconds")

    new_project = deepcopy(project)
    found = False
    for track in new_project.timeline.tracks:
        for i, clip in enumerate(track.clips):
            if clip.id == clip_id:
                track.clips[i] = replace(
                    clip,
                    timeline_start=new_timeline_start,
                    timeline_end=new_timeline_end,
                    source_in=new_source_in,
                    source_out=new_source_out,
                )
                found = True
                break
        if found:
            break

    if not found:
        raise ValueError(f"Clip with ID '{clip_id}' not found.")
    return new_project


def split_clip(project: Project, clip_id: str, split_time: float, new_clip_id: str) -> Project:
    """Split clip at split_time into two clips.

    Original clip: timeline_start → split_time,
                   source_in → source_in + (split_time - timeline_start)
    New clip:      split_time → timeline_end,
                   source_in + (split_time - timeline_start) → source_out
    New clip inherits all other properties (speed, transitions) from original.
    split_time must be strictly between clip.timeline_start and clip.timeline_end.
    Raises ValueError if split_time is outside the clip bounds.
    """
    clip = get_clip(project, clip_id)
    if clip is None:
        raise ValueError(f"Clip with ID '{clip_id}' not found.")

    if split_time <= clip.timeline_start or split_time >= clip.timeline_end:
        raise ValueError(
            f"split_time {split_time} is outside bounds "
            f"[{clip.timeline_start}, {clip.timeline_end}]."
        )

    track_id = get_clip_track(project, clip_id)
    if track_id is None:
        raise ValueError(f"Track for clip with ID '{clip_id}' not found.")

    new_project = deepcopy(project)
    track = get_track(new_project, track_id)
    if track is None:
        raise ValueError(f"Track with ID '{track_id}' not found.")

    # Find the clip index in track
    clip_index = -1
    for i, c in enumerate(track.clips):
        if c.id == clip_id:
            clip_index = i
            break

    if clip_index == -1:
        raise ValueError(f"Clip with ID '{clip_id}' not found in track clips.")

    orig_clip = track.clips[clip_index]
    # Calculate source split point using speed factor
    source_split = orig_clip.source_in + (split_time - orig_clip.timeline_start) * orig_clip.speed

    clip1 = replace(
        orig_clip,
        timeline_end=split_time,
        source_out=source_split,
    )
    clip2 = replace(
        orig_clip,
        id=new_clip_id,
        timeline_start=split_time,
        source_in=source_split,
    )

    track.clips[clip_index] = clip1
    track.clips.insert(clip_index + 1, clip2)

    return new_project


def ripple_delete(project: Project, clip_id: str) -> Project:
    """Delete clip and shift all clips on the SAME TRACK that start after

    the deleted clip's timeline_start to the left by the deleted clip's duration.
    Other tracks are NOT affected (only ripple within the same track).
    """
    clip = get_clip(project, clip_id)
    if not clip:
        return project
    track_id = get_clip_track(project, clip_id)
    if not track_id:
        return project

    duration = clip.timeline_end - clip.timeline_start
    start_time = clip.timeline_start

    new_project = remove_clip(project, clip_id)

    track = get_track(new_project, track_id)
    if track:
        for i, c in enumerate(track.clips):
            if c.timeline_start >= start_time:
                track.clips[i] = replace(
                    c,
                    timeline_start=c.timeline_start - duration,
                    timeline_end=c.timeline_end - duration,
                )
    return new_project


def change_clip_speed(
    project: Project, clip_id: str, speed: float, pitch_correction: bool
) -> Project:
    """Update speed and pitch_correction on a clip.

    Recalculates timeline_end:
    new_timeline_end = timeline_start + (source_out - source_in) / speed
    """
    new_project = deepcopy(project)
    for track in new_project.timeline.tracks:
        for idx, clip in enumerate(track.clips):
            if clip.id == clip_id:
                source_duration = clip.source_out - clip.source_in
                new_timeline_end = clip.timeline_start + source_duration / speed
                track.clips[idx] = replace(
                    clip,
                    speed=speed,
                    pitch_correction=pitch_correction,
                    timeline_end=new_timeline_end,
                )
                return new_project
    raise ValueError(f"Clip with ID '{clip_id}' not found.")


def change_clip_transition(
    project: Project,
    clip_id: str,
    edge: str,
    transition_type: str,
    duration: float,
) -> Project:
    """Update transition_in or transition_out on a clip."""
    new_project = deepcopy(project)
    from tempo.core.models import TransitionConfig

    for track in new_project.timeline.tracks:
        for idx, clip in enumerate(track.clips):
            if clip.id == clip_id:
                new_transition = TransitionConfig(type=transition_type, duration=duration)
                if edge == "in":
                    track.clips[idx] = replace(clip, transition_in=new_transition)
                elif edge == "out":
                    track.clips[idx] = replace(clip, transition_out=new_transition)
                else:
                    raise ValueError(f"Invalid transition edge '{edge}'. Expected 'in' or 'out'.")
                return new_project
    raise ValueError(f"Clip with ID '{clip_id}' not found.")


def modify_text_clip(project: Project, clip_id: str, changes: dict[str, object]) -> Project:
    """Apply a dict of field changes to a TextClip using dataclasses.replace."""
    new_project = deepcopy(project)
    for idx, tc in enumerate(new_project.timeline.text_clips):
        if tc.id == clip_id:
            from typing import Any

            tc_any: Any = tc
            new_project.timeline.text_clips[idx] = replace(tc_any, **changes)
            return new_project
    raise ValueError(f"Text clip with ID '{clip_id}' not found.")


def shift_clips_after(project: Project, track_id: str, after_time: float, delta: float) -> Project:
    """Shift all clips on track_id that start at or after after_time by delta seconds.

    Used for ripple-on-speed-change.
    """
    track = get_track(project, track_id)
    if track is None:
        return project
    new_clips = []
    for clip in track.clips:
        if clip.timeline_start >= after_time - 0.001:
            new_clips.append(
                replace(
                    clip,
                    timeline_start=clip.timeline_start + delta,
                    timeline_end=clip.timeline_end + delta,
                )
            )
        else:
            new_clips.append(clip)
    new_track = replace(track, clips=new_clips)
    new_tracks = [new_track if t.id == track_id else t for t in project.timeline.tracks]
    return replace(project, timeline=replace(project.timeline, tracks=new_tracks))
