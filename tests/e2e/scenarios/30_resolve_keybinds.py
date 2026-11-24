"""Resolve keybinds & viewer controls: split without selection, join clip + undo, double-M marker edit,
V + Ctrl+T transition at edit point, Y/Alt+Y/Ctrl+Y track/all selection, Ctrl+/ loop toggle,
Z/Alt+Shift+Z viewer zoom, Shift+F full-page viewer, Ctrl+Shift+1 video track toggle."""

from tempo_e2e import expect, expect_eq, expect_near

from _helpers import seek_by_ruler


def run(s):
    s.fit_timeline()
    expect_eq(len(s.track("V1")["clips"]), 3, "initial V1 clip count")
    expect_eq(len(s.track("A1")["clips"]), 3, "initial A1 clip count")
    expect_eq(s.state()["selection"], [], "nothing initially selected")

    # 1. Ctrl+\ splits unlocked clips under playhead even with nothing selected.
    seek_by_ruler(s, 2.0)
    s.key("ctrl+\\")
    s.wait_for(lambda: len(s.track("V1")["clips"]) == 4, "Ctrl+\\ splits V1 clip without selection")
    expect_eq(len(s.track("A1")["clips"]), 4, "Ctrl+\\ also splits unlocked A1 clip")

    # 2. Alt+\ joins adjacent same-source pieces; Ctrl+Z undoes cleanly.
    s.key("alt+\\")
    s.wait_for(lambda: len(s.track("V1")["clips"]) == 3, "Alt+\\ joins V1 pieces back together")
    expect_eq(len(s.track("A1")["clips"]), 3, "Alt+\\ joins A1 pieces back together")
    s.key("ctrl+z")
    s.wait_for(lambda: len(s.track("V1")["clips"]) == 4, "Ctrl+Z undoes Alt+\\ join")
    expect_eq(len(s.track("A1")["clips"]), 4, "Ctrl+Z restores A1 split piece")
    s.key("alt+\\")
    s.wait_for(lambda: len(s.track("V1")["clips"]) == 3, "Alt+\\ rejoins before remaining checks")

    # 3. M once adds an unnamed marker quietly; M again at the same frame opens Markers dialog.
    mark_pos = seek_by_ruler(s, 3.0)
    s.key("m")
    markers = s.wait_for(lambda: s.timeline()["markers"], "M adds a marker")
    expect_eq(len(markers), 1, "marker count after first M")
    expect_eq(markers[0]["name"], "", "first M leaves marker unnamed")
    expect_near(markers[0]["position_us"], mark_pos, 34_000, "marker at playhead")
    expect(not s.widgets(text="Markers", type="GtkLabel"), "first M does not open Markers dialog")

    s.key("m")
    s.widget(text="Markers", type="GtkLabel")
    s.fill(s.row_widget("Name", "GtkEntry"), "Beat")
    s.wait_for(lambda: not s.widgets(text="Markers", type="GtkLabel"), "Markers dialog closes after naming")
    expect_eq(s.timeline()["markers"][0]["name"], "Beat", "second M edited existing marker name")

    # 4. V selects nearest edit point; Ctrl+T applies transitions immediately.
    seek_by_ruler(s, 4.1)
    s.key("ctrl+shift+a")
    s.key("v")
    edit_sel = s.wait_for(lambda: (s.state()["edit_selection"] or {}).get("kind") == "edit" and s.state()["edit_selection"], "V selects nearest edit point")
    expect_near(edit_sel["position_us"], 4_000_000, 60_000, "V picked the 4s cut")

    s.key("ctrl+t")
    s.wait_for(lambda: len(s.transitions("V1")) == 1, "Ctrl+T adds dissolve on V1 at V-selected edit")
    expect_eq(len(s.transitions("A1")), 1, "Ctrl+T adds crossfade on linked A1")
    s.key("ctrl+z")
    s.wait_for(lambda: len(s.transitions("V1")) == 0 and len(s.transitions("A1")) == 0, "Ctrl+Z removes transitions")

    # 5. Y / Alt+Y / Ctrl+Y select forward track / forward all / backward track around playhead (4.1s).
    s.key("ctrl+shift+a")
    v1_ids = [c["id"] for c in s.track("V1")["clips"]]
    a1_ids = [c["id"] for c in s.track("A1")["clips"]]
    s.key("y")
    sel_y = s.wait_for(lambda: set(s.state()["selection"]) == {v1_ids[1], v1_ids[2]} and s.state()["selection"], "Y selects V1 clips forward")
    expect_eq(set(sel_y), {v1_ids[1], v1_ids[2]}, "Y forward single-track selection")

    s.key("alt+y")
    sel_alt_y = s.wait_for(lambda: set(s.state()["selection"]) == {v1_ids[1], v1_ids[2], a1_ids[1], a1_ids[2]} and s.state()["selection"], "Alt+Y selects all tracks forward")
    expect_eq(set(sel_alt_y), {v1_ids[1], v1_ids[2], a1_ids[1], a1_ids[2]}, "Alt+Y forward all-tracks selection")

    s.key("ctrl+y")
    sel_ctrl_y = s.wait_for(lambda: set(s.state()["selection"]) == {v1_ids[0], v1_ids[1]} and s.state()["selection"], "Ctrl+Y selects V1 clips backward")
    expect_eq(set(sel_ctrl_y), {v1_ids[0], v1_ids[1]}, "Ctrl+Y backward single-track selection")
    s.key("ctrl+shift+a")

    # 6. Ctrl+/ toggles Viewer Loop button.
    loop_btn = lambda: s.widget(type="GtkToggleButton", tooltip="Loop playback (Ctrl+/)")
    expect(not loop_btn()["active"], "loop initially off")
    s.key("ctrl+/")
    s.wait_for(lambda: loop_btn()["active"], "Ctrl+/ turns loop on")
    s.key("ctrl+/")
    s.wait_for(lambda: not loop_btn()["active"], "Ctrl+/ turns loop off again")

    # 7. Alt+Shift+Z sets viewer zoom to 100%, Z restores Fit.
    zoom_dd = lambda: s.widget(type="GtkDropDown", tooltip="Viewer zoom")
    expect_eq(zoom_dd()["text"], "Fit", "viewer zoom starts at Fit")
    s.key("alt+shift+z")
    s.wait_for(lambda: zoom_dd()["text"] == "100%", "Alt+Shift+Z sets viewer zoom to 100%")
    s.key("z")
    s.wait_for(lambda: zoom_dd()["text"] == "Fit", "Z restores viewer zoom to Fit")

    # 8. Ctrl+Shift+1 toggles V1 enable/disable state.
    expect(s.track("V1")["enabled"], "V1 starts enabled")
    s.key("ctrl+shift+1")
    s.wait_for(lambda: not s.track("V1")["enabled"], "Ctrl+Shift+1 disables V1")
    s.key("ctrl+shift+1")
    s.wait_for(lambda: s.track("V1")["enabled"], "Ctrl+Shift+1 re-enables V1")

    # 9. Shift+F toggles full-page viewer (hides/restores lower timeline pane).
    surface_before = s.widget(type="TempoVideoSurface")
    expect(s.geometry()["timeline"] is not None, "timeline visible before Shift+F")
    s.key("shift+f")
    s.wait_for(lambda: s.geometry()["timeline"] is None, "Shift+F hides timeline pane")
    surface_full = s.widget(type="TempoVideoSurface")
    expect(surface_full["rect"][3] > surface_before["rect"][3], "viewer expands vertically in full-page mode")
    s.key("shift+f")
    s.wait_for(lambda: s.geometry()["timeline"] is not None, "Shift+F restores timeline pane")
    s.screenshot("1_done")
