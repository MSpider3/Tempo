"""Properties inspector panel for audio and video timeline clips."""

from pathlib import Path

from PySide6.QtCore import Qt, Signal
from PySide6.QtWidgets import (
    QCheckBox,
    QComboBox,
    QDoubleSpinBox,
    QFrame,
    QGridLayout,
    QHBoxLayout,
    QLabel,
    QPushButton,
    QScrollArea,
    QVBoxLayout,
    QWidget,
)

from tempo.core.models import Clip, MediaItem
from tempo.ui.theme import COLORS

SPEED_STEPS = [0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 2.0, 4.0]
TRANSITION_TYPES = [
    "Cut",
    "Fade In",
    "Fade Out",
    "Cross Dissolve",
    "Cut to Black",
    "Cut to White",
    "Crossfade",
]


def create_section_header(title: str) -> QWidget:
    """Helper to create a stylized section header with a bottom divider line."""
    widget = QWidget()
    vbox = QVBoxLayout(widget)
    vbox.setContentsMargins(0, 8, 0, 4)
    vbox.setSpacing(4)

    label = QLabel(title.upper())
    label.setStyleSheet(
        f"color: {COLORS['text_secondary']}; "
        "font-size: 10px; font-weight: bold; letter-spacing: 1px;"
    )
    vbox.addWidget(label)

    line = QFrame()
    line.setFrameShape(QFrame.Shape.HLine)
    line.setFrameShadow(QFrame.Shadow.Plain)
    line.setStyleSheet(f"background-color: {COLORS['border']}; max-height: 1px; border: none;")
    vbox.addWidget(line)

    return widget


def create_info_row(label_text: str) -> tuple[QWidget, QLabel]:
    """Helper to create a 2-column read-only metadata label row."""
    row = QWidget()
    hbox = QHBoxLayout(row)
    hbox.setContentsMargins(0, 2, 0, 2)

    lbl = QLabel(label_text)
    lbl.setStyleSheet(f"color: {COLORS['text_secondary']}; font-size: 11px;")

    val = QLabel("-")
    val.setStyleSheet(f"color: {COLORS['text_primary']}; font-size: 11px;")
    val.setAlignment(Qt.AlignmentFlag.AlignRight)

    hbox.addWidget(lbl)
    hbox.addWidget(val)
    return row, val


