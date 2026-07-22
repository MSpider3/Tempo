"""Media Bin panel — import, display, and manage media assets.

Contains the MediaImportWorker QThread and MediaBinPanel widget.
No Qt imports are used in media/ or utils/ — only here in ui/.
"""

from __future__ import annotations

from pathlib import Path

from PySide6.QtCore import (
    QMimeData,
    QModelIndex,
    QPersistentModelIndex,
    QPoint,
    QRect,
    QSize,
    Qt,
    QThread,
    QTimer,
    QUrl,
    Signal,
)
from PySide6.QtGui import (
    QBrush,
    QColor,
    QDesktopServices,
    QDrag,
    QDragEnterEvent,
    QDropEvent,
    QFont,
    QPainter,
    QPen,
    QPixmap,
)
from PySide6.QtWidgets import (
    QAbstractItemView,
    QFileDialog,
    QHBoxLayout,
    QLabel,
    QListWidget,
    QListWidgetItem,
    QMenu,
    QMessageBox,
    QPushButton,
    QSizePolicy,
    QStyle,
    QStyledItemDelegate,
    QStyleOptionViewItem,
    QVBoxLayout,
    QWidget,
)

from tempo.core.models import MediaItem
from tempo.media.importer import MediaImportError, UnsupportedFormatError, import_media
from tempo.media.proxy import ProxyGenerator
from tempo.media.thumbnail import ThumbnailExtractor
from tempo.ui.theme import SPACING, get_color, get_font
from tempo.utils.cache import CacheManager
from tempo.utils.ffmpeg import check_ffmpeg
from tempo.utils.timecode import seconds_to_timecode

# ---------------------------------------------------------------------------
# Role constants stored in list items
# ---------------------------------------------------------------------------
_ROLE_MEDIA_ITEM = Qt.ItemDataRole.UserRole
_ROLE_THUMB_PATH = Qt.ItemDataRole.UserRole + 1
_ROLE_STATUS = Qt.ItemDataRole.UserRole + 2  # "ready" | "generating" | "missing"
_ROLE_SPIN_ANGLE = Qt.ItemDataRole.UserRole + 3  # float rotation angle for spinner

_THUMB_W = 80
_THUMB_H = 45
_ITEM_HEIGHT = 56

_FILE_FILTER = (
    "Media Files ("
    "*.mp4 *.mov *.mkv *.avi *.webm "
    "*.mp3 *.wav *.aac *.flac *.ogg "
    "*.jpg *.jpeg *.png *.webp"
    ")"
)


# ---------------------------------------------------------------------------
# Background Worker
# ---------------------------------------------------------------------------


class MediaImportWorker(QThread):
    """Background thread that imports media files one by one.

    Emits signals with pure Python/Path data — never touches UI widgets.
    """

    media_imported = Signal(MediaItem)  # ffprobe complete, item ready to display
    thumbnail_ready = Signal(str, Path)  # (media_id, thumb_path)
    proxy_ready = Signal(str, Path)  # (media_id, proxy_path)
    waveform_ready = Signal(str, object)  # (media_id, peaks_list)
    proxy_progress = Signal(str, int)  # (media_id, percent 0-100) — future use
    import_error = Signal(str, str)  # (file_path_str, error_message)

    def __init__(
        self,
        paths: list[Path],
        cache: CacheManager,
        parent: QWidget | None = None,
    ) -> None:
        super().__init__(parent)
        self._paths = paths
        self._cache = cache
        self._thumb_extractor = ThumbnailExtractor()
        self._proxy_generator = ProxyGenerator()

    def run(self) -> None:
        """Import each path sequentially: probe → thumbnail → proxy → waveform."""
        for path in self._paths:
            if self.isInterruptionRequested():
                break
            try:
                media_item = import_media(path, self._cache)
            except (FileNotFoundError, UnsupportedFormatError, MediaImportError) as e:
                self.import_error.emit(str(path), str(e))
                continue

            # 1. Emit basic item immediately so UI shows the filename
            self.media_imported.emit(media_item)

            if self.isInterruptionRequested():
                break

            # 2. Extract thumbnail
            if media_item.type != "audio":
                try:
                    thumb_path = self._thumb_extractor.extract(media_item, self._cache)
                    self.thumbnail_ready.emit(media_item.id, thumb_path)
                except Exception:
                    pass  # Thumbnail failure is non-fatal

            if self.isInterruptionRequested():
                break

            # 3. Generate proxy (only for videos that need one)
            if media_item.type == "video":
                try:
                    proxy_path = self._proxy_generator.generate(media_item, self._cache)
                    # proxy_path equals original_path when no proxy was generated
                    if proxy_path != Path(media_item.original_path):
                        self.proxy_ready.emit(media_item.id, proxy_path)
                except Exception:
                    pass  # Proxy failure is non-fatal

            if self.isInterruptionRequested():
                break

            # 4. Extract waveform (for audio or video with audio)
            if media_item.has_audio or media_item.type == "audio":
                try:
                    from tempo.media.waveform import WaveformExtractor

                    extractor = WaveformExtractor(self._cache)
                    peaks = extractor.extract(media_item)
                    if peaks:
                        self.waveform_ready.emit(media_item.id, peaks)
                except Exception:
                    pass  # Waveform failure is non-fatal


