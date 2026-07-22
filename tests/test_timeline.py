"""Unit tests for core timeline operations and edit history command pattern."""

from __future__ import annotations

from typing import TYPE_CHECKING

import pytest

if TYPE_CHECKING:
    from pytestqt.qtbot import QtBot

from tempo.core.edit_history import (
    AddClipCommand,
    DeleteClipCommand,
    EditHistory,
    MoveClipCommand,
    SplitClipCommand,
)
from tempo.core.models import Clip, Project, ProjectSettings, Timeline, Track
from tempo.core.timeline import (
    add_clip,
    change_clip_speed,
    clips_overlap,
    get_clip,
    modify_text_clip,
    move_clip,
    remove_clip,
    ripple_delete,
    split_clip,
    track_id_for_drop,
    trim_clip,
)


def _create_test_project() -> Project:
    """Helper to create a fresh test project with default tracks."""
    tracks = [
        Track(id="TX", type="video", index=0),
        Track(id="V3", type="video", index=1),
        Track(id="V2", type="video", index=2),
        Track(id="V1", type="video", index=3),
        Track(id="A1", type="audio", index=4),
        Track(id="A2", type="audio", index=5),
        Track(id="A3", type="audio", index=6),
    ]
    return Project(
        project_name="Test Project",
        settings=ProjectSettings(),
        created_at="2026-07-20T12:00:00Z",
        modified_at="2026-07-20T12:00:00Z",
        timeline=Timeline(tracks=tracks),
    )


def test_add_clip_to_track() -> None:
    """Add a clip, verify it appears in the correct track."""
    proj = _create_test_project()
    clip = Clip(
        id="c1",
        media_id="m1",
        source_path="mock/path.mp4",
        timeline_start=0.0,
        timeline_end=5.0,
        source_in=0.0,
        source_out=5.0,
    )
    new_proj = add_clip(proj, clip, "V1")

    # Verify track has the clip
    v1_track = next(t for t in new_proj.timeline.tracks if t.id == "V1")
    assert len(v1_track.clips) == 1
    assert v1_track.clips[0].id == "c1"


def test_remove_clip() -> None:
    """Add then remove, verify track is empty."""
    proj = _create_test_project()
    clip = Clip(
        id="c1",
        media_id="m1",
        source_path="mock/path.mp4",
        timeline_start=0.0,
        timeline_end=5.0,
        source_in=0.0,
        source_out=5.0,
    )
    proj_added = add_clip(proj, clip, "V1")
    proj_removed = remove_clip(proj_added, "c1")

    v1_track = next(t for t in proj_removed.timeline.tracks if t.id == "V1")
    assert len(v1_track.clips) == 0


def test_move_clip_changes_start() -> None:
    """Move clip, verify new timeline_start."""
    proj = _create_test_project()
    clip = Clip(
        id="c1",
        media_id="m1",
        source_path="mock/path.mp4",
        timeline_start=0.0,
        timeline_end=5.0,
        source_in=0.0,
        source_out=5.0,
    )
    proj_added = add_clip(proj, clip, "V1")
    proj_moved = move_clip(proj_added, "c1", 10.0, "V2")

    c = get_clip(proj_moved, "c1")
    assert c is not None
    assert c.timeline_start == 10.0
    assert c.timeline_end == 15.0

    # Ensure clip was removed from V1 and added to V2
    v1_track = next(t for t in proj_moved.timeline.tracks if t.id == "V1")
    v2_track = next(t for t in proj_moved.timeline.tracks if t.id == "V2")
    assert len(v1_track.clips) == 0
    assert len(v2_track.clips) == 1


def test_clips_overlap_true() -> None:
    """Two overlapping clips return True."""
    c1 = Clip(
        id="1",
        media_id="m1",
        source_path="mock/path.mp4",
        timeline_start=0.0,
        timeline_end=10.0,
        source_in=0.0,
        source_out=10.0,
    )
    c2 = Clip(
        id="2",
        media_id="m2",
        source_path="mock/path.mp4",
        timeline_start=5.0,
        timeline_end=15.0,
        source_in=0.0,
        source_out=10.0,
    )
    assert clips_overlap(c1, c2) is True
    assert clips_overlap(c2, c1) is True


