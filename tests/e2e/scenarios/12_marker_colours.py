"""Marker popup: 16 colour dots, a point marker's Duration reads 00:00:00:01, clicking Cocoa colours the marker Cocoa."""

from _helpers import seek_by_ruler
from tempo_e2e import center, expect_eq

COLOURS = ["Blue", "Cyan", "Green", "Yellow", "Red", "Pink", "Purple", "Fuchsia", "Rose", "Lavender", "Sky", "Mint", "Lemon", "Sand", "Cocoa", "Cream"]


def run(s):
    s.fit_timeline()
    seek_by_ruler(s, 2.0)
    s.key("ctrl+m")
    s.widget(text="Markers", type="GtkLabel")
    geo = s.geometry()
    dots = [w for w in s.widgets(type="GtkToggleButton", geometry=geo) if s.ancestor(w, "GtkFlowBox", geo)]
    s.note("colour_dots", [d["tooltip"] for d in dots])
    expect_eq(len(dots), 16, "colour dots in the marker popup")
    for name in COLOURS:
        expect_eq(sum(d["tooltip"] == name for d in dots), 1, f"one {name} dot")
    expect_eq(s.row_widget("Duration", "GtkEntry")["text"], "00:00:00:01", "a point marker's Duration")

    cocoa = next(d for d in dots if d["tooltip"] == "Cocoa")
    s.click(*center(cocoa["rect"]))
    s.wait_for(lambda: next(w for w in s.widgets(type="GtkToggleButton") if w["tooltip"] == "Cocoa")["active"], "the Cocoa dot to be on")
    s.screenshot("1_cocoa")
    s.click(*center(s.widget(text="Done")["rect"]))
    s.wait_for(lambda: not s.widgets(text="Markers", type="GtkLabel"), "the popup to close")
    markers = s.timeline()["markers"]
    expect_eq(len(markers), 1, "markers")
    expect_eq(markers[0]["color"], "Cocoa", "marker colour")
    s.screenshot("2_marker")