# ---------------------------------------------------------------------------
# Custom Item Delegate
# ---------------------------------------------------------------------------


class _MediaItemDelegate(QStyledItemDelegate):
    """Renders each media bin row: thumbnail | name | duration | status."""

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self._placeholder: QPixmap | None = None

    def _get_placeholder(self) -> QPixmap:
        if self._placeholder is None:
            pm = QPixmap(_THUMB_W, _THUMB_H)
            pm.fill(QColor(get_color("surface")))
            self._placeholder = pm
        return self._placeholder

    def sizeHint(  # noqa: N802
        self,
        option: QStyleOptionViewItem,
        index: QModelIndex | QPersistentModelIndex,
    ) -> QSize:
        return QSize(option.rect.width(), _ITEM_HEIGHT)

    def paint(
        self,
        painter: QPainter,
        option: QStyleOptionViewItem,
        index: QModelIndex | QPersistentModelIndex,
    ) -> None:
        painter.save()
        rect = option.rect
        pad = SPACING["item_padding"]

        # Background
        is_selected = bool(option.state & QStyle.StateFlag.State_Selected)
        if is_selected:
            painter.fillRect(rect, QColor(74, 158, 255, 77))  # accent 30% alpha
        elif bool(option.state & QStyle.StateFlag.State_MouseOver):
            painter.fillRect(rect, QColor(get_color("surface")))

        # Thumbnail
        thumb_path_raw = index.data(_ROLE_THUMB_PATH)
        thumb_path: Path | None = Path(str(thumb_path_raw)) if thumb_path_raw is not None else None
        thumb_pixmap = self._get_placeholder()
        if thumb_path is not None and thumb_path.exists():
            loaded = QPixmap(str(thumb_path))
            if not loaded.isNull():
                thumb_pixmap = loaded.scaled(
                    _THUMB_W,
                    _THUMB_H,
                    Qt.AspectRatioMode.KeepAspectRatio,
                    Qt.TransformationMode.SmoothTransformation,
                )

        thumb_x = rect.x() + pad
        thumb_y = rect.y() + (rect.height() - _THUMB_H) // 2
        painter.drawPixmap(thumb_x, thumb_y, thumb_pixmap)

        # Text area
        text_x = thumb_x + _THUMB_W + pad
        text_w = rect.width() - text_x - 28 - pad  # leave room for status icon

        media_item: MediaItem | None = index.data(_ROLE_MEDIA_ITEM)
        if media_item is not None:
            # Filename (bold, primary)
            name_font = get_font("ui", size=int(get_font("ui").pointSize()), bold=True)
            name_font.setPointSize(13)
            painter.setFont(name_font)
            painter.setPen(QColor(get_color("text_primary")))
            name_rect = QRect(text_x, rect.y() + pad, text_w, 20)
            painter.drawText(
                name_rect,
                Qt.TextFlag.TextSingleLine,
                Path(media_item.original_path).name,
            )

            # Duration (mono, secondary)
            dur_font = get_font("mono", size=11)
            painter.setFont(dur_font)
            painter.setPen(QColor(get_color("text_secondary")))
            if media_item.duration > 0:
                fps = media_item.fps or 30.0
                duration_str = seconds_to_timecode(media_item.duration, fps)
            else:
                duration_str = "Image"
            dur_rect = QRect(text_x, rect.y() + pad + 22, text_w, 18)
            painter.drawText(
                dur_rect,
                Qt.TextFlag.TextSingleLine,
                duration_str,
            )

        # Status icon
        status = index.data(_ROLE_STATUS) or "ready"
        icon_x = rect.right() - 22
        icon_y = rect.y() + (rect.height() - 16) // 2

        if status == "generating":
            angle_raw = index.data(_ROLE_SPIN_ANGLE)
            angle = float(angle_raw) if angle_raw is not None else 0.0
            self._draw_spinner(painter, icon_x, icon_y, angle)
        elif status == "missing":
            self._draw_error_circle(painter, icon_x, icon_y)

        # Bottom separator
        painter.setPen(QPen(QColor(get_color("border")), 1))
        painter.drawLine(rect.left(), rect.bottom(), rect.right(), rect.bottom())

        painter.restore()

    def _draw_spinner(self, painter: QPainter, x: int, y: int, angle: float) -> None:
        painter.save()
        painter.setRenderHint(QPainter.RenderHint.Antialiasing)
        cx, cy = x + 8, y + 8
        painter.translate(cx, cy)
        painter.rotate(angle)
        pen = QPen(QColor(get_color("warning")), 2)
        pen.setCapStyle(Qt.PenCapStyle.RoundCap)
        painter.setPen(pen)
        painter.drawArc(-6, -6, 12, 12, 90 * 16, 270 * 16)
        painter.restore()

    def _draw_error_circle(self, painter: QPainter, x: int, y: int) -> None:
        painter.save()
        painter.setRenderHint(QPainter.RenderHint.Antialiasing)
        painter.setBrush(QBrush(QColor(get_color("error"))))
        painter.setPen(Qt.PenStyle.NoPen)
        painter.drawEllipse(x, y, 16, 16)
        painter.setPen(QPen(QColor("#FFFFFF"), 1.5))
        font = QFont("Arial", 10, QFont.Weight.Bold)
        painter.setFont(font)
        painter.drawText(QRect(x, y, 16, 16), Qt.AlignmentFlag.AlignCenter, "!")
        painter.restore()


