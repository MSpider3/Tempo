"""ClipItem renderer for timeline tracks."""

from __future__ import annotations

from enum import Enum
from pathlib import Path
from typing import TYPE_CHECKING, Any

from PySide6.QtCore import QObject, QPointF, QRectF, Qt, Signal
from PySide6.QtGui import (
    QBrush,
    QColor,
    QFont,
    QFontMetrics,
    QLinearGradient,
    QPainter,
    QPen,
    QPixmap,
    QPolygonF,
)
from PySide6.QtWidgets import (
    QGraphicsItem,
    QGraphicsRectItem,
    QGraphicsSceneHoverEvent,
    QGraphicsSceneMouseEvent,
    QStyleOptionGraphicsItem,
    QWidget,
)

from tempo.ui.theme import COLORS, FONTS, RADIUS, TRACK_HEIGHT, TRACKS, TRIM_HANDLE_WIDTH
from tempo.ui.timeline.waveform_widget import WaveformItem

if TYPE_CHECKING:
    from tempo.core.models import Clip
    from tempo.ui.timeline.timeline_widget import ActiveTool


class TrimEdge(Enum):
    LEFT = "left"
    RIGHT = "right"
    NONE = "none"


class ThumbnailStripItem(QGraphicsItem):
    """Renders a row of thumbnail frames inside a video clip block."""

    THUMB_WIDTH = 80  # px per thumbnail frame
    THUMB_HEIGHT = 44  # px (TRACK_HEIGHT - 4)

    def __init__(self, clip_width: float) -> None:
        super().__init__()
        self._clip_width = clip_width
        self._pixmap: QPixmap | None = None  # single thumbnail for now
        self.setZValue(0)

    def set_thumbnail(self, path: Path) -> None:
        px = QPixmap(str(path))
        if not px.isNull():
            self._pixmap = px.scaled(
                self.THUMB_WIDTH,
                self.THUMB_HEIGHT,
                Qt.AspectRatioMode.KeepAspectRatioByExpanding,
                Qt.TransformationMode.SmoothTransformation,
            )
            self.update()

    def boundingRect(self) -> QRectF:  # noqa: N802
        return QRectF(0.0, 0.0, float(self._clip_width), float(self.THUMB_HEIGHT))

    def paint(
        self,
        painter: QPainter,
        option: QStyleOptionGraphicsItem | None = None,
        widget: QWidget | None = None,
    ) -> None:
        if self._pixmap is None:
            return
        painter.setClipRect(self.boundingRect())
        # Tile the thumbnail across the clip width
        x = 0.0
        while x < self._clip_width:
            painter.drawPixmap(QPointF(x, 0.0), self._pixmap)
            x += float(self.THUMB_WIDTH)

    def set_width(self, width: float) -> None:
        self.prepareGeometryChange()
        self._clip_width = width


