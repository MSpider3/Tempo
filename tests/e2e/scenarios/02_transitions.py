"""Transitions: drag from Effects onto a cut, drag its edge, copy/paste with Up, delete and undo, Ctrl+T / Alt+T / Shift+T."""

from tempo_e2e import center, expect, expect_eq, expect_near

FRAME_US = 33_334


def run(s):
    s.open_panel("Effects")
    s.fit_timeline()
    item = s.widget(text="Cross Dissolve", type="GtkLabel")

    # 1. Drag Cross Dissolve from the Effects panel onto the cut at 8 s on V1.
    cut_x, cut_y = s.cut_point("V1", 1)
    s.drag(*center(item["rect"]), cut_x, cut_y)
    s.screenshot("1_dropped")
    t = s.wait_for(lambda: s.transitions("V1"), "a transition on V1")[0]
    expect_eq(t["kind"], "CrossDissolve", "dropped kind")
    expect_eq(t["alignment"], "Centered", "dropped alignment")
    geo = s.transition_geo("V1", 0)
    expect_near((geo["start_us"] + geo["end_us"]) / 2, 8_000_000, FRAME_US, "the transition is centred on the cut")
    length = t["duration_us"]

    # 2. Drag its right edge outwards: it gets longer.
    x, y, w, h = geo["rect"]
    s.drag(x + w - 2, y + h / 2, x + w + 40, y + h / 2)
    s.screenshot("2_edge_dragged")
    longer = s.wait_for(lambda: s.transitions("V1")[0]["duration_us"] > length and s.transitions("V1")[0], "the transition to get longer")
    expect_near((s.transition_geo("V1", 0)["start_us"] + s.transition_geo("V1", 0)["end_us"]) / 2, 8_000_000, FRAME_US, "still centred after the edge drag")

    # 3. As in the recording: playhead on the 8 s cut, click the transition, Ctrl+C,
    #    Up (previous edit from the playhead: 4 s; the selection follows), Ctrl+V.
    for k in ("Home", "Down", "Down"):
        s.key(k)
    s.wait_for(lambda: s.state()["playhead_us"] == 8_000_000, "the playhead on the 8 s cut")
    s.click(*center(s.transition_geo("V1", 0)["rect"]))
    s.wait_for(lambda: s.state()["selected_transition"], "the transition to be selected")
    s.key("ctrl+c")
    copied = s.wait_for(lambda: s.state()["transition_clipboard"], "the transition to be copied")
    expect_eq(copied["duration_us"], longer["duration_us"], "copied length")
    s.key("Up")
    edit = s.wait_for(lambda: s.state()["edit_selection"], "an edit point to be selected")
    expect_eq((edit["kind"], edit["position_us"]), ("edit", 4_000_000), "Up selects the previous edit")
    s.key("ctrl+v")
    pasted = s.wait_for(lambda: len(s.transitions("V1")) == 2 and s.transitions("V1"), "the pasted transition")
    spans = sorted((g["start_us"], g["end_us"]) for g in s.geometry()["timeline"]["transitions"] if g["track"] == "V1")
    expect_near(sum(spans[0]) / 2, 4_000_000, FRAME_US, "the paste is centred on the 4 s cut")
    expect_near(spans[0][1] - spans[0][0], longer["duration_us"], FRAME_US, "the paste has the copied length")
    s.screenshot("3_pasted")

    # 4. Backspace removes the selected transition; Ctrl+Z brings it back.
    s.click(*center(s.transition_geo("V1", 0)["rect"]))
    s.wait_for(lambda: s.state()["selected_transition"], "the 4 s transition to be selected")
    s.key("BackSpace")
    s.wait_for(lambda: len(s.transitions("V1")) == 1, "Backspace to remove the transition")
    s.key("ctrl+z")
    s.wait_for(lambda: len(s.transitions("V1")) == 2, "Ctrl+Z to restore it")
    s.screenshot("4_undone")

    # 5. Alt+T: video only, at the end of the last clip (an edit point clicked low in the row).
    last = s.clip("V1", 2)
    s.click(last["rect"][0] + last["rect"][2] - 2, last["edit_y"])
    s.wait_for(lambda: (s.state()["edit_selection"] or {}).get("position_us") == 12_000_000, "the edit at 12 s to be selected")
    s.key("alt+t")
    s.wait_for(lambda: len(s.transitions("V1")) == 3, "Alt+T to add a video transition")
    expect_eq(len(s.transitions("A1")), 0, "Alt+T leaves the audio alone")
    # 6. Shift+T on the same edit: audio only.
    s.key("shift+t")
    a1 = s.wait_for(lambda: s.transitions("A1"), "Shift+T to add an audio transition")
    expect_eq(a1[0]["kind"], "Crossfade", "audio transition kind")
    expect_eq(len(s.transitions("V1")), 3, "Shift+T leaves the video alone")
    # 7. Ctrl+T at the start of the first clip: both.
    first = s.clip("V1", 0)
    s.click(first["rect"][0] + 2, first["edit_y"])
    s.wait_for(lambda: (s.state()["edit_selection"] or {}).get("position_us") == 0, "the edit at 0 s to be selected")
    s.key("ctrl+t")
    s.wait_for(lambda: len(s.transitions("V1")) == 4 and len(s.transitions("A1")) == 2, "Ctrl+T to add video and audio transitions")
    expect(all(tr["kind"] == "CrossDissolve" for tr in s.transitions("V1")), "video transitions are Cross Dissolves")
    s.screenshot("5_keys")
