//! Dialogs: decisions only (docs/VISUAL_DESIGN.md §7.3).

use std::path::PathBuf;
use std::rc::Rc;

use gtk4 as gtk;
use gtk4::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;
use tempo_timeline::{ClipType, Command, Marker, MarkerColor, RationalFps, SetClipSpeedCommand, Timeline, MAX_SPEED, MIN_SPEED};

use crate::actions;
use crate::keybinds;
use crate::state::{timecode, AppState};
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
    // An unnamed marker gets "Marker N" as in Resolve, and keeps it when the dialog is closed with Esc.
    let others = state.with_timeline(|t| t.markers.iter().filter(|m| m.id != marker.id).cloned().collect::<Vec<_>>()).unwrap_or_default();
    let default_name = if marker.name.is_empty() { actions::default_marker_name(&others) } else { marker.name.clone() };
    let name = gtk::Entry::builder().text(&default_name).activates_default(true).build();
    let note = gtk::Entry::builder().text(&marker.note).activates_default(true).build();
    let color = gtk::DropDown::from_strings(&MARKER_COLORS.map(|c| c.0));
    color.set_selected(MARKER_COLORS.iter().position(|c| c.1 == marker.color).unwrap_or(0) as u32);

    let fps = state.fps();
    let time = gtk::Entry::builder().text(timecode(marker.position_us, fps)).activates_default(true).build();
    let frame_us = state.frame_us();
    let duration = gtk::SpinButton::with_range(0.0, 1_000_000.0, 1.0);
    duration.set_value((marker.duration_us / frame_us) as f64);
    duration.set_activates_default(true);
    duration.set_tooltip_text(Some("Length in frames; 0 marks a single frame"));
    duration.update_property(&[gtk::accessible::Property::Label("Duration in frames")]);

    let grid = form_grid();
    form_row(&grid, 0, "Time", &time);
    form_row(&grid, 1, "Duration", &duration);
    form_row(&grid, 2, "Name", &name);
    form_row(&grid, 3, "Colour", &color);
    form_row(&grid, 4, "Note", &note);
    let hint = label("Named markers become chapters when you export.", &["tempo-dim", "tempo-small"]);
    hint.set_xalign(0.0);
    grid.attach(&hint, 1, 5, 1, 1);

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
            // A time that does not parse leaves the marker where it was.
            if let Some(us) = parse_timecode(&time.text(), fps) {
                edited.position_us = state.frame_floor(us);
            }
            edited.duration_us = duration.value_as_int().max(0) as i64 * frame_us;
            if edited != marker {
                actions::update_marker(&state, edited);
            }
        }
        // Esc or Cancel: the marker stays, with its default name if it had none.
        _ if marker.name.is_empty() => {
            let mut named = marker.clone();
            named.name = default_name.clone();
            actions::update_marker(&state, named);
        }
        _ => {}
    });
    dialog.present(Some(parent));
}

/// `HH:MM:SS:FF` (also `MM:SS:FF` or `SS:FF`, `;` as well as `:`) to microseconds.
/// None when it is malformed or the frames are past the frame rate.
fn parse_timecode(text: &str, fps: f64) -> Option<i64> {
    let fps_i = fps.round().max(1.0) as i64;
    let mut parts: Vec<i64> = text.trim().split([':', ';']).map(|p| p.trim().parse::<i64>().ok().filter(|n| *n >= 0)).collect::<Option<_>>()?;
    if parts.len() < 2 || parts.len() > 4 {
        return None;
    }
    // Right to left: frames, seconds, minutes, hours.
    parts.reverse();
    let frames = parts[0];
    if frames >= fps_i || parts.get(1).is_some_and(|s| *s >= 60) || parts.get(2).is_some_and(|m| *m >= 60) {
        return None;
    }
    let secs = parts[1] + parts.get(2).unwrap_or(&0) * 60 + parts.get(3).unwrap_or(&0) * 3600;
    Some(((secs * fps_i + frames) as f64 * 1_000_000.0 / fps) as i64)
}

