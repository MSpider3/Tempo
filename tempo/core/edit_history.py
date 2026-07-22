"""Command pattern history for undo/redo support in Tempo."""

from __future__ import annotations

from abc import ABC, abstractmethod
from copy import deepcopy
from dataclasses import replace
from typing import TYPE_CHECKING

from tempo.core.timeline import (
    add_clip,
    add_text_clip,
    change_clip_speed,
    change_clip_transition,
    get_clip,
    get_clip_track,
    modify_text_clip,
    move_clip,
    remove_clip,
    remove_text_clip,
    ripple_delete,
    split_clip,
    trim_clip,
)

if TYPE_CHECKING:
    from tempo.core.models import Clip, Project, TextClip


class EditCommand(ABC):
    """Abstract base class representing an undoable editing action."""

    @abstractmethod
    def execute(self, project: Project) -> Project:
        """Apply editing operation on the project state."""
        ...

    @abstractmethod
    def undo(self, project: Project) -> Project:
        """Revert editing operation on the project state."""
        ...

    @property
    @abstractmethod
    def description(self) -> str:
        """Human readable description of this edit."""
        ...


class AddClipCommand(EditCommand):
    """Command to add a clip to a track."""

    def __init__(self, clip: Clip, track_id: str) -> None:
        self._clip = clip
        self._track_id = track_id

    def execute(self, project: Project) -> Project:
        return add_clip(project, self._clip, self._track_id)

    def undo(self, project: Project) -> Project:
        return remove_clip(project, self._clip.id)

    @property
    def description(self) -> str:
        return f"Add clip {self._clip.id[:8]}"


class RemoveClipCommand(EditCommand):
    """Command to remove a clip, storing original state for undo."""

    def __init__(self, clip_id: str, project: Project) -> None:
        self._clip_id = clip_id
        # Cache clip and track for potential undo operations
        clip = get_clip(project, clip_id)
        if clip is None:
            raise ValueError(f"Clip with ID '{clip_id}' not found.")
        self._clip = clip

        # Find track ID containing this clip
        self._track_id = "V1"
        for track in project.timeline.tracks:
            if any(c.id == clip_id for c in track.clips):
                self._track_id = track.id
                break

    def execute(self, project: Project) -> Project:
        return remove_clip(project, self._clip_id)

    def undo(self, project: Project) -> Project:
        return add_clip(project, self._clip, self._track_id)

    @property
    def description(self) -> str:
        return f"Remove clip {self._clip_id[:8]}"


class MoveClipCommand(EditCommand):
    """Command to move a clip in time and track."""

    def __init__(self, clip_id: str, new_start: float, new_track_id: str, project: Project) -> None:
        self._clip_id = clip_id
        self._new_start = new_start
        self._new_track_id = new_track_id

        # Cache original state for undo
        clip = get_clip(project, clip_id)
        if clip is None:
            raise ValueError(f"Clip with ID '{clip_id}' not found.")
        self._old_start = clip.timeline_start

        self._old_track_id = "V1"
        for track in project.timeline.tracks:
            if any(c.id == clip_id for c in track.clips):
                self._old_track_id = track.id
                break

    def execute(self, project: Project) -> Project:
        return move_clip(project, self._clip_id, self._new_start, self._new_track_id)

    def undo(self, project: Project) -> Project:
        return move_clip(project, self._clip_id, self._old_start, self._old_track_id)

    @property
    def description(self) -> str:
        return f"Move clip {self._clip_id[:8]}"


class TrimClipCommand(EditCommand):
    """Stores original and new in/out points for both ends of the trim."""

    def __init__(
        self,
        clip_id: str,
        old_timeline_start: float,
        old_timeline_end: float,
        old_source_in: float,
        old_source_out: float,
        new_timeline_start: float,
        new_timeline_end: float,
        new_source_in: float,
        new_source_out: float,
    ) -> None:
        self._clip_id = clip_id
        self._old_timeline_start = old_timeline_start
        self._old_timeline_end = old_timeline_end
        self._old_source_in = old_source_in
        self._old_source_out = old_source_out
        self._new_timeline_start = new_timeline_start
        self._new_timeline_end = new_timeline_end
        self._new_source_in = new_source_in
        self._new_source_out = new_source_out

    def execute(self, project: Project) -> Project:
        return trim_clip(
            project,
            self._clip_id,
            self._new_timeline_start,
            self._new_timeline_end,
            self._new_source_in,
            self._new_source_out,
        )

    def undo(self, project: Project) -> Project:
        return trim_clip(
            project,
            self._clip_id,
            self._old_timeline_start,
            self._old_timeline_end,
            self._old_source_in,
            self._old_source_out,
        )

    @property
    def description(self) -> str:
        return "Trim clip"


