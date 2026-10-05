//! Main window: Loading, Project Manager, and the open project (Edit and Export pages).

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gtk4 as gtk;
use gtk4::prelude::*;
use gtk4::gio;
use libadwaita as adw;
use libadwaita::prelude::*;
use tempo_timeline::{Project, TrimEdge};

use crate::actions::{self, Place};
use crate::dialogs;
use crate::edit_page::EditPage;
use crate::export_page::ExportPage;
use crate::keybinds;
use crate::project_manager::{self, ProjectManager};
use crate::proxies::Proxies;
use crate::state::{AppState, Change, Tool};
use crate::util::{label, labelled_toggle, tool_button};
use crate::viewer::Viewer;
use crate::waveforms::Waveforms;

const AUTOSAVE_SECONDS: u32 = 120;

pub struct MainWindow {
    pub window: adw::ApplicationWindow,
    state: Rc<AppState>,
    root: gtk::Stack,
    toasts: adw::ToastOverlay,
    project_view: adw::ToolbarView,
    pages: gtk::Stack,
    viewer: Rc<Viewer>,
    edit: Rc<EditPage>,
    export: Rc<ExportPage>,
    proxies: Rc<Proxies>,
    _waveforms: Rc<Waveforms>,
    manager: std::cell::RefCell<Option<Rc<ProjectManager>>>,
    project_name: gtk::Label,
    edited: gtk::Label,
    activity: gtk::Label,
    page_buttons: [gtk::ToggleButton; 2],
    panel_buttons: [gtk::ToggleButton; 3],
    edit_only: gtk::Box,
    speed: Cell<f64>,
    cinema: Cell<bool>,
    loading_status: gtk::Label,
}

