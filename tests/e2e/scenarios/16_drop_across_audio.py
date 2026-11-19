"""Regression: a transition dragged over A1 first (refused there) and then onto a V1 cut is accepted and placed."""

from tempo_e2e import center, expect_eq, expect_near


def run(s):
    s.open_panel("Effects")
    s.fit_timeline()
    item = s.widget(text="Cross Dissolve", type="GtkLabel")
    cut_x, cut_y = s.cut_point("V1", 1)
    a1 = s.clip("A1", 1)["rect"]
    over_audio = (cut_x, a1[1] + a1[3] / 2)
    # Rest on the A1 cut (a refused place) before moving up to the V1 cut.
    s.drag(*center(item["rect"]), cut_x, cut_y, via=[over_audio, over_audio], hold=0.3)
    placed = s.wait_for(lambda: s.transitions("V1"), "the transition to be placed after crossing A1")
    expect_eq(len(s.transitions("A1")), 0, "nothing on A1")
    g = s.transition_geo("V1", 0)
    expect_near((g["start_us"] + g["end_us"]) / 2, 8_000_000, 33_334, "centred on the 8 s cut")
    s.note("kind", placed[0]["kind"])
