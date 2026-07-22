"""Modal progress dialog displayed during video export."""

from __future__ import annotations

from typing import TYPE_CHECKING

from PySide6.QtCore import Qt, Signal
from PySide6.QtWidgets import (
    QDialog,
    QLabel,
    QProgressBar,
    QPushButton,
    QVBoxLayout,
    QWidget,
)

from tempo.ui.theme import COLORS

if TYPE_CHECKING:
    from pathlib import Path


class ExportProgressDialog(QDialog):
    """Modal progress dialog shown during export. Non-closeable except via Cancel."""

    cancel_requested = Signal()

    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setWindowTitle("Exporting...")
        self.setFixedSize(400, 160)
        self.setModal(True)
        # Prevent closing with X button during active export
        self.setWindowFlag(Qt.WindowType.WindowCloseButtonHint, False)
        self._build_ui()

    def _build_ui(self) -> None:
        layout = QVBoxLayout(self)
        layout.setSpacing(12)

        self._status_label = QLabel("Preparing export...")
        self._status_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        layout.addWidget(self._status_label)

        self._progress_bar = QProgressBar()
        self._progress_bar.setRange(0, 100)
        self._progress_bar.setValue(0)
        self._progress_bar.setTextVisible(True)
        self._progress_bar.setFormat("%p%")
        layout.addWidget(self._progress_bar)

        self._time_label = QLabel("Elapsed: 0:00  |  Remaining: —")
        self._time_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self._time_label.setStyleSheet(f"color: {COLORS['text_secondary']}; font-size: 11px;")
        layout.addWidget(self._time_label)

        self._cancel_btn = QPushButton("Cancel Export")
        self._cancel_btn.clicked.connect(self.cancel_requested)
        layout.addWidget(self._cancel_btn, alignment=Qt.AlignmentFlag.AlignCenter)

    def update_progress(self, percent: int) -> None:
        """Update progress bar value."""
        self._progress_bar.setValue(percent)

    def update_time(self, elapsed: float, remaining: float) -> None:
        """Update elapsed and remaining time labels."""

        def fmt(s: float) -> str:
            m, sec = divmod(int(s), 60)
            return f"{m}:{sec:02d}"

        remaining_str = fmt(remaining) if remaining > 0 else "—"
        self._time_label.setText(f"Elapsed: {fmt(elapsed)}  |  Remaining: {remaining_str}")

    def mark_complete(self, output_path: Path) -> None:
        """Update UI state when export completes successfully."""
        self._progress_bar.setValue(100)
        self._status_label.setText("Export complete!")
        self.setWindowFlag(Qt.WindowType.WindowCloseButtonHint, True)
        self._cancel_btn.setText("Close")
        self._cancel_btn.clicked.disconnect()
        self._cancel_btn.clicked.connect(self.accept)

    def mark_failed(self, error: str) -> None:
        """Update UI state when export fails."""
        self._status_label.setText(f"Export failed: {error[:80]}")
        self._progress_bar.setStyleSheet(
            f"QProgressBar::chunk {{ background: {COLORS['error']}; }}"
        )
        self.setWindowFlag(Qt.WindowType.WindowCloseButtonHint, True)
        self._cancel_btn.setText("Close")
        self._cancel_btn.clicked.disconnect()
        self._cancel_btn.clicked.connect(self.reject)
