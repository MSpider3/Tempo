//! Dialogs: decisions only (docs/VISUAL_DESIGN.md §7.3).

use std::path::PathBuf;
use std::rc::Rc;

use gtk4 as gtk;
use gtk4::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;
use tempo_timeline::{Marker, MarkerColor, RationalFps};

use crate::actions;
use crate::keybinds;
use crate::state::AppState;
use crate::util::label;

fn form_grid() -> gtk::Grid {
    gtk::Grid::builder().row_spacing(10).column_spacing(12).build()
}

fn form_row(grid: &gtk::Grid, row: i32, name: &str, widget: &impl IsA<gtk::Widget>) {
    let l = label(name, &["tempo-dim"]);
    l.set_xalign(1.0);
    grid.attach(&l, 0, row, 1, 1);
    widget.set_hexpand(true);
    grid.attach(widget, 1, row, 1, 1);
}

pub struct NewProject {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub fps: RationalFps,
    pub path: PathBuf,
}

const SHAPES: [(&str, (u32, u32)); 3] = [("Landscape 16:9", (16, 9)), ("Vertical 9:16", (9, 16)), ("Square 1:1", (1, 1))];
const SIZES: [(&str, u32); 3] = [("720p", 720), ("1080p", 1080), ("4K", 2160)];
const RATES: [(&str, RationalFps); 6] = [
    ("23.976", RationalFps::FPS_23_976),
    ("24", RationalFps::FPS_24),
    ("25", RationalFps::FPS_25),
    ("29.97", RationalFps::FPS_29_97),
    ("30", RationalFps::FPS_30),
    ("60", RationalFps::FPS_60),
];

pub fn new_project(parent: &impl IsA<gtk::Widget>, on_create: impl Fn(NewProject) + 'static) {
    let name = gtk::Entry::builder().text("Untitled Project").activates_default(true).build();
    let shape = gtk::DropDown::from_strings(&SHAPES.map(|s| s.0));
    let size = gtk::DropDown::from_strings(&SIZES.map(|s| s.0));
    size.set_selected(1);
    let rate = gtk::DropDown::from_strings(&RATES.map(|r| r.0));
    rate.set_selected(4);

    let grid = form_grid();
    form_row(&grid, 0, "Name", &name);
    form_row(&grid, 1, "Shape", &shape);
    form_row(&grid, 2, "Resolution", &size);
    form_row(&grid, 3, "Frame rate", &rate);

    let dialog = adw::AlertDialog::builder().heading("New Project").extra_child(&grid).build();
    dialog.add_responses(&[("cancel", "Cancel"), ("create", "Create")]);
    dialog.set_default_response(Some("create"));
    dialog.set_close_response("cancel");
    dialog.connect_response(None, move |_, response| {
        if response != "create" {
            return;
        }
        let title = name.text().trim().to_string();
        let title = if title.is_empty() { "Untitled Project".to_string() } else { title };
        let (rw, rh) = SHAPES[shape.selected() as usize].1;
        let short = SIZES[size.selected() as usize].1;
        // The chosen size is the short side; the long side follows the shape.
        let (width, height) = if rw >= rh { (short * rw / rh, short) } else { (short, short * rh / rw) };
        let dir = glib::user_special_dir(glib::UserDirectory::Videos).unwrap_or_else(glib::home_dir).join("Tempo");
        let safe: String = title.chars().map(|c| if c == '/' || c == '\0' { '_' } else { c }).collect();
        // Never overwrite an existing project of the same name.
        let mut path = dir.join(format!("{safe}.tempo"));
        let mut n = 2;
        while path.exists() {
            path = dir.join(format!("{safe} {n}.tempo"));
            n += 1;
        }
        on_create(NewProject { name: title, width, height, fps: RATES[rate.selected() as usize].1, path });
    });
    dialog.present(Some(parent));
}

const MARKER_COLORS: [(&str, MarkerColor); 6] = [
    ("Blue", MarkerColor::Blue),
    ("Green", MarkerColor::Green),
    ("Yellow", MarkerColor::Yellow),
    ("Red", MarkerColor::Red),
    ("Orange", MarkerColor::Orange),
    ("Purple", MarkerColor::Purple),
];