/// Resolve's Create New Timeline (Ctrl+N, or right-click in the Media Pool): a name and
/// how many tracks. The new timeline opens; the others stay in the menu above the viewer.
pub fn new_timeline(parent: &impl IsA<gtk::Widget>, state: &Rc<AppState>) {
    let Some(default) = state.with_project(|p| p.next_timeline_name()) else { return };
    let name = gtk::Entry::builder().text(&default).activates_default(true).build();
    let tracks = |label| {
        let spin = gtk::SpinButton::with_range(1.0, 8.0, 1.0);
        spin.update_property(&[gtk::accessible::Property::Label(label)]);
        spin
    };
    let video = tracks("No. of Video Tracks");
    let audio = tracks("No. of Audio Tracks");
    let grid = form_grid();
    form_row(&grid, 0, "Timeline Name", &name);
    form_row(&grid, 1, "No. of Video Tracks", &video);
    form_row(&grid, 2, "No. of Audio Tracks", &audio);
    // On: an empty timeline. Off: it starts with the clip picked in the Media Pool.
    let empty = gtk::Switch::builder().active(true).halign(gtk::Align::Start).valign(gtk::Align::Center).build();
    empty.set_tooltip_text(Some("Turn off to start the timeline with the clip selected in the Media Pool"));
    form_row(&grid, 3, "Empty Timeline", &empty);

    let dialog = adw::AlertDialog::builder().heading("Create New Timeline").extra_child(&grid).build();
    dialog.add_responses(&[("cancel", "Cancel"), ("create", "Create")]);
    dialog.set_response_appearance("create", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("create"));
    dialog.set_close_response("cancel");
    let taken = state.with_project(|p| p.timeline_list()).unwrap_or_default();
    // Weak: the dialog owns the entry, so a strong ref here would keep both alive forever.
    let d = dialog.downgrade();
    name.connect_changed(move |e| {
        let typed = e.text().trim().to_lowercase();
        let free = !typed.is_empty() && !taken.iter().any(|(_, n)| n.to_lowercase() == typed);
        if let Some(d) = d.upgrade() {
            d.set_response_enabled("create", free);
        }
    });
    let state = state.clone();
    let field = name.clone();
    dialog.connect_response(Some("create"), move |_, _| {
        let typed = field.text().trim().to_string();
        let title = if typed.is_empty() { default.clone() } else { typed };
        state.add_timeline(Timeline::with_tracks(title, video.value_as_int() as u32, audio.value_as_int() as u32));
        if !empty.is_active() {
            match state.media_selection.get() {
                Some(source) => actions::place_source(&state, source, actions::Place::Overwrite, Some(0), None),
                None => state.message("No clip is selected in the Media Pool, so the timeline is empty."),
            }
        }
    });
    dialog.present(Some(parent));
    name.grab_focus();
}

/// Resolve's Change Clip Speed (R): a speed in percent, and Ripple Sequence (on by
/// default) to let the clip change length and move the clips after it.
pub fn change_clip_speed(parent: &impl IsA<gtk::Widget>, state: &Rc<AppState>) {
    let Some(clip) = state.selected_clip() else {
        state.message("Select a clip first, then press R to change its speed.");
        return;
    };
    if matches!(clip.clip_type, ClipType::Title | ClipType::Image) {
        state.message("Only video and sound clips have a speed.");
        return;
    }
    let Some((fps, limits)) = state.with_project(|p| {
        let limits = p.sources.iter().filter(|(_, s)| s.duration_us > 0).map(|(id, s)| (*id, s.duration_us)).collect::<std::collections::HashMap<_, _>>();
        (p.fps, limits)
    }) else {
        return;
    };
    let percent = gtk::SpinButton::with_range(MIN_SPEED * 100.0, MAX_SPEED * 100.0, 5.0);
    percent.set_value((clip.properties.speed * 100.0).round());
    percent.set_activates_default(true);
    let ripple = gtk::Switch::builder().active(true).halign(gtk::Align::Start).valign(gtk::Align::Center).build();
    let length = label("", &["tempo-dim"]);
    length.set_xalign(0.0);

    let grid = form_grid();
    form_row(&grid, 0, "Speed %", &percent);
    form_row(&grid, 1, "Ripple Sequence", &ripple);
    form_row(&grid, 2, "New length", &length);
    let command = {
        let (limits, id) = (limits.clone(), clip.id);
        let (percent, ripple) = (percent.clone(), ripple.clone());
        move || SetClipSpeedCommand::new(id, percent.value() / 100.0, ripple.is_active(), fps, limits.clone())
    };
    // The preview runs the real change on a copy, so it shows exactly what Change will do.
    let preview = {
        let (state, length, command) = (state.clone(), length.clone(), command.clone());
        move || {
            let result = state.with_timeline(|t| {
                let mut copy = t.clone();
                command().execute(&mut copy).map(|()| copy.find_clip(clip.id).map(|(_, c)| c.duration_us()).unwrap_or(0))
            });
            length.set_text(&match result {
                Some(Ok(us)) => crate::state::timecode(us, fps.to_f64()),
                Some(Err(e)) => e.to_string(),
                None => String::new(),
            });
        }
    };
    preview();
    let p = preview.clone();
    percent.connect_value_changed(move |_| p());
    let p = preview.clone();
    ripple.connect_active_notify(move |_| p());

    let dialog = adw::AlertDialog::builder().heading("Change Clip Speed").extra_child(&grid).build();
    dialog.add_responses(&[("cancel", "Cancel"), ("change", "Change")]);
    dialog.set_response_appearance("change", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("change"));
    dialog.set_close_response("cancel");
    let state = state.clone();
    dialog.connect_response(Some("change"), move |_, _| {
        state.execute(Box::new(command()));
    });
    dialog.present(Some(parent));
}

