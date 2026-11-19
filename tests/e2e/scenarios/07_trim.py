"""Trim: with nothing selected, Shift+[ trims the head of the clip under the playhead to it and Shift+] its tail; Ctrl+Z restores."""

from tempo_e2e import expect, expect_eq, expect_near

FRAME_US = 33_334


def seek_by_ruler(s, seconds):
    """Click the ruler where `seconds` is (from the clips' positions)."""
    geo = s.geometry()
    first = s.clip("V1", 0, geo)
    pps = geo["timeline"]["pps"]
    ruler = geo["timeline"]["ruler"]
    s.click(first["rect"][0] + seconds * pps, ruler[1] + ruler[3] / 2)
    return s.wait_for(lambda: abs(s.state()["playhead_us"] - seconds * 1e6) < 100_000 and s.state()["playhead_us"], f"the playhead near {seconds} s")


def v1(s):
    return sorted(s.track("V1")["clips"], key=lambda c: c["timeline_in"])


def run(s):
    s.fit_timeline()
    before = v1(s)
    expect_eq(s.state()["selection"], [], "nothing is selected")

    playhead = seek_by_ruler(s, 5.5)
    s.key("shift+[")
    s.wait_for(lambda: v1(s)[1]["timeline_in"] != before[1]["timeline_in"], "Shift+[ to trim the clip under the playhead")
    after = v1(s)
    expect_near(after[1]["timeline_in"], playhead, FRAME_US, "the head is trimmed to the playhead")
    expect_eq(after[1]["timeline_out"], before[1]["timeline_out"], "its tail stays")
    expect(after[1]["timeline_in"] > after[0]["timeline_out"], "a gap is left before it")
    s.screenshot("1_trim_start")

    playhead = seek_by_ruler(s, 10.5)
    s.key("shift+]")
    s.wait_for(lambda: v1(s)[2]["timeline_out"] != before[2]["timeline_out"], "Shift+] to trim the tail of the clip under the playhead")
    expect_near(v1(s)[2]["timeline_out"], playhead, FRAME_US, "the tail is trimmed to the playhead")
    s.screenshot("2_trim_end")

    s.key("ctrl+z")
    s.key("ctrl+z")
    s.wait_for(lambda: [(c["timeline_in"], c["timeline_out"]) for c in v1(s)] == [(c["timeline_in"], c["timeline_out"]) for c in before], "two Ctrl+Z to restore both clips")
