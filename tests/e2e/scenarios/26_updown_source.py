"""Up/Down while a source clip is shown in the viewer: the timeline playhead steps from edit to edit starting at its own position, not at the source position."""

from _helpers import seek_by_ruler
from tempo_e2e import center, expect_near

S = 1_000_000


def step(s, key, want_us):
    s.key(key)
    s.wait_for(lambda: abs(s.state()["playhead_us"] - want_us) < 40_000, f"{key} to put the timeline playhead at {want_us / S:.0f} s (it is at {s.state()['playhead_us'] / S:.2f} s)")


def run(s):
    s.open_panel("Media Pool")
    s.fit_timeline()
    seek_by_ruler(s, 4.0)
    card = s.widget(text="blue-6s.mp4", type="GtkLabel")
    s.click(*center(card["rect"]))
    s.double_click(*center(card["rect"]))
    s.settle()
    bar = max((w for w in s.widgets(type="GtkScale") if w["rect"][2] > 600), key=lambda w: w["rect"][2])
    x, y, w, h = bar["rect"]
    s.click(x + 12 + 0.9 * (w - 24), y + h / 2)  # the source is near its end (about 5.4 s), far from any timeline edit
    s.settle()
    expect_near(s.state()["playhead_us"], 4 * S, 60_000, "the timeline playhead stays at 4 s while the source is scrubbed")
    s.screenshot("1_source_shown")
    step(s, "Down", 8 * S)
    step(s, "Down", 12 * S)
    step(s, "Up", 8 * S)
    step(s, "Up", 4 * S)
    s.screenshot("2_after")
