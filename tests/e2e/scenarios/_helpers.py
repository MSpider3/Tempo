"""Steps several scenarios share. Not a scenario (no number prefix)."""

from tempo_e2e import center


def select_cut(s, index, track="V1"):
    """Click the index-th cut low in the row: selects the edit point there."""
    x, _ = s.cut_point(track, index)
    s.click(x, s.clip(track, index)["edit_y"])
    s.wait_for(lambda: (s.state()["edit_selection"] or {}).get("kind") == "edit", f"cut {index} to be selected")


def add_transition_at_cut(s, index):
    """Ctrl+T on a cut, then click the new V1 transition so the Inspector shows it."""
    before = len(s.transitions("V1"))
    select_cut(s, index)
    s.key("ctrl+t")
    s.wait_for(lambda: len(s.transitions("V1")) > before, "Ctrl+T to add a transition")
    spans = sorted(s.geometry()["timeline"]["transitions"], key=lambda g: abs((g["start_us"] + g["end_us"]) / 2 - s.clip("V1", index + 1)["in_us"]))
    s.click(*center(spans[0]["rect"]))
    s.wait_for(lambda: s.state()["selected_transition"], "the transition to be selected")


def seek_by_ruler(s, seconds):
    """Click the ruler at `seconds` (measured from the first clip); returns the playhead."""
    geo = s.geometry()
    first = s.clip("V1", 0, geo)
    ruler = geo["timeline"]["ruler"]
    s.click(first["rect"][0] + seconds * geo["timeline"]["pps"], ruler[1] + ruler[3] / 2)
    return s.wait_for(lambda: abs(s.state()["playhead_us"] - seconds * 1e6) < 60_000 and s.state()["playhead_us"], f"the playhead near {seconds} s")


def viewer_colour(s, name):
    """Mean colour of the middle of the viewer picture, once the screen is still."""
    surface = s.widget(type="TempoVideoSurface")
    cx, cy = center(surface["rect"])
    return s.screenshot(name).mean([cx - 20, cy - 20, 40, 40])
