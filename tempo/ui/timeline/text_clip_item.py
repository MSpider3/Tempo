"""TextClipItem renderer for text/title tracks."""

from __future__ import annotations

from typing import TYPE_CHECKING, Any

from PySide6.QtCore import QObject, QRectF, Qt, Signal
from PySide6.QtGui import QBrush, QColor, QFont, QPainter, QPen
from PySide6.QtWidgets import (
    QGraphicsItem,
    QGraphicsRectItem,
    QStyleOptionGraphicsItem,
    QWidget,
)

from tempo.ui.theme import COLORS, FONTS, RADIUS, TRACK_HEIGHT, TRACKS

if TYPE_CHECKING:
    from tempo.core.models import TextClip


class TextClipItem(QObject, QGraphicsRectItem):
    """Visual representation of a text/title block in the TX track."""

    Z_VALUE = 1
    text_move_committed = Signal(str, float)  # clip_id, new_start

    def __init__(self, text_clip: TextClip, pps: float) -> None:
        """Initialize the TextClipItem.

        Args:
            text_clip: Core TextClip dataclass reference.
            pps: Current pixels per second scale.
        """
        QObject.__init__(self)
        QGraphicsRectItem.__init__(self)
        self._text_clip = text_clip
        self._pps = pps
        self._selected = False
        self._move_active = False

        self.setZValue(self.Z_VALUE)
        self.setFlag(QGraphicsItem.GraphicsItemFlag.ItemIsSelectable, True)
        self.setFlag(QGraphicsItem.GraphicsItemFlag.ItemIsMovable, True)
        self.setAcceptHoverEvents(True)

        self._update_geometry()

    @property
    def clip_id(self) -> str:
        """Get the core TextClip ID."""
        return self._text_clip.id

    def _update_geometry(self) -> None:
        # Text clips always land on the "TX" track (index 0)
        track_index = TRACKS.index("TX")
        x = self._text_clip.timeline_start * self._pps
        y = track_index * TRACK_HEIGHT + 2
        w = (self._text_clip.timeline_end - self._text_clip.timeline_start) * self._pps
        h = TRACK_HEIGHT - 4
        self.setRect(0.0, 0.0, max(w, 4.0), float(h))
        self.setPos(x, y)

    def update_pps(self, pps: float) -> None:
        """Update pixels per second zoom level and refresh drawing bounds."""
        self._pps = pps
        self._update_geometry()

    def set_selected_state(self, selected: bool) -> None:
        """Set visual selection state."""
        self._selected = selected
        self.update()

    def paint(
        self,
        painter: QPainter,
        option: QStyleOptionGraphicsItem,
        widget: QWidget | None = None,
    ) -> None:
        """Render the text clip representation block."""
        rect = self.rect()
        base_color = QColor(COLORS["track_tx"])

        # 1. Purple background, rounded corners
        painter.setBrush(QBrush(base_color))
        painter.setPen(Qt.PenStyle.NoPen)
        painter.drawRoundedRect(rect, float(RADIUS["clip"]), float(RADIUS["clip"]))

        # 2. Selection overlay (light transparent accent blue)
        if self._selected:
            sel_color = QColor(COLORS["accent"])
            sel_color.setAlpha(60)
            painter.setBrush(QBrush(sel_color))
            painter.setPen(QPen(QColor(COLORS["accent"]), 1.5))
            painter.drawRoundedRect(rect, float(RADIUS["clip"]), float(RADIUS["clip"]))

        # 3. "T" icon (small, top-left corner, 12px mono)
        if rect.width() > 16.0:
            painter.setPen(QColor(COLORS["text_secondary"]))
            icon_font = QFont(str(FONTS["mono"]), 10, QFont.Weight.Bold)
            painter.setFont(icon_font)
            painter.drawText(QRectF(4.0, 2.0, 16.0, 16.0), Qt.AlignmentFlag.AlignLeft, "T")

        # 4. Text content preview (truncated, centered, italic, text_primary)
        if rect.width() > 32.0:
            painter.setPen(QColor(COLORS["text_primary"]))
            text_font = QFont(str(FONTS["ui"]), int(FONTS["size_small"]))
            text_font.setItalic(True)
            painter.setFont(text_font)

            # Draw text inside centered bounds
            text_rect = QRectF(16.0, 2.0, rect.width() - 20.0, rect.height() - 4.0)
            painter.drawText(
                text_rect,
                Qt.TextFlag.TextSingleLine | Qt.AlignmentFlag.AlignCenter,
                self._text_clip.content,
            )

        # 5. Outline border
        border_color = QColor(COLORS["accent"] if self._selected else COLORS["border"])
        painter.setBrush(Qt.BrushStyle.NoBrush)
        painter.setPen(QPen(border_color, 1.0))
        painter.drawRoundedRect(
            rect.adjusted(0.5, 0.5, -0.5, -0.5),
            float(RADIUS["clip"]),
            float(RADIUS["clip"]),
        )

    def hoverEnterEvent(self, event: Any) -> None:  # noqa: N802
        self.setCursor(Qt.CursorShape.SizeHorCursor)

    def hoverLeaveEvent(self, event: Any) -> None:  # noqa: N802
        self.setCursor(Qt.CursorShape.ArrowCursor)

    def mousePressEvent(self, event: Any) -> None:  # noqa: N802
        self._move_active = True
        super().mousePressEvent(event)

    def mouseReleaseEvent(self, event: Any) -> None:  # noqa: N802
        super().mouseReleaseEvent(event)
        if self._move_active:
            from tempo.ui.timeline.lower_timeline import x_to_time

            new_x = self.x() + self.rect().x()
            new_time = max(0.0, x_to_time(new_x, self._pps))
            self.text_move_committed.emit(self._text_clip.id, new_time)
            self._move_active = False
