//! Export page: settings at the left, viewer in the middle, render queue at
//! the right, as on DaVinci Resolve's Deliver page.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use gtk4 as gtk;
use gtk4::prelude::*;
use gtk4::gio;
use libadwaita as adw;
use libadwaita::prelude::*;
use tempo_export::{
    bitrate_for_size, chapters_from_markers, export_timeline, format_chapter_list, youtube_problems, AudioCodec, Chapter, ContainerFormat, Fit, TimelineExport,
    VideoCodec,
};
use tempo_plugin::UploadInput;
use tempo_timeline::Project;

use crate::plugins::Uploader;
use crate::state::{AppState, Change};
use crate::util::{css_color, label, pill, tool_button};

/// Output shapes. Shown by ratio, with what each suits; no platform logos.
struct Format {
    ratio: &'static str,
    width: u32,
    height: u32,
    best_for: &'static str,
    /// Keep the file under this many bytes, choosing the bitrate (and 720p when it is low).
    max_bytes: Option<u64>,
}

const CUSTOM_INDEX: usize = 6;

const FORMATS: [Format; 7] = [
    Format { ratio: "16:9", width: 1920, height: 1080, best_for: "YouTube 1080p, Vimeo", max_bytes: None },
    Format { ratio: "16:9 4K", width: 3840, height: 2160, best_for: "YouTube 2160p", max_bytes: None },
    Format { ratio: "9:16", width: 1080, height: 1920, best_for: "TikTok, Reels, Shorts", max_bytes: None },
    Format { ratio: "1:1", width: 1080, height: 1080, best_for: "Instagram feed", max_bytes: None },
    Format { ratio: "4:5", width: 1080, height: 1350, best_for: "Instagram portrait", max_bytes: None },
    // Discord's limit for accounts without Nitro.
    Format { ratio: "≤ 10 MB", width: 1920, height: 1080, best_for: "Discord, email", max_bytes: Some(10_000_000) },
    Format { ratio: "Custom", width: 1920, height: 1080, best_for: "Set your own", max_bytes: None },
];

const CONTAINERS: [(&str, ContainerFormat); 3] = [("MP4", ContainerFormat::Mp4), ("MOV", ContainerFormat::Mov), ("MKV", ContainerFormat::Mkv)];
const VIDEO_CODECS: [(&str, VideoCodec); 2] = [("H.264", VideoCodec::H264), ("H.265", VideoCodec::H265)];
const AUDIO_CODECS: [(&str, AudioCodec); 3] = [("AAC", AudioCodec::Aac), ("FLAC", AudioCodec::Flac), ("PCM", AudioCodec::Pcm)];
const NORMALIZE_MODES: [(&str, Option<f32>); 2] = [("Off", None), ("YouTube (-14 LUFS)", Some(-14.0))];

/// Sound bitrate for size-limited exports: clear speech, more room for the picture.
const SIZE_LIMITED_AUDIO_KBPS: u32 = 96;

const QUALITIES: [(&str, u8, u32); 3] = [("High", 18, 192), ("Medium", 23, 160), ("Small file", 28, 128)];

#[derive(Clone, PartialEq)]
enum Status {
    Waiting,
    Rendering,
    Done,
    Failed(String),
    Cancelled,
}

/// Where a job's upload to a plugin stands.
#[derive(Clone, PartialEq)]
enum UploadState {
    Waiting,
    Uploading,
    /// Sent; the address when the plugin returned one.
    Done(Option<String>),
    Failed(String),
    /// No saved token: the upload dialog was opened for the user.
    NeedsToken,
}

/// What to send once the render finishes. The values are per job.
struct UploadPlan {
    uploader: Uploader,
    input: UploadInput,
    state: UploadState,
}

struct Job {
    name: String,
    /// "<project> | <timeline>", as Resolve's Deliver queue shows it.
    label: String,
    format: String,
    settings: TimelineExport,
    project: Arc<Project>,
    total_us: i64,
    status: Status,
    progress: Arc<AtomicU32>,
    cancel: Arc<AtomicBool>,
    upload: Option<UploadPlan>,
}

/// The "Upload directly to" section of Render Settings.
struct UploadForm {
    section: gtk::Box,
    enabled: gtk::CheckButton,
    pick: gtk::DropDown,
    pick_model: gtk::StringList,
    title: gtk::Entry,
    description: gtk::TextView,
    thumbnail_on: gtk::CheckButton,
    thumbnail: gtk::Entry,
    privacy: gtk::DropDown,
    category: gtk::DropDown,
}

pub struct ExportPage {
    pub root: gtk::Box,
    /// Holds the viewer while this page is showing.
    pub viewer_slot: gtk::Box,
    state: Rc<AppState>,
    title: gtk::Label,
    file_name: gtk::Entry,
    location: gtk::Entry,
    free_space: gtk::Label,
    format_dd: gtk::DropDown,
    video_codec_dd: gtk::DropDown,
    audio_codec_dd: gtk::DropDown,
    quality: gtk::DropDown,
    normalize_dd: gtk::DropDown,
    width_label: gtk::Label,
    custom_width: gtk::SpinButton,
    height_label: gtk::Label,
    custom_height: gtk::SpinButton,
    fit: gtk::DropDown,
    fit_row: gtk::Box,
    chapters: gtk::CheckButton,
    upload: UploadForm,
    range: gtk::DropDown,
    format_index: Cell<usize>,
    /// Prevents signal loops between preset cards and codec/container dropdowns.
    syncing: Cell<bool>,
    /// The format toggles, in the order of `FORMATS`.
    format_buttons: RefCell<Vec<gtk::ToggleButton>>,
    /// When the current run was started, to ignore a double click on the button.
    started: Cell<Option<std::time::Instant>>,
    jobs: RefCell<Vec<Job>>,
    queue_box: gtk::Box,
    selected_job: Cell<Option<usize>>,
    render_only: Cell<Option<usize>>,
    render_all: gtk::Button,
    /// One line describing the running job, for the page bar.
    activity: RefCell<Option<Box<dyn Fn(Option<String>)>>>,
}

