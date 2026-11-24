//! Dialogs: decisions only (docs/VISUAL_DESIGN.md §7.3).

use std::path::PathBuf;
use std::rc::Rc;

use gtk4 as gtk;
use gtk4::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;
use tempo_timeline::{
    scale_us, AudioChannelMap, AudioTrackType, Clip, ClipType, Command, CompositeCommand,
    EditClipCommand, Marker, MarkerColor, RationalFps, SetClipSpeedCommand, Timeline, MAX_SPEED,
    MIN_SPEED,
};
use uuid::Uuid;

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
const RATES: [(&str, RationalFps); 8] = [
    ("23.976", RationalFps::FPS_23_976),
    ("24", RationalFps::FPS_24),
    ("25", RationalFps::FPS_25),
    ("29.97", RationalFps::FPS_29_97),
    ("30", RationalFps::FPS_30),
    ("50", RationalFps::FPS_50),
    ("59.94", RationalFps::FPS_59_94),
    ("60", RationalFps::FPS_60),
];

/// Index of `fps` in `RATES`, or of 30 when the rate is not listed.
fn rate_index(fps: RationalFps) -> u32 {
    RATES.iter().position(|r| r.1 == fps).or_else(|| RATES.iter().position(|r| r.1 == RationalFps::FPS_30)).unwrap_or(0) as u32
}

/// The timeline's own rate: None when it is the project's.
fn timeline_fps_choice(chosen: RationalFps, project: RationalFps) -> Option<RationalFps> {
    (chosen != project).then_some(chosen)
}

/// One frame in microseconds, computed as `parse_timecode` does for 00:00:00:01.
fn one_frame_us(fps: f64) -> i64 {
    (1_000_000.0 / fps) as i64
}

/// The frame a time is in, with the same rounding as `AppState::frame_floor`
/// (a time within a tenth of a frame of the next frame counts as that frame).
fn frame_count(us: i64, fps: RationalFps) -> i64 {
    let (num, den) = (fps.num.max(1) as i128, fps.den.max(1) as i128);
    ((us.max(0) as i128 * num * 10 + den * 1_000_000) / (den * 10_000_000)) as i64
}

/// The rate a new timeline will have: the project's, or the dropdown's when Use Project Settings is off.
fn chosen_timeline_fps(use_project: bool, selected: RationalFps, project: RationalFps) -> RationalFps {
    if use_project { project } else { selected }
}

/// Resolve shows a point marker's duration as one frame.
fn shown_marker_duration_us(duration_us: i64, fps: f64) -> i64 {
    duration_us.max(one_frame_us(fps))
}

/// One frame or less (counted in whole frames, as `frame_floor` does) stays a point
/// marker, stored as 0; longer is stored as `floored_us`.
fn stored_marker_duration_us(typed_us: i64, floored_us: i64, fps: RationalFps) -> i64 {
    if frame_count(typed_us, fps) <= 1 { 0 } else { floored_us }
}

fn default_project_dir() -> PathBuf {
    glib::user_special_dir(glib::UserDirectory::Videos)
        .unwrap_or_else(|| glib::home_dir().join("Videos"))
        .join("Tempo")
}

fn project_has_clips(project: &tempo_timeline::Project) -> bool {
    std::iter::once(&project.timeline)
        .chain(&project.timelines)
        .any(|t| t.tracks.iter().any(|tr| !tr.clips.is_empty()))
}

fn shape_size_match(width: u32, height: u32) -> Option<(usize, usize)> {
    for (si, &(_, (rw, rh))) in SHAPES.iter().enumerate() {
        for (zi, &(_, short)) in SIZES.iter().enumerate() {
            let expected = if rw >= rh { (short * rw / rh, short) } else { (short, short * rh / rw) };
            if expected == (width, height) {
                return Some((si, zi));
            }
        }
    }
    None
}

pub fn new_project(parent: &impl IsA<gtk::Widget>, existing_names: Vec<String>, on_create: impl Fn(NewProject) + 'static) {
    let default_name = crate::project_manager::next_default_project_name(&existing_names);
    let name = gtk::Entry::builder().text(&default_name).activates_default(true).build();
    let shape = gtk::DropDown::from_strings(&SHAPES.map(|s| s.0));
    let size = gtk::DropDown::from_strings(&SIZES.map(|s| s.0));
    size.set_selected(1);
    let rate = gtk::DropDown::from_strings(&RATES.map(|r| r.0));
    rate.set_selected(rate_index(RationalFps::FPS_30));

    let location = gtk::Entry::builder()
        .text(&*default_project_dir().to_string_lossy())
        .hexpand(true)
        .activates_default(true)
        .build();
    let browse = gtk::Button::with_label("Browse…");
    let loc_entry = location.clone();
    browse.connect_clicked(move |b| {
        let chooser = gtk::FileDialog::builder().title("Project Location").modal(true).build();
        let current = PathBuf::from(loc_entry.text().trim());
        if !current.as_os_str().is_empty() {
            chooser.set_initial_folder(Some(&gtk::gio::File::for_path(&current)));
        }
        let entry = loc_entry.clone();
        chooser.select_folder(b.root().and_downcast::<gtk::Window>().as_ref(), gtk::gio::Cancellable::NONE, move |result| {
            if let Some(path) = result.ok().and_then(|f| f.path()) {
                entry.set_text(&path.to_string_lossy());
            }
        });
    });
    let loc_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    loc_row.append(&location);
    loc_row.append(&browse);

    let grid = form_grid();
    form_row(&grid, 0, "Name", &name);
    form_row(&grid, 1, "Shape", &shape);
    form_row(&grid, 2, "Resolution", &size);
    form_row(&grid, 3, "Frame rate", &rate);
    form_row(&grid, 4, "Location", &loc_row);
    grid.set_margin_start(24);
    grid.set_margin_end(24);
    grid.set_margin_top(12);
    grid.set_margin_bottom(12);

    let cancel = gtk::Button::with_label("Cancel");
    let create = gtk::Button::with_label("Create");
    create.add_css_class("suggested-action");
    let spacer = gtk::Box::builder().hexpand(true).build();
    let bar = gtk::Box::builder().spacing(8).margin_start(24).margin_end(24).margin_top(8).margin_bottom(12).build();
    bar.append(&spacer);
    bar.append(&cancel);
    bar.append(&create);

    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&grid));
    view.add_bottom_bar(&bar);
    let dialog = adw::Dialog::builder().title("New Project").child(&view).build();
    dialog.set_default_widget(Some(&create));

    let closer = dialog.downgrade();
    cancel.connect_clicked(move |_| {
        if let Some(d) = closer.upgrade() {
            d.close();
        }
    });

    let dialog_ref = dialog.downgrade();
    let name_ref = name.clone();
    create.connect_clicked(move |_| {
        let typed = name_ref.text().trim().to_string();
        let title = if typed.is_empty() { default_name.clone() } else { typed };
        if crate::project_manager::is_duplicate_name(&existing_names, &title) {
            let alert = adw::AlertDialog::builder()
                .heading("Name Already Exists")
                .body("A project with the same name is already in use.")
                .build();
            alert.add_responses(&[("ok", "OK")]);
            alert.set_default_response(Some("ok"));
            alert.set_close_response("ok");
            let n = name_ref.clone();
            alert.connect_response(None, move |_, _| {
                n.grab_focus();
                n.select_region(0, -1);
            });
            if let Some(d) = dialog_ref.upgrade() {
                alert.present(Some(&d));
            }
            return;
        }
        let (rw, rh) = SHAPES.get(shape.selected() as usize).unwrap_or(&SHAPES[0]).1;
        let short = SIZES.get(size.selected() as usize).unwrap_or(&SIZES[1]).1;
        // The chosen size is the short side; the long side follows the shape.
        let (width, height) = if rw >= rh { (short * rw / rh, short) } else { (short, short * rh / rw) };
        let loc_text = location.text().trim().to_string();
        let dir = if loc_text.is_empty() { default_project_dir() } else { PathBuf::from(loc_text) };
        let safe: String = title.chars().map(|c| if c == '/' || c == '\0' { '_' } else { c }).collect();
        let path = dir.join(format!("{safe}.tempo"));
        let fps = RATES.get(rate.selected() as usize).unwrap_or(&RATES[4]).1;
        if let Some(d) = dialog_ref.upgrade() {
            d.close();
        }
        on_create(NewProject { name: title, width, height, fps, path });
    });

    dialog.present(Some(parent));
    name.grab_focus();
    name.select_region(0, -1);
}