def test_clips_overlap_false() -> None:
    """Non-overlapping clips return False."""
    c1 = Clip(
        id="1",
        media_id="m1",
        source_path="mock/path.mp4",
        timeline_start=0.0,
        timeline_end=5.0,
        source_in=0.0,
        source_out=5.0,
    )
    c2 = Clip(
        id="2",
        media_id="m2",
        source_path="mock/path.mp4",
        timeline_start=5.0,
        timeline_end=10.0,
        source_in=0.0,
        source_out=5.0,
    )
    assert clips_overlap(c1, c2) is False
    assert clips_overlap(c2, c1) is False


def test_track_id_for_drop() -> None:
    """Y=0 -> TX, Y=48 -> V3, Y=144 -> V1, Y=240 -> A1."""
    # ["TX", "V3", "V2", "V1", "A1", "A2", "A3"]
    # TRACK_HEIGHT = 48
    # Index 0: 0 to 47
    # Index 1: 48 to 95
    # Index 2: 96 to 143
    # Index 3: 144 to 191
    # Index 4: 192 to 239
    # Index 5: 240 to 287
    # Index 6: 288 to 335
    assert track_id_for_drop(10.0) == "TX"
    assert track_id_for_drop(48.0) == "V3"
    assert track_id_for_drop(144.0) == "V1"
    assert track_id_for_drop(192.0) == "A1"
    assert track_id_for_drop(240.0) == "A2"


def test_add_then_undo() -> None:
    """Push AddClipCommand, verify clip exists, call undo(), verify clip gone."""
    proj = _create_test_project()
    history = EditHistory()

    clip = Clip(
        id="c1",
        media_id="m1",
        source_path="mock/path.mp4",
        timeline_start=0.0,
        timeline_end=5.0,
        source_in=0.0,
        source_out=5.0,
    )
    cmd = AddClipCommand(clip, "V1")

    # Add clip
    proj = history.push(cmd, proj)
    c = get_clip(proj, "c1")
    assert c is not None

    # Undo
    proj = history.undo(proj)
    c_undone = get_clip(proj, "c1")
    assert c_undone is None


def test_move_then_undo() -> None:
    """Push MoveClipCommand, verify new position, undo(), verify original position."""
    proj = _create_test_project()
    history = EditHistory()

    clip = Clip(
        id="c1",
        media_id="m1",
        source_path="mock/path.mp4",
        timeline_start=0.0,
        timeline_end=5.0,
        source_in=0.0,
        source_out=5.0,
    )
    proj = add_clip(proj, clip, "V1")

    cmd = MoveClipCommand("c1", 10.0, "V2", proj)

    # Move clip
    proj = history.push(cmd, proj)
    c = get_clip(proj, "c1")
    assert c is not None
    assert c.timeline_start == 10.0

    # Undo
    proj = history.undo(proj)
    c_undone = get_clip(proj, "c1")
    assert c_undone is not None
    assert c_undone.timeline_start == 0.0

    # Redo
    proj = history.redo(proj)
    c_redone = get_clip(proj, "c1")
    assert c_redone is not None
    assert c_redone.timeline_start == 10.0


def test_edit_history_max_depth() -> None:
    """Push 105 commands, verify undo stack length is 100."""
    proj = _create_test_project()
    history = EditHistory()

    for i in range(105):
        clip = Clip(
            id=f"c{i}",
            media_id="m1",
            source_path="mock/path.mp4",
            timeline_start=0.0,
            timeline_end=5.0,
            source_in=0.0,
            source_out=5.0,
        )
        cmd = AddClipCommand(clip, "V1")
        proj = history.push(cmd, proj)

    assert len(history._undo_stack) == 100


