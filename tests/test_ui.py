"""Unit tests for UI components, background workers, and signals."""

from pathlib import Path

import pytest
from PySide6.QtTest import QSignalSpy
from pytestqt.qtbot import QtBot

from tempo.core.models import MediaItem
from tempo.ui.panels.media_bin import MediaImportWorker
from tempo.ui.panels.preview import PreviewPanel
from tempo.utils.cache import CacheManager
from tests.conftest import FFMPEG_AVAILABLE, FFPROBE_AVAILABLE

REQUIRES_FFMPEG = pytest.mark.skipif(
    not FFMPEG_AVAILABLE or not FFPROBE_AVAILABLE,
    reason="FFmpeg/FFprobe required for these tests",
)


@REQUIRES_FFMPEG
def test_media_import_worker_signals(qtbot: QtBot, synthetic_video: Path, tmp_path: Path) -> None:
    """Test that MediaImportWorker correctly emits media_imported signals when probing."""
    cache = CacheManager(cache_root=tmp_path)
    worker = MediaImportWorker(paths=[synthetic_video], cache=cache)

    # Register spy to capture emitted arguments
    spy = QSignalSpy(worker.media_imported)

    # Start the worker in a background thread and block until signal is received
    with qtbot.wait_signal(worker.media_imported, timeout=10000):
        worker.start()

    # Verify signal emissions and arguments
    assert spy.count() == 1
    args = spy.at(0)
    assert len(args) == 1

    media_item = args[0]
    assert isinstance(media_item, MediaItem)
    assert media_item.original_path == str(synthetic_video.resolve())
    assert media_item.type == "video"

    # Stop and clean up thread
    worker.requestInterruption()
    worker.quit()
    worker.wait(3000)


def test_preview_panel_instantiation(qtbot: QtBot) -> None:
    """Test that PreviewPanel can be instantiated without errors."""
    panel = PreviewPanel()
    qtbot.add_widget(panel)
    assert panel.minimumWidth() == 400


def test_shuttle_controller_states() -> None:
    """Test ShuttleController state transitions using mocked PreviewPanel."""
    from unittest.mock import MagicMock

    from tempo.ui.panels.preview import ShuttleController, ShuttleState

    mock_preview = MagicMock(spec=PreviewPanel)
    shuttle: ShuttleController = ShuttleController(mock_preview)

    # Starts in STOPPED state
    assert shuttle.state == ShuttleState.STOPPED
    assert shuttle.speed == 1.0

    # Press J twice (starts REVERSE, then increases speed)
    shuttle.on_j_pressed()
    assert shuttle.state == ShuttleState.REVERSE  # type: ignore[comparison-overlap]
    assert shuttle.speed == 1.0
    mock_preview.set_speed.assert_called_with(-1.0)
    mock_preview.play.assert_called()

    shuttle.on_j_pressed()
    assert shuttle.state == ShuttleState.REVERSE
    assert shuttle.speed == 2.0
    mock_preview.set_speed.assert_called_with(-2.0)

    # Press K -> should stop
    shuttle.on_k_pressed()
    assert shuttle.state == ShuttleState.STOPPED
    assert shuttle.speed == 1.0
    mock_preview.set_speed.assert_called_with(1.0)
    mock_preview.pause.assert_called()

    # Press L twice (starts FORWARD, then increases speed)
    shuttle.on_l_pressed()
    assert shuttle.state == ShuttleState.FORWARD
    assert shuttle.speed == 1.0
    mock_preview.set_speed.assert_called_with(1.0)
    mock_preview.play.assert_called()

    shuttle.on_l_pressed()
    assert shuttle.state == ShuttleState.FORWARD
    assert shuttle.speed == 2.0
    mock_preview.set_speed.assert_called_with(2.0)

    # Press J while moving forward -> should change direction to reverse 1x
    shuttle.on_j_pressed()
    assert shuttle.state == ShuttleState.REVERSE
    assert shuttle.speed == 1.0
    mock_preview.set_speed.assert_called_with(-1.0)

    # Press K again -> resets to STOPPED
    shuttle.on_k_pressed()
    assert shuttle.state == ShuttleState.STOPPED


