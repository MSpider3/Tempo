"""Transition curve graph: double-click adds a key, dragging it up raises its value, right-click removes it."""

from _helpers import add_transition_at_cut
from tempo_e2e import expect, expect_eq, expect_near

PAD = 10.0  # GRAPH_PAD in crates/tempo-ui/src/inspector.rs


def graph(s):
    return s.scroll_into_view(lambda: s.widget(type="GtkDrawingArea", tooltip="Double-click"))


def point(rect, u, v):
    x, y, w, h = rect
    return (x + PAD + u * (w - 2 * PAD), y + PAD + (1 - v) * (h - 2 * PAD))


def curve(s):
    return s.transitions("V1")[0]["curve"]


def run(s):
    s.open_panel("Inspector")
    s.fit_timeline()
    add_transition_at_cut(s, 0)
    expect_eq(curve(s), [], "no keys at first")

    # A linear transition: the curve passes through (0.5, 0.5).
    s.double_click(*point(graph(s)["rect"], 0.5, 0.5))
    keys = s.wait_for(lambda: curve(s), "a double-click to add a key")
    expect_eq(len(keys), 1, "keys after the double-click")
    expect_near(keys[0][0], 0.5, 0.05, "key place")
    expect_near(keys[0][1], 0.5, 0.05, "key value")
    s.screenshot("1_key_added")

    rect = graph(s)["rect"]
    x, y = point(rect, *keys[0])
    s.drag(x, y, x, y - 30)
    moved = s.wait_for(lambda: curve(s)[0][1] > keys[0][1] + 0.1 and curve(s), "dragging the key up to raise it")
    expect_eq(len(moved), 1, "still one key")
    s.note("key_after_drag", moved[0])
    s.screenshot("2_key_dragged")

    s.right_click(*point(graph(s)["rect"], *moved[0]))
    s.wait_for(lambda: curve(s) == [], "a right-click to remove the key")
    expect(s.transitions("V1")[0]["ease"] == "None", "Ease is untouched")
