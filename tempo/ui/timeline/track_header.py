"""Track header widget displaying labels on the left of timeline canvas."""

from __future__ import annotations

from typing import TYPE_CHECKING

from PySide6.QtCore import QRect, Qt
from PySide6.QtGui import QColor, QPainter, QPaintEvent, QPen
from PySide6.QtWidgets import QWidget

from tempo.ui.theme import (
    TRACK_HEADER_WIDTH,
    TRACK_HEIGHT,
    TRACKS,
    get_color,
    get_font,
)

if TYPE_CHECKING:
    from tempo.ui.timeline.lower_timeline import LowerTimeline

# Map TRACKS to their respective design system keys
TRACK_COLOR_KEYS = {
    "TX": "track_tx",
    "V3": "track_v3",
    "V2": "track_v2",
    "V1": "track_v1",
    "A1": "track_a1",
    "A2": "track_a2",
    "A3": "track_a3",
}


class TrackHeader(QWidget):
    """Left column labels with matching track row color keys."""

    def __init__(self, lower_timeline: LowerTimeline, parent: QWidget | None = None) -> None:
        """Initialize the TrackHeader.

        Args:
            lower_timeline: The main scrollable view to sync scroll position.
            parent: Optional parent widget.
        """
        super().__init__(parent)
        self._lower_timeline = lower_timeline
        self.setFixedWidth(TRACK_HEADER_WIDTH)

        # Connect scroll synchronizations
        self._lower_timeline.verticalScrollBar().valueChanged.connect(self._on_vertical_scroll)

    def _on_vertical_scroll(self, value: int) -> None:
        """Repaint on scroll."""
        self.update()

    def paintEvent(self, event: QPaintEvent) -> None:  # noqa: N802
        """Draw colored track blocks with text labels and separator lines."""
        painter = QPainter(self)
        painter.setRenderHint(QPainter.RenderHint.Antialiasing)

        # Base panel background
        painter.fillRect(self.rect(), QColor(get_color("panel")))

        scroll_y = self._lower_timeline.verticalScrollBar().value()

        for i, track_name in enumerate(TRACKS):
            y = i * TRACK_HEIGHT - scroll_y

            # Draw row background block
            color_key = TRACK_COLOR_KEYS.get(track_name, "panel")
            bg_color = QColor(get_color(color_key))
            # 40% Opacity (alpha 102 / 255)
            bg_color.setAlpha(102)

            row_rect = QRect(0, y, self.width(), TRACK_HEIGHT)
            painter.fillRect(row_rect, bg_color)

            # Center text label
            painter.setFont(get_font("ui", size=11, bold=True))
            painter.setPen(QColor(get_color("text_secondary")))
            painter.drawText(row_rect, Qt.AlignmentFlag.AlignCenter, track_name)

            # Draw horizontal separator at the bottom of the row
            painter.setPen(QPen(QColor(get_color("border")), 1))
            painter.drawLine(0, y + TRACK_HEIGHT, self.width(), y + TRACK_HEIGHT)