class ClipInspector(QWidget):
    """Sub-panel for inspecting and altering video/audio Clip parameters."""

    speed_change_requested = Signal(str, float)  # clip_id, new_speed
    pitch_correction_changed = Signal(str, bool)  # clip_id, enabled
    transition_in_changed = Signal(str, str, float)  # clip_id, type, duration
    transition_out_changed = Signal(str, str, float)  # clip_id, type, duration

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self._clip_id: str | None = None
        self._loading = False

        # Main scrollable canvas container
        scroll = QScrollArea(self)
        scroll.setWidgetResizable(True)
        scroll.setStyleSheet("QScrollArea { border: none; background: transparent; }")

        content = QWidget()
        content.setObjectName("content")
        content.setStyleSheet(f"QWidget#content {{ background-color: {COLORS['bg']}; }}")
        scroll.setWidget(content)

        main_layout = QVBoxLayout(self)
        main_layout.setContentsMargins(0, 0, 0, 0)
        main_layout.addWidget(scroll)

        layout = QVBoxLayout(content)
        layout.setContentsMargins(12, 12, 12, 12)
        layout.setSpacing(12)

        # ─── SECTION 1: CLIP INFO ───
        layout.addWidget(create_section_header("CLIP INFO"))

        row_name, self._lbl_name = create_info_row("Name:")
        row_dur, self._lbl_duration = create_info_row("Duration:")
        row_in, self._lbl_in = create_info_row("In:")
        row_out, self._lbl_out = create_info_row("Out:")

        layout.addWidget(row_name)
        layout.addWidget(row_dur)
        layout.addWidget(row_in)
        layout.addWidget(row_out)

        # ─── SECTION 2: SPEED ───
        layout.addWidget(create_section_header("SPEED"))

        # Speed steps 4x2 grid
        grid_widget = QWidget()
        grid = QGridLayout(grid_widget)
        grid.setContentsMargins(0, 4, 0, 4)
        grid.setSpacing(6)

        self._speed_buttons: dict[float, QPushButton] = {}
        for idx, s in enumerate(SPEED_STEPS):
            r = idx // 4
            c = idx % 4
            btn = QPushButton(f"{s}×")  # noqa: RUF001
            btn.setFixedSize(44, 24)
            btn.setCheckable(True)
            btn.clicked.connect(lambda checked, speed=s: self._on_speed_clicked(speed))
            self._speed_buttons[s] = btn
            grid.addWidget(btn, r, c)

        layout.addWidget(grid_widget)

        self._chk_pitch = QCheckBox("Pitch Correction")
        self._chk_pitch.setStyleSheet(f"color: {COLORS['text_primary']}; font-size: 11px;")
        self._chk_pitch.toggled.connect(self._on_pitch_changed)
        layout.addWidget(self._chk_pitch)

        # ─── SECTION 3: TRANSITION IN ───
        layout.addWidget(create_section_header("TRANSITION IN"))

        trans_in_widget = QWidget()
        trans_in_lay = QHBoxLayout(trans_in_widget)
        trans_in_lay.setContentsMargins(0, 2, 0, 2)
        trans_in_lay.setSpacing(6)

        self._trans_in_combo = QComboBox()
        self._trans_in_combo.addItems(TRANSITION_TYPES)
        self._trans_in_combo.currentIndexChanged.connect(self._on_trans_in_changed)

        self._trans_in_dur = QDoubleSpinBox()
        self._trans_in_dur.setRange(0.1, 10.0)
        self._trans_in_dur.setSingleStep(0.1)
        self._trans_in_dur.setSuffix(" s")
        self._trans_in_dur.valueChanged.connect(self._on_trans_in_changed)

        trans_in_lay.addWidget(self._trans_in_combo, 2)
        trans_in_lay.addWidget(self._trans_in_dur, 1)
        layout.addWidget(trans_in_widget)

        # ─── SECTION 4: TRANSITION OUT ───
        layout.addWidget(create_section_header("TRANSITION OUT"))

        trans_out_widget = QWidget()
        trans_out_lay = QHBoxLayout(trans_out_widget)
        trans_out_lay.setContentsMargins(0, 2, 0, 2)
        trans_out_lay.setSpacing(6)

        self._trans_out_combo = QComboBox()
        self._trans_out_combo.addItems(TRANSITION_TYPES)
        self._trans_out_combo.currentIndexChanged.connect(self._on_trans_out_changed)

        self._trans_out_dur = QDoubleSpinBox()
        self._trans_out_dur.setRange(0.1, 10.0)
        self._trans_out_dur.setSingleStep(0.1)
        self._trans_out_dur.setSuffix(" s")
        self._trans_out_dur.valueChanged.connect(self._on_trans_out_changed)

        trans_out_lay.addWidget(self._trans_out_combo, 2)
        trans_out_lay.addWidget(self._trans_out_dur, 1)
        layout.addWidget(trans_out_widget)

        layout.addStretch()

    def _on_speed_clicked(self, speed: float) -> None:
        if self._loading:
            return
        self._update_speed_buttons_visual(speed)
        if self._clip_id:
            self.speed_change_requested.emit(self._clip_id, speed)

    def _update_speed_buttons_visual(self, active_speed: float) -> None:
        for s, btn in self._speed_buttons.items():
            if abs(s - active_speed) < 1e-4:
                btn.setChecked(True)
                btn.setStyleSheet(
                    f"background-color: {COLORS['accent']}; color: #FFFFFF; "
                    "font-size: 10px; border: none; border-radius: 4px; font-weight: bold;"
                )
            else:
                btn.setChecked(False)
                btn.setStyleSheet(
                    f"background-color: {COLORS['surface']}; "
                    f"color: {COLORS['text_secondary']}; "
                    "font-size: 10px; border: none; border-radius: 4px;"
                )

    def _on_pitch_changed(self, checked: bool) -> None:
        if self._loading:
            return
        if self._clip_id:
            self.pitch_correction_changed.emit(self._clip_id, checked)

    def _on_trans_in_changed(self) -> None:
        if self._loading:
            return
        self._check_transition_validation()
        t_type = self._trans_in_combo.currentText()
        self._trans_in_dur.setEnabled(t_type != "Cut")
        if self._clip_id:
            self.transition_in_changed.emit(self._clip_id, t_type, self._trans_in_dur.value())

    def _on_trans_out_changed(self) -> None:
        if self._loading:
            return
        self._check_transition_validation()
        t_type = self._trans_out_combo.currentText()
        self._trans_out_dur.setEnabled(t_type != "Cut")
        if self._clip_id:
            self.transition_out_changed.emit(self._clip_id, t_type, self._trans_out_dur.value())

    def _check_transition_validation(self) -> None:
        # In transition check
        in_type = self._trans_in_combo.currentText()
        if in_type == "Fade Out":
            self._trans_in_combo.setToolTip("Warning: 'Fade Out' is not typical for transition in.")
        else:
            self._trans_in_combo.setToolTip("")

        # Out transition check
        out_type = self._trans_out_combo.currentText()
        if out_type == "Fade In":
            self._trans_out_combo.setToolTip(
                "Warning: 'Fade In' is not typical for transition out."
            )
        else:
            self._trans_out_combo.setToolTip("")

    def load(self, clip: Clip, media_item: MediaItem | None) -> None:
        """Populate the UI fields with values from the given Clip."""
        self._loading = True
        self._clip_id = clip.id

        # 1. Update Clip Info
        name_str = Path(clip.source_path).name
        if len(name_str) > 24:
            name_str = name_str[:21] + "..."
        self._lbl_name.setText(name_str)

        fps = 24.0
        if media_item is not None and media_item.fps is not None and media_item.fps > 0:
            fps = media_item.fps

        from tempo.utils.timecode import seconds_to_timecode

        duration = clip.timeline_end - clip.timeline_start
        self._lbl_duration.setText(seconds_to_timecode(duration, fps))
        self._lbl_in.setText(seconds_to_timecode(clip.timeline_start, fps))
        self._lbl_out.setText(seconds_to_timecode(clip.timeline_end, fps))

        # 2. Update Speed Grid
        self._update_speed_buttons_visual(clip.speed)

        # 3. Update Pitch Correction
        self._chk_pitch.setChecked(clip.pitch_correction)

        # 4. Transitions
        if clip.transition_in is not None:
            self._trans_in_combo.setCurrentText(clip.transition_in.type)
            self._trans_in_dur.setValue(clip.transition_in.duration)
            self._trans_in_dur.setEnabled(clip.transition_in.type != "Cut")
        else:
            self._trans_in_combo.setCurrentText("Cut")
            self._trans_in_dur.setValue(1.0)
            self._trans_in_dur.setEnabled(False)

        if clip.transition_out is not None:
            self._trans_out_combo.setCurrentText(clip.transition_out.type)
            self._trans_out_dur.setValue(clip.transition_out.duration)
            self._trans_out_dur.setEnabled(clip.transition_out.type != "Cut")
        else:
            self._trans_out_combo.setCurrentText("Cut")
            self._trans_out_dur.setValue(1.0)
            self._trans_out_dur.setEnabled(False)

        self._check_transition_validation()
        self._loading = False
