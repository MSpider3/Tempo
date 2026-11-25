"""Media Pool bins, DnD into bins, breadcrumbs, Grid/List & Sort, red used-in-timeline bar, and save/load persistence."""

from tempo_e2e import center, expect, expect_eq


def grid_label(s, text):
    """Find a visible label inside the Media Pool grid/list cells, scrolling the grid if needed."""
    def find():
        return next((w for w in s.widgets(text=text, type="GtkLabel") if "tempo-small" in w["css"]), None)

    hit = find()
    if hit is not None:
        return hit
    views = s.widgets(css="media")
    if views:
        vx, vy, vw, vh = views[0]["rect"]
        for step in (4, 4, 4, -8, -8):
            s.scroll(vx + vw / 2, vy + vh / 2, step)
            s.settle(quiet=0.1)
            hit = find()
            if hit is not None:
                return hit
    return next(w for w in s.widgets(text=text, type="GtkLabel") if "tempo-small" in w["css"])


def create_bin(s, name, shortcut=False):
    """Create a bin in the current bin via context menu or Ctrl+Shift+N."""
    if shortcut:
        s.key("ctrl+shift+n")
    else:
        pool = s.widget(css="tempo-surface")
        px, py, pw, ph = pool["rect"]
        s.right_click(px + pw - 20, py + ph - 20)
        s.click(*center(s.widget(text="New Bin…")["rect"]))
    s.widget(text="New Bin", type="GtkLabel")
    entry = s.widget(type="GtkEntry")
    s.fill(entry, name, enter=True)
    s.wait_for(
        lambda: any(b["name"] == name for b in s.state()["project"]["bins"]),
        f"bin {name!r} to be created",
    )