class ClipItem(QObject, QGraphicsRectItem):
    """Visual representation of a video or audio clip in the scene."""

    Z_VALUE = 1

    trim_committed = Signal(str, float, float, float, float, float, float, float, float)
    blade_requested = Signal(str, float)

    def __init__(self, clip: Clip, track_id: str, pps: float) -> None:
        """Initialize the ClipItem.

        Args:
            clip: Core Clip dataclass reference.
            track_id: ID of the track containing the clip.
            pps: Current pixels per second scale.
        """
        QObject.__init__(self)
        QGraphicsRectItem.__init__(self)
        self._clip = clip
        self._track_id = track_id
        self._pps = pps
        self._selected = False
        self._waveform_item: WaveformItem | None = None
        self._thumb_strip: ThumbnailStripItem | None = None

        from tempo.ui.timeline.timeline_widget import ActiveTool

        self._active_tool = ActiveTool.SELECT

        self._trim_active = False
        self._trim_edge = TrimEdge.NONE

        self.setZValue(self.Z_VALUE)
        self.setFlag(QGraphicsItem.GraphicsItemFlag.ItemIsSelectable, True)
        self.setFlag(QGraphicsItem.GraphicsItemFlag.ItemIsMovable, False)
        self.setAcceptHoverEvents(True)

        self._update_geometry()

    @property
    def clip_id(self) -> str:
        """Get the core Clip ID."""
        return self._clip.id

    def _get_scene(self) -> Any:
        return self.scene()

    def _update_geometry(self) -> None:
        track_index = TRACKS.index(self._track_id)
        x = self._clip.timeline_start * self._pps
        y = track_index * TRACK_HEIGHT + 2
        w = (self._clip.timeline_end - self._clip.timeline_start) * self._pps
        h = TRACK_HEIGHT - 4
        self.setRect(0.0, 0.0, max(w, 4.0), float(h))
        self.setPos(x, y)

    def update_pps(self, pps: float) -> None:
        """Update pixels per second zoom level and refresh drawing bounds."""
        self._pps = pps
        self._update_geometry()
        if self._waveform_item is not None:
            self._waveform_item.set_size(self.rect().width(), max(1.0, self.rect().height() - 20.0))
        if self._thumb_strip is not None:
            self._thumb_strip.set_width(self.rect().width())

    def set_waveform(self, peaks: list[tuple[float, float]]) -> None:
        """Called by LowerTimeline when waveform data becomes available."""
        if not peaks:
            return
        if self._waveform_item is None:
            h = max(1.0, self.rect().height() - 20.0)  # leave room for label at top
            self._waveform_item = WaveformItem(peaks, self.rect().width(), h)
            self._waveform_item.setParentItem(self)
            self._waveform_item.setPos(0, 18)  # below the filename label
        else:
            self._waveform_item.set_peaks(peaks)
            self._waveform_item.set_size(self.rect().width(), max(1.0, self.rect().height() - 20.0))

    def set_thumbnail(self, path: Path) -> None:
        """Set thumbnail image strip on video clip."""
        if self._thumb_strip is None:
            self._thumb_strip = ThumbnailStripItem(self.rect().width())
            self._thumb_strip.setParentItem(self)
            self._thumb_strip.setPos(0, 2)
        self._thumb_strip.set_thumbnail(path)

    def set_selected_state(self, selected: bool) -> None:
        """Set visual selection state."""
        self._selected = selected
        self.update()

    def update_clip(self, clip: Clip) -> None:
        """Update the clip data and repaint — called when inspector changes a property."""
        self._clip = clip
        self._update_geometry()
        self.update()

    @staticmethod
    def _transition_abbrev(transition_type: str) -> str:
        return {
            "Fade In": "FI",
            "Fade Out": "FO",
            "Cross Dissolve": "XD",
            "Cut to Black": "CB",
            "Cut to White": "CW",
            "Crossfade": "CF",
        }.get(transition_type, "")

    def _paint_transition_indicators(self, painter: QPainter, rect: QRectF) -> None:
        """Draw angled gradient overlays at clip edges for configured transitions."""
        transition_width = min(24.0, rect.width() * 0.25)

        # Transition IN indicator (left edge)
        if self._clip.transition_in is not None and self._clip.transition_in.type != "Cut":
            gradient = QLinearGradient(0, 0, transition_width, 0)
            gradient.setColorAt(0.0, QColor(0, 0, 0, 100))
            gradient.setColorAt(1.0, QColor(0, 0, 0, 0))
            painter.setBrush(QBrush(gradient))
            painter.setPen(Qt.PenStyle.NoPen)
            polygon = QPolygonF(
                [
                    QPointF(0, 0),
                    QPointF(transition_width, 0),
                    QPointF(transition_width * 0.6, rect.height()),
                    QPointF(0, rect.height()),
                ]
            )
            painter.drawPolygon(polygon)
            painter.setPen(QColor(255, 255, 255, 160))
            painter.setFont(QFont(str(FONTS["mono"]), 8))
            painter.drawText(
                QRectF(2, 2, transition_width - 2, 12),
                Qt.AlignmentFlag.AlignLeft,
                self._transition_abbrev(self._clip.transition_in.type),
            )

        # Transition OUT indicator (right edge)
        if self._clip.transition_out is not None and self._clip.transition_out.type != "Cut":
            gradient = QLinearGradient(rect.width() - transition_width, 0, rect.width(), 0)
            gradient.setColorAt(0.0, QColor(0, 0, 0, 0))
            gradient.setColorAt(1.0, QColor(0, 0, 0, 100))
            painter.setBrush(QBrush(gradient))
            painter.setPen(Qt.PenStyle.NoPen)
            polygon = QPolygonF(
                [
                    QPointF(rect.width() - transition_width, 0),
                    QPointF(rect.width(), 0),
                    QPointF(rect.width(), rect.height()),
                    QPointF(rect.width() - transition_width * 0.6, rect.height()),
                ]
            )
            painter.drawPolygon(polygon)
            painter.setPen(QColor(255, 255, 255, 160))
            painter.setFont(QFont(str(FONTS["mono"]), 8))
            x = rect.width() - transition_width
            painter.drawText(
                QRectF(x, 2, transition_width - 2, 12),
                Qt.AlignmentFlag.AlignRight,
                self._transition_abbrev(self._clip.transition_out.type),
            )

    def paint(
        self,
        painter: QPainter,
        option: QStyleOptionGraphicsItem,
        widget: QWidget | None = None,
    ) -> None:
        """Render the clip representation block."""
        rect = self.rect()
        track_colors = {
            "V1": COLORS["track_v1"],
            "V2": COLORS["track_v2"],
            "V3": COLORS["track_v3"],
            "A1": COLORS["track_a1"],
            "A2": COLORS["track_a2"],
            "A3": COLORS["track_a3"],
            "TX": COLORS["track_tx"],
        }
        base_color = QColor(track_colors.get(self._track_id, COLORS["surface"]))

        # 1. Background fill
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

        # 3. Video Thumbnail placeholder block (first 80px) if no thumbnail strip
        if self._track_id.startswith("V") and rect.width() > 20.0 and self._thumb_strip is None:
            thumb_w = min(rect.width() * 0.3, 80.0)
            thumb_rect = QRectF(2.0, 2.0, thumb_w, rect.height() - 4.0)
            thumb_color = base_color.lighter(115)
            painter.setBrush(QBrush(thumb_color))
            painter.setPen(Qt.PenStyle.NoPen)
            painter.drawRect(thumb_rect)

        # 3.5 Transition indicators overlay
        self._paint_transition_indicators(painter, rect)

        # 4. Clip filename label
        if rect.width() > 24.0:
            painter.setPen(QColor(COLORS["text_primary"]))
            label_font = QFont(str(FONTS["ui"]), int(FONTS["size_small"]), QFont.Weight.Bold)
            painter.setFont(label_font)
            label_rect = QRectF(6.0, 4.0, rect.width() - 12.0, 16.0)
            filename = Path(self._clip.source_path).stem
            painter.drawText(label_rect, Qt.TextFlag.TextSingleLine, filename)

        # 5. Clip duration label (bottom right)
        if rect.width() > 48.0:
            painter.setPen(QColor(COLORS["text_secondary"]))
            dur_font = QFont(str(FONTS["mono"]), int(FONTS["size_small"]) - 1)
            painter.setFont(dur_font)
            dur = self._clip.timeline_end - self._clip.timeline_start
            dur_str = f"{dur:.1f}s"
            dur_rect = QRectF(4.0, rect.height() - 16.0, rect.width() - 8.0, 14.0)
            painter.drawText(
                dur_rect,
                Qt.TextFlag.TextSingleLine
                | Qt.AlignmentFlag.AlignRight
                | Qt.AlignmentFlag.AlignBottom,
                dur_str,
            )

        # 6. Speed multiplier badge (amber warnings)
        if self._clip.speed != 1.0 and rect.width() > 32.0:
            badge_text = f"{self._clip.speed}x"
            badge_font = QFont(str(FONTS["mono"]), 9)
            painter.setFont(badge_font)
            badge_color = QColor(COLORS["warning"])
            badge_bg = QColor(0, 0, 0, 120)

            fm = QFontMetrics(badge_font)
            badge_w = fm.horizontalAdvance(badge_text) + 6
            badge_rect = QRectF(rect.width() - float(badge_w) - 3.0, 3.0, float(badge_w), 14.0)

            painter.setBrush(QBrush(badge_bg))
            painter.setPen(Qt.PenStyle.NoPen)
            painter.drawRoundedRect(badge_rect, 2.0, 2.0)

            painter.setPen(badge_color)
            painter.drawText(badge_rect, Qt.AlignmentFlag.AlignCenter, badge_text)

        # 7. Outline border
        border_color = QColor(COLORS["accent"] if self._selected else COLORS["border"])
        painter.setBrush(Qt.BrushStyle.NoBrush)
        painter.setPen(QPen(border_color, 1.0))
        painter.drawRoundedRect(
            rect.adjusted(0.5, 0.5, -0.5, -0.5),
            float(RADIUS["clip"]),
            float(RADIUS["clip"]),
        )

    def set_active_tool(self, tool: ActiveTool) -> None:
        """Set the active editing tool and update cursor state."""
        self._active_tool = tool
        from tempo.ui.timeline.timeline_widget import ActiveTool

        if tool == ActiveTool.TRIM:
            self.setCursor(Qt.CursorShape.SizeHorCursor)
        elif tool == ActiveTool.BLADE:
            self.setCursor(Qt.CursorShape.CrossCursor)
        else:
            self.setCursor(Qt.CursorShape.ArrowCursor)

    def _trim_edge_at(self, local_x: float) -> TrimEdge:
        """Determine if local_x is within a trim handle zone."""
        if local_x <= TRIM_HANDLE_WIDTH:
            return TrimEdge.LEFT
        if local_x >= self.rect().width() - TRIM_HANDLE_WIDTH:
            return TrimEdge.RIGHT
        return TrimEdge.NONE

    def hoverMoveEvent(self, event: QGraphicsSceneHoverEvent) -> None:  # noqa: N802
        from tempo.ui.timeline.timeline_widget import ActiveTool

        if self._active_tool == ActiveTool.SELECT:
            edge = self._trim_edge_at(event.pos().x())
            if edge != TrimEdge.NONE:
                self.setCursor(Qt.CursorShape.SizeHorCursor)
            else:
                self.setCursor(Qt.CursorShape.OpenHandCursor)
        elif self._active_tool == ActiveTool.TRIM:
            self.setCursor(Qt.CursorShape.SizeHorCursor)
        elif self._active_tool == ActiveTool.BLADE:
            self.setCursor(Qt.CursorShape.CrossCursor)
        super().hoverMoveEvent(event)

    def hoverLeaveEvent(self, event: QGraphicsSceneHoverEvent) -> None:  # noqa: N802
        self.setCursor(Qt.CursorShape.ArrowCursor)
        super().hoverLeaveEvent(event)

    def mousePressEvent(self, event: QGraphicsSceneMouseEvent) -> None:  # noqa: N802
        if event.button() != Qt.MouseButton.LeftButton:
            return

        from tempo.ui.timeline.lower_timeline import x_to_time
        from tempo.ui.timeline.timeline_widget import ActiveTool

        if self._active_tool == ActiveTool.BLADE:
            click_time = x_to_time(event.scenePos().x(), self._pps)
            self.blade_requested.emit(self._clip.id, click_time)
            event.accept()
            return

        edge = self._trim_edge_at(event.pos().x())
        if self._active_tool == ActiveTool.TRIM:
            edge = TrimEdge.LEFT if event.pos().x() <= self.rect().width() / 2.0 else TrimEdge.RIGHT

        if edge != TrimEdge.NONE:
            # Begin trim drag — store original values
            self._trim_active = True
            self._trim_edge = edge
            self._trim_original_start = self._clip.timeline_start
            self._trim_original_end = self._clip.timeline_end
            self._trim_original_source_in = self._clip.source_in
            self._trim_original_source_out = self._clip.source_out
            self._trim_start_scene_x = event.scenePos().x()

            scene = self._get_scene()
            if scene and hasattr(scene, "start_trim_feedback"):
                scene.start_trim_feedback(event.scenePos().x())

            event.accept()
        else:
            super().mousePressEvent(event)

    def mouseMoveEvent(self, event: QGraphicsSceneMouseEvent) -> None:  # noqa: N802
        if self._trim_active:
            dx_time = (event.scenePos().x() - self._trim_start_scene_x) / self._pps
            if self._trim_edge == TrimEdge.LEFT:
                new_start = max(0.0, self._trim_original_start + dx_time)
                new_source_in = self._trim_original_source_in + (
                    new_start - self._trim_original_start
                )
                # Clamp: can't trim past source_out - 0.1
                new_start = min(new_start, self._trim_original_end - 0.1)
                new_source_in = max(0.0, new_source_in)
                # Visual-only update — no state mutation yet
                self._preview_trim_left(new_start)

                scene = self._get_scene()
                if scene and hasattr(scene, "update_trim_feedback"):
                    scene.update_trim_feedback(new_start * self._pps)
            else:  # RIGHT
                new_end = max(self._trim_original_start + 0.1, self._trim_original_end + dx_time)
                self._preview_trim_right(new_end)

                scene = self._get_scene()
                if scene and hasattr(scene, "update_trim_feedback"):
                    scene.update_trim_feedback(new_end * self._pps)
            event.accept()
        else:
            super().mouseMoveEvent(event)

    def mouseReleaseEvent(self, event: QGraphicsSceneMouseEvent) -> None:  # noqa: N802
        if self._trim_active:
            dx_time = (event.scenePos().x() - self._trim_start_scene_x) / self._pps
            if self._trim_edge == TrimEdge.LEFT:
                new_start = max(
                    0.0, min(self._trim_original_start + dx_time, self._trim_original_end - 0.1)
                )
                new_source_in = max(
                    0.0, self._trim_original_source_in + (new_start - self._trim_original_start)
                )
                self.trim_committed.emit(
                    self._clip.id,
                    self._trim_original_start,
                    self._trim_original_end,
                    self._trim_original_source_in,
                    self._trim_original_source_out,
                    new_start,
                    self._trim_original_end,
                    new_source_in,
                    self._clip.source_out,
                )
            else:  # RIGHT
                new_end = max(self._trim_original_start + 0.1, self._trim_original_end + dx_time)
                # Clamp to media duration
                media_duration = (
                    self._trim_original_source_out
                    - self._trim_original_source_in
                    + (self._trim_original_end - self._trim_original_start)
                )
                new_end = min(new_end, self._trim_original_start + media_duration)
                new_source_out = self._trim_original_source_out + (
                    new_end - self._trim_original_end
                )
                self.trim_committed.emit(
                    self._clip.id,
                    self._trim_original_start,
                    self._trim_original_end,
                    self._trim_original_source_in,
                    self._trim_original_source_out,
                    self._trim_original_start,
                    new_end,
                    self._trim_original_source_in,
                    new_source_out,
                )

            scene = self._get_scene()
            if scene and hasattr(scene, "_remove_trim_feedback"):
                scene._remove_trim_feedback()

            self._trim_active = False
            event.accept()
        else:
            super().mouseReleaseEvent(event)

    def _preview_trim_left(self, new_start: float) -> None:
        """Visual-only: move left edge right without touching state."""
        dx = (new_start - self._trim_original_start) * self._pps
        new_x = self.x() + dx
        self.setPos(new_x, self.y())
        old_w = (self._clip.timeline_end - self._clip.timeline_start) * self._pps
        self.setRect(0, 0, max(4.0, old_w - dx), self.rect().height())
        self.update()

    def _preview_trim_right(self, new_end: float) -> None:
        """Visual-only: move right edge."""
        new_w = max(4.0, (new_end - self._trim_original_start) * self._pps)
        self.setRect(0, 0, new_w, self.rect().height())
        self.update()
