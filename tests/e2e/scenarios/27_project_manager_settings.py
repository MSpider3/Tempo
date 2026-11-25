"""Project Manager & Settings: Shift+9 Project Settings, Shift+1 Project Manager, folders, list/grid, sort, and duplicate-name check."""

from tempo_e2e import center, expect, expect_eq


def run(s):
    state = s.state()
    expect(state["project_loaded"], "fixture project starts loaded")
    orig_name = state["project"]["name"]
    expect_eq(state["project"]["width"], 1920, "initial width")
    expect_eq(state["project"]["height"], 1080, "initial height")

    # Settings gear button sits beside Home in the bottom-right page bar.
    gear = s.widget(tooltip="Project Settings (Shift+9)")
    home = s.widget(tooltip="Project Manager (Shift+1)")
    expect(gear["rect"][0] > home["rect"][0], "Settings gear button is to the right of Home")

    # Shift+9 opens Project Settings; Frame rate is locked because the fixture has clips.
    s.key("shift+9")
    s.widget(text="Project Settings", type="GtkLabel")
    fps_dd = s.row_widget("Frame rate", "GtkDropDown")
    expect(not fps_dd["sensitive"], "Frame rate is insensitive when timeline has clips")
    s.widget(text="Cannot be changed once clips are on the timeline", type="GtkLabel")

    res_dd = s.row_widget("Resolution", "GtkDropDown")
    s.click(*center(res_dd["rect"]))
    s.click(*center(s.widget(text="4K", type="GtkLabel")["rect"]))
    s.wait_for(lambda: s.row_widget("Resolution", "GtkDropDown")["text"] == "4K", "4K resolution selected")
    expect_eq(s.row_widget("Width", "GtkSpinButton")["value"], 3840.0, "4K width spinbutton")
    expect_eq(s.row_widget("Height", "GtkSpinButton")["value"], 2160.0, "4K height spinbutton")

    custom_proxy = str(s.tmp / "custom-proxies")
    s.fill(s.row_widget("Proxy folder", "GtkEntry"), custom_proxy, enter=False)
    s.screenshot("1_project_settings")
    s.click(*center(s.widget(text="Apply")["rect"]))

    s.wait_for(lambda: s.state()["project"]["width"] == 3840, "Project Settings applied")
    updated = s.state()
    expect_eq(updated["project"]["height"], 2160, "updated height")
    expect_eq(updated["project"]["proxy_dir"], custom_proxy, "updated proxy_dir")
    expect(updated["dirty"], "applying Project Settings marks project dirty")

    # Shift+1 saves and opens the Project Manager.
    s.key("shift+1")
    s.wait_for(lambda: not s.state()["project_loaded"], "Project Manager opens")
    s.widget(text="Projects", type="GtkLabel")

    # Export stays insensitive until a project is selected.
    export_btn = s.widget(text="Export", type="GtkButton")
    expect(not export_btn["sensitive"], "Export disabled when nothing selected")
    card_label = s.widget(text=orig_name, type="GtkLabel")
    s.click(*center(card_label["rect"]))
    s.wait_for(lambda: s.widget(text="Export", type="GtkButton")["sensitive"], "Export enabled after selecting project")
    expect(s.widget(text="Open", type="GtkButton")["sensitive"], "Open enabled after selecting project")

    # Thumbnail size slider updates grid card dimensions.
    thumb_before = s.widget(css="project-thumb")["rect"][2]
    scale = s.widget(type="GtkScale", tooltip="Thumbnail size")
    s.scroll(*center(scale["rect"]), -3)
    s.wait_for(lambda: s.widget(css="project-thumb")["rect"][2] != thumb_before, "Thumbnail size slider resizes cards")

    # New Folder + Move to Folder + folder navigation + Delete Folder.
    s.click(*center(s.widget(tooltip="New Folder")["rect"]))
    s.widget(text="New Folder", type="GtkLabel")
    s.fill(s.widget(type="GtkEntry"), "Client Work", enter=False)
    s.click(*center(s.widget(text="Create")["rect"]))
    folder_label = s.widget(text="Client Work", type="GtkLabel")

    s.right_click(*center(s.widget(text=orig_name, type="GtkLabel")["rect"]))
    s.click(*center(s.widget(text="Move to Folder…")["rect"]))
    s.widget(text="Move to Folder", type="GtkLabel")
    target_row = s.wait_for(
        lambda: next(
            (
                w
                for w in s.widgets(text="Client Work", type="GtkLabel")
                if s.ancestor(w, "AdwActionRow", s.geometry())
            ),
            None,
        ),
        "Client Work row inside Move to Folder dialog",
    )
    s.click(*center(target_row["rect"]))
    s.wait_for(lambda: s.widgets(text="1 project", type="GtkLabel"), "folder shows 1 project")

    s.double_click(*center(s.widget(text="Client Work", type="GtkLabel")["rect"]))
    s.widget(text="Projects / Client Work", type="GtkLabel")
    s.widget(text=orig_name, type="GtkLabel")
    s.click(*center(s.widget(tooltip="Back to Projects")["rect"]))
    s.widget(text="Projects", type="GtkLabel")

    # List view / Sort / Grid view toggle.
    s.click(*center(s.widget(tooltip="List View")["rect"]))
    s.widget(type="GtkColumnView")
    s.widget(text="Name", type="GtkLabel")
    s.widget(text="Modified", type="GtkLabel")
    s.widget(text="Path", type="GtkLabel")

    sort_dd = s.widget(type="GtkDropDown", tooltip="Sort projects")
    s.click(*center(sort_dd["rect"]))
    s.click(*center(s.widget(text="Name A–Z", type="GtkLabel")["rect"]))
    s.wait_for(lambda: s.widget(type="GtkDropDown", tooltip="Sort projects")["text"] == "Name A–Z", "Sort changed to Name A–Z")
    s.screenshot("2_project_manager_list")

    s.click(*center(s.widget(tooltip="Grid View")["rect"]))
    s.widget(type="GtkGridView")

    # Delete Folder returns entries to root.
    s.right_click(*center(s.widget(text="Client Work", type="GtkLabel")["rect"]))
    s.click(*center(s.widget(text="Delete Folder")["rect"]))
    s.wait_for(lambda: not s.widgets(text="Client Work", type="GtkLabel") and s.widgets(text=orig_name, type="GtkLabel"), "deleting folder moves project back to root")

    # New Project: default name, custom location, case-insensitive duplicate validation.
    s.click(*center(s.widget(text="New Project")["rect"]))
    name_entry = s.row_widget("Name", "GtkEntry")
    expect_eq(name_entry["text"], "Untitled Project", "default New Project name")
    loc_entry = s.row_widget("Location", "GtkEntry")
    expect(loc_entry["text"].endswith("/Videos/Tempo"), f"default location ends with /Videos/Tempo, got {loc_entry['text']!r}")
    expect(bool(s.row_widgets("Location", "GtkButton")), "Browse button on Location row")

    dup_name = orig_name.upper() if orig_name != orig_name.upper() else orig_name.lower()
    s.fill(name_entry, dup_name, enter=False)
    s.click(*center(s.widget(text="Create")["rect"]))
    s.widget(text="Name Already Exists", type="GtkLabel")
    s.widget(text="A project with the same name is already in use.", type="GtkLabel")
    s.screenshot("3_duplicate_name")
    s.click(*center(s.widget(text="OK")["rect"]))

    # New Project dialog remains open so the user can pick a fresh name and location.
    s.wait_for(lambda: not s.widgets(text="Name Already Exists", type="GtkLabel"), "duplicate alert dismissed")
    name_entry = s.row_widget("Name", "GtkEntry")
    s.fill(name_entry, "Fresh Project", enter=False)
    custom_dir = str(s.tmp / "custom-projects")
    s.fill(s.row_widget("Location", "GtkEntry"), custom_dir, enter=False)
    s.click(*center(s.widget(text="Create")["rect"]))

    s.wait_for(lambda: s.state()["project_loaded"] and s.state()["project"]["name"] == "Fresh Project", "Fresh Project created and opened")
    expect_eq(s.state()["path"], f"{custom_dir}/Fresh Project.tempo", "custom save location path")
    s.screenshot("4_created_custom_location")