/// Edit a marker's name, colour and note.
pub fn edit_marker(parent: &impl IsA<gtk::Widget>, state: &Rc<AppState>, marker: Marker) {
    let name = gtk::Entry::builder().text(&marker.name).activates_default(true).build();
    let note = gtk::Entry::builder().text(&marker.note).activates_default(true).build();
    let color = gtk::DropDown::from_strings(&MARKER_COLORS.map(|c| c.0));
    color.set_selected(MARKER_COLORS.iter().position(|c| c.1 == marker.color).unwrap_or(0) as u32);

    let grid = form_grid();
    form_row(&grid, 0, "Name", &name);
    form_row(&grid, 1, "Colour", &color);
    form_row(&grid, 2, "Note", &note);
    let hint = label("Named markers become chapters when you export.", &["tempo-dim", "tempo-small"]);
    hint.set_xalign(0.0);
    grid.attach(&hint, 1, 3, 1, 1);

    let dialog = adw::AlertDialog::builder().heading("Marker").extra_child(&grid).build();
    dialog.add_responses(&[("remove", "Remove Marker"), ("cancel", "Cancel"), ("done", "Done")]);
    dialog.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("done"));
    dialog.set_close_response("cancel");
    let state = state.clone();
    dialog.connect_response(None, move |_, response| match response {
        "remove" => actions::delete_marker(&state, marker.id),
        "done" => {
            let mut edited = marker.clone();
            edited.name = name.text().trim().to_string();
            edited.note = note.text().trim().to_string();
            edited.color = MARKER_COLORS[color.selected() as usize].1;
            if edited != marker {
                actions::update_marker(&state, edited);
            }
        }
        _ => {}
    });
    dialog.present(Some(parent));
}

/// Ask a yes/no question. `on_yes` runs when the user confirms.
pub fn confirm(parent: &impl IsA<gtk::Widget>, heading: &str, body: &str, yes: &str, no: &str, on_answer: impl Fn(bool) + 'static) {
    let dialog = adw::AlertDialog::builder().heading(heading).body(body).build();
    dialog.add_responses(&[("no", no), ("yes", yes)]);
    dialog.set_default_response(Some("yes"));
    dialog.set_close_response("no");
    dialog.connect_response(None, move |_, response| on_answer(response == "yes"));
    dialog.present(Some(parent));
}

/// The shortcut list, built from the same table that registers the keys.
pub fn shortcuts(parent: &impl IsA<gtk::Widget>) {
    let list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).margin_end(12).build();
    let mut group = String::new();
    for bind in keybinds::load() {
        if bind.group != group {
            group = bind.group.clone();
            let heading = label(&group, &["tempo-heading", "tempo-bright"]);
            heading.set_xalign(0.0);
            heading.set_margin_top(12);
            list.append(&heading);
        }
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let name = label(&bind.label, &[]);
        name.set_xalign(0.0);
        name.set_hexpand(true);
        row.append(&name);
        let keys: Vec<String> = bind.keys.iter().map(|k| keybinds::display(k)).collect();
        row.append(&label(&keys.join("  or  "), &["tempo-dim", "tempo-timecode"]));
        list.append(&row);
    }
    let scrolled = gtk::ScrolledWindow::builder()
        .child(&list)
        .min_content_height(420)
        .min_content_width(440)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    let dialog = adw::AlertDialog::builder()
        .heading("Keyboard Shortcuts")
        .body("The same keys as DaVinci Resolve, except the group marked Tempo only.")
        .extra_child(&scrolled)
        .build();
    dialog.add_responses(&[("close", "Close")]);
    dialog.present(Some(parent));
}

