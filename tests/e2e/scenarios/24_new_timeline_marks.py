"""Create New Timeline: Use Project Settings is a check button in the bottom bar left of Cancel and Create; with In/Out set on a source clip, Empty Timeline off and Use Selected Mark In/Out on, the new timeline's clip is as long as the marked span."""

from tempo_e2e import center, expect, expect_eq, expect_near

NAME = "Marked"


def scrubber(s):
    return max((w for w in s.widgets(type="GtkScale") if w["rect"][2] > 600), key=lambda w: w["rect"][2])


def mark_at(s, fraction, key):
    bar = scrubber(s)
    x, y, w, h = bar["rect"]
    s.click(x + 12 + fraction * (w - 24), y + h / 2)
    s.key(key)
    return scrubber(s)["value"]


def run(s):
    s.open_panel("Media Pool")
    card = s.widget(text="lime-6s.mp4", type="GtkLabel")
    s.click(*center(card["rect"]))
    s.double_click(*center(card["rect"]))
    s.wait_for(lambda: s.state()["media_selection"], "the clip to be selected")
    first = mark_at(s, 0.25, "i")
    last = mark_at(s, 0.75, "o")
    span_units = last - first
    s.note("scrub_values", [first, last])
    expect(span_units > 0, f"Out is after In: {first} to {last}")

    s.key("ctrl+n")
    s.widget(text="Create New Timeline", type="GtkLabel")
    geo = s.geometry()
    use_project = s.ancestor(s.widgets(text="Use Project Settings", type="GtkLabel", geometry=geo)[0], "GtkCheckButton", geo)
    cancel, create = s.widget(text="Cancel"), s.widget(text="Create")
    expect(use_project["rect"][0] < cancel["rect"][0] < create["rect"][0], "Use Project Settings is left of Cancel and Create")
    expect(abs(use_project["rect"][1] - create["rect"][1]) < 40, "and in the same bottom bar")
    s.fill(s.row_widget("Timeline Name", "GtkEntry"), NAME, enter=False)
    geo = s.geometry()
    marks = s.ancestor(s.widgets(text="Use Selected Mark In/Out", type="GtkLabel", geometry=geo)[0], "GtkCheckButton", geo)
    empty = s.ancestor(s.widgets(text="Empty Timeline", type="GtkLabel", geometry=geo)[0], "GtkCheckButton", geo)
    s.click(*center(empty["rect"]))
    s.wait_for(lambda: not s.ancestor(s.widgets(text="Empty Timeline", type="GtkLabel")[0], "GtkCheckButton", s.geometry())["active"], "Empty Timeline to be off")
    geo = s.geometry()
    marks = s.ancestor(s.widgets(text="Use Selected Mark In/Out", type="GtkLabel", geometry=geo)[0], "GtkCheckButton", geo)
    expect(marks["sensitive"], "Use Selected Mark In/Out is available with marks on the selected clip")
    s.click(*center(marks["rect"]))
    s.wait_for(lambda: s.ancestor(s.widgets(text="Use Selected Mark In/Out", type="GtkLabel")[0], "GtkCheckButton", s.geometry())["active"], "the marks box to be ticked")
    s.screenshot("1_dialog")
    s.click(*center(s.widget(text="Create")["rect"]))
    s.wait_for(lambda: s.timeline()["name"] == NAME, "the new timeline to open")
    clips = s.track("V1")["clips"]
    expect_eq(len(clips), 1, "clips on V1 of the new timeline")
    length = (clips[0]["timeline_out"] - clips[0]["timeline_in"]) / 1e6
    s.note("clip_length_s", length)
    expect_near(length * 1e6, span_units, 40_000, "the clip is the marked span (the scrubber reads microseconds; one frame of slack)")
