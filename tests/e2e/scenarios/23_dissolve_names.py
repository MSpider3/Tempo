"""Names: the Effects panel's Dissolve group and the Inspector's Transition Type dropdown list exactly Additive, Blur, Cross, Dip To Color and Non-Additive Dissolve, in that order."""

from _helpers import add_transition_at_cut
from tempo_e2e import center, expect_eq

NAMES = ["Additive Dissolve", "Blur Dissolve", "Cross Dissolve", "Dip To Color Dissolve", "Non-Additive Dissolve"]


def by_height(labels):
    return [w["text"] for w in sorted(labels, key=lambda w: (round(w["rect"][1]), w["rect"][0]))]


def run(s):
    s.open_panel("Effects")
    s.open_panel("Inspector")
    s.fit_timeline()
    listed = by_height([w for w in s.widgets(type="GtkLabel") if (w["text"] or "").endswith(" Dissolve")])
    s.note("effects_panel", listed)
    expect_eq(listed, NAMES, "the Effects panel's dissolves")
    s.screenshot("1_effects")

    add_transition_at_cut(s, 0)
    drop = s.row_widget("Transition Type", "GtkDropDown")
    s.note("selected_type", drop["text"])
    s.click(*center(drop["rect"]))
    popup = s.wait_for(lambda: [w for w in s.widgets(type="GtkLabel") if w["text"] in NAMES and w["rect"][0] > drop["rect"][0] - 20 and w["rect"][1] > drop["rect"][1] + 10], "the dropdown's list")
    expect_eq(by_height(popup), NAMES, "the Inspector's Transition Type list")
    s.screenshot("2_dropdown")
    s.key("Escape")