/// Offer to find media files that have moved. The folder search runs on a worker.
pub fn relink_missing(parent: &impl IsA<gtk::Window>, state: &Rc<AppState>) {
    let missing: Vec<String> = state
        .with_project(|p| {
            let mut names: Vec<String> = p
                .sources
                .values()
                .filter(|s| s.is_missing)
                .filter_map(|s| s.path.file_name().map(|n| n.to_string_lossy().into_owned()))
                .collect();
            names.sort();
            names
        })
        .unwrap_or_default();
    if missing.is_empty() {
        return;
    }
    let shown: Vec<&str> = missing.iter().take(8).map(String::as_str).collect();
    let more = if missing.len() > shown.len() { format!("\n… and {} more", missing.len() - shown.len()) } else { String::new() };
    let dialog = adw::AlertDialog::builder()
        .heading("Some media files were not found")
        .body(format!("{}{more}\n\nIf you moved them, pick the folder they are in now.", shown.join("\n")))
        .build();
    dialog.add_responses(&[("skip", "Keep Offline"), ("locate", "Locate Folder…")]);
    dialog.set_default_response(Some("locate"));
    dialog.set_close_response("skip");
    let state = state.clone();
    let window: gtk::Window = parent.as_ref().clone();
    let present_on = window.clone();
    dialog.connect_response(None, move |_, response| {
        if response != "locate" {
            return;
        }
        let state = state.clone();
        let chooser = gtk::FileDialog::builder().title("Folder with the Missing Files").modal(true).build();
        chooser.select_folder(Some(&window), gtk::gio::Cancellable::NONE, move |result| {
            let Some(folder) = result.ok().and_then(|f| f.path()) else { return };
            let wanted: Vec<(uuid::Uuid, std::ffi::OsString)> = state
                .with_project(|p| {
                    p.sources.values().filter(|s| s.is_missing).filter_map(|s| s.path.file_name().map(|n| (s.id, n.to_os_string()))).collect()
                })
                .unwrap_or_default();
            let total = wanted.len();
            let state = state.clone();
            glib::MainContext::default().spawn_local(async move {
                let found = gtk::gio::spawn_blocking(move || find_files(&folder, &wanted)).await.unwrap_or_default();
                let count = found.len();
                if let Some(project) = state.project.borrow_mut().as_mut() {
                    for (id, path) in found {
                        if let Some(source) = project.sources.get_mut(&id) {
                            source.path = path;
                            source.is_missing = false;
                        }
                    }
                }
                if count > 0 {
                    state.sync_player();
                    state.set_dirty(true);
                    state.emit(crate::state::Change::Media);
                    state.emit(crate::state::Change::Timeline);
                }
                state.message(format!("Found {count} of {total} missing files"));
            });
        });
    });
    dialog.present(Some(&present_on));
}

/// Look for files by name under `folder`, a few levels deep.
fn find_files(folder: &std::path::Path, wanted: &[(uuid::Uuid, std::ffi::OsString)]) -> Vec<(uuid::Uuid, PathBuf)> {
    const MAX_DEPTH: usize = 6;
    const MAX_ENTRIES: usize = 100_000;
    let mut found = Vec::new();
    let mut stack = vec![(folder.to_path_buf(), 0usize)];
    let mut seen = 0usize;
    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            seen += 1;
            if seen > MAX_ENTRIES || found.len() == wanted.len() {
                return found;
            }
            let path = entry.path();
            if path.is_dir() {
                if depth < MAX_DEPTH {
                    stack.push((path, depth + 1));
                }
            } else if let Some((id, _)) = wanted.iter().find(|(id, name)| *name == entry.file_name() && !found.iter().any(|(f, _)| f == id)) {
                found.push((*id, path));
            }
        }
    }
    found
}

