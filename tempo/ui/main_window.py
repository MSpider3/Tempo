"""Main Window layout and application shell for Tempo."""

from datetime import datetime
from pathlib import Path
from typing import Any

from PySide6.QtCore import Qt, QUrl
from PySide6.QtGui import QAction, QCloseEvent, QDesktopServices, QShowEvent
from PySide6.QtWidgets import (
    QApplication,
    QDialog,
    QFileDialog,
    QLabel,
    QMainWindow,
    QMessageBox,
    QSplitter,
    QVBoxLayout,
    QWidget,
)

from tempo.core.edit_history import (
    AddClipCommand,
    AddTextClipCommand,
    ChangeSpeedCommand,
    ChangeTransitionCommand,
    DeleteClipCommand,
    DeleteTextClipCommand,
    EditCommand,
    EditHistory,
    ModifyTextClipCommand,
    MoveClipCommand,
    PasteClipsCommand,
    RippleDeleteCommand,
    SplitClipCommand,
    TrimClipCommand,
)
from tempo.core.models import (
    Clip,
    MediaItem,
    Project,
    ProjectSettings,
    TextClip,
    Timeline,
    Track,
)
from tempo.core.project import (
    ProjectValidationError,
    ProjectVersionError,
    load_project,
    save_project,
)
from tempo.core.timeline import find_clips_at_time, get_clip, get_clip_track, shift_clips_after
from tempo.media.exporter import (
    ExportSettings,
    ExportWorker,
    build_ffmpeg_export_command,
    estimate_export_duration,
    validate_project_for_export,
)
from tempo.ui.dialogs.export_dialog import ExportDialog
from tempo.ui.dialogs.export_progress_dialog import ExportProgressDialog
from tempo.ui.keybindings import KeyBindings
from tempo.ui.panels.inspector import InspectorPanel
from tempo.ui.panels.media_bin import MediaBinPanel
from tempo.ui.panels.preview import PreviewPanel
from tempo.ui.theme import get_font
from tempo.ui.timeline.timeline_widget import TimelineWidget
from tempo.utils.recent_projects import (
    add_recent_project,
    clear_recent_projects,
    load_recent_projects,
)


def create_default_tracks() -> list[Track]:
    """Return default set of video and audio tracks."""
    return [
        Track(id="V3", type="video", index=1),
        Track(id="V2", type="video", index=2),
        Track(id="V1", type="video", index=3),
        Track(id="A1", type="audio", index=4),
        Track(id="A2", type="audio", index=5),
        Track(id="A3", type="audio", index=6),
    ]


