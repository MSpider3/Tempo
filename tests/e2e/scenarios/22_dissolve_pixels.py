"""Dissolve pixels at the middle of the 8 s cut (lime to blue): Additive is brighter than Cross Dissolve, Non-Additive is the per-channel maximum; a Blur Dissolve widens a sharp edge inside the picture at the middle of the mix, not at its start."""

import subprocess

from _helpers import seek_by_ruler, viewer_colour
from tempo_e2e import center, expect

CUT_S = 8.0
KINDS = {"Cross Dissolve": "CrossDissolve", "Additive Dissolve": "AdditiveDissolve", "Non-Additive Dissolve": "NonAdditiveDissolve", "Blur Dissolve": "BlurDissolve"}


def add(s, name, track="V1", index=1):
    s.drag(*center(s.widget(text=name, type="GtkLabel")["rect"]), *s.cut_point(track, index))
    t = s.wait_for(lambda: s.transitions(track), f"{name} on the cut")[0]
    expect(t["kind"] == KINDS[name], f"{name} stored as {t['kind']}")
    return t


def split_clip(s):
    """A 6 s clip, lime on the left half and blue on the right: a sharp edge in the middle of
    the picture. The fixture's clips are solid colours, and a blur cannot show inside those
    (it stays inside the picture, whose border is sharp whatever the blur)."""
    out = s.tmp / "split-6s.mp4"
    encoders = subprocess.run(["ffmpeg", "-hide_banner", "-encoders"], capture_output=True, text=True, env=s.env, cwd=s.tmp).stdout
    h264 = "libx264" if "libx264" in encoders else "libopenh264"
    picture = "color=c=lime:size=640x360:rate=30:duration=6,drawbox=x=320:y=0:w=320:h=360:color=blue:t=fill"
    subprocess.run(["ffmpeg", "-v", "error", "-y", "-f", "lavfi", "-i", picture, "-pix_fmt", "yuv420p", "-g", "30", "-c:v", h264, str(out)], check=True, env=s.env, cwd=s.tmp, timeout=60)
    return out


def edge_width(shot, rect):
    """Pixels across the picture's middle (lime to blue) on the middle row that are between
    10 % and 90 % of the way from one colour to the other. A sharp edge is a pixel or two;
    a blurred one is several radii wide."""
    cx, cy = (int(v) for v in center(rect))
    left, right = shot.pixel(cx - 60, cy), shot.pixel(cx + 60, cy)
    span = [b - a for a, b in zip(left, right)]
    norm = sum(d * d for d in span)
    if norm < 100 * 100:
        raise AssertionError(f"no lime/blue edge in the viewer: {left} {right}")

    def t(p):
        return sum((c - a) * d for c, a, d in zip(p, left, span)) / norm

    return sum(1 for x in range(cx - 50, cx + 50) if 0.1 < t(shot.pixel(x, cy)) < 0.9)


def run(s):
    s.open_panel("Effects")
    s.fit_timeline()
    surface = s.widget(type="TempoVideoSurface")
    seen = {}
    picture = surface["rect"]
    for name in ("Cross Dissolve", "Additive Dissolve", "Non-Additive Dissolve"):
        add(s, name)
        seek_by_ruler(s, CUT_S)
        seen[name] = viewer_colour(s, name.split()[0])
        s.note(name, seen[name])
        s.key("ctrl+z")
        s.wait_for(lambda: not s.transitions("V1"), f"undo of {name}")
    cross, add_, lighten = seen["Cross Dissolve"], seen["Additive Dissolve"], seen["Non-Additive Dissolve"]
    expect(sum(add_) > sum(cross) + 60, f"Additive is brighter than Cross: {add_} vs {cross}")
    expect(sum(lighten) > sum(cross) + 60, f"Non-Additive is brighter than Cross: {lighten} vs {cross}")
    lime, blue = (0, 255, 0), (0, 0, 255)
    want = tuple(max(a, b) for a, b in zip(lime, blue))
    expect(all(abs(a - b) < 30 for a, b in zip(lighten, want)), f"Non-Additive is the per-channel max {want}: {lighten}")

    # Blur Dissolve, on a cut inside the split clip on V2 (setup: import it, drop it above V1, razor it).
    path = split_clip(s)
    s.action(f"import:{path}")
    s.open_panel("Media Pool")
    s.fit_timeline()
    card = s.widget(text=path.name, type="GtkLabel")
    bx, by, bw, bh = s.geometry()["timeline"]["ghost_band"]
    s.drag(*center(card["rect"]), bx + bw * 0.1, by + bh / 2, hold=0.4)
    s.wait_for(lambda: s.track("V2") and len(s.track("V2")["clips"]) == 1, "the split clip on V2", timeout=8)
    s.fit_timeline()
    placed = s.clip("V2", 0)
    seek_by_ruler(s, (placed["in_us"] + placed["out_us"]) / 2e6)
    s.key("ctrl+\\")
    s.wait_for(lambda: len(s.track("V2")["clips"]) == 2, "Ctrl+\\ to split the V2 clip")
    s.fit_timeline()
    add(s, "Blur Dissolve", "V2", 0)
    geo = s.transition_geo("V2", 0)
    seek_by_ruler(s, (geo["start_us"] + geo["end_us"]) / 2e6)
    seen["blur_mid"] = edge_width(s.screenshot("blur_mid"), picture)
    seek_by_ruler(s, geo["start_us"] / 1e6 + 0.02)
    seen["blur_start"] = edge_width(s.screenshot("blur_start"), picture)
    s.note("blur_edge_widths", [seen["blur_start"], seen["blur_mid"]])
    # The same picture on both sides of the cut: without the blur the middle is as sharp as the start.
    expect(seen["blur_mid"] >= seen["blur_start"] + 4, f"Blur widens the edge in the middle: start {seen['blur_start']} px, mid {seen['blur_mid']} px")
