"""Dragging a Media Pool clip over V1 shows its landing box in the video clip colour (not green) on V1 and a green box on A1."""

from tempo_e2e import center, expect, expect_eq

LAND_S = 8.3  # inside the gap left by lifting the third clip (8-12 s)


def run(s):
    s.fit_timeline()
    blue = s.clip("V1", 2)
    s.click(*center(blue["rect"]))
    s.key("BackSpace")
    s.wait_for(lambda: len(s.track("V1")["clips"]) == 2, "Backspace to lift the third clip")

    geo = s.geometry()
    tl = geo["timeline"]
    first = s.clip("V1", 0, geo)
    v1 = next(t for t in tl["tracks"] if t["name"] == "V1")["rect"]
    a1 = next(t for t in tl["tracks"] if t["name"] == "A1")["rect"]
    reference = s.screenshot("0_before")
    clip_bar = reference.mean([first["rect"][0] + first["rect"][2] - 40, v1[1] + v1[3] - 14, 20, 8])
    x = first["rect"][0] + LAND_S * tl["pps"]
    # Far right of the box, clear of the drag icon by the pointer.
    probe_x = tl["canvas"][0] + tl["canvas"][2] - 40
    seen = {}

    def look():
        shot = s.screenshot("1_dragging")
        seen["v1_body"] = shot.mean([probe_x, v1[1] + 8, 20, 12])
        seen["v1_bar"] = shot.mean([probe_x, v1[1] + v1[3] - 14, 20, 8])
        seen["a1"] = shot.mean([probe_x, a1[1] + 8, 20, 12])
        seen["empty_before"] = reference.mean([probe_x, v1[1] + 8, 20, 12])

    s.drag(*center(s.widget(text="lime-6s.mp4", type="GtkLabel")["rect"]), x, v1[1] + v1[3] / 2, hold=0.4, before_release=look)
    s.note("colours", {**seen, "clip_bar": clip_bar})
    body, bar, a1_box = seen["v1_body"], seen["v1_bar"], seen["a1"]
    expect(body != seen["empty_before"], "a landing box is drawn on V1")
    expect(body[2] > body[1] + 15 and body[2] > body[0] + 15, f"the V1 box is blue, the video clip colour, not green: {body}")
    expect(all(abs(a - b) < 30 for a, b in zip(bar, clip_bar)), f"its name bar matches the clips' name bar: {bar} vs {clip_bar}")
    expect(a1_box[1] > a1_box[0] + 15 and a1_box[1] > a1_box[2] + 5, f"the A1 box is green: {a1_box}")
    s.wait_for(lambda: len(s.track("V1")["clips"]) == 3, "the drop to place the clip")
    expect_eq(s.track("V1")["clips"][-1]["name"], "lime-6s.mp4", "the dropped clip")
