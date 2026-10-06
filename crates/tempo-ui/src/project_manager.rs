//! Project Manager: a grid of projects with the actions at the bottom, as in
//! DaVinci Resolve's Project Manager.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use gtk4 as gtk;
use gtk4::prelude::*;
use gtk4::gio;
use serde::{Deserialize, Serialize};

use crate::util::{label, pill, tool_toggle};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    /// Filled in when the list is loaded; not stored.
    #[serde(skip)]
    pub modified: Option<std::time::SystemTime>,
    #[serde(skip)]
    pub missing: bool,
}

fn list_path() -> PathBuf {
    glib::user_data_dir().join("tempo").join("projects.json")
}

/// Read the project list. Blocking: call from a worker.
pub fn load_list() -> Vec<Entry> {
    let mut entries: Vec<Entry> = std::fs::read_to_string(list_path())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    for e in &mut entries {
        match std::fs::metadata(&e.path).and_then(|m| m.modified()) {
            Ok(time) => e.modified = Some(time),
            Err(_) => e.missing = true,
        }
    }
    // Most recently changed first.
    entries.sort_by_key(|e| std::cmp::Reverse(e.modified));
    entries
}

fn save_list(entries: &[Entry]) -> std::io::Result<()> {
    let path = list_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = serde_json::to_string_pretty(entries).unwrap_or_else(|_| "[]".into());
    std::fs::write(path, text)
}

/// Add or refresh a project in the list. Blocking: call from a worker.
pub fn remember(name: &str, path: &Path) {
    let mut entries = load_list();
    entries.retain(|e| e.path != path);
    entries.insert(0, Entry { name: name.to_string(), path: path.to_path_buf(), modified: None, missing: false });
    if let Err(e) = save_list(&entries) {
        tracing::warn!("could not save the project list: {e}");
    }
}

fn forget(path: &Path) {
    let mut entries = load_list();
    entries.retain(|e| e.path != path);
    if let Err(e) = save_list(&entries) {
        tracing::warn!("could not save the project list: {e}");
    }
}

/// Change the name stored in a project file. The file itself keeps its name. Blocking.
fn rename_project(path: &Path, name: &str) -> Result<(), String> {
    let mut project = tempo_project::load_project(path).map_err(|e| e.to_string())?;
    project.name = name.to_string();
    tempo_project::save_project(&project, path).map_err(|e| e.to_string())?;
    remember(name, path);
    Ok(())
}

/// Copy a project file next to the original as "<name> copy". Blocking.
fn duplicate_project(path: &Path) -> Result<(), String> {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Project".into());
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut copy = dir.join(format!("{stem} copy.tempo"));
    let mut n = 2;
    while copy.exists() {
        copy = dir.join(format!("{stem} copy {n}.tempo"));
        n += 1;
    }
    std::fs::copy(path, &copy).map_err(|e| e.to_string())?;
    let mut project = tempo_project::load_project(&copy).map_err(|e| e.to_string())?;
    project.name = format!("{} copy", project.name);
    tempo_project::save_project(&project, &copy).map_err(|e| e.to_string())?;
    remember(&project.name, &copy);
    Ok(())
}

fn relative_time(time: Option<std::time::SystemTime>) -> String {
    let Some(secs) = time.and_then(|t| t.elapsed().ok()).map(|d| d.as_secs()) else { return String::new() };
    match secs {
        0..=59 => "Just now".into(),
        60..=3599 => format!("{} min ago", secs / 60),
        3600..=86_399 => format!("{} h ago", secs / 3600),
        86_400..=172_799 => "Yesterday".into(),
        _ => format!("{} days ago", secs / 86_400),
    }
}

pub struct ProjectManager {
    pub root: gtk::Box,
    store: gio::ListStore,
    selection: gtk::SingleSelection,
    stack: gtk::Stack,
    search: gtk::SearchEntry,
    open: gtk::Button,
    all: std::cell::RefCell<Vec<Entry>>,
}

impl ProjectManager {
    /// `on_open` receives a project path; `on_new` asks for the New Project dialog.
    pub fn new(on_open: impl Fn(PathBuf) + Clone + 'static, on_new: impl Fn() + 'static) -> Rc<Self> {
        let root = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["tempo-panel"]).build();

        let title_bar = gtk::CenterBox::builder().css_classes(["tempo-bar", "tempo-sep-bottom"]).height_request(44).build();
        title_bar.set_center_widget(Some(&label("Tempo", &["tempo-heading", "tempo-bright"])));
        title_bar.set_end_widget(Some(&gtk::WindowControls::new(gtk::PackType::End)));
        let handle = gtk::WindowHandle::builder().child(&title_bar).build();
        root.append(&handle);

