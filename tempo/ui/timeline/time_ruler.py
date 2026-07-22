"""Time ruler widget aligning with timeline zoom and scroll offsets."""

from __future__ import annotations

from typing import TYPE_CHECKING

from PySide6.QtCore import QRect, Qt
from PySide6.QtGui import QColor, QMouseEvent, QPainter, QPaintEvent, QPen
from PySide6.QtWidgets import QWidget

from tempo.ui.theme import get_color, get_font

if TYPE_CHECKING:
    from tempo.ui.timeline.lower_timeline import LowerTimeline


def format_ruler_time(seconds: float) -> str:
    """Format seconds into MM:SS format for ruler labels."""
    mins = int(seconds) // 60
    secs = int(seconds) % 60
    return f"{mins:02d}:{secs:02d}"


class TimeRuler(QWidget):
    """Draws ticks and timecode labels based on timeline scroll and zoom."""

    def __init__(self, lower_timeline: LowerTimeline, parent: QWidget | None = None) -> None:
        """Initialize the TimeRuler.

        Args:
            lower_timeline: The main timeline editing canvas.
            parent: Optional parent widget.
        """
        super().__init__(parent)
        self._lower_timeline = lower_timeline
        self.setFixedHeight(24)
        self.setMouseTracking(True)
        self._trim_time: float | None = None

        # Connect to lower timeline trim drag feedback signals
        self._lower_timeline.trim_drag_updated.connect(self._on_trim_drag_updated)
        self._lower_timeline.trim_drag_ended.connect(self._on_trim_drag_ended)

    def _on_trim_drag_updated(self, x: float) -> None:
        pps = self._lower_timeline.pixels_per_second
        if pps > 0:
            self._trim_time = x / pps
            self.update()

    def _on_trim_drag_ended(self) -> None:
        self._trim_time = None
        self.update()

    def paintEvent(self, event: QPaintEvent) -> None:  # noqa: N802
        """Paint ticks and time labels dynamically for visible viewport."""
        painter = QPainter(self)
        painter.setRenderHint(QPainter.RenderHint.Antialiasing)

        # Background
        painter.fillRect(self.rect(), QColor(get_color("panel")))

        pps = self._lower_timeline.pixels_per_second
        if pps <= 0:
            return

        # Grid line intervals
        if pps >= 200.0:
            major, minor = 1.0, 0.1
        elif pps >= 50.0:
            major, minor = 5.0, 1.0
        elif pps >= 20.0:
            major, minor = 10.0, 5.0
        else:
            major, minor = 30.0, 10.0

        w = self.width()
        scroll_offset = self._lower_timeline.horizontalScrollBar().value()

        start_x = float(scroll_offset)
        end_x = start_x + w

        start_time = start_x / pps
        end_time = end_x / pps

        # Round down to nearest minor tick
        t = (start_time // minor) * minor
        epsilon = 1e-5

        while t <= end_time:
            x = t * pps - start_x
            is_major = abs(t % major) < epsilon or abs((t % major) - major) < epsilon

            tick_h = 12 if is_major else 6
            tick_y = self.height() - tick_h

            # Draw tick line
            painter.setPen(QPen(QColor(get_color("border")), 1))
            painter.drawLine(int(x), tick_y, int(x), self.height())

            if is_major:
                # Label
                label = format_ruler_time(t)
                painter.setPen(QColor(get_color("text_secondary")))
                painter.setFont(get_font("mono", size=9))
                # Shift text slightly to the right of the tick mark
                painter.drawText(int(x) + 4, 13, label)

            t += minor

        # Render trim feedback timecode if active
        if self._trim_time is not None:
            trim_x = self._trim_time * pps - start_x
            if 0 <= trim_x <= w:
                # Draw vertical line on ruler
                painter.setPen(QPen(QColor(get_color("accent")), 1.5))
                painter.drawLine(int(trim_x), 0, int(trim_x), self.height())

                # Format as SMPTE timecode
                fps_val = getattr(self._lower_timeline, "_fps_settings", 24.0)
                from tempo.utils.timecode import seconds_to_timecode

                tc_str = seconds_to_timecode(self._trim_time, fps_val)

                painter.setFont(get_font("mono", size=9))
                fm = painter.fontMetrics()
                tc_w = fm.horizontalAdvance(tc_str)
                tc_h = fm.height()

                # Draw semi-transparent background box for timecode label
                box_rect = QRect(int(trim_x) + 4, 2, tc_w + 6, tc_h + 2)
                painter.fillRect(box_rect, QColor(0, 0, 0, 180))
                painter.setPen(QColor(get_color("accent")))
                painter.drawText(int(trim_x) + 7, 14, tc_str)

    def mousePressEvent(self, event: QMouseEvent) -> None:  # noqa: N802
        """Click to position playhead."""
        if event.button() == Qt.MouseButton.LeftButton:
            self._scrub(event.position().x())
            event.accept()
        super().mousePressEvent(event)

    def mouseMoveEvent(self, event: QMouseEvent) -> None:  # noqa: N802
        """Drag to scrub playhead."""
        if event.buttons() & Qt.MouseButton.LeftButton:
            self._scrub(event.position().x())
            event.accept()
        super().mouseMoveEvent(event)

    def _scrub(self, local_x: float) -> None:
        scroll_offset = self._lower_timeline.horizontalScrollBar().value()
        x = local_x + scroll_offset
        pps = self._lower_timeline.pixels_per_second
        if pps > 0:
            seconds = max(0.0, x / pps)
            self._lower_timeline.playhead_moved.emit(seconds)