/// Choose a new `.tempo` path for Save Project As.
pub fn save_project_as(
    parent: &impl IsA<gtk::Window>,
    current_path: Option<&std::path::Path>,
    current_name: &str,
    on_choose: impl Fn(PathBuf) + 'static,
) {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("Tempo projects"));
    filter.add_pattern("*.tempo");
    let filters = gtk::gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    let safe_stem: String = current_name.chars().map(|c| if c == '/' || c == '\0' { '_' } else { c }).collect();
    let initial_name = if safe_stem.trim().is_empty() {
        "Untitled Project.tempo".to_string()
    } else {
        format!("{safe_stem}.tempo")
    };
    let mut builder = gtk::FileDialog::builder()
        .title("Save Project As")
        .modal(true)
        .filters(&filters)
        .default_filter(&filter)
        .initial_name(&initial_name);
    if let Some(dir) = current_path.and_then(|p| p.parent()) {
        builder = builder.initial_folder(&gtk::gio::File::for_path(dir));
    }
    let dialog = builder.build();
    dialog.save(Some(parent.as_ref()), gtk::gio::Cancellable::NONE, move |result| {
        if let Some(mut path) = result.ok().and_then(|f| f.path()) {
            if path.extension().is_none() {
                path.set_extension("tempo");
            }
            on_choose(path);
        }
    });
}