impl ExportPage {
    pub fn new(state: &Rc<AppState>) -> Rc<Self> {
        // ---- Left: render settings ------------------------------------------
        let title = label("Render Settings", &[]);
        title.set_xalign(0.0);
        title.set_hexpand(true);
        let export_help = crate::help::panel_guide_button(
            "Render Settings & Queue",
            "Deliver Page",
            "Choose a preset (like YouTube 1080p) or pick Custom to change format, codecs, and -14 LUFS loudness normalization.",
            &[
                ("Ctrl+Return", "Add job to Render Queue"),
                ("Click a Job Card", "Switch Render All -> Render 1"),
                ("Ctrl+Shift+Return", "Start rendering"),
            ],
        );
        let settings_header = gtk::Box::builder().css_classes(["panel-title"]).build();
        settings_header.append(&title);
        settings_header.append(&export_help);

        let strip = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(4).margin_top(4).margin_bottom(2).margin_start(8).margin_end(8).build();
        let strip_scroll = gtk::ScrolledWindow::builder().child(&strip).vscrollbar_policy(gtk::PolicyType::Never).build();

        let file_name = gtk::Entry::builder().hexpand(true).build();
        let location = gtk::Entry::builder().hexpand(true).build();
        let browse = pill("Browse");
        let free_space = label("", &["tempo-small", "tempo-dim"]);
        free_space.set_xalign(0.0);
        let format_dd = gtk::DropDown::from_strings(&CONTAINERS.map(|c| c.0));
        let video_codec_dd = gtk::DropDown::from_strings(&VIDEO_CODECS.map(|c| c.0));
        let audio_codec_dd = gtk::DropDown::from_strings(&AUDIO_CODECS.map(|c| c.0));
        let quality = gtk::DropDown::from_strings(&QUALITIES.map(|q| q.0));
        let normalize_dd = gtk::DropDown::from_strings(&NORMALIZE_MODES.map(|n| n.0));
        let custom_width = gtk::SpinButton::with_range(16.0, 7680.0, 2.0);
        custom_width.set_digits(0);
        custom_width.set_value(1920.0);
        custom_width.set_increments(2.0, 16.0);
        custom_width.set_visible(false);
        let custom_height = gtk::SpinButton::with_range(16.0, 7680.0, 2.0);
        custom_height.set_digits(0);
        custom_height.set_value(1080.0);
        custom_height.set_increments(2.0, 16.0);
        custom_height.set_visible(false);
        let fit = gtk::DropDown::from_strings(&["Fit (black bars)", "Fill (crop)"]);
        let chapters = gtk::CheckButton::with_label("Chapters from markers");
        chapters.set_active(true);

        let grid = gtk::Grid::builder().row_spacing(5).column_spacing(10).margin_top(6).margin_start(12).margin_end(12).build();
        let row = |r: i32, name: &str, w: &gtk::Widget| -> gtk::Label {
            let l = label(name, &["tempo-dim"]);
            l.set_xalign(1.0);
            grid.attach(&l, 0, r, 1, 1);
            grid.attach(w, 1, r, 1, 1);
            l
        };
        row(0, "File Name", file_name.upcast_ref());
        let loc_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        loc_box.append(&location);
        loc_box.append(&browse);
        row(1, "Location", loc_box.upcast_ref());
        grid.attach(&free_space, 1, 2, 1, 1);
        row(3, "Format", format_dd.upcast_ref());
        row(4, "Video Codec", video_codec_dd.upcast_ref());
        row(5, "Audio Codec", audio_codec_dd.upcast_ref());
        row(6, "Quality", quality.upcast_ref());
        row(7, "Normalize", normalize_dd.upcast_ref());
        let width_label = row(8, "Width", custom_width.upcast_ref());
        width_label.set_visible(false);
        let height_label = row(9, "Height", custom_height.upcast_ref());
        height_label.set_visible(false);
        // The Fit / Fill choice only matters when the shapes differ.
        let fit_row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let fit_label = label("Shape", &["tempo-dim"]);
        fit_label.set_xalign(1.0);
        fit_row.append(&fit_label);
        fit_row.append(&fit);
        grid.attach(&fit_row, 0, 10, 2, 1);
        grid.attach(&chapters, 1, 11, 1, 1);

        let add = pill("Add to Render Queue");
        add.set_tooltip_text(Some("Add to Render Queue (Ctrl+Return)"));
        add.set_halign(gtk::Align::End);
        add.set_margin_top(10);
        add.set_margin_end(12);
        add.set_margin_bottom(8);

        let upload = UploadForm::new();
        let settings = gtk::Box::new(gtk::Orientation::Vertical, 0);
        settings.append(&grid);
        settings.append(&upload.section);
        let left = gtk::Box::builder().orientation(gtk::Orientation::Vertical).width_request(360).css_classes(["tempo-surface", "tempo-sep-right"]).build();
        left.append(&settings_header);
        left.append(&strip_scroll);
        left.append(&gtk::ScrolledWindow::builder().child(&settings).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build());
        left.append(&add);

        // ---- Centre: viewer and range ---------------------------------------
        let viewer_slot = gtk::Box::builder().hexpand(true).vexpand(true).build();
        let range = gtk::DropDown::from_strings(&["Entire Timeline", "In/Out Range"]);
        let range_row = gtk::Box::builder().spacing(10).halign(gtk::Align::Center).margin_top(8).margin_bottom(8).build();
        range_row.append(&label("Render", &["tempo-dim"]));
        range_row.append(&range);
        let centre = gtk::Box::builder().orientation(gtk::Orientation::Vertical).hexpand(true).css_classes(["tempo-panel"]).build();
        centre.append(&viewer_slot);
        centre.append(&range_row);

        // ---- Right: render queue --------------------------------------------
        let queue_title = label("Render Queue", &["panel-title"]);
        queue_title.set_xalign(0.0);
        let queue_box = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).margin_top(8).margin_start(8).margin_end(8).build();
        let queue_scroll = gtk::ScrolledWindow::builder().child(&queue_box).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build();
        let render_all = pill("Render All");
        render_all.set_tooltip_text(Some("Render All (Ctrl+Shift+Return)"));
        render_all.set_halign(gtk::Align::End);
        render_all.set_margin_top(8);
        render_all.set_margin_end(12);
        render_all.set_margin_bottom(12);
        let right = gtk::Box::builder().orientation(gtk::Orientation::Vertical).width_request(360).css_classes(["tempo-surface", "tempo-sep-left"]).build();
        right.append(&queue_title);
        right.append(&queue_scroll);
        right.append(&render_all);

        let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        root.append(&left);
        root.append(&centre);
        root.append(&right);

        let page = Rc::new(Self {
            root,
            viewer_slot,
            state: state.clone(),
            title,
            file_name,
            location,
            free_space,
            format_dd,
            video_codec_dd,
            audio_codec_dd,
            quality,
            normalize_dd,
            width_label,
            custom_width,
            height_label,
            custom_height,
            fit,
            fit_row,
            chapters,
            upload,
            range,
            format_index: Cell::new(0),
            syncing: Cell::new(false),
            format_buttons: RefCell::new(Vec::new()),
            started: Cell::new(None),
            jobs: RefCell::new(Vec::new()),
            queue_box,
            selected_job: Cell::new(None),
            render_only: Cell::new(None),
            render_all,
            activity: RefCell::new(None),
        });

        // Format strip: one toggle per shape.
        let mut group: Option<gtk::ToggleButton> = None;
        for (i, f) in FORMATS.iter().enumerate() {
            let button = format_button(f);
            if let Some(g) = &group {
                button.set_group(Some(g));
            } else {
                group = Some(button.clone());
                button.set_active(true);
            }
            let p = page.clone();
            button.connect_toggled(move |b| {
                if b.is_active() {
                    p.format_index.set(i);
                    // Vertical video from a landscape edit: fill the frame, as Shorts and Reels expect.
                    if f.height > f.width && p.state.with_project(|pr| { let (w, h) = pr.output_size(); w > h }).unwrap_or(false) {
                        p.fit.set_selected(1);
                    }
                    if i < CUSTOM_INDEX && !p.syncing.get() {
                        p.syncing.set(true);
                        p.format_dd.set_selected(0);
                        p.video_codec_dd.set_selected(0);
                        p.audio_codec_dd.set_selected(0);
                        p.normalize_dd.set_selected(0);
                        p.custom_width.set_value(f.width as f64);
                        p.custom_height.set_value(f.height as f64);
                        p.syncing.set(false);
                    }
                    p.refresh_settings();
                }
            });
            strip.append(&button);
            page.format_buttons.borrow_mut().push(button);
        }

