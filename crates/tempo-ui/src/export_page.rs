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
use tempo_export::{chapters_from_markers, export_timeline, format_chapter_list, youtube_problems, Chapter, Fit, TimelineExport};
use tempo_timeline::Project;

use crate::state::{AppState, Change};
use crate::util::{css_color, label, pill, tool_button};

/// Output shapes. Shown by ratio, with what each suits; no platform logos.
struct Format {
    ratio: &'static str,
    width: u32,
    height: u32,
    best_for: &'static str,
}

const FORMATS: [Format; 5] = [
    Format { ratio: "16:9", width: 1920, height: 1080, best_for: "YouTube, Vimeo" },
    Format { ratio: "16:9 4K", width: 3840, height: 2160, best_for: "YouTube 4K" },
    Format { ratio: "9:16", width: 1080, height: 1920, best_for: "TikTok, Reels, Shorts" },
    Format { ratio: "1:1", width: 1080, height: 1080, best_for: "Instagram feed" },
    Format { ratio: "4:5", width: 1080, height: 1350, best_for: "Instagram portrait" },
];

const QUALITIES: [(&str, u8, u32); 3] = [("High", 18, 192), ("Medium", 23, 160), ("Small file", 28, 128)];

#[derive(Clone, PartialEq)]
enum Status {
    Waiting,
    Rendering,
    Done,
    Failed(String),
    Cancelled,
}

struct Job {
    name: String,
    format: String,
    settings: TimelineExport,
    project: Arc<Project>,
    total_us: i64,
    status: Status,
    progress: Arc<AtomicU32>,
    cancel: Arc<AtomicBool>,
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
    quality: gtk::DropDown,
    fit: gtk::DropDown,
    fit_row: gtk::Box,
    chapters: gtk::CheckButton,
    range: gtk::DropDown,
    format_index: Cell<usize>,
    jobs: RefCell<Vec<Job>>,
    queue_box: gtk::Box,
    render_all: gtk::Button,
    /// One line describing the running job, for the page bar.
    activity: RefCell<Option<Box<dyn Fn(Option<String>)>>>,
}