/// Project Settings (Shift+9): resolution, frame rate (locked when clips exist), proxy folder.
pub fn project_settings(parent: &impl IsA<gtk::Widget>, state: &Rc<AppState>) {
    let Some((cur_w, cur_h, cur_fps, cur_proxy_dir, has_clips)) =
        state.with_project(|p| (p.width, p.height, p.fps, p.proxy_dir.clone(), project_has_clips(p)))
    else {
        return;
    };

    let shape = gtk::DropDown::from_strings(&["Landscape 16:9", "Vertical 9:16", "Square 1:1", "Custom"]);
    let size = gtk::DropDown::from_strings(&SIZES.map(|s| s.0));
    let width = gtk::SpinButton::with_range(16.0, 16384.0, 2.0);
    width.set_value(cur_w as f64);
    let height = gtk::SpinButton::with_range(16.0, 16384.0, 2.0);
    height.set_value(cur_h as f64);

    if let Some((si, zi)) = shape_size_match(cur_w, cur_h) {
        shape.set_selected(si as u32);
        size.set_selected(zi as u32);
    } else {
        shape.set_selected(3);
        size.set_selected(1);
        size.set_sensitive(false);
    }

    let syncing = Rc::new(std::cell::Cell::new(false));
    let apply_preset = {
        let (shape, size, width, height, syncing) = (shape.clone(), size.clone(), width.clone(), height.clone(), syncing.clone());
        move || {
            if syncing.get() {
                return;
            }
            let si = shape.selected() as usize;
            let custom = si >= SHAPES.len();
            size.set_sensitive(!custom);
            if custom {
                return;
            }
            let (rw, rh) = SHAPES[si].1;
            let short = SIZES.get(size.selected() as usize).unwrap_or(&SIZES[1]).1;
            let (w, h) = if rw >= rh { (short * rw / rh, short) } else { (short, short * rh / rw) };
            syncing.set(true);
            width.set_value(w as f64);
            height.set_value(h as f64);
            syncing.set(false);
        }
    };
    let ap = apply_preset.clone();
    shape.connect_selected_notify(move |_| ap());
    let ap = apply_preset.clone();
    size.connect_selected_notify(move |_| ap());

    let on_dim = {
        let (shape, size, width, height, syncing) = (shape.clone(), size.clone(), width.clone(), height.clone(), syncing.clone());
        move || {
            if syncing.get() {
                return;
            }
            syncing.set(true);
            let (w, h) = (width.value_as_int().max(16) as u32, height.value_as_int().max(16) as u32);
            if let Some((si, zi)) = shape_size_match(w, h) {
                shape.set_selected(si as u32);
                size.set_selected(zi as u32);
                size.set_sensitive(true);
            } else {
                shape.set_selected(3);
                size.set_sensitive(false);
            }
            syncing.set(false);
        }
    };
    let od = on_dim.clone();
    width.connect_value_changed(move |_| od());
    height.connect_value_changed(move |_| on_dim());

    let rate = gtk::DropDown::from_strings(&RATES.map(|r| r.0));
    rate.set_selected(rate_index(cur_fps));
    if has_clips {
        rate.set_sensitive(false);
    }

    let proxy_entry = gtk::Entry::builder()
        .text(&*cur_proxy_dir.to_string_lossy())
        .hexpand(true)
        .activates_default(true)
        .build();
    let browse = gtk::Button::with_label("Browse…");
    let pe = proxy_entry.clone();
    browse.connect_clicked(move |b| {
        let chooser = gtk::FileDialog::builder().title("Proxy Folder").modal(true).build();
        let current = PathBuf::from(pe.text().trim());
        if !current.as_os_str().is_empty() {
            chooser.set_initial_folder(Some(&gtk::gio::File::for_path(&current)));
        }
        let entry = pe.clone();
        chooser.select_folder(b.root().and_downcast::<gtk::Window>().as_ref(), gtk::gio::Cancellable::NONE, move |result| {
            if let Some(path) = result.ok().and_then(|f| f.path()) {
                entry.set_text(&path.to_string_lossy());
            }
        });
    });
    let proxy_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    proxy_row.append(&proxy_entry);
    proxy_row.append(&browse);

    let grid = form_grid();
    let mut row = 0;
    form_row(&grid, row, "Shape", &shape);
    row += 1;
    form_row(&grid, row, "Resolution", &size);
    row += 1;
    form_row(&grid, row, "Width", &width);
    row += 1;
    form_row(&grid, row, "Height", &height);
    row += 1;
    form_row(&grid, row, "Frame rate", &rate);
    row += 1;
    if has_clips {
        let note = label("Cannot be changed once clips are on the timeline", &["tempo-dim", "tempo-small"]);
        note.set_xalign(0.0);
        grid.attach(&note, 1, row, 1, 1);
        row += 1;
    }
    form_row(&grid, row, "Proxy folder", &proxy_row);

    let dialog = adw::AlertDialog::builder().heading("Project Settings").extra_child(&grid).build();
    dialog.add_responses(&[("cancel", "Cancel"), ("apply", "Apply")]);
    dialog.set_response_appearance("apply", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("apply"));
    dialog.set_close_response("cancel");

    let state = state.clone();
    dialog.connect_response(Some("apply"), move |_, _| {
        let w = (width.value_as_int().max(16) as u32) & !1;
        let h = (height.value_as_int().max(16) as u32) & !1;
        let chosen_fps = RATES.get(rate.selected() as usize).unwrap_or(&RATES[4]).1;
        let proxy_text = proxy_entry.text().trim().to_string();
        let updated = if let Some(p) = state.project.borrow_mut().as_mut() {
            let locked = project_has_clips(p);
            let new_proxy = if proxy_text.is_empty() {
                p.proxy_dir.clone()
            } else {
                PathBuf::from(proxy_text)
            };
            let changed = (p.width, p.height) != (w, h) || (!locked && p.fps != chosen_fps) || p.proxy_dir != new_proxy;
            p.width = w;
            p.height = h;
            if !locked {
                p.fps = chosen_fps;
            }
            p.proxy_dir = new_proxy;
            changed
        } else {
            false
        };
        if updated {
            state.sync_player();
            state.set_dirty(true);
            state.emit(crate::state::Change::Project);
        }
    });
    dialog.present(Some(parent));
}

/// Edit a marker's name, colour and note.
pub fn edit_marker(parent: &impl IsA<gtk::Widget>, state: &Rc<AppState>, marker: Marker) {
    // An unnamed marker gets "Marker N" as in Resolve, and keeps it when the dialog is closed with Esc.
    let others = state.with_timeline(|t| t.markers.iter().filter(|m| m.id != marker.id).cloned().collect::<Vec<_>>()).unwrap_or_default();
    let default_name = if marker.name.is_empty() { actions::default_marker_name(&others) } else { marker.name.clone() };
    let name = gtk::Entry::builder().text(&default_name).activates_default(true).build();
    let note = gtk::Entry::builder().text(&marker.note).activates_default(true).build();
    // A legacy Orange marker shows no dot marked until another is picked.
    let selected_color = Rc::new(std::cell::Cell::new(marker.color));
    let color = gtk::FlowBox::builder().selection_mode(gtk::SelectionMode::None).max_children_per_line(8).min_children_per_line(8).column_spacing(2).row_spacing(2).build();
    let mut first: Option<gtk::ToggleButton> = None;
    for c in MarkerColor::RESOLVE {
        let dot = gtk::Label::new(None);
        dot.set_markup(&format!("<span foreground=\"{}\" size=\"x-large\">●</span>", c.hex()));
        let button = gtk::ToggleButton::builder().child(&dot).css_classes(["flat"]).tooltip_text(c.name()).build();
        button.update_property(&[gtk::accessible::Property::Label(c.name())]);
        button.set_focus_on_click(false);
        match &first {
            Some(f) => button.set_group(Some(f)),
            None => first = Some(button.clone()),
        }
        button.set_active(c == marker.color);
        let chosen = selected_color.clone();
        button.connect_toggled(move |b| {
            if b.is_active() {
                chosen.set(c);
            }
        });
        color.append(&button);
    }

    let fps = state.fps();
    let time = gtk::Entry::builder().text(state.position_tc(marker.position_us)).activates_default(true).build();
    let keyword = gtk::Entry::builder().text(&marker.keyword).activates_default(true).build();
    // A length, so no Start Timecode: a point marker reads as one frame, as in Resolve.
    let duration = gtk::Entry::builder().text(timecode(shown_marker_duration_us(marker.duration_us, fps), fps)).activates_default(true).build();
    duration.set_tooltip_text(Some("Length as a timecode; one frame (00:00:00:01) or less marks a single point"));

    let grid = form_grid();
    form_row(&grid, 0, "Time", &time);
    form_row(&grid, 1, "Duration", &duration);
    form_row(&grid, 2, "Name", &name);
    form_row(&grid, 3, "Notes", &note);
    form_row(&grid, 4, "Keyword", &keyword);
    form_row(&grid, 5, "Colour", &color);
    let hint = label("Named markers become chapters when you export.", &["tempo-dim", "tempo-small"]);
    hint.set_xalign(0.0);
    grid.attach(&hint, 1, 6, 1, 1);

    let dialog = adw::AlertDialog::builder().heading("Markers").extra_child(&grid).build();
    dialog.add_responses(&[("remove", "Remove Marker"), ("done", "Done")]);
    dialog.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("done"));
    dialog.set_close_response("close");
    let state = state.clone();
    let name_field = name.clone();
    dialog.connect_response(None, move |_, response| match response {
        "remove" => actions::delete_marker(&state, marker.id),
        "done" => {
            let mut edited = marker.clone();
            edited.name = name_field.text().trim().to_string();
            edited.note = note.text().trim().to_string();
            edited.keyword = keyword.text().trim().to_string();
            edited.color = selected_color.get();
            // A time that does not parse leaves the marker where it was.
            if let Some(us) = typed_position_us(&time.text(), fps, state.start_tc_us()) {
                edited.position_us = state.frame_floor(us);
            }
            // A duration that does not parse stays as it was.
            if let Some(us) = parse_timecode(&duration.text(), fps) {
                let rate = state.with_project(|p| p.timeline_fps()).unwrap_or(RationalFps::FPS_30);
                edited.duration_us = stored_marker_duration_us(us, state.frame_floor(us), rate);
            }
            if edited != marker {
                actions::update_marker(&state, edited);
            }
        }
        // Esc: the marker stays, with its default name if it had none.
        _ if marker.name.is_empty() => {
            let mut named = marker.clone();
            named.name = default_name.clone();
            actions::update_marker(&state, named);
        }
        _ => {}
    });
    dialog.present(Some(parent));
    // Resolve opens with the name selected: type it and press Enter.
    name.grab_focus();
    name.select_region(0, -1);
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

