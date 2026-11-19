"""Copy a transition on timeline 1 (Ctrl+C), switch to another timeline with the dropdown above the viewer, select an edit there, Ctrl+V: the transition lands on that edit."""

from _helpers import add_transition_at_cut, select_cut
from tempo_e2e import center, expect, expect_eq


def timeline_names(s):
    """The open timeline's name, then the project's other timelines (the state keeps the open one apart)."""
    project = s.state()["project"]
    return [project["timeline"]["name"]] + [t["name"] for t in project["timelines"]]


def run(s):
    s.open_panel("Media Pool")
    s.fit_timeline()
    # A second timeline with the same clips (handles included), made from the Media Pool's menu.
    first_name = s.state()["project"]["timeline"]["name"]
    pool_item = next(w for w in s.widgets(text=first_name, type="GtkLabel") if "tempo-small" in w["css"])
    s.right_click(*center(pool_item["rect"]))
    s.click(*center(s.widget(text="Duplicate Timeline")["rect"]))
    s.wait_for(lambda: len(timeline_names(s)) == 2, "the duplicate timeline")
    names = timeline_names(s)
    s.note("timelines", names)

    # Timeline 1: add a Cross Dissolve at the first cut and copy it.
    s.fit_timeline()
    add_transition_at_cut(s, 0)
    s.key("ctrl+c")
    copied = s.wait_for(lambda: s.state()["transition_clipboard"], "the transition to be copied")
    s.note("clipboard", copied)

    # Switch with the dropdown above the viewer.
    current = s.state()["project"]["timeline"]["name"]
    other = next(n for n in names if n != current)
    s.click(*center(s.widget(text=current, type="GtkLabel", css="tempo-heading")["rect"]))
    s.click(*center(s.widget(text=other, type="GtkLabel")["rect"]))
    s.wait_for(lambda: s.state()["project"]["timeline"]["name"] == other, f"timeline {other!r} to open")
    s.fit_timeline()
    expect_eq(len(s.transitions("V1")), 0, "the other timeline has no transition yet")
    select_cut(s, 1)
    s.key("ctrl+v")
    pasted = s.wait_for(lambda: s.transitions("V1"), "Ctrl+V to paste the transition on the other timeline")
    expect_eq(len(pasted), 1, "pasted transitions")
    expect_eq(pasted[0]["kind"], copied["kind"], "the pasted transition's kind")
    expect_eq(pasted[0]["duration_us"], copied["duration_us"], "the pasted transition's length")
    # It sits on the second cut (8 s), not the first.
    geo = s.transition_geo("V1", 0)
    mid = (geo["start_us"] + geo["end_us"]) / 2
    expect(abs(mid - 8_000_000) < 100_000, f"the pasted transition is centred on the 8 s cut: {mid}")
    s.screenshot("pasted")
