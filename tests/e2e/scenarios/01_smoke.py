"""Smoke: the app starts on the private Wayland display, fullscreen, and draws the Edit page."""

from tempo_e2e import expect, expect_eq


def run(s):
    state = s.state()
    expect(state["fullscreen"], "the window is fullscreen")
    expect(state["project_loaded"], "the fixture project is open")
    expect_eq([s.track_name(t) for t in s.tracks()], ["V1", "A1"], "tracks")
    geo = s.geometry()
    expect_eq(geo["window"][2:], list(s.size), "window size")
    expect(len(geo["timeline"]["clips"]) == 6, f"six clips are on the timeline: {len(geo['timeline']['clips'])}")
    shot = s.screenshot("edit_page")
    expect_eq((shot.width, shot.height), tuple(s.size), "screenshot size")
    # Not black: the Edit page has panels, text and coloured clips.
    expect(shot.brightness() > 8, f"the screen is not black (mean brightness {shot.brightness():.1f})")
    clip = s.clip("V1", 0, geo)
    x, y, w, h = clip["rect"]
    r, g, b = shot.mean([x + 4, y + 4, max(w - 8, 1), max(h - 30, 1)])
    expect(r + g + b > 60, f"the first clip is drawn on the timeline (mean colour {r, g, b})")
