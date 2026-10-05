//! Application start-up: style, dark scheme, main window.

use gtk4 as gtk;
use gtk4::prelude::*;
use libadwaita as adw;

use crate::window::MainWindow;

pub const APP_ID: &str = "dev.tempo.Tempo";
const CSS: &str = include_str!("../../../assets/style/tempo.css");

pub struct TempoApp {
    app: adw::Application,
}

impl Default for TempoApp {
    fn default() -> Self {
        Self::new()
    }
}

impl TempoApp {
    pub fn new() -> Self {
        let app = adw::Application::builder().application_id(APP_ID).build();

        app.connect_startup(|_| {
            // Tempo is dark only, like the editor it prepares people for.
            adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceDark);
            let provider = gtk::CssProvider::new();
            provider.load_from_string(CSS);
            if let Some(display) = gtk::gdk::Display::default() {
                // Above user themes in ~/.config/gtk-4.0: Tempo has one fixed look by design.
                gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_USER + 1);
            }
        });

        app.connect_activate(|app| {
            if let Some(window) = app.active_window() {
                window.present();
                return;
            }
            let win = MainWindow::new(app);
            win.window.present();
            dev_hooks(app, &win);
        });

        Self { app }
    }

    pub fn run(&self) -> glib::ExitCode {
        // Tempo reads its own flags in main(); GTK gets only the program name.
        let program = std::env::args().next().unwrap_or_else(|| "tempo".to_string());
        self.app.run_with_args(&[program])
    }
}

/// Hooks for automated checks. They do nothing unless their variable is set.
///
/// * `TEMPO_SMOKE_TEST=1` — start, then quit as soon as the window is up.
/// * `TEMPO_SCREENSHOT=/path.png` — after `TEMPO_SCREENSHOT_DELAY_MS` (default 1500)
///   save a picture of the window and quit.
/// * `TEMPO_ACTIONS=win.page-export,…` — run these actions one second after start.
fn dev_hooks(app: &adw::Application, win: &std::rc::Rc<MainWindow>) {
    let window = &win.window;
    if let Ok(list) = std::env::var("TEMPO_ACTIONS") {
        let win = win.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(1000), move || {
            for action in list.split(',') {
                win.run_action(action.trim());
            }
        });
    }
    if std::env::var_os("TEMPO_SMOKE_TEST").is_some() {
        let app = app.clone();
        glib::idle_add_local_once(move || app.quit());
        return;
    }
    let Some(path) = std::env::var_os("TEMPO_SCREENSHOT") else { return };
    let delay = std::env::var("TEMPO_SCREENSHOT_DELAY_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(1500u64);
    let app = app.clone();
    let window = window.clone();
    glib::timeout_add_local_once(std::time::Duration::from_millis(delay), move || {
        let (w, h) = (window.width(), window.height());
        let paintable = gtk::WidgetPaintable::new(Some(&window));
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(&snapshot, w as f64, h as f64);
        let saved = snapshot
            .to_node()
            .zip(window.native().and_then(|n| n.renderer()))
            .map(|(node, renderer)| renderer.render_texture(&node, None).save_to_png(&path));
        match saved {
            Some(Ok(())) => tracing::info!("screenshot saved"),
            other => tracing::error!("screenshot failed: {other:?}"),
        }
        app.quit();
    });
}
