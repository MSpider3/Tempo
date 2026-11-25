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
use crate::plugins::Plugins;
use crate::project_manager::{self, ProjectManager};
use crate::proxies::Proxies;
use crate::state::{AppState, Change, Tool};
use crate::util::{label, labelled_toggle, tool_button};
use crate::viewer::Viewer;
use crate::waveforms::Waveforms;

/// How often the autosave timer looks at the clock; the interval itself is a setting.
const AUTOSAVE_CHECK_SECONDS: u32 = 30;

pub struct MainWindow {
    pub window: adw::ApplicationWindow,
    pub(crate) state: Rc<AppState>,
    root: gtk::Stack,
    toasts: adw::ToastOverlay,
    project_view: adw::ToolbarView,
    pages: gtk::Stack,
    viewer: Rc<Viewer>,
    /// The second viewer of dual-viewer mode, made the first time it is switched on.
    source_viewer: std::cell::RefCell<Option<Rc<Viewer>>>,
    pub(crate) edit: Rc<EditPage>,
    export: Rc<ExportPage>,
    proxies: Rc<Proxies>,
    plugins: Rc<Plugins>,
    /// The part of the main menu that lists plugin commands.
    plugin_menu: gio::Menu,
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
    full_viewer: Cell<bool>,
    /// When the last rolling backup was started, and whether a failure was already reported.
    last_backup: Cell<std::time::Instant>,
    backup_warned: Cell<bool>,
    splash: Rc<crate::splash::SplashScreen>,
    splash_skipped: Rc<Cell<bool>>,
    splash_finish: Rc<std::cell::RefCell<Option<Rc<dyn Fn()>>>>,
    checklist: Rc<crate::help::FirstEditChecklist>,
}