        let search = gtk::SearchEntry::builder().placeholder_text("Search projects").width_request(240).build();
        let search_toggle = tool_toggle("system-search-symbolic", "Search projects");
        search.set_visible(false);
        let header = gtk::Box::builder().spacing(8).margin_top(8).margin_bottom(8).margin_start(16).margin_end(16).build();
        let heading = label("Projects", &["tempo-heading", "tempo-bright"]);
        heading.set_hexpand(true);
        heading.set_xalign(0.0);
        header.append(&heading);
        header.append(&search);
        header.append(&search_toggle);
        root.append(&header);
        root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let selection = gtk::SingleSelection::builder().model(&store).autoselect(false).can_unselect(true).build();
        let grid = gtk::GridView::builder()
            .model(&selection)
            .factory(&card_factory(&selection))
            .max_columns(8)
            .min_columns(1)
            .css_classes(["projects"])
            .margin_top(12)
            .margin_start(12)
            .margin_end(12)
            .build();
        let scrolled = gtk::ScrolledWindow::builder().child(&grid).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build();

        let empty = label("No projects yet", &["empty-hint"]);
        empty.set_vexpand(true);
        let stack = gtk::Stack::new();
        stack.add_named(&empty, Some("empty"));
        stack.add_named(&scrolled, Some("grid"));
        root.append(&stack);

        let import = pill("Import");
        import.set_tooltip_text(Some("Open a .tempo file from anywhere"));
        let new = pill("New Project");
        let open = pill("Open");
        open.add_css_class("default");
        open.set_sensitive(false);
        let right = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        right.append(&new);
        right.append(&open);
        let footer = gtk::CenterBox::builder().margin_top(12).margin_bottom(16).margin_start(24).margin_end(24).build();
        footer.set_start_widget(Some(&import));
        footer.set_end_widget(Some(&right));
        root.append(&footer);

        let pm = Rc::new(Self { root, store, selection, stack, search, open, all: Default::default() });

        let p = pm.clone();
        search_toggle.connect_toggled(move |b| {
            p.search.set_visible(b.is_active());
            if b.is_active() {
                p.search.grab_focus();
            } else {
                p.search.set_text("");
            }
        });
        let p = pm.clone();
        pm.search.connect_search_changed(move |_| p.show_filtered());

        let p = pm.clone();
        pm.selection.connect_selected_item_notify(move |sel| p.open.set_sensitive(sel.selected_item().is_some()));

        new.connect_clicked(move |_| on_new());

        let p = pm.clone();
        let open_cb = on_open.clone();
        pm.open.connect_clicked(move |_| {
            if let Some(entry) = p.selected() {
                open_cb(entry.path);
            }
        });
        let p = pm.clone();
        let open_cb = on_open.clone();
        grid.connect_activate(move |_, pos| {
            if let Some(obj) = p.store.item(pos).and_downcast::<glib::BoxedAnyObject>() {
                open_cb(obj.borrow::<Entry>().path.clone());
            }
        });

