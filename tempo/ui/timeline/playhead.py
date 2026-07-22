"""Playhead graphics item for Timeline editing canvas."""

from __future__ import annotations

from PySide6.QtCore import QPointF, QRectF, Qt
from PySide6.QtGui import QBrush, QColor, QPainter, QPen, QPolygonF
from PySide6.QtWidgets import (
    QGraphicsItem,
    QStyleOptionGraphicsItem,
    QWidget,
)

from tempo.ui.theme import get_color


class PlayheadItem(QGraphicsItem):
    """Playhead indicator drawn as a vertical line with a top handle."""

    def __init__(self, height: float) -> None:
        """Initialize the PlayheadItem.

        Args:
            height: Total height of the timeline track area.
        """
        super().__init__()
        self._height = height
        self.setZValue(1000)  # Always render on top of clip graphics items
        self.setFlag(QGraphicsItem.GraphicsItemFlag.ItemIsMovable, False)

    def boundingRect(self) -> QRectF:  # noqa: N802
        """Define boundaries for redraw region."""
        return QRectF(-4.0, 0.0, 8.0, self._height)

    def paint(
        self,
        painter: QPainter,
        option: QStyleOptionGraphicsItem,
        widget: QWidget | None = None,
    ) -> None:
        """Draw playhead line and top triangle handle."""
        painter.save()
        painter.setRenderHint(QPainter.RenderHint.Antialiasing)

        # Draw 2px wide vertical line
        pen = QPen(QColor(get_color("playhead")), 2)
        painter.setPen(pen)
        painter.drawLine(0, 0, 0, int(self._height))

        # Draw downward-pointing triangle at the top (8px wide, 6px tall)
        painter.setPen(Qt.PenStyle.NoPen)
        painter.setBrush(QBrush(QColor(get_color("playhead"))))

        triangle = QPolygonF(
            [
                QPointF(-4.0, 0.0),
                QPointF(4.0, 0.0),
                QPointF(0.0, 6.0),
            ]
        )
        painter.drawPolygon(triangle)

        painter.restore()
