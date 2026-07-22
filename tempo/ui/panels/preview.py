"""Preview Player panel featuring embedded python-mpv.

Handles video loading, scrubbing, J/K/L shuttle speed, transport controls,
and In/Out point marking.
"""

from __future__ import annotations

import contextlib
import os
from enum import Enum
from typing import TYPE_CHECKING, Any

import mpv
from PySide6.QtCore import QObject, QPoint, QPointF, QRect, QRectF, Qt, Signal
from PySide6.QtGui import (
    QBrush,
    QColor,
    QFont,
    QFontMetrics,
    QMouseEvent,
    QPainter,
    QPaintEvent,
    QResizeEvent,
)
from PySide6.QtWidgets import (
    QHBoxLayout,
    QLabel,
    QPushButton,
    QSlider,
    QVBoxLayout,
    QWidget,
)

from tempo.ui.theme import get_color, get_font
from tempo.utils.timecode import seconds_to_timecode

if TYPE_CHECKING:
    from pathlib import Path

    from tempo.core.models import TextClip

# ---------------------------------------------------------------------------
# MPV Signal Bridge
# ---------------------------------------------------------------------------


class _MPVSignalBridge(QObject):
    """Receives MPV callbacks on internal thread and re-emits them to Qt main thread."""

    time_pos_changed = Signal(float)
    duration_changed = Signal(float)
    pause_changed = Signal(bool)
    eof_reached = Signal()


# ---------------------------------------------------------------------------
# MPV Native Widget
# ---------------------------------------------------------------------------


class MPVWidget(QWidget):
    """Widget wrapper that maps to an embedded MPV player instance."""

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setAttribute(Qt.WidgetAttribute.WA_DontCreateNativeAncestors)
        self.setAttribute(Qt.WidgetAttribute.WA_NativeWindow)
        self.setStyleSheet("background-color: #000000;")
        self._mpv: mpv.MPV | None = None
        self._bridge = _MPVSignalBridge()

    def init_mpv(self) -> None:
        """Call AFTER the widget is shown and has a valid winId."""
        wid = int(self.winId())
        # Detect wayland session at runtime
        vo = "x11" if os.environ.get("WAYLAND_DISPLAY") is None else "libmpv"

        self._mpv = mpv.MPV(
            wid=str(wid),
            vo=vo,
            hwdec="no",  # CPU decode only
            keep_open="yes",  # Stay at end of file
            idle="yes",  # Run when no file is loaded
        )

        # Connect property observers through the Qt main-thread bridge
        def _on_time_pos(name: str, value: float | None) -> None:
            if value is not None:
                self._bridge.time_pos_changed.emit(float(value))

        def _on_duration(name: str, value: float | None) -> None:
            if value is not None:
                self._bridge.duration_changed.emit(float(value))

        def _on_pause(name: str, value: bool | None) -> None:
            if value is not None:
                self._bridge.pause_changed.emit(bool(value))

        self._mpv.observe_property("time-pos", _on_time_pos)
        self._mpv.observe_property("duration", _on_duration)
        self._mpv.observe_property("pause", _on_pause)

    def cleanup(self) -> None:
        """Terminate the MPV process safely."""
        if self._mpv is not None:
            self._mpv.terminate()
            self._mpv = None

    def get_property(self, name: str, default: Any = 0.0) -> Any:
        """Get property value from MPV safely, returning a default on error."""
        if self._mpv is None:
            return default
        try:
            val = self._mpv[name]
            return val if val is not None else default
        except Exception:
            return default


# ---------------------------------------------------------------------------
# Custom Seek Bar QSlider
# ---------------------------------------------------------------------------


