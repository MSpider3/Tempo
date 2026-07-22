"""Timeline widget containing track headers, ruler, overview, and controls."""

from __future__ import annotations

import math
from enum import Enum
from typing import TYPE_CHECKING

from PySide6.QtCore import Qt, Signal
from PySide6.QtWidgets import (
    QHBoxLayout,
    QPushButton,
    QSlider,
    QVBoxLayout,
    QWidget,
)

from tempo.ui.theme import TRACK_HEADER_WIDTH, get_color, get_font
from tempo.ui.timeline.lower_timeline import LowerTimeline
from tempo.ui.timeline.time_ruler import TimeRuler
from tempo.ui.timeline.track_header import TrackHeader
from tempo.ui.timeline.upper_timeline import UpperTimeline

if TYPE_CHECKING:
    from PySide6.QtGui import QResizeEvent


class ActiveTool(Enum):
    """Representing current timeline cursor select tool mode."""

    SELECT = "select"
    BLADE = "blade"
    TRIM = "trim"
    TEXT = "text"


def pps_to_slider(pps: float) -> int:
    """Map pixels_per_second value (10-500) logarithmically to slider index (0-100)."""
    ratio = max(10.0, min(500.0, pps)) / 10.0
    val = 100.0 * math.log(ratio) / math.log(50.0)
    return int(max(0.0, min(100.0, val)))


def slider_to_pps(val: float) -> float:
    """Map slider index (0-100) logarithmically to pixels_per_second value (10-500)."""
    return 10.0 * math.pow(50.0, val / 100.0)