class DragListWidget(QListWidget):
    """Subclass of QListWidget that handles dragging media items."""

    def startDrag(self, supported_actions: Qt.DropAction) -> None:  # noqa: N802
        item = self.currentItem()
        if item is None:
            return
        media_item = item.data(_ROLE_MEDIA_ITEM)
        if media_item is None:
            return

        # Create MIME data carrying the media ID and duration
        mime = QMimeData()
        mime.setData("application/tempo-media-id", media_item.id.encode("utf-8"))
        mime.setData(
            "application/tempo-media-duration",
            str(media_item.duration).encode("utf-8"),
        )

        drag = QDrag(self)
        drag.setMimeData(mime)

        # Scale thumbnail pixmap for drag indicator
        thumb_path_raw = item.data(_ROLE_THUMB_PATH)
        if thumb_path_raw:
            pm = QPixmap(str(thumb_path_raw))
            if not pm.isNull():
                drag.setPixmap(pm.scaled(60, 34, Qt.AspectRatioMode.KeepAspectRatio))

        drag.exec(Qt.DropAction.CopyAction)


# ---------------------------------------------------------------------------
# Media Bin Panel
# ---------------------------------------------------------------------------


class MediaBinPanel(QWidget):
    """Left panel containing imported media items."""

    item_double_clicked = Signal(MediaItem)
    media_imported = Signal(MediaItem)
    waveform_ready = Signal(str, object)

    def __init__(
        self,
        cache: CacheManager | None = None,
        parent: QWidget | None = None,
    ) -> None:
        super().__init__(parent)
        self.setMinimumWidth(200)
        self.setSizePolicy(
            QSizePolicy.Policy.Preferred,
            QSizePolicy.Policy.Expanding,
        )
        self.setAcceptDrops(True)

        self._cache = cache or CacheManager()
        self._workers: list[MediaImportWorker] = []
        self._media_items: dict[str, MediaItem] = {}  # id → MediaItem
        self._list_items: dict[str, QListWidgetItem] = {}  # id → list item

        # Spinner animation timer
        self._spin_timer = QTimer(self)
        self._spin_timer.setInterval(30)  # ~33fps
        self._spin_timer.timeout.connect(self._tick_spinners)
        self._spin_angles: dict[str, float] = {}  # media_id → current angle

        self._build_ui()

    # ── Layout ──────────────────────────────────────────────────────────────

    def _build_ui(self) -> None:
        root = QVBoxLayout(self)
        root.setContentsMargins(0, 0, 0, 0)
        root.setSpacing(0)

        # Header
        header = QWidget()
        header.setFixedHeight(36)
        header.setStyleSheet(
            f"background-color: {get_color('panel')};"
            f"border-bottom: 1px solid {get_color('border')};"
        )
        header_layout = QHBoxLayout(header)
        header_layout.setContentsMargins(SPACING["panel_padding"], 0, SPACING["panel_padding"], 0)

        title_label = QLabel("Media")
        title_label.setFont(get_font("ui", bold=True))
        header_layout.addWidget(title_label)
        header_layout.addStretch()
        root.addWidget(header)

        # Search bar
        from PySide6.QtWidgets import QLineEdit

        self._search_bar = QLineEdit()
        self._search_bar.setPlaceholderText("Search media...")
        self._search_bar.setFixedHeight(28)
        self._search_bar.setStyleSheet(f"margin: {SPACING['unit']}px {SPACING['panel_padding']}px;")
        self._search_bar.textChanged.connect(self._on_search_changed)
        root.addWidget(self._search_bar)

        # List widget
        self._list_widget = DragListWidget(self)
        self._list_widget.setItemDelegate(_MediaItemDelegate(self._list_widget))
        self._list_widget.setViewMode(QListWidget.ViewMode.ListMode)
        self._list_widget.setUniformItemSizes(True)
        self._list_widget.setSelectionMode(QAbstractItemView.SelectionMode.ExtendedSelection)
        self._list_widget.setMouseTracking(True)
        self._list_widget.setDragEnabled(True)
        self._list_widget.setDragDropMode(QAbstractItemView.DragDropMode.DragOnly)
        self._list_widget.setStyleSheet("border-left: none; border-right: none; border-radius: 0;")
        self._list_widget.setContextMenuPolicy(Qt.ContextMenuPolicy.CustomContextMenu)
        self._list_widget.customContextMenuRequested.connect(self._show_context_menu)
        self._list_widget.itemDoubleClicked.connect(self._on_item_double_clicked)
        root.addWidget(self._list_widget, stretch=1)

        # Footer
        footer = QWidget()
        footer.setFixedHeight(36)
        footer.setStyleSheet(
            f"background-color: {get_color('panel')};border-top: 1px solid {get_color('border')};"
        )
        footer_layout = QHBoxLayout(footer)
        footer_layout.setContentsMargins(SPACING["panel_padding"], 0, SPACING["panel_padding"], 0)

        self._count_label = QLabel("0 items")
        self._count_label.setFont(get_font("ui", size=11))
        self._count_label.setStyleSheet(f"color: {get_color('text_secondary')};")
        footer_layout.addWidget(self._count_label)
        footer_layout.addStretch()

        self._import_btn = QPushButton("+ Import Media")
        self._import_btn.setProperty("accent", True)
        self._import_btn.setStyleSheet(
            self._import_btn.styleSheet() + f"border-radius: {4}px; padding: 4px 10px;"
        )
        self._import_btn.clicked.connect(self.trigger_import)
        footer_layout.addWidget(self._import_btn)

        root.addWidget(footer)

    # ── Public API ───────────────────────────────────────────────────────────

    def trigger_import(self) -> None:
        """Open a file dialog and start importing selected files."""
        paths_str, _ = QFileDialog.getOpenFileNames(
            self,
            "Import Media",
            "",
            _FILE_FILTER,
        )
        if not paths_str:
            return
        paths = [Path(p) for p in paths_str]
        self._start_import_worker(paths)

    def all_media_items(self) -> list[MediaItem]:
        """Return all currently imported media items."""
        return list(self._media_items.values())

    def refresh(self) -> None:
        """Refresh the media bin items and viewport."""
        self._list_widget.viewport().update()
        self._update_count()

    def stop_all_workers(self) -> None:
        """Request interruption of all running import workers and wait for them."""
        for worker in self._workers:
            if worker.isRunning():
                worker.requestInterruption()
                worker.quit()
                worker.wait(3000)
        self._workers.clear()

    # ── Import Worker ────────────────────────────────────────────────────────

    def _start_import_worker(self, paths: list[Path]) -> None:
        """Create and start a MediaImportWorker for the given paths."""
        # Check ffmpeg is available before starting
        _ffmpeg_ok, ffprobe_ok = check_ffmpeg()
        if not ffprobe_ok:
            QMessageBox.warning(
                self,
                "FFprobe Not Found",
                "ffprobe is not installed or not in PATH.\nInstall FFmpeg to import media.",
            )
            return

        worker = MediaImportWorker(paths, self._cache, parent=None)

        # All cross-thread signal connections MUST use QueuedConnection
        worker.media_imported.connect(
            self._on_media_imported,
            Qt.ConnectionType.QueuedConnection,
        )
        worker.thumbnail_ready.connect(
            self._on_thumbnail_ready,
            Qt.ConnectionType.QueuedConnection,
        )
        worker.proxy_ready.connect(
            self._on_proxy_ready,
            Qt.ConnectionType.QueuedConnection,
        )
        worker.waveform_ready.connect(
            self._on_waveform_ready,
            Qt.ConnectionType.QueuedConnection,
        )
        worker.import_error.connect(
            self._on_import_error,
            Qt.ConnectionType.QueuedConnection,
        )
        worker.finished.connect(
            lambda: self._on_worker_finished(worker),
            Qt.ConnectionType.QueuedConnection,
        )

        self._workers.append(worker)
        worker.start()

    # ── Signal slots ─────────────────────────────────────────────────────────

    def _on_media_imported(self, media_item: MediaItem) -> None:
        """Slot: add new item to the list immediately after ffprobe."""
        self._media_items[media_item.id] = media_item

        item = QListWidgetItem()
        item.setData(_ROLE_MEDIA_ITEM, media_item)
        item.setData(_ROLE_STATUS, "generating")
        item.setData(_ROLE_SPIN_ANGLE, 0.0)
        item.setSizeHint(QSize(self._list_widget.width(), _ITEM_HEIGHT))

        self._list_widget.addItem(item)
        self._list_items[media_item.id] = item
        self._update_count()

        # Start spinner if not already running
        if not self._spin_timer.isActive():
            self._spin_timer.start()
        self._spin_angles[media_item.id] = 0.0
        self.media_imported.emit(media_item)

    def _on_waveform_ready(self, media_id: str, peaks: list[tuple[float, float]]) -> None:
        """Slot: emit waveform_ready signal."""
        self.waveform_ready.emit(media_id, peaks)

    def _on_thumbnail_ready(self, media_id: str, thumb_path: Path) -> None:
        """Slot: update item thumbnail."""
        item = self._list_items.get(media_id)
        if item is None:
            return
        item.setData(_ROLE_THUMB_PATH, str(thumb_path))
        self._list_widget.viewport().update()

    def _on_proxy_ready(self, media_id: str, proxy_path: Path) -> None:
        """Slot: mark item as ready and stop its spinner."""
        item = self._list_items.get(media_id)
        if item is None:
            return
        item.setData(_ROLE_STATUS, "ready")
        self._spin_angles.pop(media_id, None)
        if not self._spin_angles:
            self._spin_timer.stop()
        self._list_widget.viewport().update()

    def _on_import_error(self, file_path: str, error_message: str) -> None:
        """Slot: show import error dialog."""
        QMessageBox.warning(
            self,
            "Import Failed",
            f"Failed to import:\n{file_path}\n\n{error_message}",
        )

    def _on_worker_finished(self, worker: MediaImportWorker) -> None:
        """Slot: clean up finished worker. Mark any still-generating items ready."""
        if worker in self._workers:
            self._workers.remove(worker)

        # Any items still in "generating" state that have no active worker get marked ready
        if not self._workers:
            for _media_id, item in self._list_items.items():
                if item.data(_ROLE_STATUS) == "generating":
                    item.setData(_ROLE_STATUS, "ready")
            self._spin_angles.clear()
            self._spin_timer.stop()
            self._list_widget.viewport().update()

    # ── Spinner animation ────────────────────────────────────────────────────

    def _tick_spinners(self) -> None:
        """Advance all active spinner angles and trigger a repaint."""
        if not self._spin_angles:
            self._spin_timer.stop()
            return
        for media_id in list(self._spin_angles.keys()):
            self._spin_angles[media_id] = (self._spin_angles[media_id] + 8.0) % 360.0
            item = self._list_items.get(media_id)
            if item is not None:
                item.setData(_ROLE_SPIN_ANGLE, self._spin_angles[media_id])
        self._list_widget.viewport().update()

    # ── Search ───────────────────────────────────────────────────────────────

    def _on_search_changed(self, text: str) -> None:
        """Filter visible items by filename substring (case-insensitive)."""
        query = text.lower()
        for i in range(self._list_widget.count()):
            item = self._list_widget.item(i)
            if item is None:
                continue
            media_item: MediaItem | None = item.data(_ROLE_MEDIA_ITEM)
            if media_item is None:
                continue
            name = Path(media_item.original_path).name.lower()
            item.setHidden(bool(query) and query not in name)

    # ── Context Menu ─────────────────────────────────────────────────────────

    def _show_context_menu(self, pos: QPoint) -> None:
        """Show right-click context menu for a media item."""
        item = self._list_widget.itemAt(pos)
        if item is None:
            return
        media_item: MediaItem | None = item.data(_ROLE_MEDIA_ITEM)
        if media_item is None:
            return

        menu = QMenu(self)

        remove_action = menu.addAction("Remove from Project")
        reveal_action = menu.addAction("Reveal in File Manager")
        relink_action = menu.addAction("Re-link Media...")

        chosen = menu.exec(self._list_widget.viewport().mapToGlobal(pos))

        if chosen == remove_action:
            self._remove_item(media_item.id)
        elif chosen == reveal_action:
            folder = Path(media_item.original_path).parent
            QDesktopServices.openUrl(QUrl.fromLocalFile(str(folder)))
        elif chosen == relink_action:
            QMessageBox.information(
                self,
                "Re-link Media",
                "Re-link is not yet implemented.\n(Coming in Phase 4)",
            )

    def _remove_item(self, media_id: str) -> None:
        """Remove a media item from the bin."""
        item = self._list_items.pop(media_id, None)
        self._media_items.pop(media_id, None)
        self._spin_angles.pop(media_id, None)
        if item is not None:
            row = self._list_widget.row(item)
            self._list_widget.takeItem(row)
        self._update_count()

    # ── Drag and Drop ────────────────────────────────────────────────────────

    def dragEnterEvent(self, event: QDragEnterEvent) -> None:  # noqa: N802
        if event.mimeData().hasUrls():
            event.acceptProposedAction()
        else:
            event.ignore()

    def dropEvent(self, event: QDropEvent) -> None:  # noqa: N802
        mime: QMimeData = event.mimeData()
        if not mime.hasUrls():
            event.ignore()
            return
        paths = [Path(url.toLocalFile()) for url in mime.urls() if url.isLocalFile()]
        if paths:
            self._start_import_worker(paths)
        event.acceptProposedAction()

    def _on_item_double_clicked(self, item: QListWidgetItem) -> None:
        """Handle list item double click by emitting MediaItem."""
        media_item = item.data(_ROLE_MEDIA_ITEM)
        if isinstance(media_item, MediaItem):
            self.item_double_clicked.emit(media_item)

    # ── Helpers ──────────────────────────────────────────────────────────────

    def _update_count(self) -> None:
        count = len(self._media_items)
        noun = "item" if count == 1 else "items"
        self._count_label.setText(f"{count} {noun}")
