"""Dissolve pixels at the middle of the 8 s cut (lime to blue): Additive is brighter than Cross Dissolve, Non-Additive is the per-channel maximum, Blur has softer edges in the middle than at the start of the mix."""

from _helpers import seek_by_ruler, viewer_colour
from tempo_e2e import center, expect

CUT_S = 8.0
KINDS = {"Cross Dissolve": "CrossDissolve", "Additive Dissolve": "AdditiveDissolve", "Non-Additive Dissolve": "NonAdditiveDissolve", "Blur Dissolve": "BlurDissolve"}


def add(s, name):
    s.drag(*center(s.widget(text=name, type="GtkLabel")["rect"]), *s.cut_point("V1", 1))
    t = s.wait_for(lambda: s.transitions("V1"), f"{name} on the cut")[0]
    expect(t["kind"] == KINDS[name], f"{name} stored as {t['kind']}")
    return t


def picture_edge(shot, rect):
    """x of the picture's left edge on the viewer's middle row: the first pixel far from the background."""
    x0, y0, w, h = rect
    y = int(y0 + h / 2)
    bg = shot.pixel(int(x0) + 4, y)
    return next(x for x in range(int(x0) + 4, int(x0 + w)) if sum(abs(a - b) for a, b in zip(shot.pixel(x, y), bg)) > 150)


def edge_roughness(shot, rect, edge):
    """Largest step between neighbouring pixels on the middle row around the picture's edge (a blurred edge steps less)."""
    y = int(rect[1] + rect[3] / 2)
    row = [shot.pixel(edge + i, y) for i in range(-40, 40)]
    return max(sum(abs(a - b) for a, b in zip(p, q)) for p, q in zip(row, row[1:]))


def run(s):
    s.open_panel("Effects")
    s.fit_timeline()
    surface = s.widget(type="TempoVideoSurface")
    seen = {}
    picture = surface["rect"]
    edge = None
    for name in KINDS:
        t = add(s, name)
        seek_by_ruler(s, CUT_S)
        seen[name] = viewer_colour(s, name.split()[0])
        if edge is None:
            edge = picture_edge(s.screenshot("edge"), picture)
        if name == "Blur Dissolve":
            seen["blur_mid"] = edge_roughness(s.screenshot("blur_mid"), picture, edge)
            geo = s.transition_geo("V1", 0)
            seek_by_ruler(s, geo["start_us"] / 1e6 + 0.02)
            seen["blur_start"] = edge_roughness(s.screenshot("blur_start"), picture, edge)
        s.note(name, seen[name])
        s.key("ctrl+z")
        s.wait_for(lambda: not s.transitions("V1"), f"undo of {name}")
    s.note("blur_edges", [seen["blur_start"], seen["blur_mid"]])
    cross, add_, lighten = seen["Cross Dissolve"], seen["Additive Dissolve"], seen["Non-Additive Dissolve"]
    expect(sum(add_) > sum(cross) + 60, f"Additive is brighter than Cross: {add_} vs {cross}")
    expect(sum(lighten) > sum(cross) + 60, f"Non-Additive is brighter than Cross: {lighten} vs {cross}")
    lime, blue = (0, 255, 0), (0, 0, 255)
    want = tuple(max(a, b) for a, b in zip(lime, blue))
    expect(all(abs(a - b) < 30 for a, b in zip(lighten, want)), f"Non-Additive is the per-channel max {want}: {lighten}")
    expect(seen["blur_mid"] < seen["blur_start"] * 0.8, f"Blur softens the edges in the middle: start {seen['blur_start']}, mid {seen['blur_mid']}")
