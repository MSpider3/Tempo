"""Tempo design system — single source of truth for all colors, fonts, spacing.

NO hardcoded colors, fonts, or spacing values anywhere else in the codebase.
Every UI file imports from here.
"""

from typing import Any

from PySide6.QtGui import QFont, QFontDatabase
from PySide6.QtWidgets import QApplication

# ---------------------------------------------------------------------------
# Color Palette
# ---------------------------------------------------------------------------

COLORS: dict[str, str] = {
    "bg": "#1A1A1F",  # main app background
    "panel": "#242429",  # panel backgrounds
    "surface": "#2E2E35",  # cards, clip blocks
    "border": "#3A3A42",  # separators
    "accent": "#4A9EFF",  # primary blue, playhead, selection
    "accent_hover": "#6AB4FF",
    "text_primary": "#E8E8EE",
    "text_secondary": "#8A8A95",
    "text_disabled": "#55555E",
    "success": "#4CAF82",  # proxy ready
    "warning": "#E8A43A",  # proxy generating
    "error": "#E85A4A",  # missing file
    "track_v1": "#3A5A8A",
    "track_v2": "#2E4A7A",
    "track_v3": "#223A6A",
    "track_a1": "#2E6A4A",
    "track_a2": "#225A3A",
    "track_a3": "#1A4A2E",
    "track_tx": "#6A3A8A",  # text track / text clips
    "waveform": "#5AE8A0",
    "playhead": "#4A9EFF",
    "selection": "rgba(74, 158, 255, 0.25)",
}

# ---------------------------------------------------------------------------
# Typography
# ---------------------------------------------------------------------------

FONTS: dict[str, Any] = {
    "ui": "Inter",  # all UI labels, buttons
    "mono": "JetBrains Mono",  # timecodes, file sizes
    "size_base": 13,
    "size_small": 11,
    "size_timecode": 14,
}

# ---------------------------------------------------------------------------
# Spacing
# ---------------------------------------------------------------------------

SPACING: dict[str, int] = {
    "unit": 4,  # base grid unit in px
    "panel_padding": 8,
    "item_padding": 6,
}

# ---------------------------------------------------------------------------
# Border Radii
# ---------------------------------------------------------------------------

# ---------------------------------------------------------------------------
# Track Layout Settings
# ---------------------------------------------------------------------------

TRACK_HEIGHT = 48  # px per track row
TRACK_HEADER_WIDTH = 64  # px for the left label column
TRACKS = ["TX", "V3", "V2", "V1", "A1", "A2", "A3"]
TRIM_HANDLE_WIDTH = 8  # px

RADIUS: dict[str, int] = {
    "clip": 4,
    "button": 6,
    "dialog": 8,
}


# ---------------------------------------------------------------------------
# Public API
# ---------------------------------------------------------------------------


def get_color(name: str) -> str:
    """Return a hex color string from the design palette.

    Args:
        name: A key from COLORS.

    Returns:
        The hex color string (e.g. "#1A1A1F").

    Raises:
        KeyError: If the color name is not in the palette.
    """
    if name not in COLORS:
        available = ", ".join(sorted(COLORS.keys()))
        raise KeyError(f"Color '{name}' not found in Tempo palette. Available: {available}")
    return COLORS[name]


def get_font(name: str, size: int | None = None, bold: bool = False) -> QFont:
    """Return a QFont from the design system.

    Args:
        name: Font role — "ui" or "mono".
        size: Point size override. Uses FONTS default if None.
        bold: Whether the font should be bold.

    Returns:
        A configured QFont instance.

    Raises:
        KeyError: If the font role is not in the palette.
    """
    if name not in ("ui", "mono"):
        raise KeyError(f"Font role '{name}' not found. Use 'ui' or 'mono'.")

    family = str(FONTS[name])
    if size is None:
        size = int(FONTS["size_base"])

    font = QFont(family, size)
    font.setBold(bold)
    return font