        for dd in [&page.format_dd, &page.video_codec_dd, &page.audio_codec_dd] {
            let p = page.clone();
            dd.connect_selected_notify(move |_| p.switch_to_custom());
        }
        for spin in [&page.custom_width, &page.custom_height] {
            let p = page.clone();
            spin.connect_value_changed(move |_| {
                if !p.syncing.get() && p.format_index.get() == CUSTOM_INDEX {
                    p.refresh_settings();
                }
            });
        }

        let empty_click = gtk::GestureClick::builder().button(gtk::gdk::BUTTON_PRIMARY).build();
        let p = page.clone();
        empty_click.connect_pressed(move |gesture, _, x, y| {
            let Some(widget) = gesture.widget() else { return };
            if inside_job_card(widget.pick(x, y, gtk::PickFlags::DEFAULT)) {
                return;
            }
            if p.selected_job.get().is_some() {
                p.selected_job.set(None);
                p.refresh_selection_ui();
            }
        });
        queue_scroll.add_controller(empty_click);

        let p = page.clone();
        add.connect_clicked(move |_| {
            p.add_to_queue();
        });
        let p = page.clone();
        page.render_all.connect_clicked(move |_| p.render_all_clicked());
        let p = page.clone();
        browse.connect_clicked(move |b| {
            let window = b.root().and_downcast::<gtk::Window>();
            let dialog = gtk::FileDialog::builder().title("Export Location").modal(true).build();
            let p = p.clone();
            dialog.select_folder(window.as_ref(), gio::Cancellable::NONE, move |result| {
                if let Some(path) = result.ok().and_then(|f| f.path()) {
                    p.location.set_text(&path.to_string_lossy());
                    p.update_free_space();
                }
            });
        });
        let p = page.clone();
        page.location.connect_activate(move |_| p.update_free_space());

