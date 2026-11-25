"""Multi-clip batch Inspector editing, TitleData font/color/background controls + undo, and Audio Transitions > Crossfade & Filters drag/click."""

from tempo_e2e import center, expect, expect_eq, expect_near


def timeline_clip(s, track, index):
    clips = sorted(s.track(track)["clips"], key=lambda c: c["timeline_in"])
    return clips[index]


def run(s):
    s.open_panel("Effects")
    s.open_panel("Inspector")
    s.fit_timeline()

    # 1. Single-clip edit so two clips differ on Opacity, then Ctrl+click second clip -> "Mixed" + batch edit + 1 Ctrl+Z.
    s.click(*center(s.clip("V1", 0)["rect"]))
    v0_id = s.clip("V1", 0)["id"]
    s.wait_for(lambda: v0_id in s.state()["selection"], "first clip selected")

    s.fill(s.row_widget("Opacity", "GtkSpinButton"), "50")
    s.wait_for(lambda: abs(timeline_clip(s, "V1", 0)["properties"]["opacity"] - 0.5) < 0.02, "first clip opacity 50%")

    v1_id = s.clip("V1", 1)["id"]
    s.click(*center(s.clip("V1", 1)["rect"]), mods=("ctrl",))
    s.wait_for(lambda: v0_id in s.state()["selection"] and v1_id in s.state()["selection"], "two video clips selected together")

    opacity_labels = [w["text"] for w in s.row_widgets("Opacity", "GtkLabel")]
    expect("Mixed" in opacity_labels, f"Opacity shows Mixed badge when clips differ: {opacity_labels}")
    s.screenshot("1_batch_mixed")

    s.fill(s.row_widget("Opacity", "GtkSpinButton"), "75")
    s.wait_for(
        lambda: abs(timeline_clip(s, "V1", 0)["properties"]["opacity"] - 0.75) < 0.02
        and abs(timeline_clip(s, "V1", 1)["properties"]["opacity"] - 0.75) < 0.02,
        "batch Opacity updates all selected video clips",
    )
    expect("Mixed" not in [w["text"] for w in s.row_widgets("Opacity", "GtkLabel")], "Mixed badge clears after batch edit")

    # Batch-edit Volume % across both linked audio clips as well.
    s.fill(s.row_widget("Volume %", "GtkSpinButton"), "60")
    s.wait_for(
        lambda: abs(timeline_clip(s, "A1", 0)["properties"]["volume"] - 0.6) < 0.02
        and abs(timeline_clip(s, "A1", 1)["properties"]["volume"] - 0.6) < 0.02,
        "batch Volume updates both selected audio clips",
    )
    s.screenshot("2_batch_edited")

    # Single Ctrl+Z reverts Volume across both A1 clips; next Ctrl+Z reverts Opacity across both V1 clips.
    s.key("ctrl+z")
    s.wait_for(
        lambda: abs(timeline_clip(s, "A1", 0)["properties"]["volume"] - 1.0) < 0.02
        and abs(timeline_clip(s, "A1", 1)["properties"]["volume"] - 1.0) < 0.02,
        "single Ctrl+Z reverts batch Volume across all selected audio clips",
    )
    s.key("ctrl+z")
    s.wait_for(
        lambda: abs(timeline_clip(s, "V1", 0)["properties"]["opacity"] - 0.5) < 0.02
        and abs(timeline_clip(s, "V1", 1)["properties"]["opacity"] - 1.0) < 0.02,
        "single Ctrl+Z reverts batch Opacity back to 50% and 100%",
    )

    # 2. TitleData controls: add Text title at end of timeline, edit font_family & background_color + undo.
    s.key("End")
    s.wait_for(lambda: s.state()["playhead_us"] >= 11_900_000, "playhead at end of timeline")
    text_effect = s.scroll_into_view(lambda: s.widget(text="Text", type="GtkLabel", hidden=True))
    s.click(*center(text_effect["rect"]))
    s.wait_for(lambda: len(s.track("V1")["clips"]) == 4, "Text title added on V1")
    expect_eq(timeline_clip(s, "V1", 3)["clip_type"], "Title", "added clip is Title")

    s.fill(s.row_widget("Text", "GtkEntry"), "Intro Title")
    s.wait_for(lambda: timeline_clip(s, "V1", 3)["title_data"]["text"] == "Intro Title", "title text saved")

    s.fill(s.row_widget("Font", "GtkEntry"), "Monospace")
    s.wait_for(lambda: timeline_clip(s, "V1", 3)["title_data"]["font_family"] == "Monospace", "font_family saved")

    color_btn = s.row_widget("Color", "GtkColorDialogButton")
    expect(color_btn["sensitive"], "Title Color dialog button is sensitive")
    bg_btn = s.row_widget("Background", "GtkColorDialogButton")
    expect(not bg_btn["sensitive"], "Background Color dialog button starts disabled when transparent")

    bg_toggle = s.row_widget("Background", "GtkCheckButton")
    s.click(*center(bg_toggle["rect"]))
    s.wait_for(lambda: timeline_clip(s, "V1", 3)["title_data"]["background_color"] is not None, "background_color enabled")
    expect(s.row_widget("Background", "GtkColorDialogButton")["sensitive"], "Background Color dialog button becomes sensitive")
    s.screenshot("3_title_controls")

    s.click(*center(s.row_widget("Background", "GtkCheckButton")["rect"]))
    s.wait_for(lambda: timeline_clip(s, "V1", 3)["title_data"]["background_color"] is None, "background_color cleared back to transparent")

    s.key("ctrl+z")
    s.wait_for(lambda: timeline_clip(s, "V1", 3)["title_data"]["background_color"] is not None, "Ctrl+Z restores background_color")
    s.key("ctrl+z")
    s.wait_for(lambda: timeline_clip(s, "V1", 3)["title_data"]["background_color"] is None, "Ctrl+Z clears background_color again")
    s.key("ctrl+z")
    s.wait_for(lambda: timeline_clip(s, "V1", 3)["title_data"]["font_family"] == "Sans", "Ctrl+Z restores font_family to Sans")

    # 3. Audio Transitions > Crossfade visibility + click & drag application on audio clips, plus Filters drag.
    s.fit_timeline()
    audio_heading = s.scroll_into_view(lambda: s.widget(text="Audio Transitions", type="GtkLabel", hidden=True))
    expect(audio_heading is not None, "Audio Transitions heading visible")
    crossfade = s.scroll_into_view(lambda: s.widget(text="Crossfade", type="GtkLabel", hidden=True))

    # Click Crossfade with A1[0] selected -> adds Crossfade on A1.
    s.click(*center(s.clip("A1", 0)["rect"]))
    s.wait_for(lambda: s.clip("A1", 0)["id"] in s.state()["selection"], "A1 clip 0 selected")
    s.click(*center(crossfade["rect"]))
    s.wait_for(lambda: len(s.transitions("A1")) >= 1, "clicking Crossfade applies audio transition")
    expect_eq(s.transitions("A1")[0]["kind"], "Crossfade", "clicked audio transition kind")
    s.key("ctrl+z")
    s.wait_for(lambda: len(s.transitions("A1")) == 0, "Ctrl+Z removes clicked Crossfade")

    # Drag Crossfade onto the second cut on A1 (at 8 s).
    crossfade = s.scroll_into_view(lambda: s.widget(text="Crossfade", type="GtkLabel", hidden=True))
    cut_x, cut_y = s.cut_point("A1", 1)
    s.drag(*center(crossfade["rect"]), cut_x, cut_y)
    placed = s.wait_for(lambda: s.transitions("A1"), "dragging Crossfade onto A1 cut places audio transition")
    expect_eq(placed[0]["kind"], "Crossfade", "dragged audio transition kind")
    geo = s.transition_geo("A1", 0)
    expect_near((geo["start_us"] + geo["end_us"]) / 2, 8_000_000, 33_334, "Crossfade centred on A1 cut")

    # Drag a plugin Filter onto V1 clip 0.
    bw = s.scroll_into_view(lambda: s.widget(text="Black and White", type="GtkLabel", hidden=True))
    s.drag(*center(bw["rect"]), *center(s.clip("V1", 0)["rect"]))
    s.wait_for(
        lambda: any(e["id"] == "dev.tempo.filters/black-and-white" for e in timeline_clip(s, "V1", 0)["properties"]["effects"]),
        "dragging Black and White filter applies it to V1 clip",
    )
    s.screenshot("4_effects_applied")
