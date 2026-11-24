//! Media Pool: bins/folders, sorting, grid/list views, thumbnails, and drag-and-drop.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use gtk4 as gtk;
use gtk4::prelude::*;
use gtk4::{gdk, gio};
use libadwaita as adw;
use libadwaita::prelude::*;
use tempo_media::MediaEngine;
use tempo_timeline::{MediaType, MASTER_BIN_ID};
use uuid::Uuid;

use crate::actions::{self, Place};
use crate::state::{AppState, Change};
use crate::util::{label, short_duration, tool_button, tool_toggle};

const DEFAULT_THUMB_W: i32 = 96;
const MIN_THUMB_W: i32 = 64;
const MAX_THUMB_W: i32 = 160;
const HOVER_SCRUB_DEBOUNCE_MS: u64 = 80;

const TIMELINE_DRAG_PREFIX: &str = "timeline:";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum SortMode {
    #[default]
    ImportOrder,
    Name,
    Duration,
}

impl SortMode {
    fn from_index(index: u32) -> Self {
        match index {
            1 => Self::Name,
            2 => Self::Duration,
            _ => Self::ImportOrder,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ItemKind {
    Bin,
    Media,
    Timeline,
}

#[derive(Clone, Copy)]
struct CellMeta {
    id: Uuid,
    kind: ItemKind,
    duration_us: i64,
    hover_scrubbable: bool,
}

impl CellMeta {
    fn drag_payload(self) -> Option<String> {
        match self.kind {
            // Media clips keep plain `<uuid>` so timeline DnD stays unchanged.
            ItemKind::Media => Some(self.id.to_string()),
            ItemKind::Timeline => Some(format!("{TIMELINE_DRAG_PREFIX}{}", self.id)),
            ItemKind::Bin => None,
        }
    }
}

/// One row of the pool list model.
struct Item {
    id: Uuid,
    kind: ItemKind,
    name: String,
    detail: String,
    duration_us: i64,
    import_order: u32,
    sort_order: i32,
    media_type: MediaType,
    missing: bool,
    /// The timeline that is currently open.
    open: bool,
    /// Whether this media source is referenced by the open timeline.
    used: bool,
    thumbnail: Option<gdk::Texture>,
}

impl Item {
    fn type_label(&self) -> &'static str {
        match self.kind {
            ItemKind::Bin => "Bin",
            ItemKind::Timeline => "Timeline",
            ItemKind::Media => match self.media_type {
                MediaType::Video => "Video",
                MediaType::Audio => "Audio",
                MediaType::Image => "Image",
            },
        }
    }
}

pub struct MediaPool {
    pub root: gtk::Box,
    state: Rc<AppState>,
    store: gio::ListStore,
    selection: gtk::SingleSelection,
    /// The right-click menu while it is open.
    menu: RefCell<Option<gtk::PopoverMenu>>,
    stack: gtk::Stack,
    empty_hint: gtk::Label,
    /// The drop hint under the timeline items while the project has no media yet.
    media_hint: gtk::Label,
    search: gtk::SearchEntry,
    sort: gtk::DropDown,
    sidebar_toggle: gtk::ToggleButton,
    sidebar_revealer: gtk::Revealer,
    sidebar_list: gtk::ListBox,
    sidebar_bins: RefCell<Vec<Option<Uuid>>>,
    breadcrumb_box: gtk::Box,
    list_toggle: gtk::ToggleButton,
    current_bin: Cell<Option<Uuid>>,
    sidebar_open: Cell<bool>,
    syncing_sidebar: Cell<bool>,
    sort_mode: Cell<SortMode>,
    list_mode: Cell<bool>,
    thumb_width: Rc<Cell<i32>>,
    thumb_cells: Rc<RefCell<Vec<(glib::WeakRef<gtk::Picture>, glib::WeakRef<gtk::Label>)>>>,
    thumb_cache: RefCell<HashMap<Uuid, (usize, gdk::Texture)>>,
    hover_gen: Cell<u64>,
    hover_restore: Cell<Option<(Option<Uuid>, (Option<i64>, Option<i64>))>>,
    window: glib::WeakRef<gtk::Window>,
    on_open: Rc<dyn Fn(Uuid)>,
}

impl MediaPool {
    pub fn new(state: &Rc<AppState>, on_open: impl Fn(Uuid) + 'static) -> Rc<Self> {
        let on_open: Rc<dyn Fn(Uuid)> = Rc::new(on_open);
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .width_request(300)
            .hexpand(false)
            .css_classes(["tempo-surface", "tempo-sep-right"])
            .build();

        // ---- Header: top row (sidebar toggle, breadcrumbs, actions) + filter row ----
        let sidebar_toggle = tool_toggle("folder-symbolic", "Show or hide Bins sidebar");
        let breadcrumb_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(2)
            .css_classes(["bin-breadcrumb"])
            .build();
        let breadcrumb_scroll = gtk::ScrolledWindow::builder()
            .child(&breadcrumb_box)
            .hexpand(true)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .build();

        let import = tool_button("list-add-symbolic", "Import media (Ctrl+I)");

        let thumb_width = Rc::new(Cell::new(DEFAULT_THUMB_W));
        let zoom_scale = gtk::Scale::builder()
            .orientation(gtk::Orientation::Horizontal)
            .adjustment(&gtk::Adjustment::new(
                DEFAULT_THUMB_W as f64,
                MIN_THUMB_W as f64,
                MAX_THUMB_W as f64,
                8.0,
                16.0,
                0.0,
            ))
            .draw_value(false)
            .width_request(140)
            .tooltip_text("Thumbnail size")
            .build();
        let zoom_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(4)
            .margin_top(6)
            .margin_bottom(6)
            .margin_start(8)
            .margin_end(8)
            .build();
        zoom_box.append(&label("Thumbnail Size", &["tempo-small", "tempo-dim"]));
        zoom_box.append(&zoom_scale);
        let zoom_popover = gtk::Popover::builder().child(&zoom_box).build();
        let zoom_btn = gtk::MenuButton::builder()
            .icon_name("zoom-in-symbolic")
            .tooltip_text("Thumbnail size")
            .popover(&zoom_popover)
            .css_classes(["tool"])
            .build();
        zoom_btn.update_property(&[gtk::accessible::Property::Label("Thumbnail size")]);

        let list_toggle = tool_toggle("view-list-symbolic", "Switch between Grid and List view");

        let header_top = gtk::Box::builder()
            .spacing(4)
            .margin_top(6)
            .margin_bottom(2)
            .margin_start(6)
            .margin_end(6)
            .build();
        header_top.append(&sidebar_toggle);
        header_top.append(&breadcrumb_scroll);
        header_top.append(&import);
        header_top.append(&zoom_btn);
        header_top.append(&list_toggle);

        let search = gtk::SearchEntry::builder().placeholder_text("Search").hexpand(true).build();
        let sort = gtk::DropDown::from_strings(&["Import Order", "Name", "Duration"]);
        sort.set_tooltip_text(Some("Sort Media Pool"));
        let header_filter = gtk::Box::builder()
            .spacing(6)
            .margin_top(2)
            .margin_bottom(6)
            .margin_start(8)
            .margin_end(8)
            .build();
        header_filter.append(&search);
        header_filter.append(&sort);

        root.append(&header_top);
        root.append(&header_filter);

        // ---- Body: left bin tree sidebar + right content stack ----
        let sidebar_new_bin = tool_button("list-add-symbolic", "New Bin (Ctrl+Shift+N)");
        let sidebar_head = gtk::Box::builder()
            .spacing(4)
            .margin_top(4)
            .margin_bottom(2)
            .margin_start(8)
            .margin_end(4)
            .build();
        let bins_heading = label("Bins", &["tempo-small", "tempo-dim"]);
        bins_heading.set_xalign(0.0);
        bins_heading.set_hexpand(true);
        sidebar_head.append(&bins_heading);
        sidebar_head.append(&sidebar_new_bin);

        let sidebar_list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .build();
        let sidebar_scroll = gtk::ScrolledWindow::builder()
            .child(&sidebar_list)
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        let sidebar_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .width_request(130)
            .css_classes(["bin-sidebar"])
            .build();
        sidebar_box.append(&sidebar_head);
        sidebar_box.append(&sidebar_scroll);

        let sidebar_revealer = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::SlideRight)
            .reveal_child(false)
            .child(&sidebar_box)
            .build();

        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let selection = gtk::SingleSelection::builder()
            .model(&store)
            .autoselect(false)
            .can_unselect(true)
            .build();

        let grid = gtk::GridView::builder()
            .model(&selection)
            .max_columns(6)
            .min_columns(1)
            .css_classes(["media"])
            .build();
        let grid_scroll = gtk::ScrolledWindow::builder()
            .child(&grid)
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();

        let column_view = gtk::ColumnView::builder()
            .model(&selection)
            .css_classes(["media"])
            .build();
        let list_scroll = gtk::ScrolledWindow::builder()
            .child(&column_view)
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();

        let empty_hint = label("Drop video, audio or images here,\nor press Ctrl+I", &["drop-hint"]);
        empty_hint.set_justify(gtk::Justification::Center);
        empty_hint.set_valign(gtk::Align::Start);

        let stack = gtk::Stack::new();
        stack.add_named(&empty_hint, Some("empty"));
        stack.add_named(&grid_scroll, Some("grid"));
        stack.add_named(&list_scroll, Some("list"));
        stack.set_vexpand(true);

        let media_hint = label("Drop video, audio or images here,\nor press Ctrl+I", &["drop-hint"]);
        media_hint.set_justify(gtk::Justification::Center);
        media_hint.set_vexpand(true);
        media_hint.set_valign(gtk::Align::Start);

        let content_col = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .hexpand(true)
            .vexpand(true)
            .build();
        content_col.append(&stack);
        content_col.append(&media_hint);

        let body = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .vexpand(true)
            .build();
        body.append(&sidebar_revealer);
        body.append(&content_col);
        root.append(&body);

        let pool = Rc::new(Self {
            root,
            state: state.clone(),
            store,
            selection: selection.clone(),
            menu: RefCell::new(None),
            stack,
            empty_hint,
            media_hint,
            search,
            sort,
            sidebar_toggle,
            sidebar_revealer,
            sidebar_list,
            sidebar_bins: RefCell::new(Vec::new()),
            breadcrumb_box,
            list_toggle,
            current_bin: Cell::new(None),
            sidebar_open: Cell::new(false),
            syncing_sidebar: Cell::new(false),
            sort_mode: Cell::new(SortMode::ImportOrder),
            list_mode: Cell::new(false),
            thumb_width,
            thumb_cells: Rc::new(RefCell::new(Vec::new())),
            thumb_cache: RefCell::new(HashMap::new()),
            hover_gen: Cell::new(0),
            hover_restore: Cell::new(None),
            window: glib::WeakRef::new(),
            on_open,
        });

        grid.set_factory(Some(&grid_factory(&pool)));
        let name_col = gtk::ColumnViewColumn::builder()
            .title("Name")
            .expand(true)
            .resizable(true)
            .factory(&list_name_factory(&pool))
            .build();
        let dur_col = gtk::ColumnViewColumn::builder()
            .title("Duration")
            .fixed_width(64)
            .factory(&list_text_factory(&pool, ListCol::Duration))
            .build();
        let type_col = gtk::ColumnViewColumn::builder()
            .title("Type")
            .fixed_width(62)
            .factory(&list_text_factory(&pool, ListCol::Type))
            .build();
        column_view.append_column(&name_col);
        column_view.append_column(&dur_col);
        column_view.append_column(&type_col);

        pool.install_menu_actions();

        // Right-clicking empty space offers New Bin / Import / New Timeline.
        let click = gtk::GestureClick::builder().button(gdk::BUTTON_SECONDARY).build();
        let p = pool.clone();
        click.connect_pressed(move |_, _, x, y| p.show_menu(None, x, y));
        pool.root.add_controller(click);

        let p = pool.clone();
        import.connect_clicked(move |_| p.choose_files());

        let p = pool.clone();
        sidebar_new_bin.connect_clicked(move |_| p.new_bin_dialog());

        let p = pool.clone();
        pool.sidebar_toggle.connect_toggled(move |b| {
            let active = b.is_active();
            p.sidebar_open.set(active);
            p.sidebar_revealer.set_reveal_child(active);
        });

        let p = pool.clone();
        pool.sidebar_list.connect_row_selected(move |_, row| {
            if p.syncing_sidebar.get() {
                return;
            }
            if let Some(row) = row {
                let idx = row.index();
                if idx >= 0 {
                    if let Some(&bin_id) = p.sidebar_bins.borrow().get(idx as usize) {
                        p.navigate_to(bin_id);
                    }
                }
            }
        });

        let p = pool.clone();
        pool.list_toggle.connect_toggled(move |b| {
            p.list_mode.set(b.is_active());
            p.update_visible_stack();
        });

        let p = pool.clone();
        pool.sort.connect_selected_notify(move |dd| {
            p.sort_mode.set(SortMode::from_index(dd.selected()));
            p.reload();
        });

        let p = pool.clone();
        zoom_scale.connect_value_changed(move |s| {
            let w = (s.value().round() as i32).clamp(MIN_THUMB_W, MAX_THUMB_W);
            p.set_thumb_width(w);
        });

        let p = pool.clone();
        pool.search.connect_search_changed(move |_| p.reload());

        let s = state.clone();
        selection.connect_selected_item_notify(move |sel| {
            let media_id = sel
                .selected_item()
                .and_downcast::<glib::BoxedAnyObject>()
                .and_then(|o| {
                    let item = o.borrow::<Item>();
                    (item.kind == ItemKind::Media).then_some(item.id)
                });
            s.media_selection.set(media_id);
        });

        // Double-click (or Enter) opens bin, opens timeline, or opens clip in viewer.
        let p = pool.clone();
        grid.connect_activate(move |_, pos| p.activate_index(pos));
        let p = pool.clone();
        column_view.connect_activate(move |_, pos| p.activate_index(pos));

        // Files or folders dropped from the file manager.
        let drop = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
        let p = pool.clone();
        drop.connect_drop(move |_, value, _, _| {
            let Ok(list) = value.get::<gdk::FileList>() else { return false };
            p.import(list.files().into_iter().filter_map(|f| f.path()).collect());
            true
        });
        pool.root.add_controller(drop);

        let p = pool.clone();
        state.connect(move |change| {
            match change {
                Change::Project => {
                    p.end_hover_scrub();
                    p.hover_restore.set(None);
                    p.thumb_cache.borrow_mut().clear();
                    p.current_bin.set(None);
                    let has_bins = p.state.with_project(|proj| !proj.bins.is_empty()).unwrap_or(false);
                    p.set_sidebar_open(has_bins);
                    p.reload();
                }
                Change::Media | Change::Timeline | Change::Timelines => {
                    p.reload();
                }
                _ => {}
            }
        });
        pool.reload();
        pool
    }

    pub fn current_bin(&self) -> Option<Uuid> {
        self.current_bin.get()
    }

    fn set_sidebar_open(&self, open: bool) {
        self.sidebar_open.set(open);
        if self.sidebar_toggle.is_active() != open {
            self.sidebar_toggle.set_active(open);
        } else {
            self.sidebar_revealer.set_reveal_child(open);
        }
    }

    fn set_thumb_width(&self, w: i32) {
        let w = w.clamp(MIN_THUMB_W, MAX_THUMB_W);
        if self.thumb_width.replace(w) == w {
            return;
        }
        let h = w * 9 / 16;
        let chars = (w / 8).max(8);
        self.thumb_cells.borrow_mut().retain(|(pic_ref, lbl_ref)| {
            let Some(pic) = pic_ref.upgrade() else { return false };
            pic.set_size_request(w, h);
            if let Some(lbl) = lbl_ref.upgrade() {
                lbl.set_max_width_chars(chars);
            }
            true
        });
    }

    fn navigate_to(self: &Rc<Self>, bin_id: Option<Uuid>) {
        let valid = match bin_id.filter(|id| *id != MASTER_BIN_ID) {
            Some(id) => self.state.with_project(|p| p.bins.iter().any(|b| b.id == id)).unwrap_or(false).then_some(id),
            None => None,
        };
        if self.current_bin.replace(valid) != valid {
            self.end_hover_scrub();
            self.reload();
        }
    }

    fn activate_index(self: &Rc<Self>, pos: u32) {
        let Some(obj) = self.store.item(pos).and_downcast::<glib::BoxedAnyObject>() else { return };
        let (id, kind) = {
            let item = obj.borrow::<Item>();
            (item.id, item.kind)
        };
        match kind {
            ItemKind::Bin => self.navigate_to(Some(id)),
            ItemKind::Timeline => self.state.open_timeline(id),
            ItemKind::Media => {
                // Promote any active hover preview so leaving the cell keeps the clip open.
                self.hover_gen.set(self.hover_gen.get().wrapping_add(1));
                self.hover_restore.set(None);
                (self.on_open)(id);
            }
        }
    }

    /// Actions behind the right-click menu.
    fn install_menu_actions(self: &Rc<Self>) {
        let group = gio::SimpleActionGroup::new();
        let add = |name: &str, run: Box<dyn Fn(&Rc<Self>, Uuid)>| {
            let action = gio::SimpleAction::new(name, Some(glib::VariantTy::STRING));
            let pool = self.clone();
            action.connect_activate(move |_, target| {
                if let Some(id) = target.and_then(|t| t.str()).and_then(|t| Uuid::parse_str(t).ok()) {
                    run(&pool, id);
                }
            });
            group.add_action(&action);
        };
        add("insert", Box::new(|p, id| actions::place_source(&p.state, id, Place::Insert, None, None, true)));
        add("overwrite", Box::new(|p, id| actions::place_source(&p.state, id, Place::Overwrite, None, None, true)));
        add("append", Box::new(|p, id| actions::place_source(&p.state, id, Place::Append, None, None, true)));
        add("open", Box::new(|p, id| {
            p.hover_gen.set(p.hover_gen.get().wrapping_add(1));
            p.hover_restore.set(None);
            p.state.show_source(Some(id));
        }));
        add("remove", Box::new(|p, id| actions::remove_source(&p.state, id)));
        add("new-timeline-with-clip", Box::new(|p, id| p.new_timeline_with_clip_dialog(id)));
        add("open-timeline", Box::new(|p, id| p.state.open_timeline(id)));
        add("rename-timeline", Box::new(|p, id| p.rename_dialog(id)));
        add("duplicate-timeline", Box::new(|p, id| {
            p.state.duplicate_timeline(id);
        }));
        add("delete-timeline", Box::new(|p, id| p.delete_dialog(id)));
        add("open-bin", Box::new(|p, id| p.navigate_to(Some(id))));
        add("rename-bin", Box::new(|p, id| p.rename_bin_dialog(id)));
        add("delete-bin", Box::new(|p, id| p.delete_bin(id)));
        add(
            "reveal",
            Box::new(|p, id| {
                if let Some(path) = p.state.with_project(|proj| proj.sources.get(&id).map(|s| s.path.clone())).flatten() {
                    gtk::FileLauncher::new(Some(&gio::File::for_path(path))).open_containing_folder(p.window.upgrade().as_ref(), gio::Cancellable::NONE, |_| {});
                }
            }),
        );

        let add_pair = |name: &str, run: Box<dyn Fn(&Rc<Self>, Uuid, Option<Uuid>)>| {
            let action = gio::SimpleAction::new(name, Some(glib::VariantTy::STRING));
            let pool = self.clone();
            action.connect_activate(move |_, target| {
                if let Some((item_s, bin_s)) = target.and_then(|t| t.str()).and_then(|t| t.split_once(':')) {
                    if let (Ok(item_id), Ok(bin_id)) = (Uuid::parse_str(item_s), Uuid::parse_str(bin_s)) {
                        let bin = (bin_id != MASTER_BIN_ID).then_some(bin_id);
                        run(&pool, item_id, bin);
                    }
                }
            });
            group.add_action(&action);
        };
        add_pair("move-source-to-bin", Box::new(|p, id, bin| { p.move_source_to_bin(id, bin); }));
        add_pair("move-timeline-to-bin", Box::new(|p, id, bin| { p.move_timeline_to_bin(id, bin); }));

        let new_bin = gio::SimpleAction::new("new-bin", None);
        let pool = self.clone();
        new_bin.connect_activate(move |_, _| pool.new_bin_dialog());
        group.add_action(&new_bin);

        let import = gio::SimpleAction::new("import", None);
        let pool = self.clone();
        import.connect_activate(move |_, _| pool.choose_files());
        group.add_action(&import);

        let import_folder = gio::SimpleAction::new("import-folder", None);
        let pool = self.clone();
        import_folder.connect_activate(move |_, _| pool.choose_folder());
        group.add_action(&import_folder);

        self.root.insert_action_group("pool", Some(&group));
    }

    fn bin_menu_paths(&self) -> Vec<(Uuid, String)> {
        self.state
            .with_project(|p| {
                let mut out = Vec::new();
                let mut seen = HashSet::new();
                collect_bin_tree(&p.bins, None, 0, &mut seen, &mut out);
                out.into_iter()
                    .map(|(id, name, depth)| {
                        let prefix = "  ".repeat(depth);
                        (id, format!("{prefix}{name}"))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn build_move_submenu(&self, item_id: Uuid, current_bin: Option<Uuid>, action: &str) -> Option<gio::Menu> {
        let sub = gio::Menu::new();
        if current_bin.is_some() {
            let item = gio::MenuItem::new(Some("Master"), None);
            item.set_action_and_target_value(Some(action), Some(&format!("{item_id}:{MASTER_BIN_ID}").to_variant()));
            sub.append_item(&item);
        }
        for (bin_id, label) in self.bin_menu_paths() {
            if current_bin == Some(bin_id) {
                continue;
            }
            let item = gio::MenuItem::new(Some(&label), None);
            item.set_action_and_target_value(Some(action), Some(&format!("{item_id}:{bin_id}").to_variant()));
            sub.append_item(&item);
        }
        (sub.n_items() > 0).then_some(sub)
    }

    /// The right-click menu, for a bin/clip/timeline or (with `None`) for empty space.
    fn show_menu(&self, target: Option<(Uuid, ItemKind)>, x: f64, y: f64) {
        if self.state.project.borrow().is_none() {
            return;
        }
        let menu = gio::Menu::new();
        match target {
            Some((id, ItemKind::Bin)) => {
                let part = gio::Menu::new();
                for (label, action) in [
                    ("Open Bin", "pool.open-bin"),
                    ("Rename Bin…", "pool.rename-bin"),
                    ("Delete Bin", "pool.delete-bin"),
                ] {
                    let item = gio::MenuItem::new(Some(label), None);
                    item.set_action_and_target_value(Some(action), Some(&id.to_string().to_variant()));
                    part.append_item(&item);
                }
                menu.append_section(None, &part);
            }
            Some((id, ItemKind::Timeline)) => {
                let part = gio::Menu::new();
                let (is_open, current_bin) = self
                    .state
                    .with_project(|p| {
                        let open = p.timeline.id == id;
                        let bin = std::iter::once(&p.timeline)
                            .chain(&p.timelines)
                            .find(|t| t.id == id)
                            .and_then(|t| t.bin_id);
                        (open, bin)
                    })
                    .unwrap_or((false, None));
                for (label, action) in [
                    ("Open", "pool.open-timeline"),
                    ("Rename…", "pool.rename-timeline"),
                    ("Duplicate Timeline", "pool.duplicate-timeline"),
                    ("Delete Timeline", "pool.delete-timeline"),
                ] {
                    if is_open && action == "pool.delete-timeline" {
                        part.append_item(&gio::MenuItem::new(Some(label), Some("pool.none")));
                        continue;
                    }
                    let item = gio::MenuItem::new(Some(label), None);
                    item.set_action_and_target_value(Some(action), Some(&id.to_string().to_variant()));
                    part.append_item(&item);
                }
                menu.append_section(None, &part);
                if let Some(sub) = self.build_move_submenu(id, current_bin, "pool.move-timeline-to-bin") {
                    let move_sec = gio::Menu::new();
                    move_sec.append_submenu(Some("Move to Bin"), &sub);
                    menu.append_section(None, &move_sec);
                }
            }
            Some((id, ItemKind::Media)) => {
                let section = |items: &[(&str, &str)]| {
                    let part = gio::Menu::new();
                    for (label, action) in items {
                        let item = gio::MenuItem::new(Some(label), None);
                        item.set_action_and_target_value(Some(action), Some(&id.to_string().to_variant()));
                        part.append_item(&item);
                    }
                    part
                };
                menu.append_section(
                    None,
                    &section(&[
                        ("Insert at Playhead", "pool.insert"),
                        ("Overwrite at Playhead", "pool.overwrite"),
                        ("Add to End of Timeline", "pool.append"),
                    ]),
                );
                menu.append_section(
                    None,
                    &section(&[("Create New Timeline Using Selected Clip…", "pool.new-timeline-with-clip")]),
                );
                menu.append_section(None, &section(&[("Open in Viewer", "pool.open"), ("Show in Files", "pool.reveal")]));
                let current_bin = self
                    .state
                    .with_project(|p| p.sources.get(&id).and_then(|s| s.bin_id))
                    .flatten();
                if let Some(sub) = self.build_move_submenu(id, current_bin, "pool.move-source-to-bin") {
                    let move_sec = gio::Menu::new();
                    move_sec.append_submenu(Some("Move to Bin"), &sub);
                    menu.append_section(None, &move_sec);
                }
                menu.append_section(None, &section(&[("Remove from Media Pool", "pool.remove")]));
            }
            None => {}
        }
        let general = gio::Menu::new();
        general.append(Some("New Bin…"), Some("pool.new-bin"));
        general.append(Some("Import Media…"), Some("pool.import"));
        general.append(Some("Import Folder…"), Some("pool.import-folder"));
        let new_timeline = gio::MenuItem::new(Some("Create New Timeline…"), None);
        new_timeline.set_action_and_target_value(Some("win.do"), Some(&"win.new-timeline".to_variant()));
        general.append_item(&new_timeline);
        menu.append_section(None, &general);

        self.popup_menu(&menu, x, y);
    }

    fn popup_menu(&self, menu: &gio::Menu, x: f64, y: f64) {
        if let Some(old) = self.menu.take() {
            old.unparent();
        }
        let popover = gtk::PopoverMenu::from_model(Some(menu));
        popover.set_parent(&self.root);
        popover.set_has_arrow(false);
        popover.set_halign(gtk::Align::Start);
        popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        *self.menu.borrow_mut() = Some(popover.clone());
        popover.popup();
    }

    pub fn set_window(&self, window: &impl IsA<gtk::Window>) {
        self.window.set(Some(window.upcast_ref()));
    }

    /// Select the nth item, as a click would. For scripted checks.
    pub fn select_index(&self, index: u32) {
        self.selection.set_selected(index);
        if let Some(obj) = self.store.item(index).and_downcast::<glib::BoxedAnyObject>() {
            let item = obj.borrow::<Item>();
            self.state.media_selection.set((item.kind == ItemKind::Media).then_some(item.id));
        }
    }

    /// The id of the nth item. For scripted checks.
    pub fn id_at(&self, index: u32) -> Option<Uuid> {
        self.store.item(index).and_downcast::<glib::BoxedAnyObject>().map(|o| o.borrow::<Item>().id)
    }

    /// Open the right-click menu of the nth item. For scripted checks.
    pub fn show_menu_for_index(&self, index: u32) {
        self.selection.set_selected(index);
        let target = self
            .store
            .item(index)
            .and_downcast::<glib::BoxedAnyObject>()
            .map(|o| (o.borrow::<Item>().id, o.borrow::<Item>().kind));
        self.show_menu(target, 60.0, 90.0);
    }

    pub fn focus(&self) {
        self.search.grab_focus();
    }

    pub fn choose_files(self: &Rc<Self>) {
        let dialog = gtk::FileDialog::builder().title("Import Media").modal(true).build();
        let pool = self.clone();
        dialog.open_multiple(self.window.upgrade().as_ref(), gio::Cancellable::NONE, move |result| {
            if let Ok(files) = result {
                let paths = (0..files.n_items())
                    .filter_map(|i| files.item(i).and_downcast::<gio::File>())
                    .filter_map(|f| f.path())
                    .collect();
                pool.import(paths);
            }
        });
    }

    pub fn choose_folder(self: &Rc<Self>) {
        let dialog = gtk::FileDialog::builder().title("Import Folder").modal(true).build();
        let pool = self.clone();
        dialog.select_folder(self.window.upgrade().as_ref(), gio::Cancellable::NONE, move |result| {
            if let Ok(folder) = result {
                if let Some(path) = folder.path() {
                    pool.import_folder(path);
                }
            }
        });
    }

    fn import_folder(self: &Rc<Self>, folder: PathBuf) {
        if self.state.project.borrow().is_none() {
            return;
        }
        let target_bin = self.current_bin.get();
        let pool = self.clone();
        glib::MainContext::default().spawn_local(async move {
            let found = gio::spawn_blocking(move || scan_media_folder(&folder)).await;
            match found {
                Ok(files) if files.is_empty() => {
                    pool.state.message("No media files found in that folder.");
                }
                Ok(files) => pool.import_files_into_bin(files, target_bin),
                Err(_) => pool.state.message("Could not scan that folder."),
            }
        });
    }

    /// Probe each file (or folder recursively) on a worker thread; nothing is converted.
    pub fn import(self: &Rc<Self>, paths: Vec<PathBuf>) {
        if self.state.project.borrow().is_none() || paths.is_empty() {
            return;
        }
        let target_bin = self.current_bin.get();
        let pool = self.clone();
        glib::MainContext::default().spawn_local(async move {
            let expanded = gio::spawn_blocking(move || expand_paths(paths)).await;
            match expanded {
                Ok(files) => pool.import_files_into_bin(files, target_bin),
                Err(_) => pool.state.message("Import failed unexpectedly."),
            }
        });
    }

    fn import_files_into_bin(self: &Rc<Self>, paths: Vec<PathBuf>, target_bin: Option<Uuid>) {
        if self.state.project.borrow().is_none() {
            return;
        }
        // Resolve asks once, for media brought into an empty Media Pool.
        let may_offer = Rc::new(Cell::new(self.state.with_project(|p| p.sources.is_empty()).unwrap_or(false)));
        for path in paths {
            let may_offer = may_offer.clone();
            let already = self.state.with_project(|p| p.sources.values().any(|s| s.path == path)).unwrap_or(false);
            if already {
                continue;
            }
            let pool = self.clone();
            glib::MainContext::default().spawn_local(async move {
                let probe_path = path.clone();
                let probed = gio::spawn_blocking(move || MediaEngine::create_media_source(&probe_path)).await;
                match probed {
                    Ok(Ok(mut source)) => {
                        let state = &pool.state;
                        let offer = may_offer
                            .get()
                            .then(|| state.with_project(|p| p.first_import_offer(&source)).flatten())
                            .flatten();
                        if let Some(p) = state.project.borrow_mut().as_mut() {
                            source.import_order = p.sources.len() as u32;
                            source.bin_id = target_bin.filter(|bid| p.bins.iter().any(|b| b.id == *bid));
                            p.sources.insert(source.id, source);
                        }
                        state.sync_player();
                        state.set_dirty(true);
                        state.emit(Change::Media);
                        if let Some(offer) = offer {
                            may_offer.set(false);
                            pool.offer_format(offer);
                        }
                    }
                    Ok(Err(e)) => {
                        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        pool.state.message(format!("Could not import {name}: {e}"));
                    }
                    Err(_) => pool.state.message("Import failed unexpectedly."),
                }
            });
        }
    }

    pub fn new_bin_dialog(self: &Rc<Self>) {
        let parent_id = self.current_bin.get();
        let Some((default_name, siblings)) = self.state.with_project(|p| {
            let siblings: Vec<(Uuid, String)> = p
                .bins
                .iter()
                .filter(|b| b.parent_id == parent_id)
                .map(|b| (b.id, b.name.clone()))
                .collect();
            (p.next_bin_name(parent_id), siblings)
        }) else {
            return;
        };
        let entry = gtk::Entry::builder().text(&default_name).activates_default(true).build();
        let dialog = adw::AlertDialog::builder().heading("New Bin").extra_child(&entry).build();
        dialog.add_responses(&[("cancel", "Cancel"), ("create", "Create")]);
        dialog.set_response_appearance("create", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("create"));
        dialog.set_close_response("cancel");
        let d = dialog.downgrade();
        entry.connect_changed(move |e| {
            if let Some(d) = d.upgrade() {
                d.set_response_enabled("create", name_available(&siblings, Uuid::nil(), &e.text()));
            }
        });
        let pool = self.clone();
        let field = entry.clone();
        dialog.connect_response(Some("create"), move |_, _| {
            let created = {
                let mut proj = pool.state.project.borrow_mut();
                proj.as_mut().and_then(|p| p.create_bin(field.text().trim(), parent_id))
            };
            if created.is_some() {
                pool.set_sidebar_open(true);
                pool.state.set_dirty(true);
                pool.state.emit(Change::Media);
            } else {
                pool.state.message("That bin name is empty or already used here.");
            }
        });
        dialog.present(self.window.upgrade().as_ref());
        entry.grab_focus();
        entry.select_region(0, -1);
    }

    fn rename_bin_dialog(self: &Rc<Self>, id: Uuid) {
        let Some((current_name, siblings)) = self.state.with_project(|p| {
            let bin = p.bins.iter().find(|b| b.id == id)?;
            let siblings: Vec<(Uuid, String)> = p
                .bins
                .iter()
                .filter(|b| b.parent_id == bin.parent_id)
                .map(|b| (b.id, b.name.clone()))
                .collect();
            Some((bin.name.clone(), siblings))
        }).flatten() else {
            return;
        };
        let entry = gtk::Entry::builder().text(&current_name).activates_default(true).build();
        let dialog = adw::AlertDialog::builder().heading("Rename Bin").extra_child(&entry).build();
        dialog.add_responses(&[("cancel", "Cancel"), ("rename", "Rename")]);
        dialog.set_response_appearance("rename", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("rename"));
        dialog.set_close_response("cancel");
        let d = dialog.downgrade();
        entry.connect_changed(move |e| {
            if let Some(d) = d.upgrade() {
                d.set_response_enabled("rename", name_available(&siblings, id, &e.text()));
            }
        });
        let pool = self.clone();
        let field = entry.clone();
        dialog.connect_response(Some("rename"), move |_, _| {
            let renamed = {
                let mut proj = pool.state.project.borrow_mut();
                proj.as_mut().is_some_and(|p| p.rename_bin(id, field.text().trim()))
            };
            if renamed {
                pool.state.set_dirty(true);
                pool.state.emit(Change::Media);
            } else {
                pool.state.message("That bin name is empty or already used here.");
            }
        });
        dialog.present(self.window.upgrade().as_ref());
        entry.grab_focus();
        entry.select_region(0, -1);
    }

    fn delete_bin(&self, id: Uuid) {
        let parent_of_deleted = self.state.with_project(|p| p.bins.iter().find(|b| b.id == id).and_then(|b| b.parent_id)).flatten();
        let deleted = {
            let mut proj = self.state.project.borrow_mut();
            proj.as_mut().is_some_and(|p| p.delete_bin(id))
        };
        if deleted {
            if self.current_bin.get() == Some(id) {
                self.current_bin.set(parent_of_deleted);
            }
            self.state.set_dirty(true);
            self.state.emit(Change::Media);
            self.state.emit(Change::Timelines);
        }
    }

    fn move_source_to_bin(&self, source_id: Uuid, bin_id: Option<Uuid>) -> bool {
        let target = bin_id.filter(|id| *id != MASTER_BIN_ID);
        let moved = {
            let mut proj = self.state.project.borrow_mut();
            let Some(p) = proj.as_mut() else { return false };
            if p.sources.get(&source_id).map(|s| s.bin_id) == Some(target) {
                return true;
            }
            p.move_source_to_bin(source_id, target)
        };
        if moved {
            self.state.set_dirty(true);
            self.state.emit(Change::Media);
        }
        moved
    }

    fn move_timeline_to_bin(&self, timeline_id: Uuid, bin_id: Option<Uuid>) -> bool {
        let target = bin_id.filter(|id| *id != MASTER_BIN_ID);
        let moved = {
            let mut proj = self.state.project.borrow_mut();
            let Some(p) = proj.as_mut() else { return false };
            let cur = std::iter::once(&p.timeline)
                .chain(&p.timelines)
                .find(|t| t.id == timeline_id)
                .map(|t| t.bin_id);
            if cur == Some(target) {
                return true;
            }
            p.move_timeline_to_bin(timeline_id, target)
        };
        if moved {
            self.state.set_dirty(true);
            self.state.emit(Change::Timelines);
        }
        moved
    }

    fn handle_bin_drop(&self, text: &str, target_bin: Option<Uuid>) -> bool {
        if let Some(tl_str) = text.strip_prefix(TIMELINE_DRAG_PREFIX) {
            let Ok(tl_id) = Uuid::parse_str(tl_str) else { return false };
            return self.move_timeline_to_bin(tl_id, target_bin);
        }
        let Ok(src_id) = Uuid::parse_str(text) else { return false };
        self.move_source_to_bin(src_id, target_bin)
    }

    fn new_timeline_with_clip_dialog(self: &Rc<Self>, source_id: Uuid) {
        let Some(default_name) = self.state.with_project(|p| p.next_timeline_name()) else { return };
        let taken = self.state.with_project(|p| p.timeline_list()).unwrap_or_default();
        let entry = gtk::Entry::builder().text(&default_name).activates_default(true).build();
        let dialog = adw::AlertDialog::builder()
            .heading("Create New Timeline Using Selected Clip")
            .extra_child(&entry)
            .build();
        dialog.add_responses(&[("cancel", "Cancel"), ("create", "Create")]);
        dialog.set_response_appearance("create", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("create"));
        dialog.set_close_response("cancel");
        let d = dialog.downgrade();
        entry.connect_changed(move |e| {
            if let Some(d) = d.upgrade() {
                d.set_response_enabled("create", name_available(&taken, Uuid::nil(), &e.text()));
            }
        });
        let pool = self.clone();
        let field = entry.clone();
        dialog.connect_response(Some("create"), move |_, _| {
            let text = field.text();
            let typed = text.trim();
            let title = if typed.is_empty() { default_name.clone() } else { typed.to_string() };
            let mut tl = tempo_timeline::Timeline::new_default();
            tl.name = title;
            tl.bin_id = pool.current_bin.get();
            pool.state.add_timeline(tl);
            actions::place_source(&pool.state, source_id, Place::Overwrite, Some(0), None, true);
        });
        dialog.present(self.window.upgrade().as_ref());
        entry.grab_focus();
        entry.select_region(0, -1);
    }

    /// Resolve's "Change Project Frame Rate?" for the first video in an empty project.
    fn offer_format(&self, offer: tempo_timeline::FormatOffer) {
        let rate = |f: tempo_timeline::RationalFps| {
            format!("{:.3}", f.to_f64()).trim_end_matches('0').trim_end_matches('.').to_string()
        };
        let Some((w, h, fps)) = self.state.with_project(|p| (p.width, p.height, p.fps)) else { return };
        let dialog = adw::AlertDialog::builder()
            .heading("Change Project Frame Rate?")
            .body(format!(
                "This clip is {} × {} at {} fps. The project is {w} × {h} at {} fps.\n\nChange the project to match? It can't be changed once clips are on the timeline.",
                offer.width,
                offer.height,
                rate(offer.fps),
                rate(fps)
            ))
            .build();
        dialog.add_responses(&[("keep", "Don't Change"), ("change", "Change")]);
        dialog.set_response_appearance("change", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("change"));
        dialog.set_close_response("keep");
        let state = self.state.clone();
        dialog.connect_response(Some("change"), move |_, _| {
            // Not undoable, as in Resolve; the project is empty, so nothing is lost.
            let applied = state.project.borrow_mut().as_mut().map(|p| p.apply_format(&offer));
            match applied {
                Some(Ok(())) => {
                    state.sync_player();
                    state.set_dirty(true);
                    state.emit(Change::Project);
                }
                Some(Err(e)) => state.message(e.to_string()),
                None => {}
            }
        });
        dialog.present(self.window.upgrade().as_ref());
    }

    fn rename_dialog(&self, id: Uuid) {
        let Some(current) = self
            .state
            .with_project(|p| p.timeline_list().into_iter().find(|(i, _)| *i == id).map(|(_, n)| n))
            .flatten()
        else {
            return;
        };
        let taken = self.state.with_project(|p| p.timeline_list()).unwrap_or_default();
        let entry = gtk::Entry::builder().text(&current).activates_default(true).build();
        let dialog = adw::AlertDialog::builder().heading("Rename Timeline").extra_child(&entry).build();
        dialog.add_responses(&[("cancel", "Cancel"), ("rename", "Rename")]);
        dialog.set_response_appearance("rename", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("rename"));
        dialog.set_close_response("cancel");
        let d = dialog.downgrade();
        entry.connect_changed(move |e| {
            if let Some(d) = d.upgrade() {
                d.set_response_enabled("rename", name_available(&taken, id, &e.text()));
            }
        });
        let state = self.state.clone();
        let field = entry.clone();
        dialog.connect_response(Some("rename"), move |_, _| {
            if !state.rename_timeline(id, field.text().trim()) {
                state.message("That name is empty or already used by another timeline.");
            }
        });
        dialog.present(self.window.upgrade().as_ref());
        entry.grab_focus();
    }

    fn delete_dialog(&self, id: Uuid) {
        let Some(name) = self
            .state
            .with_project(|p| p.timeline_list().into_iter().find(|(i, _)| *i == id).map(|(_, n)| n))
            .flatten()
        else {
            return;
        };
        let dialog = adw::AlertDialog::builder()
            .heading("Delete Timeline?")
            .body(format!("\"{name}\" and its edits will be deleted. This cannot be undone."))
            .build();
        dialog.add_responses(&[("cancel", "Cancel"), ("delete", "Delete")]);
        dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
        dialog.set_close_response("cancel");
        let state = self.state.clone();
        dialog.connect_response(Some("delete"), move |_, _| {
            if !state.remove_timeline(id) {
                state.message("The open timeline cannot be deleted. Open another timeline first.");
            }
        });
        dialog.present(self.window.upgrade().as_ref());
    }

    fn schedule_hover_scrub(self: &Rc<Self>, id: Uuid, duration_us: i64, x: f64, width: i32) {
        if duration_us <= 0 || width <= 0 || self.state.player.is_playing() {
            return;
        }
        let frac = (x / width as f64).clamp(0.0, 1.0);
        let target_us = ((duration_us as f64) * frac).round() as i64;
        let generation = self.hover_gen.get().wrapping_add(1);
        self.hover_gen.set(generation);
        let pool = self.clone();
        glib::timeout_add_local_once(Duration::from_millis(HOVER_SCRUB_DEBOUNCE_MS), move || {
            if pool.hover_gen.get() != generation || pool.state.player.is_playing() {
                return;
            }
            pool.preview_hover(id, target_us);
        });
    }

    fn preview_hover(&self, id: Uuid, target_us: i64) {
        let valid = self
            .state
            .with_project(|p| {
                p.sources
                    .get(&id)
                    .is_some_and(|s| !s.is_missing && s.media_type == MediaType::Video && s.duration_us > 0)
            })
            .unwrap_or(false);
        if !valid {
            return;
        }
        if self.hover_restore.get().is_none() {
            let prev = self.state.source_clip.get();
            let marks = (self.state.src_in.get(), self.state.src_out.get());
            self.hover_restore.set(Some((prev, marks)));
        }
        if self.state.source_clip.get() != Some(id) {
            self.state.show_source(Some(id));
        }
        if !self.state.dual_viewer.get() {
            self.state.player.seek(target_us, false);
        }
    }

    fn end_hover_scrub(&self) {
        self.hover_gen.set(self.hover_gen.get().wrapping_add(1));
        if let Some((prev, (src_in, src_out))) = self.hover_restore.take() {
            if self.state.source_clip.get() != prev {
                self.state.show_source(prev);
                if prev.is_some() {
                    self.state.src_in.set(src_in);
                    self.state.src_out.set(src_out);
                    self.state.emit(Change::Options);
                }
            }
        }
    }

    fn reload_breadcrumbs(self: &Rc<Self>) {
        while let Some(child) = self.breadcrumb_box.first_child() {
            self.breadcrumb_box.remove(&child);
        }
        let ancestors = self
            .state
            .with_project(|p| p.bin_ancestors(self.current_bin.get()))
            .unwrap_or_else(|| vec![(None, "Master".to_string())]);
        let count = ancestors.len();
        for (i, (bin_id, name)) in ancestors.into_iter().enumerate() {
            if i > 0 {
                self.breadcrumb_box.append(&label("/", &["tempo-dim", "tempo-small"]));
            }
            let btn = gtk::Button::with_label(&name);
            btn.set_focus_on_click(false);
            if i + 1 == count {
                btn.add_css_class("current");
            }
            let pool = self.clone();
            btn.connect_clicked(move |_| pool.navigate_to(bin_id));

            let drop = gtk::DropTarget::new(glib::Type::STRING, gdk::DragAction::COPY);
            let pool = self.clone();
            drop.connect_drop(move |_, value, _, _| {
                value.get::<String>().is_ok_and(|text| pool.handle_bin_drop(&text, bin_id))
            });
            btn.add_controller(drop);

            self.breadcrumb_box.append(&btn);
        }
    }

    fn reload_sidebar(self: &Rc<Self>) {
        self.syncing_sidebar.set(true);
        while let Some(row) = self.sidebar_list.first_child() {
            self.sidebar_list.remove(&row);
        }
        let mut entries: Vec<(Option<Uuid>, String, usize)> = vec![(None, "Master".to_string(), 0)];
        if let Some(tree) = self.state.with_project(|p| {
            let mut out = Vec::new();
            let mut seen = HashSet::new();
            collect_bin_tree(&p.bins, None, 1, &mut seen, &mut out);
            out
        }) {
            entries.extend(tree.into_iter().map(|(id, name, depth)| (Some(id), name, depth)));
        }

        let cur = self.current_bin.get();
        let mut ids = Vec::with_capacity(entries.len());
        let mut select_row = None;
        for (bin_id, name, depth) in entries {
            let row = gtk::ListBoxRow::builder().css_classes(["bin-row"]).build();
            let row_box = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(6)
                .margin_start(8 + depth as i32 * 12)
                .margin_end(6)
                .margin_top(2)
                .margin_bottom(2)
                .build();
            row_box.append(&gtk::Image::from_icon_name("folder-symbolic"));
            let lbl = gtk::Label::builder()
                .label(&name)
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .hexpand(true)
                .build();
            row_box.append(&lbl);
            row.set_child(Some(&row_box));

            let drop = gtk::DropTarget::new(glib::Type::STRING, gdk::DragAction::COPY);
            let r = row.downgrade();
            drop.connect_enter(move |_, _, _| {
                if let Some(r) = r.upgrade() {
                    r.add_css_class("drop-hover");
                }
                gdk::DragAction::COPY
            });
            drop.connect_motion(move |_, _, _| gdk::DragAction::COPY);
            let r = row.downgrade();
            drop.connect_leave(move |_| {
                if let Some(r) = r.upgrade() {
                    r.remove_css_class("drop-hover");
                }
            });
            let r = row.downgrade();
            let pool = self.clone();
            drop.connect_drop(move |_, value, _, _| {
                if let Some(r) = r.upgrade() {
                    r.remove_css_class("drop-hover");
                }
                value.get::<String>().is_ok_and(|text| pool.handle_bin_drop(&text, bin_id))
            });
            row.add_controller(drop);

            let click = gtk::GestureClick::builder().button(gdk::BUTTON_SECONDARY).build();
            let pool = self.clone();
            let r = row.downgrade();
            click.connect_pressed(move |gesture, _, x, y| {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                if let Some(r) = r.upgrade() {
                    if let Some(point) = r.compute_point(&pool.root, &gtk::graphene::Point::new(x as f32, y as f32)) {
                        pool.show_menu(bin_id.map(|id| (id, ItemKind::Bin)), point.x() as f64, point.y() as f64);
                    }
                }
            });
            row.add_controller(click);

            if bin_id == cur {
                select_row = Some(row.clone());
            }
            ids.push(bin_id);
            self.sidebar_list.append(&row);
        }
        *self.sidebar_bins.borrow_mut() = ids;
        self.sidebar_list.select_row(select_row.as_ref());
        self.syncing_sidebar.set(false);
    }

    fn cached_thumbnail(&self, id: Uuid, data: Option<&Vec<u8>>) -> Option<gdk::Texture> {
        let data = data?;
        let mut cache = self.thumb_cache.borrow_mut();
        if let Some((len, tex)) = cache.get(&id) {
            if *len == data.len() {
                return Some(tex.clone());
            }
        }
        let tex = thumbnail_texture(data)?;
        cache.insert(id, (data.len(), tex.clone()));
        Some(tex)
    }

    fn reload(self: &Rc<Self>) {
        let query = self.search.text().trim().to_lowercase();
        let sort = self.sort_mode.get();

        // Reset current_bin if it disappeared (e.g. after project reload or deletion).
        if let Some(bid) = self.current_bin.get() {
            let exists = self.state.with_project(|p| p.bins.iter().any(|b| b.id == bid)).unwrap_or(false);
            if !exists {
                self.current_bin.set(None);
            }
        }
        let cur_bin = self.current_bin.get();
        self.reload_breadcrumbs();
        self.reload_sidebar();

        let mut items: Vec<Item> = Vec::new();
        if let Some((mut bins, mut sources, mut timelines)) = self.state.with_project(|p| {
            let bins: Vec<Item> = p
                .bins
                .iter()
                .filter(|b| {
                    if query.is_empty() {
                        b.parent_id == cur_bin
                    } else {
                        b.name.to_lowercase().contains(&query)
                    }
                })
                .map(|b| {
                    let count = p.bins.iter().filter(|c| c.parent_id == Some(b.id)).count()
                        + p.sources.values().filter(|s| s.bin_id == Some(b.id)).count()
                        + std::iter::once(&p.timeline)
                            .chain(&p.timelines)
                            .filter(|t| t.bin_id == Some(b.id))
                            .count();
                    let detail = match count {
                        0 => "Empty".to_string(),
                        1 => "1 item".to_string(),
                        n => format!("{n} items"),
                    };
                    Item {
                        id: b.id,
                        kind: ItemKind::Bin,
                        name: b.name.clone(),
                        detail,
                        duration_us: 0,
                        import_order: 0,
                        sort_order: b.sort_order,
                        media_type: MediaType::Video,
                        missing: false,
                        open: false,
                        used: false,
                        thumbnail: None,
                    }
                })
                .collect();

            let sources: Vec<Item> = p
                .sources
                .values()
                .filter(|s| !query.is_empty() || s.bin_id == cur_bin)
                .filter_map(|s| {
                    let name = s.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    if !query.is_empty() && !name.to_lowercase().contains(&query) {
                        return None;
                    }
                    let detail = match (s.video_width, s.video_height, s.video_fps) {
                        (Some(w), Some(h), Some(fps)) => {
                            format!("{w} × {h}, {:.2} fps, {}", fps.to_f64(), s.video_codec.clone().unwrap_or_default())
                        }
                        _ => s.audio_codec.clone().unwrap_or_default(),
                    };
                    let thumbnail = self.cached_thumbnail(s.id, s.thumbnail_data.as_ref());
                    Some(Item {
                        id: s.id,
                        kind: ItemKind::Media,
                        name,
                        detail,
                        duration_us: s.duration_us,
                        import_order: s.import_order,
                        sort_order: 0,
                        media_type: s.media_type,
                        missing: s.is_missing,
                        open: false,
                        used: p.source_used_in_open_timeline(s.id),
                        thumbnail,
                    })
                })
                .collect();

            let timelines: Vec<Item> = std::iter::once(&p.timeline)
                .chain(&p.timelines)
                .filter(|t| !query.is_empty() || t.bin_id == cur_bin)
                .filter(|t| query.is_empty() || t.name.to_lowercase().contains(&query))
                .map(|t| Item {
                    id: t.id,
                    kind: ItemKind::Timeline,
                    name: t.name.clone(),
                    detail: "Timeline".into(),
                    duration_us: t.duration_us(),
                    import_order: 0,
                    sort_order: 0,
                    media_type: MediaType::Video,
                    missing: false,
                    open: t.id == p.timeline.id,
                    used: false,
                    thumbnail: None,
                })
                .collect();

            (bins, sources, timelines)
        }) {
            sort_bins(&mut bins, sort);
            sort_sources(&mut sources, sort);
            timelines.sort_by_key(|a| a.name.to_lowercase());
            items.extend(bins);
            items.extend(sources);
            items.extend(timelines);
        }

        // Rebuilding the list clears the selection; put it back, so a clip picked
        // while other files are still importing stays picked.
        let selected = self.state.media_selection.get();
        self.store.remove_all();
        let mut reselect = None;
        for (index, item) in items.into_iter().enumerate() {
            if Some(item.id) == selected && item.kind == ItemKind::Media {
                reselect = Some(index as u32);
            }
            self.store.append(&glib::BoxedAnyObject::new(item));
        }
        match reselect {
            Some(index) => self.selection.set_selected(index),
            None => self.state.media_selection.set(
                selected.filter(|id| self.state.with_project(|p| p.sources.contains_key(id)).unwrap_or(false)),
            ),
        }

        self.update_visible_stack();
    }

    fn update_visible_stack(&self) {
        let has_project = self.state.project.borrow().is_some();
        let is_empty = self.store.n_items() == 0;
        let query = self.search.text().trim().to_string();
        let cur_bin = self.current_bin.get();

        if !has_project {
            self.empty_hint.set_text("Drop video, audio or images here,\nor press Ctrl+I");
            self.stack.set_visible_child_name("empty");
        } else if is_empty {
            self.empty_hint.set_text(if !query.is_empty() {
                "No matching clips, timelines or bins"
            } else {
                "This bin is empty.\nDrop video, audio or images here, or press Ctrl+I"
            });
            self.stack.set_visible_child_name("empty");
        } else if self.list_mode.get() {
            self.stack.set_visible_child_name("list");
        } else {
            self.stack.set_visible_child_name("grid");
        }

        // With no media in Master, the hint sits right under the timeline items.
        let no_media = has_project
            && !is_empty
            && cur_bin.is_none()
            && query.is_empty()
            && self.state.with_project(|p| p.sources.is_empty()).unwrap_or(false);
        self.media_hint.set_visible(no_media);
        self.stack.set_vexpand(!no_media);
    }
}

fn collect_bin_tree(
    bins: &[tempo_timeline::Bin],
    parent_id: Option<Uuid>,
    depth: usize,
    seen: &mut HashSet<Uuid>,
    out: &mut Vec<(Uuid, String, usize)>,
) {
    let mut children: Vec<&tempo_timeline::Bin> = bins.iter().filter(|b| b.parent_id == parent_id).collect();
    children.sort_by(|a, b| a.sort_order.cmp(&b.sort_order).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    for b in children {
        if !seen.insert(b.id) {
            continue;
        }
        out.push((b.id, b.name.clone(), depth));
        collect_bin_tree(bins, Some(b.id), depth + 1, seen, out);
    }
}

fn sort_bins(bins: &mut [Item], sort: SortMode) {
    match sort {
        SortMode::Name => bins.sort_by_key(|a| a.name.to_lowercase()),
        SortMode::ImportOrder | SortMode::Duration => bins.sort_by(|a, b| {
            a.sort_order
                .cmp(&b.sort_order)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        }),
    }
}

fn sort_sources(sources: &mut [Item], sort: SortMode) {
    match sort {
        SortMode::ImportOrder => sources.sort_by(|a, b| {
            a.import_order
                .cmp(&b.import_order)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        }),
        SortMode::Name => sources.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then_with(|| a.import_order.cmp(&b.import_order))
        }),
        SortMode::Duration => sources.sort_by(|a, b| {
            a.duration_us
                .cmp(&b.duration_us)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        }),
    }
}

/// Whether `typed` may name item `id`: not blank, not another sibling's name (any case).
fn name_available(existing: &[(Uuid, String)], id: Uuid, typed: &str) -> bool {
    let typed = typed.trim().to_lowercase();
    !typed.is_empty() && !existing.iter().any(|(i, n)| *i != id && n.to_lowercase() == typed)
}

/// Probe thumbnails are raw RGBA; work out their size from the byte count.
fn thumbnail_texture(data: &[u8]) -> Option<gdk::Texture> {
    let (w, h) = [(160, 90), (320, 180), (96, 54)].into_iter().find(|(w, h)| w * h * 4 == data.len())?;
    let bytes = glib::Bytes::from(data);
    Some(gdk::MemoryTexture::new(w as i32, h as i32, gdk::MemoryFormat::R8g8b8a8, &bytes, w * 4).upcast())
}

fn is_supported_media_extension(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()).map(|s| s.to_ascii_lowercase()) else {
        return false;
    };
    matches!(
        ext.as_str(),
        "mp4"
            | "mov"
            | "mkv"
            | "webm"
            | "avi"
            | "m4v"
            | "mts"
            | "m2ts"
            | "ts"
            | "mpg"
            | "mpeg"
            | "flv"
            | "wmv"
            | "ogv"
            | "wav"
            | "mp3"
            | "aac"
            | "flac"
            | "ogg"
            | "opus"
            | "m4a"
            | "wma"
            | "aiff"
            | "aif"
            | "png"
            | "jpg"
            | "jpeg"
            | "webp"
            | "bmp"
            | "tiff"
            | "tif"
            | "gif"
    )
}

fn scan_media_folder(folder: &Path) -> Vec<PathBuf> {
    const MAX_DEPTH: usize = 10;
    const MAX_FILES: usize = 10_000;
    let mut files = Vec::new();
    let mut stack = vec![(folder.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            if files.len() >= MAX_FILES {
                break;
            }
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let path = entry.path();
            if path.is_dir() {
                if depth < MAX_DEPTH {
                    stack.push((path, depth + 1));
                }
            } else if path.is_file() && is_supported_media_extension(&path) {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

fn expand_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for path in paths {
        if path.is_dir() {
            out.extend(scan_media_folder(&path));
        } else {
            out.push(path);
        }
    }
    out
}

fn meta_from_item(item: &Item) -> CellMeta {
    CellMeta {
        id: item.id,
        kind: item.kind,
        duration_us: item.duration_us,
        hover_scrubbable: item.kind == ItemKind::Media
            && !item.missing
            && item.media_type == MediaType::Video
            && item.duration_us > 0,
    }
}

fn attach_cell_controllers(
    widget: &impl IsA<gtk::Widget>,
    hover_target: Option<&gtk::Widget>,
    drop_highlight: Option<&gtk::Widget>,
    meta_fn: Rc<dyn Fn() -> Option<CellMeta>>,
    thumb_of: Rc<dyn Fn() -> Option<gdk::Texture>>,
    position_of: Rc<dyn Fn() -> u32>,
    pool: &Rc<MediaPool>,
) {
    let weak_pool = Rc::downgrade(pool);
    let drag = gtk::DragSource::builder().actions(gdk::DragAction::COPY).build();
    let meta = meta_fn.clone();
    drag.connect_prepare(move |_, _, _| {
        let payload = meta()?.drag_payload()?;
        Some(gdk::ContentProvider::for_value(&payload.to_value()))
    });
    let thumb_fn = thumb_of;
    let wp = weak_pool.clone();
    drag.connect_drag_begin(move |source, _| {
        if let Some(p) = wp.upgrade() {
            p.end_hover_scrub();
        }
        if let Some(t) = thumb_fn() {
            source.set_icon(Some(&t), 0, 0);
        }
    });
    widget.add_controller(drag);

    // Bin cells accept dragged media sources or timelines.
    let drop = gtk::DropTarget::new(glib::Type::STRING, gdk::DragAction::COPY);
    let meta = meta_fn.clone();
    drop.connect_accept(move |_, _| meta().is_some_and(|m| m.kind == ItemKind::Bin));
    let meta = meta_fn.clone();
    let hl = drop_highlight.map(|w| w.downgrade());
    drop.connect_enter(move |_, _, _| {
        if meta().is_some_and(|m| m.kind == ItemKind::Bin) {
            if let Some(w) = hl.as_ref().and_then(|r| r.upgrade()) {
                w.add_css_class("drop-hover");
            }
            gdk::DragAction::COPY
        } else {
            gdk::DragAction::empty()
        }
    });
    let meta = meta_fn.clone();
    drop.connect_motion(move |_, _, _| {
        if meta().is_some_and(|m| m.kind == ItemKind::Bin) {
            gdk::DragAction::COPY
        } else {
            gdk::DragAction::empty()
        }
    });
    let hl = drop_highlight.map(|w| w.downgrade());
    drop.connect_leave(move |_| {
        if let Some(w) = hl.as_ref().and_then(|r| r.upgrade()) {
            w.remove_css_class("drop-hover");
        }
    });
    let meta = meta_fn.clone();
    let hl = drop_highlight.map(|w| w.downgrade());
    let wp = weak_pool.clone();
    drop.connect_drop(move |_, value, _, _| {
        if let Some(w) = hl.as_ref().and_then(|r| r.upgrade()) {
            w.remove_css_class("drop-hover");
        }
        let Some(m) = meta().filter(|m| m.kind == ItemKind::Bin) else {
            return false;
        };
        let Some(p) = wp.upgrade() else { return false };
        value.get::<String>().is_ok_and(|text| p.handle_bin_drop(&text, Some(m.id)))
    });
    widget.add_controller(drop);

    let click = gtk::GestureClick::builder().button(gdk::BUTTON_SECONDARY).build();
    let meta = meta_fn.clone();
    let pos_fn = position_of;
    let wp = weak_pool.clone();
    click.connect_pressed(move |gesture, _, x, y| {
        let (Some(p), Some(w), Some(m)) = (wp.upgrade(), gesture.widget(), meta()) else { return };
        gesture.set_state(gtk::EventSequenceState::Claimed);
        p.selection.set_selected(pos_fn());
        if let Some(point) = w.compute_point(&p.root, &gtk::graphene::Point::new(x as f32, y as f32)) {
            p.show_menu(Some((m.id, m.kind)), point.x() as f64, point.y() as f64);
        }
    });
    widget.add_controller(click);

    if let Some(hover_widget) = hover_target {
        let motion = gtk::EventControllerMotion::new();
        let meta = meta_fn.clone();
        let wp = weak_pool.clone();
        motion.connect_enter(move |ctl, x, _| {
            let (Some(p), Some(w), Some(m)) = (wp.upgrade(), ctl.widget(), meta()) else { return };
            if m.hover_scrubbable {
                p.schedule_hover_scrub(m.id, m.duration_us, x, w.width());
            }
        });
        let meta = meta_fn;
        let wp = weak_pool.clone();
        motion.connect_motion(move |ctl, x, _| {
            let (Some(p), Some(w), Some(m)) = (wp.upgrade(), ctl.widget(), meta()) else { return };
            if m.hover_scrubbable {
                p.schedule_hover_scrub(m.id, m.duration_us, x, w.width());
            }
        });
        let wp = weak_pool;
        motion.connect_leave(move |_| {
            if let Some(p) = wp.upgrade() {
                p.end_hover_scrub();
            }
        });
        hover_widget.add_controller(motion);
    }
}

fn grid_factory(pool: &Rc<MediaPool>) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    let weak_pool = Rc::downgrade(pool);
    let thumb_width_setup = pool.thumb_width.clone();
    let thumb_width_bind = pool.thumb_width.clone();
    let thumb_cells = pool.thumb_cells.clone();

    factory.connect_setup(move |_, obj| {
        let Some(list_item) = obj.downcast_ref::<gtk::ListItem>() else { return };
        let Some(pool_rc) = weak_pool.upgrade() else { return };
        let w = thumb_width_setup.get();
        let h = w * 9 / 16;

        let picture = gtk::Picture::builder().content_fit(gtk::ContentFit::Cover).build();
        picture.set_size_request(w, h);
        let icon = gtk::Image::builder().pixel_size(24).build();
        let duration = label("", &["badge"]);
        duration.set_halign(gtk::Align::End);
        duration.set_valign(gtk::Align::End);
        duration.set_margin_end(2);
        duration.set_margin_bottom(2);
        let used_bar = gtk::Box::builder()
            .height_request(3)
            .hexpand(true)
            .valign(gtk::Align::End)
            .can_target(false)
            .css_classes(["media-used-bar"])
            .visible(false)
            .build();
        let thumb = gtk::Overlay::builder()
            .child(&picture)
            .css_classes(["media-thumb"])
            .overflow(gtk::Overflow::Hidden)
            .build();
        thumb.add_overlay(&icon);
        thumb.add_overlay(&duration);
        thumb.add_overlay(&used_bar);

        let name = gtk::Label::builder()
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .max_width_chars((w / 8).max(8))
            .css_classes(["tempo-small"])
            .build();
        thumb_cells.borrow_mut().push((picture.downgrade(), name.downgrade()));

        let cell = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).build();
        cell.append(&thumb);
        cell.append(&name);

        let weak_item = list_item.downgrade();
        let meta_fn: Rc<dyn Fn() -> Option<CellMeta>> = Rc::new(move || {
            let obj = weak_item.upgrade()?.item().and_downcast::<glib::BoxedAnyObject>()?;
            let item = obj.borrow::<Item>();
            Some(meta_from_item(&item))
        });
        let weak_item = list_item.downgrade();
        let thumb_fn: Rc<dyn Fn() -> Option<gdk::Texture>> = Rc::new(move || {
            weak_item
                .upgrade()
                .and_then(|i| i.item())
                .and_downcast::<glib::BoxedAnyObject>()
                .and_then(|o| o.borrow::<Item>().thumbnail.clone())
        });
        let weak_item = list_item.downgrade();
        let pos_fn: Rc<dyn Fn() -> u32> = Rc::new(move || weak_item.upgrade().map_or(0, |i| i.position()));

        attach_cell_controllers(
            &cell,
            Some(thumb.upcast_ref()),
            Some(thumb.upcast_ref()),
            meta_fn,
            thumb_fn,
            pos_fn,
            &pool_rc,
        );
        list_item.set_child(Some(&cell));
    });

    factory.connect_bind(move |_, obj| {
        let Some(list_item) = obj.downcast_ref::<gtk::ListItem>() else { return };
        let Some(boxed) = list_item.item().and_downcast::<glib::BoxedAnyObject>() else { return };
        let item = boxed.borrow::<Item>();
        let Some(cell) = list_item.child().and_downcast::<gtk::Box>() else { return };
        let Some(thumb) = cell.first_child().and_downcast::<gtk::Overlay>() else { return };
        let Some(name) = cell.last_child().and_downcast::<gtk::Label>() else { return };
        let picture = thumb.child().and_downcast::<gtk::Picture>();
        let icon = picture.as_ref().and_then(|p| p.next_sibling()).and_downcast::<gtk::Image>();
        let duration = icon.as_ref().and_then(|i| i.next_sibling()).and_downcast::<gtk::Label>();
        let used_bar = duration.as_ref().and_then(|d| d.next_sibling()).and_downcast::<gtk::Box>();

        let w = thumb_width_bind.get();
        if let Some(p) = &picture {
            p.set_size_request(w, w * 9 / 16);
            p.set_paintable(item.thumbnail.as_ref());
        }
        name.set_max_width_chars((w / 8).max(8));
        if let Some(i) = &icon {
            let icon_name = item_icon_name(&item);
            i.set_icon_name(icon_name);
            i.set_visible(icon_name.is_some());
        }
        if let Some(d) = &duration {
            d.set_text(&short_duration(item.duration_us));
            d.set_visible(item.kind == ItemKind::Media && item.media_type != MediaType::Image);
        }
        if let Some(u) = &used_bar {
            u.set_visible(item.kind == ItemKind::Media && item.used);
        }
        name.set_text(&item.name);

        if item.kind == ItemKind::Bin {
            thumb.add_css_class("bin-cell");
        } else {
            thumb.remove_css_class("bin-cell");
        }
        if item.kind == ItemKind::Timeline && item.open {
            thumb.add_css_class("pool-open-timeline");
        } else {
            thumb.remove_css_class("pool-open-timeline");
        }
        cell.set_tooltip_text(Some(&item_tooltip(&item)));
    });
    factory
}

#[derive(Clone, Copy)]
enum ListCol {
    Duration,
    Type,
}

fn list_cell_fns(cell: &gtk::ColumnViewCell) -> (Rc<dyn Fn() -> Option<CellMeta>>, Rc<dyn Fn() -> Option<gdk::Texture>>, Rc<dyn Fn() -> u32>) {
    let w1 = cell.downgrade();
    let meta_fn: Rc<dyn Fn() -> Option<CellMeta>> = Rc::new(move || {
        let obj = w1.upgrade()?.item().and_downcast::<glib::BoxedAnyObject>()?;
        let item = obj.borrow::<Item>();
        Some(meta_from_item(&item))
    });
    let w2 = cell.downgrade();
    let thumb_fn: Rc<dyn Fn() -> Option<gdk::Texture>> = Rc::new(move || {
        w2.upgrade()
            .and_then(|c| c.item())
            .and_downcast::<glib::BoxedAnyObject>()
            .and_then(|o| o.borrow::<Item>().thumbnail.clone())
    });
    let w3 = cell.downgrade();
    let pos_fn: Rc<dyn Fn() -> u32> = Rc::new(move || w3.upgrade().map_or(0, |c| c.position()));
    (meta_fn, thumb_fn, pos_fn)
}

fn list_name_factory(pool: &Rc<MediaPool>) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    let weak_pool = Rc::downgrade(pool);
    factory.connect_setup(move |_, obj| {
        let Some(cell) = obj.downcast_ref::<gtk::ColumnViewCell>() else { return };
        let Some(pool_rc) = weak_pool.upgrade() else { return };

        let picture = gtk::Picture::builder().content_fit(gtk::ContentFit::Cover).build();
        picture.set_size_request(28, 16);
        let icon = gtk::Image::builder().pixel_size(14).build();
        let used_bar = gtk::Box::builder()
            .height_request(3)
            .hexpand(true)
            .valign(gtk::Align::End)
            .can_target(false)
            .css_classes(["media-used-bar"])
            .visible(false)
            .build();
        let thumb = gtk::Overlay::builder()
            .child(&picture)
            .css_classes(["media-thumb"])
            .overflow(gtk::Overflow::Hidden)
            .build();
        thumb.add_overlay(&icon);
        thumb.add_overlay(&used_bar);

        let name = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .hexpand(true)
            .css_classes(["tempo-small"])
            .build();
        let row = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(6)
            .margin_start(4)
            .margin_end(4)
            .margin_top(2)
            .margin_bottom(2)
            .build();
        row.append(&thumb);
        row.append(&name);

        let (meta_fn, thumb_fn, pos_fn) = list_cell_fns(cell);
        attach_cell_controllers(
            &row,
            None,
            Some(thumb.upcast_ref()),
            meta_fn,
            thumb_fn,
            pos_fn,
            &pool_rc,
        );
        cell.set_child(Some(&row));
    });
    factory.connect_bind(|_, obj| {
        let Some(cell) = obj.downcast_ref::<gtk::ColumnViewCell>() else { return };
        let Some(boxed) = cell.item().and_downcast::<glib::BoxedAnyObject>() else { return };
        let item = boxed.borrow::<Item>();
        let Some(row) = cell.child().and_downcast::<gtk::Box>() else { return };
        let Some(thumb) = row.first_child().and_downcast::<gtk::Overlay>() else { return };
        let Some(name) = row.last_child().and_downcast::<gtk::Label>() else { return };
        let picture = thumb.child().and_downcast::<gtk::Picture>();
        let icon = picture.as_ref().and_then(|p| p.next_sibling()).and_downcast::<gtk::Image>();
        let used_bar = icon.as_ref().and_then(|i| i.next_sibling()).and_downcast::<gtk::Box>();

        if let Some(p) = &picture {
            p.set_paintable(item.thumbnail.as_ref());
        }
        if let Some(i) = &icon {
            let icon_name = item_icon_name(&item);
            i.set_icon_name(icon_name);
            i.set_visible(icon_name.is_some());
        }
        if let Some(u) = &used_bar {
            u.set_visible(item.kind == ItemKind::Media && item.used);
        }
        if item.kind == ItemKind::Bin {
            thumb.add_css_class("bin-cell");
        } else {
            thumb.remove_css_class("bin-cell");
        }
        if item.kind == ItemKind::Timeline && item.open {
            thumb.add_css_class("pool-open-timeline");
        } else {
            thumb.remove_css_class("pool-open-timeline");
        }
        name.set_text(&item.name);
        row.set_tooltip_text(Some(&item_tooltip(&item)));
    });
    factory
}

fn list_text_factory(pool: &Rc<MediaPool>, col: ListCol) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    let weak_pool = Rc::downgrade(pool);
    factory.connect_setup(move |_, obj| {
        let Some(cell) = obj.downcast_ref::<gtk::ColumnViewCell>() else { return };
        let Some(pool_rc) = weak_pool.upgrade() else { return };
        let lbl = gtk::Label::builder()
            .xalign(if matches!(col, ListCol::Duration) { 1.0 } else { 0.0 })
            .margin_start(4)
            .margin_end(4)
            .css_classes(["tempo-small", "tempo-dim"])
            .build();
        let (meta_fn, thumb_fn, pos_fn) = list_cell_fns(cell);
        attach_cell_controllers(&lbl, None, None, meta_fn, thumb_fn, pos_fn, &pool_rc);
        cell.set_child(Some(&lbl));
    });
    factory.connect_bind(move |_, obj| {
        let Some(cell) = obj.downcast_ref::<gtk::ColumnViewCell>() else { return };
        let Some(boxed) = cell.item().and_downcast::<glib::BoxedAnyObject>() else { return };
        let item = boxed.borrow::<Item>();
        let Some(lbl) = cell.child().and_downcast::<gtk::Label>() else { return };
        match col {
            ListCol::Duration => {
                let text = match item.kind {
                    ItemKind::Media if item.media_type != MediaType::Image => short_duration(item.duration_us),
                    ItemKind::Timeline if item.duration_us > 0 => short_duration(item.duration_us),
                    _ => String::new(),
                };
                lbl.set_text(&text);
            }
            ListCol::Type => lbl.set_text(item.type_label()),
        }
    });
    factory
}

fn item_icon_name(item: &Item) -> Option<&'static str> {
    match (item.kind, item.missing, item.thumbnail.is_some(), item.media_type) {
        (ItemKind::Bin, _, _, _) => Some("folder-symbolic"),
        (ItemKind::Timeline, _, _, _) => Some("view-list-symbolic"),
        (ItemKind::Media, true, _, _) => Some("dialog-warning-symbolic"),
        (ItemKind::Media, false, true, _) => None,
        (ItemKind::Media, false, false, MediaType::Audio) => Some("audio-x-generic-symbolic"),
        (ItemKind::Media, false, false, MediaType::Image) => Some("image-x-generic-symbolic"),
        (ItemKind::Media, false, false, MediaType::Video) => Some("video-x-generic-symbolic"),
    }
}

fn item_tooltip(item: &Item) -> String {
    match item.kind {
        ItemKind::Bin => format!("{}\nBin ({}). Double-click to open.", item.name, item.detail),
        ItemKind::Timeline => {
            if item.open {
                format!("{}\nTimeline (open)", item.name)
            } else {
                format!("{}\nTimeline. Double-click to open.", item.name)
            }
        }
        ItemKind::Media => {
            if item.missing {
                format!("{} — file not found", item.name)
            } else {
                format!("{}\n{}", item.name, item.detail)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_item(name: &str, import_order: u32, duration_us: i64) -> Item {
        Item {
            id: Uuid::new_v4(),
            kind: ItemKind::Media,
            name: name.to_string(),
            detail: String::new(),
            duration_us,
            import_order,
            sort_order: 0,
            media_type: MediaType::Video,
            missing: false,
            open: false,
            used: false,
            thumbnail: None,
        }
    }

    #[test]
    fn name_available_refuses_blank_and_other_siblings_names() {
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let list = vec![(a, "Main".to_string()), (b, "Alt".to_string())];
        assert!(!name_available(&list, a, "  "));
        assert!(!name_available(&list, a, "alt"));
        assert!(name_available(&list, a, "MAIN"));
        assert!(name_available(&list, a, "Fresh"));
    }

    #[test]
    fn sort_sources_orders_by_import_name_and_duration() {
        let mut items = vec![
            test_item("zebra.mp4", 0, 6_000_000),
            test_item("Alpha.mp4", 2, 2_000_000),
            test_item("beta.mp4", 1, 10_000_000),
        ];
        sort_sources(&mut items, SortMode::ImportOrder);
        assert_eq!(items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["zebra.mp4", "beta.mp4", "Alpha.mp4"]);

        sort_sources(&mut items, SortMode::Name);
        assert_eq!(items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["Alpha.mp4", "beta.mp4", "zebra.mp4"]);

        sort_sources(&mut items, SortMode::Duration);
        assert_eq!(items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["Alpha.mp4", "zebra.mp4", "beta.mp4"]);
    }

    #[test]
    fn scan_media_folder_collects_nested_media_and_skips_hidden_and_other_files() {
        let dir = std::env::temp_dir().join(format!("tempo-media-pool-test-{}", Uuid::new_v4()));
        let sub = dir.join("day1");
        let hidden = dir.join(".hidden");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::create_dir_all(&hidden).unwrap();
        std::fs::write(dir.join("clip.MP4"), b"").unwrap();
        std::fs::write(dir.join("notes.txt"), b"").unwrap();
        std::fs::write(sub.join("take.wav"), b"").unwrap();
        std::fs::write(sub.join(".ignored.mp4"), b"").unwrap();
        std::fs::write(hidden.join("secret.png"), b"").unwrap();

        let found = scan_media_folder(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        let names: Vec<_> = found.iter().filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).collect();
        assert_eq!(names, vec!["clip.MP4", "take.wav"]);
    }

    #[test]
    fn timeline_drag_payload_uses_prefix_while_media_stays_plain_uuid() {
        let media = test_item("a.mp4", 0, 1_000_000);
        assert_eq!(meta_from_item(&media).drag_payload(), Some(media.id.to_string()));
        let mut tl = test_item("Timeline 1", 0, 0);
        tl.kind = ItemKind::Timeline;
        assert_eq!(meta_from_item(&tl).drag_payload(), Some(format!("timeline:{}", tl.id)));
        let mut bin = test_item("Footage", 0, 0);
        bin.kind = ItemKind::Bin;
        assert_eq!(meta_from_item(&bin).drag_payload(), None);
    }
}