        let p = page.clone();
        page.range.connect_selected_notify(move |_| p.update_size_cards());
        let p = page.clone();
        // The file name follows the open timeline; edits to the same timeline keep what was typed.
        let shown = std::cell::Cell::new(state.with_timeline(|t| t.id));
        state.connect(move |change| {
            p.update_size_cards();
            match change {
                Change::Project => {
                    shown.set(p.state.with_timeline(|t| t.id));
                    p.load_defaults();
                }
                Change::Timeline if shown.get() != p.state.with_timeline(|t| t.id) => {
                    shown.set(p.state.with_timeline(|t| t.id));
                    p.file_name.set_text(&p.state.with_project(default_file_name).unwrap_or_default());
                    p.reset_upload_title();
                }
                // The Share menus list the upload targets of enabled plugins.
                Change::Plugins => {
                    p.sync_uploaders();
                    p.rebuild_queue();
                }
                _ => {}
            }
        });
        let p = page.clone();
        page.upload.pick.connect_selected_notify(move |_| p.sync_uploader_label());
        page.sync_uploaders();
        page.load_defaults();
        page.rebuild_queue();
        page
    }

    fn reset_upload_title(&self) {
        self.upload.title.set_text(&self.state.with_project(|p| p.timeline.name.clone()).unwrap_or_default());
    }

    fn selected_uploader(&self) -> Option<Uploader> {
        self.state.uploaders.borrow().get(self.upload.pick.selected() as usize).cloned()
    }

    fn sync_uploader_label(&self) {
        let name = self.selected_uploader().map(|u| u.name).unwrap_or_default();
        self.upload.enabled.set_label(Some(&format!("Upload directly to {name}")));
    }

    /// Show the section only while an enabled plugin offers an uploader.
    fn sync_uploaders(&self) {
        let names: Vec<String> = self.state.uploaders.borrow().iter().map(|u| u.name.clone()).collect();
        let keep = self.upload.pick.selected();
        self.upload.pick_model.splice(0, self.upload.pick_model.n_items(), &names.iter().map(String::as_str).collect::<Vec<_>>());
        self.upload.pick.set_selected(if (keep as usize) < names.len() { keep } else { 0 });
        self.upload.pick.set_visible(names.len() > 1);
        self.upload.section.set_visible(!names.is_empty());
        self.sync_uploader_label();
    }

    /// The upload to attach to a job being queued, or None when the box is not ticked.
    /// `Err` carries a message for the user.
    fn upload_plan(&self, output: &std::path::Path, chapters: &[Chapter]) -> Result<Option<UploadPlan>, String> {
        let form = &self.upload;
        let Some(uploader) = self.selected_uploader().filter(|_| form.enabled.is_active()) else { return Ok(None) };
        let thumbnail = form.thumbnail.text().trim().to_string();
        if form.thumbnail_on.is_active() && thumbnail.is_empty() {
            return Err("Choose a thumbnail image, or untick Upload Thumbnail.".into());
        }
        let buffer = form.description.buffer();
        let description = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false);
        let title = form.title.text().trim().to_string();
        let input = UploadInput {
            file: output.to_path_buf(),
            title: if title.is_empty() { self.state.with_project(|p| p.timeline.name.clone()).unwrap_or_default() } else { title },
            description: tempo_plugin::description_with_chapters(&description, &format_chapter_list(chapters)),
            privacy: tempo_plugin::PRIVACY.get(form.privacy.selected() as usize).unwrap_or(&"private").to_string(),
            category: tempo_plugin::CATEGORIES.get(form.category.selected() as usize).unwrap_or(&"Film & Animation").to_string(),
            thumbnail: (form.thumbnail_on.is_active()).then(|| PathBuf::from(thumbnail)),
            token: String::new(),
        };
        Ok(Some(UploadPlan { uploader, input, state: UploadState::Waiting }))
    }

    pub fn connect_activity(&self, f: impl Fn(Option<String>) + 'static) {
        *self.activity.borrow_mut() = Some(Box::new(f));
    }

    fn load_defaults(self: &Rc<Self>) {
        let name = self.state.with_project(default_file_name).unwrap_or_default();
        self.file_name.set_text(&name);
        if self.location.text().is_empty() {
            let dir = glib::user_special_dir(glib::UserDirectory::Videos).unwrap_or_else(glib::home_dir);
            self.location.set_text(&dir.to_string_lossy());
        }
        // Start from the shape of the project: a vertical project exports
        // vertical, a 4K one in 4K. Shapes are compared by cross-multiplying.
        if let Some((w, h)) = self.state.with_project(|p| { let (w, h) = p.output_size(); (w as u64, h as u64) }) {
            let same_shape = |f: &Format| f.width as u64 * h == f.height as u64 * w;
            let best = FORMATS[..CUSTOM_INDEX]
                .iter()
                .enumerate()
                .filter(|(_, f)| same_shape(f))
                .min_by_key(|(_, f)| (f.height as i64 - h as i64).abs())
                .map(|(i, _)| i);
            if let Some(button) = best.and_then(|i| self.format_buttons.borrow().get(i).cloned()) {
                button.set_active(true);
            }
        }
        self.update_free_space();
        self.refresh_settings();
    }

    fn selected_container(&self) -> ContainerFormat {
        CONTAINERS.get(self.format_dd.selected() as usize).map(|c| c.1).unwrap_or_default()
    }

    fn selected_video_codec(&self) -> VideoCodec {
        VIDEO_CODECS.get(self.video_codec_dd.selected() as usize).map(|c| c.1).unwrap_or_default()
    }

    fn selected_audio_codec(&self) -> AudioCodec {
        AUDIO_CODECS.get(self.audio_codec_dd.selected() as usize).map(|c| c.1).unwrap_or_default()
    }

    fn selected_normalize_lufs(&self) -> Option<f32> {
        NORMALIZE_MODES.get(self.normalize_dd.selected() as usize).and_then(|n| n.1)
    }

    fn current_output_size(&self) -> (u32, u32) {
        if self.format_index.get() == CUSTOM_INDEX {
            let w = ((self.custom_width.value_as_int().clamp(16, 7680) as u32) + 1) & !1;
            let h = ((self.custom_height.value_as_int().clamp(16, 7680) as u32) + 1) & !1;
            (w.min(7680), h.min(7680))
        } else {
            let f = &FORMATS[self.format_index.get().min(CUSTOM_INDEX)];
            (f.width, f.height)
        }
    }

    fn switch_to_custom(&self) {
        if self.syncing.get() {
            return;
        }
        self.syncing.set(true);
        if let Some(btn) = self.format_buttons.borrow().get(CUSTOM_INDEX) {
            btn.set_active(true);
        }
        self.syncing.set(false);
        self.refresh_settings();
    }

    /// Grey out size-limited cards when what would be exported is too long for them.
    fn update_size_cards(&self) {
        let end = self.state.with_project(|p| p.timeline.tracks.iter().map(|t| t.duration_us()).max().unwrap_or(0)).unwrap_or(0);
        let length = match (self.range.selected(), self.state.mark_in.get(), self.state.mark_out.get()) {
            (1, Some(a), Some(b)) if b > a => b - a,
            _ => end,
        };
        for (f, button) in FORMATS.iter().zip(self.format_buttons.borrow().iter()) {
            let Some(max) = f.max_bytes else { continue };
            let fits = length <= 0 || bitrate_for_size(max, length, SIZE_LIMITED_AUDIO_KBPS).is_some();
            button.set_sensitive(fits);
            button.set_tooltip_text(Some(if fits {
                "Up to 10 MB — picks the bitrate, and 720p when it must, so the file fits Discord's limit"
            } else {
                "Too long to fit in 10 MB and still look good. Set In and Out around a shorter part"
            }));
        }
    }

    fn refresh_settings(&self) {
        let idx = self.format_index.get().min(CUSTOM_INDEX);
        let f = &FORMATS[idx];
        let (w, h) = self.current_output_size();
        self.title.set_text(&format!("Render Settings - {} - {} × {}", f.ratio, w, h));
        let is_custom = idx == CUSTOM_INDEX;
        self.width_label.set_visible(is_custom);
        self.custom_width.set_visible(is_custom);
        self.height_label.set_visible(is_custom);
        self.custom_height.set_visible(is_custom);
        // Compare shapes by cross-multiplying; no rounding trouble.
        let same_shape = self
            .state
            .with_project(|p| { let (pw, ph) = p.output_size(); pw as u64 * h as u64 == ph as u64 * w as u64 })
            .unwrap_or(true);
        self.fit_row.set_visible(!same_shape);
    }

    fn update_free_space(self: &Rc<Self>) {
        let dir = gio::File::for_path(self.location.text().as_str());
        let page = self.clone();
        glib::MainContext::default().spawn_local(async move {
            let info = dir.query_filesystem_info_future("filesystem::free", glib::Priority::DEFAULT).await;
            let text = match info {
                Ok(i) => format!("{} free", glib::format_size(i.attribute_uint64("filesystem::free"))),
                Err(_) => "This folder does not exist yet; it will be created.".to_string(),
            };
            page.free_space.set_text(&text);
        });
    }

    pub fn add_to_queue(self: &Rc<Self>) -> bool {
        let Some(project) = self.state.project.borrow().clone() else { return false };
        let timeline_end = project.timeline.tracks.iter().map(|t| t.duration_us()).max().unwrap_or(0);
        if timeline_end <= 0 {
            self.state.message("The timeline is empty; there is nothing to export.");
            return false;
        }
        let range = if self.range.selected() == 1 {
            match (self.state.mark_in.get(), self.state.mark_out.get()) {
                (Some(a), Some(b)) if b > a => Some((a, b)),
                _ => {
                    self.state.message("Set In and Out points first (I and O), or render the entire timeline.");
                    return false;
                }
            }
        } else {
            None
        };
        let (r0, r1) = range.unwrap_or((0, timeline_end));
        let name = self.file_name.text().trim().to_string();
        let name = if name.is_empty() { default_file_name(&project) } else { name };
        let idx = self.format_index.get().min(CUSTOM_INDEX);
        let f = &FORMATS[idx];
        let container = self.selected_container();
        let video_codec = self.selected_video_codec();
        let audio_codec = self.selected_audio_codec();
        let normalize_lufs = self.selected_normalize_lufs();
        if container == ContainerFormat::Mp4 && audio_codec == AudioCodec::Pcm {
            self.state.message("PCM audio is not supported in MP4; select MOV or MKV.");
            return false;
        }
        if container == ContainerFormat::Mov && audio_codec == AudioCodec::Flac {
            self.state.message("FLAC audio is not supported in MOV; select MP4 or MKV.");
            return false;
        }

        let (_, crf, mut audio_kbps) = QUALITIES.get(self.quality.selected() as usize).copied().unwrap_or(QUALITIES[1]);
        let (mut width, mut height) = self.current_output_size();
        let mut max_video_kbps = None;
        if let Some(max) = f.max_bytes {
            let Some(kbps) = bitrate_for_size(max, r1 - r0, SIZE_LIMITED_AUDIO_KBPS) else {
                self.state.message("This is too long to fit in 10 MB and still look good. Set In and Out around a shorter part.");
                return false;
            };
            // Below about 1.5 Mb/s, 720p looks better than a starved 1080p. Portrait stays portrait.
            (width, height) = if kbps < 1500 { (1280, 720) } else { (1920, 1080) };
            if project.output_size().1 > project.output_size().0 {
                (width, height) = (height, width);
            }
            (audio_kbps, max_video_kbps) = (SIZE_LIMITED_AUDIO_KBPS, Some(kbps));
        }

        // Do not silently replace an existing file or an output already queued.
        let dir = PathBuf::from(self.location.text().as_str());
        let taken = |p: &PathBuf| p.exists() || self.jobs.borrow().iter().any(|j| &j.settings.output == p);
        let ext = container.extension();
        let mut output = dir.join(format!("{name}.{ext}"));
        let mut n = 2;
        while taken(&output) {
            output = dir.join(format!("{name} {n}.{ext}"));
            n += 1;
        }

        let chapters = if self.chapters.is_active() { chapters_from_markers(&project.timeline.markers, r0, r1) } else { Vec::new() };
        let settings = TimelineExport {
            width,
            height,
            crf,
            audio_kbps,
            fit: if self.fit.selected() == 1 { Fit::Fill } else { Fit::Fit },
            range,
            chapters,
            output,
            // A size limit needs the processor's encoder, which keeps to a bitrate exactly.
            hardware: self.state.settings.borrow().hardware_encode && max_video_kbps.is_none(),
            max_video_kbps,
            container,
            video_codec,
            audio_codec,
            fps: None,
            normalize_lufs,
        };
        let upload = match self.upload_plan(&settings.output, &settings.chapters) {
            Ok(plan) => plan,
            Err(message) => {
                self.state.message(message);
                return false;
            }
        };
        let format = if normalize_lufs.is_some() {
            format!("{} · {} × {} · {} · -14 LUFS", f.ratio, width, height, ext.to_uppercase())
        } else {
            format!("{} · {} × {} · {}", f.ratio, width, height, ext.to_uppercase())
        };
        // Like Resolve: the next job starts without the last cover image.
        self.upload.thumbnail.set_text("");
        self.jobs.borrow_mut().push(Job {
            upload,
            label: job_label(&project),
            name,
            format,
            settings,
            // A copy of the project as it is now: later edits do not change this job.
            project: Arc::new(project),
            total_us: r1 - r0,
            status: Status::Waiting,
            progress: Arc::new(AtomicU32::new(0)),
            cancel: Arc::new(AtomicBool::new(false)),
        });
        self.rebuild_queue();
        self.state.checklist_queued.set(true);
        self.state.emit(Change::Options);
        true
    }

    fn is_rendering(&self) -> bool {
        self.jobs.borrow().iter().any(|j| j.status == Status::Rendering)
    }

    pub fn has_unfinished_jobs(&self) -> bool {
        self.is_rendering()
    }

    fn begin_render(self: &Rc<Self>) {
        let only = self
            .selected_job
            .get()
            .filter(|&i| self.jobs.borrow().get(i).is_some_and(|j| j.status == Status::Waiting));
        self.render_only.set(only);
        self.started.set(Some(std::time::Instant::now()));
        self.start_next();
    }

    fn render_all_clicked(self: &Rc<Self>) {
        if self.is_rendering() {
            // The button reads "Stop" while a job runs. The second click of a
            // double click on "Render All" must not stop what the first started.
            if self.started.get().is_some_and(|t| t.elapsed() < Duration::from_millis(700)) {
                return;
            }
            self.cancel_all();
        } else {
            self.begin_render();
        }
    }

    /// Stop the running job and drop the ones waiting.
    pub fn cancel_all(self: &Rc<Self>) {
        self.render_only.set(None);
        for job in self.jobs.borrow().iter().filter(|j| j.status == Status::Rendering) {
            job.cancel.store(true, Ordering::Relaxed);
        }
        for job in self.jobs.borrow_mut().iter_mut().filter(|j| j.status == Status::Waiting) {
            job.status = Status::Cancelled;
        }
        self.rebuild_queue();
    }

    pub fn render_all(self: &Rc<Self>) {
        if !self.is_rendering() {
            self.begin_render();
        }
    }

    /// Jobs run one at a time on a worker thread.
    fn start_next(self: &Rc<Self>) {
        let next = {
            let mut jobs = self.jobs.borrow_mut();
            let idx = match self.render_only.get() {
                Some(i) => jobs.get(i).is_some_and(|j| j.status == Status::Waiting).then_some(i),
                None => jobs.iter().position(|j| j.status == Status::Waiting),
            };
            idx.and_then(|i| {
                jobs.get_mut(i).map(|job| {
                    job.status = Status::Rendering;
                    job.cancel.store(false, Ordering::Relaxed);
                    (i, job.project.clone(), job.settings.clone(), job.progress.clone(), job.cancel.clone())
                })
            })
        };
        let Some((index, project, settings, progress, cancel)) = next else {
            self.render_only.set(None);
            self.rebuild_queue();
            self.report_activity();
            return;
        };
        self.rebuild_queue();

        // Refresh the progress bar a few times a second while this job runs.
        let page = self.clone();
        glib::timeout_add_local(Duration::from_millis(250), move || {
            page.report_activity();
            if page.jobs.borrow().get(index).is_some_and(|j| j.status == Status::Rendering) {
                page.update_progress(index);
                glib::ControlFlow::Continue
            } else {
                glib::ControlFlow::Break
            }
        });

        let page = self.clone();
        glib::MainContext::default().spawn_local(async move {
            let cancel_flag = cancel.clone();
            let settings_output = settings.output.clone();
            let limited = settings.max_video_kbps.is_some();
            let result = gio::spawn_blocking(move || {
                export_timeline(&project, &settings, &cancel_flag, |p| progress.store((p * 1000.0) as u32, Ordering::Relaxed))
                    .map(|()| std::fs::metadata(&settings.output).map(|m| m.len()).unwrap_or(0))
            })
            .await;
            let limit = FORMATS.iter().find_map(|f| f.max_bytes).unwrap_or(u64::MAX);
            let (status, toast) = match result {
                Ok(Ok(size)) if limited && size > limit => (
                    Status::Done,
                    Some(format!("Export finished, but it came out at {:.1} MB, over the 10 MB limit. Try a shorter part.", size as f64 / 1e6)),
                ),
                Ok(Ok(_)) => (Status::Done, Some("Export finished".to_string())),
                _ if cancel.load(Ordering::Relaxed) => (Status::Cancelled, None),
                Ok(Err(e)) => (Status::Failed(e.to_string()), Some("Export failed".to_string())),
                Err(_) => (Status::Failed("The export stopped unexpectedly.".into()), Some("Export failed".to_string())),
            };
            let rendered = status == Status::Done;
            let output = settings_output;
            if let Some(job) = page.jobs.borrow_mut().get_mut(index) {
                job.status = status;
            }
            if let Some(t) = toast {
                page.state.message(t);
            }
            if rendered {
                page.start_upload(output);
            }
            page.start_next();
        });
    }

    fn set_upload_state(self: &Rc<Self>, output: &std::path::Path, state: UploadState) {
        if let Some(plan) = self.jobs.borrow_mut().iter_mut().find(|j| j.settings.output == output).and_then(|j| j.upload.as_mut()) {
            plan.state = state;
        }
        self.rebuild_queue();
    }

    /// Send a rendered job to its plugin on a worker, with the token saved in the keyring.
    fn start_upload(self: &Rc<Self>, output: PathBuf) {
        let Some((uploader, input)) = self.jobs.borrow().iter().find(|j| j.settings.output == output).and_then(|j| j.upload.as_ref().map(|u| (u.uploader.clone(), u.input.clone()))) else { return };
        self.set_upload_state(&output, UploadState::Uploading);
        let page = self.clone();
        glib::MainContext::default().spawn_local(async move {
            let (plugin, function, mut sent) = (uploader.plugin.clone(), uploader.function.clone(), input.clone());
            let result = gio::spawn_blocking(move || match crate::plugins::saved_token(&plugin.id) {
                Some(token) => {
                    sent.token = token;
                    tempo_plugin::run_upload(&plugin, &function, sent).map(Some)
                }
                None => Ok(None),
            })
            .await;
            let state = match result {
                Ok(Ok(Some(out))) => {
                    for message in out.messages.into_iter().take(3) {
                        page.state.message(message);
                    }
                    UploadState::Done(out.url)
                }
                // Nothing saved yet: let the user paste a token in the usual dialog.
                Ok(Ok(None)) => {
                    crate::dialogs::upload(&page.root, &page.state, uploader.clone(), input);
                    UploadState::NeedsToken
                }
                Ok(Err(e)) => UploadState::Failed(e.to_string()),
                Err(_) => UploadState::Failed("The upload stopped unexpectedly.".into()),
            };
            page.set_upload_state(&output, state);
        });
    }

    fn report_activity(&self) {
        let text = self
            .jobs
            .borrow()
            .iter()
            .find(|j| j.status == Status::Rendering)
            .map(|j| format!("Rendering {} — {}%", j.label, j.progress.load(Ordering::Relaxed) / 10));
        if let Some(f) = self.activity.borrow().as_ref() {
            f(text);
        }
    }

    fn update_progress(&self, index: usize) {
        let Some(card) = nth_child(&self.queue_box, index) else { return };
        let fraction = self.jobs.borrow().get(index).map(|j| j.progress.load(Ordering::Relaxed) as f64 / 1000.0).unwrap_or(0.0);
        if let Some(bar) = find_descendant::<gtk::ProgressBar>(&card) {
            bar.set_fraction(fraction);
        }
        if let Some(l) = find_named_label(&card, "percent") {
            l.set_text(&format!("{:.0}%", fraction * 100.0));
        }
    }

    fn refresh_render_button(&self) {
        let jobs = self.jobs.borrow();
        let rendering = jobs.iter().any(|j| j.status == Status::Rendering);
        let waiting = jobs.iter().any(|j| j.status == Status::Waiting);
        let selected_waiting = !rendering && self.selected_job.get().is_some_and(|i| jobs.get(i).is_some_and(|j| j.status == Status::Waiting));
        let label = if rendering {
            "Stop"
        } else if selected_waiting {
            "Render 1"
        } else {
            "Render All"
        };
        self.render_all.set_label(label);
        self.render_all.set_tooltip_text(Some(if rendering {
            "Stop rendering"
        } else if selected_waiting {
            "Render selected job (Ctrl+Shift+Return)"
        } else {
            "Render All (Ctrl+Shift+Return)"
        }));
        self.render_all.set_sensitive(rendering || waiting);
    }

    fn refresh_selection_ui(&self) {
        let selected = self.selected_job.get();
        let mut child = self.queue_box.first_child();
        let mut idx = 0usize;
        while let Some(c) = child {
            if c.has_css_class("job-card") {
                if selected == Some(idx) {
                    c.add_css_class("selected");
                } else {
                    c.remove_css_class("selected");
                }
                idx += 1;
            }
            child = c.next_sibling();
        }
        self.refresh_render_button();
    }

    fn rebuild_queue(self: &Rc<Self>) {
        while let Some(child) = self.queue_box.first_child() {
            self.queue_box.remove(&child);
        }
        let jobs = self.jobs.borrow();
        if self.selected_job.get().is_some_and(|i| i >= jobs.len()) {
            self.selected_job.set(None);
        }
        if jobs.is_empty() {
            let hint = label("No jobs yet.\nPick a shape, then Add to Render Queue.", &["empty-hint"]);
            hint.set_justify(gtk::Justification::Center);
            hint.set_margin_top(24);
            self.queue_box.append(&hint);
        }
        for (index, job) in jobs.iter().enumerate() {
            self.queue_box.append(&self.job_card(index, job));
        }
        drop(jobs);
        self.refresh_render_button();
    }

    fn job_card(self: &Rc<Self>, index: usize, job: &Job) -> gtk::Box {
        let card = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["job-card"]).build();
        if self.selected_job.get() == Some(index) {
            card.add_css_class("selected");
        }
        let click = gtk::GestureClick::builder().button(gtk::gdk::BUTTON_PRIMARY).build();
        let page = self.clone();
        click.connect_pressed(move |gesture, _, x, y| {
            let Some(widget) = gesture.widget() else { return };
            if widget.pick(x, y, gtk::PickFlags::DEFAULT).is_some_and(|hit| {
                hit.ancestor(gtk::Button::static_type()).is_some() || hit.ancestor(gtk::MenuButton::static_type()).is_some()
            }) {
                return;
            }
            gesture.set_state(gtk::EventSequenceState::Claimed);
            page.selected_job.set(Some(index));
            page.refresh_selection_ui();
        });
        card.add_controller(click);

        let header = gtk::Box::builder().css_classes(["job-header"]).build();
        let job_title = label(&format!("Job {}", index + 1), &[]);
        job_title.set_hexpand(true);
        job_title.set_xalign(0.0);
        header.append(&job_title);
        let remove = tool_button("window-close-symbolic", if job.status == Status::Rendering { "Cancel this job" } else { "Remove this job" });
        let page = self.clone();
        remove.connect_clicked(move |_| {
            let rendering = page.jobs.borrow().get(index).is_some_and(|j| j.status == Status::Rendering);
            if rendering {
                if let Some(j) = page.jobs.borrow().get(index) {
                    j.cancel.store(true, Ordering::Relaxed);
                }
            } else if !page.is_rendering() {
                // Indexes are stable while a job runs, so only remove when idle.
                page.jobs.borrow_mut().remove(index);
                if let Some(sel) = page.selected_job.get() {
                    if sel == index {
                        page.selected_job.set(None);
                    } else if sel > index {
                        page.selected_job.set(Some(sel - 1));
                    }
                }
                page.rebuild_queue();
            } else {
                page.state.message("Wait for the running job to finish, or stop it first.");
            }
        });
        header.append(&remove);
        card.append(&header);

        let body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).margin_top(8).margin_bottom(8).margin_start(10).margin_end(10).build();
        let name = label(&job.label, &["tempo-heading", "tempo-bright"]);
        name.set_xalign(0.0);
        name.set_ellipsize(gtk::pango::EllipsizeMode::End);
        body.append(&name);
        let detail = label(&job.format, &["tempo-small", "tempo-dim"]);
        detail.set_xalign(0.0);
        body.append(&detail);
        let path = label(&job.settings.output.to_string_lossy(), &["tempo-small", "tempo-dim"]);
        path.set_xalign(0.0);
        path.set_ellipsize(gtk::pango::EllipsizeMode::Start);
        path.set_tooltip_text(Some(&job.settings.output.to_string_lossy()));
        body.append(&path);

        if let (Some(plan), true) = (&job.upload, matches!(job.status, Status::Waiting | Status::Rendering)) {
            let l = status_line(&format!("↑ Will upload to {}", plan.uploader.name), &["tempo-small", "tempo-dim"]);
            l.set_tooltip_text(Some(&format!("Privacy: {} · Category: {}", plan.input.privacy, plan.input.category)));
            body.append(&l);
        }
        match &job.status {
            Status::Waiting => body.append(&status_line("Waiting", &["tempo-small", "tempo-dim"])),
            Status::Rendering => {
                let fraction = job.progress.load(Ordering::Relaxed) as f64 / 1000.0;
                body.append(&gtk::ProgressBar::builder().fraction(fraction).margin_top(4).build());
                let percent = status_line(&format!("{:.0}%", fraction * 100.0), &["tempo-small", "tempo-dim"]);
                percent.set_widget_name("percent");
                body.append(&percent);
            }
            Status::Done => {
                body.append(&status_line("✓ Completed", &["tempo-small"]));
                if let Some(plan) = &job.upload {
                    body.append(&upload_status(plan));
                }
                body.append(&self.done_actions(index, job));
            }
            Status::Failed(reason) => {
                let l = status_line(&format!("⚠ {reason}"), &["tempo-small", "error-text"]);
                l.set_wrap(true);
                body.append(&l);
                body.append(&self.retry_button(index));
            }
            Status::Cancelled => {
                body.append(&status_line("Cancelled", &["tempo-small", "tempo-dim"]));
                body.append(&self.retry_button(index));
            }
        }
        card.append(&body);
        card
    }

    fn retry_button(self: &Rc<Self>, index: usize) -> gtk::Button {
        let retry = gtk::Button::builder().label("Retry").css_classes(["link-text"]).halign(gtk::Align::Start).build();
        let page = self.clone();
        retry.connect_clicked(move |_| {
            if let Some(j) = page.jobs.borrow_mut().get_mut(index) {
                j.status = Status::Waiting;
                j.progress.store(0, Ordering::Relaxed);
            }
            page.rebuild_queue();
        });
        retry
    }

    fn done_actions(self: &Rc<Self>, index: usize, job: &Job) -> gtk::Box {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        let output = job.settings.output.clone();

        let show = gtk::Button::builder().label("Show in Files").css_classes(["link-text"]).build();
        let file = output.clone();
        show.connect_clicked(move |b| reveal(b, &file));
        row.append(&show);

        if !job.settings.chapters.is_empty() {
            let copy = gtk::Button::builder().label("Copy chapters").css_classes(["link-text"]).build();
            let page = self.clone();
            copy.connect_clicked(move |b| page.copy_job_chapters(b, index));
            row.append(&copy);
        }

        // Share: open the site's upload page and show the file, ready to drag in.
        let share = gtk::MenuButton::builder().label("Share…").css_classes(["link-text"]).build();
        let list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).margin_top(6).margin_bottom(6).margin_start(6).margin_end(6).build();
        for (text, uri) in [
            ("Open YouTube upload page", "https://www.youtube.com/upload"),
            ("Open TikTok upload page", "https://www.tiktok.com/upload"),
            ("Open Instagram", "https://www.instagram.com/"),
        ] {
            let item = gtk::Button::builder().label(text).css_classes(["link-text"]).halign(gtk::Align::Start).build();
            let file = output.clone();
            let popover_owner = share.clone();
            item.connect_clicked(move |b| {
                popover_owner.popdown();
                let window = b.root().and_downcast::<gtk::Window>();
                gtk::UriLauncher::new(uri).launch(window.as_ref(), gio::Cancellable::NONE, |_| {});
                reveal(b, &file);
            });
            list.append(&item);
        }
        // Upload targets from plugins send the file themselves.
        for uploader in self.state.uploaders.borrow().iter().cloned() {
            let item = gtk::Button::builder().label(format!("Upload to {}…", uploader.name)).css_classes(["link-text"]).halign(gtk::Align::Start).build();
            let (state, file, name, popover_owner) = (self.state.clone(), output.clone(), job.name.clone(), share.clone());
            // The values queued with the job, or fresh ones for a job queued without upload.
            let draft = job.upload.as_ref().map(|u| u.input.clone()).unwrap_or_else(|| UploadInput {
                file,
                title: name,
                description: format_chapter_list(&job.settings.chapters),
                privacy: tempo_plugin::PRIVACY[0].into(),
                category: tempo_plugin::CATEGORIES[0].into(),
                ..Default::default()
            });
            item.connect_clicked(move |b| {
                popover_owner.popdown();
                crate::dialogs::upload(b, &state, uploader.clone(), draft.clone());
            });
            list.append(&item);
        }
        share.set_popover(Some(&gtk::Popover::builder().child(&list).build()));
        row.append(&share);
        row
    }

    fn copy_job_chapters(self: &Rc<Self>, widget: &impl IsA<gtk::Widget>, index: usize) {
        let Some((chapters, total)) = self.jobs.borrow().get(index).map(|j| (j.settings.chapters.clone(), j.total_us)) else { return };
        copy_chapters(&self.state, widget, chapters, total);
    }

    /// Copy the chapter list for the current timeline (Ctrl+Alt+M).
    pub fn copy_timeline_chapters(&self, widget: &impl IsA<gtk::Widget>) {
        let Some((chapters, total)) = self.state.with_project(|p| {
            let end = p.timeline.tracks.iter().map(|t| t.duration_us()).max().unwrap_or(0);
            (chapters_from_markers(&p.timeline.markers, 0, end), end)
        }) else {
            return;
        };
        copy_chapters(&self.state, widget, chapters, total);
    }
}