def test_split_clip_at_midpoint() -> None:
    """Verify two clips are created with correct boundaries when split at midpoint."""
    proj = _create_test_project()
    clip = Clip(
        id="c1",
        media_id="m1",
        source_path="mock/path.mp4",
        timeline_start=0.0,
        timeline_end=10.0,
        source_in=0.0,
        source_out=10.0,
    )
    proj = add_clip(proj, clip, "V1")

    new_proj = split_clip(proj, "c1", 5.0, "c2")

    c1 = get_clip(new_proj, "c1")
    c2 = get_clip(new_proj, "c2")
    assert c1 is not None
    assert c2 is not None

    assert c1.timeline_start == 0.0
    assert c1.timeline_end == 5.0
    assert c1.source_in == 0.0
    assert c1.source_out == 5.0

    assert c2.timeline_start == 5.0
    assert c2.timeline_end == 10.0
    assert c2.source_in == 5.0
    assert c2.source_out == 10.0


def test_split_clip_outside_bounds_raises() -> None:
    """ValueError expected when split_time is outside bounds."""
    proj = _create_test_project()
    clip = Clip(
        id="c1",
        media_id="m1",
        source_path="mock/path.mp4",
        timeline_start=2.0,
        timeline_end=8.0,
        source_in=0.0,
        source_out=6.0,
    )
    proj = add_clip(proj, clip, "V1")

    with pytest.raises(ValueError):
        split_clip(proj, "c1", 1.0, "c2")

    with pytest.raises(ValueError):
        split_clip(proj, "c1", 9.0, "c2")


def test_split_clip_inherits_speed() -> None:
    """New clip has the same speed and properties as original."""
    proj = _create_test_project()
    clip = Clip(
        id="c1",
        media_id="m1",
        source_path="mock/path.mp4",
        timeline_start=0.0,
        timeline_end=10.0,
        source_in=0.0,
        source_out=20.0,
        speed=2.0,
    )
    proj = add_clip(proj, clip, "V1")

    new_proj = split_clip(proj, "c1", 5.0, "c2")

    c1 = get_clip(new_proj, "c1")
    c2 = get_clip(new_proj, "c2")
    assert c1 is not None
    assert c2 is not None

    assert c1.speed == 2.0
    assert c2.speed == 2.0
    assert c1.source_out == 10.0
    assert c2.source_in == 10.0


def test_trim_left_edge() -> None:
    """Move timeline_start right → source_in increases."""
    proj = _create_test_project()
    clip = Clip(
        id="c1",
        media_id="m1",
        source_path="mock/path.mp4",
        timeline_start=0.0,
        timeline_end=5.0,
        source_in=0.0,
        source_out=5.0,
    )
    proj = add_clip(proj, clip, "V1")

    # Trim 2s from left: start goes 0->2, source_in goes 0->2
    new_proj = trim_clip(proj, "c1", 2.0, 5.0, 2.0, 5.0)
    c = get_clip(new_proj, "c1")
    assert c is not None
    assert c.timeline_start == 2.0
    assert c.timeline_end == 5.0
    assert c.source_in == 2.0
    assert c.source_out == 5.0


def test_trim_right_edge() -> None:
    """Move timeline_end left → source_out decreases."""
    proj = _create_test_project()
    clip = Clip(
        id="c1",
        media_id="m1",
        source_path="mock/path.mp4",
        timeline_start=0.0,
        timeline_end=5.0,
        source_in=0.0,
        source_out=5.0,
    )
    proj = add_clip(proj, clip, "V1")

    # Trim 2s from right: end goes 5->3, source_out goes 5->3
    new_proj = trim_clip(proj, "c1", 0.0, 3.0, 0.0, 3.0)
    c = get_clip(new_proj, "c1")
    assert c is not None
    assert c.timeline_start == 0.0
    assert c.timeline_end == 3.0
    assert c.source_in == 0.0
    assert c.source_out == 3.0


def test_trim_minimum_duration() -> None:
    """Can't trim below 0.1s."""
    proj = _create_test_project()
    clip = Clip(
        id="c1",
        media_id="m1",
        source_path="mock/path.mp4",
        timeline_start=0.0,
        timeline_end=5.0,
        source_in=0.0,
        source_out=5.0,
    )
    proj = add_clip(proj, clip, "V1")

    with pytest.raises(ValueError):
        trim_clip(proj, "c1", 0.0, 0.05, 0.0, 0.05)


