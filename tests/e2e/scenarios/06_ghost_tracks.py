"""Ghost tracks: drag a Media Pool clip into the band above V1; ghost lanes V2 and A2 show mid-drag, then V2 and A2 are made with the clip; one Ctrl+Z removes them."""

from tempo_e2e import center, expect, expect_eq


def names(s):
    return sorted(s.track_name(t) for t in s.tracks())


def run(s):
    s.fit_timeline()
    expect_eq(names(s), ["A1", "V1"], "tracks before")
    item = s.widget(text="lime-6s.mp4", type="GtkLabel")
    band = s.geometry()["timeline"]["ghost_band"]
    bx, by, bw, bh = band
    # Over the band at about 2 s, a quarter of the way along.
    seen = {}

    def look():
        shot = s.screenshot("0_during_drag")
        seen["lanes"] = {t["name"]: t["rect"] for t in s.geometry()["timeline"]["tracks"] if t["ghost"]}
        # The ghost V2 lane fills the band; the drop preview draws the clip box in it.
        seen["band_brightness"] = shot.brightness(band)

    idle_brightness = s.screenshot().brightness(band)
    s.drag(*center(item["rect"]), bx + bw * 0.25, by + bh / 2, hold=0.4, before_release=look)
    s.note("mid_drag", seen)
    expect_eq(sorted(seen["lanes"]), ["A2", "V2"], "ghost lanes V2 and A2 show while dragging over the band")
    expect_eq(seen["lanes"]["V2"], band, "the V2 ghost lane fills the band")
    expect(abs(seen["band_brightness"] - idle_brightness) > 3, f"the band is drawn differently while dragging: {idle_brightness:.1f} -> {seen['band_brightness']:.1f}")
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
