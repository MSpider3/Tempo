"""Curve graph keys: click a key, Delete removes the key and not the transition; BackSpace does the same."""

from _helpers import add_transition_at_cut, curve, graph, point
from tempo_e2e import expect_eq


def run(s):
    s.open_panel("Inspector")
    s.fit_timeline()
    add_transition_at_cut(s, 0)
    for n, key in enumerate(("Delete", "BackSpace")):
        s.double_click(*point(graph(s)["rect"], 0.5, 0.5))
        keys = s.wait_for(lambda: curve(s), f"a double-click to add a key ({key})")
        expect_eq(len(keys), 1, "keys after the double-click")
        s.click(*point(graph(s)["rect"], *keys[0]))  # selects the key, focuses the graph
        s.wait_for(lambda: s.widget(type="GtkDrawingArea", tooltip="Double-click")["focus"], "the graph to have keyboard focus")
        s.screenshot(f"{n}_key_selected")
        s.key(key)
        s.wait_for(lambda: curve(s) == [], f"{key} to remove the key")
        expect_eq(len(s.transitions("V1")), 1, f"{key} leaves the transition on the timeline")
        expect_eq(len(s.track("V1")["clips"]), 3, f"{key} leaves the clips alone")
