"""Context-sensitive inspector panel for clip and text clip settings."""

from PySide6.QtCore import Signal
from PySide6.QtWidgets import QLabel, QStackedWidget, QVBoxLayout, QWidget

from tempo.core.models import Clip, MediaItem, TextClip
from tempo.ui.panels.inspector_clip import ClipInspector
from tempo.ui.panels.inspector_text import TextInspector
from tempo.ui.theme import get_color


class _EmptyPage(QWidget):
    """Fallback page shown when no timeline element is selected."""

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        layout = QVBoxLayout(self)
        layout.setContentsMargins(16, 16, 16, 16)

        self.label = QLabel("No clip selected\n\nClick a clip on the\ntimeline to see its options")
        self.label.setStyleSheet(f"color: {get_color('text_secondary')}; font-size: 12px;")
        self.label.setWordWrap(True)
        from PySide6.QtCore import Qt

        self.label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        layout.addWidget(self.label)


class InspectorPanel(QWidget):
    """Context-sensitive inspector. Switches between sub-panels based on selection."""

    # Signals emitted to MainWindow — MainWindow pushes to EditHistory
    speed_change_requested = Signal(str, float)  # clip_id, new_speed
    pitch_correction_changed = Signal(str, bool)  # clip_id, enabled
    transition_in_changed = Signal(str, str, float)  # clip_id, type, duration
    transition_out_changed = Signal(str, str, float)  # clip_id, type, duration
    text_property_changed = Signal(str, dict)  # text_clip_id, {field: value}

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setMinimumWidth(220)

        self._stack = QStackedWidget(self)
        self._empty_page = _EmptyPage()
        self._clip_page = ClipInspector()
        self._text_page = TextInspector()

        self._stack.addWidget(self._empty_page)  # index 0
        self._stack.addWidget(self._clip_page)  # index 1
        self._stack.addWidget(self._text_page)  # index 2

        layout = QVBoxLayout(self)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.addWidget(self._stack)

        # Forward signals from sub-panels
        self._clip_page.speed_change_requested.connect(self.speed_change_requested)
        self._clip_page.pitch_correction_changed.connect(self.pitch_correction_changed)
        self._clip_page.transition_in_changed.connect(self.transition_in_changed)
        self._clip_page.transition_out_changed.connect(self.transition_out_changed)
        self._text_page.text_property_changed.connect(self.text_property_changed)

    def show_empty(self) -> None:
        """Switch stacked page to the empty notice page."""
        self._stack.setCurrentIndex(0)

    def show_clip(self, clip: Clip, media_item: MediaItem | None) -> None:
        """Load and display the properties of a regular audio/video clip."""
        self._clip_page.load(clip, media_item)
        self._stack.setCurrentIndex(1)

    def show_text_clip(self, text_clip: TextClip) -> None:
        """Load and display the properties of a text clip."""
        self._text_page.load(text_clip)
        self._stack.setCurrentIndex(2)
