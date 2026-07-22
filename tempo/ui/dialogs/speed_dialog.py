"""Modal dialog for setting clip speed."""

from __future__ import annotations

from typing import ClassVar

from PySide6.QtCore import Qt
from PySide6.QtWidgets import (
    QCheckBox,
    QDialog,
    QGridLayout,
    QHBoxLayout,
    QLabel,
    QPushButton,
    QVBoxLayout,
    QWidget,
)

from tempo.ui.theme import COLORS, SPACING


class SpeedDialog(QDialog):
    """Modal dialog for setting clip speed. Shows current speed, allows selection."""

    SPEED_STEPS: ClassVar[list[float]] = [0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 2.0, 4.0]

    def __init__(
        self, current_speed: float, current_pitch: bool, parent: QWidget | None = None
    ) -> None:
        super().__init__(parent)
        self.setWindowTitle("Change Speed")
        self.setFixedSize(320, 220)
        self.setModal(True)
        self._selected_speed = current_speed
        self._selected_pitch = current_pitch
        self._build_ui()

    def _build_ui(self) -> None:
        layout = QVBoxLayout(self)
        layout.setSpacing(SPACING["unit"] * 2)

        # Speed button grid (same as inspector but larger)
        grid = QGridLayout()
        grid.setSpacing(4)
        self._speed_buttons: dict[float, QPushButton] = {}
        for i, speed in enumerate(self.SPEED_STEPS):
            btn = QPushButton(f"{speed}×")  # noqa: RUF001
            btn.setFixedSize(60, 32)
            btn.setCheckable(True)
            btn.setChecked(speed == self._selected_speed)
            btn.clicked.connect(lambda _, s=speed: self._on_speed_selected(s))
            self._speed_buttons[speed] = btn
            grid.addWidget(btn, i // 4, i % 4)
        layout.addLayout(grid)

        # Pitch correction checkbox
        self._pitch_check = QCheckBox("Maintain audio pitch")
        self._pitch_check.setChecked(self._selected_pitch)
        layout.addWidget(self._pitch_check)

        # Result preview label
        self._preview_label = QLabel()
        self._preview_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self._preview_label.setStyleSheet(f"color: {COLORS['text_secondary']}; font-size: 11px;")
        layout.addWidget(self._preview_label)
        self._update_preview_label()

        # OK / Cancel buttons
        btn_row = QHBoxLayout()
        btn_row.addStretch()
        ok = QPushButton("Apply")
        ok.setFixedWidth(80)
        ok.setDefault(True)
        ok.clicked.connect(self.accept)
        cancel = QPushButton("Cancel")
        cancel.setFixedWidth(80)
        cancel.clicked.connect(self.reject)
        btn_row.addWidget(cancel)
        btn_row.addWidget(ok)
        layout.addLayout(btn_row)

    def _on_speed_selected(self, speed: float) -> None:
        self._selected_speed = speed
        for s, btn in self._speed_buttons.items():
            btn.setChecked(s == speed)
        self._update_preview_label()

    def _update_preview_label(self) -> None:
        s = self._selected_speed
        if s < 1.0:
            desc = f"Slow motion — {s}× speed"  # noqa: RUF001
        elif s > 1.0:
            desc = f"Fast motion — {s}× speed"  # noqa: RUF001
        else:
            desc = "Normal speed"
        self._preview_label.setText(desc)

    @property
    def selected_speed(self) -> float:
        return self._selected_speed

    @property
    def pitch_correction(self) -> bool:
        return self._pitch_check.isChecked()