def test_ripple_delete_shifts_subsequent() -> None:
    """Clips after deletion move left by deleted clip's duration."""
    proj = _create_test_project()
    c1 = Clip(
        id="c1",
        media_id="m1",
        source_path="mock/path.mp4",
        timeline_start=0.0,
        timeline_end=5.0,
        source_in=0.0,
        source_out=5.0,
    )
    c2 = Clip(
        id="c2",
        media_id="m2",
        source_path="mock/path.mp4",
        timeline_start=10.0,
        timeline_end=15.0,
        source_in=0.0,
        source_out=5.0,
    )
    proj = add_clip(proj, c1, "V1")
    proj = add_clip(proj, c2, "V1")

    new_proj = ripple_delete(proj, "c1")

    assert get_clip(new_proj, "c1") is None
    c2_new = get_clip(new_proj, "c2")
    assert c2_new is not None
    # Shunted left by 5.0 (c1's duration)
    assert c2_new.timeline_start == 5.0
    assert c2_new.timeline_end == 10.0


def test_ripple_delete_other_tracks_unaffected() -> None:
    """V2 clips don't move when V1 ripples."""
    proj = _create_test_project()
    c1 = Clip(
        id="c1",
        media_id="m1",
        source_path="mock/path.mp4",
        timeline_start=0.0,
        timeline_end=5.0,
        source_in=0.0,
        source_out=5.0,
    )
    c2 = Clip(
        id="c2",
        media_id="m2",
        source_path="mock/path.mp4",
        timeline_start=10.0,
        timeline_end=15.0,
        source_in=0.0,
        source_out=5.0,
    )
    proj = add_clip(proj, c1, "V1")
    proj = add_clip(proj, c2, "V2")

    new_proj = ripple_delete(proj, "c1")

    c2_new = get_clip(new_proj, "c2")
    assert c2_new is not None
    # V2 track shouldn't move
    assert c2_new.timeline_start == 10.0
    assert c2_new.timeline_end == 15.0


def test_delete_clip_undo_restores() -> None:
    """Full delete/undo round-trip through EditHistory."""
    proj = _create_test_project()
    history = EditHistory()
    clip = Clip(
        id="c1",
        media_id="m1",
        source_path="mock/path.mp4",
        timeline_start=2.0,
        timeline_end=7.0,
        source_in=0.0,
        source_out=5.0,
    )
    proj = add_clip(proj, clip, "V1")

    cmd = DeleteClipCommand("c1")
    proj = history.push(cmd, proj)
    assert get_clip(proj, "c1") is None

    proj = history.undo(proj)
    c = get_clip(proj, "c1")
    assert c is not None
    assert c.timeline_start == 2.0
    assert c.timeline_end == 7.0


def test_split_then_undo_merges() -> None:
    """SplitClipCommand undo restores single clip."""
    proj = _create_test_project()
    history = EditHistory()
    clip = Clip(
        id="c1",
        media_id="m1",
        source_path="mock/path.mp4",
        timeline_start=0.0,
        timeline_end=10.0,
        source_in=0.0,
        source_out=10.0,
    )
    proj = add_clip(proj, clip, "V1")

    cmd = SplitClipCommand("c1", 4.0, "V1", "c2")
    proj = history.push(cmd, proj)

    assert get_clip(proj, "c1") is not None
    assert get_clip(proj, "c2") is not None

    # Undo split
    proj = history.undo(proj)
    assert get_clip(proj, "c2") is None
    c1 = get_clip(proj, "c1")
    assert c1 is not None
    assert c1.timeline_start == 0.0
    assert c1.timeline_end == 10.0
    assert c1.source_in == 0.0
    assert c1.source_out == 10.0


def test_speed_change_recalculates_duration() -> None:
    """10s source at 2x -> timeline_end - timeline_start == 5.0."""
    proj = _create_test_project()
    clip = Clip(
        id="c1",
        media_id="m1",
        source_path="mock/path.mp4",
        timeline_start=0.0,
        timeline_end=10.0,
        source_in=0.0,
        source_out=10.0,
    )
    proj = add_clip(proj, clip, "V1")
    proj = change_clip_speed(proj, "c1", 2.0, True)

    c = get_clip(proj, "c1")
    assert c is not None
    assert c.speed == 2.0
    assert c.timeline_end - c.timeline_start == 5.0


