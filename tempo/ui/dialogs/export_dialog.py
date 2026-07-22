"""Export settings dialog shown before export starts."""

from __future__ import annotations

from pathlib import Path
from typing import TYPE_CHECKING, ClassVar

from PySide6.QtCore import Signal
from PySide6.QtWidgets import (
    QComboBox,
    QDialog,
    QFileDialog,
    QFormLayout,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QPushButton,
    QVBoxLayout,
    QWidget,
)

from tempo.media.exporter import (
    ExportSettings,
    estimate_export_duration,
    validate_project_for_export,
)
from tempo.ui.theme import COLORS, SPACING
from tempo.utils.timecode import seconds_to_timecode

if TYPE_CHECKING:
    from tempo.core.models import Project


class ExportDialog(QDialog):
    """Export settings dialog. Shows before export starts."""

    export_requested = Signal(ExportSettings)

    RESOLUTIONS: ClassVar[dict[str, tuple[int, int] | None]] = {
        "Original": None,
        "1080p": (1920, 1080),
        "720p": (1280, 720),
        "480p": (854, 480),
    }
    QUALITY_PRESETS: ClassVar[dict[str, int]] = {
        "High (larger file)": 18,
        "Medium (recommended)": 23,
        "Low (smaller file)": 28,
    }

    def __init__(
        self,
        project: Project,
        default_output_dir: Path,
        parent: QWidget | None = None,
    ) -> None:
        super().__init__(parent)
        self.setWindowTitle("Export Video")
        self.setFixedSize(480, 280)
        self.setModal(True)
        self._project = project
        self._warnings = validate_project_for_export(project)
        self._build_ui(default_output_dir)

    def _build_ui(self, default_dir: Path) -> None:
        layout = QVBoxLayout(self)
        layout.setSpacing(SPACING["unit"] * 3)

        # Output path row
        path_row = QHBoxLayout()
        self._path_edit = QLineEdit()
        default_name = (self._project.project_name or "output").replace(" ", "_")
        self._path_edit.setText(str(default_dir / f"{default_name}.mp4"))
        browse_btn = QPushButton("Browse...")
        browse_btn.setFixedWidth(80)
        browse_btn.clicked.connect(self._browse_output)
        path_row.addWidget(QLabel("Output file:"))
        path_row.addWidget(self._path_edit, stretch=1)
        path_row.addWidget(browse_btn)
        layout.addLayout(path_row)

        # Resolution and quality
        form = QFormLayout()
        self._res_combo = QComboBox()
        self._res_combo.addItems(list(self.RESOLUTIONS.keys()))
        self._res_combo.setCurrentText("1080p")
        self._qual_combo = QComboBox()
        self._qual_combo.addItems(list(self.QUALITY_PRESETS.keys()))
        self._qual_combo.setCurrentText("Medium (recommended)")
        form.addRow("Resolution:", self._res_combo)
        form.addRow("Quality:", self._qual_combo)
        layout.addLayout(form)

        # Duration estimate
        dur = estimate_export_duration(self._project)
        dur_label = QLabel(f"Estimated duration:  {seconds_to_timecode(dur, fps=30)}")
        dur_label.setStyleSheet(f"color: {COLORS['text_secondary']};")
        layout.addWidget(dur_label)

        # Warnings
        if self._warnings:
            warn_label = QLabel(
                f"⚠ {len(self._warnings)} warning(s) — some source files may be missing."
            )
            warn_label.setStyleSheet(f"color: {COLORS['warning']};")
            layout.addWidget(warn_label)

        # Buttons
        btn_row = QHBoxLayout()
        btn_row.addStretch()
        cancel_btn = QPushButton("Cancel")
        cancel_btn.setFixedWidth(80)
        cancel_btn.clicked.connect(self.reject)
        export_btn = QPushButton("Export →")
        export_btn.setFixedWidth(100)
        export_btn.setDefault(True)
        export_btn.clicked.connect(self._on_export_clicked)

        has_blocking_warning = any(
            "missing" in w.lower() or "no clips" in w.lower() or "not available" in w.lower()
            for w in self._warnings
        )
        export_btn.setEnabled(not has_blocking_warning)

        btn_row.addWidget(cancel_btn)
        btn_row.addWidget(export_btn)
        layout.addLayout(btn_row)

    def _browse_output(self) -> None:
        path, _ = QFileDialog.getSaveFileName(
            self, "Save Video As", self._path_edit.text(), "MP4 Video (*.mp4)"
        )
        if path:
            if not path.endswith(".mp4"):
                path += ".mp4"
            self._path_edit.setText(path)

    def _on_export_clicked(self) -> None:
        output = Path(self._path_edit.text())
        res_name = self._res_combo.currentText()
        resolution = self.RESOLUTIONS[res_name] or (1920, 1080)
        crf = self.QUALITY_PRESETS[self._qual_combo.currentText()]
        settings = ExportSettings(
            output_path=output,
            resolution=resolution,
            crf=crf,
        )
        self.export_requested.emit(settings)
        self.accept()