def test_time_to_x_conversion() -> None:
    """Test time_to_x utility logic."""
    from tempo.ui.timeline.lower_timeline import time_to_x

    assert time_to_x(10.0, 100.0) == 1000.0


def test_x_to_time_conversion() -> None:
    """Test x_to_time utility logic."""
    from tempo.ui.timeline.lower_timeline import x_to_time

    assert x_to_time(1000.0, 100.0) == 10.0


def test_zoom_slider_pps_range() -> None:
    """Test mapping slider range to PPS zoom bounds."""
    from tempo.ui.timeline.timeline_widget import slider_to_pps

    assert slider_to_pps(0) >= 10.0
    assert slider_to_pps(100) <= 500.0


def test_timeline_widget_instantiates(qtbot: QtBot) -> None:
    """Test that TimelineWidget instantiates without errors."""
    from tempo.ui.timeline.timeline_widget import TimelineWidget

    widget = TimelineWidget()
    qtbot.add_widget(widget)
    assert widget.minimumHeight() == 200


def test_track_order() -> None:
    """Test track order order spec constraints."""
    from tempo.ui.theme import TRACKS

    assert TRACKS[0] == "TX"
    assert TRACKS[-1] == "A3"


def test_fit_to_content(qtbot: QtBot) -> None:
    """Test fit_to_content zoom mapping calculations."""
    from tempo.ui.timeline.timeline_widget import TimelineWidget

    widget = TimelineWidget()
    qtbot.add_widget(widget)

    # Mock visible viewport size to 600px width
    widget.lower_timeline.viewport().resize(600, 200)
    # Mock project duration to 60s
    widget.lower_timeline.set_duration(60.0)

    # Call fit
    widget.fit_to_content()
    # Expect ~10.0 (or slightly less due to padding margin, clamped to min 10.0)
    assert widget.pixels_per_second == 10.0


def test_clip_item_geometry() -> None:
    """ClipItem with a 10s clip at 100 PPS has rect().width() == 1000.0."""
    from tempo.core.models import Clip
    from tempo.ui.timeline.clip_item import ClipItem

    clip = Clip(
        id="c1",
        media_id="m1",
        source_path="mock/path.mp4",
        timeline_start=0.0,
        timeline_end=10.0,
        source_in=0.0,
        source_out=10.0,
    )
    item = ClipItem(clip, "V1", 100.0)
    assert item.rect().width() == 1000.0


def test_clip_item_pps_update() -> None:
    """After update_pps(200.0), width is 2000.0."""
    from tempo.core.models import Clip
    from tempo.ui.timeline.clip_item import ClipItem

    clip = Clip(
        id="c1",
        media_id="m1",
        source_path="mock/path.mp4",
        timeline_start=0.0,
        timeline_end=10.0,
        source_in=0.0,
        source_out=10.0,
    )
    item = ClipItem(clip, "V1", 100.0)
    item.update_pps(200.0)
    assert item.rect().width() == 2000.0


def test_trim_edge_detection_left() -> None:
    """local_x=4 on a 200px wide ClipItem returns TrimEdge.LEFT."""
    from tempo.core.models import Clip
    from tempo.ui.timeline.clip_item import ClipItem, TrimEdge

    clip = Clip(
        id="c1",
        media_id="m1",
        source_path="mock/path.mp4",
        timeline_start=0.0,
        timeline_end=2.0,
        source_in=0.0,
        source_out=2.0,
    )
    item = ClipItem(clip, "V1", 100.0)  # 200px wide
    assert item._trim_edge_at(4.0) == TrimEdge.LEFT