impl MainWindow {
    pub fn new(app: &adw::Application) -> Rc<Self> {
        let state = AppState::new();
        let viewer = Viewer::new(&state);
        let edit = EditPage::new(&state, &viewer);
        let export = ExportPage::new(&state);
        edit.viewer_slot.append(&viewer.root);

        // ---- Top bar ---------------------------------------------------------
        let media_btn = labelled_toggle("folder-symbolic", "Media Pool", "Show or hide the Media Pool");
        let effects_btn = labelled_toggle("emblem-favorite-symbolic", "Effects", "Show or hide Effects");
        let inspector_btn = labelled_toggle("document-properties-symbolic", "Inspector", "Show or hide the Inspector");
        media_btn.set_active(true);
        let quick_export = gtk::Button::builder().tooltip_text("Quick Export (Ctrl+Shift+E): render with the current export settings").css_classes(["tool"]).build();
        let qe = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        qe.append(&gtk::Image::from_icon_name("document-send-symbolic"));
        qe.append(&gtk::Label::new(Some("Quick Export")));
        quick_export.set_child(Some(&qe));

        let project_name = label("", &["project-name"]);
        let edited = label("", &["tempo-dim"]);
        let title = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        title.append(&project_name);
        title.append(&edited);

        let menu = gio::Menu::new();
        let file = gio::Menu::new();
        file.append(Some("Import Media…"), Some("win.import-media"));
        file.append(Some("Save"), Some("win.save"));
        menu.append_section(None, &file);
        let edit_menu = gio::Menu::new();
        edit_menu.append(Some("Undo"), Some("win.undo"));
        edit_menu.append(Some("Redo"), Some("win.redo"));
        edit_menu.append(Some("Copy Chapter List"), Some("win.copy-chapters"));
        menu.append_section(None, &edit_menu);
        let help = gio::Menu::new();
        help.append(Some("Keyboard Shortcuts"), Some("win.show-shortcuts"));
        help.append(Some("About Tempo"), Some("win.about"));
        help.append(Some("Quit"), Some("win.quit"));
        menu.append_section(None, &help);
        let menu_btn = gtk::MenuButton::builder().icon_name("view-more-symbolic").tooltip_text("Menu").menu_model(&menu).css_classes(["tool"]).build();

        // Buttons that only make sense on the Edit page.
        let edit_only = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        edit_only.append(&quick_export);
        edit_only.append(&inspector_btn);
        let header = adw::HeaderBar::builder().title_widget(&title).build();
        header.pack_start(&media_btn);
        header.pack_start(&effects_btn);
        header.pack_end(&menu_btn);
        header.pack_end(&edit_only);

        // ---- Pages and page bar -----------------------------------------------
        let pages = gtk::Stack::new();
        pages.add_named(&edit.root, Some("edit"));
        pages.add_named(&export.root, Some("export"));

        let page_button = |icon: &str, text: &str, tip: &str| {
            let content = gtk::Box::builder().spacing(8).halign(gtk::Align::Center).build();
            content.append(&gtk::Image::from_icon_name(icon));
            content.append(&gtk::Label::new(Some(text)));
            gtk::ToggleButton::builder().child(&content).tooltip_text(tip).css_classes(["page"]).build()
        };
        let edit_page_btn = page_button("document-edit-symbolic", "Edit", "Edit page (Shift+4)");
        let export_page_btn = page_button("send-to-symbolic", "Export", "Export page (Shift+8)");
        export_page_btn.set_group(Some(&edit_page_btn));
        edit_page_btn.set_active(true);
        let page_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        page_box.append(&edit_page_btn);
        page_box.append(&export_page_btn);

        let activity = label("", &["tempo-small", "tempo-dim"]);
        let left = gtk::Box::builder().spacing(14).margin_start(12).build();
        let logo_box = gtk::Box::builder().spacing(8).build();
        let logo_icon = gtk::Image::builder()
            .icon_name("dev.tempo.Tempo-symbolic")
            .pixel_size(16)
            .build();
        logo_box.append(&logo_icon);
        logo_box.append(&label("Tempo", &["tempo-heading"]));
        left.append(&logo_box);
        left.append(&activity);
        let home = tool_button("go-home-symbolic", "Project Manager (Shift+1)");
        home.set_margin_end(8);
        let page_bar = gtk::CenterBox::builder().css_classes(["page-bar"]).build();
        page_bar.set_start_widget(Some(&left));
        page_bar.set_center_widget(Some(&page_box));
        page_bar.set_end_widget(Some(&home));

        let project_view = adw::ToolbarView::builder().content(&pages).build();
        project_view.add_top_bar(&header);
        // A one-time hint for first-time users, shown under the top bar.
        let hint = adw::Banner::builder()
            .title("Drop clips into the Media Pool, then drag them to the timeline. Space plays, B is the blade, Shift+Backspace deletes and closes the gap. F1 lists every key.")
            .button_label("Got it")
            .build();
        hint.connect_button_clicked(|banner| {
            banner.set_revealed(false);
            glib::MainContext::default().spawn_local(async {
                let _ = gio::spawn_blocking(|| {
                    let path = hint_marker();
                    if let Some(dir) = path.parent() {
                        let _ = std::fs::create_dir_all(dir);
                    }
                    let _ = std::fs::write(path, "seen");
                })
                .await;
            });
        });
        project_view.add_top_bar(&hint);
        let hint_for_start = hint.clone();
        glib::MainContext::default().spawn_local(async move {
            let seen = gio::spawn_blocking(|| hint_marker().exists()).await.unwrap_or(true);
            hint_for_start.set_revealed(!seen);
        });
        project_view.add_bottom_bar(&page_bar);

        // ---- Loading -----------------------------------------------------------
        let loading_status = label("STARTING UP", &["tempo-small", "tempo-dim"]);
        loading_status.set_xalign(0.0);
        let loading = loading_view(&loading_status);

        let root = gtk::Stack::new();
        root.add_named(&gtk::Box::builder().css_classes(["loading"]).build(), Some("blank"));
        root.add_named(&loading, Some("loading"));
        root.add_named(&project_view, Some("project"));

        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&root));
        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("Tempo")
            .default_width(1366)
            .default_height(768)
            .width_request(1024)
            .height_request(640)
            .content(&toasts)
            .css_classes(["tempo"])
            .build();
        edit.media_pool.set_window(&window);

        let state_for_proxies = state.clone();
        let win = Rc::new(Self {
            window,
            state,
            root,
            toasts,
            project_view,
            pages,
            viewer,
            edit,
            export,
            proxies: Proxies::new(&state_for_proxies),
            _waveforms: Waveforms::new(&state_for_proxies),
            manager: Default::default(),
            project_name,
            edited,
            activity,
            page_buttons: [edit_page_btn, export_page_btn],
            panel_buttons: [media_btn, effects_btn, inspector_btn],
            edit_only,
            speed: Cell::new(0.0),
            cinema: Cell::new(false),
            loading_status,
        });

        // Project Manager needs callbacks into the window, so it is built last.
        let w = win.clone();
        let w2 = win.clone();
        let manager = ProjectManager::new(move |path| w.open_project(path), move || w2.new_project());
        win.root.add_named(&manager.root, Some("projects"));
        *win.manager.borrow_mut() = Some(manager);

        win.connect_signals(&home, &quick_export);
        win.install_actions();
        let w = win.clone();
        keybinds::install(&win.window, move |action| w.run_action(action));
        win.start();
        win
    }

    fn connect_signals(self: &Rc<Self>, home: &gtk::Button, quick_export: &gtk::Button) {
        let w = self.clone();
        self.panel_buttons[0].connect_toggled(move |b| w.edit.set_media_pool_open(b.is_active()));
        let w = self.clone();
        self.panel_buttons[1].connect_toggled(move |b| w.edit.set_effects_open(b.is_active()));
        let w = self.clone();
        self.panel_buttons[2].connect_toggled(move |b| w.edit.set_inspector_open(b.is_active()));

        let w = self.clone();
        self.page_buttons[0].connect_toggled(move |b| {
            if b.is_active() {
                w.show_page("edit");
            }
        });
        let w = self.clone();
        self.page_buttons[1].connect_toggled(move |b| {
            if b.is_active() {
                w.show_page("export");
            }
        });

        let w = self.clone();
        home.connect_clicked(move |_| w.run_action("app.project-manager"));
        let w = self.clone();
        quick_export.connect_clicked(move |_| w.run_action("win.quick-export"));

        let w = self.clone();
        self.export.connect_activity(move |text| w.activity.set_text(text.as_deref().unwrap_or("")));
        // Export progress takes the status line; proxy progress shows when no export runs.
        let w = self.clone();
        self.proxies.connect_activity(move |text| {
            if !w.export.has_unfinished_jobs() {
                w.activity.set_text(text.as_deref().unwrap_or(""));
            }
        });

        let w = self.clone();
        self.state.connect(move |change| match change {
            Change::Message(text) => w.toasts.add_toast(adw::Toast::builder().title(text.as_str()).timeout(3).build()),
            Change::Project | Change::Dirty => w.refresh_title(),
            _ => {}
        });

        // Autosave: only when something changed, and on a worker thread.
        let w = self.clone();
        glib::timeout_add_seconds_local(AUTOSAVE_SECONDS, move || {
            w.autosave();
            glib::ControlFlow::Continue
        });

        let w = self.clone();
        self.window.connect_close_request(move |_| w.on_close_request());

        // Escape leaves the full-screen viewer.
        let keys = gtk::EventControllerKey::new();
        let w = self.clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape && w.cinema.get() {
                w.set_cinema(false);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        self.window.add_controller(keys);
    }

    /// Menu items are window actions that forward to `run_action`.
    fn install_actions(self: &Rc<Self>) {
        for name in ["import-media", "save", "undo", "redo", "copy-chapters", "show-shortcuts", "about", "quit"] {
            let action = gio::SimpleAction::new(name, None);
            let w = self.clone();
            let full = match name {
                "save" | "quit" => format!("app.{name}"),
                _ => format!("win.{name}"),
            };
            action.connect_activate(move |_, _| w.run_action(&full));
            self.window.add_action(&action);
        }
    }

    // ---- Start-up ----------------------------------------------------------------

    fn start(self: &Rc<Self>) {
        self.root.set_visible_child_name("blank");
        // Show the loading screen only if start-up is not instant.
        let w = self.clone();
        glib::timeout_add_local_once(Duration::from_millis(300), move || {
            if w.root.visible_child_name().as_deref() == Some("blank") {
                w.root.set_visible_child_name("loading");
            }
        });

        let w = self.clone();
        glib::MainContext::default().spawn_local(async move {
            w.loading_status.set_text("STARTING MEDIA ENGINE");
            let _ = gio::spawn_blocking(tempo_media::ensure_ffmpeg_init).await;
            w.loading_status.set_text("OPENING PROJECTS");
            match std::env::var_os("TEMPO_OPEN").map(PathBuf::from) {
                Some(path) => w.open_project(path),
                None => w.show_manager(),
            }
        });
    }

    fn show_manager(self: &Rc<Self>) {
        if let Some(m) = self.manager.borrow().as_ref() {
            m.reload();
        }
        self.window.set_title(Some("Tempo"));
        self.root.set_visible_child_name("projects");
    }

    // ---- Projects ------------------------------------------------------------------

    fn new_project(self: &Rc<Self>) {
        let w = self.clone();
        dialogs::new_project(&self.window, move |np| {
            let w = w.clone();
            glib::MainContext::default().spawn_local(async move {
                let path = np.path.clone();
                let created = gio::spawn_blocking(move || -> Result<Project, String> {
                    if let Some(dir) = np.path.parent() {
                        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
                    }
                    let mut project =
                        tempo_project::create_new_project(&np.name, np.width, np.height, np.fps, &np.path).map_err(|e| e.to_string())?;
                    project.proxy_dir = glib::user_cache_dir().join("tempo").join("proxies");
                    project_manager::remember(&project.name, &np.path);
                    Ok(project)
                })
                .await;
                match created {
                    Ok(Ok(project)) => w.enter_project(project, path),
                    Ok(Err(e)) => w.state.message(format!("Could not create the project: {e}")),
                    Err(_) => w.state.message("Could not create the project."),
                }
            });
        });
    }

    fn open_project(self: &Rc<Self>, path: PathBuf) {
        let w = self.clone();
        glib::MainContext::default().spawn_local(async move {
            // A newer autosave means the last session did not end cleanly.
            let check = path.clone();
            let autosave = gio::spawn_blocking(move || tempo_project::check_autosave_recovery(&check)).await.ok().flatten();
            match autosave {
                Some(auto) => {
                    let w2 = w.clone();
                    let path2 = path.clone();
                    dialogs::confirm(
                        &w.window,
                        "Restore unsaved changes?",
                        "Tempo found changes that were not saved the last time this project was open.",
                        "Restore",
                        "Discard",
                        move |restore| w2.load_project(path2.clone(), restore.then(|| auto.clone())),
                    );
                }
                None => w.load_project(path, None),
            }
        });
    }

    /// Load `path`, or the autosave `from` if given (the project still saves to `path`).
    fn load_project(self: &Rc<Self>, path: PathBuf, from: Option<PathBuf>) {
        let w = self.clone();
        glib::MainContext::default().spawn_local(async move {
            let read_from = from.clone().unwrap_or_else(|| path.clone());
            let remember_path = path.clone();
            let loaded = gio::spawn_blocking(move || {
                tempo_project::load_project(&read_from).map(|mut project| {
                    for source in project.sources.values_mut() {
                        source.is_missing = !source.path.exists();
                    }
                    project_manager::remember(&project.name, &remember_path);
                    project
                })
            })
            .await;
            match loaded {
                Ok(Ok(project)) => {
                    let missing = project.sources.values().filter(|s| s.is_missing).count();
                    w.enter_project(project, path);
                    if from.is_some() {
                        w.state.set_dirty(true);
                    }
                    if missing > 0 {
                        dialogs::relink_missing(&w.window, &w.state);
                    }
                }
                Ok(Err(e)) => {
                    w.state.message(format!("Could not open the project: {e}"));
                    w.show_manager();
                }
                Err(_) => w.show_manager(),
            }
        });
    }

    fn enter_project(self: &Rc<Self>, project: Project, path: PathBuf) {
        self.state.set_project(Some(project), Some(path));
        self.viewer.show_source(None);
        self.show_page("edit");
        self.page_buttons[0].set_active(true);
        self.root.set_visible_child_name("project");
        self.refresh_title();
        let canvas = self.edit.timeline.canvas.clone();
        glib::idle_add_local_once(move || canvas.zoom_fit());
    }

    fn refresh_title(&self) {
        let name = self.state.with_project(|p| p.name.clone()).unwrap_or_default();
        self.project_name.set_text(&name);
        self.edited.set_text(if self.state.dirty.get() { "Edited" } else { "" });
        if !name.is_empty() {
            self.window.set_title(Some(&format!("{name} — Tempo")));
        }
    }

    /// Save on a worker thread. `then` runs on the GTK thread once the file is written.
    fn save(self: &Rc<Self>, then: impl FnOnce() + 'static) {
        let (Some(project), Some(path)) = (self.state.project.borrow().clone(), self.state.path.borrow().clone()) else {
            then();
            return;
        };
        let w = self.clone();
        glib::MainContext::default().spawn_local(async move {
            let result = gio::spawn_blocking(move || {
                let saved = tempo_project::save_project(&project, &path);
                if saved.is_ok() {
                    tempo_project::remove_autosave(&path);
                }
                saved
            })
            .await;
            match result {
                Ok(Ok(())) => {
                    w.state.set_dirty(false);
                    w.flash_activity("Saved");
                    then();
                }
                Ok(Err(e)) => w.state.message(format!("Could not save: {e}")),
                Err(_) => w.state.message("Could not save."),
            }
        });
    }

    fn autosave(self: &Rc<Self>) {
        if !self.state.dirty.get() {
            return;
        }
        let (Some(project), Some(path)) = (self.state.project.borrow().clone(), self.state.path.borrow().clone()) else { return };
        let w = self.clone();
        glib::MainContext::default().spawn_local(async move {
            match gio::spawn_blocking(move || tempo_project::auto_save_project(&project, &path)).await {
                Ok(Ok(_)) => w.flash_activity("Autosaved"),
                Ok(Err(e)) => tracing::warn!("autosave failed: {e}"),
                Err(_) => {}
            }
        });
    }

    /// Show a short status in the page bar, unless an export is reporting there.
    fn flash_activity(self: &Rc<Self>, text: &'static str) {
        if self.export.has_unfinished_jobs() {
            return;
        }
        self.activity.set_text(text);
        let w = self.clone();
        glib::timeout_add_seconds_local_once(3, move || {
            if w.activity.text() == text {
                w.activity.set_text("");
            }
        });
    }

    fn on_close_request(self: &Rc<Self>) -> glib::Propagation {
        if self.export.has_unfinished_jobs() {
            let w = self.clone();
            dialogs::confirm(&self.window, "An export is running", "Closing now stops it.", "Close Anyway", "Keep Open", move |yes| {
                if yes {
                    w.state.player.shutdown();
                    w.window.destroy();
                }
            });
            return glib::Propagation::Stop;
        }
        if self.state.dirty.get() {
            let w = self.clone();
            self.save(move || {
                w.state.player.shutdown();
                w.window.destroy();
            });
            return glib::Propagation::Stop;
        }
        self.state.player.shutdown();
        glib::Propagation::Proceed
    }

    // ---- Pages ---------------------------------------------------------------------

    fn show_page(&self, name: &str) {
        let (from, to) = if name == "export" {
            (&self.edit.viewer_slot, &self.export.viewer_slot)
        } else {
            (&self.export.viewer_slot, &self.edit.viewer_slot)
        };
        // One viewer, moved between the pages, so only one picture is ever decoded.
        let root = &self.viewer.root;
        if root.parent().as_ref() != Some(to.upcast_ref()) {
            if root.parent().is_some() {
                from.remove(root);
            }
            to.append(root);
        }
        if name == "export" {
            self.viewer.show_source(None);
        }
        self.pages.set_visible_child_name(name);
        let is_edit = name == "edit";
        self.edit_only.set_visible(is_edit);
        self.panel_buttons[0].set_visible(is_edit);
        self.panel_buttons[1].set_visible(is_edit);
    }

    fn set_cinema(&self, on: bool) {
        self.cinema.set(on);
        self.project_view.set_reveal_top_bars(!on);
        self.project_view.set_reveal_bottom_bars(!on);
        self.edit.set_cinema(on);
        if on {
            self.window.fullscreen();
        } else {
            self.window.unfullscreen();
            self.edit.set_inspector_open(self.panel_buttons[2].is_active());
        }
    }

    /// Playback figures for automated checks.
    pub fn stats(&self) -> String {
        let p = &self.state.player;
        format!("position_us={} dropped_frames={} playing={}", p.position_us(), p.dropped_frames(), p.is_playing())
    }

    fn in_project(&self) -> bool {
        self.root.visible_child_name().as_deref() == Some("project")
    }

    // ---- Actions (keys, menu, buttons) -----------------------------------------------

    pub fn run_action(self: &Rc<Self>, action: &str) {
        let state = &self.state;
        let player = &state.player;
        match action {
            "app.quit" => {
                self.window.close();
                return;
            }
            "win.show-shortcuts" => {
                dialogs::shortcuts(&self.window);
                return;
            }
            "win.about" => {
                adw::AboutDialog::builder()
                    .application_name("Tempo")
                    .version(env!("CARGO_PKG_VERSION"))
                    .comments("A light video editor for Linux that works like DaVinci Resolve's Edit page.")
                    .license_type(gtk::License::Gpl30)
                    .build()
                    .present(Some(&self.window));
                return;
            }
            _ => {}
        }
        if !self.in_project() {
            return;
        }
        let on_edit = self.pages.visible_child_name().as_deref() == Some("edit");
        match action {
            // Playback
            "win.play-toggle" => {
                self.speed.set(0.0);
                self.viewer.toggle_play();
            }
            "win.play-forward" | "win.play-reverse" => {
                // Pressing the same direction again doubles the speed, up to 8×.
                let dir = if action == "win.play-forward" { 1.0 } else { -1.0 };
                let cur = self.speed.get();
                let next = if player.is_playing() && cur * dir > 0.0 { (cur.abs() * 2.0).min(8.0) * dir } else { dir };
                self.speed.set(next);
                player.play(next);
            }
            "win.stop" => {
                self.speed.set(0.0);
                player.pause();
            }
            "win.step-forward" => actions::step_frames(state, 1),
            "win.step-reverse" => actions::step_frames(state, -1),
            "win.large-step-forward" => actions::step_frames(state, state.fps().round() as i64),
            "win.large-step-reverse" => actions::step_frames(state, -(state.fps().round() as i64)),
            "win.clip-next" => actions::goto_edit(state, true),
            "win.clip-prev" => actions::goto_edit(state, false),
            "win.timeline-start" => player.seek(0, true),
            "win.timeline-end" => player.seek(player.duration_us(), true),

            // In and Out
            "win.mark-in" => actions::set_mark(state, true, false),
            "win.mark-out" => actions::set_mark(state, false, false),
            "win.clear-in" => actions::set_mark(state, true, true),
            "win.clear-out" => actions::set_mark(state, false, true),
            "win.clear-in-out" => {
                actions::set_mark(state, true, true);
                actions::set_mark(state, false, true);
            }
            "win.mark-clip" => actions::mark_clip(state),
            "win.goto-in" | "win.goto-out" => {
                let source = state.source_clip.get().is_some();
                let mark = match (source, action == "win.goto-in") {
                    (true, true) => state.src_in.get(),
                    (true, false) => state.src_out.get(),
                    (false, true) => state.mark_in.get(),
                    (false, false) => state.mark_out.get(),
                };
                if let Some(us) = mark {
                    player.seek(us, true);
                }
            }

            // Markers
            "win.marker-add" => {
                actions::add_marker(state);
            }
            "win.marker-add-modify" => {
                if let Some(m) = actions::add_marker(state) {
                    dialogs::edit_marker(&self.window, state, m);
                }
            }
            "win.marker-modify" => match actions::marker_at_playhead(state) {
                Some(m) => dialogs::edit_marker(&self.window, state, m),
                None => state.message("Move the playhead onto a marker first (Shift+Up / Shift+Down)."),
            },
            "win.marker-delete" => {
                if let Some(m) = actions::marker_at_playhead(state) {
                    actions::delete_marker(state, m.id);
                }
            }
            "win.marker-next" => actions::goto_marker(state, true),
            "win.marker-prev" => actions::goto_marker(state, false),
            "win.copy-chapters" => self.export.copy_timeline_chapters(&self.window),

            // Pages and files
            "app.project-manager" => {
                let w = self.clone();
                self.save(move || {
                    w.state.set_project(None, None);
                    w.show_manager();
                });
            }
            "win.page-edit" => self.page_buttons[0].set_active(true),
            "win.page-export" => self.page_buttons[1].set_active(true),
            "app.save" => self.save(|| {}),
            "win.undo" => state.undo(),
            "win.redo" => state.redo(),
            "win.quick-export" => {
                if self.export.add_to_queue() {
                    self.export.render_all();
                    state.message("Exporting with the current export settings");
                }
            }
            "win.queue-add" if !on_edit => {
                self.export.add_to_queue();
            }
            "win.queue-render-all" if !on_edit => self.export.render_all(),
            "win.viewer-cinema" => self.set_cinema(!self.cinema.get()),

            // Everything below edits the timeline and belongs to the Edit page.
            _ if !on_edit => {}
            "win.import-media" => self.edit.media_pool.choose_files(),
            "win.tool-select" => self.set_tool(Tool::Select),
            "win.tool-trim" => self.set_tool(Tool::Trim),
            "win.tool-blade" => self.set_tool(Tool::Blade),
            "win.snap-toggle" => {
                state.snapping.set(!state.snapping.get());
                state.emit(Change::Options);
            }
            "win.edit-insert" => actions::place_current(state, Place::Insert),
            "win.edit-overwrite" => actions::place_current(state, Place::Overwrite),
            "win.edit-place-on-top" => actions::place_current(state, Place::OnTop),
            "win.edit-append" => actions::place_current(state, Place::Append),
            "win.clip-enable-toggle" => actions::toggle_enabled(state),
            "win.copy" => actions::copy_selected(state, false),
            "win.cut" => actions::copy_selected(state, true),
            "win.paste" => actions::paste(state),
            "win.edit-replace" => actions::replace_selected(state),
            "win.razor" => actions::razor(state),
            "win.split-clip" => actions::split_selected(state),
            "win.delete" => actions::delete_selected(state, false),
            "win.ripple-delete" => actions::delete_selected(state, true),
            "win.trim-start" => actions::trim_to_playhead(state, TrimEdge::In),
            "win.trim-end" => actions::trim_to_playhead(state, TrimEdge::Out),
            "win.nudge-reverse" => actions::nudge(state, -1),
            "win.nudge-forward" => actions::nudge(state, 1),
            "win.nudge-multi-left" => actions::nudge(state, -5),
            "win.nudge-multi-right" => actions::nudge(state, 5),
            "win.clip-move-up" => actions::move_track(state, 1),
            "win.clip-move-down" => actions::move_track(state, -1),
            "win.zoom-in" => {
                let c = &self.edit.timeline.canvas;
                c.set_pps(c.pps() * 1.4, None);
            }
            "win.zoom-out" => {
                let c = &self.edit.timeline.canvas;
                c.set_pps(c.pps() / 1.4, None);
            }
            "win.zoom-fit" => self.edit.timeline.canvas.zoom_fit(),
            "win.viewer-toggle" => {
                // Back to the timeline, or to the clip selected in the Media Pool.
                let target = if state.source_clip.get().is_some() { None } else { state.media_selection.get() };
                self.viewer.show_source(target);
            }
            "win.focus-media-pool" => {
                self.panel_buttons[0].set_active(true);
                self.edit.media_pool.focus();
            }
            "win.focus-timeline" => {
                self.edit.timeline.canvas.grab_focus();
            }
            "win.focus-inspector" => {
                self.panel_buttons[2].set_active(true);
                self.edit.inspector.root.child_focus(gtk::DirectionType::TabForward);
            }
            other => tracing::debug!("no handler for action {other}"),
        }
    }

    fn set_tool(&self, tool: Tool) {
        if self.state.tool.replace(tool) != tool {
            self.state.emit(Change::Options);
        }
    }
}