/// Preferences. Each change is applied and saved at once.
pub fn preferences(parent: &impl IsA<gtk::Widget>, state: &Rc<AppState>) {
    use crate::settings::{cache_dir, cache_size, Settings};
    const AUTOSAVE: [(&str, u32); 4] = [("Every minute", 60), ("Every 2 minutes", 120), ("Every 5 minutes", 300), ("Every 10 minutes", 600)];
    const QUALITY: [(&str, u32); 3] = [("Full", 0), ("Half", 540), ("Quarter", 270)];
    let current = state.settings.borrow().clone();

    // Apply a change to the settings, then save them on a worker.
    let change = {
        let state = state.clone();
        move |apply: &dyn Fn(&mut Settings)| {
            apply(&mut state.settings.borrow_mut());
            let snapshot = state.settings.borrow().clone();
            state.player.set_max_height(snapshot.playback_height);
            // Switching proxies on starts work on footage that is waiting for one.
            state.emit(crate::state::Change::Media);
            glib::MainContext::default().spawn_local(async move {
                if let Ok(Err(e)) = gtk::gio::spawn_blocking(move || snapshot.save()).await {
                    tracing::warn!("could not save settings: {e}");
                }
            });
        }
    };

    let general = adw::PreferencesGroup::builder().title("General").build();
    let autosave = adw::ComboRow::builder().title("Autosave").model(&gtk::StringList::new(&AUTOSAVE.map(|a| a.0))).build();
    autosave.set_selected(AUTOSAVE.iter().position(|a| a.1 == current.autosave_seconds).unwrap_or(1) as u32);
    let c = change.clone();
    autosave.connect_selected_notify(move |row| c(&|s| s.autosave_seconds = AUTOSAVE[row.selected() as usize].1));
    general.add(&autosave);

    let playback = adw::PreferencesGroup::builder()
        .title("Playback")
        .description("Lower quality and proxies make playback smoother on a slow computer. Export always uses full quality.")
        .build();
    let quality = adw::ComboRow::builder().title("Playback quality").model(&gtk::StringList::new(&QUALITY.map(|q| q.0))).build();
    quality.set_selected(QUALITY.iter().position(|q| q.1 == current.playback_height).unwrap_or(1) as u32);
    let c = change.clone();
    quality.connect_selected_notify(move |row| c(&|s| s.playback_height = QUALITY[row.selected() as usize].1));
    playback.add(&quality);
    let proxies = adw::SwitchRow::builder()
        .title("Make proxies automatically")
        .subtitle("Small copies of heavy footage, made in the background")
        .active(current.auto_proxies)
        .build();
    let c = change.clone();
    proxies.connect_active_notify(move |row| c(&|s| s.auto_proxies = row.is_active()));
    playback.add(&proxies);
    let hardware = adw::SwitchRow::builder()
        .title("Hardware decoding (experimental)")
        .subtitle("Let the graphics chip decode video when it can. Takes effect for files opened after the change")
        .active(current.hardware_decode)
        .build();
    let c = change.clone();
    hardware.connect_active_notify(move |row| {
        c(&|s| s.hardware_decode = row.is_active());
        tempo_media::set_hardware_decode(row.is_active());
    });
    playback.add(&hardware);

    let storage = adw::PreferencesGroup::builder().title("Storage").build();
    let cache = adw::ActionRow::builder().title("Cache").subtitle("Waveforms and proxies. Tempo makes them again when needed.").build();
    let clear = gtk::Button::builder().label("Clear").valign(gtk::Align::Center).css_classes(["pill-outline"]).build();
    cache.add_suffix(&clear);
    storage.add(&cache);
    let show_size = {
        let cache = cache.clone();
        move || {
            let cache = cache.clone();
            glib::MainContext::default().spawn_local(async move {
                let bytes = gtk::gio::spawn_blocking(cache_size).await.unwrap_or(0);
                cache.set_title(&format!("Cache — {}", glib::format_size(bytes)));
            });
        }
    };
    show_size();
    let state_for_clear = state.clone();
    clear.connect_clicked(move |_| {
        let show_size = show_size.clone();
        let state = state_for_clear.clone();
        glib::MainContext::default().spawn_local(async move {
            let _ = gtk::gio::spawn_blocking(|| std::fs::remove_dir_all(cache_dir())).await;
            // Proxies are gone: go back to the original files until they are made again.
            if let Some(project) = state.project.borrow_mut().as_mut() {
                for source in project.sources.values_mut() {
                    source.proxy_ready = false;
                    source.proxy_path = None;
                }
            }
            state.waveforms.borrow_mut().clear();
            state.sync_player();
            state.emit(crate::state::Change::Media);
            state.emit(crate::state::Change::Waveform);
            show_size();
        });
    });

    let page = adw::PreferencesPage::new();
    page.add(&general);
    page.add(&playback);
    page.add(&storage);
    let dialog = adw::PreferencesDialog::builder().title("Preferences").build();
    dialog.add(&page);
    dialog.present(Some(parent));
}