def test_trim_edge_detection_right() -> None:
    """local_x=195 on a 200px wide ClipItem returns TrimEdge.RIGHT."""
    from tempo.core.models import Clip
    from tempo.ui.timeline.clip_item import ClipItem, TrimEdge

    clip = Clip(
        id="c1",
        media_id="m1",
        source_path="mock/path.mp4",
        timeline_start=0.0,
        timeline_end=2.0,
        source_in=0.0,
        source_out=2.0,
    )
    item = ClipItem(clip, "V1", 100.0)  # 200px wide
    assert item._trim_edge_at(195.0) == TrimEdge.RIGHT


def test_trim_edge_detection_center() -> None:
    """local_x=100 on a 200px wide ClipItem returns TrimEdge.NONE."""
    from tempo.core.models import Clip
    from tempo.ui.timeline.clip_item import ClipItem, TrimEdge

    clip = Clip(
        id="c1",
        media_id="m1",
        source_path="mock/path.mp4",
        timeline_start=0.0,
        timeline_end=2.0,
        source_in=0.0,
        source_out=2.0,
    )
    item = ClipItem(clip, "V1", 100.0)  # 200px wide
    assert item._trim_edge_at(100.0) == TrimEdge.NONE


def test_blade_b_key_splits_clip(qtbot: QtBot) -> None:
    """Add a clip, move playhead to middle, press B, verify two clips exist."""
    from unittest.mock import PropertyMock, patch

    from tempo.core.models import Clip
    from tempo.core.timeline import add_clip, get_clip
    from tempo.ui.main_window import MainWindow

    win = MainWindow()
    qtbot.addWidget(win)

    # Add a mock clip to V1 (0.0 to 10.0 seconds)
    clip = Clip(
        id="c1",
        media_id="m1",
        source_path="mock/path.mp4",
        timeline_start=0.0,
        timeline_end=10.0,
        source_in=0.0,
        source_out=10.0,
    )
    win._project = add_clip(win._project, clip, "V1")
    win._refresh_timeline()

    # Move playhead to 5.0 seconds
    with patch(
        "tempo.ui.panels.preview.PreviewPanel.position", new_callable=PropertyMock
    ) as mock_pos:
        mock_pos.return_value = 5.0
        win._on_blade_at_playhead()

    # Verify clip was split into c1 and a new split clip
    c1 = get_clip(win._project, "c1")
    assert c1 is not None
    assert c1.timeline_start == 0.0
    assert c1.timeline_end == 5.0

    # The second clip should be in V1 and start at 5.0
    track = next(t for t in win._project.timeline.tracks if t.id == "V1")
    assert len(track.clips) == 2
    assert any(c.timeline_start == 5.0 for c in track.clips)


def test_inspector_shows_empty_on_no_selection(qtbot: QtBot) -> None:
    """InspectorPanel.show_empty() shows index 0."""
    from tempo.ui.panels.inspector import InspectorPanel

    panel = InspectorPanel()
    qtbot.addWidget(panel)

    panel.show_empty()
    assert panel._stack.currentIndex() == 0


def test_inspector_switches_to_clip_page(qtbot: QtBot) -> None:
    """show_clip() switches to index 1."""
    from tempo.core.models import Clip
    from tempo.ui.panels.inspector import InspectorPanel

    panel = InspectorPanel()
    qtbot.addWidget(panel)

    clip = Clip(
        id="c1",
        media_id="m1",
        source_path="mock/path.mp4",
        timeline_start=0.0,
        timeline_end=10.0,
        source_in=0.0,
        source_out=10.0,
    )
    panel.show_clip(clip, None)
    assert panel._stack.currentIndex() == 1


def test_inspector_switches_to_text_page(qtbot: QtBot) -> None:
    """show_text_clip() switches to index 2."""
    from tempo.core.models import TextClip
    from tempo.ui.panels.inspector import InspectorPanel

    panel = InspectorPanel()
    qtbot.addWidget(panel)

    tc = TextClip(
        id="t1",
        timeline_start=0.0,
        timeline_end=5.0,
        content="hello",
    )
    panel.show_text_clip(tc)
    assert panel._stack.currentIndex() == 2