/// A typed timecode (which includes the Start Timecode) as a timeline position.
/// None when it is malformed or lies before the timeline starts.
fn typed_position_us(text: &str, fps: f64, start_tc_us: i64) -> Option<i64> {
    parse_timecode(text, fps).map(|us| us - start_tc_us).filter(|us| *us >= 0)
}

/// Resolve's 01:00:00:00 counted in frames, so it reads that at 23.976/29.97/59.94 too.
fn default_start_tc_us(fps: f64) -> i64 {
    parse_timecode("01:00:00:00", fps).unwrap_or(tempo_timeline::RESOLVE_START_TC_US)
}

/// What the Create New Timeline dialog asks for, as a timeline.
fn timeline_from_form(name: String, video: u32, audio: u32, audio_type: AudioTrackType, start_tc_us: i64, resolution: Option<(u32, u32)>, fps: Option<RationalFps>) -> Timeline {
    let mut timeline = Timeline::with_tracks(name, video, audio);
    timeline.start_tc_us = start_tc_us;
    timeline.resolution = resolution;
    timeline.fps = fps;
    for track in timeline.tracks.iter_mut().filter(|t| t.kind == tempo_timeline::TrackKind::Audio) {
        track.audio_type = audio_type;
    }
    timeline
}

/// Mark In/Out can seed the new timeline only when the Media Pool pick is the clip open in
/// the viewer's source mode and has an In or an Out.
fn marks_available(selected: Option<uuid::Uuid>, source: Option<uuid::Uuid>, src_in: Option<i64>, src_out: Option<i64>) -> bool {
    selected.is_some() && selected == source && (src_in.is_some() || src_out.is_some())
}

