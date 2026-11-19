"""Inspector: select a transition, type End Ratio, pick Ease In, change Duration, Set as Default Duration, then Ctrl+T uses that length."""

from tempo_e2e import center, expect_eq, expect_near

FPS = 30.0


def frames_us(frames):
    return frames * 1_000_000 / FPS


def select_edit(s, index):
    """Click the index-th cut on V1 low in the row: selects the edit point."""
    x, _ = s.cut_point("V1", index)
    s.click(x, s.clip("V1", index)["edit_y"])
    s.wait_for(lambda: (s.state()["edit_selection"] or {}).get("kind") == "edit", f"cut {index} to be selected")


def run(s):
    s.open_panel("Inspector")
    s.fit_timeline()
    select_edit(s, 0)
    s.key("ctrl+t")
    s.wait_for(lambda: s.transitions("V1"), "Ctrl+T to add a transition")
    s.click(*center(s.transition_geo("V1", 0)["rect"]))
    s.wait_for(lambda: s.state()["selected_transition"], "the transition to be selected")
    s.screenshot("1_inspector")

    # End Ratio: type 80 into its field (committed after the field is still for a moment).
    s.fill(s.row_widget("End Ratio", "GtkSpinButton"), "80")
    s.wait_for(lambda: abs(s.transitions("V1")[0]["end_ratio"] - 0.8) < 0.01, "End Ratio 80 to be saved")

    # Ease: open the dropdown and click "In" in its popup.
    s.click(*center(s.row_widget("Ease", "GtkDropDown")["rect"]))
    choice = s.widget(text="In", type="GtkLabel")
    s.click(*center(choice["rect"]))
    s.wait_for(lambda: s.transitions("V1")[0]["ease"] == "In", "Ease In to be saved")
    s.screenshot("2_ratio_ease")

    # Duration in frames (the second field on the Duration row), then Set as Default Duration.
    s.fill(s.row_widget("Duration", "GtkSpinButton", index=1), "20")
    s.wait_for(lambda: abs(s.transitions("V1")[0]["duration_us"] - frames_us(20)) < 1000, "a 20-frame duration")
    button = s.scroll_into_view(lambda: s.widget(text="Set as Default Duration", type="GtkButton"))
    s.click(*center(button["rect"]))
    s.wait_for(lambda: abs(s.state()["settings"]["transition_us"] - frames_us(20)) < 1000, "the default duration to become 20 frames")

    # Ctrl+T on the other cut now makes a 20-frame transition.
    select_edit(s, 1)
    s.key("ctrl+t")
    added = s.wait_for(lambda: len(s.transitions("V1")) == 2 and s.transitions("V1"), "Ctrl+T on the second cut")
    for t in added:
        expect_near(t["duration_us"], frames_us(20), 1000, "transition length after Set as Default Duration")
    expect_eq(sorted(t["ease"] for t in added), ["In", "None"], "only the edited transition eases in")
    s.screenshot("3_default_used")