class SeekBar(QSlider):
    """Horizontal seek bar with custom drawing representing playback progress."""

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(Qt.Orientation.Horizontal, parent)
        self.setRange(0, 10000)
        self.setFixedHeight(12)
        self.setStyleSheet("background: transparent;")

    def paintEvent(self, event: QPaintEvent) -> None:  # noqa: N802
        """Draw the seekBar with a clean design token style."""
        painter = QPainter(self)
        painter.setRenderHint(QPainter.RenderHint.Antialiasing)

        # Draw grey track (6px height, rounded)
        track_h = 6
        track_y = (self.height() - track_h) // 2
        track_rect = QRect(0, track_y, self.width(), track_h)
        painter.setBrush(QBrush(QColor(get_color("border"))))
        painter.setPen(Qt.PenStyle.NoPen)
        painter.drawRoundedRect(track_rect, 3, 3)

        # Draw filled progress (blue accent, rounded)
        val = self.value()
        max_val = self.maximum() or 1
        percentage = val / max_val
        fill_w = int(self.width() * percentage)
        if fill_w > 0:
            fill_rect = QRect(0, track_y, fill_w, track_h)
            painter.setBrush(QBrush(QColor(get_color("accent"))))
            painter.drawRoundedRect(fill_rect, 3, 3)

        # Draw slider handle circle
        handle_r = 6
        handle_x = int(self.width() * percentage)
        handle_x = max(handle_r, min(self.width() - handle_r, handle_x))
        handle_y = self.height() // 2
        painter.setBrush(QBrush(QColor(get_color("accent"))))
        painter.drawEllipse(QPoint(handle_x, handle_y), handle_r, handle_r)

    def mousePressEvent(self, event: QMouseEvent) -> None:  # noqa: N802
        """Seek directly to click location on track."""
        if event.button() == Qt.MouseButton.LeftButton:
            val = self.minimum() + (
                (self.maximum() - self.minimum()) * event.position().x() / self.width()
            )
            self.setValue(int(val))
            self.sliderMoved.emit(self.value())
            event.accept()
        super().mousePressEvent(event)


# ---------------------------------------------------------------------------
# Visual In/Out point bar
# ---------------------------------------------------------------------------


class InOutBar(QWidget):
    """Sub-bar that highlights sequence selection limits."""

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setFixedHeight(4)
        self.in_point: float | None = None
        self.out_point: float | None = None
        self.duration: float = 0.0

    def set_points(self, in_pt: float | None, out_pt: float | None, duration: float) -> None:
        """Update points and redraw bar."""
        self.in_point = in_pt
        self.out_point = out_pt
        self.duration = duration
        self.update()

    def paintEvent(self, event: QPaintEvent) -> None:  # noqa: N802
        """Draw In/Out range highlighting."""
        painter = QPainter(self)
        # Base track
        painter.fillRect(self.rect(), QColor(get_color("border")))

        if self.duration <= 0.0:
            return

        w = self.width()

        # Highlight region between In and Out points
        if self.in_point is not None and self.out_point is not None:
            in_x = int((self.in_point / self.duration) * w)
            out_x = int((self.out_point / self.duration) * w)
            x1, x2 = min(in_x, out_x), max(in_x, out_x)
            painter.fillRect(
                QRect(x1, 0, x2 - x1, self.height()),
                QColor(74, 158, 255, 38),  # 15% opacity accent
            )

        # Draw green In point line
        if self.in_point is not None:
            in_x = int((self.in_point / self.duration) * w)
            painter.fillRect(
                QRect(in_x - 1, 0, 2, self.height()),
                QColor(get_color("success")),
            )

        # Draw red Out point line
        if self.out_point is not None:
            out_x = int((self.out_point / self.duration) * w)
            painter.fillRect(
                QRect(out_x - 1, 0, 2, self.height()),
                QColor(get_color("error")),
            )


# ---------------------------------------------------------------------------
# Shuttle State / Controller
# ---------------------------------------------------------------------------


class ShuttleState(Enum):
    """Representing current J/K/L shuttle direction."""

    STOPPED = "stopped"
    FORWARD = "forward"
    REVERSE = "reverse"


class ShuttleController:
    """Manages playback shuttle speed steps (J/K/L keys)."""

    def __init__(self, preview: PreviewPanel) -> None:
        self._preview = preview
        self.state: ShuttleState = ShuttleState.STOPPED
        self.speed: float = 1.0

    def on_j_pressed(self) -> None:
        """Rewind speed steps: 1x -> 2x -> 4x -> 8x reverse."""
        if self.state in (ShuttleState.STOPPED, ShuttleState.FORWARD):
            self.state = ShuttleState.REVERSE
            self.speed = 1.0
        elif self.state == ShuttleState.REVERSE:
            if self.speed < 1.0:
                self.speed = 1.0
            elif self.speed == 1.0:
                self.speed = 2.0
            elif self.speed == 2.0:
                self.speed = 4.0
            else:
                self.speed = 8.0
        self._apply()

    def on_k_pressed(self) -> None:
        """Stop player shuttle."""
        self.state = ShuttleState.STOPPED
        self.speed = 1.0
        self._apply()

    def on_l_pressed(self) -> None:
        """Forward speed steps: 1x -> 2x -> 4x -> 8x forward."""
        if self.state in (ShuttleState.STOPPED, ShuttleState.REVERSE):
            self.state = ShuttleState.FORWARD
            self.speed = 1.0
        elif self.state == ShuttleState.FORWARD:
            if self.speed < 1.0:
                self.speed = 1.0
            elif self.speed == 1.0:
                self.speed = 2.0
            elif self.speed == 2.0:
                self.speed = 4.0
            else:
                self.speed = 8.0
        self._apply()

    def on_k_plus_j_held(self) -> None:
        """Hold slow rewind (0.25x)."""
        self.state = ShuttleState.REVERSE
        self.speed = 0.25
        self._apply()

    def on_k_plus_l_held(self) -> None:
        """Hold slow forward (0.25x)."""
        self.state = ShuttleState.FORWARD
        self.speed = 0.25
        self._apply()

    def _apply(self) -> None:
        if self.state == ShuttleState.STOPPED:
            self._preview.pause()
            self._preview.set_speed(1.0)
        elif self.state == ShuttleState.FORWARD:
            self._preview.set_speed(self.speed)
            self._preview.play()
        elif self.state == ShuttleState.REVERSE:
            self._preview.set_speed(-self.speed)
            self._preview.play()


