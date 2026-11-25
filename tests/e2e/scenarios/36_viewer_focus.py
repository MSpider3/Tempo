"""Ctrl+3 / Ctrl+5 pick the viewer the keys act on. One viewer: Ctrl+3 shows the selected Media Pool clip there and keeps the layout. Dual viewer: after Ctrl+3, L plays the source viewer (not the timeline), I/O mark it and Loop loops its own In..Out; Ctrl+5, Ctrl+4 or a click in the timeline hands the keys back to the timeline viewer."""

import re
import time

from tempo_e2e import center, expect, expect_eq

TC = re.compile(r"^(\d\d):(\d\d):(\d\d):(\d\d)$")


def surfaces(s):
    return sorted(s.widgets(type="TempoVideoSurface"), key=lambda w: w["rect"][0])


def source_seconds(s):
    """The source viewer's timecode (the left viewer of the two), in seconds."""
    geo = s.geometry()
    x, _, w, _ = surfaces(s)[0]["rect"]
    for label in s.widgets(css="tempo-timecode", type="GtkLabel", geometry=geo):
        m = TC.match(label["text"] or "")
        if m and x <= center(label["rect"])[0] <= x + w:
            h, mi, sec, f = (int(v) for v in m.groups())
            return h * 3600 + mi * 60 + sec + f / 30
    raise AssertionError("no timecode over the source viewer")


def run(s):
    s.open_panel("Media Pool")
    s.fit_timeline()
    card = s.widget(text="blue-6s.mp4", type="GtkLabel")
    s.click(*center(card["rect"]))
    s.wait_for(lambda: s.state()["media_selection"], "the Media Pool clip to be selected")

    # 1. One viewer: Ctrl+3 shows the clip in it; there is still one viewer.
    s.key("ctrl+3")
    s.wait_for(lambda: s.widgets(text="Back to Timeline"), "Ctrl+3 to show the source clip in the viewer")
    expect_eq(len(surfaces(s)), 1, "viewers after Ctrl+3 with one viewer")
    s.key("ctrl+5")
    s.wait_for(lambda: not s.widgets(text="Back to Timeline"), "Ctrl+5 to go back to the timeline")
    s.screenshot("1_single")

    # 2. Dual viewer from the viewer's options menu.
    s.click(*center(s.widget(tooltip="Viewer options")["rect"]))
    s.click(*center(s.widget(text="Dual viewer")["rect"]))
    s.wait_for(lambda: len(surfaces(s)) == 2, "the second viewer")
    s.key("Escape")
    s.settle()

    # 3. Ctrl+3: L plays the source viewer; the timeline stays still.
    s.key("ctrl+3")
    start = source_seconds(s)
    s.key("l")
    s.wait_for(lambda: source_seconds(s) > start + 0.5, "L to play the source viewer")
    expect(not s.state()["playing"], "the timeline does not play after Ctrl+3")
    s.key("k")

    # 4. I at 1 s, O at 2 s (Shift+Right steps a second), Loop on, Space: it stays in 1..2 s.
    s.key("Home")
    s.wait_for(lambda: source_seconds(s) < 0.05, "Home to the start of the source")
    s.key("shift+Right")
    s.key("i")
    s.key("shift+Right")
    s.key("o")
    s.wait_for(lambda: abs(source_seconds(s) - 2.0) < 0.05, "the source viewer at 2 s")
    s.key("ctrl+/")
    s.key("space")
    seen = []
    end = time.monotonic() + 3.0
    while time.monotonic() < end:
        seen.append(source_seconds(s))
    s.key("space")
    s.note("loop_positions", seen)
    expect(not s.state()["playing"], "the timeline does not play")
    expect(max(seen) < 2.2, f"the source viewer loops its own In..Out (1 s..2 s): {seen}")
    expect(min(seen) < 1.6, f"the source viewer came round to In again: {seen}")
    s.screenshot("2_source_loop")

    # 5. Ctrl+5: the keys act on the timeline viewer again.
    s.key("ctrl+5")
    s.key("l")
    s.wait_for(lambda: s.state()["playing"], "L to play the timeline after Ctrl+5")
    s.key("k")

    # 6. Ctrl+4 (Timeline window active) also hands the keys back to the timeline viewer.
    s.wait_for(lambda: not s.state()["playing"], "K to stop the timeline")
    s.key("ctrl+3")
    s.key("ctrl+4")
    s.key("l")
    s.wait_for(lambda: s.state()["playing"], "L to play the timeline after Ctrl+3, Ctrl+4")
    s.key("k")
    s.wait_for(lambda: not s.state()["playing"], "K to stop the timeline")

    # 7. So does a click in the timeline.
    s.key("ctrl+3")
    x, y, w, h = s.widget(type="TempoTimelineCanvas")["rect"]
    s.click(x + w // 2, y + h - 10)
    s.key("l")
    s.wait_for(lambda: s.state()["playing"], "L to play the timeline after Ctrl+3 and a timeline click")
    s.key("k")