/// Copy the list, first explaining anything YouTube would reject.
fn copy_chapters(state: &Rc<AppState>, widget: &impl IsA<gtk::Widget>, chapters: Vec<Chapter>, total_us: i64) {
    if chapters.is_empty() {
        state.message("No chapters: give a marker a name first (M, then Shift+M).");
        return;
    }
    let text = format_chapter_list(&chapters);
    let clipboard = widget.clipboard();
    let problems = youtube_problems(&chapters, total_us);
    if problems.is_empty() {
        clipboard.set_text(&text);
        state.message("Chapters copied");
        return;
    }
    let dialog = adw::AlertDialog::builder()
        .heading("YouTube may not accept these chapters")
        .body(problems.join("\n"))
        .build();
    dialog.add_responses(&[("cancel", "Cancel"), ("copy", "Copy Anyway")]);
    dialog.set_close_response("cancel");
    let state = state.clone();
    dialog.connect_response(None, move |_, response| {
        if response == "copy" {
            clipboard.set_text(&text);
            state.message("Chapters copied");
        }
    });
    dialog.present(Some(widget));
}

/// One line for a job card: how its upload is going.
fn upload_status(plan: &UploadPlan) -> gtk::Widget {
    let name = &plan.uploader.name;
    match &plan.state {
        UploadState::Waiting => status_line(&format!("↑ Upload to {name} is next"), &["tempo-small", "tempo-dim"]).upcast(),
        UploadState::Uploading => status_line(&format!("Uploading to {name}…"), &["tempo-small", "tempo-dim"]).upcast(),
        UploadState::NeedsToken => status_line(&format!("Upload to {name} needs a token: use the dialog, or Share…"), &["tempo-small", "tempo-dim"]).upcast(),
        UploadState::Failed(e) => {
            let l = status_line(&format!("⚠ Upload to {name} failed: {e}"), &["tempo-small", "error-text"]);
            l.set_wrap(true);
            l.upcast()
        }
        UploadState::Done(Some(url)) => {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            row.append(&status_line("✓ Uploaded:", &["tempo-small"]));
            row.append(&gtk::LinkButton::builder().uri(url).label(url).css_classes(["tempo-small"]).build());
            row.upcast()
        }
        UploadState::Done(None) => status_line(&format!("✓ Uploaded to {name}"), &["tempo-small"]).upcast(),
    }
}

