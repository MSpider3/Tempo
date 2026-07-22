"""Upper timeline overview widget showing zoomed-out project and visible range."""

from __future__ import annotations

from typing import Any

from PySide6.QtCore import QRectF, Qt, Signal
from PySide6.QtGui import QColor, QMouseEvent, QPainter, QPaintEvent, QPen
from PySide6.QtWidgets import QWidget

from tempo.ui.theme import get_color


class UpperTimeline(QWidget):
    """Overview ruler displaying full project duration and viewport indicator."""

    seek_requested = Signal(float)  # Emits proportional time position in seconds

    def __init__(self, parent: QWidget | None = None) -> None:
        """Initialize the UpperTimeline.

        Args:
            parent: Optional parent widget.
        """
        super().__init__(parent)
        self.setFixedHeight(48)

        # Sync states
        self._project_duration: float = 0.0
        self._playhead_time: float = 0.0
        self._scroll_offset: float = 0.0
        self._viewport_width: float = 0.0
        self._pps: float = 100.0
        self._clips: list[Any] = []  # List of clips (to be populated in Week 6)

    def set_project_duration(self, seconds: float) -> None:
        """Update total project duration."""
        self._project_duration = seconds
        self.update()

    def set_playhead(self, seconds: float) -> None:
        """Update current playhead time."""
        self._playhead_time = seconds
        self.update()

    def set_viewport_info(self, scroll_offset: float, viewport_width: float, pps: float) -> None:
        """Sync horizontal scrollbar and zoom changes from LowerTimeline."""
        self._scroll_offset = scroll_offset
        self._viewport_width = viewport_width
        self._pps = pps
        self.update()

    def paintEvent(self, event: QPaintEvent) -> None:  # noqa: N802
        """Paint overview clips, viewport bounds, and playhead position."""
        painter = QPainter(self)
        painter.setRenderHint(QPainter.RenderHint.Antialiasing)

        # 1. Fill Background
        painter.fillRect(self.rect(), QColor(get_color("bg")))

        widget_w = self.width()

        if self._project_duration <= 0.0:
            return

        # 2. Draw clips placeholder colored rectangles (populated in Week 6)
        # For now, let's make sure code layout is ready
        # format: (clip, track_color_key)
        for _clip in self._clips:
            # Future week clip loop logic
            pass

        # 3. Viewport Indicator Rectangle
        total_timeline_pixels = self._pps * self._project_duration
        if total_timeline_pixels > 0:
            ratio_x = self._scroll_offset / total_timeline_pixels
            ratio_w = self._viewport_width / total_timeline_pixels

            indicator_x = ratio_x * widget_w
            indicator_w = ratio_w * widget_w

            # Clamp boundaries
            indicator_w = min(widget_w - indicator_x, indicator_w)

            indicator_rect = QRectF(indicator_x, 2.0, indicator_w, self.height() - 4.0)

            # Draw white border with transparent white fill
            painter.setPen(QPen(QColor("#FFFFFF"), 1))
            # 8% white opacity (alpha 20 / 255)
            painter.setBrush(QColor(255, 255, 255, 20))
            painter.drawRect(indicator_rect)

        # 4. Playhead Marker Line
        playhead_ratio = self._playhead_time / self._project_duration
        playhead_x = playhead_ratio * widget_w

        painter.setPen(QPen(QColor(get_color("playhead")), 1))
        painter.setBrush(Qt.BrushStyle.NoBrush)
        painter.drawLine(int(playhead_x), 0, int(playhead_x), self.height())

    def mousePressEvent(self, event: QMouseEvent) -> None:  # noqa: N802
        """Click to seek playhead proportionally."""
        if event.button() == Qt.MouseButton.LeftButton:
            self._seek_to_mouse(event.position().x())
            event.accept()
        super().mousePressEvent(event)

    def mouseMoveEvent(self, event: QMouseEvent) -> None:  # noqa: N802
        """Drag to scrub playhead proportionally."""
        if event.buttons() & Qt.MouseButton.LeftButton:
            self._seek_to_mouse(event.position().x())
            event.accept()
        super().mouseMoveEvent(event)

    def _seek_to_mouse(self, local_x: float) -> None:
        ratio = max(0.0, min(1.0, local_x / self.width()))
        self.seek_requested.emit(ratio * self._project_duration)
