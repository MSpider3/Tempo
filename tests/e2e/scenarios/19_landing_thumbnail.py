"""Dragging a Media Pool clip over V1: the landing box shows the clip's thumbnail (lime pixels for the lime clip), and no lime was there before."""

from tempo_e2e import center, expect

LAND_S = 8.3  # inside the gap left by lifting the third clip (8-12 s)


def lime_share(shot, x0, x1, y0, y1, step=6):
    """Share of sampled pixels in the area that are clearly lime (green high, red and blue low)."""
    hits = total = 0
    for y in range(int(y0), int(y1), step):
        for x in range(int(x0), int(x1), step):
            r, g, b = shot.pixel(x, y)[:3]
            hits += g > 150 and r < 90 and b < 90
            total += 1
    return hits / max(total, 1)


def run(s):
    s.fit_timeline()
    s.click(*center(s.clip("V1", 2)["rect"]))
    s.key("BackSpace")
    s.wait_for(lambda: len(s.track("V1")["clips"]) == 2, "Backspace to lift the third clip")

    geo = s.geometry()
    tl = geo["timeline"]
    v1 = next(t for t in tl["tracks"] if t["name"] == "V1")["rect"]
    first = s.clip("V1", 0, geo)
    gap_x0 = first["rect"][0] + 8.0 * tl["pps"]
    gap_x1 = first["rect"][0] + 12.0 * tl["pps"]
    area = (gap_x0 + 4, gap_x1 - 4, v1[1] + 6, v1[1] + v1[3] - 24)  # above the name bar
    before = lime_share(s.screenshot("0_before"), *area)
    seen = {}

    def look():
        shot = s.screenshot("1_dragging")
        seen["during"] = lime_share(shot, *area)

    x = first["rect"][0] + LAND_S * tl["pps"]
    s.drag(*center(s.widget(text="lime-6s.mp4", type="GtkLabel")["rect"]), x, v1[1] + v1[3] / 2, hold=0.4, before_release=look)
    s.note("lime_share", {"before": before, "during": seen.get("during")})
    expect(before < 0.01, f"the gap has no lime before the drag: {before:.3f}")
    expect(seen["during"] > 0.08, f"the landing box shows the lime thumbnail: lime share {seen['during']:.3f}")