impl UploadForm {
    fn new() -> Self {
        let enabled = gtk::CheckButton::with_label("Upload directly to");
        let pick_model = gtk::StringList::new(&[]);
        let pick = gtk::DropDown::builder().model(&pick_model).visible(false).margin_start(12).margin_end(12).build();
        let title = gtk::Entry::builder().hexpand(true).build();
        let text = gtk::TextView::builder().wrap_mode(gtk::WrapMode::WordChar).top_margin(6).bottom_margin(6).left_margin(8).right_margin(8).build();
        let description = gtk::ScrolledWindow::builder().child(&text).min_content_height(48).css_classes(["card"]).build();
        let thumbnail_on = gtk::CheckButton::with_label("Upload Thumbnail");
        let thumbnail = gtk::Entry::builder().hexpand(true).placeholder_text("Thumbnail path").sensitive(false).build();
        let pick_file = gtk::Button::with_label("…");
        pick_file.set_tooltip_text(Some("Choose a thumbnail image (jpg or png)"));
        pick_file.set_sensitive(false);
        let privacy = gtk::DropDown::from_strings(&["Private", "Unlisted", "Public"]);
        let category = gtk::DropDown::from_strings(&tempo_plugin::CATEGORIES);

        let thumb_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        thumb_row.append(&thumbnail);
        thumb_row.append(&pick_file);
        let fields = gtk::Grid::builder().row_spacing(5).column_spacing(10).margin_top(6).margin_start(12).margin_end(12).visible(false).build();
        let row = |r: i32, name: &str, w: &gtk::Widget| {
            let l = label(name, &["tempo-dim"]);
            l.set_xalign(1.0);
            l.set_valign(gtk::Align::Start);
            fields.attach(&l, 0, r, 1, 1);
            fields.attach(w, 1, r, 1, 1);
        };
        row(0, "Title", title.upcast_ref());
        row(1, "Description", description.upcast_ref());
        fields.attach(&thumbnail_on, 1, 2, 1, 1);
        row(3, "Thumbnail", thumb_row.upcast_ref());
        row(4, "Privacy", privacy.upcast_ref());
        row(5, "Category", category.upcast_ref());

        let section = gtk::Box::builder().orientation(gtk::Orientation::Vertical).margin_top(8).margin_bottom(6).visible(false).build();
        let head = gtk::Box::builder().margin_start(12).margin_end(12).build();
        head.append(&enabled);
        section.append(&head);
        section.append(&pick);
        section.append(&fields);

        let f = fields.clone();
        enabled.connect_toggled(move |c| f.set_visible(c.is_active()));
        let (entry, button) = (thumbnail.clone(), pick_file.clone());
        thumbnail_on.connect_toggled(move |c| {
            entry.set_sensitive(c.is_active());
            button.set_sensitive(c.is_active());
        });
        let entry = thumbnail.clone();
        pick_file.connect_clicked(move |b| {
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
        Self { section, enabled, pick, pick_model, title, description: text, thumbnail_on, thumbnail, privacy, category }
    }
}

fn reveal(widget: &impl IsA<gtk::Widget>, file: &std::path::Path) {
    let window = widget.root().and_downcast::<gtk::Window>();
    gtk::FileLauncher::new(Some(&gio::File::for_path(file))).open_containing_folder(window.as_ref(), gio::Cancellable::NONE, |_| {});
}

fn status_line(text: &str, classes: &[&str]) -> gtk::Label {
    let l = label(text, classes);
    l.set_xalign(0.0);
    l
}

fn nth_child(parent: &gtk::Box, n: usize) -> Option<gtk::Widget> {
    let mut child = parent.first_child();
    for _ in 0..n {
        child = child.and_then(|c| c.next_sibling());
    }
    child
}

fn find_descendant<T: IsA<gtk::Widget>>(root: &gtk::Widget) -> Option<T> {
    let mut child = root.first_child();
    while let Some(c) = child {
        if let Ok(found) = c.clone().downcast::<T>() {
            return Some(found);
        }
        if let Some(found) = find_descendant::<T>(&c) {
            return Some(found);
        }
        child = c.next_sibling();
    }
    None
}

fn job_label(p: &Project) -> String {
    format!("{} | {}", p.name, p.timeline.name)
}

/// With several timelines the timeline name is added so two jobs do not default to one file.
fn default_file_name(p: &Project) -> String {
    let name = if p.timelines.is_empty() { p.name.clone() } else { format!("{} {}", p.name, p.timeline.name) };
    name.replace(['/', '\\'], "-")
}

fn inside_job_card(mut widget: Option<gtk::Widget>) -> bool {
    while let Some(cur) = widget {
        if cur.has_css_class("job-card") {
            return true;
        }
        widget = cur.parent();
    }
    false
}

fn find_named_label(root: &gtk::Widget, name: &str) -> Option<gtk::Label> {
    let mut child = root.first_child();
    while let Some(c) = child {
        if c.widget_name() == name {
            return c.downcast::<gtk::Label>().ok();
        }
        if let Some(found) = find_named_label(&c, name) {
            return Some(found);
        }
        child = c.next_sibling();
    }
    None
}

/// A format item: the frame shape drawn to scale, then ratio, size and use.
fn format_button(f: &Format) -> gtk::ToggleButton {
    let shape = gtk::DrawingArea::builder().content_width(44).content_height(44).halign(gtk::Align::Center).build();
    let (fw, fh) = (f.width as f64, f.height as f64);
    shape.set_draw_func(move |area, cr, w, h| {
        let scale = ((w as f64 - 4.0) / fw).min((h as f64 - 4.0) / fh);
        let (rw, rh) = (fw * scale, fh * scale);
        let c = css_color(area, "tempo_text_dim");
        // The button's own text colour, so the outline brightens when selected.
        #[allow(deprecated)]
        let c = area.parent().map(|p| p.style_context().color()).unwrap_or(c);
        cr.set_source_rgba(c.red() as f64, c.green() as f64, c.blue() as f64, c.alpha() as f64);
        cr.set_line_width(1.5);
        cr.rectangle((w as f64 - rw) / 2.0, (h as f64 - rh) / 2.0, rw, rh);
        let _ = cr.stroke();
    });
    let content = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).build();
    content.append(&shape);
    content.append(&label(f.ratio, &["tempo-heading"]));
    let size = match f.max_bytes {
        Some(_) => "720p or 1080p".to_string(),
        None => format!("{} × {}", f.width, f.height),
    };
    content.append(&label(&size, &["tempo-small"]));
    content.append(&label(f.best_for, &["tempo-small"]));
    let button = gtk::ToggleButton::builder().child(&content).css_classes(["format"]).build();
    button.set_tooltip_text(Some(&format!("{} — best for {}", f.ratio, f.best_for)));
    // Redraw the outline when the selection (and so the text colour) changes.
    button.connect_toggled(move |_| shape.queue_draw());
    button
}