def test_speed_change_undo() -> None:
    """Change speed, undo, verify original timeline_end restored."""
    from tempo.core.edit_history import ChangeSpeedCommand

    proj = _create_test_project()
    history = EditHistory()
    clip = Clip(
        id="c1",
        media_id="m1",
        source_path="mock/path.mp4",
        timeline_start=0.0,
        timeline_end=10.0,
        source_in=0.0,
        source_out=10.0,
        speed=1.0,
    )
    proj = add_clip(proj, clip, "V1")

    cmd = ChangeSpeedCommand("c1", 1.0, 2.0, True, True)
    proj = history.push(cmd, proj)

    c = get_clip(proj, "c1")
    assert c is not None
    assert c.timeline_end == 5.0

    proj = history.undo(proj)
    c_undone = get_clip(proj, "c1")
    assert c_undone is not None
    assert c_undone.timeline_end == 10.0


def test_modify_text_clip_single_field() -> None:
    """Change font_size to 48, verify only that field changed."""
    from tempo.core.models import TextClip

    proj = _create_test_project()
    tc = TextClip(
        id="t1",
        timeline_start=0.0,
        timeline_end=5.0,
        content="hello",
        font_size=36,
        font_family="Inter",
    )
    proj.timeline.text_clips.append(tc)

    proj = modify_text_clip(proj, "t1", {"font_size": 48})
    tc_new = proj.timeline.text_clips[0]
    assert tc_new.font_size == 48
    assert tc_new.content == "hello"
    assert tc_new.font_family == "Inter"


def test_modify_text_clip_undo() -> None:
    """Change content, undo, verify original content restored."""
    from tempo.core.edit_history import ModifyTextClipCommand
    from tempo.core.models import TextClip

    proj = _create_test_project()
    history = EditHistory()
    tc = TextClip(
        id="t1",
        timeline_start=0.0,
        timeline_end=5.0,
        content="original",
    )
    proj.timeline.text_clips.append(tc)

    cmd = ModifyTextClipCommand("t1", {"content": "original"}, {"content": "modified"})
    proj = history.push(cmd, proj)

    assert proj.timeline.text_clips[0].content == "modified"

    proj = history.undo(proj)
    assert proj.timeline.text_clips[0].content == "original"


def test_change_transition_in() -> None:
    """Set transition_in type and duration, verify on clip."""
    from tempo.core.edit_history import ChangeTransitionCommand

    proj = _create_test_project()
    history = EditHistory()
    clip = Clip(
        id="c1",
        media_id="m1",
        source_path="mock/path.mp4",
        timeline_start=0.0,
        timeline_end=10.0,
        source_in=0.0,
        source_out=10.0,
    )
    proj = add_clip(proj, clip, "V1")

    cmd = ChangeTransitionCommand("c1", "in", "Cut", 0.0, "Fade In", 1.5)
    proj = history.push(cmd, proj)

    c = get_clip(proj, "c1")
    assert c is not None
    assert c.transition_in is not None
    assert c.transition_in.type == "Fade In"
    assert c.transition_in.duration == 1.5

    proj = history.undo(proj)
    c_undone = get_clip(proj, "c1")
    assert c_undone is not None
    assert c_undone.transition_in is not None
    assert c_undone.transition_in.type == "Cut"


def test_paste_clips_at_playhead() -> None:
    """Copy clip at t=5, paste at t=10, verify new clip at t=10."""
    from dataclasses import replace

    from tempo.core.edit_history import PasteClipsCommand
    from tempo.core.models import Clip
    from tempo.core.timeline import add_clip, get_clip

    proj = _create_test_project()
    c1 = Clip(
        id="c1",
        media_id="m1",
        source_path="mock/path.mp4",
        timeline_start=5.0,
        timeline_end=8.0,
        source_in=0.0,
        source_out=3.0,
    )
    proj = add_clip(proj, c1, "V1")

    pasted_clip = replace(c1, id="c2", timeline_start=10.0, timeline_end=13.0)
    cmd = PasteClipsCommand([(pasted_clip, "V1")])
    proj = cmd.execute(proj)

    c2 = get_clip(proj, "c2")
    assert c2 is not None
    assert c2.timeline_start == 10.0
    assert c2.timeline_end == 13.0


