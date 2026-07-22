"""Lower timeline editing canvas view and scene with drag/drop and selection."""

from __future__ import annotations

from typing import TYPE_CHECKING

from PySide6.QtCore import QEvent, QObject, QPointF, QRect, QRectF, Qt, Signal
from PySide6.QtGui import (
    QBrush,
    QColor,
    QDragEnterEvent,
    QDragMoveEvent,
    QDropEvent,
    QMouseEvent,
    QPainter,
    QPen,
    QResizeEvent,
    QTransform,
    QWheelEvent,
)
from PySide6.QtWidgets import (
    QGraphicsLineItem,
    QGraphicsRectItem,
    QGraphicsScene,
    QGraphicsSceneMouseEvent,
    QGraphicsView,
    QWidget,
)

from tempo.core.timeline import track_id_for_drop
from tempo.ui.theme import COLORS, TRACK_HEIGHT, TRACKS, get_color
from tempo.ui.timeline.clip_item import ClipItem
from tempo.ui.timeline.playhead import PlayheadItem
from tempo.ui.timeline.text_clip_item import TextClipItem

if TYPE_CHECKING:
    from tempo.core.models import Clip, MediaItem, Project
    from tempo.ui.timeline.timeline_widget import ActiveTool

# ---------------------------------------------------------------------------
# Coordinate Conversions
# ---------------------------------------------------------------------------


def time_to_x(seconds: float, pps: float) -> float:
    """Convert time in seconds to X coordinate in pixels."""
    return seconds * pps


def x_to_time(x: float, pps: float) -> float:
    """Convert X coordinate in pixels to time in seconds."""
    return x / pps


# ---------------------------------------------------------------------------
# Timeline Scene
# ---------------------------------------------------------------------------


