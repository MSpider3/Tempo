//! Media Pool: the files imported into the project.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use gtk4 as gtk;
use gtk4::prelude::*;
use gtk4::{gdk, gio};
use tempo_media::MediaEngine;
use tempo_timeline::MediaType;
use uuid::Uuid;

use crate::actions::{self, Place};
use crate::state::{AppState, Change};
use crate::util::{label, short_duration, tool_button};

const THUMB_W: i32 = 96;
const THUMB_H: i32 = 54;

/// One row of the grid model.
struct Item {
    id: Uuid,
    name: String,
    detail: String,
    duration_us: i64,
    media_type: MediaType,
    missing: bool,
    /// A timeline of the project rather than a media file.
    timeline: bool,
    /// The timeline that is open.
    open: bool,
    thumbnail: Option<gdk::Texture>,
}

pub struct MediaPool {
    pub root: gtk::Box,
    state: Rc<AppState>,
    store: gio::ListStore,
    selection: gtk::SingleSelection,
    /// The right-click menu while it is open.
    menu: RefCell<Option<gtk::PopoverMenu>>,
    stack: gtk::Stack,
    /// The drop hint under the timeline items while the project has no media yet.
    media_hint: gtk::Label,
    search: gtk::SearchEntry,
    window: glib::WeakRef<gtk::Window>,
}