def test_paste_updates_ids() -> None:
    """Pasted clip has different ID from original."""
    from dataclasses import replace

    from tempo.core.edit_history import PasteClipsCommand
    from tempo.core.models import Clip
    from tempo.core.timeline import add_clip

    proj = _create_test_project()
    c1 = Clip(
        id="orig",
        media_id="m1",
        source_path="mock/path.mp4",
        timeline_start=0.0,
        timeline_end=5.0,
        source_in=0.0,
        source_out=5.0,
    )
    proj = add_clip(proj, c1, "V1")

    pasted_clip = replace(c1, id="new_id", timeline_start=10.0, timeline_end=15.0)
    cmd = PasteClipsCommand([(pasted_clip, "V1")])
    proj = cmd.execute(proj)

    v1_track = next(t for t in proj.timeline.tracks if t.id == "V1")
    ids = {c.id for c in v1_track.clips}
    assert "orig" in ids
    assert "new_id" in ids


def test_cut_removes_original() -> None:
    """After cut, original clip is gone from project."""
    from tempo.core.models import Clip
    from tempo.core.timeline import add_clip, get_clip, remove_clip

    proj = _create_test_project()
    c1 = Clip(
        id="c1",
        media_id="m1",
        source_path="mock/path.mp4",
        timeline_start=0.0,
        timeline_end=5.0,
        source_in=0.0,
        source_out=5.0,
    )
    proj = add_clip(proj, c1, "V1")
    proj = remove_clip(proj, "c1")
    assert get_clip(proj, "c1") is None


def test_paste_multiple_clips_preserves_relative_spacing() -> None:
    """Two clips 3s apart, paste at t=20, verify still 3s apart."""
    from dataclasses import replace

    from tempo.core.edit_history import PasteClipsCommand
    from tempo.core.models import Clip
    from tempo.core.timeline import add_clip

    proj = _create_test_project()
    c1 = Clip(
        id="c1",
        media_id="m1",
        source_path="mock.mp4",
        timeline_start=2.0,
        timeline_end=4.0,
        source_in=0.0,
        source_out=2.0,
    )
    c2 = Clip(
        id="c2",
        media_id="m2",
        source_path="mock2.mp4",
        timeline_start=7.0,
        timeline_end=9.0,
        source_in=0.0,
        source_out=2.0,
    )
    proj = add_clip(proj, c1, "V1")
    proj = add_clip(proj, c2, "V1")

    # Pasting with playhead at 20.0 (offset = 20.0 - 2.0 = 18.0)
    p1 = replace(c1, id="p1", timeline_start=20.0, timeline_end=22.0)
    p2 = replace(c2, id="p2", timeline_start=25.0, timeline_end=27.0)

    cmd = PasteClipsCommand([(p1, "V1"), (p2, "V1")])
    proj = cmd.execute(proj)

    v1_track = next(t for t in proj.timeline.tracks if t.id == "V1")
    p1_found = next(c for c in v1_track.clips if c.id == "p1")
    p2_found = next(c for c in v1_track.clips if c.id == "p2")

    assert p1_found.timeline_start == 20.0
    assert p2_found.timeline_start == 25.0
    assert p2_found.timeline_start - p1_found.timeline_end == 3.0