class SplitClipCommand(EditCommand):
    """Splits one clip into two at a given time. Undo merges them back."""

    def __init__(self, clip_id: str, split_time: float, track_id: str, new_clip_id: str) -> None:
        self._clip_id = clip_id
        self._split_time = split_time
        self._track_id = track_id
        self._new_clip_id = new_clip_id
        self._old_timeline_start: float = 0.0
        self._old_timeline_end: float = 0.0
        self._old_source_in: float = 0.0
        self._old_source_out: float = 0.0

    def execute(self, project: Project) -> Project:
        clip = get_clip(project, self._clip_id)
        if clip is not None:
            self._old_timeline_start = clip.timeline_start
            self._old_timeline_end = clip.timeline_end
            self._old_source_in = clip.source_in
            self._old_source_out = clip.source_out
        return split_clip(project, self._clip_id, self._split_time, self._new_clip_id)

    def undo(self, project: Project) -> Project:
        p = remove_clip(project, self._new_clip_id)
        return trim_clip(
            p,
            self._clip_id,
            self._old_timeline_start,
            self._old_timeline_end,
            self._old_source_in,
            self._old_source_out,
        )

    @property
    def description(self) -> str:
        return "Split clip"


class DeleteClipCommand(EditCommand):
    """Deletes a clip. Stores full clip + track_id for undo (re-add)."""

    def __init__(self, clip_id: str) -> None:
        self._clip_id = clip_id
        self._clip: Clip | None = None
        self._track_id: str | None = None

    def execute(self, project: Project) -> Project:
        clip = get_clip(project, self._clip_id)
        if clip is None:
            raise ValueError(f"Clip with ID '{self._clip_id}' not found.")
        self._clip = clip
        self._track_id = get_clip_track(project, self._clip_id)
        return remove_clip(project, self._clip_id)

    def undo(self, project: Project) -> Project:
        if self._clip is None or self._track_id is None:
            return project
        return add_clip(project, self._clip, self._track_id)

    @property
    def description(self) -> str:
        return "Delete clip"


class RippleDeleteCommand(EditCommand):
    """Deletes a clip AND shifts all subsequent clips left by the clip's duration.

    Undo restores the clip AND shifts subsequent clips back right.
    """

    def __init__(self, clip_id: str) -> None:
        self._clip_id = clip_id
        self._clip: Clip | None = None
        self._track_id: str | None = None

    def execute(self, project: Project) -> Project:
        clip = get_clip(project, self._clip_id)
        if clip is None:
            raise ValueError(f"Clip with ID '{self._clip_id}' not found.")
        self._clip = clip
        self._track_id = get_clip_track(project, self._clip_id)
        return ripple_delete(project, self._clip_id)

    def undo(self, project: Project) -> Project:
        if self._clip is None or self._track_id is None:
            return project
        duration = self._clip.timeline_end - self._clip.timeline_start
        start_time = self._clip.timeline_start

        # Shift clips starting at or after start_time to the right by duration
        new_project = deepcopy(project)
        track = None
        for t in new_project.timeline.tracks:
            if t.id == self._track_id:
                track = t
                break
        if track:
            for i, c in enumerate(track.clips):
                if c.timeline_start >= start_time:
                    track.clips[i] = replace(
                        c,
                        timeline_start=c.timeline_start + duration,
                        timeline_end=c.timeline_end + duration,
                    )
        return add_clip(new_project, self._clip, self._track_id)

    @property
    def description(self) -> str:
        return "Ripple delete"


class ChangeSpeedCommand(EditCommand):
    """Command to change clip speed and pitch correction."""

    def __init__(
        self,
        clip_id: str,
        old_speed: float,
        new_speed: float,
        old_pitch: bool,
        new_pitch: bool,
    ) -> None:
        self._clip_id = clip_id
        self._old_speed = old_speed
        self._new_speed = new_speed
        self._old_pitch = old_pitch
        self._new_pitch = new_pitch

    def execute(self, project: Project) -> Project:
        return change_clip_speed(project, self._clip_id, self._new_speed, self._new_pitch)

    def undo(self, project: Project) -> Project:
        return change_clip_speed(project, self._clip_id, self._old_speed, self._old_pitch)

    @property
    def description(self) -> str:
        return f"Change speed to {self._new_speed}×"  # noqa: RUF001


