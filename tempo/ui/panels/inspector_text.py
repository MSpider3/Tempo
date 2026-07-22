"""Properties inspector panel for text overlay clips."""

from PySide6.QtCore import QEvent, Qt, QTimer, Signal
from PySide6.QtGui import QColor, QPainter, QPen
from PySide6.QtWidgets import (
    QButtonGroup,
    QColorDialog,
    QComboBox,
    QFrame,
    QHBoxLayout,
    QLabel,
    QPushButton,
    QScrollArea,
    QSlider,
    QSpinBox,
    QTextEdit,
    QVBoxLayout,
    QWidget,
)

from tempo.core.models import TextClip
from tempo.ui.theme import COLORS


class ColorSwatchButton(QPushButton):
    """A flat button that shows a colored rectangle. Click opens QColorDialog."""

    color_changed = Signal(QColor)

    def __init__(self, initial_color: QColor, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self._color = initial_color
        self.setFixedHeight(24)
        self.clicked.connect(self._pick_color)

    def set_color(self, color: QColor) -> None:
        """Programmatically update the displayed color."""
        self._color = color
        self.update()

    def _pick_color(self) -> None:
        color = QColorDialog.getColor(
            self._color,
            self,
            "Select Color",
            QColorDialog.ColorDialogOption.ShowAlphaChannel,
        )
        if color.isValid():
            self._color = color
            self.update()
            self.color_changed.emit(color)

    def paintEvent(self, event: QEvent) -> None:  # noqa: N802
        painter = QPainter(self)
        # Draw the selected color
        painter.fillRect(self.rect(), self._color)
        # Draw border
        painter.setPen(QPen(QColor(COLORS["border"]), 1))
        painter.drawRect(self.rect().adjusted(0, 0, -1, -1))


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


class TextInspector(QWidget):
    """Sub-panel for inspecting and altering TextClip parameters."""

    text_property_changed = Signal(str, dict)  # text_clip_id, {field: value}

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self._clip_id: str | None = None
        self._loading = False

        # Debounce Timer for text box updates
        self._debounce_timer = QTimer(self)
        self._debounce_timer.setSingleShot(True)
        self._debounce_timer.setInterval(400)
        self._debounce_timer.timeout.connect(self._on_text_debounced)

        # Scroll Area Setup
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

        # ─── SECTION 1: TEXT CONTENT ───
        layout.addWidget(create_section_header("TEXT CONTENT"))

        self._text_edit = QTextEdit()
        self._text_edit.setMinimumHeight(72)
        self._text_edit.setMaximumHeight(120)
        self._text_edit.setPlaceholderText("Enter text...")
        self._text_edit.setStyleSheet(
            f"background-color: {COLORS['surface']}; "
            f"color: {COLORS['text_primary']}; "
            "font-family: monospace; font-size: 11px;"
        )
        self._text_edit.textChanged.connect(self._on_text_changed)
        layout.addWidget(self._text_edit)

        # ─── SECTION 2: FONT ───
        layout.addWidget(create_section_header("FONT"))

        # Font Family
        fam_widget = QWidget()
        fam_lay = QHBoxLayout(fam_widget)
        fam_lay.setContentsMargins(0, 2, 0, 2)
        fam_lbl = QLabel("Family:")
        fam_lbl.setStyleSheet(f"color: {COLORS['text_secondary']}; font-size: 11px;")
        self._font_combo = QComboBox()
        self._font_combo.addItems(["Inter", "Arial", "Roboto", "Times New Roman", "Courier New"])
        self._font_combo.currentIndexChanged.connect(self._on_family_changed)
        fam_lay.addWidget(fam_lbl)
        fam_lay.addWidget(self._font_combo, 1)
        layout.addWidget(fam_widget)

        # Font Size & Styles
        style_widget = QWidget()
        style_lay = QHBoxLayout(style_widget)
        style_lay.setContentsMargins(0, 2, 0, 2)

        size_lbl = QLabel("Size:")
        size_lbl.setStyleSheet(f"color: {COLORS['text_secondary']}; font-size: 11px;")
        self._size_spin = QSpinBox()
        self._size_spin.setRange(8, 120)
        self._size_spin.setSuffix(" px")
        self._size_spin.valueChanged.connect(self._on_size_changed)

        btn_style = (
            f"QPushButton {{ background-color: {COLORS['surface']}; "
            f"border: 1px solid {COLORS['border']}; border-radius: 4px; "
            "font-size: 10px; font-weight: bold; }\n"
            f"QPushButton:checked {{ background-color: {COLORS['accent']}; "
            "color: #FFFFFF; }"
        )

        self._btn_bold = QPushButton("B")
        self._btn_bold.setFixedSize(28, 24)
        self._btn_bold.setCheckable(True)
        self._btn_bold.setStyleSheet(btn_style)
        self._btn_bold.clicked.connect(self._on_bold_toggled)

        self._btn_italic = QPushButton("I")
        self._btn_italic.setFixedSize(28, 24)
        self._btn_italic.setCheckable(True)
        self._btn_italic.setStyleSheet(btn_style)
        self._btn_italic.clicked.connect(self._on_italic_toggled)

        self._btn_underline = QPushButton("U")
        self._btn_underline.setFixedSize(28, 24)
        self._btn_underline.setCheckable(True)
        self._btn_underline.setStyleSheet(btn_style)
        self._btn_underline.clicked.connect(self._on_underline_toggled)

        style_lay.addWidget(size_lbl)
        style_lay.addWidget(self._size_spin)
        style_lay.addWidget(self._btn_bold)
        style_lay.addWidget(self._btn_italic)
        style_lay.addWidget(self._btn_underline)
        layout.addWidget(style_widget)

        # Color
        color_widget = QWidget()
        color_lay = QHBoxLayout(color_widget)
        color_lay.setContentsMargins(0, 2, 0, 2)
        color_lbl = QLabel("Color:")
        color_lbl.setStyleSheet(f"color: {COLORS['text_secondary']}; font-size: 11px;")
        self._btn_color = ColorSwatchButton(QColor("#FFFFFF"))
        self._btn_color.color_changed.connect(self._on_color_changed)
        color_lay.addWidget(color_lbl)
        color_lay.addWidget(self._btn_color, 1)
        layout.addWidget(color_widget)

        # ─── SECTION 3: BACKGROUND ───
        layout.addWidget(create_section_header("BACKGROUND"))

        # Background Color
        bg_col_widget = QWidget()
        bg_col_lay = QHBoxLayout(bg_col_widget)
        bg_col_lay.setContentsMargins(0, 2, 0, 2)
        bg_col_lbl = QLabel("Color:")
        bg_col_lbl.setStyleSheet(f"color: {COLORS['text_secondary']}; font-size: 11px;")
        self._btn_bg_color = ColorSwatchButton(QColor("#000000"))
        self._btn_bg_color.color_changed.connect(self._on_bg_color_changed)
        bg_col_lay.addWidget(bg_col_lbl)
        bg_col_lay.addWidget(self._btn_bg_color, 1)
        layout.addWidget(bg_col_widget)

        # Background Opacity
        bg_op_widget = QWidget()
        bg_op_lay = QHBoxLayout(bg_op_widget)
        bg_op_lay.setContentsMargins(0, 2, 0, 2)
        bg_op_lbl = QLabel("Opacity:")
        bg_op_lbl.setStyleSheet(f"color: {COLORS['text_secondary']}; font-size: 11px;")

        self._op_slider = QSlider(Qt.Orientation.Horizontal)
        self._op_slider.setRange(0, 100)
        self._op_slider.valueChanged.connect(self._on_opacity_changed)

        self._lbl_op_val = QLabel("0%")
        self._lbl_op_val.setFixedWidth(36)
        self._lbl_op_val.setAlignment(Qt.AlignmentFlag.AlignRight | Qt.AlignmentFlag.AlignVCenter)
        self._lbl_op_val.setStyleSheet(f"color: {COLORS['text_primary']}; font-size: 11px;")

        bg_op_lay.addWidget(bg_op_lbl)
        bg_op_lay.addWidget(self._op_slider)
        bg_op_lay.addWidget(self._lbl_op_val)
        layout.addWidget(bg_op_widget)

        # ─── SECTION 4: ALIGNMENT ───
        layout.addWidget(create_section_header("ALIGNMENT"))

        align_widget = QWidget()
        align_lay = QHBoxLayout(align_widget)
        align_lay.setContentsMargins(0, 2, 0, 2)
        align_lay.setSpacing(6)

        self._align_group = QButtonGroup(self)
        self._align_group.setExclusive(True)

        self._btn_left = QPushButton("◄ Left")
        self._btn_left.setCheckable(True)
        self._btn_left.setStyleSheet(btn_style)
        self._btn_left.clicked.connect(lambda: self._on_align_changed("left"))

        self._btn_center = QPushButton("■ Center")
        self._btn_center.setCheckable(True)
        self._btn_center.setStyleSheet(btn_style)
        self._btn_center.clicked.connect(lambda: self._on_align_changed("center"))

        self._btn_right = QPushButton("► Right")
        self._btn_right.setCheckable(True)
        self._btn_right.setStyleSheet(btn_style)
        self._btn_right.clicked.connect(lambda: self._on_align_changed("right"))

        self._align_group.addButton(self._btn_left)
        self._align_group.addButton(self._btn_center)
        self._align_group.addButton(self._btn_right)

        align_lay.addWidget(self._btn_left)
        align_lay.addWidget(self._btn_center)
        align_lay.addWidget(self._btn_right)
        layout.addWidget(align_widget)

        # ─── SECTION 5: POSITION ───
        layout.addWidget(create_section_header("POSITION"))

        # Pos X
        pos_x_widget = QWidget()
        pos_x_lay = QHBoxLayout(pos_x_widget)
        pos_x_lay.setContentsMargins(0, 2, 0, 2)
        pos_x_lbl = QLabel("X:")
        pos_x_lbl.setFixedWidth(16)
        pos_x_lbl.setStyleSheet(f"color: {COLORS['text_secondary']}; font-size: 11px;")

        self._pos_x_slider = QSlider(Qt.Orientation.Horizontal)
        self._pos_x_slider.setRange(0, 100)
        self._pos_x_slider.valueChanged.connect(self._on_pos_x_changed)

        self._lbl_pos_x = QLabel("50%")
        self._lbl_pos_x.setFixedWidth(36)
        self._lbl_pos_x.setAlignment(Qt.AlignmentFlag.AlignRight | Qt.AlignmentFlag.AlignVCenter)
        self._lbl_pos_x.setStyleSheet(f"color: {COLORS['text_primary']}; font-size: 11px;")

        pos_x_lay.addWidget(pos_x_lbl)
        pos_x_lay.addWidget(self._pos_x_slider)
        pos_x_lay.addWidget(self._lbl_pos_x)
        layout.addWidget(pos_x_widget)

        # Pos Y
        pos_y_widget = QWidget()
        pos_y_lay = QHBoxLayout(pos_y_widget)
        pos_y_lay.setContentsMargins(0, 2, 0, 2)
        pos_y_lbl = QLabel("Y:")
        pos_y_lbl.setFixedWidth(16)
        pos_y_lbl.setStyleSheet(f"color: {COLORS['text_secondary']}; font-size: 11px;")

        self._pos_y_slider = QSlider(Qt.Orientation.Horizontal)
        self._pos_y_slider.setRange(0, 100)
        self._pos_y_slider.valueChanged.connect(self._on_pos_y_changed)

        self._lbl_pos_y = QLabel("85%")
        self._lbl_pos_y.setFixedWidth(36)
        self._lbl_pos_y.setAlignment(Qt.AlignmentFlag.AlignRight | Qt.AlignmentFlag.AlignVCenter)
        self._lbl_pos_y.setStyleSheet(f"color: {COLORS['text_primary']}; font-size: 11px;")

        pos_y_lay.addWidget(pos_y_lbl)
        pos_y_lay.addWidget(self._pos_y_slider)
        pos_y_lay.addWidget(self._lbl_pos_y)
        layout.addWidget(pos_y_widget)

        # ─── SECTION 6: ROTATION ───
        layout.addWidget(create_section_header("ROTATION"))

        rot_widget = QWidget()
        rot_lay = QHBoxLayout(rot_widget)
        rot_lay.setContentsMargins(0, 2, 0, 2)
        rot_lbl = QLabel("-180°")
        rot_lbl.setStyleSheet(f"color: {COLORS['text_secondary']}; font-size: 9px;")

        self._rot_slider = QSlider(Qt.Orientation.Horizontal)
        self._rot_slider.setRange(-180, 180)
        self._rot_slider.valueChanged.connect(self._on_rotation_slider_changed)

        rot_lbl_right = QLabel("+180°")
        rot_lbl_right.setStyleSheet(f"color: {COLORS['text_secondary']}; font-size: 9px;")

        self._rotation_spin = QSpinBox()
        self._rotation_spin.setRange(-180, 180)
        self._rotation_spin.setSuffix("°")
        self._rotation_spin.valueChanged.connect(self._on_rotation_spin_changed)

        rot_lay.addWidget(rot_lbl)
        rot_lay.addWidget(self._rot_slider)
        rot_lay.addWidget(rot_lbl_right)
        rot_lay.addWidget(self._rotation_spin)
        layout.addWidget(rot_widget)

        layout.addStretch()

    # ─── Events ───
    def _on_text_changed(self) -> None:
        if self._loading:
            return
        self._debounce_timer.start()

    def _on_text_debounced(self) -> None:
        if self._loading:
            return
        if self._clip_id:
            text_val = self._text_edit.toPlainText()
            self.text_property_changed.emit(self._clip_id, {"content": text_val})

    def _on_family_changed(self, idx: int) -> None:
        if self._loading:
            return
        if self._clip_id:
            fam = self._font_combo.currentText()
            self.text_property_changed.emit(self._clip_id, {"font_family": fam})

    def _on_size_changed(self, val: int) -> None:
        if self._loading:
            return
        if self._clip_id:
            self.text_property_changed.emit(self._clip_id, {"font_size": val})

    def _on_bold_toggled(self, checked: bool) -> None:
        if self._loading:
            return
        if self._clip_id:
            self.text_property_changed.emit(self._clip_id, {"bold": checked})

    def _on_italic_toggled(self, checked: bool) -> None:
        if self._loading:
            return
        if self._clip_id:
            self.text_property_changed.emit(self._clip_id, {"italic": checked})

    def _on_underline_toggled(self, checked: bool) -> None:
        if self._loading:
            return
        if self._clip_id:
            self.text_property_changed.emit(self._clip_id, {"underline": checked})

    def _on_color_changed(self, color: QColor) -> None:
        if self._loading:
            return
        if self._clip_id:
            self.text_property_changed.emit(self._clip_id, {"font_color": color.name()})

    def _on_bg_color_changed(self, color: QColor) -> None:
        if self._loading:
            return
        if self._clip_id:
            self.text_property_changed.emit(self._clip_id, {"background_color": color.name()})

    def _on_opacity_changed(self, val: int) -> None:
        self._lbl_op_val.setText(f"{val}%")
        if self._loading:
            return
        if self._clip_id:
            self.text_property_changed.emit(self._clip_id, {"background_opacity": val / 100.0})

    def _on_align_changed(self, align: str) -> None:
        if self._loading:
            return
        if self._clip_id:
            self.text_property_changed.emit(self._clip_id, {"alignment": align})

    def _on_pos_x_changed(self, val: int) -> None:
        self._lbl_pos_x.setText(f"{val}%")
        if self._loading:
            return
        if self._clip_id:
            self.text_property_changed.emit(self._clip_id, {"position_x": float(val)})

    def _on_pos_y_changed(self, val: int) -> None:
        self._lbl_pos_y.setText(f"{val}%")
        if self._loading:
            return
        if self._clip_id:
            self.text_property_changed.emit(self._clip_id, {"position_y": float(val)})

    def _on_rotation_slider_changed(self, val: int) -> None:
        self._rotation_spin.blockSignals(True)
        self._rotation_spin.setValue(val)
        self._rotation_spin.blockSignals(False)
        self._on_rotation_changed(val)

    def _on_rotation_spin_changed(self, val: int) -> None:
        self._rot_slider.blockSignals(True)
        self._rot_slider.setValue(val)
        self._rot_slider.blockSignals(False)
        self._on_rotation_changed(val)

    def _on_rotation_changed(self, val: int) -> None:
        if self._loading:
            return
        if self._clip_id:
            self.text_property_changed.emit(self._clip_id, {"rotation": float(val)})

    # ─── Load state ───
    def load(self, text_clip: TextClip) -> None:
        """Populate the UI fields with values from the given TextClip."""
        self._loading = True
        self._clip_id = text_clip.id

        # Text Content
        self._text_edit.setPlainText(text_clip.content)

        # Font Combo
        self._font_combo.setCurrentText(text_clip.font_family)

        # Font Size
        self._size_spin.setValue(text_clip.font_size)

        # Styles (Checked state updates visual class immediately)
        self._btn_bold.setChecked(text_clip.bold)
        self._btn_italic.setChecked(text_clip.italic)
        self._btn_underline.setChecked(text_clip.underline)

        # Colors
        self._btn_color.set_color(QColor(text_clip.font_color))
        self._btn_bg_color.set_color(QColor(text_clip.background_color))

        # Opacity
        op_percent = int(text_clip.background_opacity * 100.0)
        self._op_slider.setValue(op_percent)
        self._lbl_op_val.setText(f"{op_percent}%")

        # Alignment
        if text_clip.alignment == "left":
            self._btn_left.setChecked(True)
        elif text_clip.alignment == "right":
            self._btn_right.setChecked(True)
        else:
            self._btn_center.setChecked(True)

        # Position
        self._pos_x_slider.setValue(int(text_clip.position_x))
        self._lbl_pos_x.setText(f"{int(text_clip.position_x)}%")

        self._pos_y_slider.setValue(int(text_clip.position_y))
        self._lbl_pos_y.setText(f"{int(text_clip.position_y)}%")

        # Rotation
        rot_val = int(text_clip.rotation)
        self._rot_slider.setValue(rot_val)
        self._rotation_spin.setValue(rot_val)

        self._loading = False
