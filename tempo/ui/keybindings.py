"""Centralized keyboard shortcut registration for Tempo."""

from __future__ import annotations

from typing import TYPE_CHECKING

from PySide6.QtCore import Qt
from PySide6.QtGui import QKeySequence, QShortcut

from tempo.ui.timeline.timeline_widget import ActiveTool

if TYPE_CHECKING:
    from collections.abc import Callable

    from tempo.ui.main_window import MainWindow


class KeyBindings:
    """Manages application-wide keyboard shortcuts."""

    def __init__(self, main_window: MainWindow) -> None:
        """Initialize KeyBindings.

        Args:
            main_window: The main application window.
        """
        self._window = main_window
        self._shortcuts: list[QShortcut] = []
        self._register_all()

    def _register_all(self) -> None:
        """Register keyboard shortcuts for all application functions."""
        ctrl = Qt.KeyboardModifier.ControlModifier
        shift = Qt.KeyboardModifier.ShiftModifier
        alt = Qt.KeyboardModifier.AltModifier
        no_mod = Qt.KeyboardModifier.NoModifier

        # ── Project & File Shortcuts ──
        self._add(Qt.Key.Key_I, ctrl, self._window.media_bin.trigger_import)
        self._add(Qt.Key.Key_S, ctrl, self._window.save_project)
        self._add(Qt.Key.Key_S, ctrl | shift, self._window.save_project_as)
        self._add(Qt.Key.Key_O, ctrl, self._window.open_project)
        self._add(Qt.Key.Key_E, ctrl, self._window.show_export_dialog)
        self._add(Qt.Key.Key_F5, no_mod, self._window.media_bin.refresh)

        # ── History & Edit Operations ──
        self._add(Qt.Key.Key_Z, ctrl, self._window.undo)
        self._add(Qt.Key.Key_Z, ctrl | shift, self._window.redo)
        self._add(Qt.Key.Key_C, ctrl, self._window.copy_clips)
        self._add(Qt.Key.Key_X, ctrl, self._window.cut_clips)
        self._add(Qt.Key.Key_V, ctrl, self._window.paste_clips)

        # ── Preview Player Controls ──
        self._add(Qt.Key.Key_Space, no_mod, self._window.preview_panel.toggle_play_pause)
        self._add(Qt.Key.Key_J, no_mod, self._window.preview_panel.shuttle.on_j_pressed)
        self._add(Qt.Key.Key_K, no_mod, self._window.preview_panel.shuttle.on_k_pressed)
        self._add(Qt.Key.Key_L, no_mod, self._window.preview_panel.shuttle.on_l_pressed)
        self._add(
            Qt.Key.Key_Left,
            no_mod,
            lambda: self._window.preview_panel.step_frames(-1),
        )
        self._add(
            Qt.Key.Key_Right,
            no_mod,
            lambda: self._window.preview_panel.step_frames(1),
        )
        self._add(
            Qt.Key.Key_Left,
            shift,
            lambda: self._window.preview_panel.step_seconds(-1.0),
        )
        self._add(
            Qt.Key.Key_Right,
            shift,
            lambda: self._window.preview_panel.step_seconds(1.0),
        )
        self._add(Qt.Key.Key_Home, no_mod, self._window.preview_panel.seek_to_start)
        self._add(Qt.Key.Key_End, no_mod, self._window.preview_panel.seek_to_end)

        # ── In/Out Points ──
        self._add(Qt.Key.Key_I, no_mod, self._window.preview_panel.set_in_point)
        self._add(Qt.Key.Key_O, no_mod, self._window.preview_panel.set_out_point)
        self._add(Qt.Key.Key_I, alt, self._window.preview_panel.clear_in_point)
        self._add(Qt.Key.Key_O, alt, self._window.preview_panel.clear_out_point)

        # ── Timeline Tool Switching & Editing ──
        self._add(
            Qt.Key.Key_A,
            no_mod,
            lambda: self._window.timeline_widget.set_tool(ActiveTool.SELECT),
        )
        self._add(
            Qt.Key.Key_T,
            no_mod,
            lambda: self._window.timeline_widget.set_tool(ActiveTool.TRIM),
        )
        self._add(Qt.Key.Key_B, no_mod, self._window._on_blade_at_playhead)
        self._add(Qt.Key.Key_Backspace, no_mod, self._window._on_gap_delete)
        self._add(Qt.Key.Key_Delete, no_mod, self._window._on_ripple_delete)
        self._add(Qt.Key.Key_T, ctrl, self._window.add_text_at_playhead)
        self._add(Qt.Key.Key_Up, ctrl, self._window.speed_up_selected)
        self._add(Qt.Key.Key_Down, ctrl, self._window.speed_down_selected)
        self._add(Qt.Key.Key_R, ctrl, self._window.show_speed_dialog)

        # ── Timeline Zoom Controls ──
        self._add(Qt.Key.Key_Equal, ctrl, self._window.timeline_widget.zoom_in)
        self._add(Qt.Key.Key_Minus, ctrl, self._window.timeline_widget.zoom_out)
        self._add(
            Qt.Key.Key_F,
            ctrl | shift,
            self._window.timeline_widget.fit_to_content,
        )
        self._add(Qt.Key.Key_Z, shift, self._window.timeline_widget.fit_to_content)

    def _add(
        self,
        key: Qt.Key,
        modifier: Qt.KeyboardModifier,
        slot: Callable[[], None],
    ) -> None:
        """Helper to create and register a QShortcut.

        Args:
            key: Qt Key constant.
            modifier: Qt KeyboardModifier constant.
            slot: The callback function to connect.
        """
        shortcut = QShortcut(QKeySequence(modifier | key), self._window)
        shortcut.activated.connect(slot)
        self._shortcuts.append(shortcut)
