//! Media Pool: the files imported into the project.

use std::path::PathBuf;
use std::rc::Rc;

use gtk4 as gtk;
use gtk4::prelude::*;
use gtk4::{gdk, gio};
use tempo_media::MediaEngine;
use tempo_timeline::MediaType;
use uuid::Uuid;

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
    thumbnail: Option<gdk::Texture>,
}

pub struct MediaPool {
    pub root: gtk::Box,
    state: Rc<AppState>,
    store: gio::ListStore,
    stack: gtk::Stack,
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

        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let selection = gtk::SingleSelection::builder().model(&store).autoselect(false).can_unselect(true).build();
        let grid = gtk::GridView::builder()
            .model(&selection)
            .factory(&item_factory())
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
        stack.add_named(&scrolled, Some("grid"));
        stack.set_vexpand(true);
        root.append(&stack);

        let pool = Rc::new(Self {
            root,
            state: state.clone(),
            store,
            stack,
            search,
            window: glib::WeakRef::new(),
        });

        let p = pool.clone();
        import.connect_clicked(move |_| p.choose_files());

        let p = pool.clone();
        pool.search.connect_search_changed(move |_| p.reload());

        let s = state.clone();
        selection.connect_selected_item_notify(move |sel| {
            let id = sel.selected_item().and_downcast::<glib::BoxedAnyObject>().map(|o| o.borrow::<Item>().id);
            s.media_selection.set(id);
        });

        // Double-click (or Enter) opens the clip in the viewer.
        let store_for_open = pool.store.clone();
        grid.connect_activate(move |_, pos| {
            if let Some(obj) = store_for_open.item(pos).and_downcast::<glib::BoxedAnyObject>() {
                on_open(obj.borrow::<Item>().id);
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
            if matches!(change, Change::Project | Change::Media) {
                p.reload();
            }
        });
        pool.reload();
        pool
    }

    pub fn set_window(&self, window: &impl IsA<gtk::Window>) {
        self.window.set(Some(window.upcast_ref()));
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
        for path in paths {
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
                        if let Some(p) = state.project.borrow_mut().as_mut() {
                            source.import_order = p.sources.len() as u32;
                            p.sources.insert(source.id, source);
                        }
                        state.sync_player();
                        state.set_dirty(true);
                        state.emit(Change::Media);
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
                                thumbnail,
                            },
                        )
                    })
                    .filter(|(_, i)| query.is_empty() || i.name.to_lowercase().contains(&query))
                    .collect()
            })
            .unwrap_or_default();
        items.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.name.cmp(&b.1.name)));

        let has_any = self.state.with_project(|p| !p.sources.is_empty()).unwrap_or(false);
        self.store.remove_all();
        for (_, item) in items {
            self.store.append(&glib::BoxedAnyObject::new(item));
        }
        self.stack.set_visible_child_name(if has_any { "grid" } else { "empty" });
    }
}

/// Probe thumbnails are raw RGBA; work out their size from the byte count.
fn thumbnail_texture(data: &[u8]) -> Option<gdk::Texture> {
    let (w, h) = [(160, 90), (320, 180), (96, 54)].into_iter().find(|(w, h)| w * h * 4 == data.len())?;
    let bytes = glib::Bytes::from(data);
    Some(gdk::MemoryTexture::new(w as i32, h as i32, gdk::MemoryFormat::R8g8b8a8, &bytes, w * 4).upcast())
}

fn item_factory() -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, obj| {
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
        cell.add_controller(drag);
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
            d.set_visible(item.media_type != MediaType::Image);
        }
        name.set_text(&item.name);
        let tip = if item.missing { format!("{} — file not found", item.name) } else { format!("{}\n{}", item.name, item.detail) };
        cell.set_tooltip_text(Some(&tip));
    });
    factory
}
