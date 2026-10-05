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
            .factory(&card_factory())
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

        // Right-click: remove a project from the list (the file is left alone).
        let menu_click = gtk::GestureClick::builder().button(gtk::gdk::BUTTON_SECONDARY).build();
        let p = pm.clone();
        menu_click.connect_pressed(move |gesture, _, x, y| {
            let Some(entry) = p.selected() else { return };
            let Some(widget) = gesture.widget() else { return };
            let remove = gtk::Button::builder().label("Remove from List").css_classes(["link-text"]).build();
            let popover = gtk::Popover::builder().child(&remove).has_arrow(false).build();
            popover.set_parent(&widget);
            popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
            let p = p.clone();
            let pop = popover.clone();
            remove.connect_clicked(move |_| {
                pop.popdown();
                let path = entry.path.clone();
                let p = p.clone();
                glib::MainContext::default().spawn_local(async move {
                    let _ = gio::spawn_blocking(move || forget(&path)).await;
                    p.reload();
                });
            });
            popover.connect_closed(|p| p.unparent());
            popover.popup();
        });
        grid.add_controller(menu_click);

        pm.reload();
        pm
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

fn card_factory() -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, obj| {
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