impl MainWindow {
    pub fn new(app: &adw::Application) -> Rc<Self> {
        let state = AppState::new();
        let viewer = Viewer::new(&state);
        let edit = EditPage::new(&state, &viewer);
        let export = ExportPage::new(&state);
        edit.viewer_slot.append(&viewer.root);
        hint_over(&state, &viewer.root, crate::keybinds::HintZone::Viewer);
        hint_over(&state, &edit.media_pool.root, crate::keybinds::HintZone::MediaPool);

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
        file.append(Some("Relink Missing Media…"), Some("win.relink-media"));
        file.append(Some("Save"), Some("win.save"));
        file.append(Some("Save As…"), Some("win.save-as"));
        file.append(Some("Project Settings…"), Some("win.project-settings"));
        menu.append_section(None, &file);
        let edit_menu = gio::Menu::new();
        edit_menu.append(Some("Undo"), Some("win.undo"));
        edit_menu.append(Some("Redo"), Some("win.redo"));
        edit_menu.append(Some("Copy Chapter List"), Some("win.copy-chapters"));
        menu.append_section(None, &edit_menu);
        // Filled with the commands of enabled plugins; see `refresh_plugin_menu`.
        let plugin_menu = gio::Menu::new();
        menu.append_section(Some("Plugins"), &plugin_menu);
        let help = gio::Menu::new();
        help.append(Some("How Do I…? Guide"), Some("win.how-do-i"));
        help.append(Some("First-Edit Checklist"), Some("win.show-checklist"));
        help.append(Some("Keyboard Shortcuts"), Some("win.show-shortcuts"));
        help.append(Some("Plugins…"), Some("win.plugins"));
        help.append(Some("Preferences"), Some("win.preferences"));
        help.append(Some("About Tempo"), Some("win.about"));
        help.append(Some("Quit"), Some("win.quit"));
        menu.append_section(None, &help);
        let menu_btn = gtk::MenuButton::builder().icon_name("view-more-symbolic").tooltip_text("Menu").menu_model(&menu).css_classes(["tool"]).build();
        let how_do_i_btn = gtk::Button::builder()
            .label("?")
            .tooltip_text("How Do I…? Beginner Guide")
            .css_classes(["tool", "panel-help-btn"])
            .build();
        how_do_i_btn.update_property(&[gtk::accessible::Property::Label("How Do I…? Guide")]);
        how_do_i_btn.set_action_name(Some("win.how-do-i"));
        let checklist = crate::help::FirstEditChecklist::new(&state);

        // Buttons that only make sense on the Edit page.
        let edit_only = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        edit_only.append(&quick_export);
        edit_only.append(&inspector_btn);
        let header = adw::HeaderBar::builder().title_widget(&title).build();
        header.pack_start(&media_btn);
        header.pack_start(&effects_btn);
        header.pack_end(&menu_btn);
        header.pack_end(&how_do_i_btn);
        header.pack_end(&checklist.button);
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
        let settings_btn = tool_button("emblem-system-symbolic", "Project Settings (Shift+9)");
        settings_btn.set_margin_end(8);
        // Shortcut hints for what is under the pointer, from the keybind table.
        let hint = label("", &["tempo-small", "tempo-dim"]);
        hint.set_ellipsize(gtk::pango::EllipsizeMode::End);
        hint.set_xalign(1.0);
        // Takes the free space beside the page switcher without pushing it off centre.
        hint.set_max_width_chars(8);
        hint.set_hexpand(true);
        let right = gtk::Box::builder().spacing(6).margin_start(24).build();
        right.append(&hint);
        right.append(&home);
        right.append(&settings_btn);
        let binds = crate::keybinds::load();
        let shown = Cell::new(None);
        *state.show_hint.borrow_mut() = Some(Rc::new(move |zone| {
            if shown.replace(zone) != zone {
                hint.set_text(&zone.map(|z| crate::keybinds::hint(z, &binds, crate::keybinds::display)).unwrap_or_default());
            }
        }));
        let page_bar = gtk::CenterBox::builder().css_classes(["page-bar"]).build();
        page_bar.set_start_widget(Some(&left));
        page_bar.set_center_widget(Some(&page_box));
        page_bar.set_end_widget(Some(&right));

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
        let splash_skipped = Rc::new(Cell::new(false));
        let splash_finish = Rc::new(std::cell::RefCell::new(None::<Rc<dyn Fn()>>));
        let splash = crate::splash::SplashScreen::new({
            let skipped = splash_skipped.clone();
            let finish = splash_finish.clone();
            move || {
                skipped.set(true);
                let cb = finish.borrow_mut().take();
                if let Some(cb) = cb {
                    cb();
                }
            }
        });

        let root = gtk::Stack::new();
        root.add_named(&gtk::Box::builder().css_classes(["loading"]).build(), Some("blank"));
        root.add_named(&splash.root, Some("loading"));
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
            source_viewer: Default::default(),
            edit,
            export,
            proxies: Proxies::new(&state_for_proxies),
            plugins: Plugins::new(&state_for_proxies),
            plugin_menu,
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
            full_viewer: Cell::new(false),
            last_backup: Cell::new(std::time::Instant::now()),
            backup_warned: Cell::new(false),
            splash,
            splash_skipped,
            splash_finish,
            checklist,
        });

        // Project Manager needs callbacks into the window, so it is built last.
        let w = win.clone();
        let w2 = win.clone();
        let manager = ProjectManager::new(move |path| w.open_project(path), move |folder| w2.new_project(folder));
        win.root.add_named(&manager.root, Some("projects"));
        *win.manager.borrow_mut() = Some(manager);

        win.connect_signals(&home, &settings_btn, &quick_export);
        win.install_actions();
        let w = win.clone();
        keybinds::install(&win.window, move |action| {
            // Outside a project only a few keys mean anything; the rest are left
            // for the Project Manager's own list and fields.
            let used = w.in_project() || matches!(action, "app.quit" | "win.show-shortcuts" | "app.preferences");
            if used {
                w.run_action(action);
            }
            used
        });
        let splash_keys = gtk::EventControllerKey::new();
        splash_keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let w = win.clone();
        splash_keys.connect_key_pressed(move |_, key, _, _| {
            use gtk::gdk::Key;
            let modifier = matches!(
                key,
                Key::Shift_L
                    | Key::Shift_R
                    | Key::Control_L
                    | Key::Control_R
                    | Key::Alt_L
                    | Key::Alt_R
                    | Key::Super_L
                    | Key::Super_R
                    | Key::Meta_L
                    | Key::Meta_R
                    | Key::Caps_Lock
                    | Key::Num_Lock
            );
            if w.root.visible_child_name().as_deref() == Some("loading") && !modifier {
                w.splash_skipped.set(true);
                let cb = w.splash_finish.borrow_mut().take();
                if let Some(cb) = cb {
                    cb();
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        win.window.add_controller(splash_keys);

        win.start();
        win
    }

    fn connect_signals(self: &Rc<Self>, home: &gtk::Button, settings_btn: &gtk::Button, quick_export: &gtk::Button) {
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
        settings_btn.connect_clicked(move |_| w.run_action("win.project-settings"));
        let w = self.clone();
        quick_export.connect_clicked(move |_| w.run_action("win.quick-export"));

        let w = self.clone();
        self.export.connect_activity(move |text| {
            // An export wants the whole processor: proxy work waits for it.
            w.proxies.set_busy(text.is_some());
            w.activity.set_text(text.as_deref().unwrap_or(""));
        });
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
        let waited = Cell::new(0u32);
        glib::timeout_add_seconds_local(AUTOSAVE_CHECK_SECONDS, move || {
            waited.set(waited.get() + AUTOSAVE_CHECK_SECONDS);
            if waited.get() >= w.state.settings.borrow().autosave_seconds {
                waited.set(0);
                w.autosave();
            }
            w.backup_if_due();
            glib::ControlFlow::Continue
        });

        // Where a Media Pool clip opens depends on the viewer mode.
        let w = self.clone();
        *self.state.open_source.borrow_mut() = Some(Rc::new(move |source| {
            let source_viewer = w.source_viewer.borrow().clone();
            match source_viewer.filter(|_| w.state.dual_viewer.get()) {
                Some(v) => v.show_source(source),
                None => w.viewer.show_source(source),
            }
        }));
        let w = self.clone();
        self.viewer.connect_dual(move |on| w.set_dual_viewer(on));

        let w = self.clone();
        self.window.connect_close_request(move |_| w.on_close_request());

        // Escape leaves the full-screen viewer, else lets go of a selected transition or edit point.
        let keys = gtk::EventControllerKey::new();
        let w = self.clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape && w.cinema.get() {
                w.set_cinema(false);
                return glib::Propagation::Stop;
            }
            if key == gtk::gdk::Key::Escape && w.state.edit_selection.get().is_some() {
                w.state.select_edit(None);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        self.window.add_controller(keys);
    }

    /// Menu items are window actions that forward to `run_action`.
    fn install_actions(self: &Rc<Self>) {
        // Plugin commands carry "plugin-id::command-id" as the action's parameter.
        let run_plugin = gio::SimpleAction::new("run-plugin", Some(glib::VariantTy::STRING));
        let w = self.clone();
        run_plugin.connect_activate(move |_, target| {
            if let Some(target) = target.and_then(|t| t.str()) {
                if w.in_project() {
                    w.plugins.run(target);
                }
            }
        });
        self.window.add_action(&run_plugin);
        // Right-click menus name the action to run, the same names the keys use.
        let run = gio::SimpleAction::new("do", Some(glib::VariantTy::STRING));
        let w = self.clone();
        run.connect_activate(move |_, target| {
            if let Some(action) = target.and_then(|t| t.str()) {
                w.run_action(action);
            }
        });
        self.window.add_action(&run);
        let w = self.clone();
        self.plugins.connect_changed(move || w.refresh_plugin_menu());
        self.refresh_plugin_menu();

        for name in [
            "import-media",
            "relink-media",
            "save",
            "save-as",
            "project-settings",
            "undo",
            "redo",
            "copy-chapters",
            "how-do-i",
            "show-checklist",
            "clip-attributes",
            "plugins",
            "preferences",
            "show-shortcuts",
            "about",
            "quit",
        ] {
            let action = gio::SimpleAction::new(name, None);
            let w = self.clone();
            let full = match name {
                "save" | "save-as" | "quit" | "preferences" => format!("app.{name}"),
                _ => format!("win.{name}"),
            };
            action.connect_activate(move |_, _| w.run_action(&full));
            self.window.add_action(&action);
        }
    }

    // ---- Start-up ----------------------------------------------------------------

    fn start(self: &Rc<Self>) {
        let force_splash = std::env::var_os("TEMPO_SHOW_SPLASH").is_some();
        let automated = std::env::var_os("TEMPO_SMOKE_TEST").is_some()
            || std::env::var_os("TEMPO_TEST_CONTROL").is_some()
            || std::env::var_os("TEMPO_ACTIONS").is_some()
            || std::env::var_os("TEMPO_SCREENSHOT").is_some()
            || std::env::var_os("TEMPO_OPEN").is_some();
        let started_at = std::time::Instant::now();

        if force_splash {
            self.splash.activate();
            self.splash.set_progress(0.15);
            self.splash.set_status("STARTING UP");
            self.root.set_visible_child_name("loading");
        } else {
            self.root.set_visible_child_name("blank");
            // Show the loading screen only if start-up is not instant.
            let w = self.clone();
            glib::timeout_add_local_once(Duration::from_millis(300), move || {
                if w.root.visible_child_name().as_deref() == Some("blank") {
                    w.splash.activate();
                    w.root.set_visible_child_name("loading");
                }
            });
        }

        let w = self.clone();
        glib::MainContext::default().spawn_local(async move {
            w.splash.set_status("STARTING MEDIA ENGINE");
            w.splash.set_progress(0.35);
            let (settings, hardware, turned_off) = gio::spawn_blocking(|| {
                tempo_media::ensure_ffmpeg_init();
                let mut settings = crate::settings::Settings::load();
                let hardware = (tempo_media::hardware_decode_available(), tempo_export::ffmpeg::vaapi_h264_available());
                // A saved setting the machine can't do is switched off, as Resolve resets
                // an invalid GPU configuration, and the user is told once.
                let turned_off = (settings.hardware_decode && !hardware.0) || (settings.hardware_encode && !hardware.1);
                if turned_off {
                    settings.hardware_decode &= hardware.0;
                    settings.hardware_encode &= hardware.1;
                    if let Err(e) = settings.save() {
                        tracing::warn!("could not save settings: {e}");
                    }
                }
                (settings, hardware, turned_off)
            })
            .await
            .unwrap_or_default();
            w.state.hardware.set(hardware);
            if turned_off {
                w.state.message("Hardware video acceleration isn't available on this computer, so it was turned off.");
            }
            w.state.player.set_max_height(settings.playback_height);
            tempo_media::set_hardware_decode(settings.hardware_decode);
            let show_splash = force_splash || (!automated && settings.show_splash);
            *w.state.settings.borrow_mut() = settings;
            // The viewer shows the saved preview quality.
            w.state.emit(Change::Options);
            w.plugins.reload();
            w.splash.set_status("OPENING PROJECTS");
            w.splash.set_progress(0.75);

            let open_target = std::env::var_os("TEMPO_OPEN").map(PathBuf::from);
            let completed = Rc::new(Cell::new(false));
            let finish: Rc<dyn Fn()> = {
                let w = w.clone();
                let completed = completed.clone();
                Rc::new(move || {
                    if completed.replace(true) {
                        return;
                    }
                    *w.splash_finish.borrow_mut() = None;
                    w.splash.release();
                    match open_target.as_ref() {
                        Some(path) => w.open_project(path.clone()),
                        None => w.show_manager(),
                    }
                })
            };

            if show_splash && !w.splash_skipped.get() {
                w.splash.set_progress(1.0);
                w.splash.set_status("READY — CLICK OR PRESS ANY KEY TO CONTINUE");
                if w.root.visible_child_name().as_deref() != Some("loading") {
                    w.splash.activate();
                    w.root.set_visible_child_name("loading");
                }
                *w.splash_finish.borrow_mut() = Some(finish.clone());
                let hold_ms: u64 = if force_splash && automated {
                    20_000
                } else if force_splash {
                    4_500
                } else {
                    2_400
                };
                let remaining_ms = hold_ms.saturating_sub(started_at.elapsed().as_millis() as u64);
                let finish_timer = finish;
                glib::timeout_add_local_once(Duration::from_millis(remaining_ms.max(150)), move || {
                    finish_timer();
                });
            } else {
                finish();
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

    /// New Project dialog; the project is filed in `folder` of the Project Manager.
    fn new_project(self: &Rc<Self>, folder: Option<String>) {
        let w = self.clone();
        glib::MainContext::default().spawn_local(async move {
            let existing_names = gio::spawn_blocking(|| {
                project_manager::load_list().into_iter().map(|e| e.name).collect::<Vec<_>>()
            })
            .await
            .unwrap_or_default();
            let w2 = w.clone();
            dialogs::new_project(&w.window, existing_names, move |np| {
                let w = w2.clone();
                let folder = folder.clone();
                glib::MainContext::default().spawn_local(async move {
                    let created = gio::spawn_blocking(move || -> Result<(Project, PathBuf), String> {
                        let mut target_path = np.path.clone();
                        if let Some(dir) = target_path.parent().map(std::path::Path::to_path_buf) {
                            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                            if target_path.exists() {
                                let stem = target_path
                                    .file_stem()
                                    .map(|s| s.to_string_lossy().into_owned())
                                    .unwrap_or_else(|| "Project".into());
                                let mut n = 2;
                                while target_path.exists() {
                                    target_path = dir.join(format!("{stem} {n}.tempo"));
                                    n += 1;
                                }
                            }
                        }
                        let mut project =
                            tempo_project::create_new_project(&np.name, np.width, np.height, np.fps, &target_path)
                                .map_err(|e| e.to_string())?;
                        project.proxy_dir = glib::user_cache_dir().join("tempo").join("proxies");
                        project_manager::remember_in(&project.name, &target_path, folder.as_deref());
                        Ok((project, target_path))
                    })
                    .await;
                    match created {
                        Ok(Ok((project, path))) => w.enter_project(project, path),
                        Ok(Err(e)) => w.state.message(format!("Could not create the project: {e}")),
                        Err(_) => w.state.message("Could not create the project."),
                    }
                });
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
                        move |restore| {
                            if !restore {
                                // Discarded for good, so the question is not asked again next time.
                                let stale = path2.clone();
                                glib::MainContext::default().spawn_local(async move {
                                    let _ = gio::spawn_blocking(move || tempo_project::remove_autosave(&stale)).await;
                                });
                            }
                            w2.load_project(path2.clone(), restore.then(|| auto.clone()));
                        },
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
            let loaded = gio::spawn_blocking(move || -> Result<Project, String> {
                // Opening a path that is not there would create an empty file at it.
                if !read_from.is_file() {
                    return Err("its file was moved or deleted. Right-click it in the list to remove it.".into());
                }
                tempo_project::load_project(&read_from)
                    .map(|mut project| {
                        for source in project.sources.values_mut() {
                            source.is_missing = !source.path.exists();
                        }
                        project_manager::remember(&project.name, &remember_path);
                        project
                    })
                    .map_err(|e| e.to_string())
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
                    w.state.message(format!("Could not open the project: {e}. Right-click it and choose Project Backups… to open an earlier copy."));
                    w.show_manager();
                }
                Err(_) => w.show_manager(),
            }
        });
    }

    fn enter_project(self: &Rc<Self>, project: Project, path: PathBuf) {
        self.full_viewer.set(false);
        if let Some(lower) = self.edit.timeline.root.parent() {
            lower.set_visible(true);
        }
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
        self.save_then(false, then);
    }

    /// Save the open project to a chosen path (`Ctrl+Shift+S`).
    fn save_as(self: &Rc<Self>) {
        let current_path = self.state.path.borrow().clone();
        let Some(current_name) = self.state.with_project(|p| p.name.clone()) else { return };
        let w = self.clone();
        let old_path = current_path.clone();
        dialogs::save_project_as(&self.window, current_path.as_deref(), &current_name, move |new_path| {
            let Some(mut project) = w.state.project.borrow().clone() else { return };
            let w2 = w.clone();
            let path_for_state = new_path.clone();
            let old_path = old_path.clone();
            glib::MainContext::default().spawn_local(async move {
                let res = gio::spawn_blocking(move || -> Result<(String, uuid::Uuid), String> {
                    let (name, id) =
                        project_manager::save_as_identity(&project.name, project.id, old_path.as_deref(), &new_path)?;
                    project.name = name;
                    project.id = id;
                    if let Some(dir) = new_path.parent() {
                        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
                    }
                    tempo_project::save_project(&project, &new_path).map_err(|e| e.to_string())?;
                    tempo_project::remove_autosave(&new_path);
                    project_manager::remember(&project.name, &new_path);
                    project_manager::save_project_thumbnail(&project);
                    Ok((project.name, project.id))
                })
                .await;
                match res {
                    Ok(Ok((saved_name, saved_id))) => {
                        *w2.state.path.borrow_mut() = Some(path_for_state);
                        if let Some(p) = w2.state.project.borrow_mut().as_mut() {
                            p.name = saved_name;
                            p.id = saved_id;
                        }
                        w2.state.set_dirty(false);
                        w2.state.emit(Change::Project);
                        w2.flash_activity("Saved");
                    }
                    Ok(Err(e)) => w2.state.message(format!("Could not save: {e}")),
                    Err(_) => w2.state.message("Could not save: the save stopped unexpectedly"),
                }
            });
        });
    }

    /// Save, then do `then`. With `leaving` (closing the window or the project),
    /// a failed save asks whether to leave anyway instead of trapping the user
    /// in a project that cannot be written.
    fn save_then(self: &Rc<Self>, leaving: bool, then: impl FnOnce() + 'static) {
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
                    project_manager::save_project_thumbnail(&project);
                }
                saved
            })
            .await;
            let problem = match result {
                Ok(Ok(())) => {
                    w.state.set_dirty(false);
                    w.flash_activity("Saved");
                    then();
                    return;
                }
                Ok(Err(e)) => e.to_string(),
                Err(_) => "the save stopped unexpectedly".to_string(),
            };
            if !leaving {
                w.state.message(format!("Could not save: {problem}"));
                return;
            }
            // Escape and Enter both mean "stay": leaving loses work, so it takes a click.
            let dialog = adw::AlertDialog::builder()
                .heading("The project could not be saved")
                .body(format!("{problem}\n\nIf you leave now, the changes since the last save are lost."))
                .build();
            dialog.add_responses(&[("leave", "Leave Without Saving"), ("stay", "Stay")]);
            dialog.set_response_appearance("leave", adw::ResponseAppearance::Destructive);
            dialog.set_default_response(Some("stay"));
            dialog.set_close_response("stay");
            let then = std::cell::RefCell::new(Some(then));
            dialog.connect_response(Some("leave"), move |_, _| {
                if let Some(then) = then.borrow_mut().take() {
                    then();
                }
            });
            dialog.present(Some(&w.window));
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

    /// Every few minutes of editing, keep a checked copy of the project (Resolve's
    /// Project Backups). Runs on a worker; a failure is reported once and never blocks.
    fn backup_if_due(self: &Rc<Self>) {
        let every = std::time::Duration::from_secs(tempo_project::BackupPolicy::default().every_min as u64 * 60);
        if self.last_backup.get().elapsed() < every || !self.state.changed_since_backup.get() {
            return;
        }
        let Some(project) = self.state.project.borrow().clone() else { return };
        self.last_backup.set(std::time::Instant::now());
        self.state.changed_since_backup.set(false);
        let dir = crate::project_manager::backup_dir(project.id);
        let w = self.clone();
        glib::MainContext::default().spawn_local(async move {
            let result = gio::spawn_blocking(move || tempo_project::backup_now(&project, &dir)).await;
            let problem = match result {
                Ok(Ok(path)) => {
                    tracing::debug!("project backup written to {}", path.display());
                    return;
                }
                Ok(Err(e)) => e.to_string(),
                Err(_) => "the backup stopped unexpectedly".to_string(),
            };
            tracing::warn!("project backup failed: {problem}");
            w.state.changed_since_backup.set(true);
            if !w.backup_warned.replace(true) {
                w.state.message(format!("Could not write a project backup: {problem}"));
            }
        });
    }

    /// Show a short status in the page bar, unless an export is reporting there.
    fn flash_activity(self: &Rc<Self>, text: impl Into<String>) {
        if self.export.has_unfinished_jobs() {
            return;
        }
        let text: String = text.into();
        self.activity.set_text(&text);
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
                    // Stop the export cleanly (its unfinished file is removed),
                    // then save the project like any other close.
                    w.export.cancel_all();
                    let w2 = w.clone();
                    w.save_then(true, move || {
                        w2.state.player.shutdown();
                        w2.window.destroy();
                    });
                }
            });
            return glib::Propagation::Stop;
        }
        if self.state.dirty.get() {
            let w = self.clone();
            self.save_then(true, move || {
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

    fn refresh_plugin_menu(&self) {
        self.plugin_menu.remove_all();
        for (label, target) in self.plugins.commands() {
            let item = gio::MenuItem::new(Some(&label), None);
            item.set_action_and_target_value(Some("win.run-plugin"), Some(&target.to_variant()));
            self.plugin_menu.append_item(&item);
        }
    }

    /// Dual viewer: the source clip gets its own viewer and playback beside the timeline viewer.
    fn set_dual_viewer(self: &Rc<Self>, on: bool) {
        if on == self.state.dual_viewer.get() {
            return;
        }
        // Leave single-viewer source mode before switching.
        self.viewer.show_source(None);
        self.state.dual_viewer.set(on);
        if on && self.source_viewer.borrow().is_none() {
            let player = crate::player::Player::new();
            player.set_max_height(self.state.settings.borrow().playback_height);
            *self.state.source_player.borrow_mut() = Some(player.clone());
            let source = Viewer::new_source(&self.state, player);
            self.edit.source_slot.append(&source.root);
            hint_over(&self.state, &source.root, crate::keybinds::HintZone::Viewer);
            *self.source_viewer.borrow_mut() = Some(source);
        }
        self.edit.source_slot.set_visible(on);
        if let Some(v) = self.source_viewer.borrow().as_ref() {
            v.show_source(if on { self.state.media_selection.get() } else { None });
        }
        self.state.source_clip.set(if on { self.state.media_selection.get() } else { None });
        self.state.emit(Change::Options);
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
            if self.full_viewer.get() {
                if let Some(lower) = self.edit.timeline.root.parent() {
                    lower.set_visible(false);
                }
            }
        }
    }

    /// One step of a scripted check (see `TEMPO_ACTIONS` in app.rs). Steps with a
    /// colon drive the interface the way a pointer would; anything else is an
    /// ordinary action name.
    ///
    /// `seek:SECONDS` · `media:INDEX` · `tool:select|trim|blade` ·
    /// `click:SECONDS,TRACK[,ctrl]` · `drag:SECONDS,TRACK,TO_SECONDS[,TO_TRACK]` ·
    /// `trim:EDGE_SECONDS,TRACK,DELTA_SECONDS[,shift]` · `title:center|lower` ·
    /// `fade:in|out,SECONDS` · `panel:effects|inspector` · `menu:SECONDS,TRACK` ·
    /// `poolmenu:INDEX` · `pick:inspector|quick|effects` · `edit:SECONDS,TRACK` (click a
    /// clip edge below the transition boxes) · `droptransition:KIND,SECONDS,TRACK` · `dump`.
    /// A `click` or `drag` in the middle of a row hits a transition box there.
    pub fn run_script_step(self: &Rc<Self>, step: &str) {
        use gtk::gdk::ModifierType;
        let canvas = &self.edit.timeline.canvas;
        if step == "dump" {
            let selected_transition = actions::selected_transition(&self.state);
            let text = self
                .state
                .with_timeline(|t| {
                    let mut lines = Vec::new();
                    for row in crate::timeline::rows(t) {
                        let Some(track) = t.find_track(row.track_id) else { continue };
                        let clips: Vec<String> = track
                            .clips
                            .iter()
                            .map(|c| {
                                let sel = if self.state.is_selected(c.id) { "*" } else { "" };
                                let off = if c.properties.enabled { "" } else { "(off)" };
                                let p = &c.properties;
                                let fades = match (p.fade_in_us, p.fade_out_us) {
                                    (0, 0) => String::new(),
                                    (i, o) => format!("(fade {:.2}/{:.2})", i as f64 / 1e6, o as f64 / 1e6),
                                };
                                format!("{sel}{}{off}{fades}[{:.2}-{:.2}]", c.name, c.timeline_in as f64 / 1e6, c.timeline_out as f64 / 1e6)
                            })
                            .collect();
                        // Transitions: kind, alignment and span, `*` when selected.
                        let transitions: Vec<String> = track
                            .transitions
                            .iter()
                            .filter_map(|tr| {
                                let (a, b) = track.transition_span(tr)?;
                                let sel = selected_transition.as_ref().is_some_and(|(id, s)| *id == track.id && s.id == tr.id);
                                Some(format!("{}{:?}/{:?}[{:.2}-{:.2}]", if sel { "*" } else { "" }, tr.kind, tr.alignment, a as f64 / 1e6, b as f64 / 1e6))
                            })
                            .collect();
                        let kind = if row.kind == tempo_timeline::TrackKind::Video { "V" } else { "A" };
                        let transitions = if transitions.is_empty() { String::new() } else { format!(" transitions: {}", transitions.join(" ")) };
                        lines.push(format!("{kind}{}: {}{transitions}", row.index, clips.join(" ")));
                        if let Some(crate::state::EditSelection::Edit { track_id, position_us }) = self.state.edit_selection.get() {
                            if track_id == track.id {
                                lines.push(format!("edit: {kind}{} {:.2}", row.index, position_us as f64 / 1e6));
                            }
                        }
                    }
                    let copied = self.state.transition_clipboard.borrow().as_ref().map(|t| format!("{:?} {:.2}", t.kind, t.duration_us as f64 / 1e6));
                    lines.push(format!("copied: {}", copied.unwrap_or_default()));
                    lines.push(format!("markers: {}", t.markers.iter().map(|m| format!("{:.2}", m.position_us as f64 / 1e6)).collect::<Vec<_>>().join(" ")));
                    lines.join(" | ")
                })
                .unwrap_or_default();
            tracing::info!("dump {text}");
            return;
        }
        let Some((op, args)) = step.split_once(':') else {
            self.run_action(step);
            return;
        };
        let parts: Vec<&str> = args.split(',').map(str::trim).collect();
        let num = |i: usize| parts.get(i).and_then(|p| p.parse::<f64>().ok());
        match op {
            "seek" => {
                if let Some(s) = num(0) {
                    self.state.player.seek((s * 1e6) as i64, true);
                }
            }
            "media" => self.edit.media_pool.select_index(num(0).unwrap_or(0.0) as u32),
            "import" => self.edit.media_pool.import(parts.iter().map(PathBuf::from).collect()),
            "tool" => self.set_tool(match parts.first().copied() {
                Some("trim") => Tool::Trim,
                Some("blade") => Tool::Blade,
                _ => Tool::Select,
            }),
            // Click a clip edge low in the row, outside any transition box: selects the edit point.
            "edit" => {
                if let Some(p) = num(0).and_then(|s| canvas.bottom_point_for(s, parts.get(1).copied().unwrap_or("V1"))) {
                    canvas.simulate_drag(p, p, ModifierType::empty());
                }
            }
            "click" => {
                if let Some(p) = num(0).and_then(|s| canvas.point_for(s, parts.get(1).copied().unwrap_or("V1"))) {
                    let mods = if parts.get(2) == Some(&"ctrl") { ModifierType::CONTROL_MASK } else { ModifierType::empty() };
                    canvas.simulate_drag(p, p, mods);
                }
            }
            "hint" => {
                use crate::keybinds::HintZone;
                let zone = match parts.first().copied() {
                    Some("clip") => Some(HintZone::ClipBody),
                    Some("edge") => Some(HintZone::ClipEdge),
                    Some("fade") => Some(HintZone::FadeHandle),
                    Some("ruler") => Some(HintZone::Ruler),
                    Some("viewer") => Some(HintZone::Viewer),
                    Some("media") => Some(HintZone::MediaPool),
                    _ => None,
                };
                self.state.hint(zone);
            }
            "fadedrag" => {
                let track = parts.get(1).copied().unwrap_or("V1");
                if let (Some(from), Some(to)) = (num(0).and_then(|s| canvas.top_point_for(s, track)), num(2).and_then(|s| canvas.top_point_for(s, track))) {
                    canvas.simulate_drag(from, to, ModifierType::empty());
                }
            }
            "drag" => {
                let track = parts.get(1).copied().unwrap_or("V1");
                let from = num(0).and_then(|s| canvas.point_for(s, track));
                let to = num(2).and_then(|s| canvas.point_for(s, parts.get(3).copied().unwrap_or(track)));
                if let (Some(from), Some(to)) = (from, to) {
                    canvas.simulate_drag(from, to, ModifierType::empty());
                }
            }
            "trim" => {
                let track = parts.get(1).copied().unwrap_or("V1");
                let from = num(0).and_then(|s| canvas.point_for(s, track));
                let to = num(0).zip(num(2)).and_then(|(s, d)| canvas.point_for(s + d, track));
                if let (Some(from), Some(to)) = (from, to) {
                    let mods = if parts.get(3) == Some(&"shift") { ModifierType::SHIFT_MASK } else { ModifierType::empty() };
                    canvas.simulate_drag(from, to, mods);
                }
            }
            // Which widget a pointer press in the middle of a top-bar button would
            // reach: the button itself, unless something lies over it.
            "pick" => {
                let button: gtk::Widget = match parts.first().copied() {
                    Some("inspector") => self.panel_buttons[2].clone().upcast(),
                    Some("effects") => self.panel_buttons[1].clone().upcast(),
                    _ => match self.edit_only.first_child() {
                        Some(quick_export) => quick_export,
                        None => return,
                    },
                };
                let centre = gtk::graphene::Point::new(button.width() as f32 / 2.0, button.height() as f32 / 2.0);
                let hit = button
                    .compute_point(&self.window, &centre)
                    .and_then(|p| self.window.pick(p.x() as f64, p.y() as f64, gtk::PickFlags::DEFAULT));
                let reached = hit.as_ref().is_some_and(|w| *w == button || w.is_ancestor(&button));
                tracing::info!("pick {args}: reached={reached} widget={:?} sensitive={} size={}x{}", hit.map(|w| w.type_().name().to_string()), button.is_sensitive(), button.width(), button.height());
            }
            // Open the right-click menu of the timeline, or of a Media Pool clip.
            "menu" => {
                if let Some(p) = num(0).and_then(|s| canvas.point_for(s, parts.get(1).copied().unwrap_or("V1"))) {
                    canvas.show_menu(p.0, p.1);
                }
            }
            "poolmenu" => self.edit.media_pool.show_menu_for_index(num(0).unwrap_or(0.0) as u32),
            // Drop the nth Media Pool clip on a track at a time, as the mouse would.
            "drop" => {
                let source = self.edit.media_pool.id_at(num(0).unwrap_or(0.0) as u32);
                let track = parts.get(2).and_then(|name| {
                    self.state.with_timeline(|t| {
                        crate::timeline::rows(t)
                            .into_iter()
                            .find(|r| format!("{}{}", if r.kind == tempo_timeline::TrackKind::Video { "V" } else { "A" }, r.index).eq_ignore_ascii_case(name))
                            .map(|r| r.track_id)
                    })
                    .flatten()
                });
                if let (Some(source), Some(at)) = (source, num(1)) {
                    actions::place_source(&self.state, source, Place::Overwrite, Some((at * 1e6) as i64), track, true);
                }
            }
            // Open the nth Media Pool clip in the viewer, as a double click would.
            "open" => {
                if let Some(source) = self.edit.media_pool.id_at(num(0).unwrap_or(0.0) as u32) {
                    self.state.show_source(Some(source));
                }
            }
            "dual" => self.set_dual_viewer(parts.first() == Some(&"on")),
            "plugin" => self.plugins.run(args),
            "filter" => {
                let found = self.state.filters.borrow().iter().find(|f| f.id.ends_with(args)).cloned();
                if let Some(filter) = found {
                    actions::add_filter(&self.state, &filter);
                }
            }
            "title" => actions::add_title(&self.state, parts.first() == Some(&"lower")),
            // Drop a transition from the Effects panel at a time on a track, as the mouse would.
            "droptransition" => {
                if let Some((x, y)) = num(1).and_then(|s| canvas.point_for(s, parts.get(2).copied().unwrap_or("V1"))) {
                    let placed = canvas.drop_transition(parts.first().copied().unwrap_or("cross-dissolve"), x, y);
                    tracing::info!("droptransition {args}: placed={placed}");
                }
            }
            "fade" => actions::set_fade(&self.state, parts.first() == Some(&"in"), num(1).unwrap_or(0.5)),
            "panel" => match parts.first().copied() {
                Some("effects") => self.panel_buttons[1].set_active(true),
                Some("inspector") => self.panel_buttons[2].set_active(true),
                _ => {}
            },
            other => tracing::warn!("unknown script step {other}"),
        }
    }

    /// Playback figures for automated checks.
    pub fn stats(&self) -> String {
        let p = &self.state.player;
        format!("position_us={} dropped_frames={} playing={} degraded={} canvas_draws={}", p.position_us(), p.dropped_frames(), p.is_playing(), p.is_degraded() as u8, crate::timeline::draw_count())
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
            "win.how-do-i" => {
                let w = self.clone();
                let w2 = self.clone();
                crate::help::show_how_do_i(
                    &self.window,
                    move || w.checklist.show(),
                    move || dialogs::shortcuts(&w2.window),
                );
                return;
            }
            "win.show-checklist" => {
                self.checklist.show();
                return;
            }
            "app.preferences" => {
                dialogs::preferences(&self.window, &self.state);
                return;
            }
            "win.plugins" => {
                dialogs::plugins(&self.window, &self.plugins);
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
                // Pressing the same direction again doubles the speed, up to 64×.
                let dir = if action == "win.play-forward" { 1.0 } else { -1.0 };
                // Playback started with Space or the on-screen button counts as 1×.
                let cur = match (player.is_playing(), self.speed.get()) {
                    (true, 0.0) => 1.0,
                    (_, speed) => speed,
                };
                // At the end of the timeline, L starts again from the beginning.
                if dir > 0.0 && !player.is_playing() && player.position_us() >= player.duration_us() {
                    player.seek(0, true);
                }
                let next = if player.is_playing() && cur * dir > 0.0 { (cur.abs() * 2.0).min(64.0) * dir } else { dir };
                self.speed.set(next);
                player.play(next);
                if next.abs() > 1.0 {
                    self.flash_activity(format!("Playing at {}{}×", if next < 0.0 { "−" } else { "" }, next.abs()));
                }
            }
            // Shift+L and Shift+J step through the speeds: each press of Shift+L is
            // one step faster forward, each press of Shift+J one step the other way.
            "win.fast-forward" | "win.fast-reverse" => {
                const SPEEDS: [f64; 15] = [-64.0, -32.0, -16.0, -8.0, -4.0, -2.0, -1.0, 0.0, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0];
                let now = if player.is_playing() { self.speed.get() } else { 0.0 };
                // A plain "play" (Space or the button) counts as 1×.
                let now = if player.is_playing() && now == 0.0 { 1.0 } else { now };
                let at = SPEEDS.iter().position(|s| *s == now).unwrap_or(SPEEDS.len() / 2);
                let first_press = !player.is_playing();
                let next = match (action == "win.fast-forward", first_press) {
                    // From a stop, "fast" starts at double speed.
                    (true, true) => 2.0,
                    (false, true) => -2.0,
                    (true, false) => SPEEDS[(at + 1).min(SPEEDS.len() - 1)],
                    (false, false) => SPEEDS[at.saturating_sub(1)],
                };
                self.speed.set(next);
                if next == 0.0 {
                    player.pause();
                    self.flash_activity("Stopped".to_string());
                } else {
                    player.play(next);
                    self.flash_activity(format!("Playing at {}{}×", if next < 0.0 { "−" } else { "" }, next.abs()));
                }
            }
            "win.play-slow" => {
                self.speed.set(0.5);
                player.play(0.5);
                self.flash_activity("Playing at half speed");
            }
            "win.play-again" => {
                self.speed.set(1.0);
                player.play_again();
            }
            "win.play-in-to-out" => {
                if actions::play_in_to_out(state) {
                    self.speed.set(1.0);
                }
            }
            "win.play-around" => {
                if actions::play_around(state) {
                    self.speed.set(1.0);
                }
            }
            "win.loop-toggle" => state.set_loop_playback(!state.loop_playback.get()),
            "win.scrub-audio-toggle" => {
                let on = !state.scrub_audio.get();
                state.scrub_audio.set(on);
                state.emit(Change::Options);
                self.flash_activity(if on { "Audio scrubbing on" } else { "Audio scrubbing off" });
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
            "win.first-frame" => actions::first_or_last_frame(state, false),
            "win.last-frame" => actions::first_or_last_frame(state, true),
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
            "win.mark-selection" => actions::mark_selection(state),
            "win.goto-in" | "win.goto-out" => {
                let source = state.source_mode();
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

            "win.new-timeline" => dialogs::new_timeline(&self.window, state, self.edit.media_pool.current_bin()),
            "win.new-bin" => {
                self.panel_buttons[0].set_active(true);
                self.edit.media_pool.new_bin_dialog();
            }

            // Markers
            "win.marker-add" => {
                if let Some(existing) = actions::marker_at_playhead(state) {
                    dialogs::edit_marker(&self.window, state, existing);
                } else {
                    actions::add_marker(state, false);
                }
            }
            "win.marker-add-modify" => {
                if let Some(m) = actions::add_marker(state, true) {
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
                self.save_then(true, move || {
                    w.state.set_project(None, None);
                    w.show_manager();
                });
            }
            "win.page-edit" => self.page_buttons[0].set_active(true),
            "win.page-export" => self.page_buttons[1].set_active(true),
            "win.project-settings" => dialogs::project_settings(&self.window, state),
            "app.save" => self.save(|| {}),
            "app.save-as" => self.save_as(),
            // Only where the timeline can be seen: an undo nobody sees is a surprise later.
            "win.undo" if on_edit => state.undo(),
            "win.redo" if on_edit => state.redo(),
            "win.relink-media" => {
                if state.with_project(|p| p.sources.values().any(|s| s.is_missing)).unwrap_or(false) {
                    dialogs::relink_missing(&self.window, state);
                } else {
                    state.message("No media is missing.");
                }
            }
            "win.quick-export" => {
                if self.export.add_to_queue() {
                    self.export.render_all();
                    // Show the job and its progress, so it is plain that something started.
                    self.page_buttons[1].set_active(true);
                    state.message("Export started. You can go back to the Edit page while it runs.");
                }
            }
            "win.queue-add" if !on_edit => {
                self.export.add_to_queue();
            }
            "win.queue-render-all" if !on_edit => self.export.render_all(),
            "win.viewer-cinema" => self.set_cinema(!self.cinema.get()),
            "win.viewer-fit" => state.set_viewer_zoom(None),
            "win.viewer-actual-size" => state.set_viewer_zoom(Some(1.0)),

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
            "win.edit-ripple-overwrite" => actions::ripple_overwrite(state),
            "win.edit-place-on-top" => actions::place_current(state, Place::OnTop),
            "win.edit-append" => actions::place_current(state, Place::Append),
            "win.clip-enable-toggle" => actions::toggle_enabled(state),
            // With a transition selected, Ctrl+C / Ctrl+X take only the transition, and
            // with an edit point selected Ctrl+V puts the copied transition there (Resolve).
            "win.copy" => {
                if !actions::copy_transition(state) {
                    actions::copy_selected(state, false);
                }
            }
            "win.cut" => {
                if actions::copy_transition(state) {
                    actions::delete_transition(state);
                } else {
                    actions::copy_selected(state, true);
                }
            }
            "win.paste" => {
                if !actions::paste_transition(state) {
                    actions::paste(state);
                }
            }
            "win.paste-insert" => actions::paste_insert(state),
            "win.edit-replace" => actions::replace_selected(state),
            "win.select-all" => state.select_all(),
            "win.deselect-all" => state.select(None),
            "win.select-track-after" => actions::select_range_relative(state, true, false),
            "win.select-all-after" => actions::select_range_relative(state, true, true),
            "win.select-track-before" => actions::select_range_relative(state, false, false),
            "win.select-all-before" => actions::select_range_relative(state, false, true),
            "win.select-edit-point" => actions::select_nearest_edit(state),
            "win.link-toggle" => actions::toggle_link(state),
            "win.linked-selection-toggle" => {
                state.linked_selection.set(!state.linked_selection.get());
                state.emit(Change::Options);
            }
            "win.transition-add" => actions::add_standard_transitions(state, true, true),
            "win.transition-add-video" => actions::add_standard_transitions(state, true, false),
            "win.transition-add-audio" => actions::add_standard_transitions(state, false, true),
            "win.transition-duration" => self.change_transition_duration(),
            "win.clip-speed" => dialogs::change_clip_speed(&self.window, state),
            "win.clip-attributes" => dialogs::clip_attributes_timeline(&self.window, state),
            "win.fade-in" => actions::set_fade(state, true, actions::STANDARD_TRANSITION_S),
            "win.fade-out" => actions::set_fade(state, false, actions::STANDARD_TRANSITION_S),
            "win.razor" => actions::razor(state),
            "win.split-clip" => actions::split_selected(state),
            "win.join-clip" => actions::join_clip(state),
            // A selected transition is deleted alone, whichever delete key was pressed.
            "win.delete" | "win.ripple-delete" if actions::delete_transition(state) => {}
            "win.delete" => actions::delete_selected(state, false),
            "win.ripple-delete" => actions::delete_selected(state, true),
            "win.trim-start" => actions::trim_to_playhead(state, TrimEdge::In, false),
            "win.trim-end" => actions::trim_to_playhead(state, TrimEdge::Out, false),
            "win.ripple-start" => actions::trim_to_playhead(state, TrimEdge::In, true),
            "win.ripple-end" => actions::trim_to_playhead(state, TrimEdge::Out, true),
            "win.fade-in-to-playhead" => actions::fade_to_playhead(state, true),
            "win.fade-out-to-playhead" => actions::fade_to_playhead(state, false),
            "win.select-at-playhead" => actions::select_at_playhead(state),
            "win.ripple-cut" => {
                actions::copy_selected(state, false);
                actions::delete_selected(state, true);
            }
            "win.focus-effects" => self.panel_buttons[1].set_active(!self.panel_buttons[1].is_active()),
            "win.nudge-reverse" => actions::nudge(state, -1),
            "win.nudge-forward" => actions::nudge(state, 1),
            "win.nudge-multi-left" => actions::nudge(state, -5),
            "win.nudge-multi-right" => actions::nudge(state, 5),
            "win.swap-reverse" => actions::swap_clips(state, false),
            "win.swap-forward" => actions::swap_clips(state, true),
            "win.volume-up-1" => actions::adjust_volume_db(state, 1.0),
            "win.volume-down-1" => actions::adjust_volume_db(state, -1.0),
            "win.volume-up-3" => actions::adjust_volume_db(state, 3.0),
            "win.volume-down-3" => actions::adjust_volume_db(state, -3.0),
            "win.video-track-toggle-1" => actions::toggle_video_track(state, 1),
            "win.video-track-toggle-2" => actions::toggle_video_track(state, 2),
            "win.video-track-toggle-3" => actions::toggle_video_track(state, 3),
            "win.video-track-toggle-4" => actions::toggle_video_track(state, 4),
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
                // With two viewers both are always on screen, so there is nothing to switch.
                if !state.dual_viewer.get() {
                    let target = if state.source_clip.get().is_some() { None } else { state.media_selection.get() };
                    self.viewer.show_source(target);
                }
            }
            "win.viewer-full-page" => {
                let on = !self.full_viewer.get();
                self.full_viewer.set(on);
                if let Some(lower) = self.edit.timeline.root.parent() {
                    lower.set_visible(!on);
                }
            }
            "win.focus-media-pool" => {
                self.panel_buttons[0].set_active(true);
                self.edit.media_pool.focus();
            }
            "win.focus-source-viewer" => {
                let prev_source = state.media_selection.get().or_else(|| state.source_clip.get());
                if !state.dual_viewer.get() {
                    self.set_dual_viewer(true);
                }
                if let Some(sv) = self.source_viewer.borrow().as_ref() {
                    if state.source_clip.get().is_none() && prev_source.is_some() {
                        sv.show_source(prev_source);
                        state.source_clip.set(prev_source);
                    }
                    sv.surface.grab_focus();
                }
            }
            "win.focus-timeline" => {
                self.edit.timeline.canvas.grab_focus();
            }
            "win.focus-timeline-viewer" => {
                if !state.dual_viewer.get() && state.source_mode() {
                    self.viewer.show_source(None);
                }
                self.viewer.surface.grab_focus();
            }
            "win.focus-inspector" => {
                self.panel_buttons[2].set_active(true);
                self.edit.inspector.root.child_focus(gtk::DirectionType::TabForward);
            }
            other => tracing::debug!("no handler for action {other}"),
        }
    }

    /// The transition right-click menu's "Change Transition Duration…": its length in frames.
    fn change_transition_duration(&self) {
        let state = &self.state;
        let Some((track, t)) = actions::selected_transition(state) else { return };
        let Some(fps) = state.with_project(|p| p.timeline_fps()) else { return };
        let frames = gtk::SpinButton::with_range(1.0, 100_000.0, 1.0);
        frames.set_value(tempo_timeline::us_to_frame(t.duration_us, fps) as f64);
        frames.set_activates_default(true);
        let seconds = label("", &["tempo-dim"]);
        seconds.set_xalign(0.0);
        let show_seconds = {
            let seconds = seconds.clone();
            move |f: f64| seconds.set_text(&format!("{:.2} seconds", tempo_timeline::frame_to_us(f as i64, fps) as f64 / 1e6))
        };
        show_seconds(frames.value());
        frames.connect_value_changed(move |b| show_seconds(b.value()));
        let column = gtk::Box::new(gtk::Orientation::Vertical, 6);
        column.append(&frames);
        column.append(&seconds);
        let dialog = adw::AlertDialog::builder().heading("Change Transition Duration").body("Length in frames").extra_child(&column).build();
        dialog.add_responses(&[("cancel", "Cancel"), ("change", "Change")]);
        dialog.set_response_appearance("change", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("change"));
        dialog.set_close_response("cancel");
        let state = state.clone();
        dialog.connect_response(Some("change"), move |_, _| {
            let length = tempo_timeline::frame_to_us(frames.value() as i64, fps);
            actions::set_transition_length(&state, track, t.clip_id, t.edge, length);
        });
        dialog.present(Some(&self.window));
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

/// Show `zone`'s shortcut hints while the pointer is over `widget`.
fn hint_over(state: &Rc<AppState>, widget: &impl IsA<gtk::Widget>, zone: crate::keybinds::HintZone) {
    let motion = gtk::EventControllerMotion::new();
    let s = state.clone();
    motion.connect_enter(move |_, _, _| s.hint(Some(zone)));
    let s = state.clone();
    motion.connect_leave(move |_| s.hint(None));
    widget.add_controller(motion);
}