def run(s):
    s.open_panel("Media Pool")
    s.fit_timeline()

    # 1. Red used-in-timeline indicator bar on all 3 placed clips, then reacts to timeline delete/undo.
    bars = s.wait_for(lambda: len(s.widgets(css="media-used-bar")) == 3 and s.widgets(css="media-used-bar"), "3 used bars")
    shot = s.screenshot("0_initial_used_bars")
    r, g, b = shot.mean(bars[0]["rect"], step=1)
    expect(r > g + 30 and r > b + 30, f"used bar is red: {(r, g, b)}")

    s.click(*center(s.clip("V1", 2)["rect"]))
    s.key("BackSpace")
    s.wait_for(lambda: len(s.track("V1")["clips"]) == 2, "blue-6s.mp4 to be deleted")
    s.wait_for(lambda: len(s.widgets(css="media-used-bar")) == 2, "used bar count to drop to 2")
    s.key("ctrl+z")
    s.wait_for(lambda: len(s.widgets(css="media-used-bar")) == 3, "used bar count to restore to 3")

    # 2. Create bins ("Footage" via context menu, "Timelines" via Ctrl+Shift+N); bin sidebar auto-reveals.
    create_bin(s, "Footage")
    expect(bool(s.widgets(css="bin-sidebar")), "bin sidebar auto-revealed when first bin was created")
    create_bin(s, "Timelines", shortcut=True)

    bins = {b["name"]: b["id"] for b in s.state()["project"]["bins"]}
    footage_id = bins["Footage"]
    timelines_id = bins["Timelines"]

    # 3. Drag media clip onto "Footage" folder cell in grid, and drag Timeline 1 onto "Timelines" row in sidebar.
    lime_card = grid_label(s, "lime-6s.mp4")
    footage_cell = grid_label(s, "Footage")
    s.drag(*center(lime_card["rect"]), *center(footage_cell["rect"]), hold=0.3)
    s.wait_for(
        lambda: any(
            src["path"].endswith("lime-6s.mp4") and src.get("bin_id") == footage_id
            for src in s.state()["project"]["sources"].values()
        ),
        "lime-6s.mp4 to move into Footage bin",
    )

    tl_card = grid_label(s, "Timeline 1")
    sidebar_rows = s.widgets(css="bin-row")
    timelines_row = next(r for r in sidebar_rows if any(l.get("text") == "Timelines" for l in s.descendants(r, "GtkLabel")))
    s.drag(*center(tl_card["rect"]), *center(timelines_row["rect"]), hold=0.3)
    s.wait_for(
        lambda: s.state()["project"]["timeline"].get("bin_id") == timelines_id,
        "Timeline 1 to move into Timelines bin",
    )
    s.screenshot("1_moved_to_bins")

    # 3b. Open "Timelines" bin via sidebar, press Ctrl+N to verify new timelines inherit current bin.
    s.click(*center(timelines_row["rect"]))
    s.wait_for(lambda: not s.widgets(text="red-6s.mp4", type="GtkLabel"), "Timelines bin opened")
    s.key("ctrl+n")
    s.widget(text="Create New Timeline", type="GtkLabel")
    s.click(*center(s.widget(text="Create", type="GtkButton")["rect"]))
    s.wait_for(
        lambda: s.state()["project"]["timeline"]["name"] == "Timeline 2"
        and s.state()["project"]["timeline"].get("bin_id") == timelines_id,
        "new Ctrl+N timeline placed in active Timelines bin",
    )
    # Reopen Master via breadcrumb (and switch back to Timeline 1).
    bc = s.widget(css="bin-breadcrumb")
    master_btn = next(
        btn
        for btn in s.descendants(bc, "GtkButton")
        if any(l.get("text") == "Master" for l in s.descendants(btn, "GtkLabel"))
    )
    s.click(*center(master_btn["rect"]))
    s.wait_for(lambda: bool(s.widgets(text="red-6s.mp4", type="GtkLabel")), "returned to Master")

    # 4. Double-click "Footage" folder in grid, create nested "Selects" bin, test breadcrumb navigation.
    s.double_click(*center(grid_label(s, "Footage")["rect"]))
    s.wait_for(lambda: not s.widgets(text="red-6s.mp4", type="GtkLabel"), "Footage bin contents to replace Master")
    expect(bool(s.widgets(text="lime-6s.mp4", type="GtkLabel")), "lime-6s.mp4 is inside Footage")

    create_bin(s, "Selects")
    selects_id = next(b["id"] for b in s.state()["project"]["bins"] if b["name"] == "Selects")
    expect_eq(
        next(b["parent_id"] for b in s.state()["project"]["bins"] if b["id"] == selects_id),
        footage_id,
        "Selects parent_id",
    )
    s.double_click(*center(grid_label(s, "Selects")["rect"]))
    bc = s.widget(css="bin-breadcrumb")
    bc_labels = [l["text"] for l in s.descendants(bc, "GtkLabel") if l.get("text") != "/"]
    expect_eq(bc_labels, ["Master", "Footage", "Selects"], "breadcrumb path")
    s.screenshot("2_nested_breadcrumb")

    # Click "Master" in breadcrumb to navigate back to root.
    master_btn = next(
        btn
        for btn in s.descendants(bc, "GtkButton")
        if any(l.get("text") == "Master" for l in s.descendants(btn, "GtkLabel"))
    )
    s.click(*center(master_btn["rect"]))
    s.wait_for(lambda: bool(s.widgets(text="red-6s.mp4", type="GtkLabel")), "breadcrumb click to return to Master")

    # 5. Grid/List toggle and Sort dropdown.
    list_toggle = s.widget(type="GtkToggleButton", tooltip="Switch between Grid and List view")
    s.click(*center(list_toggle["rect"]))
    s.wait_for(lambda: bool(s.widgets(type="GtkColumnView", css="media")), "List view (GtkColumnView) to appear")
    expect(bool(s.widgets(text="red-6s.mp4", type="GtkLabel")), "red-6s.mp4 visible in List view")
    s.screenshot("3_list_view")

    s.click(*center(list_toggle["rect"]))
    s.wait_for(lambda: bool(s.widgets(type="GtkGridView", css="media")), "Grid view (GtkGridView) to return")

    sort_dd = s.widget(type="GtkDropDown", tooltip="Sort Media Pool")
    s.click(*center(sort_dd["rect"]))
    s.click(*center(s.widget(text="Name", type="GtkLabel")["rect"]))
    s.wait_for(lambda: sort_dd["text"] == "Name" or s.widget(type="GtkDropDown", tooltip="Sort Media Pool")["text"] == "Name", "Sort Name selected")
    # Alphabetically, blue-6s.mp4 comes before red-6s.mp4.
    blue_r = grid_label(s, "blue-6s.mp4")["rect"]
    red_r = grid_label(s, "red-6s.mp4")["rect"]
    expect((blue_r[1], blue_r[0]) < (red_r[1], red_r[0]), f"sorted by Name: blue {blue_r} before red {red_r}")

    # 6. Save & reload via Project Manager -> verify SQLite v7 persistence of bins, sources, and timelines.
    s.key("ctrl+s")
    s.wait_for(lambda: not s.state()["dirty"], "project to save cleanly")
    s.key("shift+1")
    s.wait_for(lambda: not s.state()["project_loaded"], "Project Manager to open")
    s.double_click(*center(s.widget(text="E2E Project", type="GtkLabel")["rect"]))
    s.wait_for(lambda: s.state()["project_loaded"] and s.geometry()["timeline"], "E2E Project to reload from SQLite")

    reloaded = s.state()["project"]
    reloaded_bins = {b["name"]: b for b in reloaded["bins"]}
    expect_eq(sorted(reloaded_bins), ["Footage", "Selects", "Timelines"], "reloaded bin names")
    expect_eq(reloaded_bins["Selects"]["parent_id"], reloaded_bins["Footage"]["id"], "reloaded nested bin parent")
    expect_eq(reloaded["timeline"]["bin_id"], reloaded_bins["Timelines"]["id"], "reloaded timeline bin_id")
    expect(
        any(
            src["path"].endswith("lime-6s.mp4") and src.get("bin_id") == reloaded_bins["Footage"]["id"]
            for src in reloaded["sources"].values()
        ),
        "reloaded source bin_id",
    )
    expect(bool(s.widgets(css="bin-sidebar")), "bin sidebar stays visible when project has saved bins")
    s.screenshot("4_reloaded")