/// Resolve's Create New Timeline (Ctrl+N, or right-click in the Media Pool): a name and
/// how many tracks. The new timeline opens; the others stay in the menu above the viewer.
pub fn new_timeline(parent: &impl IsA<gtk::Widget>, state: &Rc<AppState>, bin_id: Option<uuid::Uuid>) {
    let Some(default) = state.with_project(|p| p.next_timeline_name()) else { return };
    let name = gtk::Entry::builder().text(&default).activates_default(true).build();
    let tracks = |label| {
        let spin = gtk::SpinButton::with_range(1.0, 8.0, 1.0);
        spin.update_property(&[gtk::accessible::Property::Label(label)]);
        spin
    };
    let video = tracks("No. of Video Tracks");
    let audio = tracks("No. of Audio Tracks");
    let project_rate = state.with_project(|p| p.fps).unwrap_or(RationalFps::FPS_30);
    let fps = project_rate.to_f64();
    let start = gtk::Entry::builder().text(timecode(default_start_tc_us(fps), fps)).activates_default(true).build();
    let audio_type = gtk::DropDown::from_strings(&["Mono", "Stereo"]);
    audio_type.set_selected(1);
    // Resolve's options are checkboxes whose label toggles them.
    let check_button = |text: &str| gtk::CheckButton::builder().label(text).active(true).halign(gtk::Align::Start).build();
    // On: an empty timeline. Off: it starts with the clip picked in the Media Pool.
    let empty = check_button("Empty Timeline");
    empty.set_tooltip_text(Some("Turn off to start the timeline with the clip selected in the Media Pool"));
    let use_project = check_button("Use Project Settings");
    // Resolve's option is off until asked for, and greyed out unless the picked clip has marks.
    let use_marks = check_button("Use Selected Mark In/Out");
    use_marks.set_active(false);
    let marks = marks_available(state.media_selection.get(), state.source_clip.get(), state.src_in.get(), state.src_out.get());
    use_marks.set_sensitive(marks);
    if !marks {
        use_marks.set_tooltip_text(Some("Set In/Out on the clip in the viewer's source mode and select it in the Media Pool"));
    }
    empty.bind_property("active", &use_marks, "sensitive").transform_to(move |_, empty: bool| Some(marks && !empty)).sync_create().build();
    let (project_w, project_h, project_fps) = state.with_project(|p| (p.width, p.height, p.fps)).unwrap_or((1920, 1080, RationalFps::FPS_30));
    let width = gtk::SpinButton::with_range(16.0, 16384.0, 2.0);
    width.set_value(project_w as f64);
    let height = gtk::SpinButton::with_range(16.0, 16384.0, 2.0);
    height.set_value(project_h as f64);
    let size = form_grid();
    form_row(&size, 0, "Width", &width);
    form_row(&size, 1, "Height", &height);
    let rate = gtk::DropDown::from_strings(&RATES.map(|r| r.0));
    rate.set_selected(rate_index(project_fps));
    form_row(&size, 2, "Timeline Frame Rate", &rate);
    let reveal = gtk::Revealer::builder().child(&size).build();
    use_project.bind_property("active", &reveal, "reveal-child").transform_to(|_, on: bool| Some(!on)).sync_create().build();

    let grid = form_grid();
    form_row(&grid, 0, "Start Timecode", &start);
    form_row(&grid, 1, "Timeline Name", &name);
    form_row(&grid, 2, "No. of Video Tracks", &video);
    form_row(&grid, 3, "No. of Audio Tracks", &audio);
    form_row(&grid, 4, "Audio Track Type", &audio_type);
    grid.attach(&empty, 1, 5, 1, 1);
    grid.attach(&use_marks, 1, 6, 1, 1);
    grid.attach(&reveal, 0, 7, 2, 1);
    grid.set_margin_start(24);
    grid.set_margin_end(24);
    grid.set_margin_top(12);
    grid.set_margin_bottom(12);

    // Resolve's button row: Use Project Settings bottom left, Cancel and Create right.
    let cancel = gtk::Button::with_label("Cancel");
    let create = gtk::Button::with_label("Create");
    create.add_css_class("suggested-action");
    let spacer = gtk::Box::builder().hexpand(true).build();
    let bar = gtk::Box::builder().spacing(8).margin_start(24).margin_end(24).margin_top(8).margin_bottom(12).build();
    bar.append(&use_project);
    bar.append(&spacer);
    bar.append(&cancel);
    bar.append(&create);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&grid));
    view.add_bottom_bar(&bar);
    let dialog = adw::Dialog::builder().title("Create New Timeline").child(&view).build();
    dialog.set_default_widget(Some(&create));
    let closer = dialog.downgrade();
    cancel.connect_clicked(move |_| {
        if let Some(d) = closer.upgrade() {
            d.close();
        }
    });
    let taken = state.with_project(|p| p.timeline_list()).unwrap_or_default();
    // The rate the new timeline gets decides what the Start Timecode means.
    let chosen_fps = {
        let (use_project, rate) = (use_project.clone(), rate.clone());
        move || chosen_timeline_fps(use_project.is_active(), RATES[rate.selected() as usize].1, project_fps).to_f64()
    };
    // The default follows the rate until the user types a different Start Timecode.
    let default_start = Rc::new(std::cell::RefCell::new(start.text().to_string()));
    // Weak: the dialog owns the entry, so a strong ref here would keep both alive forever.
    let check = {
        let (name, start, chosen_fps, create) = (name.clone(), start.clone(), chosen_fps.clone(), create.clone());
        move || {
            let typed = name.text().trim().to_lowercase();
            let free = !typed.is_empty() && !taken.iter().any(|(_, n)| n.to_lowercase() == typed);
            let valid_start = parse_timecode(&start.text(), chosen_fps()).is_some();
            create.set_sensitive(free && valid_start);
        }
    };
    let on_name = check.clone();
    name.connect_changed(move |_| on_name());
    let on_start = check.clone();
    start.connect_changed(move |_| on_start());
    let follow = {
        let (start, chosen_fps, default_start) = (start.clone(), chosen_fps.clone(), default_start.clone());
        move || {
            let rate = chosen_fps();
            if start.text() == default_start.borrow().as_str() {
                let text = timecode(default_start_tc_us(rate), rate);
                *default_start.borrow_mut() = text.clone();
                start.set_text(&text);
            }
            check();
        }
    };
    let on_switch = follow.clone();
    use_project.connect_active_notify(move |_| on_switch());
    rate.connect_selected_notify(move |_| follow());
    let state = state.clone();
    let field = name.clone();
    let closer = dialog.downgrade();
    create.connect_clicked(move |_| {
        let typed = field.text().trim().to_string();
        let title = if typed.is_empty() { default.clone() } else { typed };
        let new_fps = chosen_fps();
        let start_tc_us = parse_timecode(&start.text(), new_fps).unwrap_or_else(|| default_start_tc_us(new_fps));
        let timeline_fps = (!use_project.is_active()).then(|| timeline_fps_choice(RATES[rate.selected() as usize].1, project_fps)).flatten();
        let resolution = (!use_project.is_active()).then(|| (width.value_as_int() as u32, height.value_as_int() as u32));
        let kind = if audio_type.selected() == 0 { AudioTrackType::Mono } else { AudioTrackType::Stereo };
        let mut tl = timeline_from_form(title, video.value_as_int() as u32, audio.value_as_int() as u32, kind, start_tc_us, resolution, timeline_fps);
        tl.bin_id = bin_id;
        state.add_timeline(tl);
        if !empty.is_active() {
            match state.media_selection.get() {
                Some(source) => {
                    actions::place_source(&state, source, actions::Place::Overwrite, Some(0), None, use_marks.is_active());
                }
                None => state.message("No clip is selected in the Media Pool, so the timeline is empty."),
            }
        }
        if let Some(d) = closer.upgrade() {
            d.close();
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
        (p.timeline_fps(), limits)
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

struct ClipAttributesSpec {
    subtitle: String,
    initial_name: String,
    file_name: String,
    has_video: bool,
    video_summary: String,
    native_fps: Option<RationalFps>,
    initial_conform_fps: Option<RationalFps>,
    source_duration_us: i64,
    preview_fps: RationalFps,
    has_audio: bool,
    audio_summary: String,
    initial_channel_map: AudioChannelMap,
}

struct ClipAttributesPatch {
    display_name: String,
    conform_fps: Option<RationalFps>,
    fps_changed: bool,
    channel_map: AudioChannelMap,
    channel_map_changed: bool,
}

fn present_clip_attributes_dialog(
    parent: &impl IsA<gtk::Widget>,
    spec: ClipAttributesSpec,
    on_apply: impl FnOnce(ClipAttributesPatch) + 'static,
) {
    let stack = gtk::Stack::builder().transition_type(gtk::StackTransitionType::Crossfade).vhomogeneous(false).build();
    let switcher = gtk::StackSwitcher::builder().stack(&stack).halign(gtk::Align::Center).build();

    // ---- Video tab
    let video_grid = form_grid();
    let src_vid = label(&spec.video_summary, &[]);
    src_vid.set_xalign(0.0);
    form_row(&video_grid, 0, "Source", &src_vid);

    let native_label = match spec.native_fps {
        Some(f) => format!("Native ({:.2} fps)", f.to_f64()),
        None => "Native".to_string(),
    };
    let mut fps_labels = vec![native_label];
    for (r_label, _) in RATES {
        fps_labels.push(format!("{r_label} fps"));
    }
    let fps_refs: Vec<&str> = fps_labels.iter().map(String::as_str).collect();
    let fps_drop = gtk::DropDown::from_strings(&fps_refs);
    let initial_fps_idx = spec
        .initial_conform_fps
        .and_then(|cf| RATES.iter().position(|(_, r)| r.num == cf.num && r.den == cf.den))
        .map(|i| (i + 1) as u32)
        .unwrap_or(0);
    fps_drop.set_selected(initial_fps_idx);
    fps_drop.set_sensitive(spec.has_video && spec.native_fps.is_some());
    form_row(&video_grid, 1, "Video Frame Rate", &fps_drop);

    let speed_preview = label("", &["tempo-dim"]);
    speed_preview.set_xalign(0.0);
    form_row(&video_grid, 2, "Playback", &speed_preview);

    let update_preview = {
        let (fps_drop, speed_preview) = (fps_drop.clone(), speed_preview.clone());
        let (native_fps, source_dur, preview_fps, has_video) =
            (spec.native_fps, spec.source_duration_us, spec.preview_fps, spec.has_video);
        move || {
            if !has_video || native_fps.is_none() {
                speed_preview.set_text("Video frame rate conform applies to video clips.");
                return;
            }
            let sel = fps_drop.selected() as usize;
            let spd = match (native_fps, sel) {
                (Some(n), idx) if idx > 0 => RATES
                    .get(idx - 1)
                    .map(|(_, t)| (t.to_f64() / n.to_f64().max(1.0)).clamp(MIN_SPEED, MAX_SPEED))
                    .unwrap_or(1.0),
                _ => 1.0,
            };
            let dur = if (spd - 1.0).abs() < 1e-6 {
                source_dur
            } else {
                scale_us(source_dur, 1.0 / spd).max(1)
            };
            speed_preview.set_text(&format!("{spd:.2}× speed · {}", timecode(dur, preview_fps.to_f64())));
        }
    };
    update_preview();
    let up = update_preview.clone();
    fps_drop.connect_selected_notify(move |_| up());
    stack.add_titled(&video_grid, Some("video"), "Video");

    // ---- Audio tab
    let audio_grid = form_grid();
    let src_aud = label(&spec.audio_summary, &[]);
    src_aud.set_xalign(0.0);
    form_row(&audio_grid, 0, "Source", &src_aud);

    let ch_labels: Vec<&str> = AudioChannelMap::ALL.iter().map(|m| m.label()).collect();
    let ch_drop = gtk::DropDown::from_strings(&ch_labels);
    let initial_ch_idx = AudioChannelMap::ALL
        .iter()
        .position(|m| *m == spec.initial_channel_map)
        .unwrap_or(0) as u32;
    ch_drop.set_selected(initial_ch_idx);
    ch_drop.set_sensitive(spec.has_audio);
    form_row(&audio_grid, 1, "Channel Mapping", &ch_drop);

    let audio_hint = label(
        if spec.has_audio {
            "Tip: Pick Channel 1 (Left -> Both) if a mic only plays on one side."
        } else {
            "This clip has no audio stream."
        },
        &["tempo-small", "tempo-dim"],
    );
    audio_hint.set_xalign(0.0);
    audio_hint.set_wrap(true);
    audio_grid.attach(&audio_hint, 1, 2, 1, 1);
    stack.add_titled(&audio_grid, Some("audio"), "Audio");

    // ---- Name tab
    let name_grid = form_grid();
    let name_entry = gtk::Entry::builder().text(&spec.initial_name).activates_default(true).build();
    name_entry.update_property(&[gtk::accessible::Property::Label("Clip Name")]);
    form_row(&name_grid, 0, "Clip Name", &name_entry);
    let file_lbl = label(&spec.file_name, &["tempo-dim"]);
    file_lbl.set_xalign(0.0);
    file_lbl.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    form_row(&name_grid, 1, "Disk File", &file_lbl);
    let disk_note = label("Renaming here changes project display only, not the file on disk.", &["tempo-small", "tempo-dim"]);
    disk_note.set_xalign(0.0);
    disk_note.set_wrap(true);
    name_grid.attach(&disk_note, 1, 2, 1, 1);
    stack.add_titled(&name_grid, Some("name"), "Name");

    let body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(12).width_request(400).build();
    body.append(&switcher);
    body.append(&stack);

    let dialog = adw::AlertDialog::builder()
        .heading("Clip Attributes")
        .body(&spec.subtitle)
        .extra_child(&body)
        .build();
    dialog.add_responses(&[("cancel", "Cancel"), ("apply", "Apply")]);
    dialog.set_response_appearance("apply", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("apply"));
    dialog.set_close_response("cancel");

    let on_apply = std::cell::Cell::new(Some(on_apply));
    dialog.connect_response(Some("apply"), move |_, _| {
        let sel_fps = fps_drop.selected();
        let conform_fps = if sel_fps > 0 {
            RATES.get((sel_fps - 1) as usize).map(|(_, f)| *f)
        } else {
            None
        };
        let sel_ch = ch_drop.selected();
        let channel_map = AudioChannelMap::ALL.get(sel_ch as usize).copied().unwrap_or_default();
        if let Some(cb) = on_apply.take() {
            cb(ClipAttributesPatch {
                display_name: name_entry.text().trim().to_string(),
                conform_fps,
                fps_changed: sel_fps != initial_fps_idx,
                channel_map,
                channel_map_changed: sel_ch != initial_ch_idx,
            });
        }
    });
    dialog.present(Some(parent));
}

/// Resolve-style Clip Attributes for a Media Pool item (Video FPS Conform, Audio Mapping, Clip Name).
pub fn clip_attributes_source(parent: &impl IsA<gtk::Widget>, state: &Rc<AppState>, source_id: Uuid) {
    let Some(spec) = state.with_project(|p| {
        p.sources.get(&source_id).map(|s| {
            let file_name = s.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let has_video = s.media_type == tempo_timeline::MediaType::Video;
            let video_summary = match (s.video_width, s.video_height, s.video_fps) {
                (Some(w), Some(h), Some(fps)) => format!("{w} × {h}, Native {:.2} fps", fps.to_f64()),
                _ if s.media_type == tempo_timeline::MediaType::Image => format!(
                    "{} × {} Still Image",
                    s.video_width.unwrap_or(0),
                    s.video_height.unwrap_or(0)
                ),
                _ => "Audio only".to_string(),
            };
            let has_audio = s.audio_channels.is_some();
            let audio_summary = match (s.audio_channels, s.audio_sample_rate) {
                (Some(ch), Some(sr)) => format!("{ch} ch · {sr} Hz"),
                (Some(ch), None) => format!("{ch} ch"),
                _ => "No audio stream".to_string(),
            };
            ClipAttributesSpec {
                subtitle: format!("Media Pool · {file_name}"),
                initial_name: s.display_name(),
                file_name,
                has_video,
                video_summary,
                native_fps: s.video_fps,
                initial_conform_fps: s.conform_fps,
                source_duration_us: s.duration_us,
                preview_fps: p.timeline_fps(),
                has_audio,
                audio_summary,
                initial_channel_map: s.audio_channel_map,
            }
        })
    }).flatten() else {
        return;
    };

    let state = state.clone();
    present_clip_attributes_dialog(parent, spec, move |patch| {
        let changed = {
            let mut guard = state.project.borrow_mut();
            guard
                .as_mut()
                .and_then(|p| p.sources.get_mut(&source_id))
                .is_some_and(|src| {
                    let disk_name = src.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let next_name = if patch.display_name.is_empty() || patch.display_name == disk_name {
                        None
                    } else {
                        Some(patch.display_name)
                    };
                    if src.display_name == next_name && src.conform_fps == patch.conform_fps && src.audio_channel_map == patch.channel_map {
                        return false;
                    }
                    src.display_name = next_name;
                    src.conform_fps = patch.conform_fps;
                    src.audio_channel_map = patch.channel_map;
                    true
                })
        };
        if changed {
            state.set_dirty(true);
            if state.source_clip.get() == Some(source_id) {
                state.show_source(Some(source_id));
            }
            state.emit(crate::state::Change::Media);
            state.emit(crate::state::Change::Options);
        }
    });
}

/// Resolve-style Clip Attributes for selected Timeline clip(s) (undoable via `CommandLog`).
pub fn clip_attributes_timeline(parent: &impl IsA<gtk::Widget>, state: &Rc<AppState>) {
    let selected = state.selected_ids();
    if selected.is_empty() {
        state.message("Select a clip on the timeline first, then open Clip Attributes…");
        return;
    }

    let Some((clips, spec, timeline_fps, source_limits, lead_fps)) = state.with_project(|p| {
        let mut collected: Vec<Clip> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for id in &selected {
            if let Some((_, clip)) = p.timeline.find_clip(*id) {
                if seen.insert(clip.id) {
                    collected.push(clip.clone());
                }
                if let Some(link_id) = clip.properties.link {
                    for t in &p.timeline.tracks {
                        for c in t.clips.iter().filter(|c| c.properties.link == Some(link_id)) {
                            if seen.insert(c.id) {
                                collected.push(c.clone());
                            }
                        }
                    }
                }
            }
        }
        let primary = collected.first()?.clone();
        let src = p.sources.get(&primary.source_id);
        let native_fps = src.and_then(|s| s.video_fps);
        let has_video = collected.iter().any(|c| c.clip_type == ClipType::Video);
        let has_audio = collected.iter().any(|c| c.clip_type == ClipType::Audio);
        let first_audio = collected.iter().find(|c| c.clip_type == ClipType::Audio);
        let file_name = src
            .and_then(|s| s.path.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| primary.name.clone());
        let video_summary = match (src.and_then(|s| s.video_width), src.and_then(|s| s.video_height), native_fps) {
            (Some(w), Some(h), Some(fps)) => format!("{w} × {h}, Native {:.2} fps", fps.to_f64()),
            _ if primary.clip_type == ClipType::Title => "Title Overlay".to_string(),
            _ if primary.clip_type == ClipType::Image => "Still Image".to_string(),
            _ => "Audio Clip".to_string(),
        };
        let audio_summary = match src.and_then(|s| Some((s.audio_channels?, s.audio_sample_rate))) {
            Some((ch, Some(sr))) => format!("{ch} ch · {sr} Hz"),
            Some((ch, None)) => format!("{ch} ch"),
            None if has_audio => "Timeline Audio".to_string(),
            None => "No audio stream".to_string(),
        };
        let initial_conform_fps = native_fps.and_then(|nf| {
            let eff = nf.to_f64() * primary.properties.speed;
            RATES
                .iter()
                .map(|(_, r)| *r)
                .find(|r| (r.to_f64() - eff).abs() < 0.05 && (r.to_f64() - nf.to_f64()).abs() >= 0.05)
        });
        let subtitle = if collected.len() == 1 {
            format!("Timeline Clip · {}", primary.name)
        } else {
            format!("Timeline Selection · {} clips", collected.len())
        };
        let spec = ClipAttributesSpec {
            subtitle,
            initial_name: primary.name.clone(),
            file_name,
            has_video,
            video_summary,
            native_fps,
            initial_conform_fps,
            source_duration_us: (primary.source_out - primary.source_in).max(1),
            preview_fps: p.timeline_fps(),
            has_audio,
            audio_summary,
            initial_channel_map: first_audio.map(|c| c.properties.audio_channel_map).unwrap_or_default(),
        };
        let limits = p
            .sources
            .iter()
            .filter(|(_, s)| s.duration_us > 0)
            .map(|(id, s)| (*id, s.duration_us))
            .collect::<std::collections::HashMap<_, _>>();
        Some((collected, spec, p.timeline_fps(), limits, native_fps))
    }).flatten() else {
        return;
    };

    let state = state.clone();
    let initial_name = spec.initial_name.clone();
    present_clip_attributes_dialog(parent, spec, move |patch| {
        let mut cmds: Vec<Box<dyn Command>> = Vec::new();
        let rename_to = (!patch.display_name.is_empty() && patch.display_name != initial_name).then_some(patch.display_name);

        for clip in &clips {
            let mut next = clip.clone();
            let mut edited = false;
            if let Some(ref new_name) = rename_to {
                if next.name != *new_name {
                    next.name = new_name.clone();
                    edited = true;
                }
            }
            if patch.channel_map_changed && next.clip_type == ClipType::Audio && next.properties.audio_channel_map != patch.channel_map {
                next.properties.audio_channel_map = patch.channel_map;
                edited = true;
            }
            if edited {
                cmds.push(Box::new(EditClipCommand::new("Clip Attributes", next)));
            }
        }

        if patch.fps_changed {
            if let Some(native) = lead_fps.filter(|f| f.to_f64() > 0.0) {
                let target_speed = match patch.conform_fps {
                    Some(target) => (target.to_f64() / native.to_f64()).clamp(MIN_SPEED, MAX_SPEED),
                    None => 1.0,
                };
                let mut seen_links = std::collections::HashSet::new();
                for clip in &clips {
                    if !matches!(clip.clip_type, ClipType::Video | ClipType::Audio) {
                        continue;
                    }
                    let key = clip.properties.link.unwrap_or(clip.id);
                    if seen_links.insert(key) && (clip.properties.speed - target_speed).abs() > 1e-4 {
                        cmds.push(Box::new(SetClipSpeedCommand::new(
                            clip.id,
                            target_speed,
                            true,
                            timeline_fps,
                            source_limits.clone(),
                        )));
                    }
                }
            }
        }

        if !cmds.is_empty() {
            state.execute(Box::new(CompositeCommand::new("Clip Attributes", cmds)));
        }
    });
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
    let show_splash = adw::SwitchRow::builder()
        .title("Show startup splash screen")
        .subtitle("GTA-style behind-the-scenes cards and tips while loading (press any key to skip)")
        .active(current.show_splash)
        .build();
    let c = change.clone();
    show_splash.connect_active_notify(move |row| c(&|s| s.show_splash = row.is_active()));
    general.add(&show_splash);

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
    use super::{marks_available, chosen_timeline_fps, default_start_tc_us, frame_count, one_frame_us, parse_timecode, rate_index, shown_marker_duration_us, stored_marker_duration_us, timeline_fps_choice, timeline_from_form, typed_position_us, RATES};
    use tempo_timeline::{AudioTrackType, RationalFps, TrackKind};

    #[test]
    fn marks_apply_only_to_the_selected_clip_with_a_mark() {
        let (a, b) = (uuid::Uuid::new_v4(), uuid::Uuid::new_v4());
        assert!(marks_available(Some(a), Some(a), Some(5), None));
        assert!(marks_available(Some(a), Some(a), None, Some(9)));
        assert!(!marks_available(Some(a), Some(a), None, None));
        assert!(!marks_available(Some(a), Some(b), Some(5), Some(9)));
        assert!(!marks_available(None, None, Some(5), Some(9)));
    }

    #[test]
    fn typed_position_subtracts_start_timecode() {
        let start = 3_600_000_000;
        assert_eq!(typed_position_us("01:00:01:00", 24.0, start), Some(1_000_000));
        assert_eq!(typed_position_us("01:00:00:00", 24.0, start), Some(0));
        assert_eq!(typed_position_us("00:59:59:00", 24.0, start), None);
    }

    /// Every listed rate, as the dialogs see it.
    fn all_rates() -> impl Iterator<Item = RationalFps> {
        RATES.iter().map(|r| r.1)
    }

    /// `AppState::frame_floor` for a rate: the start of the frame a time is in.
    fn frame_floor(us: i64, fps: RationalFps) -> i64 {
        (frame_count(us, fps) as i128 * fps.den as i128 * 1_000_000 + fps.num as i128 / 2).div_euclid(fps.num as i128) as i64
    }

    #[test]
    fn point_marker_shows_one_frame_and_one_frame_stores_as_point() {
        for rate in all_rates() {
            let fps = rate.to_f64();
            let one = one_frame_us(fps);
            assert_eq!(shown_marker_duration_us(0, fps), one);
            assert_eq!(shown_marker_duration_us(one * 5, fps), one * 5);
            assert_eq!(crate::state::timecode(shown_marker_duration_us(0, fps), fps), "00:00:00:01", "{fps}");
            // Renaming a point marker re-parses its shown duration and must keep it a point.
            let typed = parse_timecode("00:00:00:01", fps).unwrap();
            assert_eq!(stored_marker_duration_us(typed, frame_floor(typed, rate), rate), 0, "{fps}");
            assert_eq!(stored_marker_duration_us(0, 0, rate), 0, "{fps}");
            let two = parse_timecode("00:00:00:02", fps).unwrap();
            assert_eq!(stored_marker_duration_us(two, frame_floor(two, rate), rate), frame_floor(two, rate), "{fps}");
        }
    }

    #[test]
    fn start_timecode_is_read_at_the_new_timelines_rate() {
        let (project, selected) = (RationalFps::FPS_30, RationalFps::FPS_25);
        assert_eq!(chosen_timeline_fps(true, selected, project), project);
        assert_eq!(chosen_timeline_fps(false, selected, project), selected);
        for rate in [RationalFps::FPS_29_97, RationalFps::FPS_23_976] {
            let fps = chosen_timeline_fps(false, rate, project).to_f64();
            assert_eq!(crate::state::timecode(default_start_tc_us(fps), fps), "01:00:00:00");
        }
        let at25 = chosen_timeline_fps(false, selected, project).to_f64();
        assert_eq!(parse_timecode("01:00:00:27", at25), None);
        let us = parse_timecode("01:00:00:12", at25).unwrap();
        assert_eq!(crate::state::timecode(us, at25), "01:00:00:12");
    }

    #[test]
    fn timeline_fps_is_none_when_it_equals_the_project_rate() {
        assert_eq!(timeline_fps_choice(RationalFps::FPS_30, RationalFps::FPS_30), None);
        assert_eq!(timeline_fps_choice(RationalFps::FPS_24, RationalFps::FPS_30), Some(RationalFps::FPS_24));
        assert_eq!(RATES[rate_index(RationalFps::FPS_59_94) as usize].0, "59.94");
    }

    #[test]
    fn marker_duration_timecode_round_trips_without_start_offset() {
        for fps in [24.0, 25.0, 29.97] {
            let one_frame = (1_000_000.0 / fps) as i64;
            let text = crate::state::timecode(one_frame, fps);
            assert_eq!(text, "00:00:00:01");
            assert_eq!(parse_timecode(&text, fps), Some(one_frame));
        }
        assert_eq!(parse_timecode("00:00:00:00", 24.0), Some(0));
    }

    #[test]
    fn default_start_reads_one_hour_at_ntsc_rates() {
        for fps in [23.976, 29.97, 59.94, 24.0, 25.0] {
            assert_eq!(crate::state::timecode(default_start_tc_us(fps), fps), "01:00:00:00", "{fps}");
        }
    }

    #[test]
    fn form_values_reach_the_timeline() {
        let t = timeline_from_form("T".into(), 1, 2, AudioTrackType::Mono, 5, Some((1080, 1920)), Some(RationalFps::FPS_50));
        assert_eq!((t.start_tc_us, t.resolution), (5, Some((1080, 1920))));
        assert!(t.tracks.iter().filter(|x| x.kind == TrackKind::Audio).all(|x| x.audio_type == AudioTrackType::Mono));
        assert_eq!(timeline_from_form("T".into(), 1, 1, AudioTrackType::Stereo, 0, None, None).resolution, None);
    }

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