def make_stylesheet() -> str:
    """Build and return the complete Tempo QSS stylesheet string.

    Returns:
        A multi-line QSS stylesheet string.
    """
    c = COLORS
    r = RADIUS
    sp = SPACING

    return f"""
/* ── Base ─────────────────────────────────────────────────────────── */
QMainWindow, QWidget {{
    background-color: {c["bg"]};
    color: {c["text_primary"]};
    font-family: "{FONTS["ui"]}";
    font-size: {FONTS["size_base"]}px;
    border: none;
    outline: none;
}}

/* ── Panels / Frames ───────────────────────────────────────────────── */
QFrame, QStackedWidget {{
    background-color: {c["bg"]};
    border: none;
}}

/* ── Buttons ───────────────────────────────────────────────────────── */
QPushButton {{
    background-color: {c["surface"]};
    color: {c["text_primary"]};
    border: 1px solid {c["border"]};
    border-radius: {r["button"]}px;
    padding: {sp["item_padding"]}px {sp["panel_padding"] + 4}px;
    min-height: 24px;
}}
QPushButton:hover {{
    background-color: {c["border"]};
    border-color: {c["accent"]};
}}
QPushButton:pressed {{
    background-color: {c["bg"]};
}}
QPushButton:disabled {{
    background-color: {c["surface"]};
    color: {c["text_disabled"]};
    border-color: {c["border"]};
}}
QPushButton[accent="true"] {{
    background-color: {c["accent"]};
    color: #FFFFFF;
    border: none;
    font-weight: bold;
}}
QPushButton[accent="true"]:hover {{
    background-color: {c["accent_hover"]};
}}
QPushButton[accent="true"]:pressed {{
    background-color: {c["accent"]};
}}

/* ── Labels ────────────────────────────────────────────────────────── */
QLabel {{
    background-color: transparent;
    color: {c["text_primary"]};
    border: none;
}}

/* ── List & Tree widgets ───────────────────────────────────────────── */
QListWidget, QListView, QTreeView, QTreeWidget {{
    background-color: {c["panel"]};
    color: {c["text_primary"]};
    border: 1px solid {c["border"]};
    border-radius: {r["clip"]}px;
    outline: none;
    alternate-background-color: {c["bg"]};
    selection-background-color: {c["accent"]};
    selection-color: #FFFFFF;
}}
QListWidget::item, QListView::item {{
    border-bottom: 1px solid {c["border"]};
    padding: 0;
}}
QListWidget::item:hover, QListView::item:hover {{
    background-color: {c["surface"]};
}}
QListWidget::item:selected, QListView::item:selected {{
    background-color: rgba(74, 158, 255, 0.30);
    color: {c["text_primary"]};
}}

/* ── Scroll Bars ───────────────────────────────────────────────────── */
QScrollBar:vertical {{
    background-color: {c["panel"]};
    width: 4px;
    margin: 0;
    border-radius: 2px;
}}
QScrollBar::handle:vertical {{
    background-color: {c["border"]};
    border-radius: 2px;
    min-height: 24px;
}}
QScrollBar::handle:vertical:hover {{
    background-color: {c["text_secondary"]};
}}
QScrollBar::add-line:vertical, QScrollBar::sub-line:vertical,
QScrollBar::add-page:vertical, QScrollBar::sub-page:vertical {{
    background: none;
    border: none;
    height: 0px;
}}
QScrollBar:horizontal {{
    background-color: {c["panel"]};
    height: 4px;
    margin: 0;
    border-radius: 2px;
}}
QScrollBar::handle:horizontal {{
    background-color: {c["border"]};
    border-radius: 2px;
    min-width: 24px;
}}
QScrollBar::handle:horizontal:hover {{
    background-color: {c["text_secondary"]};
}}
QScrollBar::add-line:horizontal, QScrollBar::sub-line:horizontal,
QScrollBar::add-page:horizontal, QScrollBar::sub-page:horizontal {{
    background: none;
    border: none;
    width: 0px;
}}

/* ── Splitter ──────────────────────────────────────────────────────── */
QSplitter::handle {{
    background-color: {c["border"]};
}}
QSplitter::handle:horizontal {{
    width: 2px;
}}
QSplitter::handle:vertical {{
    height: 2px;
}}
QSplitter::handle:hover {{
    background-color: {c["accent"]};
}}

/* ── Menu Bar ──────────────────────────────────────────────────────── */
QMenuBar {{
    background-color: {c["panel"]};
    color: {c["text_primary"]};
    border-bottom: 1px solid {c["border"]};
    padding: 2px;
}}
QMenuBar::item {{
    background-color: transparent;
    padding: 4px 8px;
    border-radius: {r["clip"]}px;
}}
QMenuBar::item:selected {{
    background-color: {c["surface"]};
}}
QMenuBar::item:pressed {{
    background-color: {c["border"]};
}}

/* ── Menus ─────────────────────────────────────────────────────────── */
QMenu {{
    background-color: {c["panel"]};
    color: {c["text_primary"]};
    border: 1px solid {c["border"]};
    border-radius: {r["clip"]}px;
    padding: 4px;
}}
QMenu::item {{
    padding: 6px 24px 6px 12px;
    border-radius: {r["clip"]}px;
}}
QMenu::item:selected {{
    background-color: {c["surface"]};
}}
QMenu::item:disabled {{
    color: {c["text_disabled"]};
}}
QMenu::separator {{
    height: 1px;
    background-color: {c["border"]};
    margin: 4px 0;
}}

/* ── Status Bar ────────────────────────────────────────────────────── */
QStatusBar {{
    background-color: {c["panel"]};
    color: {c["text_secondary"]};
    border-top: 1px solid {c["border"]};
    padding: 0 {sp["panel_padding"]}px;
}}
QStatusBar::item {{
    border: none;
}}

/* ── Dialogs ───────────────────────────────────────────────────────── */
QDialog {{
    background-color: {c["panel"]};
    color: {c["text_primary"]};
    border: 1px solid {c["border"]};
    border-radius: {r["dialog"]}px;
}}

/* ── Progress Bar ──────────────────────────────────────────────────── */
QProgressBar {{
    background-color: {c["surface"]};
    color: {c["text_primary"]};
    border: 1px solid {c["border"]};
    border-radius: {r["clip"]}px;
    text-align: center;
    min-height: 8px;
    max-height: 8px;
}}
QProgressBar::chunk {{
    background-color: {c["accent"]};
    border-radius: {r["clip"]}px;
}}

/* ── Text Inputs ───────────────────────────────────────────────────── */
QLineEdit {{
    background-color: {c["surface"]};
    color: {c["text_primary"]};
    border: 1px solid {c["border"]};
    border-radius: {r["clip"]}px;
    padding: 4px {sp["item_padding"]}px;
    selection-background-color: {c["accent"]};
}}
QLineEdit:focus {{
    border-color: {c["accent"]};
}}
QLineEdit:disabled {{
    color: {c["text_disabled"]};
}}
QLineEdit[placeholder] {{
    color: {c["text_secondary"]};
}}

/* ── ComboBox ──────────────────────────────────────────────────────── */
QComboBox {{
    background-color: {c["surface"]};
    color: {c["text_primary"]};
    border: 1px solid {c["border"]};
    border-radius: {r["clip"]}px;
    padding: 4px {sp["item_padding"]}px;
    min-height: 24px;
}}
QComboBox:hover {{
    border-color: {c["accent"]};
}}
QComboBox::drop-down {{
    border: none;
    width: 20px;
}}
QComboBox::down-arrow {{
    width: 8px;
    height: 8px;
}}
QComboBox QAbstractItemView {{
    background-color: {c["panel"]};
    color: {c["text_primary"]};
    border: 1px solid {c["border"]};
    selection-background-color: {c["accent"]};
    outline: none;
}}

/* ── SpinBox ───────────────────────────────────────────────────────── */
QSpinBox, QDoubleSpinBox {{
    background-color: {c["surface"]};
    color: {c["text_primary"]};
    border: 1px solid {c["border"]};
    border-radius: {r["clip"]}px;
    padding: 4px;
    min-height: 24px;
}}
QSpinBox:focus, QDoubleSpinBox:focus {{
    border-color: {c["accent"]};
}}
QSpinBox::up-button, QSpinBox::down-button,
QDoubleSpinBox::up-button, QDoubleSpinBox::down-button {{
    background-color: {c["border"]};
    border: none;
    width: 16px;
}}

/* ── CheckBox ──────────────────────────────────────────────────────── */
QCheckBox {{
    color: {c["text_primary"]};
    spacing: 6px;
}}
QCheckBox::indicator {{
    width: 14px;
    height: 14px;
    border: 1px solid {c["border"]};
    border-radius: 3px;
    background-color: {c["surface"]};
}}
QCheckBox::indicator:checked {{
    background-color: {c["accent"]};
    border-color: {c["accent"]};
}}

/* ── Slider ────────────────────────────────────────────────────────── */
QSlider::groove:horizontal {{
    background-color: {c["surface"]};
    height: 4px;
    border-radius: 2px;
}}
QSlider::handle:horizontal {{
    background-color: {c["accent"]};
    border: none;
    width: 14px;
    height: 14px;
    margin: -5px 0;
    border-radius: 7px;
}}
QSlider::sub-page:horizontal {{
    background-color: {c["accent"]};
    border-radius: 2px;
}}

/* ── Toolbar ───────────────────────────────────────────────────────── */
QToolBar {{
    background-color: {c["panel"]};
    border-bottom: 1px solid {c["border"]};
    spacing: {sp["unit"]}px;
    padding: {sp["unit"]}px;
}}

/* ── Size Grip ─────────────────────────────────────────────────────── */
QSizeGrip {{
    background-color: transparent;
    width: 12px;
    height: 12px;
}}
"""


def apply_theme(app: QApplication) -> None:
    """Apply the Tempo dark theme to the QApplication.

    Must be called after QApplication is created and after setStyle("Fusion").

    Args:
        app: The QApplication instance.
    """
    # Load fonts from assets if available
    from pathlib import Path

    fonts_dir = Path(__file__).parent.parent / "assets" / "fonts"
    if fonts_dir.exists():
        for font_file in fonts_dir.glob("*.ttf"):
            QFontDatabase.addApplicationFont(str(font_file))
        for font_file in fonts_dir.glob("*.otf"):
            QFontDatabase.addApplicationFont(str(font_file))

    # Set application-wide default font
    default_font = QFont(str(FONTS["ui"]), int(FONTS["size_base"]))
    app.setFont(default_font)

    # Apply the stylesheet
    app.setStyleSheet(make_stylesheet())