/// A file whose presence means the first-use hint was dismissed.
fn hint_marker() -> PathBuf {
    glib::user_config_dir().join("tempo").join("first-use-hint-seen")
}

fn loading_view(status: &gtk::Label) -> gtk::Box {
    let name = label("Tempo", &["tempo-title"]);
    name.set_xalign(0.0);
    let version = label(env!("CARGO_PKG_VERSION"), &["tempo-dim"]);
    version.set_xalign(0.0);
    let progress = gtk::ProgressBar::builder().width_request(280).halign(gtk::Align::Start).build();
    progress.set_pulse_step(0.15);
    // Animate only while the loading screen is on screen, then stop for good.
    let pulsing = progress.clone();
    let was_mapped = Cell::new(false);
    glib::timeout_add_local(Duration::from_millis(120), move || {
        if pulsing.is_mapped() {
            was_mapped.set(true);
            pulsing.pulse();
            glib::ControlFlow::Continue
        } else if was_mapped.get() || pulsing.root().is_none() {
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
    let column = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .valign(gtk::Align::Center)
        .margin_start(96)
        .build();
    let app_icon = gtk::Image::builder()
        .icon_name("dev.tempo.Tempo")
        .pixel_size(64)
        .halign(gtk::Align::Start)
        .margin_bottom(12)
        .build();
    column.append(&app_icon);
    column.append(&name);
    column.append(&version);
    column.append(&gtk::Box::builder().height_request(40).build());
    column.append(status);
    column.append(&progress);
    let view = gtk::Box::builder().css_classes(["loading"]).build();
    view.append(&column);
    view
}