class ChangeTransitionCommand(EditCommand):
    """Command to change a transition configuration."""

    def __init__(
        self,
        clip_id: str,
        edge: str,  # "in" or "out"
        old_type: str,
        old_dur: float,
        new_type: str,
        new_dur: float,
    ) -> None:
        self._clip_id = clip_id
        self._edge = edge
        self._old_type = old_type
        self._old_dur = old_dur
        self._new_type = new_type
        self._new_dur = new_dur

    def execute(self, project: Project) -> Project:
        return change_clip_transition(
            project, self._clip_id, self._edge, self._new_type, self._new_dur
        )

    def undo(self, project: Project) -> Project:
        return change_clip_transition(
            project, self._clip_id, self._edge, self._old_type, self._old_dur
        )

    @property
    def description(self) -> str:
        return f"Set transition {self._edge}"


class ModifyTextClipCommand(EditCommand):
    """Apply a dict of field changes to a TextClip."""

    def __init__(
        self,
        clip_id: str,
        old_values: dict[str, object],
        new_values: dict[str, object],
    ) -> None:
        self._clip_id = clip_id
        self._old_values = old_values
        self._new_values = new_values

    def execute(self, project: Project) -> Project:
        return modify_text_clip(project, self._clip_id, self._new_values)

    def undo(self, project: Project) -> Project:
        return modify_text_clip(project, self._clip_id, self._old_values)

    @property
    def description(self) -> str:
        return "Edit text"


class PasteClipsCommand(EditCommand):
    """Pastes a list of clips at new positions. Undo removes them."""

    def __init__(self, clips_with_tracks: list[tuple[Clip, str]]) -> None:
        self._clips = clips_with_tracks

    def execute(self, project: Project) -> Project:
        for clip, track_id in self._clips:
            project = add_clip(project, clip, track_id)
        return project

    def undo(self, project: Project) -> Project:
        for clip, _ in self._clips:
            project = remove_clip(project, clip.id)
        return project

    @property
    def description(self) -> str:
        return f"Paste {len(self._clips)} clip(s)"


class AddTextClipCommand(EditCommand):
    """Command to add a TextClip to the project."""

    def __init__(self, text_clip: TextClip) -> None:
        self._text_clip = text_clip

    def execute(self, project: Project) -> Project:
        return add_text_clip(project, self._text_clip)

    def undo(self, project: Project) -> Project:
        return remove_text_clip(project, self._text_clip.id)

    @property
    def description(self) -> str:
        return "Add text"


class DeleteTextClipCommand(EditCommand):
    """Command to remove a TextClip from the project."""

    def __init__(self, clip_id: str) -> None:
        self._clip_id = clip_id
        self._cached: TextClip | None = None

    def execute(self, project: Project) -> Project:
        self._cached = next(
            (tc for tc in project.timeline.text_clips if tc.id == self._clip_id), None
        )
        return remove_text_clip(project, self._clip_id)

    def undo(self, project: Project) -> Project:
        if self._cached:
            return add_text_clip(project, self._cached)
        return project

    @property
    def description(self) -> str:
        return "Delete text"


class EditHistory:
    """Manages command history stacks for project edit operations."""

    MAX_DEPTH = 100

    def __init__(self) -> None:
        self._undo_stack: list[EditCommand] = []
        self._redo_stack: list[EditCommand] = []

    def push(self, command: EditCommand, project: Project) -> Project:
        """Execute command and push to undo stack. Clears redo stack."""
        new_project = command.execute(project)
        self._undo_stack.append(command)
        self._redo_stack.clear()

        if len(self._undo_stack) > self.MAX_DEPTH:
            self._undo_stack.pop(0)

        return new_project

    def undo(self, project: Project) -> Project:
        """Revert the last executed command."""
        if not self._undo_stack:
            return project
        command = self._undo_stack.pop()
        new_project = command.undo(project)
        self._redo_stack.append(command)
        return new_project

    def redo(self, project: Project) -> Project:
        """Re-execute the last undone command."""
        if not self._redo_stack:
            return project
        command = self._redo_stack.pop()
        new_project = command.execute(project)
        self._undo_stack.append(command)
        return new_project

    @property
    def can_undo(self) -> bool:
        """Check if any actions can be undone."""
        return bool(self._undo_stack)

    @property
    def can_redo(self) -> bool:
        """Check if any actions can be redone."""
        return bool(self._redo_stack)

    @property
    def undo_description(self) -> str | None:
        """Get description of last action."""
        return self._undo_stack[-1].description if self._undo_stack else None
