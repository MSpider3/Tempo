"""Beginner helpers: searchable 'How Do I…?' guide, live auto-ticking First-Edit Checklist, and per-panel '?' popovers."""

from _helpers import seek_by_ruler
from tempo_e2e import center, expect


def run(s):
    s.open_panel("Media Pool")
    s.open_panel("Inspector")
    s.fit_timeline()

    # 1. Open "How Do I…?" from top-bar ? button, verify guides and live search.
    help_btn = s.widget(tooltip="How Do I…? Beginner Guide")
    s.click(*center(help_btn["rect"]))
    s.wait_for(lambda: s.widgets(text="How Do I…?", type="GtkLabel"), "How Do I dialog opens")
    for title in (
        "Import & Organize in Bins",
        "Cut Out Mistakes",
        "Close Gaps",
        "Add Transitions to Every Cut",
        "Add Titles & Markers",
        "Export for YouTube",
    ):
        s.wait_for(lambda t=title: s.widgets(text=t, type="GtkLabel", hidden=True), f"guide {title!r}")

    search = next(w for w in s.widgets(type="GtkSearchEntry") if w["rect"][2] > 260)
    s.fill(search, "youtube", enter=False)
    s.wait_for(
        lambda: s.widgets(text="Export for YouTube", type="GtkLabel")
        and not s.widgets(text="Cut Out Mistakes", type="GtkLabel"),
        "search filters to Export for YouTube",
    )
    s.screenshot("1_how_do_i_filtered")
    s.fill(search, "bin", enter=False)
    s.wait_for(
        lambda: s.widgets(text="Import & Organize in Bins", type="GtkLabel")
        and not s.widgets(text="Export for YouTube", type="GtkLabel"),
        "search filters to Import & Organize in Bins",
    )
    s.key("Escape")
    s.wait_for(lambda: not s.widgets(text="How Do I…?", type="GtkLabel"), "How Do I dialog closes")

    # 2. First-Edit Checklist auto-ticks across real edits (2/5 -> 3/5 -> 4/5 -> 5/5).
    s.wait_for(lambda: s.widgets(text="First Edit 2/5", type="GtkLabel"), "initial 2/5 progress on 3-clip fixture")
    checklist_btn = s.widget(tooltip="First-Edit Checklist — tracks your first 5 editing steps")
    s.click(*center(checklist_btn["rect"]))
    s.wait_for(lambda: s.widgets(text="First-Edit Checklist", type="GtkLabel"), "checklist popover opens")
    s.screenshot("2_checklist_open")
    s.key("Escape")
    s.wait_for(lambda: not s.widgets(text="First-Edit Checklist", type="GtkLabel"), "checklist popover closes")

    # Split a clip -> 3/5.
    s.click(*center(s.clip("V1", 0)["rect"]))
    seek_by_ruler(s, 2.0)
    s.key("ctrl+backslash")
    s.wait_for(lambda: s.widgets(text="First Edit 3/5", type="GtkLabel"), "split ticks checklist to 3/5")

    # Add a transition -> 4/5.
    seek_by_ruler(s, 2.0)
    s.key("v")
    s.wait_for(lambda: (s.state()["edit_selection"] or {}).get("kind") == "edit", "nearest edit selected")
    s.key("ctrl+t")
    s.wait_for(lambda: s.widgets(text="First Edit 4/5", type="GtkLabel"), "transition ticks checklist to 4/5")

    # Switch to Export page and queue an export -> 5/5.
    s.key("shift+8")
    s.wait_for(lambda: s.widgets(contains="Render Settings"), "Export page opens")
    s.click(*center(s.widget(text="Add to Render Queue")["rect"]))
    s.wait_for(lambda: s.widgets(text="First Edit 5/5 ✓", type="GtkLabel"), "queued export completes checklist 5/5")
    s.screenshot("3_checklist_complete")

    # Dismiss checklist and restore it from "How Do I…?".
    s.click(*center(s.widget(tooltip="First-Edit Checklist — tracks your first 5 editing steps")["rect"]))
    s.wait_for(lambda: s.widgets(text="Dismiss checklist"), "Dismiss button visible")
    s.click(*center(s.widget(text="Dismiss checklist")["rect"]))
    s.wait_for(lambda: not s.widgets(contains="First Edit", type="GtkLabel"), "checklist pill hidden after dismiss")

    s.click(*center(s.widget(tooltip="How Do I…? Beginner Guide")["rect"]))
    s.wait_for(lambda: s.widgets(text="Show First-Edit Checklist"), "Show First-Edit Checklist button")
    s.settle()
    s.click(*center(s.widget(text="Show First-Edit Checklist")["rect"]))
    s.wait_for(lambda: s.widgets(text="First Edit 5/5 ✓", type="GtkLabel"), "checklist restored")

    # 3. Per-panel ? mini-guide popovers on Export, Media Pool, Timeline, and Inspector.
    for tooltip, expected_resolve in [
        ("Render Settings & Queue guide", "DaVinci Resolve: Deliver Page"),
    ]:
        s.click(*center(s.widget(tooltip=tooltip)["rect"]))
        s.wait_for(lambda t=expected_resolve: s.widgets(text=t, type="GtkLabel"), f"{tooltip} popover opens")
        s.key("Escape")
        s.wait_for(lambda t=expected_resolve: not s.widgets(text=t, type="GtkLabel"), f"{tooltip} popover closes")

    s.key("shift+4")
    s.wait_for(lambda: s.widgets(tooltip="Timeline guide"), "back on Edit page")
    for tooltip, expected_resolve in [
        ("Media Pool guide", "DaVinci Resolve: Media Pool & Bins"),
        ("Timeline guide", "DaVinci Resolve: Edit Page Timeline"),
        ("Inspector guide", "DaVinci Resolve: Inspector"),
    ]:
        s.click(*center(s.widget(tooltip=tooltip)["rect"]))
        s.wait_for(lambda t=expected_resolve: s.widgets(text=t, type="GtkLabel"), f"{tooltip} popover opens")
        if tooltip == "Timeline guide":
            s.screenshot("4_timeline_mini_guide")
        s.key("Escape")
        s.wait_for(lambda t=expected_resolve: not s.widgets(text=t, type="GtkLabel"), f"{tooltip} popover closes")

    expect(True, "all beginner helpers verified")