/// Ask for the details of an upload, then run the plugin's uploader on a worker.
/// `draft` holds the starting values; its token is ignored.
pub fn upload(parent: &impl IsA<gtk::Widget>, state: &Rc<AppState>, uploader: crate::plugins::Uploader, draft: tempo_plugin::UploadInput) {
    let title_entry = gtk::Entry::builder().text(draft.title.as_str()).build();
    let text = gtk::TextView::builder().wrap_mode(gtk::WrapMode::WordChar).top_margin(6).bottom_margin(6).left_margin(8).right_margin(8).build();
    text.buffer().set_text(&draft.description);
    let description_box = gtk::ScrolledWindow::builder().child(&text).min_content_height(110).min_content_width(320).css_classes(["card"]).build();
    let privacy = gtk::DropDown::from_strings(&["Private", "Unlisted", "Public"]);
    privacy.set_selected(tempo_plugin::PRIVACY.iter().position(|p| *p == draft.privacy).unwrap_or(0) as u32);
    let category = gtk::DropDown::from_strings(&tempo_plugin::CATEGORIES);
    category.set_selected(tempo_plugin::CATEGORIES.iter().position(|c| *c == draft.category).unwrap_or(0) as u32);
    let thumb_entry = gtk::Entry::builder().hexpand(true).placeholder_text("No thumbnail").build();
    if let Some(t) = &draft.thumbnail {
        thumb_entry.set_text(&t.to_string_lossy());
    }
    let thumb_pick = gtk::Button::with_label("…");
    thumb_pick.set_tooltip_text(Some("Choose a thumbnail image (jpg or png)"));
    let entry = thumb_entry.clone();
    thumb_pick.connect_clicked(move |b| {
        let filter = gtk::FileFilter::new();
        filter.set_name(Some("Images (jpg, png)"));
        filter.add_mime_type("image/jpeg");
        filter.add_mime_type("image/png");
        let dialog = gtk::FileDialog::builder().title("Thumbnail").modal(true).default_filter(&filter).build();
        let entry = entry.clone();
        dialog.open(b.root().and_downcast::<gtk::Window>().as_ref(), gio::Cancellable::NONE, move |result| {
            if let Some(path) = result.ok().and_then(|f| f.path()) {
                entry.set_text(&path.to_string_lossy());
            }
        });
    });
    let thumb_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    thumb_row.append(&thumb_entry);
    thumb_row.append(&thumb_pick);
    let token = gtk::PasswordEntry::builder().show_peek_icon(true).build();

    let grid = form_grid();
    form_row(&grid, 0, "Title", &title_entry);
    form_row(&grid, 1, "Description", &description_box);
    form_row(&grid, 2, "Thumbnail", &thumb_row);
    form_row(&grid, 3, "Privacy", &privacy);
    form_row(&grid, 4, "Category", &category);
    form_row(&grid, 5, "Access token", &token);
    let hint = label(&format!("The token is kept in your keyring and given only to {}.", uploader.plugin.name), &["tempo-dim", "tempo-small"]);
    hint.set_xalign(0.0);
    grid.attach(&hint, 1, 6, 1, 1);
    let file = draft.file;

    // Fill in the token saved last time.
    let id = uploader.plugin.id.clone();
    let field = token.clone();
    glib::MainContext::default().spawn_local(async move {
        if let Ok(Some(saved)) = gio::spawn_blocking(move || crate::plugins::saved_token(&id)).await {
            if field.text().is_empty() {
                field.set_text(&saved);
            }
        }
    });

    let dialog = adw::AlertDialog::builder().heading(format!("Upload to {}", uploader.name)).extra_child(&grid).build();
    dialog.add_responses(&[("cancel", "Cancel"), ("upload", "Upload")]);
    dialog.set_response_appearance("upload", adw::ResponseAppearance::Suggested);
    dialog.set_close_response("cancel");
    let state = state.clone();
    let parent_widget = parent.clone().upcast::<gtk::Widget>();
    dialog.connect_response(Some("upload"), move |_, _| {
        let buffer = text.buffer();
        let input = tempo_plugin::UploadInput {
            file: file.clone(),
            title: title_entry.text().trim().to_string(),
            description: buffer.text(&buffer.start_iter(), &buffer.end_iter(), false).to_string(),
            privacy: tempo_plugin::PRIVACY.get(privacy.selected() as usize).unwrap_or(&"private").to_string(),
            category: tempo_plugin::CATEGORIES.get(category.selected() as usize).unwrap_or(&"Film & Animation").to_string(),
            thumbnail: Some(thumb_entry.text().trim().to_string()).filter(|t| !t.is_empty()).map(std::path::PathBuf::from),
            token: token.text().to_string(),
        };
        let (state, uploader, parent) = (state.clone(), uploader.clone(), parent_widget.clone());
        state.message(format!("Uploading to {}…", uploader.name));
        glib::MainContext::default().spawn_local(async move {
            let plugin = uploader.plugin.clone();
            let function = uploader.function.clone();
            let result = gio::spawn_blocking(move || {
                if !input.token.is_empty() {
                    crate::plugins::save_token(&plugin.id, &input.token);
                }
                tempo_plugin::run_upload(&plugin, &function, input)
            })
            .await;
            match result {
                Ok(Ok(output)) => {
                    for message in output.messages.into_iter().take(3) {
                        state.message(message);
                    }
                    match output.url {
                        Some(url) => uploaded(&parent, &uploader.name, url),
                        None => state.message(format!("Uploaded to {}", uploader.name)),
                    }
                }
                Ok(Err(e)) => state.message(format!("Upload to {} failed: {e}", uploader.name)),
                Err(_) => state.message(format!("Upload to {} stopped unexpectedly.", uploader.name)),
            }
        });
    });
    dialog.present(Some(parent));
}