impl MediaPool {
    pub fn new(state: &Rc<AppState>, on_open: impl Fn(Uuid) + 'static) -> Rc<Self> {
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .width_request(300)
            .hexpand(false)
            .css_classes(["tempo-surface", "tempo-sep-right"])
            .build();

        let import = tool_button("list-add-symbolic", "Import media (Ctrl+I)");
        let search = gtk::SearchEntry::builder().placeholder_text("Search").hexpand(true).build();
        let header = gtk::Box::builder().spacing(6).margin_top(6).margin_bottom(6).margin_start(8).margin_end(8).build();
        header.append(&label("Master", &["tempo-heading", "tempo-bright"]));
        header.append(&search);
        header.append(&import);
        root.append(&header);

        // Filled in below, once the pool exists: called when a clip is right-clicked.
        let on_menu: MenuHandler = Rc::new(RefCell::new(None));
        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let selection = gtk::SingleSelection::builder().model(&store).autoselect(false).can_unselect(true).build();
        let grid = gtk::GridView::builder()
            .model(&selection)
            .factory(&item_factory(on_menu.clone()))
            .max_columns(6)
            .min_columns(2)
            .css_classes(["media"])
            .build();
        let scrolled = gtk::ScrolledWindow::builder().child(&grid).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build();

        let hint = label("Drop video, audio or images here,\nor press Ctrl+I", &["drop-hint"]);
        hint.set_justify(gtk::Justification::Center);
        hint.set_valign(gtk::Align::Start);

        let stack = gtk::Stack::new();
        stack.add_named(&hint, Some("empty"));
        let media_hint = label("Drop video, audio or images here,\nor press Ctrl+I", &["drop-hint"]);
        media_hint.set_justify(gtk::Justification::Center);
        media_hint.set_vexpand(true);
        media_hint.set_valign(gtk::Align::Start);
        let page = gtk::Box::new(gtk::Orientation::Vertical, 0);
        page.append(&scrolled);
        page.append(&media_hint);
        stack.add_named(&page, Some("grid"));
        stack.set_vexpand(true);
        root.append(&stack);

        let pool = Rc::new(Self {
            root,
            state: state.clone(),
            store,
            selection: selection.clone(),
            menu: RefCell::new(None),
            stack,
            media_hint,
            search,
            window: glib::WeakRef::new(),
        });
        pool.install_menu_actions();
        let p = pool.clone();
        *on_menu.borrow_mut() = Some(Box::new(move |position, widget, x, y| {
            // Right-clicking a clip selects it, as a left click would.
            p.selection.set_selected(position);
            let id = p.store.item(position).and_downcast::<glib::BoxedAnyObject>().map(|o| (o.borrow::<Item>().id, o.borrow::<Item>().timeline));
            if let Some(point) = widget.compute_point(&p.root, &gtk::graphene::Point::new(x as f32, y as f32)) {
                p.show_menu(id, point.x() as f64, point.y() as f64);
            }
        }));
        // Right-clicking empty space offers Import.
        let click = gtk::GestureClick::builder().button(gdk::BUTTON_SECONDARY).build();
        let p = pool.clone();
        click.connect_pressed(move |_, _, x, y| p.show_menu(None, x, y));
        pool.root.add_controller(click);

        let p = pool.clone();
        import.connect_clicked(move |_| p.choose_files());

        let p = pool.clone();
        pool.search.connect_search_changed(move |_| p.reload());

        let s = state.clone();
        selection.connect_selected_item_notify(move |sel| {
            let id = sel.selected_item().and_downcast::<glib::BoxedAnyObject>().map(|o| o.borrow::<Item>().id);
            // Timelines are not media: they never become the clip the pool "has selected".
            let is_media = id.is_some_and(|id| s.with_project(|p| p.sources.contains_key(&id)).unwrap_or(false));
            s.media_selection.set(id.filter(|_| is_media));
        });

        // Double-click (or Enter) opens the clip in the viewer.
        let store_for_open = pool.store.clone();
        let state_for_open = state.clone();
        grid.connect_activate(move |_, pos| {
            if let Some(obj) = store_for_open.item(pos).and_downcast::<glib::BoxedAnyObject>() {
                let item = obj.borrow::<Item>();
                if item.timeline {
                    state_for_open.open_timeline(item.id);
                } else {
                    on_open(item.id);
                }
            }
        });

        // Files dropped from the file manager.
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
            if matches!(change, Change::Project | Change::Media | Change::Timelines) {
                p.reload();
            }
        });
        pool.reload();
        pool
    }

    /// Actions behind the right-click menu. Each takes the clip's id as text.
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
        add("insert", Box::new(|p, id| actions::place_source(&p.state, id, Place::Insert, None, None)));
        add("overwrite", Box::new(|p, id| actions::place_source(&p.state, id, Place::Overwrite, None, None)));
        add("append", Box::new(|p, id| actions::place_source(&p.state, id, Place::Append, None, None)));
        add("open", Box::new(|p, id| p.state.show_source(Some(id))));
        add("remove", Box::new(|p, id| actions::remove_source(&p.state, id)));
        add("open-timeline", Box::new(|p, id| p.state.open_timeline(id)));
        add("rename-timeline", Box::new(|p, id| p.rename_dialog(id)));
        add("duplicate-timeline", Box::new(|p, id| {
            p.state.duplicate_timeline(id);
        }));
        add("delete-timeline", Box::new(|p, id| p.delete_dialog(id)));
        // Target of the greyed-out "Delete Timeline" item: no such action, so it stays disabled.
        add(
            "reveal",
            Box::new(|p, id| {
                if let Some(path) = p.state.with_project(|proj| proj.sources.get(&id).map(|s| s.path.clone())).flatten() {
                    gtk::FileLauncher::new(Some(&gio::File::for_path(path))).open_containing_folder(p.window.upgrade().as_ref(), gio::Cancellable::NONE, |_| {});
                }
            }),
        );
        let import = gio::SimpleAction::new("import", None);
        let pool = self.clone();
        import.connect_activate(move |_, _| pool.choose_files());
        group.add_action(&import);
        self.root.insert_action_group("pool", Some(&group));
    }

    /// The right-click menu, for a clip or (with `None`) for empty space.
    fn show_menu(&self, clip: Option<(Uuid, bool)>, x: f64, y: f64) {
        if self.state.project.borrow().is_none() {
            return;
        }
        let menu = gio::Menu::new();
        if let Some((id, true)) = clip {
            let part = gio::Menu::new();
            let is_open = self.state.with_timeline(|t| t.id) == Some(id);
            for (label, action) in [("Open", "pool.open-timeline"), ("Rename…", "pool.rename-timeline"), ("Duplicate Timeline", "pool.duplicate-timeline"), ("Delete Timeline", "pool.delete-timeline")] {
                // The open timeline cannot be deleted; a disabled item cannot be activated.
                if is_open && action == "pool.delete-timeline" {
                    part.append_item(&gio::MenuItem::new(Some(label), Some("pool.none")));
                    continue;
                }
                let item = gio::MenuItem::new(Some(label), None);
                item.set_action_and_target_value(Some(action), Some(&id.to_string().to_variant()));
                part.append_item(&item);
            }
            menu.append_section(None, &part);
        } else if let Some((id, false)) = clip {
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
                &section(&[("Insert at Playhead", "pool.insert"), ("Overwrite at Playhead", "pool.overwrite"), ("Add to End of Timeline", "pool.append")]),
            );
            menu.append_section(None, &section(&[("Open in Viewer", "pool.open"), ("Show in Files", "pool.reveal")]));
            menu.append_section(None, &section(&[("Remove from Media Pool", "pool.remove")]));
        }
        let import = gio::Menu::new();
        import.append(Some("Import Media…"), Some("pool.import"));
        let new_timeline = gio::MenuItem::new(Some("Create New Timeline…"), None);
        new_timeline.set_action_and_target_value(Some("win.do"), Some(&"win.new-timeline".to_variant()));
        import.append_item(&new_timeline);
        menu.append_section(None, &import);

        if let Some(old) = self.menu.take() {
            old.unparent();
        }
        let popover = gtk::PopoverMenu::from_model(Some(&menu));
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
        if let Some(obj) = self.store.item(index).and_downcast::<glib::BoxedAnyObject>() {
            self.state.media_selection.set(Some(obj.borrow::<Item>().id));
        }
    }

    /// The id of the nth item. For scripted checks.
    pub fn id_at(&self, index: u32) -> Option<Uuid> {
        self.store.item(index).and_downcast::<glib::BoxedAnyObject>().map(|o| o.borrow::<Item>().id)
    }

    /// Open the right-click menu of the nth item. For scripted checks.
    pub fn show_menu_for_index(&self, index: u32) {
        self.selection.set_selected(index);
        let id = self.store.item(index).and_downcast::<glib::BoxedAnyObject>().map(|o| (o.borrow::<Item>().id, o.borrow::<Item>().timeline));
        self.show_menu(id, 60.0, 90.0);
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

    /// Probe each file on a worker thread; nothing is converted.
    pub fn import(self: &Rc<Self>, paths: Vec<PathBuf>) {
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
                        let offer = may_offer.get().then(|| state.with_project(|p| p.first_import_offer(&source)).flatten()).flatten();
                        if let Some(p) = state.project.borrow_mut().as_mut() {
                            source.import_order = p.sources.len() as u32;
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

    /// Resolve's "Change Project Frame Rate?" for the first video in an empty project.
    fn offer_format(&self, offer: tempo_timeline::FormatOffer) {
        use libadwaita as adw;
        use libadwaita::prelude::*;
        let rate = |f: tempo_timeline::RationalFps| format!("{:.3}", f.to_f64()).trim_end_matches('0').trim_end_matches('.').to_string();
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
        use libadwaita as adw;
        use libadwaita::prelude::*;
        let Some(current) = self.state.with_project(|p| p.timeline_list().into_iter().find(|(i, _)| *i == id).map(|(_, n)| n)).flatten() else { return };
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
        use libadwaita as adw;
        use libadwaita::prelude::*;
        let Some(name) = self.state.with_project(|p| p.timeline_list().into_iter().find(|(i, _)| *i == id).map(|(_, n)| n)).flatten() else { return };
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

    fn reload(&self) {
        let query = self.search.text().to_lowercase();
        let mut items: Vec<(u32, Item)> = self
            .state
            .with_project(|p| {
                p.sources
                    .values()
                    .map(|s| {
                        let name = s.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        let detail = match (s.video_width, s.video_height, s.video_fps) {
                            (Some(w), Some(h), Some(fps)) => {
                                format!("{w} × {h}, {:.2} fps, {}", fps.to_f64(), s.video_codec.clone().unwrap_or_default())
                            }
                            _ => s.audio_codec.clone().unwrap_or_default(),
                        };
                        let thumbnail = s.thumbnail_data.as_ref().and_then(|data| thumbnail_texture(data));
                        (
                            s.import_order,
                            Item {
                                id: s.id,
                                name,
                                detail,
                                duration_us: s.duration_us,
                                media_type: s.media_type,
                                missing: s.is_missing,
                                timeline: false,
                                open: false,
                                thumbnail,
                            },
                        )
                    })
                    .filter(|(_, i)| query.is_empty() || i.name.to_lowercase().contains(&query))
                    .collect()
            })
            .unwrap_or_default();
        items.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.name.cmp(&b.1.name)));
        // Timelines follow the media, as items of the pool.
        let open = self.state.with_timeline(|t| t.id);
        let timelines = self.state.with_project(|p| p.timeline_list()).unwrap_or_default();
        items.extend(timelines.into_iter().filter(|(_, n)| query.is_empty() || n.to_lowercase().contains(&query)).map(|(id, name)| {
            let item = Item {
                id,
                name,
                detail: "Timeline".into(),
                duration_us: 0,
                media_type: MediaType::Video,
                missing: false,
                timeline: true,
                open: open == Some(id),
                thumbnail: None,
            };
            (u32::MAX, item)
        }));

        // A project always has a timeline, so the grid shows whenever one is open.
        let has_any = self.state.project.borrow().is_some();
        // Rebuilding the list clears the selection; put it back, so a clip picked
        // while other files are still importing stays picked.
        let selected = self.state.media_selection.get();
        self.store.remove_all();
        let mut reselect = None;
        for (index, (_, item)) in items.into_iter().enumerate() {
            if Some(item.id) == selected {
                reselect = Some(index as u32);
            }
            self.store.append(&glib::BoxedAnyObject::new(item));
        }
        match reselect {
            Some(index) => self.selection.set_selected(index),
            None => self.state.media_selection.set(selected.filter(|id| self.state.with_project(|p| p.sources.contains_key(id)).unwrap_or(false))),
        }
        self.stack.set_visible_child_name(if has_any { "grid" } else { "empty" });
        // With no media the hint sits right under the timeline items, not at the bottom.
        let no_media = self.state.with_project(|p| p.sources.is_empty()).unwrap_or(false);
        self.media_hint.set_visible(no_media);
        if let Some(list) = self.media_hint.prev_sibling() {
            list.set_vexpand(!no_media);
        }
    }
}

/// Whether `typed` may name timeline `id`: not blank, not another timeline's name (any case).
fn name_available(timelines: &[(Uuid, String)], id: Uuid, typed: &str) -> bool {
    let typed = typed.trim().to_lowercase();
    !typed.is_empty() && !timelines.iter().any(|(i, n)| *i != id && n.to_lowercase() == typed)
}

/// Probe thumbnails are raw RGBA; work out their size from the byte count.
fn thumbnail_texture(data: &[u8]) -> Option<gdk::Texture> {
    let (w, h) = [(160, 90), (320, 180), (96, 54)].into_iter().find(|(w, h)| w * h * 4 == data.len())?;
    let bytes = glib::Bytes::from(data);
    Some(gdk::MemoryTexture::new(w as i32, h as i32, gdk::MemoryFormat::R8g8b8a8, &bytes, w * 4).upcast())
}

/// Called with the item's position, the cell and the pointer position in it.
type MenuHandler = Rc<RefCell<Option<Box<dyn Fn(u32, &gtk::Widget, f64, f64)>>>>;

fn item_factory(on_menu: MenuHandler) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(move |_, obj| {
        let Some(list_item) = obj.downcast_ref::<gtk::ListItem>() else { return };
        let picture = gtk::Picture::builder().content_fit(gtk::ContentFit::Cover).build();
        picture.set_size_request(THUMB_W, THUMB_H);
        let icon = gtk::Image::builder().pixel_size(24).build();
        let duration = label("", &["badge"]);
        duration.set_halign(gtk::Align::End);
        duration.set_valign(gtk::Align::End);
        duration.set_margin_end(2);
        duration.set_margin_bottom(2);
        let thumb = gtk::Overlay::builder().child(&picture).css_classes(["media-thumb"]).overflow(gtk::Overflow::Hidden).build();
        thumb.add_overlay(&icon);
        thumb.add_overlay(&duration);
        let name = gtk::Label::builder()
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .max_width_chars(12)
            .css_classes(["tempo-small"])
            .build();
        let cell = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).build();
        cell.append(&thumb);
        cell.append(&name);

        // Dragging a cell carries the source id as text; the timeline accepts it.
        let drag = gtk::DragSource::builder().actions(gdk::DragAction::COPY).build();
        let weak_item = list_item.downgrade();
        drag.connect_prepare(move |_, _, _| {
            let obj = weak_item.upgrade()?.item().and_downcast::<glib::BoxedAnyObject>()?;
            let id = obj.borrow::<Item>().id.to_string();
            Some(gdk::ContentProvider::for_value(&id.to_value()))
        });
        // The picture under the pointer is the clip's thumbnail, as in Resolve.
        let weak_item = list_item.downgrade();
        drag.connect_drag_begin(move |source, _| {
            let thumb = weak_item.upgrade().and_then(|i| i.item()).and_downcast::<glib::BoxedAnyObject>().and_then(|o| o.borrow::<Item>().thumbnail.clone());
            if let Some(t) = thumb {
                source.set_icon(Some(&t), 0, 0);
            }
        });
        cell.add_controller(drag);

        let click = gtk::GestureClick::builder().button(gdk::BUTTON_SECONDARY).build();
        let weak_item = list_item.downgrade();
        let on_menu = on_menu.clone();
        click.connect_pressed(move |gesture, _, x, y| {
            let (Some(item), Some(widget)) = (weak_item.upgrade(), gesture.widget()) else { return };
            // Claimed, so the pool's own "empty space" menu does not open as well.
            gesture.set_state(gtk::EventSequenceState::Claimed);
            if let Some(show) = on_menu.borrow().as_ref() {
                show(item.position(), &widget, x, y);
            }
        });
        cell.add_controller(click);
        list_item.set_child(Some(&cell));
    });
    factory.connect_bind(|_, obj| {
        let Some(list_item) = obj.downcast_ref::<gtk::ListItem>() else { return };
        let Some(boxed) = list_item.item().and_downcast::<glib::BoxedAnyObject>() else { return };
        let item = boxed.borrow::<Item>();
        let Some(cell) = list_item.child().and_downcast::<gtk::Box>() else { return };
        let Some(thumb) = cell.first_child().and_downcast::<gtk::Overlay>() else { return };
        let Some(name) = cell.last_child().and_downcast::<gtk::Label>() else { return };
        let picture = thumb.child().and_downcast::<gtk::Picture>();
        let icon = picture.as_ref().and_then(|p| p.next_sibling()).and_downcast::<gtk::Image>();
        let duration = icon.as_ref().and_then(|i| i.next_sibling()).and_downcast::<gtk::Label>();

        if let Some(p) = &picture {
            p.set_paintable(item.thumbnail.as_ref());
        }
        if let Some(i) = &icon {
            let name = match (item.missing, item.thumbnail.is_some(), item.media_type) {
                _ if item.timeline => Some("view-list-symbolic"),
                (true, _, _) => Some("dialog-warning-symbolic"),
                (_, true, _) => None,
                (_, _, MediaType::Audio) => Some("audio-x-generic-symbolic"),
                (_, _, MediaType::Image) => Some("image-x-generic-symbolic"),
                _ => Some("video-x-generic-symbolic"),
            };
            i.set_icon_name(name);
            i.set_visible(name.is_some());
        }
        if let Some(d) = &duration {
            d.set_text(&short_duration(item.duration_us));
            d.set_visible(item.media_type != MediaType::Image && !item.timeline);
        }
        name.set_text(&item.name);
        // The open timeline is marked.
        if item.open {
            thumb.add_css_class("pool-open-timeline");
        } else {
            thumb.remove_css_class("pool-open-timeline");
        }
        if item.timeline {
            let tip = if item.open { format!("{}\nTimeline (open)", item.name) } else { format!("{}\nTimeline. Double-click to open.", item.name) };
            cell.set_tooltip_text(Some(&tip));
            return;
        }
        let tip = if item.missing { format!("{} — file not found", item.name) } else { format!("{}\n{}", item.name, item.detail) };
        cell.set_tooltip_text(Some(&tip));
    });
    factory
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_available_refuses_blank_and_other_timelines_names() {
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let list = vec![(a, "Main".to_string()), (b, "Alt".to_string())];
        assert!(!name_available(&list, a, "  "));
        assert!(!name_available(&list, a, "alt"));
        assert!(name_available(&list, a, "MAIN"));
        assert!(name_available(&list, a, "Fresh"));
    }
}