def test_paste_undo() -> None:
    """Paste then undo, pasted clips gone."""
    from dataclasses import replace

    from tempo.core.edit_history import EditHistory, PasteClipsCommand
    from tempo.core.models import Clip
    from tempo.core.timeline import add_clip, get_clip

    proj = _create_test_project()
    history = EditHistory()
    c1 = Clip(
        id="c1",
        media_id="m1",
        source_path="mock.mp4",
        timeline_start=0.0,
        timeline_end=5.0,
        source_in=0.0,
        source_out=5.0,
    )
    proj = add_clip(proj, c1, "V1")

    pasted = replace(c1, id="c_pasted", timeline_start=10.0, timeline_end=15.0)
    cmd = PasteClipsCommand([(pasted, "V1")])
    proj = history.push(cmd, proj)

    assert get_clip(proj, "c_pasted") is not None

    proj = history.undo(proj)
    assert get_clip(proj, "c_pasted") is None


def test_add_text_clip_to_tx_track() -> None:
    """Create text clip, verify in project.timeline.text_clips."""
    from tempo.core.models import TextClip
    from tempo.core.timeline import add_text_clip

    proj = _create_test_project()
    tc = TextClip(id="t1", timeline_start=0.0, timeline_end=5.0, content="Hello")
    proj = add_text_clip(proj, tc)

    assert len(proj.timeline.text_clips) == 1
    assert proj.timeline.text_clips[0].id == "t1"
    assert proj.timeline.text_clips[0].content == "Hello"


def test_delete_text_clip() -> None:
    """Add then remove text clip, verify list is empty."""
    from tempo.core.models import TextClip
    from tempo.core.timeline import add_text_clip, remove_text_clip

    proj = _create_test_project()
    tc = TextClip(id="t1", timeline_start=0.0, timeline_end=5.0, content="Hello")
    proj = add_text_clip(proj, tc)
    proj = remove_text_clip(proj, "t1")

    assert len(proj.timeline.text_clips) == 0


def test_add_text_clip_undo() -> None:
    """Push AddTextClipCommand, undo, verify text clip gone."""
    from tempo.core.edit_history import AddTextClipCommand, EditHistory
    from tempo.core.models import TextClip

    proj = _create_test_project()
    history = EditHistory()
    tc = TextClip(id="t1", timeline_start=0.0, timeline_end=5.0, content="Hello")
    cmd = AddTextClipCommand(tc)
    proj = history.push(cmd, proj)

    assert len(proj.timeline.text_clips) == 1

    proj = history.undo(proj)
    assert len(proj.timeline.text_clips) == 0


def test_move_text_clip_updates_start() -> None:
    """Modify timeline_start via ModifyTextClipCommand."""
    from tempo.core.edit_history import EditHistory, ModifyTextClipCommand
    from tempo.core.models import TextClip
    from tempo.core.timeline import add_text_clip

    proj = _create_test_project()
    history = EditHistory()
    tc = TextClip(id="t1", timeline_start=0.0, timeline_end=5.0, content="Hello")
    proj = add_text_clip(proj, tc)

    cmd = ModifyTextClipCommand("t1", {"timeline_start": 0.0}, {"timeline_start": 10.0})
    proj = history.push(cmd, proj)

    assert proj.timeline.text_clips[0].timeline_start == 10.0


def test_speed_step_up_from_1x(qtbot: QtBot) -> None:
    """_step_speed(+1) on 1.0x clip -> speed becomes 1.25x."""
    from tempo.core.timeline import add_clip, get_clip
    from tempo.ui.main_window import MainWindow

    win = MainWindow()
    qtbot.addWidget(win)
    c1 = Clip(
        id="c1",
        media_id="m1",
        source_path="mock.mp4",
        timeline_start=0.0,
        timeline_end=10.0,
        source_in=0.0,
        source_out=10.0,
        speed=1.0,
    )
    win._project = add_clip(win._project, c1, "V1")
    win._refresh_timeline()
    win.timeline_widget.lower_timeline.restore_selection({"c1"})
    win._step_speed(+1)
    updated = get_clip(win._project, "c1")
    assert updated is not None
    assert updated.speed == 1.25