/// Show where the upload ended up.
fn uploaded(parent: &gtk::Widget, target: &str, url: String) {
    let dialog = adw::AlertDialog::builder().heading(format!("Uploaded to {target}")).body(&url).build();
    dialog.add_responses(&[("close", "Close"), ("copy", "Copy Link"), ("open", "Open")]);
    dialog.set_default_response(Some("open"));
    dialog.set_close_response("close");
    let widget = parent.clone();
    dialog.connect_response(None, move |_, response| match response {
        "copy" => widget.clipboard().set_text(&url),
        "open" => gtk::UriLauncher::new(&url).launch(widget.root().and_downcast::<gtk::Window>().as_ref(), gio::Cancellable::NONE, |_| {}),
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
    let st = state.clone();
    quality.connect_selected_notify(move |row| st.set_playback_height(QUALITY[row.selected() as usize].1));
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
    let (can_decode, can_encode) = state.hardware.get();
    if !can_decode {
        hardware.set_sensitive(false);
        hardware.set_subtitle("Not available: the graphics driver can't decode video on this computer");
    }
    playback.add(&hardware);
    let encode = adw::SwitchRow::builder()
        .title("Hardware encoding for export (experimental)")
        .subtitle("Try the graphics chip first; if it cannot be used, export continues the normal way")
        .active(current.hardware_encode)
        .build();
    let c = change.clone();
    encode.connect_active_notify(move |row| c(&|s| s.hardware_encode = row.is_active()));
    if !can_encode {
        encode.set_sensitive(false);
        encode.set_subtitle("Not available: the graphics driver can't encode video on this computer");
    }
    playback.add(&encode);

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
            // Saved readiness must change too, or the next launch trusts files that are gone.
            state.set_dirty(true);
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

/// The Plugins dialog: what is installed, what each may do, and its on/off switch.
pub fn plugins(parent: &impl IsA<gtk::Window>, plugins: &Rc<crate::plugins::Plugins>) {
    let group = adw::PreferencesGroup::builder()
        .title("Installed")
        .description("A plugin can only do what is listed under its name. Plugins run in a sandbox: they cannot read your files, and reach only the sites listed.")
        .build();
    for plugin in plugins.list.borrow().iter() {
        let access = match plugin.timeline {
            tempo_plugin::TimelineAccess::Write => "May change the timeline",
            tempo_plugin::TimelineAccess::Read => "May read the timeline",
            tempo_plugin::TimelineAccess::None => "No access to the timeline",
        };
        let access = if plugin.network.is_empty() { access.to_string() } else { format!("{access} · May contact {}", plugin.network.join(", ")) };
        let origin = if plugin.built_in { "Comes with Tempo" } else { "Installed by you" };
        let row = adw::SwitchRow::builder()
            .title(format!("{} {}", plugin.name, plugin.version))
            .subtitle(format!("{}\n{access} · {origin}", plugin.description))
            .active(plugins.is_enabled(&plugin.id))
            .build();
        let p = plugins.clone();
        let id = plugin.id.clone();
        row.connect_active_notify(move |r| p.set_enabled(&id, r.is_active()));
        group.add(&row);
    }

    let install_group = adw::PreferencesGroup::builder()
        .title("Add a plugin")
        .description("Pick the folder that holds the plugin's plugin.toml and main.lua.")
        .build();
    let install_row = adw::ActionRow::builder().title("Install from Folder…").activatable(true).build();
    install_group.add(&install_row);

    let page = adw::PreferencesPage::new();
    page.add(&group);
    page.add(&install_group);
    let dialog = adw::PreferencesDialog::builder().title("Plugins").build();
    dialog.add(&page);

    let p = plugins.clone();
    let window: gtk::Window = parent.as_ref().clone();
    let d = dialog.clone();
    install_row.connect_activated(move |_| {
        let p = p.clone();
        let d = d.clone();
        let chooser = gtk::FileDialog::builder().title("Plugin Folder").modal(true).build();
        chooser.select_folder(Some(&window), gtk::gio::Cancellable::NONE, move |result| {
            if let Some(folder) = result.ok().and_then(|f| f.path()) {
                p.install(folder);
                // The list has changed; the dialog is reopened from the menu to show it.
                d.close();
            }
        });
    });
    dialog.present(Some(parent.as_ref()));
}

#[cfg(test)]
mod marker_time_tests {
    use super::parse_timecode;

    #[test]
    fn timecode_text_reads_back() {
        assert_eq!(parse_timecode("00:00:01:00", 24.0), Some(1_000_000));
        assert_eq!(parse_timecode("01:00:00:00", 25.0), Some(3_600_000_000));
        assert_eq!(parse_timecode("1:12", 24.0), Some(1_500_000));
        assert_eq!(parse_timecode("00:00:00;05", 25.0), Some(200_000));
    }

    #[test]
    fn bad_timecode_is_refused() {
        assert_eq!(parse_timecode("abc", 24.0), None);
        assert_eq!(parse_timecode("00:00:00:24", 24.0), None);
        assert_eq!(parse_timecode("00:75:00:00", 24.0), None);
        assert_eq!(parse_timecode("-1:00", 24.0), None);
        assert_eq!(parse_timecode("5", 24.0), None);
    }
}