# ---------------------------------------------------------------------------
# Text Overlay Widget
# ---------------------------------------------------------------------------


class TextOverlayWidget(QWidget):
    """Transparent overlay drawn on top of the MPV video widget.

    Renders TextClip properties using QPainter.
    """

    def __init__(self, parent: QWidget) -> None:
        super().__init__(parent)
        self.setAttribute(Qt.WidgetAttribute.WA_TransparentForMouseEvents)
        self.setAttribute(Qt.WidgetAttribute.WA_NoSystemBackground)
        self.setAttribute(Qt.WidgetAttribute.WA_TranslucentBackground)
        self.setStyleSheet("background: transparent;")
        self._text_clips: list[TextClip] = []  # clips active at current time
        self._current_time: float = 0.0

    def set_active_text_clips(self, clips: list[TextClip]) -> None:
        """Called by PreviewPanel when playhead position changes."""
        self._text_clips = clips
        self.update()  # trigger repaint

    def paintEvent(self, event: QPaintEvent) -> None:  # noqa: N802
        if not self._text_clips:
            return
        painter = QPainter(self)
        painter.setRenderHint(QPainter.RenderHint.Antialiasing)
        w, h = self.width(), self.height()
        for tc in self._text_clips:
            self._draw_text_clip(painter, tc, w, h)
        painter.end()

    def _draw_text_clip(self, painter: QPainter, tc: TextClip, w: int, h: int) -> None:
        # Font
        font = QFont(tc.font_family, tc.font_size)
        font.setBold(tc.bold)
        font.setItalic(tc.italic)
        font.setUnderline(tc.underline)
        painter.setFont(font)

        # Measure text
        fm = QFontMetrics(font)
        lines = tc.content.split("\n")
        text_w = max((fm.horizontalAdvance(line) for line in lines), default=0)
        text_h = len(lines) * fm.height()

        # Position: percentage of widget size
        cx = (tc.position_x / 100.0) * w
        cy = (tc.position_y / 100.0) * h
        x = cx - text_w / 2
        y = cy - text_h / 2

        # Rotation
        if tc.rotation != 0.0:
            painter.save()
            painter.translate(cx, cy)
            painter.rotate(tc.rotation)
            painter.translate(-cx, -cy)

        # Background box
        if tc.background_opacity > 0.0:
            bg = QColor(tc.background_color)
            alpha = (
                tc.background_opacity / 100.0
                if tc.background_opacity > 1.0
                else tc.background_opacity
            )
            bg.setAlphaF(alpha)
            padding = 6
            bg_rect = QRectF(x - padding, y - padding, text_w + padding * 2, text_h + padding * 2)
            painter.fillRect(bg_rect, bg)

        # Text color
        painter.setPen(QColor(tc.font_color))

        # Multiline text support
        for line_idx, line in enumerate(lines):
            line_y = y + fm.ascent() + line_idx * fm.height()
            line_w = fm.horizontalAdvance(line)
            if tc.alignment == "center":
                line_x = cx - line_w / 2
            elif tc.alignment == "right":
                line_x = cx + text_w / 2 - line_w
            else:
                line_x = x
            painter.drawText(QPointF(line_x, line_y), line)

        if tc.rotation != 0.0:
            painter.restore()


# ---------------------------------------------------------------------------
# Preview Panel
# ---------------------------------------------------------------------------


