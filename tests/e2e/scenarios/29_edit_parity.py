"""Edit page parity: track header context menu, Shift+Scroll track height resize, and Alt+drag clip duplication."""

from tempo_e2e import center, expect, expect_eq, expect_near

SHIFT_KEYSYM = 0xFFE1  # MODIFIERS["shift"] in tempo_e2e.py


def track_names(s):
    return sorted(s.track_name(t) for t in s.tracks())


def track_geo(s, name):
    return next(t for t in s.geometry()["timeline"]["tracks"] if t["name"] == name)


def shift_scroll(s, x, y, steps):
    s._rd("NotifyKeyboardKeysym", "(ub)", SHIFT_KEYSYM, True)
    try:
        s.scroll(x, y, steps)
    finally:
        s._rd("NotifyKeyboardKeysym", "(ub)", SHIFT_KEYSYM, False)
    s._after_input()


def run(s):
    s.fit_timeline()
    expect_eq(track_names(s), ["A1", "V1"], "initial tracks")
    expect_eq(len(s.track("V1")["clips"]), 3, "initial V1 clips")
    expect_eq(len(s.track("A1")["clips"]), 3, "initial A1 clips")

    # 1. Track header right-click menu: Delete Track is disabled when only 1 track of that kind exists.
    v1_btn = s.widget(text="V1", type="GtkButton")
    s.right_click(*center(v1_btn["rect"]))
    del_lbl = s.widget(text="Delete Track", type="GtkLabel")
    geo = s.geometry()
    del_btn = s.ancestor(del_lbl, "GtkModelButton", geo) or del_lbl
    expect(not del_btn["sensitive"] or not del_lbl["sensitive"], "Delete Track is disabled on last video track")

    # Add Video Track -> V2 appears.
    s.click(*center(s.widget(text="Add Video Track", type="GtkLabel")["rect"]))
    s.wait_for(lambda: track_names(s) == ["A1", "V1", "V2"], "V2 added from track header menu")

    # Right-click A1: Delete Track is disabled on last audio track; Add Audio Track (Stereo) -> A2 appears.
    a1_btn = s.widget(text="A1", type="GtkButton")
    s.right_click(*center(a1_btn["rect"]))
    del_lbl = s.widget(text="Delete Track", type="GtkLabel")
    geo = s.geometry()
    del_btn = s.ancestor(del_lbl, "GtkModelButton", geo) or del_lbl
    expect(not del_btn["sensitive"] or not del_lbl["sensitive"], "Delete Track is disabled on last audio track")
    s.click(*center(s.widget(text="Add Audio Track (Stereo)", type="GtkLabel")["rect"]))
    s.wait_for(lambda: track_names(s) == ["A1", "A2", "V1", "V2"], "A2 (Stereo) added from track header menu")
    expect_eq(s.track("A2")["audio_type"], "Stereo", "A2 audio type")

    # Right-click V2: Delete Track is enabled; Delete Track removes V2; Ctrl+Z undoes; Ctrl+Shift+Z redoes.
    v2_btn = s.widget(text="V2", type="GtkButton")
    s.right_click(*center(v2_btn["rect"]))
    del_lbl = s.widget(text="Delete Track", type="GtkLabel")
    geo = s.geometry()
    del_btn = s.ancestor(del_lbl, "GtkModelButton", geo) or del_lbl
    expect(del_btn["sensitive"] and del_lbl["sensitive"], "Delete Track is enabled when extra video track exists")
    s.click(*center(del_lbl["rect"]))
    s.wait_for(lambda: track_names(s) == ["A1", "A2", "V1"], "V2 deleted from track header menu")
    s.screenshot("1_track_deleted")

    s.key("ctrl+z")
    s.wait_for(lambda: track_names(s) == ["A1", "A2", "V1", "V2"], "Ctrl+Z restores deleted V2 track")

    s.key("ctrl+shift+z")
    s.wait_for(lambda: track_names(s) == ["A1", "A2", "V1"], "Ctrl+Shift+Z re-deletes V2 track")

    # Undo A2 addition too so we verify clean track count before next steps.
    s.key("ctrl+z")
    s.wait_for(lambda: track_names(s) == ["A1", "A2", "V1", "V2"], "Ctrl+Z restores V2")
    s.key("ctrl+z")
    s.wait_for(lambda: track_names(s) == ["A1", "V1", "V2"], "Ctrl+Z removes A2")
    s.key("ctrl+z")
    s.wait_for(lambda: track_names(s) == ["A1", "V1"], "Ctrl+Z removes V2")

    # 2. Shift+Scroll changes track height without zooming horizontally.
    pps_before = s.geometry()["timeline"]["pps"]
    v1_h0 = track_geo(s, "V1")["rect"][3]
    a1_h0 = track_geo(s, "A1")["rect"][3]
    cx, cy = center(s.clip("V1", 0)["rect"])

    shift_scroll(s, cx, cy, -3)
    v1_h1 = s.wait_for(
        lambda: track_geo(s, "V1")["rect"][3] > v1_h0 + 5 and track_geo(s, "V1")["rect"][3],
        "Shift+Scroll up increases V1 track height",
    )
    a1_h1 = track_geo(s, "A1")["rect"][3]
    expect(a1_h1 > a1_h0 + 5, f"A1 height grew from {a1_h0} to {a1_h1}")
    expect_near(s.geometry()["timeline"]["pps"], pps_before, 0.05, "Shift+Scroll does not zoom horizontally")
    s.screenshot("2_track_resized_up")

    # Scroll down to shrink and check minimum clamp (36 px).
    shift_scroll(s, cx, cy, 15)
    v1_h2 = s.wait_for(
        lambda: track_geo(s, "V1")["rect"][3] < v1_h1 and track_geo(s, "V1")["rect"][3],
        "Shift+Scroll down shrinks V1 track height",
    )
    expect(v1_h2 >= 36.0, f"V1 track height clamped >= 36 px (got {v1_h2})")
    expect_near(s.geometry()["timeline"]["pps"], pps_before, 0.05, "Shift+Scroll down does not zoom horizontally")

    # Restore comfortable height for Alt+drag.
    shift_scroll(s, cx, cy, -3)
    s.wait_for(lambda: track_geo(s, "V1")["rect"][3] >= 55.0, "track height restored")

    # 3. Alt+drag duplicates a clip (and linked audio companion) while leaving original intact; single Ctrl+Z removes duplicate.
    # Zoom out twice so empty space after 12 s is visible on canvas.
    s.key("ctrl+minus")
    s.key("ctrl+minus")
    s.settle()

    geo = s.geometry()
    pps = geo["timeline"]["pps"]
    orig_v0 = s.clip("V1", 0, geo)
    orig_a0 = s.clip("A1", 0, geo)
    orig_v_in, orig_v_out = orig_v0["in_us"], orig_v0["out_us"]
    orig_a_in, orig_a_out = orig_a0["in_us"], orig_a0["out_us"]

    from_x, from_y = center(orig_v0["rect"])
    # Drag ~14 seconds right so the 4 s clip lands around 14..18 s (well past the 12 s end).
    to_x = from_x + 14.0 * pps
    s.drag(from_x, from_y, to_x, from_y, mods=("alt",))

    s.wait_for(lambda: len(s.track("V1")["clips"]) == 4 and len(s.track("A1")["clips"]) == 4, "Alt+drag duplicates V1 and linked A1 clips")
    v_clips = sorted(s.track("V1")["clips"], key=lambda c: c["timeline_in"])
    a_clips = sorted(s.track("A1")["clips"], key=lambda c: c["timeline_in"])
    # Original clip 0 stays intact at 0..4 s.
    expect_eq((v_clips[0]["timeline_in"], v_clips[0]["timeline_out"]), (orig_v_in, orig_v_out), "original V1 clip intact")
    expect_eq((a_clips[0]["timeline_in"], a_clips[0]["timeline_out"]), (orig_a_in, orig_a_out), "original A1 clip intact")
    # Duplicate landed after 12 s with its own link group.
    expect(v_clips[3]["timeline_in"] >= 12_000_000, f"duplicate V1 clip placed after 12 s: {v_clips[3]}")
    expect_eq(v_clips[3]["properties"]["link"], a_clips[3]["properties"]["link"], "duplicated pair shares link")
    expect(v_clips[3]["properties"]["link"] != v_clips[0]["properties"]["link"], "duplicate link differs from original")
    s.screenshot("3_alt_drag_duplicated")

    # Single Ctrl+Z removes both duplicated clips while preserving the original.
    s.key("ctrl+z")
    s.wait_for(lambda: len(s.track("V1")["clips"]) == 3 and len(s.track("A1")["clips"]) == 3, "single Ctrl+Z removes duplicated V1 and A1 clips")
    v_after = sorted(s.track("V1")["clips"], key=lambda c: c["timeline_in"])
    expect_eq((v_after[0]["timeline_in"], v_after[0]["timeline_out"]), (orig_v_in, orig_v_out), "original V1 clip preserved after undo")
    s.screenshot("4_alt_drag_undone")
