"""Unit tests for FFmpeg export filter complex builder in tempo/media/exporter.py."""

from __future__ import annotations

from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from pathlib import Path

from tempo.core.models import (
    Clip,
    Project,
    ProjectSettings,
    TextClip,
    Timeline,
    Track,
    TransitionConfig,
)
from tempo.media.exporter import (
    ExportSettings,
    _apply_drawtext,
    _apply_transition,
    _build_atempo_chain,
    build_ffmpeg_export_command,
    estimate_export_duration,
    validate_project_for_export,
)


def _create_minimal_project() -> Project:
    now_str = "2026-01-01T00:00:00Z"
    tracks = [
        Track(id="TX", type="video", index=0),
        Track(id="V1", type="video", index=1),
        Track(id="A1", type="audio", index=2),
    ]
    return Project(
        project_name="Test Project",
        settings=ProjectSettings(),
        created_at=now_str,
        modified_at=now_str,
        timeline=Timeline(tracks=tracks),
    )


def test_build_atempo_chain_4x() -> None:
    result = _build_atempo_chain(4.0)
    assert "atempo=2.0" in result


def test_build_atempo_chain_025x() -> None:
    result = _build_atempo_chain(0.25)
    assert "atempo=0.5" in result


def test_build_atempo_chain_1x() -> None:
    assert _build_atempo_chain(1.0) == "atempo=1.0000"


def test_atempo_chain_edge_cases() -> None:
    assert _build_atempo_chain(2.0) == "atempo=2.0000"
    assert _build_atempo_chain(0.5) == "atempo=0.5000"


def test_apply_transition_cross_dissolve() -> None:
    config = TransitionConfig(type="Cross Dissolve", duration=1.0)
    result = _apply_transition("va", "vb", config, "vout", clip_a_duration=5.0)
    assert "xfade=transition=dissolve" in result
    assert "offset=4.0" in result


def test_apply_transition_cut_returns_empty() -> None:
    config = TransitionConfig(type="Cut", duration=0.0)
    result = _apply_transition("va", "vb", config, "vout", clip_a_duration=5.0)
    assert result == ""


def test_apply_drawtext_empty_returns_empty() -> None:
    assert _apply_drawtext([], "vin", "vout", 30.0) == ""


def test_apply_drawtext_escapes_quotes() -> None:
    tc = TextClip(
        id="t1",
        timeline_start=0.0,
        timeline_end=5.0,
        content="don't break",
    )
    result = _apply_drawtext([tc], "vin", "vout", 30.0)
    assert "don\\'t break" in result


def test_apply_drawtext_time_scoped() -> None:
    tc = TextClip(
        id="t1",
        timeline_start=2.0,
        timeline_end=7.0,
        content="hello",
    )
    result = _apply_drawtext([tc], "vin", "vout", 30.0)
    assert "between(t,2.0,7.0)" in result


def test_validate_empty_project_returns_warning() -> None:
    project = _create_minimal_project()
    warnings = validate_project_for_export(project)
    assert any("no clips" in w.lower() for w in warnings)


def test_validate_missing_file_returns_warning(tmp_path: Path) -> None:
    project = _create_minimal_project()
    clip = Clip(
        id="c1",
        media_id="m1",
        source_path=str(tmp_path / "missing.mp4"),
        timeline_start=0.0,
        timeline_end=5.0,
        source_in=0.0,
        source_out=5.0,
    )
    project.timeline.tracks[1].clips.append(clip)
    warnings = validate_project_for_export(project)
    assert any("missing" in w.lower() for w in warnings)


def test_estimate_duration_single_clip() -> None:
    project = _create_minimal_project()
    clip = Clip(
        id="c1",
        media_id="m1",
        source_path="mock.mp4",
        timeline_start=0.0,
        timeline_end=10.0,
        source_in=0.0,
        source_out=10.0,
    )
    project.timeline.tracks[1].clips.append(clip)
    assert estimate_export_duration(project) == 10.0


def test_estimate_duration_multiple_clips() -> None:
    project = _create_minimal_project()
    c1 = Clip(
        id="c1",
        media_id="m1",
        source_path="mock.mp4",
        timeline_start=0.0,
        timeline_end=5.0,
        source_in=0.0,
        source_out=5.0,
    )
    c2 = Clip(
        id="c2",
        media_id="m2",
        source_path="mock.mp4",
        timeline_start=5.0,
        timeline_end=15.0,
        source_in=0.0,
        source_out=10.0,
    )
    project.timeline.tracks[1].clips.extend([c1, c2])
    assert estimate_export_duration(project) == 15.0


def test_build_command_returns_list(tmp_path: Path) -> None:
    project = _create_minimal_project()
    dummy_file = tmp_path / "clip.mp4"
    dummy_file.touch()
    clip = Clip(
        id="c1",
        media_id="m1",
        source_path=str(dummy_file),
        timeline_start=0.0,
        timeline_end=5.0,
        source_in=0.0,
        source_out=5.0,
    )
    project.timeline.tracks[1].clips.append(clip)
    settings = ExportSettings(output_path=tmp_path / "out.mp4", resolution=(1920, 1080))
    cmd = build_ffmpeg_export_command(project, settings)
    assert isinstance(cmd, list)
    assert cmd[0] == "ffmpeg"
    assert "-filter_complex" in cmd or "concat" in " ".join(cmd)