class TimelineScene(QGraphicsScene):
    """Graphics scene representing the tracks and clips canvas."""

    selection_changed = Signal(list)  # Emits list of selected clip IDs
    clip_move_committed = Signal(str, float, str)  # Emits (clip_id, new_time, track_id)
    trim_committed = Signal(str, float, float, float, float, float, float, float, float)
    blade_requested = Signal(str, float)
    text_clip_creation_requested = Signal(float)  # timeline_start in seconds
    text_move_committed = Signal(str, float)  # clip_id, new_start

    def mouseDoubleClickEvent(self, event: QGraphicsSceneMouseEvent) -> None:  # noqa: N802
        item = self.itemAt(event.scenePos(), QTransform())
        if item is None or not isinstance(item, (ClipItem, TextClipItem)):
            track_id = track_id_for_drop(event.scenePos().y())
            if track_id == "TX":
                view = self.parent_view()
                pps = view.pixels_per_second if view is not None else 100.0
                click_time = x_to_time(event.scenePos().x(), pps)
                self.text_clip_creation_requested.emit(click_time)
                event.accept()
                return
        super().mouseDoubleClickEvent(event)

    def __init__(self, parent: QObject | None = None) -> None:
        super().__init__(parent)
        self._dragging = False
        self._drag_clip_id: str | None = None
        self._drag_start_scene_x: float = 0.0
        self._drag_original_x: dict[str, float] = {}
        self._selected_ids: set[str] = set()
        from tempo.ui.timeline.timeline_widget import ActiveTool

        self._active_tool = ActiveTool.SELECT
        self._trim_line: QGraphicsLineItem | None = None

    def parent_view(self) -> LowerTimeline | None:
        """Helper to get parent LowerTimeline view safely."""
        p = self.parent()
        return p if isinstance(p, LowerTimeline) else None

    def drawBackground(self, painter: QPainter, rect: QRectF | QRect) -> None:  # noqa: N802
        """Draw track row backgrounds, row separators, and grid lines."""
        painter.save()

        # 1. Fill entire scene background
        painter.fillRect(rect, QColor(get_color("panel")))

        view = self.parent_view()
        pps = view.pixels_per_second if view is not None else 100.0

        # 2 & 3. Draw alternating tracks and horizontal borders
        for i in range(len(TRACKS)):
            y = i * TRACK_HEIGHT
            # Only draw visible track rows
            if y + TRACK_HEIGHT < rect.top() or y > rect.bottom():
                continue

            # Base background alternating
            base_color = QColor(get_color("surface" if i % 2 == 1 else "bg"))
            # Video tracks slightly lighter, audio tracks slightly darker
            color = base_color.darker(108) if i >= 4 else base_color.lighter(108)

            row_rect = QRectF(rect.left(), y, rect.width(), TRACK_HEIGHT)
            painter.fillRect(row_rect, color)

            # Horizontal track separator at the bottom of the row
            painter.setPen(QPen(QColor(get_color("border")), 1))
            painter.drawLine(
                int(rect.left()), y + TRACK_HEIGHT, int(rect.right()), y + TRACK_HEIGHT
            )

        # 4. Draw vertical grid ticks
        if pps > 0:
            if pps >= 200.0:
                major, minor = 1.0, 0.1
            elif pps >= 50.0:
                major, minor = 5.0, 1.0
            elif pps >= 20.0:
                major, minor = 10.0, 5.0
            else:
                major, minor = 30.0, 10.0

            start_time = max(0.0, rect.left() / pps)
            end_time = rect.right() / pps

            # Align start time to minor tick
            t = (start_time // minor) * minor
            epsilon = 1e-5

            while t <= end_time:
                x = t * pps
                is_major = abs(t % major) < epsilon or abs((t % major) - major) < epsilon

                if is_major:
                    # 60% opacity border
                    color = QColor(get_color("border"))
                    color.setAlpha(153)
                else:
                    # 25% opacity border
                    color = QColor(get_color("border"))
                    color.setAlpha(64)

                painter.setPen(QPen(color, 1))
                painter.drawLine(int(x), int(rect.top()), int(x), int(rect.bottom()))
                t += minor

        painter.restore()

    def start_trim_feedback(self, x: float) -> None:
        """Create a thin vertical line feedback item for trimming."""
        self._remove_trim_feedback()
        from PySide6.QtWidgets import QGraphicsLineItem

        h = len(TRACKS) * TRACK_HEIGHT
        self._trim_line = QGraphicsLineItem(x, 0, x, h)
        self._trim_line.setZValue(999)
        self._trim_line.setPen(QPen(QColor(COLORS["accent"]), 1.5, Qt.PenStyle.DashLine))
        self.addItem(self._trim_line)

        view = self.parent_view()
        if view is not None:
            view.trim_drag_updated.emit(x)

    def update_trim_feedback(self, x: float) -> None:
        """Update position of the vertical line feedback item."""
        if self._trim_line is not None:
            h = len(TRACKS) * TRACK_HEIGHT
            self._trim_line.setLine(x, 0, x, h)
            view = self.parent_view()
            if view is not None:
                view.trim_drag_updated.emit(x)

    def _remove_trim_feedback(self) -> None:
        """Remove the trim line and clear ruler display."""
        if self._trim_line is not None:
            self.removeItem(self._trim_line)
            self._trim_line = None
            view = self.parent_view()
            if view is not None:
                view.trim_drag_ended.emit()

    def _on_tool_changed(self, tool: ActiveTool) -> None:
        """Propagate active tool mode to all clip items and cursor views."""
        self._active_tool = tool
        view = self.parent_view()
        if view is not None:
            for item in view._clip_items.values():
                if hasattr(item, "set_active_tool"):
                    item.set_active_tool(tool)
            from tempo.ui.timeline.timeline_widget import ActiveTool

            if tool == ActiveTool.BLADE:
                view.setCursor(Qt.CursorShape.CrossCursor)
            elif tool == ActiveTool.TRIM:
                view.setCursor(Qt.CursorShape.SizeHorCursor)
            else:
                view.setCursor(Qt.CursorShape.ArrowCursor)

    def mousePressEvent(self, event: QGraphicsSceneMouseEvent) -> None:  # noqa: N802
        """Handle clip item selection and drag-move start."""
        item = self.itemAt(event.scenePos(), QTransform())
        view = self.parent_view()

        from tempo.ui.timeline.clip_item import TrimEdge
        from tempo.ui.timeline.timeline_widget import ActiveTool

        # Determine if we should delegate directly to the item's
        # mousePressEvent (e.g. for trim or blade)
        delegate_to_item = False
        if self._active_tool == ActiveTool.BLADE or self._active_tool == ActiveTool.TRIM:
            delegate_to_item = True
        elif self._active_tool == ActiveTool.SELECT and isinstance(item, ClipItem):
            local_pos = item.mapFromScene(event.scenePos())
            if item._trim_edge_at(local_pos.x()) != TrimEdge.NONE:
                delegate_to_item = True

        if delegate_to_item:
            super().mousePressEvent(event)
            return

        if isinstance(item, (ClipItem, TextClipItem)) and view is not None:
            # Control multi-select modifier check
            if not (event.modifiers() & Qt.KeyboardModifier.ControlModifier):
                self._deselect_all()

            item.set_selected_state(True)
            self._selected_ids.add(item.clip_id)
            self.selection_changed.emit(list(self._selected_ids))

            # Initialise drag move tracking variables (bypassing raw ItemIsMovable)
            self._dragging = True
            self._drag_clip_id = item.clip_id
            self._drag_start_scene_x = event.scenePos().x()

            # Store original X positions of all selected clips
            self._drag_original_x.clear()
            for cid in self._selected_ids:
                citem = view._clip_items.get(cid)
                if citem is not None:
                    self._drag_original_x[cid] = citem.x()
            event.accept()
        else:
            self._deselect_all()
            super().mousePressEvent(event)

    def mouseMoveEvent(self, event: QGraphicsSceneMouseEvent) -> None:  # noqa: N802
        """Handle visual moving offset on active clip items."""
        view = self.parent_view()
        if self._dragging and self._drag_clip_id and view is not None:
            dx = event.scenePos().x() - self._drag_start_scene_x
            for cid in self._selected_ids:
                citem = view._clip_items.get(cid)
                if citem is not None:
                    # Update local X visually
                    citem.setX(self._drag_original_x[cid] + dx)
            event.accept()
        else:
            super().mouseMoveEvent(event)

    def mouseReleaseEvent(self, event: QGraphicsSceneMouseEvent) -> None:  # noqa: N802
        """Commit final clip locations through EditHistory state loop."""
        view = self.parent_view()
        if self._dragging and self._drag_clip_id and view is not None:
            for cid in list(self._selected_ids):
                citem = view._clip_items.get(cid)
                if citem is not None:
                    new_x = citem.x()
                    new_time = max(0.0, x_to_time(new_x, view.pixels_per_second))
                    new_track = track_id_for_drop(event.scenePos().y())
                    self.clip_move_committed.emit(cid, new_time, new_track)

            self._dragging = False
            self._drag_clip_id = None
            event.accept()
        else:
            super().mouseReleaseEvent(event)

    def _deselect_all(self) -> None:
        """Deselect all clips."""
        view = self.parent_view()
        if view is not None:
            for item in view._clip_items.values():
                item.set_selected_state(False)
        self._selected_ids.clear()
        self.selection_changed.emit([])


# ---------------------------------------------------------------------------
# Lower Timeline Graphics View
# ---------------------------------------------------------------------------


class LowerTimeline(QGraphicsView):
    """Main horizontal scrollable editing view for tracks."""

    playhead_moved = Signal(float)  # Emits current time seconds
    zoom_changed = Signal(float)  # Emits new PPS Zoom
    clip_drop_requested = Signal(str, float, str)  # (media_id, drop_time, track_id)
    trim_drag_updated = Signal(float)  # Emits scene X coordinate during trim drag
    trim_drag_ended = Signal()

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setAcceptDrops(True)
        self.setMouseTracking(True)

        self._scene = TimelineScene(self)
        self.setScene(self._scene)

        # Configure display settings
        self.setAlignment(Qt.AlignmentFlag.AlignLeft | Qt.AlignmentFlag.AlignTop)
        self.setHorizontalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAlwaysOn)
        self.setVerticalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAlwaysOff)
        self.setRenderHint(QPainter.RenderHint.Antialiasing, False)

        # Core navigation properties
        self._pps: float = 100.0
        self._duration: float = 0.0
        self._playhead_time: float = 0.0
        self._playhead_x: float = 0.0
        self._is_playing: bool = False

        from tempo.ui.timeline.timeline_widget import ActiveTool

        self._active_tool = ActiveTool.SELECT
        self._blade_hover_line: QGraphicsLineItem | None = None
        self._fps_settings = 24.0

        # Drag/drop ghost indicator, clip items pool, and caches
        self._drop_ghost: QGraphicsRectItem | None = None
        self._clip_items: dict[str, ClipItem | TextClipItem] = {}
        self._waveform_cache: dict[str, list[tuple[float, float]]] = {}
        self._media_lookup: dict[str, MediaItem] = {}

        # Add Playhead
        h = len(TRACKS) * TRACK_HEIGHT
        self._playhead_item = PlayheadItem(float(h))
        self._scene.addItem(self._playhead_item)

        self.update_scene_rect()

    # ── PPS Zoom Property ────────────────────────────────────────────────────

    @property
    def pixels_per_second(self) -> float:
        """Get the current time zoom level."""
        return self._pps

    @pixels_per_second.setter
    def pixels_per_second(self, val: float) -> None:
        """Set the time zoom level and refresh layout."""
        self._set_pps(val)

    def _set_pps(self, val: float) -> None:
        self._pps = max(10.0, min(500.0, val))
        # Update each clip item scaling factor
        for item in self._clip_items.values():
            item.update_pps(self._pps)

        self.update_scene_rect()
        # Reposition playhead
        self._playhead_x = time_to_x(self._playhead_time, self._pps)
        self._playhead_item.setX(self._playhead_x)
        self.zoom_changed.emit(self._pps)
        self.viewport().update()

    # ── Scene Management & Rebuild ───────────────────────────────────────────

    def update_scene_rect(self) -> None:
        """Recalculate total scene bounding area."""
        w = max(
            self._duration * self._pps + 2000.0,
            float(self.viewport().width()),
        )
        h = float(len(TRACKS) * TRACK_HEIGHT)
        self.setSceneRect(0.0, 0.0, w, h)

    def set_duration(self, seconds: float) -> None:
        """Update timeline project duration."""
        self._duration = seconds
        self.update_scene_rect()

    def load_waveform(self, media_id: str, peaks: list[tuple[float, float]]) -> None:
        """Store waveform peaks in cache and push to active ClipItems."""
        self._waveform_cache[media_id] = peaks
        for item in self._clip_items.values():
            if isinstance(item, ClipItem) and item._clip.media_id == media_id:
                item.set_waveform(peaks)

    def set_media_lookup(self, lookup: dict[str, MediaItem]) -> None:
        """Update media lookup dict and apply thumbnails/waveforms."""
        self._media_lookup = lookup
        for item in self._clip_items.values():
            if isinstance(item, ClipItem):
                media = self._media_lookup.get(item._clip.media_id)
                if media and media.thumbnail_path and item._track_id.startswith("V"):
                    from pathlib import Path

                    item.set_thumbnail(Path(media.thumbnail_path))

    def update_clip_item(self, clip_id: str, clip: Clip) -> None:
        """Update single ClipItem by ID without full scene rebuild."""
        item = self._clip_items.get(clip_id)
        if isinstance(item, ClipItem):
            item.update_clip(clip)
            self.update_scene_rect()

    def rebuild_from_project(self, project: Project) -> None:
        """Completely clear and recreate all ClipItems representing project state."""
        # 1. Clear existing items from scene
        for item in self._clip_items.values():
            self._scene.removeItem(item)
        self._clip_items.clear()

        # Save settings fps
        self._fps_settings = project.settings.framerate

        # 2. Add ClipItem elements
        for track in project.timeline.tracks:
            for clip in track.clips:
                item = ClipItem(clip, track.id, self._pps)
                item.set_active_tool(self._active_tool)
                self._scene.addItem(item)
                item.trim_committed.connect(self._scene.trim_committed)
                item.blade_requested.connect(self._scene.blade_requested)
                self._clip_items[clip.id] = item

                # Load waveform if available in cache
                media = self._media_lookup.get(clip.media_id)
                has_audio = track.id.startswith("A") or (media is not None and media.has_audio)
                if has_audio:
                    peaks = self._waveform_cache.get(clip.media_id)
                    if peaks is not None:
                        item.set_waveform(peaks)

                # Load thumbnail if available for video tracks
                if media and media.thumbnail_path and track.id.startswith("V"):
                    from pathlib import Path

                    item.set_thumbnail(Path(media.thumbnail_path))

        # 3. Add TextClipItem elements
        for text_clip in project.timeline.text_clips:
            item = TextClipItem(text_clip, self._pps)
            self._scene.addItem(item)
            item.text_move_committed.connect(self._scene.text_move_committed)
            self._clip_items[text_clip.id] = item

        # 4. Sync duration and size boundaries
        all_ends = [c.timeline_end for t in project.timeline.tracks for c in t.clips]
        max_end = max(all_ends, default=0.0)
        self._duration = max_end
        self.update_scene_rect()

    # ── Playhead Functions ───────────────────────────────────────────────────

    def set_playhead(self, seconds: float) -> None:
        """Update playhead time and handle playback auto-scrolling."""
        self._playhead_time = seconds
        x = time_to_x(seconds, self._pps)
        self._playhead_x = x
        self._playhead_item.setX(x)

        # Auto-scroll when playhead leaves active screen range
        if self._is_playing:
            vp_width = self.viewport().width()
            scroll_x = self.horizontalScrollBar().value()
            if x < scroll_x or x > scroll_x + vp_width * 0.85:
                self.horizontalScrollBar().setValue(int(x - vp_width * 0.10))

    def set_playing_state(self, playing: bool) -> None:
        """Set playing mode status for scroll tracking."""
        self._is_playing = playing

    # ── Drag and Drop Handlers ───────────────────────────────────────────────

    def dragEnterEvent(self, event: QDragEnterEvent) -> None:  # noqa: N802
        if event.mimeData().hasFormat("application/tempo-media-id"):
            event.acceptProposedAction()
        else:
            event.ignore()

    def dragMoveEvent(self, event: QDragMoveEvent) -> None:  # noqa: N802
        if event.mimeData().hasFormat("application/tempo-media-id"):
            event.acceptProposedAction()
            scene_pos = self.mapToScene(event.position().toPoint())

            # Read duration from payload to resize ghost accurately
            dur_bytes = event.mimeData().data("application/tempo-media-duration")
            try:
                duration = float(bytes(dur_bytes.data()).decode("utf-8"))
            except Exception:
                duration = 5.0  # Default fallback duration

            self._update_drop_ghost(scene_pos, duration)
        else:
            event.ignore()

    def dropEvent(self, event: QDropEvent) -> None:  # noqa: N802
        if event.mimeData().hasFormat("application/tempo-media-id"):
            media_id = bytes(event.mimeData().data("application/tempo-media-id").data()).decode(
                "utf-8"
            )
            scene_pos = self.mapToScene(event.position().toPoint())
            drop_time = max(0.0, x_to_time(scene_pos.x(), self._pps))
            drop_track = track_id_for_drop(scene_pos.y())

            self.clip_drop_requested.emit(media_id, drop_time, drop_track)
            self._remove_drop_ghost()
            event.acceptProposedAction()

    def _update_drop_ghost(self, scene_pos: QPointF, duration: float) -> None:
        """Show semi-transparent bounding outline where clip will land."""
        track_id = track_id_for_drop(scene_pos.y())
        track_index = TRACKS.index(track_id)

        # Coordinate snaps
        x = max(0.0, x_to_time(scene_pos.x(), self._pps)) * self._pps
        y = track_index * TRACK_HEIGHT + 2
        w = duration * self._pps
        h = TRACK_HEIGHT - 4

        if self._drop_ghost is None:
            self._drop_ghost = QGraphicsRectItem()
            self._scene.addItem(self._drop_ghost)

        self._drop_ghost.setRect(0.0, 0.0, max(w, 4.0), float(h))
        self._drop_ghost.setPos(x, y)

        # Translucent visual style Snaps Y
        color = QColor(COLORS.get("accent", "#4A9EFF"))
        color.setAlpha(102)  # 40% Snapped opacity

        self._drop_ghost.setBrush(QBrush(color))
        self._drop_ghost.setPen(QPen(QColor(COLORS.get("accent")), 1.5, Qt.PenStyle.DashLine))
        self._drop_ghost.setZValue(2)

    def _remove_drop_ghost(self) -> None:
        if self._drop_ghost is not None:
            self._scene.removeItem(self._drop_ghost)
            self._drop_ghost = None

    # ── Event Handlers ───────────────────────────────────────────────────────

    def resizeEvent(self, event: QResizeEvent) -> None:  # noqa: N802
        super().resizeEvent(event)
        self.update_scene_rect()

    def wheelEvent(self, event: QWheelEvent) -> None:  # noqa: N802
        """Handle Alt+Scroll shortcut zooming."""
        if event.modifiers() & Qt.KeyboardModifier.AltModifier:
            delta = event.angleDelta().y()
            factor = 1.15 if delta > 0 else 1.0 / 1.15
            new_pps = max(10.0, min(500.0, self._pps * factor))

            # Zoom centered on mouse pointer X coordinate
            mouse_scene_x = self.mapToScene(event.position().toPoint()).x()
            mouse_time = x_to_time(mouse_scene_x, self._pps)

            self._set_pps(new_pps)

            new_x = time_to_x(mouse_time, new_pps)
            self.centerOn(new_x, self.sceneRect().height() / 2.0)
            event.accept()
        else:
            super().wheelEvent(event)

    @property
    def selected_clip_ids(self) -> set[str]:
        """Return the current selection from TimelineScene."""
        return self._scene._selected_ids

    def restore_selection(self, clip_ids: set[str]) -> None:
        """Restore selection for IDs that still exist."""
        self._scene._selected_ids.clear()
        for cid in clip_ids:
            item = self._clip_items.get(cid)
            if item is not None:
                item.set_selected_state(True)
                self._scene._selected_ids.add(cid)
        self._scene.selection_changed.emit(list(self._scene._selected_ids))

    def set_active_tool(self, tool: ActiveTool) -> None:
        """Set active editing tool."""
        self._active_tool = tool
        self._scene._on_tool_changed(tool)

    def mouseMoveEvent(self, event: QMouseEvent) -> None:  # noqa: N802
        from tempo.ui.timeline.timeline_widget import ActiveTool

        if getattr(self, "_active_tool", ActiveTool.SELECT) == ActiveTool.BLADE:
            scene_pos = self.mapToScene(event.position().toPoint())
            self._update_blade_hover_feedback(scene_pos.x())
        else:
            self._remove_blade_hover_feedback()
        super().mouseMoveEvent(event)

    def leaveEvent(self, event: QEvent) -> None:  # noqa: N802
        self._remove_blade_hover_feedback()
        super().leaveEvent(event)

    def _update_blade_hover_feedback(self, x: float) -> None:
        if self._blade_hover_line is None:
            from PySide6.QtWidgets import QGraphicsLineItem

            h = len(TRACKS) * TRACK_HEIGHT
            self._blade_hover_line = QGraphicsLineItem(x, 0, x, h)
            self._blade_hover_line.setZValue(998)
            self._blade_hover_line.setPen(
                QPen(QColor(COLORS["warning"]), 1.5, Qt.PenStyle.SolidLine)
            )
            self._scene.addItem(self._blade_hover_line)
        else:
            h = len(TRACKS) * TRACK_HEIGHT
            self._blade_hover_line.setLine(x, 0, x, h)

    def _remove_blade_hover_feedback(self) -> None:
        if self._blade_hover_line is not None:
            self._scene.removeItem(self._blade_hover_line)
            self._blade_hover_line = None