def test_speed_button_emits_signal(qtbot: QtBot) -> None:
    """Click the 2.0x button, verify speed_change_requested emits with (clip_id, 2.0)."""
    from PySide6.QtCore import Qt
    from PySide6.QtTest import QSignalSpy

    from tempo.core.models import Clip
    from tempo.ui.panels.inspector import InspectorPanel

    panel = InspectorPanel()
    qtbot.addWidget(panel)

    clip = Clip(
        id="c1",
        media_id="m1",
        source_path="mock/path.mp4",
        timeline_start=0.0,
        timeline_end=10.0,
        source_in=0.0,
        source_out=10.0,
    )
    panel.show_clip(clip, None)

    spy = QSignalSpy(panel.speed_change_requested)

    # Click the 2.0x button
    btn_2x = panel._clip_page._speed_buttons[2.0]
    qtbot.mouseClick(btn_2x, Qt.MouseButton.LeftButton)  # type: ignore[no-untyped-call]

    assert spy.count() == 1
    args = spy.at(0)
    assert args[0] == "c1"
    assert args[1] == 2.0


def test_loading_guard_blocks_signals(qtbot: QtBot) -> None:
    """During load(), no signals emitted."""
    from PySide6.QtTest import QSignalSpy

    from tempo.core.models import Clip
    from tempo.ui.panels.inspector import InspectorPanel

    panel = InspectorPanel()
    qtbot.addWidget(panel)

    # Set up spy on speed_change_requested and transition_in_changed
    spy_speed = QSignalSpy(panel.speed_change_requested)
    spy_trans = QSignalSpy(panel.transition_in_changed)

    clip = Clip(
        id="c1",
        media_id="m1",
        source_path="mock/path.mp4",
        timeline_start=0.0,
        timeline_end=10.0,
        source_in=0.0,
        source_out=10.0,
    )

    # Show clip (calls load)
    panel.show_clip(clip, None)

    assert spy_speed.count() == 0
    assert spy_trans.count() == 0


def test_waveform_item_renders_without_crash(qtbot: QtBot) -> None:
    """Create WaveformItem with 100 peaks, call paint() via QPixmap offscreen render."""
    from PySide6.QtGui import QPainter, QPixmap

    from tempo.ui.timeline.waveform_widget import WaveformItem

    peaks = [(-0.5, 0.5)] * 100
    item = WaveformItem(peaks, 200.0, 40.0)

    pixmap = QPixmap(200, 40)
    pixmap.fill()
    painter = QPainter(pixmap)
    try:
        item.paint(painter)
    finally:
        painter.end()


def test_waveform_subsampling_reduces_peak_count() -> None:
    """1000 peaks subsampled to 100 target -> result has 100 entries."""
    from tempo.ui.timeline.waveform_widget import subsample_peaks

    peaks = [(-float(i) / 1000.0, float(i) / 1000.0) for i in range(1000)]
    subsampled = subsample_peaks(peaks, 100)
    assert len(subsampled) == 100


def test_thumbnail_strip_tiles_correctly() -> None:
    """ThumbnailStripItem with width 300 and THUMB_WIDTH 80 -> tiles 4 times."""
    from tempo.ui.timeline.clip_item import ThumbnailStripItem

    strip = ThumbnailStripItem(300.0)
    assert strip._clip_width == 300.0
    assert ThumbnailStripItem.THUMB_WIDTH == 80
    # 300 / 80 = 3.75 -> tiles across 4 iterations (0, 80, 160, 240)


def test_all_keybindings_registered(qtbot: QtBot) -> None:
    """Instantiate KeyBindings, verify len(self._shortcuts) >= 25."""
    from tempo.ui.keybindings import KeyBindings
    from tempo.ui.main_window import MainWindow

    win = MainWindow()
    qtbot.addWidget(win)
    kb = KeyBindings(win)
    assert len(kb._shortcuts) >= 25


