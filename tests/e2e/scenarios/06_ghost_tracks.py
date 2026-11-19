"""Ghost tracks: drag a Media Pool clip into the band above V1; V2 and A2 are made with the clip; one Ctrl+Z removes them."""

from tempo_e2e import center, expect_eq


def names(s):
    return sorted(s.track_name(t) for t in s.tracks())


def run(s):
    s.fit_timeline()
    expect_eq(names(s), ["A1", "V1"], "tracks before")
    item = s.widget(text="lime-6s.mp4", type="GtkLabel")
    band = s.geometry()["timeline"]["ghost_band"]
    bx, by, bw, bh = band
    # Over the band at about 2 s, a quarter of the way along.
    s.drag(*center(item["rect"]), bx + bw * 0.25, by + bh / 2, hold=0.4, before_release=lambda: s.screenshot("0_during_drag"))
    s.wait_for(lambda: names(s) == ["A1", "A2", "V1", "V2"], "V2 and A2 to be made", timeout=8)
    for name in ("V2", "A2"):
        clips = s.track(name)["clips"]
        expect_eq(len(clips), 1, f"clips on {name}")
        expect_eq(clips[0]["name"], "lime-6s.mp4", f"the clip on {name}")
    expect_eq(len(s.track("V1")["clips"]), 3, "V1 is untouched")
    s.screenshot("1_dropped")

    s.key("ctrl+z")
    s.wait_for(lambda: names(s) == ["A1", "V1"], "one Ctrl+Z to remove V2 and A2")
    expect_eq(len(s.track("V1")["clips"]), 3, "V1 after undo")
    s.screenshot("2_undone")
