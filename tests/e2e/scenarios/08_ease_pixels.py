"""Pixels: a Dip To Color dissolve from red, measured in the viewer a quarter of the way in, is brighter with Ease In than linear."""

from tempo_e2e import center, expect

QUARTER_S = 3.75  # the dip runs 3.5-4.5 s around the cut at 4 s


def viewer_colour(s, name):
    """Mean colour of the middle of the picture, once the viewer has stopped changing."""
    surface = s.widget(type="TempoVideoSurface")
    cx, cy = center(surface["rect"])
    return s.screenshot(name).mean([cx - 20, cy - 20, 40, 40])


def run(s):
    s.open_panel("Effects")
    s.open_panel("Inspector")
    s.fit_timeline()
    item = s.widget(text="Dip To Color Dissolve", type="GtkLabel")
    s.drag(*center(item["rect"]), *s.cut_point("V1", 0))
    dip = s.wait_for(lambda: s.transitions("V1"), "a Dip To Color dissolve")[0]
    expect(dip["kind"] == "DipToColor" and dip["ease"] == "None", f"a linear dip: {dip['kind']} {dip['ease']}")

    geo = s.geometry()
    first, ruler, pps = s.clip("V1", 0, geo), geo["timeline"]["ruler"], geo["timeline"]["pps"]
    s.click(first["rect"][0] + QUARTER_S * pps, ruler[1] + ruler[3] / 2)
    playhead = s.wait_for(lambda: abs(s.state()["playhead_us"] - QUARTER_S * 1e6) < 60_000 and s.state()["playhead_us"], "the playhead a quarter into the dip")
    linear = viewer_colour(s, "1_linear")

    s.click(*center(s.transition_geo("V1", 0)["rect"]))
    s.wait_for(lambda: s.state()["selected_transition"], "the dip to be selected")
    s.click(*center(s.row_widget("Ease", "GtkDropDown")["rect"]))
    s.click(*center(s.widget(text="In", type="GtkLabel")["rect"]))
    s.wait_for(lambda: s.transitions("V1")[0]["ease"] == "In", "Ease In")
    expect(s.state()["playhead_us"] == playhead, "the playhead did not move")
    eased = viewer_colour(s, "2_ease_in")
    s.note("viewer_rgb_linear", linear)
    s.note("viewer_rgb_ease_in", eased)

    # Linear has gone half way to black by now; Ease In (u²) only an eighth of the way.
    expect(linear[0] > 60 and eased[0] > 60, f"the picture is still red-ish: linear {linear}, eased {eased}")
    expect(eased[0] - linear[0] > 30, f"Ease In is clearly brighter than linear: linear {linear}, eased {eased}")
    expect(max(linear[1], linear[2], eased[1], eased[2]) < 40, f"only red, dipping to black: linear {linear}, eased {eased}")