class MainWindow(QMainWindow):
    """The main window of the Tempo video editor."""

    def __init__(self) -> None:
        """Initialize the MainWindow."""
        super().__init__()

        # Set up project state
        self._project_path: Path | None = None
        self._mpv_initialized = False
        now_str = datetime.utcnow().isoformat() + "Z"
        self._project = Project(
            project_name="Untitled Project",
            settings=ProjectSettings(),
            created_at=now_str,
            modified_at=now_str,
            timeline=Timeline(tracks=create_default_tracks()),
        )
        self._history = EditHistory()

        # Window properties
        self.setMinimumSize(1100, 650)
        self.resize(1400, 850)
        self._update_title()

        # Center window on screen
        self._center_on_screen()

        # Initialize UI components
        self._build_ui()
        self._build_menu()
        self._build_status_bar()

        self._clipboard: list[tuple[Clip, str]] = []
        self._ripple_on_speed: bool = False
        self._export_worker: ExportWorker | None = None
        self._progress_dialog: ExportProgressDialog | None = None

        # Connect double-click signal from media bin to load media in preview player
        self.media_bin.item_double_clicked.connect(self._on_media_selected)
        self.media_bin.waveform_ready.connect(self._on_waveform_ready)
        self.media_bin.media_imported.connect(self._on_media_imported)

        # Set up keyboard shortcuts
        self._keybindings = KeyBindings(self)

    def _center_on_screen(self) -> None:
        """Center the window on the active screen."""
        screen = QApplication.primaryScreen()
        if screen:
            geom = screen.geometry()
            x = (geom.width() - self.width()) // 2
            y = (geom.height() - self.height()) // 2
            self.move(x, y)

    def _update_title(self) -> None:
        """Update the window title based on the current project name."""
        self.setWindowTitle(f"Tempo — {self._project.project_name}")

    def _build_ui(self) -> None:
        """Construct the main window layout using QSplitters."""
        central = QWidget(self)
        self.setCentralWidget(central)

        layout = QVBoxLayout(central)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.setSpacing(0)

        # Outer splitter (splits top panels and bottom timeline vertically)
        self._outer_splitter = QSplitter(Qt.Orientation.Vertical)
        layout.addWidget(self._outer_splitter)

        # Top area splitter (splits media bin, preview, and inspector horizontally)
        self._top_splitter = QSplitter(Qt.Orientation.Horizontal)
        self._outer_splitter.addWidget(self._top_splitter)

        # 1. Media Bin Panel (Left)
        # Create cache manager and pass to media bin
        from tempo.utils.cache import CacheManager

        self.cache_manager = CacheManager()
        self.media_bin = MediaBinPanel(cache=self.cache_manager, parent=self)
        self._top_splitter.addWidget(self.media_bin)

        # 2. Preview Panel (Center)
        self.preview_panel = PreviewPanel(self)
        self._top_splitter.addWidget(self.preview_panel)

        # 3. Inspector Panel (Right)
        self.inspector_panel = InspectorPanel(self)
        self._top_splitter.addWidget(self.inspector_panel)

        # Set stretch factors and initial sizes for top panels
        # Index: 0 -> MediaBin, 1 -> Preview, 2 -> Inspector
        self._top_splitter.setStretchFactor(0, 0)
        self._top_splitter.setStretchFactor(1, 2)
        self._top_splitter.setStretchFactor(2, 0)
        # Suggest default sizes
        self._top_splitter.setSizes([260, 860, 280])

        # 4. Timeline Widget (Bottom)
        self.timeline_widget = TimelineWidget(self)
        self._outer_splitter.addWidget(self.timeline_widget)

        # Outer splitter setup: index 0 is top panels, index 1 is timeline
        self._outer_splitter.setStretchFactor(0, 3)
        self._outer_splitter.setStretchFactor(1, 1)
        self._outer_splitter.setSizes([570, 280])

        # Playhead sync: preview -> timeline
        self.preview_panel.position_changed.connect(
            self.timeline_widget.lower_timeline.set_playhead
        )
        self.preview_panel.position_changed.connect(
            self.timeline_widget.upper_timeline.set_playhead
        )
        self.preview_panel.position_changed.connect(self._on_playhead_moved)
        self.preview_panel.duration_changed.connect(
            self.timeline_widget.lower_timeline.set_duration
        )
        self.preview_panel.duration_changed.connect(
            self.timeline_widget.upper_timeline.set_project_duration
        )
        self.preview_panel.is_playing_changed.connect(
            self.timeline_widget.lower_timeline.set_playing_state
        )

        # Playhead sync: timeline -> preview
        self.timeline_widget.lower_timeline.playhead_moved.connect(self.preview_panel.seek)
        self.timeline_widget.upper_timeline.seek_requested.connect(self.preview_panel.seek)

        # Drag and drop / move sync
        self.timeline_widget.lower_timeline.clip_drop_requested.connect(self._on_clip_drop)
        self.timeline_widget.lower_timeline._scene.clip_move_committed.connect(self._on_clip_move)
        self.timeline_widget.lower_timeline._scene.trim_committed.connect(self._on_trim)
        self.timeline_widget.lower_timeline._scene.blade_requested.connect(self._on_blade)
        self.timeline_widget.lower_timeline._scene.selection_changed.connect(
            self._on_selection_changed
        )
        self.timeline_widget.lower_timeline._scene.text_clip_creation_requested.connect(
            self._create_text_clip
        )
        self.timeline_widget.lower_timeline._scene.text_move_committed.connect(
            self._on_text_clip_move
        )

        # Inspector Panel connections
        self.inspector_panel.speed_change_requested.connect(self._on_speed_change)
        self.inspector_panel.pitch_correction_changed.connect(self._on_pitch_change)
        self.inspector_panel.transition_in_changed.connect(self._on_transition_in)
        self.inspector_panel.transition_out_changed.connect(self._on_transition_out)
        self.inspector_panel.text_property_changed.connect(self._on_text_property)

    def _build_menu(self) -> None:
        """Create the menu bar and populate it with actions."""
        menubar = self.menuBar()

        # ── File Menu ──
        file_menu = menubar.addMenu("File")

        new_action = QAction("New Project", self)
        new_action.setShortcut("Ctrl+N")
        new_action.triggered.connect(self.new_project)
        file_menu.addAction(new_action)

        open_action = QAction("Open Project...", self)
        open_action.setShortcut("Ctrl+O")
        open_action.triggered.connect(self.open_project)
        file_menu.addAction(open_action)

        save_action = QAction("Save Project", self)
        save_action.setShortcut("Ctrl+S")
        save_action.triggered.connect(self.save_project)
        file_menu.addAction(save_action)

        save_as_action = QAction("Save Project As...", self)
        save_as_action.setShortcut("Ctrl+Shift+S")
        save_as_action.triggered.connect(self.save_project_as)
        file_menu.addAction(save_as_action)

        file_menu.addSeparator()

        # Recent Projects Submenu
        self._recent_menu = file_menu.addMenu("Recent Projects")
        self._update_recent_menu()

        file_menu.addSeparator()

        quit_action = QAction("Quit", self)
        quit_action.setShortcut("Ctrl+Q")
        quit_action.triggered.connect(self.close)
        file_menu.addAction(quit_action)

        # ── Edit Menu ──
        edit_menu = menubar.addMenu("Edit")

        self._action_undo = QAction("Undo", self)
        self._action_undo.setShortcut("Ctrl+Z")
        self._action_undo.setEnabled(False)
        self._action_undo.triggered.connect(self.undo)
        edit_menu.addAction(self._action_undo)

        self._action_redo = QAction("Redo", self)
        self._action_redo.setShortcut("Ctrl+Shift+Z")
        self._action_redo.setEnabled(False)
        self._action_redo.triggered.connect(self.redo)
        edit_menu.addAction(self._action_redo)

        edit_menu.addSeparator()

        self._action_cut = QAction("Cut", self)
        self._action_cut.setShortcut("Ctrl+X")
        self._action_cut.triggered.connect(self.cut_clips)
        edit_menu.addAction(self._action_cut)

        self._action_copy = QAction("Copy", self)
        self._action_copy.setShortcut("Ctrl+C")
        self._action_copy.triggered.connect(self.copy_clips)
        edit_menu.addAction(self._action_copy)

        self._action_paste = QAction("Paste", self)
        self._action_paste.setShortcut("Ctrl+V")
        self._action_paste.setEnabled(False)
        self._action_paste.triggered.connect(self.paste_clips)
        edit_menu.addAction(self._action_paste)

        edit_menu.addSeparator()

        import_action = QAction("Import Media...", self)
        import_action.setShortcut("Ctrl+I")
        import_action.triggered.connect(self.media_bin.trigger_import)
        edit_menu.addAction(import_action)

        # ── Export Menu ──
        export_menu = menubar.addMenu("Export")

        export_action = QAction("Export Video...", self)
        export_action.setShortcut("Ctrl+E")
        export_action.triggered.connect(self.show_export_dialog)
        export_menu.addAction(export_action)

        # ── Help Menu ──
        help_menu = menubar.addMenu("Help")

        shortcuts_action = QAction("Keyboard Shortcuts", self)
        shortcuts_action.setShortcut("?")
        shortcuts_action.setEnabled(False)
        help_menu.addAction(shortcuts_action)

        about_action = QAction("About Tempo", self)
        about_action.triggered.connect(self._show_about)
        help_menu.addAction(about_action)

    def _build_status_bar(self) -> None:
        """Initialize the status bar with project name and duration info."""
        status = self.statusBar()

        self._status_project_label = QLabel("Untitled Project", self)
        status.addWidget(self._status_project_label)

        status.addPermanentWidget(QLabel("Duration: ", self))
        self._status_duration_label = QLabel("00:00:00:00", self)
        self._status_duration_label.setFont(get_font("mono", size=12))
        status.addPermanentWidget(self._status_duration_label)

    def _show_about(self) -> None:
        """Show the About Tempo dialog."""
        QMessageBox.about(
            self,
            "About Tempo",
            "<h3>Tempo Video Editor</h3>"
            "<p>Version 0.1.0</p>"
            "<p>A lightweight CPU-only desktop video editor inspired by "
            "DaVinci Resolve's Cut Page.</p>"
            "<p>Built using PySide6 (Qt) and FFmpeg.</p>",
        )

    # ── Project Management ───────────────────────────────────────────────────

    def new_project(self) -> None:
        """Create a new project after prompting to save changes."""
        # Simple prompt for new project (no changes check implemented yet in Week 3)
        reply = QMessageBox.question(
            self,
            "New Project",
            "Start a new project? Any unsaved changes will be lost.",
            QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.No,
            QMessageBox.StandardButton.No,
        )
        if reply == QMessageBox.StandardButton.No:
            return

        now_str = datetime.utcnow().isoformat() + "Z"
        self._project = Project(
            project_name="Untitled Project",
            settings=ProjectSettings(),
            created_at=now_str,
            modified_at=now_str,
            timeline=Timeline(tracks=create_default_tracks()),
        )
        self._history = EditHistory()
        self._project_path = None
        self._refresh_timeline()
        self._update_undo_redo_actions()
        self._update_title()
        self._status_project_label.setText("Untitled Project")
        # In a real app we'd clear panels/timeline here too

    def open_project(self) -> None:
        """Open a .tempo project file."""
        filepath_str, _ = QFileDialog.getOpenFileName(
            self,
            "Open Project",
            "",
            "Tempo Project (*.tempo)",
        )
        if not filepath_str:
            return
        self._load_project_path(Path(filepath_str))

    def _load_project_path(self, filepath: Path) -> None:
        """Load project from a given Path object."""
        try:
            loaded_project = load_project(filepath)
            if not loaded_project.timeline.tracks:
                loaded_project.timeline.tracks = create_default_tracks()
            self._project = loaded_project
            self._project_path = filepath
            self._history = EditHistory()
            self._refresh_timeline()
            self._update_undo_redo_actions()
            self._update_title()
            self._status_project_label.setText(self._project.project_name)
            add_recent_project(filepath)
            self._update_recent_menu()
        except ProjectVersionError as e:
            QMessageBox.critical(
                self,
                "Incompatible Project Version",
                f"Failed to load project: {e}",
            )
        except ProjectValidationError as e:
            QMessageBox.critical(
                self,
                "Invalid Project File",
                f"Project validation failed:\n{e}",
            )
        except Exception as e:
            QMessageBox.critical(
                self,
                "Error Opening Project",
                f"An unexpected error occurred:\n{e}",
            )

    def save_project(self) -> None:
        """Save the current project to the current file path, or trigger Save As."""
        if self._project_path is None:
            self.save_project_as()
        else:
            self._save_to_path(self._project_path)

    def save_project_as(self) -> None:
        """Prompt the user for a new file path and save the project there."""
        filepath_str, _ = QFileDialog.getSaveFileName(
            self,
            "Save Project As",
            "",
            "Tempo Project (*.tempo)",
        )
        if not filepath_str:
            return

        filepath = Path(filepath_str)
        if filepath.suffix != ".tempo":
            filepath = filepath.with_suffix(".tempo")

        self._project_path = filepath
        self._project.project_name = filepath.stem
        self._save_to_path(filepath)

    def _save_to_path(self, filepath: Path) -> None:
        """Serialize project and write to disk."""
        try:
            # Sync current media list from the media bin
            self._project.media = self.media_bin.all_media_items()
            self._project.modified_at = datetime.utcnow().isoformat() + "Z"

            save_project(self._project, filepath)
            self._update_title()
            self._status_project_label.setText(self._project.project_name)
            self.statusBar().showMessage("Project saved successfully.", 3000)
            add_recent_project(filepath)
            self._update_recent_menu()
        except Exception as e:
            QMessageBox.critical(
                self,
                "Error Saving Project",
                f"Failed to save project file:\n{e}",
            )

    # ── Show / Close Events ──────────────────────────────────────────────────

    def showEvent(self, event: QShowEvent) -> None:  # noqa: N802
        """Initialize MPV widget after the window is first shown."""
        super().showEvent(event)
        if not self._mpv_initialized:
            self._mpv_initialized = True
            self.preview_panel.mpv_widget.init_mpv()

    def closeEvent(self, event: QCloseEvent) -> None:  # noqa: N802
        """Clean up MPV and background workers before closing."""
        if (
            hasattr(self, "_export_worker")
            and self._export_worker is not None
            and self._export_worker.isRunning()
        ):
            reply = QMessageBox.question(
                self,
                "Export in Progress",
                "An export is currently running. Cancel it and quit?",
                QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.No,
            )
            if reply == QMessageBox.StandardButton.No:
                event.ignore()
                return
            self._export_worker.cancel()
            self._export_worker.wait(5000)

        self.preview_panel.mpv_widget.cleanup()
        self.media_bin.stop_all_workers()
        event.accept()

    def _update_recent_menu(self) -> None:
        """Dynamically populate Recent Projects submenu."""
        if not hasattr(self, "_recent_menu"):
            return
        self._recent_menu.clear()
        recent = load_recent_projects()
        if not recent:
            action = self._recent_menu.addAction("No recent projects")
            action.setEnabled(False)
            return
        for path in recent:
            action = self._recent_menu.addAction(path.name)
            action.setToolTip(str(path))
            action.triggered.connect(lambda checked=False, p=path: self._load_project_path(p))
        self._recent_menu.addSeparator()
        clear_act = self._recent_menu.addAction("Clear Recent Projects")
        clear_act.triggered.connect(self._clear_recent_projects_action)

    def _clear_recent_projects_action(self) -> None:
        """Clear recent project history."""
        clear_recent_projects()
        self._update_recent_menu()

    def _on_media_selected(self, item: MediaItem) -> None:
        """Load double-clicked media item into the preview player."""
        if item.proxy_path and Path(item.proxy_path).exists():
            path_to_load = Path(item.proxy_path)
        else:
            path_to_load = Path(item.original_path)
        self.preview_panel.load_media(path_to_load)

    def _find_media(self, media_id: str) -> MediaItem | None:
        """Find a MediaItem by ID inside the current project's media list."""
        for item in self._project.media:
            if item.id == media_id:
                return item
        return None

    def _on_clip_drop(self, media_id: str, drop_time: float, track_id: str) -> None:
        """Handle clip dropped onto the timeline from the media bin."""
        media_item = self._find_media(media_id)
        if media_item is None:
            return

        # Validate track compatibility
        if track_id.startswith("A") and media_item.type == "video":
            track_id = "V1"
        elif track_id.startswith("V") and media_item.type == "audio":
            track_id = "A1"
        elif track_id == "TX":
            track_id = "V1" if media_item.type == "video" else "A1"

        import uuid

        clip = Clip(
            id=str(uuid.uuid4()),
            media_id=media_id,
            source_path=str(media_item.original_path),
            timeline_start=max(0.0, drop_time),
            timeline_end=max(0.0, drop_time) + media_item.duration,
            source_in=0.0,
            source_out=media_item.duration,
            speed=1.0,
            pitch_correction=True,
        )

        cmd = AddClipCommand(clip, track_id)
        self._project = self._history.push(cmd, self._project)
        self._refresh_timeline()
        self._update_undo_redo_actions()

    def _on_clip_move(self, clip_id: str, new_start: float, new_track_id: str) -> None:
        """Handle clip moved to new position/track on timeline."""
        from tempo.core.timeline import get_clip as core_get_clip

        clip = core_get_clip(self._project, clip_id)
        if clip is not None:
            # Normal clip cannot move onto TX track
            if new_track_id == "TX":
                new_track_id = "V1" if clip.source_path else "A1"

            cmd = MoveClipCommand(clip_id, new_start, new_track_id, self._project)
            self._project = self._history.push(cmd, self._project)
            self._refresh_timeline()
            self._update_undo_redo_actions()

    def _on_selection_changed(self, selected_ids: list[str]) -> None:
        """Slot for handling selection updates."""
        if not selected_ids:
            self.inspector_panel.show_empty()
            return
        clip_id = selected_ids[0]  # inspector shows first selected clip only

        # Check if it's a TextClip
        for tc in self._project.timeline.text_clips:
            if tc.id == clip_id:
                self.inspector_panel.show_text_clip(tc)
                return

        # Check regular clips
        clip = get_clip(self._project, clip_id)
        if clip:
            media = self._find_media(clip.media_id)
            self.inspector_panel.show_clip(clip, media)

    def _refresh_single_clip(self, clip_id: str) -> None:
        """Lightweight refresh for single-clip property changes."""
        clip = get_clip(self._project, clip_id)
        if clip:
            self.timeline_widget.lower_timeline.update_clip_item(clip_id, clip)
        self.timeline_widget.upper_timeline.set_project_duration(
            self.timeline_widget.lower_timeline._duration
        )
        self._update_status_bar()
        self._update_undo_redo_actions()

    def _on_speed_change(self, clip_id: str, new_speed: float) -> None:
        clip = get_clip(self._project, clip_id)
        if clip is None:
            return
        cmd = ChangeSpeedCommand(
            clip_id, clip.speed, new_speed, clip.pitch_correction, clip.pitch_correction
        )
        self._project = self._history.push(cmd, self._project)
        if self._ripple_on_speed:
            track_id = get_clip_track(self._project, clip_id)
            updated_clip = get_clip(self._project, clip_id)
            if track_id and updated_clip:
                old_end = clip.timeline_end
                new_end = updated_clip.timeline_end
                delta = new_end - old_end
                if abs(delta) > 0.001:
                    self._project = shift_clips_after(self._project, track_id, old_end, delta)
                    self._refresh_timeline()
                else:
                    self._refresh_single_clip(clip_id)
            else:
                self._refresh_single_clip(clip_id)
        else:
            self._refresh_single_clip(clip_id)

        # Re-load inspector with updated clip so duration display updates
        updated = get_clip(self._project, clip_id)
        if updated:
            self.inspector_panel.show_clip(updated, self._find_media(updated.media_id))

    def _on_pitch_change(self, clip_id: str, enabled: bool) -> None:
        clip = get_clip(self._project, clip_id)
        if clip is None:
            return
        cmd = ChangeSpeedCommand(clip_id, clip.speed, clip.speed, clip.pitch_correction, enabled)
        self._project = self._history.push(cmd, self._project)
        self._refresh_single_clip(clip_id)
        updated = get_clip(self._project, clip_id)
        if updated:
            self.inspector_panel.show_clip(updated, self._find_media(updated.media_id))

    def _on_transition_in(self, clip_id: str, type_str: str, duration: float) -> None:
        clip = get_clip(self._project, clip_id)
        if clip is None:
            return
        old_type = "Cut"
        old_dur = 0.0
        if clip.transition_in is not None:
            old_type = clip.transition_in.type
            old_dur = clip.transition_in.duration
        cmd = ChangeTransitionCommand(clip_id, "in", old_type, old_dur, type_str, duration)
        self._project = self._history.push(cmd, self._project)
        self._refresh_single_clip(clip_id)

    def _on_transition_out(self, clip_id: str, type_str: str, duration: float) -> None:
        clip = get_clip(self._project, clip_id)
        if clip is None:
            return
        old_type = "Cut"
        old_dur = 0.0
        if clip.transition_out is not None:
            old_type = clip.transition_out.type
            old_dur = clip.transition_out.duration
        cmd = ChangeTransitionCommand(clip_id, "out", old_type, old_dur, type_str, duration)
        self._project = self._history.push(cmd, self._project)
        self._refresh_single_clip(clip_id)

    def _on_text_property(self, clip_id: str, changes: dict[str, Any]) -> None:
        # Find old values for undo
        old_clip = next(
            (tc for tc in self._project.timeline.text_clips if tc.id == clip_id),
            None,
        )
        if old_clip is None:
            return
        old_values = {k: getattr(old_clip, k) for k in changes}
        cmd = ModifyTextClipCommand(clip_id, old_values, changes)
        self._project = self._history.push(cmd, self._project)
        self._refresh_timeline()
        # Immediately update overlay — don't wait for next position_changed
        self._on_playhead_moved(self.preview_panel.position)

    def _on_trim(
        self,
        clip_id: str,
        old_ts: float,
        old_te: float,
        old_si: float,
        old_so: float,
        new_ts: float,
        new_te: float,
        new_si: float,
        new_so: float,
    ) -> None:
        """Handle clip trimmed commits."""
        cmd = TrimClipCommand(
            clip_id, old_ts, old_te, old_si, old_so, new_ts, new_te, new_si, new_so
        )
        self._project = self._history.push(cmd, self._project)
        self._refresh_timeline()

    def _on_blade(self, clip_id: str, split_time: float) -> None:
        """Handle blade split clicks on clips."""
        track_id = get_clip_track(self._project, clip_id)
        if track_id is None:
            return
        import uuid

        new_clip_id = str(uuid.uuid4())
        cmd = SplitClipCommand(clip_id, split_time, track_id, new_clip_id)
        self._project = self._history.push(cmd, self._project)
        self._refresh_timeline()

    def _on_blade_at_playhead(self) -> None:
        """Split clips on all tracks at current playhead time (B key shortcut)."""
        playhead_time = self.preview_panel.position
        clips_at_time = find_clips_at_time(self._project, playhead_time)
        import uuid

        any_split = False
        for track_id, clip in clips_at_time:
            # Prevent splitting too close to clip edges
            if (
                playhead_time > clip.timeline_start + 0.05
                and playhead_time < clip.timeline_end - 0.05
            ):
                new_clip_id = str(uuid.uuid4())
                cmd = SplitClipCommand(clip.id, playhead_time, track_id, new_clip_id)
                self._project = self._history.push(cmd, self._project)
                any_split = True
        if any_split:
            self._refresh_timeline()

    def _on_gap_delete(self) -> None:
        """Delete selected clips leaving gaps (Backspace)."""
        selected_ids = list(self.timeline_widget.lower_timeline.selected_clip_ids)
        if not selected_ids:
            return
        text_clip_ids = {tc.id for tc in self._project.timeline.text_clips}
        for clip_id in selected_ids:
            cmd: EditCommand
            if clip_id in text_clip_ids:
                cmd = DeleteTextClipCommand(clip_id)
            else:
                cmd = DeleteClipCommand(clip_id)
            self._project = self._history.push(cmd, self._project)
        self._refresh_timeline()

    def _on_ripple_delete(self) -> None:
        """Delete selected clips shifting subsequent clips left (Delete)."""
        selected_ids = list(self.timeline_widget.lower_timeline.selected_clip_ids)
        if not selected_ids:
            return
        text_clip_ids = {tc.id for tc in self._project.timeline.text_clips}
        selected_clips = []
        for cid in selected_ids:
            if cid in text_clip_ids:
                cmd: EditCommand = DeleteTextClipCommand(cid)
                self._project = self._history.push(cmd, self._project)
            else:
                c = get_clip(self._project, cid)
                if c is not None:
                    selected_clips.append(c)

        selected_clips.sort(key=lambda c: c.timeline_start, reverse=True)
        for clip in selected_clips:
            cmd_clip: EditCommand = RippleDeleteCommand(clip.id)
            self._project = self._history.push(cmd_clip, self._project)
        self._refresh_timeline()

    def add_text_at_playhead(self) -> None:
        """Add a text clip at the current playhead position (Ctrl+T)."""
        self._create_text_clip(self.preview_panel.position)

    def _create_text_clip(self, start_time: float) -> None:
        """Create a new default TextClip and auto-select it."""
        import uuid

        text_clip = TextClip(
            id=str(uuid.uuid4()),
            timeline_start=start_time,
            timeline_end=start_time + 5.0,
            content="Text",
            font_family="Inter",
            font_size=36,
            font_color="#FFFFFF",
            background_color="#000000",
            background_opacity=0.0,
            bold=False,
            italic=False,
            underline=False,
            alignment="center",
            position_x=50.0,
            position_y=85.0,
            rotation=0.0,
        )
        cmd = AddTextClipCommand(text_clip)
        self._project = self._history.push(cmd, self._project)
        self._refresh_timeline()
        # Auto-select the new text clip and open inspector
        self.timeline_widget.lower_timeline.restore_selection({text_clip.id})
        self._on_selection_changed([text_clip.id])
        if hasattr(self.inspector_panel._text_page, "_text_edit"):
            self.inspector_panel._text_page._text_edit.setFocus()
            self.inspector_panel._text_page._text_edit.selectAll()

    def _on_text_clip_move(self, clip_id: str, new_start: float) -> None:
        """Handle moving a TextClip on the TX track."""
        old_clip = next(
            (tc for tc in self._project.timeline.text_clips if tc.id == clip_id),
            None,
        )
        if old_clip is None:
            return
        changes: dict[str, Any] = {
            "timeline_start": new_start,
            "timeline_end": new_start + (old_clip.timeline_end - old_clip.timeline_start),
        }
        old_values: dict[str, Any] = {
            "timeline_start": old_clip.timeline_start,
            "timeline_end": old_clip.timeline_end,
        }
        cmd = ModifyTextClipCommand(clip_id, old_values, changes)
        self._project = self._history.push(cmd, self._project)
        self._refresh_timeline()

    def _on_playhead_moved(self, position: float) -> None:
        """Update text overlay whenever playhead moves."""
        active_text_clips = [
            tc
            for tc in self._project.timeline.text_clips
            if tc.timeline_start <= position <= tc.timeline_end
        ]
        self.preview_panel.update_text_overlays(active_text_clips)

    def _on_waveform_ready(self, media_id: str, peaks: list[tuple[float, float]]) -> None:
        """Slot for handling waveform extraction completion."""
        self.timeline_widget.lower_timeline.load_waveform(media_id, peaks)

    def _on_media_imported(self, media_item: MediaItem) -> None:
        """Slot for handling imported media items."""
        if media_item not in self._project.media:
            self._project.media.append(media_item)
        lookup = {m.id: m for m in self._project.media}
        self.timeline_widget.lower_timeline.set_media_lookup(lookup)

    def copy_clips(self) -> None:
        """Copy selected clips to internal clipboard (Ctrl+C)."""
        selected_ids = list(self.timeline_widget.lower_timeline.selected_clip_ids)
        self._clipboard.clear()
        for clip_id in selected_ids:
            clip = get_clip(self._project, clip_id)
            track_id = get_clip_track(self._project, clip_id)
            if clip and track_id:
                from copy import deepcopy

                self._clipboard.append((deepcopy(clip), track_id))
        self._clipboard.sort(key=lambda ct: ct[0].timeline_start)
        self._update_paste_action()

    def cut_clips(self) -> None:
        """Cut selected clips to internal clipboard (Ctrl+X)."""
        self.copy_clips()
        self._on_gap_delete()

    def paste_clips(self) -> None:
        """Paste clips from internal clipboard at playhead position (Ctrl+V)."""
        if not self._clipboard:
            return
        paste_time = self.preview_panel.position
        earliest_start = min(c.timeline_start for c, _ in self._clipboard)
        time_offset = paste_time - earliest_start

        import uuid
        from dataclasses import replace

        new_clips: list[tuple[Clip, str]] = []
        for clip, track_id in self._clipboard:
            new_clip = replace(
                clip,
                id=str(uuid.uuid4()),
                timeline_start=clip.timeline_start + time_offset,
                timeline_end=clip.timeline_end + time_offset,
            )
            new_clips.append((new_clip, track_id))

        cmd = PasteClipsCommand(new_clips)
        self._project = self._history.push(cmd, self._project)
        self._refresh_timeline()
        new_ids = {c.id for c, _ in new_clips}
        self.timeline_widget.lower_timeline.restore_selection(new_ids)

    def _update_paste_action(self) -> None:
        """Update enabled state and text of paste menu action."""
        if hasattr(self, "_action_paste") and self._action_paste is not None:
            self._action_paste.setEnabled(bool(self._clipboard))
            count = len(self._clipboard)
            self._action_paste.setText(
                f"Paste ({count} clip{'s' if count != 1 else ''})" if self._clipboard else "Paste"
            )

    def show_export_dialog(self) -> None:
        """Called by Ctrl+E. Validates, shows ExportDialog, starts export."""
        warnings = validate_project_for_export(self._project)
        critical = [
            w
            for w in warnings
            if "no clips" in w.lower() or "ffmpeg" in w.lower() or "not available" in w.lower()
        ]
        if critical:
            QMessageBox.critical(self, "Cannot Export", "\n".join(critical))
            return

        default_dir = Path(self._project_path).parent if self._project_path else Path.home()
        dialog = ExportDialog(self._project, default_dir, parent=self)
        dialog.export_requested.connect(self._start_export)
        dialog.exec()

    def _start_export(self, settings: ExportSettings) -> None:
        """Build FFmpeg command and launch ExportWorker."""
        try:
            command = build_ffmpeg_export_command(self._project, settings)
        except Exception as e:
            QMessageBox.critical(self, "Export Error", f"Failed to build export command:\n{e}")
            return

        total_dur = estimate_export_duration(self._project)
        self._export_worker = ExportWorker(command, settings.output_path, total_dur)
        self._progress_dialog = ExportProgressDialog(parent=self)

        self._export_worker.progress_updated.connect(self._progress_dialog.update_progress)
        self._export_worker.time_updated.connect(self._progress_dialog.update_time)
        self._export_worker.export_finished.connect(self._on_export_finished)
        self._export_worker.export_failed.connect(self._on_export_failed)
        self._export_worker.export_cancelled.connect(self._progress_dialog.close)
        self._progress_dialog.cancel_requested.connect(self._export_worker.cancel)

        self._export_worker.start()
        self._progress_dialog.exec()

    def _on_export_finished(self, output_path: Path) -> None:
        if hasattr(self, "_progress_dialog") and self._progress_dialog is not None:
            self._progress_dialog.mark_complete(output_path)
        msg = QMessageBox(self)
        msg.setWindowTitle("Export Complete")
        msg.setText("Video exported successfully.")
        msg.setInformativeText(str(output_path))
        open_file_btn = msg.addButton("Open File", QMessageBox.ButtonRole.ActionRole)
        open_folder_btn = msg.addButton("Open Folder", QMessageBox.ButtonRole.ActionRole)
        msg.addButton(QMessageBox.StandardButton.Ok)
        msg.exec()
        if msg.clickedButton() == open_file_btn:
            QDesktopServices.openUrl(QUrl.fromLocalFile(str(output_path)))
        elif msg.clickedButton() == open_folder_btn:
            QDesktopServices.openUrl(QUrl.fromLocalFile(str(output_path.parent)))

    def _on_export_failed(self, error: str) -> None:
        if hasattr(self, "_progress_dialog") and self._progress_dialog is not None:
            self._progress_dialog.mark_failed(error)
        QMessageBox.critical(self, "Export Failed", error)

    def show_speed_dialog(self) -> None:
        selected_ids = self.timeline_widget.lower_timeline.selected_clip_ids
        if not selected_ids:
            QMessageBox.information(
                self,
                "No Clip Selected",
                "Select a video or audio clip on the timeline first.",
            )
            return
        clip_id = next(iter(selected_ids))
        clip = get_clip(self._project, clip_id)
        if clip is None:
            return
        from tempo.ui.dialogs.speed_dialog import SpeedDialog

        dialog = SpeedDialog(clip.speed, clip.pitch_correction, parent=self)
        if dialog.exec() == QDialog.DialogCode.Accepted:
            cmd = ChangeSpeedCommand(
                clip_id,
                clip.speed,
                dialog.selected_speed,
                clip.pitch_correction,
                dialog.pitch_correction,
            )
            self._project = self._history.push(cmd, self._project)
            if self._ripple_on_speed:
                track_id = get_clip_track(self._project, clip_id)
                updated_clip = get_clip(self._project, clip_id)
                if track_id and updated_clip:
                    old_end = clip.timeline_end
                    new_end = updated_clip.timeline_end
                    delta = new_end - old_end
                    if abs(delta) > 0.001:
                        self._project = shift_clips_after(self._project, track_id, old_end, delta)
                        self._refresh_timeline()
                    else:
                        self._refresh_single_clip(clip_id)
                else:
                    self._refresh_single_clip(clip_id)
            else:
                self._refresh_single_clip(clip_id)

            # Re-show inspector with updated values
            updated = get_clip(self._project, clip_id)
            if updated:
                self.inspector_panel.show_clip(updated, self._find_media(updated.media_id))

    def speed_up_selected(self) -> None:
        """Increase speed of selected clip by one step."""
        self._step_speed(direction=+1)

    def speed_down_selected(self) -> None:
        """Decrease speed of selected clip by one step."""
        self._step_speed(direction=-1)

    def _step_speed(self, direction: int) -> None:
        speed_steps = [0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 2.0, 4.0]
        selected_ids = self.timeline_widget.lower_timeline.selected_clip_ids
        if not selected_ids:
            return
        clip_id = next(iter(selected_ids))
        clip = get_clip(self._project, clip_id)
        if clip is None:
            return
        try:
            current_index = speed_steps.index(clip.speed)
        except ValueError:
            current_index = min(
                range(len(speed_steps)),
                key=lambda i: abs(speed_steps[i] - clip.speed),
            )
        new_index = max(0, min(len(speed_steps) - 1, current_index + direction))
        if new_index == current_index:
            return
        new_speed = speed_steps[new_index]
        cmd = ChangeSpeedCommand(
            clip_id,
            clip.speed,
            new_speed,
            clip.pitch_correction,
            clip.pitch_correction,
        )
        self._project = self._history.push(cmd, self._project)
        if self._ripple_on_speed:
            track_id = get_clip_track(self._project, clip_id)
            updated_clip = get_clip(self._project, clip_id)
            if track_id and updated_clip:
                old_end = clip.timeline_end
                new_end = updated_clip.timeline_end
                delta = new_end - old_end
                if abs(delta) > 0.001:
                    self._project = shift_clips_after(self._project, track_id, old_end, delta)
                    self._refresh_timeline()
                else:
                    self._refresh_single_clip(clip_id)
            else:
                self._refresh_single_clip(clip_id)
        else:
            self._refresh_single_clip(clip_id)

        updated = get_clip(self._project, clip_id)
        if updated:
            self.inspector_panel.show_clip(updated, self._find_media(updated.media_id))

    def _refresh_timeline(self) -> None:
        """Rebuild timeline UI components to match project state."""
        selected_before = set(self.timeline_widget.lower_timeline.selected_clip_ids)
        lookup = {m.id: m for m in self._project.media}
        self.timeline_widget.lower_timeline.set_media_lookup(lookup)
        self.timeline_widget.lower_timeline.rebuild_from_project(self._project)
        self.timeline_widget.lower_timeline.restore_selection(selected_before)
        duration = self.timeline_widget.lower_timeline._duration
        self.timeline_widget.upper_timeline.set_project_duration(duration)
        self._update_status_bar()

    def undo(self) -> None:
        """Undo last timeline action."""
        self._project = self._history.undo(self._project)
        self._refresh_timeline()
        self._update_undo_redo_actions()
        self._on_selection_changed(list(self.timeline_widget.lower_timeline.selected_clip_ids))

    def redo(self) -> None:
        """Redo last timeline action."""
        self._project = self._history.redo(self._project)
        self._refresh_timeline()
        self._update_undo_redo_actions()
        self._on_selection_changed(list(self.timeline_widget.lower_timeline.selected_clip_ids))

    def _update_undo_redo_actions(self) -> None:
        """Sync enabled state for Edit menu entries."""
        self._action_undo.setEnabled(self._history.can_undo)
        self._action_redo.setEnabled(self._history.can_redo)
        if self._history.undo_description:
            self._action_undo.setText(f"Undo {self._history.undo_description}")
        else:
            self._action_undo.setText("Undo")

    def _update_status_bar(self) -> None:
        """Update duration label on status bar."""
        from tempo.utils.timecode import seconds_to_timecode

        fps = self._project.settings.framerate
        tc = seconds_to_timecode(self.timeline_widget.lower_timeline._duration, fps)
        self._status_duration_label.setText(tc)
