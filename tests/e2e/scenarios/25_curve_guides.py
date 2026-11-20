"""Curve graph: dashed guides are drawn at Start Ratio (20 %) and End Ratio (80 %) in the outline colour and not elsewhere; after dragging a key it is still selected, so Delete removes it."""

from _helpers import add_transition_at_cut, curve, graph, point
from tempo_e2e import expect, expect_eq


def run(s):
    s.open_panel("Inspector")
    s.fit_timeline()
    add_transition_at_cut(s, 0)
    s.fill(s.row_widget("Start Ratio", "GtkSpinButton"), "20")
    s.fill(s.row_widget("End Ratio", "GtkSpinButton"), "80")
    s.wait_for(lambda: abs(s.transitions("V1")[0]["start_ratio"] - 0.2) < 0.01 and abs(s.transitions("V1")[0]["end_ratio"] - 0.8) < 0.01, "both ratios")
    rect = graph(s)["rect"]
    shot = s.screenshot("1_guides")
    bg = shot.pixel(int(point(rect, 0.5, 0.97)[0]), int(point(rect, 0.5, 0.97)[1]))  # inside the box, clear of guides and curve
    border = shot.pixel(int(point(rect, 0.5, 1.0)[0]), int(point(rect, 0.5, 1.0)[1]))
    s.note("graph_bg_border", [bg, border])
    dist = lambda a, b: sum(abs(p - q) for p, q in zip(a, b))  # noqa: E731

    def guide_pixels(v, places):
        """Pixels near level v (within one row) that have the outline colour."""
        x_y = [(point(rect, u / 100, v)[0], point(rect, u / 100, v)[1]) for u in places]
        return sum(1 for x, y in x_y if any(dist(shot.pixel(int(x), int(y) + dy), border) < 40 for dy in (-1, 0, 1)))

    expect(dist(bg, border) > 40, f"the outline colour differs from the background: {border} vs {bg}")
    low = guide_pixels(0.2, range(55, 95))   # the curve is far above here
    high = guide_pixels(0.8, range(5, 45))   # and far below here
    none = guide_pixels(0.5, range(80, 95))  # no guide at 50 %
    s.note("guide_pixels", [low, high, none])
    expect(low >= 12, f"a guide is drawn at the Start Ratio level: {low} outline-coloured pixels")
    expect(high >= 12, f"a guide is drawn at the End Ratio level: {high} outline-coloured pixels")
    expect(none <= 2, f"no guide at 50 %: {none}")

    # A dragged key stays selected: Delete then removes it, not the transition.
    s.double_click(*point(graph(s)["rect"], 0.5, 0.5))
    keys = s.wait_for(lambda: curve(s), "a double-click to add a key")
    x, y = point(graph(s)["rect"], keys[0][0], 0.2 + 0.6 * keys[0][1])
    s.drag(x, y, x, y - 20)
    s.wait_for(lambda: curve(s) and curve(s)[0][1] > keys[0][1] + 0.05, "the key to move up")
    s.wait_for(lambda: s.widget(type="GtkDrawingArea", tooltip="Double-click")["focus"], "the graph to keep keyboard focus")
    s.screenshot("2_dragged")
    s.key("Delete")
    s.wait_for(lambda: curve(s) == [], "Delete to remove the dragged key")
    expect_eq(len(s.transitions("V1")), 1, "the transition stays")