class TimelineWidget(QWidget):
    """Container widget housing all lower/upper tracks, time ruler, and toolbar."""

    tool_changed = Signal(ActiveTool)
    zoom_changed = Signal(float)

    lower_timeline: LowerTimeline
    upper_timeline: UpperTimeline

    def __init__(self, parent: QWidget | None = None) -> None:
        """Initialize the TimelineWidget.

        Args:
            parent: Optional parent widget.
        """
        super().__init__(parent)
        self.setMinimumHeight(200)

        self._active_tool = ActiveTool.SELECT

        self._build_ui()
        self._setup_connections()

        # Establish default zoom slider position
        self._set_pps(100.0)

    # ── Properties ───────────────────────────────────────────────────────────

    @property
    def active_tool(self) -> ActiveTool:
        """Get the current active timeline editor tool."""
        return self._active_tool

    @active_tool.setter
    def active_tool(self, val: ActiveTool) -> None:
        """Set the current active timeline editor tool."""
        self.set_tool(val)

    @property
    def pixels_per_second(self) -> float:
        """Get current pixels per second zoom level."""
        return self.lower_timeline.pixels_per_second

    @pixels_per_second.setter
    def pixels_per_second(self, val: float) -> None:
        """Set pixels per second zoom level."""
        self._set_pps(val)

    # ── Tool Selection ───────────────────────────────────────────────────────

    def set_tool(self, tool: ActiveTool) -> None:
        """Select editor tool and sync button states."""
        self._active_tool = tool
        self._btn_select.setChecked(tool == ActiveTool.SELECT)
        self._btn_blade.setChecked(tool == ActiveTool.BLADE)
        self._btn_trim.setChecked(tool == ActiveTool.TRIM)
        self._btn_text.setChecked(tool == ActiveTool.TEXT)
        self.lower_timeline.set_active_tool(tool)
        self.tool_changed.emit(tool)

    # ── Layout Construction ──────────────────────────────────────────────────

    def _build_ui(self) -> None:
        root = QVBoxLayout(self)
        root.setContentsMargins(0, 0, 0, 0)
        root.setSpacing(0)

        # 1. Timeline Toolbar (36px height)
        toolbar = QWidget(self)
        toolbar.setFixedHeight(36)
        toolbar.setStyleSheet(
            f"background-color: {get_color('panel')};"
            f"border-bottom: 1px solid {get_color('border')};"
        )
        toolbar_layout = QHBoxLayout(toolbar)
        toolbar_layout.setContentsMargins(8, 0, 8, 0)
        toolbar_layout.setSpacing(6)

        # Tool buttons group
        self._btn_select = QPushButton("A Select", self)
        self._btn_blade = QPushButton("B Blade", self)
        self._btn_trim = QPushButton("T Trim", self)
        self._btn_text = QPushButton("T Text", self)

        # Configure toggle group attributes
        for btn in (
            self._btn_select,
            self._btn_blade,
            self._btn_trim,
            self._btn_text,
        ):
            btn.setCheckable(True)
            btn.setFixedHeight(24)
            btn.setFont(get_font("ui", size=11))
            btn.setStyleSheet(
                "QPushButton {"
                f"  background-color: {get_color('surface')};"
                "  border: 1px solid transparent;"
                "  border-radius: 3px;"
                "  padding: 2px 8px;"
                "}"
                "QPushButton:hover {"
                f"  background-color: {get_color('border')};"
                "}"
                "QPushButton:checked {"
                f"  background-color: {get_color('accent')};"
                "  color: #FFFFFF;"
                "}"
            )
            toolbar_layout.addWidget(btn)

        self._btn_select.setChecked(True)

        # Add vertical line separator
        sep = QWidget(self)
        sep.setFixedWidth(1)
        sep.setFixedHeight(20)
        sep.setStyleSheet(f"background-color: {get_color('border')};")
        toolbar_layout.addWidget(sep)

        toolbar_layout.addStretch()

        # Zoom controls
        self._btn_zoom_out = QPushButton("-", self)
        self._btn_zoom_out.setFixedSize(20, 20)
        toolbar_layout.addWidget(self._btn_zoom_out)

        self._zoom_slider = QSlider(Qt.Orientation.Horizontal, self)
        self._zoom_slider.setRange(0, 10000)  # high precision slider
        self._zoom_slider.setFixedWidth(120)
        toolbar_layout.addWidget(self._zoom_slider)

        self._btn_zoom_in = QPushButton("+", self)
        self._btn_zoom_in.setFixedSize(20, 20)
        toolbar_layout.addWidget(self._btn_zoom_in)

        self._btn_zoom_fit = QPushButton("Fit", self)
        self._btn_zoom_fit.setFixedHeight(20)
        self._btn_zoom_fit.setFont(get_font("ui", size=10))
        toolbar_layout.addWidget(self._btn_zoom_fit)

        root.addWidget(toolbar)

        # 2. Upper Overview Timeline (48px height)
        self.upper_timeline = UpperTimeline(self)
        root.addWidget(self.upper_timeline)

        # 3. Lower tracks area
        lower_area = QWidget(self)
        lower_layout = QHBoxLayout(lower_area)
        lower_layout.setContentsMargins(0, 0, 0, 0)
        lower_layout.setSpacing(0)

        # Left Column: HeaderArea with vertical alignment spacer
        header_area = QWidget(lower_area)
        header_area.setFixedWidth(TRACK_HEADER_WIDTH)
        header_vbox = QVBoxLayout(header_area)
        header_vbox.setContentsMargins(0, 0, 0, 0)
        header_vbox.setSpacing(0)

        # Spacer mirroring TimeRuler height
        spacer = QWidget(header_area)
        spacer.setFixedHeight(24)
        spacer.setStyleSheet(
            f"background-color: {get_color('panel')};"
            f"border-right: 1px solid {get_color('border')};"
            f"border-bottom: 1px solid {get_color('border')};"
        )
        header_vbox.addWidget(spacer)

        # Instantiate LowerTimeline first so TrackHeader can connect to it
        self.lower_timeline = LowerTimeline(lower_area)

        self.track_header = TrackHeader(self.lower_timeline, header_area)
        header_vbox.addWidget(self.track_header, stretch=1)
        lower_layout.addWidget(header_area)

        # Right Column: TimeRuler and editing canvas
        canvas_container = QWidget(lower_area)
        canvas_vbox = QVBoxLayout(canvas_container)
        canvas_vbox.setContentsMargins(0, 0, 0, 0)
        canvas_vbox.setSpacing(0)

        self.time_ruler = TimeRuler(self.lower_timeline, canvas_container)
        canvas_vbox.addWidget(self.time_ruler)
        canvas_vbox.addWidget(self.lower_timeline, stretch=1)

        lower_layout.addWidget(canvas_container, stretch=1)
        root.addWidget(lower_area, stretch=1)

    # ── Connections Sync ─────────────────────────────────────────────────────

    def _setup_connections(self) -> None:
        """Wire toolbar triggers, scrolling hooks, and event routing."""
        # Tool buttons select
        self._btn_select.clicked.connect(lambda: self.set_tool(ActiveTool.SELECT))
        self._btn_blade.clicked.connect(lambda: self.set_tool(ActiveTool.BLADE))
        self._btn_trim.clicked.connect(lambda: self.set_tool(ActiveTool.TRIM))
        self._btn_text.clicked.connect(lambda: self.set_tool(ActiveTool.TEXT))

        # Zoom triggers
        self._zoom_slider.valueChanged.connect(self._on_slider_changed)
        self._btn_zoom_in.clicked.connect(self.zoom_in)
        self._btn_zoom_out.clicked.connect(self.zoom_out)
        self._btn_zoom_fit.clicked.connect(self.fit_to_content)

        # Zoom sync back from view
        self.lower_timeline.zoom_changed.connect(self._on_pps_changed)

        # Ruler/Overview scroll value changes synchronization
        self.lower_timeline.horizontalScrollBar().valueChanged.connect(self.time_ruler.update)
        self.lower_timeline.horizontalScrollBar().valueChanged.connect(self._sync_upper_timeline)

        # Sync overview playhead when playhead moves
        self.lower_timeline.playhead_moved.connect(self.upper_timeline.set_playhead)

    # ── Scrolling and Viewport synchronization ───────────────────────────────

    def _sync_upper_timeline(self) -> None:
        """Synchronize UpperTimeline viewport box indicator on scroll/zoom changes."""
        scroll_offset = float(self.lower_timeline.horizontalScrollBar().value())
        vp_width = float(self.lower_timeline.viewport().width())
        pps = self.lower_timeline.pixels_per_second
        self.upper_timeline.set_viewport_info(scroll_offset, vp_width, pps)

    # ── Zoom Management ──────────────────────────────────────────────────────

    def _set_pps(self, val: float) -> None:
        """Safely set internal zoom levels without triggering recursion loop."""
        val = max(10.0, min(500.0, val))

        # Block view and slider updates loop
        self.lower_timeline.blockSignals(True)
        self.lower_timeline.pixels_per_second = val
        self.lower_timeline.blockSignals(False)

        self._zoom_slider.blockSignals(True)
        # slider expects 0-10000 range
        slider_val = int(pps_to_slider(val) * 100)
        self._zoom_slider.setValue(slider_val)
        self._zoom_slider.blockSignals(False)

        self._sync_upper_timeline()
        self.upper_timeline.set_viewport_info(
            float(self.lower_timeline.horizontalScrollBar().value()),
            float(self.lower_timeline.viewport().width()),
            val,
        )
        self.zoom_changed.emit(val)

    def _on_slider_changed(self, value: int) -> None:
        """Slot: update pps from zoom slider drags."""
        pps = slider_to_pps(value / 100.0)
        self._set_pps(pps)

    def _on_pps_changed(self, pps: float) -> None:
        """Slot: update zoom slider on Alt+Scroll view changes."""
        self._set_pps(pps)

    def zoom_in(self) -> None:
        """Increment zoom scale."""
        self._set_pps(self.lower_timeline.pixels_per_second * 1.25)

    def zoom_out(self) -> None:
        """Decrement zoom scale."""
        self._set_pps(self.lower_timeline.pixels_per_second / 1.25)

    def fit_to_content(self) -> None:
        """Set zoom scale so project is fully visible within current viewport width."""
        dur = self.lower_timeline._duration
        if dur <= 0.0:
            self._set_pps(100.0)
            return

        w = float(self.lower_timeline.viewport().width())
        # Provide small buffer margin
        fit_pps = (w - 40.0) / dur
        self._set_pps(fit_pps)

    # ── Resize override ──────────────────────────────────────────────────────

    def resizeEvent(self, event: QResizeEvent) -> None:  # noqa: N802
        super().resizeEvent(event)
        self._sync_upper_timeline()