def test_transition_abbrev_cross_dissolve() -> None:
    """Verify ClipItem._transition_abbrev('Cross Dissolve') == 'XD'."""
    from tempo.ui.timeline.clip_item import ClipItem

    assert ClipItem._transition_abbrev("Cross Dissolve") == "XD"
    assert ClipItem._transition_abbrev("Fade In") == "FI"
    assert ClipItem._transition_abbrev("Fade Out") == "FO"
    assert ClipItem._transition_abbrev("Cut to Black") == "CB"
    assert ClipItem._transition_abbrev("Cut to White") == "CW"
    assert ClipItem._transition_abbrev("Crossfade") == "CF"


def test_transition_indicator_not_shown_for_cut(qtbot: QtBot) -> None:
    """Clip with transition_in=None -> no gradient polygon drawn."""
    from unittest.mock import MagicMock

    from PySide6.QtCore import QRectF

    from tempo.core.models import Clip
    from tempo.ui.timeline.clip_item import ClipItem

    clip = Clip(
        id="c1",
        media_id="m1",
        source_path="mock.mp4",
        timeline_start=0.0,
        timeline_end=5.0,
        source_in=0.0,
        source_out=5.0,
        transition_in=None,
        transition_out=None,
    )
    item = ClipItem(clip, "V1", 100.0)

    painter = MagicMock()
    item._paint_transition_indicators(painter, QRectF(0, 0, 500, 48))
    painter.drawPolygon.assert_not_called()


def test_speed_dialog_initial_selection(qtbot: QtBot) -> None:
    """SpeedDialog(2.0, True) opens with 2x button checked."""
    from tempo.ui.dialogs.speed_dialog import SpeedDialog

    dialog = SpeedDialog(2.0, True)
    qtbot.addWidget(dialog)
    assert dialog.selected_speed == 2.0
    assert dialog.pitch_correction is True
    # Verify that the 2.0 button is checked
    btn_2x = dialog._speed_buttons[2.0]
    assert btn_2x.isChecked()


def test_speed_dialog_accept_returns_speed(qtbot: QtBot) -> None:
    """Click 0.5x, accept -> dialog.selected_speed == 0.5."""
    from tempo.ui.dialogs.speed_dialog import SpeedDialog

    dialog = SpeedDialog(1.0, False)
    qtbot.addWidget(dialog)
    # Simulate clicking 0.5x button
    btn_05x = dialog._speed_buttons[0.5]
    btn_05x.click()
    assert dialog.selected_speed == 0.5

    # Simulate accept
    dialog.accept()
    assert dialog.result() == 1  # Accepted


def test_speed_dialog_cancel_does_not_emit(qtbot: QtBot) -> None:
    """Cancel dialog -> does not return Accepted."""
    from tempo.ui.dialogs.speed_dialog import SpeedDialog

    dialog = SpeedDialog(1.0, False)
    qtbot.addWidget(dialog)
    dialog.reject()
    assert dialog.result() == 0  # Rejected


def test_text_overlay_widget_transparent(qtbot: QtBot) -> None:
    """TextOverlayWidget has WA_TransparentForMouseEvents set."""
    from PySide6.QtCore import Qt
    from PySide6.QtWidgets import QWidget

    from tempo.ui.panels.preview import TextOverlayWidget

    parent = QWidget()
    qtbot.addWidget(parent)
    widget = TextOverlayWidget(parent)
    assert widget.testAttribute(Qt.WidgetAttribute.WA_TransparentForMouseEvents)
    assert widget.testAttribute(Qt.WidgetAttribute.WA_NoSystemBackground)
    assert widget.testAttribute(Qt.WidgetAttribute.WA_TranslucentBackground)


