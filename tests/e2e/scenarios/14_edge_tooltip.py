"""Transition edge drag: while dragging, a black tooltip box is drawn by the pointer (Resolve's "+ss:ff / ss:ff"); dragging outwards lengthens it (the "+" sign)."""

from _helpers import add_transition_at_cut
from tempo_e2e import expect

# A box below and right of the pointer (draw_tip in timeline.rs: x + 12, y + 18).
TIP = (14, 20, 40, 26)


def tip_look(image, x, y):
    """(share of near-black pixels, share of bright pixels) where the tooltip goes."""
    dx, dy, w, h = TIP
    dark = bright = n = 0
    for j in range(int(y + dy), int(y + dy + h), 2):
        for i in range(int(x + dx), int(x + dx + w), 2):
            r, g, b = image.pixel(i, j)
            dark += r + g + b < 60
            bright += min(r, g, b) > 170
            n += 1
    return dark / n, bright / n


def run(s):
    s.open_panel("Inspector")
    s.fit_timeline()
    add_transition_at_cut(s, 1)
    before = s.transitions("V1")[0]["duration_us"]
    x, y, w, h = s.transition_geo("V1", 0)["rect"]
    sx, sy, tx = x + w - 2, y + h / 2, x + w + 40
    idle = tip_look(s.screenshot("0_before"), tx, sy)
    seen = {}

    def look():
        seen["tip"] = tip_look(s.screenshot("1_dragging"), tx, sy)

    s.drag(sx, sy, tx, sy, before_release=look)
    after = s.wait_for(lambda: s.transitions("V1")[0]["duration_us"] > before and s.transitions("V1")[0]["duration_us"], "the drag to lengthen the transition")
    s.note("length_us", [before, after])
    s.note("tip_dark_bright_idle_vs_dragging", [idle, seen["tip"]])
    dark, bright = seen["tip"]
    expect(dark > 0.4 and bright > 0.02, f"a black tooltip with white text shows while dragging: dark {dark:.2f}, bright {bright:.2f}")
    expect(idle[0] < dark - 0.2, f"and it is not there before the drag: {idle} vs {seen['tip']}")