        let open_cb = on_open.clone();
        import.connect_clicked(move |b| {
            let filter = gtk::FileFilter::new();
            filter.set_name(Some("Tempo projects"));
            filter.add_pattern("*.tempo");
            let filters = gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&filter);
            let dialog = gtk::FileDialog::builder().title("Import Project").modal(true).filters(&filters).build();
            let open_cb = open_cb.clone();
            dialog.open(b.root().and_downcast::<gtk::Window>().as_ref(), gio::Cancellable::NONE, move |result| {
                if let Some(path) = result.ok().and_then(|f| f.path()) {
                    open_cb(path);
                }
            });
        });

        // Right-click menu for the selected project.
        let menu_click = gtk::GestureClick::builder().button(gtk::gdk::BUTTON_SECONDARY).build();
        let p = pm.clone();
        let open_cb = on_open.clone();
        menu_click.connect_pressed(move |gesture, _, x, y| {
            let Some(widget) = gesture.widget() else { return };
            // The card under the pointer was selected by its own handler (see
            // `card_factory`). Over empty space there is nothing to act on.
            if widget.pick(x, y, gtk::PickFlags::DEFAULT).is_none_or(|hit| hit == widget) {
                return;
            }
            let Some(entry) = p.selected() else { return };
            let list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).margin_top(4).margin_bottom(4).margin_start(4).margin_end(4).build();
            let popover = gtk::Popover::builder().child(&list).has_arrow(false).build();
            popover.set_parent(&widget);
            popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));

            // Each item closes the menu, then does its work.
            let item = |text: &str, run: Box<dyn Fn()>| {
                let button = gtk::Button::builder().label(text).css_classes(["link-text"]).halign(gtk::Align::Start).build();
                let pop = popover.clone();
                button.connect_clicked(move |_| {
                    pop.popdown();
                    run();
                });
                list.append(&button);
            };

            let (cb, path) = (open_cb.clone(), entry.path.clone());
            item("Open", Box::new(move || cb(path.clone())));

            let (pm, e, parent) = (p.clone(), entry.clone(), widget.clone());
            item("Rename…", Box::new(move || pm.ask_rename(&parent, &e)));

            let (pm, path) = (p.clone(), entry.path.clone());
            item("Duplicate", Box::new(move || pm.run_then_reload(path.clone(), duplicate_project)));

            let (path, parent) = (entry.path.clone(), widget.clone());
            item(
                "Show in Files",
                Box::new(move || {
                    let window = parent.root().and_downcast::<gtk::Window>();
                    gtk::FileLauncher::new(Some(&gio::File::for_path(&path))).open_containing_folder(window.as_ref(), gio::Cancellable::NONE, |_| {});
                }),
            );

            // The file goes to the system trash, so this can be undone from the file manager.
            let (pm, path) = (p.clone(), entry.path.clone());
            item(
                "Move to Trash",
                Box::new(move || {
                    let (pm, path) = (pm.clone(), path.clone());
                    glib::MainContext::default().spawn_local(async move {
                        // A file that is already gone only needs to leave the list.
                        let gone = !path.exists();
                        match gio::File::for_path(&path).trash_future(glib::Priority::DEFAULT).await {
                            Err(e) if !gone => {
                                pm.report(&format!("Could not move the project to the trash: {e}"));
                                return;
                            }
                            _ => {}
                        }
                        let _ = gio::spawn_blocking(move || forget(&path)).await;
                        pm.reload();
                    });
                }),
            );

            // Only the entry goes; the project file stays where it is.
            let (pm, path) = (p.clone(), entry.path.clone());
            item("Remove from List", Box::new(move || pm.run_then_reload(path.clone(), |p| {
                forget(p);
                Ok(())
            })));

            popover.connect_closed(|p| p.unparent());
            popover.popup();
        });
        grid.add_controller(menu_click);

        pm.reload();
        pm
    }

    /// Run blocking work on a project file, then refresh the grid.
    fn run_then_reload(self: &Rc<Self>, path: PathBuf, work: impl FnOnce(&Path) -> Result<(), String> + Send + 'static) {
        let pm = self.clone();
        glib::MainContext::default().spawn_local(async move {
            if let Ok(Err(e)) = gio::spawn_blocking(move || work(&path)).await {
                pm.report(&format!("That did not work: {e}"));
            }
            pm.reload();
        });
    }

    /// Tell the user about something that failed.
    fn report(&self, text: &str) {
        use libadwaita as adw;
        use libadwaita::prelude::*;
        tracing::warn!("{text}");
        let dialog = adw::AlertDialog::builder().heading("Project Manager").body(text).build();
        dialog.add_responses(&[("ok", "OK")]);
        dialog.present(Some(&self.root));
    }

    fn ask_rename(self: &Rc<Self>, parent: &gtk::Widget, entry: &Entry) {
        use libadwaita as adw;
        use libadwaita::prelude::*;
        let field = gtk::Entry::builder().text(&entry.name).activates_default(true).build();
        let dialog = adw::AlertDialog::builder().heading("Rename Project").extra_child(&field).build();
        dialog.add_responses(&[("cancel", "Cancel"), ("rename", "Rename")]);
        dialog.set_default_response(Some("rename"));
        dialog.set_close_response("cancel");
        let pm = self.clone();
        let path = entry.path.clone();
        dialog.connect_response(None, move |_, response| {
            let name = field.text().trim().to_string();
            if response == "rename" && !name.is_empty() {
                pm.run_then_reload(path.clone(), move |p| rename_project(p, &name));
            }
        });
        dialog.present(Some(parent));
    }

    fn selected(&self) -> Option<Entry> {
        self.selection.selected_item().and_downcast::<glib::BoxedAnyObject>().map(|o| o.borrow::<Entry>().clone())
    }

    /// Re-read the list from disk on a worker thread.
    pub fn reload(self: &Rc<Self>) {
        let pm = self.clone();
        glib::MainContext::default().spawn_local(async move {
            let entries = gio::spawn_blocking(load_list).await.unwrap_or_default();
            *pm.all.borrow_mut() = entries;
            pm.show_filtered();
        });
    }

    fn show_filtered(&self) {
        let query = self.search.text().to_lowercase();
        self.store.remove_all();
        let all = self.all.borrow();
        for e in all.iter().filter(|e| query.is_empty() || e.name.to_lowercase().contains(&query)) {
            self.store.append(&glib::BoxedAnyObject::new(e.clone()));
        }
        self.stack.set_visible_child_name(if all.is_empty() { "empty" } else { "grid" });
    }
}