impl ExportPage {
    pub fn new(state: &Rc<AppState>) -> Rc<Self> {
        // ---- Left: render settings ------------------------------------------
        let title = label("Render Settings", &["panel-title"]);
        title.set_xalign(0.0);

        let strip = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(4).margin_top(8).margin_bottom(4).margin_start(8).margin_end(8).build();
        let strip_scroll = gtk::ScrolledWindow::builder().child(&strip).vscrollbar_policy(gtk::PolicyType::Never).build();

        let file_name = gtk::Entry::builder().hexpand(true).build();
        let location = gtk::Entry::builder().hexpand(true).build();
        let browse = pill("Browse");
        let free_space = label("", &["tempo-small", "tempo-dim"]);
        free_space.set_xalign(0.0);
        let quality = gtk::DropDown::from_strings(&QUALITIES.map(|q| q.0));
        let fit = gtk::DropDown::from_strings(&["Fit (black bars)", "Fill (crop)"]);
        let chapters = gtk::CheckButton::with_label("Chapters from markers");
        chapters.set_active(true);

        let grid = gtk::Grid::builder().row_spacing(8).column_spacing(10).margin_top(10).margin_start(12).margin_end(12).build();
        let row = |r: i32, name: &str, w: &gtk::Widget| {
            let l = label(name, &["tempo-dim"]);
            l.set_xalign(1.0);
            grid.attach(&l, 0, r, 1, 1);
            grid.attach(w, 1, r, 1, 1);
        };
        row(0, "File Name", file_name.upcast_ref());
        let loc_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        loc_box.append(&location);
        loc_box.append(&browse);
        row(1, "Location", loc_box.upcast_ref());
        grid.attach(&free_space, 1, 2, 1, 1);
        row(3, "Quality", quality.upcast_ref());
        // The Fit / Fill choice only matters when the shapes differ.
        let fit_row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let fit_label = label("Shape", &["tempo-dim"]);
        fit_label.set_xalign(1.0);
        fit_row.append(&fit_label);
        fit_row.append(&fit);
        grid.attach(&fit_row, 0, 4, 2, 1);
        grid.attach(&chapters, 1, 5, 1, 1);

        let add = pill("Add to Render Queue");
        add.set_tooltip_text(Some("Add to Render Queue (Ctrl+Return)"));
        add.set_halign(gtk::Align::End);
        add.set_margin_top(16);
        add.set_margin_end(12);
        add.set_margin_bottom(12);

        let left = gtk::Box::builder().orientation(gtk::Orientation::Vertical).width_request(360).css_classes(["tempo-surface", "tempo-sep-right"]).build();
        left.append(&title);
        left.append(&strip_scroll);
        left.append(&grid);
        left.append(&gtk::Box::builder().vexpand(true).build());
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
        let render_all = pill("Render All");
        render_all.set_tooltip_text(Some("Render All (Ctrl+Shift+Return)"));
        render_all.set_halign(gtk::Align::End);
        render_all.set_margin_top(8);
        render_all.set_margin_end(12);
        render_all.set_margin_bottom(12);
        let right = gtk::Box::builder().orientation(gtk::Orientation::Vertical).width_request(360).css_classes(["tempo-surface", "tempo-sep-left"]).build();
        right.append(&queue_title);
        right.append(&gtk::ScrolledWindow::builder().child(&queue_box).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build());
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
            quality,
            fit,
            fit_row,
            chapters,
            range,
            format_index: Cell::new(0),
            jobs: RefCell::new(Vec::new()),
            queue_box,
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
                    p.refresh_settings();
                }
            });
            strip.append(&button);
        }

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
        state.connect(move |change| {
            match change {
                Change::Project => p.load_defaults(),
                // The Share menus list the upload targets of enabled plugins.
                Change::Plugins => p.rebuild_queue(),
                _ => {}
            }
        });
        page.load_defaults();
        page.rebuild_queue();
        page
    }

    pub fn connect_activity(&self, f: impl Fn(Option<String>) + 'static) {
        *self.activity.borrow_mut() = Some(Box::new(f));
    }

    fn load_defaults(self: &Rc<Self>) {
        let name = self.state.with_project(|p| p.name.clone()).unwrap_or_default();
        self.file_name.set_text(&name);
        if self.location.text().is_empty() {
            let dir = glib::user_special_dir(glib::UserDirectory::Videos).unwrap_or_else(glib::home_dir);
            self.location.set_text(&dir.to_string_lossy());
        }
        self.update_free_space();
        self.refresh_settings();
    }

    fn refresh_settings(&self) {
        let f = &FORMATS[self.format_index.get()];
        self.title.set_text(&format!("Render Settings - {} - {} × {}", f.ratio, f.width, f.height));
        // Compare shapes by cross-multiplying; no rounding trouble.
        let same_shape = self
            .state
            .with_project(|p| p.width as u64 * f.height as u64 == p.height as u64 * f.width as u64)
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
        let name = if name.is_empty() { project.name.clone() } else { name };
        let f = &FORMATS[self.format_index.get()];
        let (_, crf, audio_kbps) = QUALITIES[self.quality.selected() as usize];

        // Do not silently replace an existing file or an output already queued.
        let dir = PathBuf::from(self.location.text().as_str());
        let taken = |p: &PathBuf| p.exists() || self.jobs.borrow().iter().any(|j| &j.settings.output == p);
        let mut output = dir.join(format!("{name}.mp4"));
        let mut n = 2;
        while taken(&output) {
            output = dir.join(format!("{name} {n}.mp4"));
            n += 1;
        }

        let chapters = if self.chapters.is_active() { chapters_from_markers(&project.timeline.markers, r0, r1) } else { Vec::new() };
        let settings = TimelineExport {
            width: f.width,
            height: f.height,
            crf,
            audio_kbps,
            fit: if self.fit.selected() == 1 { Fit::Fill } else { Fit::Fit },
            range,
            chapters,
            output,
            hardware: self.state.settings.borrow().hardware_encode,
        };
        self.jobs.borrow_mut().push(Job {
            name,
            format: format!("{} · {} × {}", f.ratio, f.width, f.height),
            settings,
            // A copy of the project as it is now: later edits do not change this job.
            project: Arc::new(project),
            total_us: r1 - r0,
            status: Status::Waiting,
            progress: Arc::new(AtomicU32::new(0)),
            cancel: Arc::new(AtomicBool::new(false)),
        });
        self.rebuild_queue();
        true
    }

    fn is_rendering(&self) -> bool {
        self.jobs.borrow().iter().any(|j| j.status == Status::Rendering)
    }

    pub fn has_unfinished_jobs(&self) -> bool {
        self.is_rendering()
    }

    fn render_all_clicked(self: &Rc<Self>) {
        if self.is_rendering() {
            // The button reads "Stop" while a job runs.
            for job in self.jobs.borrow().iter().filter(|j| j.status == Status::Rendering) {
                job.cancel.store(true, Ordering::Relaxed);
            }
            for job in self.jobs.borrow_mut().iter_mut().filter(|j| j.status == Status::Waiting) {
                job.status = Status::Cancelled;
            }
        } else {
            self.start_next();
        }
    }

    pub fn render_all(self: &Rc<Self>) {
        if !self.is_rendering() {
            self.start_next();
        }
    }

    /// Jobs run one at a time on a worker thread.
    fn start_next(self: &Rc<Self>) {
        let next = {
            let mut jobs = self.jobs.borrow_mut();
            jobs.iter_mut().enumerate().find(|(_, j)| j.status == Status::Waiting).map(|(i, job)| {
                job.status = Status::Rendering;
                job.cancel.store(false, Ordering::Relaxed);
                (i, job.project.clone(), job.settings.clone(), job.progress.clone(), job.cancel.clone())
            })
        };
        let Some((index, project, settings, progress, cancel)) = next else {
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
            let result = gio::spawn_blocking(move || {
                export_timeline(&project, &settings, &cancel_flag, |p| progress.store((p * 1000.0) as u32, Ordering::Relaxed))
            })
            .await;
            let (status, toast) = match result {
                Ok(Ok(())) => (Status::Done, Some("Export finished".to_string())),
                _ if cancel.load(Ordering::Relaxed) => (Status::Cancelled, None),
                Ok(Err(e)) => (Status::Failed(e.to_string()), Some("Export failed".to_string())),
                Err(_) => (Status::Failed("The export stopped unexpectedly.".into()), Some("Export failed".to_string())),
            };
            if let Some(job) = page.jobs.borrow_mut().get_mut(index) {
                job.status = status;
            }
            if let Some(t) = toast {
                page.state.message(t);
            }
            page.start_next();
        });
    }

    fn report_activity(&self) {
        let text = self
            .jobs
            .borrow()
            .iter()
            .find(|j| j.status == Status::Rendering)
            .map(|j| format!("Rendering {} — {}%", j.name, j.progress.load(Ordering::Relaxed) / 10));
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

    fn rebuild_queue(self: &Rc<Self>) {
        while let Some(child) = self.queue_box.first_child() {
            self.queue_box.remove(&child);
        }
        let jobs = self.jobs.borrow();
        if jobs.is_empty() {
            let hint = label("No jobs yet.\nPick a shape, then Add to Render Queue.", &["empty-hint"]);
            hint.set_justify(gtk::Justification::Center);
            hint.set_margin_top(24);
            self.queue_box.append(&hint);
        }
        for (index, job) in jobs.iter().enumerate() {
            self.queue_box.append(&self.job_card(index, job));
        }
        let rendering = jobs.iter().any(|j| j.status == Status::Rendering);
        let waiting = jobs.iter().any(|j| j.status == Status::Waiting);
        self.render_all.set_label(if rendering { "Stop" } else { "Render All" });
        self.render_all.set_sensitive(rendering || waiting);
    }

    fn job_card(self: &Rc<Self>, index: usize, job: &Job) -> gtk::Box {
        let card = gtk::Box::builder().orientation(gtk::Orientation::Vertical).css_classes(["job-card"]).build();

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
                page.rebuild_queue();
            } else {
                page.state.message("Wait for the running job to finish, or stop it first.");
            }
        });
        header.append(&remove);
        card.append(&header);

        let body = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).margin_top(8).margin_bottom(8).margin_start(10).margin_end(10).build();
        let name = label(&job.name, &["tempo-heading", "tempo-bright"]);
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
            let description = format_chapter_list(&job.settings.chapters);
            item.connect_clicked(move |b| {
                popover_owner.popdown();
                crate::dialogs::upload(b, &state, uploader.clone(), file.clone(), &name, &description);
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
    content.append(&label(&format!("{} × {}", f.width, f.height), &["tempo-small"]));
    content.append(&label(f.best_for, &["tempo-small"]));
    let button = gtk::ToggleButton::builder().child(&content).css_classes(["format"]).build();
    button.set_tooltip_text(Some(&format!("{} — best for {}", f.ratio, f.best_for)));
    // Redraw the outline when the selection (and so the text colour) changes.
    button.connect_toggled(move |_| shape.queue_draw());
    button
}