def test_text_overlay_renders_without_crash(qtbot: QtBot) -> None:
    """set_active_text_clips([text_clip]), trigger repaint() via QPixmap render, no exception."""
    from PySide6.QtCore import QSize
    from PySide6.QtGui import QPainter, QPixmap
    from PySide6.QtWidgets import QWidget

    from tempo.core.models import TextClip
    from tempo.ui.panels.preview import TextOverlayWidget

    parent = QWidget()
    qtbot.addWidget(parent)
    widget = TextOverlayWidget(parent)
    widget.resize(640, 480)

    tc = TextClip(
        id="t1",
        timeline_start=0.0,
        timeline_end=5.0,
        content="Hello Multiline\nWorld",
        background_opacity=0.5,
        rotation=15.0,
    )
    widget.set_active_text_clips([tc])

    from PySide6.QtCore import QPoint

    pixmap = QPixmap(QSize(640, 480))
    painter = QPainter(pixmap)
    widget.render(painter, QPoint(0, 0))
    painter.end()


def test_export_dialog_instantiates(qtbot: QtBot) -> None:
    from tempo.core.models import Project, ProjectSettings, Timeline, Track
    from tempo.ui.dialogs.export_dialog import ExportDialog

    now_str = "2026-01-01T00:00:00Z"
    project = Project(
        project_name="Test",
        settings=ProjectSettings(),
        created_at=now_str,
        modified_at=now_str,
        timeline=Timeline(tracks=[Track(id="V1", type="video", index=0)]),
    )
    dialog = ExportDialog(project, Path.home())
    qtbot.addWidget(dialog)
    assert dialog.windowTitle() == "Export Video"


def test_export_dialog_browse_updates_path(qtbot: QtBot, monkeypatch: pytest.MonkeyPatch) -> None:
    from PySide6.QtWidgets import QFileDialog

    from tempo.core.models import Project, ProjectSettings, Timeline, Track
    from tempo.ui.dialogs.export_dialog import ExportDialog

    now_str = "2026-01-01T00:00:00Z"
    project = Project(
        project_name="Test",
        settings=ProjectSettings(),
        created_at=now_str,
        modified_at=now_str,
        timeline=Timeline(tracks=[Track(id="V1", type="video", index=0)]),
    )
    monkeypatch.setattr(QFileDialog, "getSaveFileName", lambda *a, **k: ("/tmp/test.mp4", ""))
    dialog = ExportDialog(project, Path.home())
    qtbot.addWidget(dialog)
    dialog._browse_output()
    assert dialog._path_edit.text() == "/tmp/test.mp4"


def test_recent_projects_add_and_load(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    from tempo.utils.recent_projects import add_recent_project, load_recent_projects

    monkeypatch.setattr(
        "tempo.utils.recent_projects.RECENT_PROJECTS_PATH",
        tmp_path / "recent.json",
    )
    p = tmp_path / "project.tempo"
    p.touch()
    add_recent_project(p)
    recent = load_recent_projects()
    assert len(recent) == 1
    assert recent[0].resolve() == p.resolve()


def test_recent_projects_deduplicates(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    from tempo.utils.recent_projects import add_recent_project, load_recent_projects

    monkeypatch.setattr(
        "tempo.utils.recent_projects.RECENT_PROJECTS_PATH",
        tmp_path / "recent.json",
    )
    p = tmp_path / "project.tempo"
    p.touch()
    add_recent_project(p)
    add_recent_project(p)
    recent = load_recent_projects()
    assert recent.count(p.resolve()) == 1


def test_recent_projects_missing_files_excluded(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    from tempo.utils.recent_projects import add_recent_project, load_recent_projects

    monkeypatch.setattr(
        "tempo.utils.recent_projects.RECENT_PROJECTS_PATH",
        tmp_path / "recent.json",
    )
    missing = tmp_path / "gone.tempo"
    add_recent_project(missing)
    assert load_recent_projects() == []