def test_speed_step_down_from_025x(qtbot: QtBot) -> None:
    """_step_speed(-1) on 0.25x clip -> speed stays 0.25x (clamped)."""
    from tempo.core.timeline import add_clip, get_clip
    from tempo.ui.main_window import MainWindow

    win = MainWindow()
    qtbot.addWidget(win)
    c1 = Clip(
        id="c1",
        media_id="m1",
        source_path="mock.mp4",
        timeline_start=0.0,
        timeline_end=40.0,
        source_in=0.0,
        source_out=10.0,
        speed=0.25,
    )
    win._project = add_clip(win._project, c1, "V1")
    win._refresh_timeline()
    win.timeline_widget.lower_timeline.restore_selection({"c1"})
    win._step_speed(-1)
    updated = get_clip(win._project, "c1")
    assert updated is not None
    assert updated.speed == 0.25


def test_speed_step_up_from_4x(qtbot: QtBot) -> None:
    """_step_speed(+1) on 4.0x clip -> speed stays 4x (clamped)."""
    from tempo.core.timeline import add_clip, get_clip
    from tempo.ui.main_window import MainWindow

    win = MainWindow()
    qtbot.addWidget(win)
    c1 = Clip(
        id="c1",
        media_id="m1",
        source_path="mock.mp4",
        timeline_start=0.0,
        timeline_end=2.5,
        source_in=0.0,
        source_out=10.0,
        speed=4.0,
    )
    win._project = add_clip(win._project, c1, "V1")
    win._refresh_timeline()
    win.timeline_widget.lower_timeline.restore_selection({"c1"})
    win._step_speed(+1)
    updated = get_clip(win._project, "c1")
    assert updated is not None
    assert updated.speed == 4.0


def test_speed_step_nonstandard_snaps(qtbot: QtBot) -> None:
    """clip at 1.3x steps up -> snaps to 1.25x first, then 1.5x."""
    from tempo.core.timeline import add_clip, get_clip
    from tempo.ui.main_window import MainWindow

    win = MainWindow()
    qtbot.addWidget(win)
    c1 = Clip(
        id="c1",
        media_id="m1",
        source_path="mock.mp4",
        timeline_start=0.0,
        timeline_end=7.69,
        source_in=0.0,
        source_out=10.0,
        speed=1.3,
    )
    win._project = add_clip(win._project, c1, "V1")
    win._refresh_timeline()
    win.timeline_widget.lower_timeline.restore_selection({"c1"})
    win._step_speed(+1)
    updated = get_clip(win._project, "c1")
    assert updated is not None
    assert updated.speed == 1.5


def test_shift_clips_after_positive_delta() -> None:
    """clip at t=10 shifted by +5 -> now at t=15."""
    from tempo.core.timeline import add_clip, shift_clips_after

    proj = _create_test_project()
    c1 = Clip(
        id="c1",
        media_id="m1",
        source_path="mock.mp4",
        timeline_start=10.0,
        timeline_end=15.0,
        source_in=0.0,
        source_out=5.0,
    )
    proj = add_clip(proj, c1, "V1")
    proj = shift_clips_after(proj, "V1", 10.0, 5.0)

    v1_track = next(t for t in proj.timeline.tracks if t.id == "V1")
    shifted = v1_track.clips[0]
    assert shifted.timeline_start == 15.0
    assert shifted.timeline_end == 20.0


def test_shift_clips_after_other_tracks_unaffected() -> None:
    """shift V1 clips, V2 clips unchanged."""
    from tempo.core.timeline import add_clip, shift_clips_after

    proj = _create_test_project()
    c1 = Clip(
        id="c1",
        media_id="m1",
        source_path="mock.mp4",
        timeline_start=10.0,
        timeline_end=15.0,
        source_in=0.0,
        source_out=5.0,
    )
    c2 = Clip(
        id="c2",
        media_id="m2",
        source_path="mock.mp4",
        timeline_start=10.0,
        timeline_end=15.0,
        source_in=0.0,
        source_out=5.0,
    )
    proj = add_clip(proj, c1, "V1")
    proj = add_clip(proj, c2, "V2")

    proj = shift_clips_after(proj, "V1", 10.0, 5.0)

    v1_track = next(t for t in proj.timeline.tracks if t.id == "V1")
    v2_track = next(t for t in proj.timeline.tracks if t.id == "V2")

    assert v1_track.clips[0].timeline_start == 15.0
    assert v2_track.clips[0].timeline_start == 10.0