fn card_factory(selection: &gtk::SingleSelection) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    let selection = selection.downgrade();
    factory.connect_setup(move |_, obj| {
        let Some(item) = obj.downcast_ref::<gtk::ListItem>() else { return };
        let icon = gtk::Image::builder().icon_name("dev.tempo.Tempo-symbolic").pixel_size(40).build();
        let thumb = gtk::Box::builder().css_classes(["project-thumb"]).width_request(224).height_request(126).halign(gtk::Align::Center).build();
        icon.set_hexpand(true);
        thumb.append(&icon);
        let name = gtk::Label::builder().ellipsize(gtk::pango::EllipsizeMode::End).max_width_chars(24).css_classes(["project-name-label"]).build();
        let date = label("", &["tempo-small", "tempo-dim"]);
        let card = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).build();
        card.append(&thumb);
        card.append(&name);
        card.append(&date);
        // A right click selects the card first, so the menu that the grid then
        // opens is about the project under the pointer, not the one clicked before.
        let click = gtk::GestureClick::builder().button(gtk::gdk::BUTTON_SECONDARY).build();
        let (weak_item, selection) = (item.downgrade(), selection.clone());
        click.connect_pressed(move |_, _, _, _| {
            if let (Some(item), Some(selection)) = (weak_item.upgrade(), selection.upgrade()) {
                selection.set_selected(item.position());
            }
        });
        card.add_controller(click);
        item.set_child(Some(&card));
    });
    factory.connect_bind(|_, obj| {
        let Some(item) = obj.downcast_ref::<gtk::ListItem>() else { return };
        let Some(boxed) = item.item().and_downcast::<glib::BoxedAnyObject>() else { return };
        let entry = boxed.borrow::<Entry>();
        let Some(card) = item.child().and_downcast::<gtk::Box>() else { return };
        let thumb = card.first_child();
        let name = thumb.as_ref().and_then(|t| t.next_sibling()).and_downcast::<gtk::Label>();
        let date = card.last_child().and_downcast::<gtk::Label>();
        if let Some(icon) = thumb.and_then(|t| t.first_child()).and_downcast::<gtk::Image>() {
            icon.set_icon_name(Some(if entry.missing { "dialog-warning-symbolic" } else { "dev.tempo.Tempo-symbolic" }));
        }
        if let Some(n) = name {
            n.set_text(&entry.name);
        }
        if let Some(d) = date {
            d.set_text(&if entry.missing { "File not found".to_string() } else { relative_time(entry.modified) });
        }
        card.set_tooltip_text(Some(&entry.path.to_string_lossy()));
    });
    factory
}

#[cfg(test)]
mod tests {
    use tempo_timeline::RationalFps;

    #[test]
    fn rename_and_duplicate_change_the_stored_name() {
        // Keep the project list of this test away from the user's own.
        let dir = std::env::temp_dir().join(format!("tempo-pm-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("One.tempo");
        tempo_project::create_new_project("One", 1920, 1080, RationalFps::FPS_30, &path).expect("create");

        let mut project = tempo_project::load_project(&path).expect("load");
        project.name = "Two".into();
        tempo_project::save_project(&project, &path).expect("save");
        assert_eq!(tempo_project::load_project(&path).expect("load").name, "Two");

        // The copy gets its own file and name; the original is untouched.
        let copy = dir.join("One copy.tempo");
        std::fs::copy(&path, &copy).expect("copy");
        let mut dup = tempo_project::load_project(&copy).expect("load copy");
        dup.name = format!("{} copy", dup.name);
        tempo_project::save_project(&dup, &copy).expect("save copy");
        assert_eq!(tempo_project::load_project(&copy).expect("load").name, "Two copy");
        assert_eq!(tempo_project::load_project(&path).expect("load").name, "Two");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