class PreviewPanel(QWidget):
    """The central player panel with transport controls, slider, and OSD metadata."""

    position_changed = Signal(float)
    duration_changed = Signal(float)
    is_playing_changed = Signal(bool)

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setMinimumWidth(400)
        self._fps = 30.0

        # Selection markers
        self.in_point: float | None = None
        self.out_point: float | None = None

        self._build_ui()
        self._setup_connections()

        self.shuttle = ShuttleController(self)

        self._overlay = TextOverlayWidget(self.mpv_widget)
        self._overlay.resize(self.mpv_widget.size())
        self._overlay.raise_()

    def _build_ui(self) -> None:
        root = QVBoxLayout(self)
        root.setContentsMargins(0, 0, 0, 0)
        root.setSpacing(0)

        # 1. MPV Video Canvas
        self.mpv_widget = MPVWidget(self)
        self.mpv_widget.setMinimumHeight(240)
        root.addWidget(self.mpv_widget, stretch=1)

        # 2. Seek Bar
        self.seek_bar = SeekBar(self)
        root.addWidget(self.seek_bar)

        # 3. In/Out points selection display bar
        self.in_out_bar = InOutBar(self)
        root.addWidget(self.in_out_bar)

        # 4. Transport Bar
        transport_widget = QWidget(self)
        transport_widget.setFixedHeight(44)
        transport_widget.setStyleSheet(
            f"background-color: {get_color('panel')};border-top: 1px solid {get_color('border')};"
        )
        transport_layout = QHBoxLayout(transport_widget)
        transport_layout.setContentsMargins(12, 0, 12, 0)

        # Current timecode (left)
        self.timecode_label = QLabel("00:00:00:00", self)
        self.timecode_label.setFont(get_font("mono", size=12))
        transport_layout.addWidget(self.timecode_label)
        transport_layout.addStretch()

        # Controls container (center)
        controls_layout = QHBoxLayout()
        controls_layout.setSpacing(4)

        # Buttons configuration: (Text symbol, callback)
        buttons_config = [
            ("|◄", self.seek_to_start),
            ("◄◄", lambda: self.step_seconds(-5.0)),
            ("▶", self.toggle_play_pause),
            ("►►", lambda: self.step_seconds(5.0)),
            ("►|", self.seek_to_end),
        ]

        self._buttons: list[QPushButton] = []
        for text, cb in buttons_config:
            btn = QPushButton(text, self)
            btn.setFixedSize(32, 32)
            btn.setFont(get_font("mono", size=13))
            btn.setStyleSheet(
                "QPushButton {"
                "  background: transparent;"
                "  border: none;"
                "}"
                "QPushButton:hover {"
                f"  background-color: {get_color('surface')};"
                "}"
            )
            btn.clicked.connect(cb)
            controls_layout.addWidget(btn)
            self._buttons.append(btn)

        self._play_button = self._buttons[2]  # Keep handle to toggle symbol

        transport_layout.addLayout(controls_layout)
        transport_layout.addStretch()

        # Total duration (right)
        self.duration_label = QLabel("00:00:00:00", self)
        self.duration_label.setFont(get_font("mono", size=12))
        self.duration_label.setStyleSheet(f"color: {get_color('text_secondary')};")
        transport_layout.addWidget(self.duration_label)

        root.addWidget(transport_widget)

    def _setup_connections(self) -> None:
        """Connect bridge signals to UI slots using QueuedConnection."""
        bridge = self.mpv_widget._bridge
        bridge.time_pos_changed.connect(
            self._on_bridge_position,
            Qt.ConnectionType.QueuedConnection,
        )
        bridge.duration_changed.connect(
            self._on_bridge_duration,
            Qt.ConnectionType.QueuedConnection,
        )
        bridge.pause_changed.connect(
            self._on_bridge_pause,
            Qt.ConnectionType.QueuedConnection,
        )

        # Seek bar scrubbing
        self.seek_bar.sliderMoved.connect(self._on_slider_scrub)

    # ── MPV Properties Safe Fetchers ────────────────────────────────────────

    def _get_property(self, name: str, default: Any = 0.0) -> Any:
        return self.mpv_widget.get_property(name, default)

    @property
    def position(self) -> float:
        """Get current position in seconds."""
        return float(self._get_property("time-pos", 0.0))

    @property
    def duration(self) -> float:
        """Get total duration in seconds."""
        return float(self._get_property("duration", 0.0))

    @property
    def is_playing(self) -> bool:
        """Check if player is playing (not paused)."""
        paused = self._get_property("pause", True)
        return not paused

    # ── Public API ───────────────────────────────────────────────────────────

    def load_media(self, path: Path) -> None:
        """Load a media file. Do not auto-play."""
        if self.mpv_widget._mpv is not None:
            # Clear selection markers on new media load
            self.clear_in_point()
            self.clear_out_point()

            self.mpv_widget._mpv.command("loadfile", str(path.resolve()))
            self.mpv_widget._mpv.pause = True

    def play(self) -> None:
        """Start media playback."""
        if self.mpv_widget._mpv is not None:
            self.mpv_widget._mpv.pause = False

    def pause(self) -> None:
        """Pause media playback."""
        if self.mpv_widget._mpv is not None:
            self.mpv_widget._mpv.pause = True

    def toggle_play_pause(self) -> None:
        """Toggle play/pause status."""
        if self.is_playing:
            self.pause()
        else:
            self.play()

    def seek(self, seconds: float) -> None:
        """Seek player to absolute time in seconds."""
        if self.mpv_widget._mpv is not None:
            # Clamp value to duration
            dur = self.duration
            target = max(0.0, min(dur, seconds))
            self.mpv_widget._mpv.command("seek", target, "absolute")

    def seek_to_start(self) -> None:
        """Seek to beginning of media."""
        self.seek(0.0)

    def seek_to_end(self) -> None:
        """Seek to end of media."""
        self.seek(self.duration)

    def step_frames(self, count: int) -> None:
        """Seek forward/backward frame by frame."""
        if self.mpv_widget._mpv is not None:
            if count > 0:
                for _ in range(count):
                    self.mpv_widget._mpv.command("frame-step")
            elif count < 0:
                for _ in range(-count):
                    self.mpv_widget._mpv.command("frame-back-step")

    def step_seconds(self, count: float) -> None:
        """Seek relative by duration seconds."""
        if self.mpv_widget._mpv is not None:
            self.mpv_widget._mpv.command("seek", count, "relative")

    def set_speed(self, speed: float) -> None:
        """Set playback rate speed multiplier."""
        if self.mpv_widget._mpv is not None:
            with contextlib.suppress(Exception):
                self.mpv_widget._mpv.speed = speed

    # ── In/Out markers ───────────────────────────────────────────────────────

    def set_in_point(self) -> None:
        """Set In point selection threshold."""
        self.in_point = self.position
        self._update_in_out_bar()

    def set_out_point(self) -> None:
        """Set Out point selection threshold."""
        self.out_point = self.position
        self._update_in_out_bar()

    def clear_in_point(self) -> None:
        """Remove In point threshold."""
        self.in_point = None
        self._update_in_out_bar()

    def clear_out_point(self) -> None:
        """Remove Out point threshold."""
        self.out_point = None
        self._update_in_out_bar()

    def _update_in_out_bar(self) -> None:
        self.in_out_bar.set_points(self.in_point, self.out_point, self.duration)

    # ── Slots ────────────────────────────────────────────────────────────────

    def _on_bridge_position(self, seconds: float) -> None:
        """Update slider value and label when playhead moves."""
        self.position_changed.emit(seconds)

        # Sync seek bar value only if user is not actively dragging the handle
        if not self.seek_bar.isSliderDown():
            dur = self.duration
            if dur > 0:
                permille = int((seconds / dur) * 10000)
                self.seek_bar.setValue(permille)

        # Update timecode label
        tc = seconds_to_timecode(seconds, self._fps)
        self.timecode_label.setText(tc)

    def _on_bridge_duration(self, seconds: float) -> None:
        """Store duration and update duration labels."""
        self.duration_changed.emit(seconds)
        tc = seconds_to_timecode(seconds, self._fps)
        self.duration_label.setText(tc)
        self._update_in_out_bar()

    def _on_bridge_pause(self, paused: bool) -> None:
        """Sync play/pause button symbol and emit state change."""
        if paused:
            self._play_button.setText("▶")
        else:
            self._play_button.setText("⏸")
        self.is_playing_changed.emit(not paused)

    def _on_slider_scrub(self, permille: int) -> None:
        """Seek player when user drags seek bar slider."""
        dur = self.duration
        if dur > 0:
            target = (permille / 10000.0) * dur
            self.seek(target)

    def resizeEvent(self, event: QResizeEvent) -> None:  # noqa: N802
        super().resizeEvent(event)
        if hasattr(self, "_overlay"):
            self._overlay.setGeometry(self.mpv_widget.geometry())

    def update_text_overlays(self, clips: list[TextClip]) -> None:
        """Update text overlays overlaying the player."""
        if hasattr(self, "_overlay"):
            self._overlay.set_active_text_clips(clips)
