"""Resolve-style Clip Attributes dialog (Media Pool & Timeline): Video FPS conform, Audio channel mapping, Clip Name, undo/redo, and persistence."""

from tempo_e2e import center, expect, expect_eq


def stack_tab(s, title):
    geo = s.geometry()
    switcher = s.widget(type="GtkStackSwitcher", geometry=geo)
    sx, sy, sw, sh = switcher["rect"]
    lbl = next(
        w
        for w in s.widgets(text=title, type="GtkLabel", geometry=geo)
        if sx <= w["rect"][0] <= sx + sw and sy <= w["rect"][1] <= sy + sh
    )
    return s.ancestor(lbl, "GtkToggleButton", geo)


def run(s):
    s.open_panel("Media Pool")
    s.fit_timeline()

    # 1. Media Pool Clip Attributes on red-6s.mp4
    red_lbl = s.widget(text="red-6s.mp4", type="GtkLabel")
    s.right_click(*center(red_lbl["rect"]))
    s.click(*center(s.widget(text="Clip Attributes…")["rect"]))
    s.wait_for(lambda: s.widgets(text="Clip Attributes", type="GtkLabel"), "Clip Attributes dialog opens")
    s.settle()
    s.screenshot("1_media_pool_clip_attributes")

    # Conform Video Frame Rate to 60 fps (2x speed on 30 fps fixture).
    s.click(*center(s.row_widget("Video Frame Rate", "GtkDropDown")["rect"]))
    s.click(*center(s.widget(text="60 fps", type="GtkLabel")["rect"]))
    s.wait_for(lambda: s.widgets(contains="2.00× speed", type="GtkLabel"), "live conform speed preview updates")

    # Audio tab: Channel 1 (Left -> Both).
    s.click(*center(stack_tab(s, "Audio")["rect"]))
    s.wait_for(lambda: s.widgets(text="Channel Mapping", type="GtkLabel"), "Audio tab visible")
    s.click(*center(s.row_widget("Channel Mapping", "GtkDropDown")["rect"]))
    s.click(*center(s.widget(text="Channel 1 (Left -> Both)", type="GtkLabel")["rect"]))

    # Name tab: rename project display name without renaming disk file.
    s.click(*center(stack_tab(s, "Name")["rect"]))
    s.wait_for(lambda: s.widgets(text="Clip Name", type="GtkLabel"), "Name tab visible")
    name_entry = s.row_widget("Clip Name", "GtkEntry")
    s.fill(name_entry, "Hero Take 1", enter=False)

    s.click(*center(s.widget(text="Apply")["rect"]))
    s.wait_for(lambda: not s.widgets(text="Clip Attributes", type="GtkLabel"), "dialog closes")
    s.wait_for(
        lambda: any(src.get("display_name") == "Hero Take 1" for src in s.state()["project"]["sources"].values()),
        "source display_name saved",
    )
    red_src = next(src for src in s.state()["project"]["sources"].values() if src.get("display_name") == "Hero Take 1")
    expect_eq(red_src.get("audio_channel_map"), "LeftToBoth", "source channel map")
    expect_eq(red_src.get("conform_fps"), {"num": 60, "den": 1}, "source conform fps")
    s.wait_for(lambda: s.widgets(text="Hero Take 1", type="GtkLabel"), "renamed Media Pool label")
    s.wait_for(lambda: s.widgets(text="0:03", type="GtkLabel"), "conformed 0:03 badge")
    expect(s.widgets(tooltip="(conformed)"), "conformed tooltip shown in Media Pool")
    s.screenshot("2_media_pool_renamed")

    # 2. Timeline Clip Attributes on first V1 clip + undo/redo check.
    v1_clip = s.clip("V1", 0)
    old_name = v1_clip["name"]
    s.right_click(*center(v1_clip["rect"]))
    s.click(*center(s.widget(text="Clip Attributes…")["rect"]))
    s.wait_for(lambda: s.widgets(text="Clip Attributes", type="GtkLabel"), "Timeline Clip Attributes opens")
    s.settle()

    # Audio tab -> Swap Left / Right.
    s.click(*center(stack_tab(s, "Audio")["rect"]))
    s.wait_for(lambda: s.widgets(text="Channel Mapping", type="GtkLabel"), "Audio tab visible")
    s.click(*center(s.row_widget("Channel Mapping", "GtkDropDown")["rect"]))
    s.click(*center(s.widget(text="Swap Left / Right", type="GtkLabel")["rect"]))

    # Name tab -> Opening Shot.
    s.click(*center(stack_tab(s, "Name")["rect"]))
    s.wait_for(lambda: s.widgets(text="Clip Name", type="GtkLabel"), "Name tab visible")
    s.fill(s.row_widget("Clip Name", "GtkEntry"), "Opening Shot", enter=False)
    s.screenshot("3_timeline_clip_attributes")

    s.click(*center(s.widget(text="Apply")["rect"]))
    s.wait_for(lambda: s.clip("V1", 0)["name"] == "Opening Shot", "timeline clip renamed")

    def a1_channel_map():
        a1 = next(t for t in s.state()["project"]["timeline"]["tracks"] if t["kind"] == "Audio" and t["kind_index"] == 1)
        return a1["clips"][0]["properties"]["audio_channel_map"]

    expect_eq(a1_channel_map(), "SwapLeftRight", "linked A1 channel map updated")

    # Undo/redo atomically via CommandLog.
    s.key("ctrl+z")
    s.wait_for(lambda: s.clip("V1", 0)["name"] == old_name, "undo restores clip name")
    expect_eq(a1_channel_map(), "Stereo", "undo restores audio channel map")
    s.key("ctrl+shift+z")
    s.wait_for(lambda: s.clip("V1", 0)["name"] == "Opening Shot", "redo restores clip name")
    expect_eq(a1_channel_map(), "SwapLeftRight", "redo restores audio channel map")
