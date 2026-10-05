//! Application start-up: style, dark scheme, main window.

use gtk4 as gtk;
use gtk4::prelude::*;
use libadwaita as adw;

use crate::window::MainWindow;

pub const APP_ID: &str = "dev.tempo.Tempo";
const CSS: &str = include_str!("../../../assets/style/tempo.css");
pub(crate) const GRESOURCE_BYTES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/tempo.gresource"));

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
            let bytes = glib::Bytes::from_static(GRESOURCE_BYTES);
            if let Ok(resource) = gio::Resource::from_data(&bytes) {
                gio::resources_register(&resource);
            }

            // Tempo is dark only, like the editor it prepares people for.
            adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceDark);
            let provider = gtk::CssProvider::new();
            provider.load_from_string(CSS);
            if let Some(display) = gtk::gdk::Display::default() {
                // Above user themes in ~/.config/gtk-4.0: Tempo has one fixed look by design.
                gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_USER + 1);
                let theme = gtk::IconTheme::for_display(&display);
                theme.add_resource_path("/dev/tempo/Tempo/icons");
            }
            gtk::Window::set_default_icon_name(APP_ID);
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
/// * `TEMPO_ACTIONS="step;step;…"` — one second after start, run these steps one
///   after another, 150 ms apart. A step is an action name or a scripted pointer
///   step (see `MainWindow::run_script_step`).
fn dev_hooks(app: &adw::Application, win: &std::rc::Rc<MainWindow>) {
    let window = &win.window;
    if let Ok(list) = std::env::var("TEMPO_ACTIONS") {
        let win = win.clone();
        // Semicolons separate steps; a lone comma list of plain action names also works.
        let separator = if list.contains(';') || list.contains(':') { ';' } else { ',' };
        let steps: Vec<String> = list.split(separator).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        for (i, step) in steps.into_iter().enumerate() {
            let win = win.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(1000 + 150 * i as u64), move || win.run_script_step(&step));
        }
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
    let win = win.clone();
    glib::timeout_add_local_once(std::time::Duration::from_millis(delay), move || {
        tracing::info!("stats {}", win.stats());
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_icons_are_present() {
        let bytes = glib::Bytes::from_static(GRESOURCE_BYTES);
        let resource = gio::Resource::from_data(&bytes).expect("bundled resource is valid GVDB");

        let expected_icons = [
            "audio-volume-high-symbolic",
            "audio-volume-muted-symbolic",
            "audio-x-generic-symbolic",
            "changes-allow-symbolic",
            "changes-prevent-symbolic",
            "dev.tempo.Tempo-symbolic",
            "dev.tempo.Tempo",
            "dialog-warning-symbolic",
            "document-edit-symbolic",
            "document-properties-symbolic",
            "document-send-symbolic",
            "edit-undo-symbolic",
            "emblem-favorite-symbolic",
            "folder-symbolic",
            "go-first-symbolic",
            "go-home-symbolic",
            "go-last-symbolic",
            "image-x-generic-symbolic",
            "list-add-symbolic",
            "media-playback-pause-symbolic",
            "media-playback-start-symbolic",
            "media-playback-stop-symbolic",
            "media-seek-backward-symbolic",
            "media-skip-backward-symbolic",
            "media-skip-forward-symbolic",
            "send-to-symbolic",
            "system-search-symbolic",
            "video-x-generic-symbolic",
            "view-conceal-symbolic",
            "view-more-symbolic",
            "view-reveal-symbolic",
            "window-close-symbolic",
            "zoom-fit-best-symbolic",
            "zoom-in-symbolic",
            "zoom-out-symbolic",
        ];

        for icon in expected_icons {
            let found = resource
                .lookup_data(
                    &format!("/dev/tempo/Tempo/icons/scalable/actions/{icon}.svg"),
                    gio::ResourceLookupFlags::NONE,
                )
                .is_ok()
                || resource
                    .lookup_data(
                        &format!("/dev/tempo/Tempo/icons/scalable/mimetypes/{icon}.svg"),
                        gio::ResourceLookupFlags::NONE,
                    )
                    .is_ok()
                || resource
                    .lookup_data(
                        &format!("/dev/tempo/Tempo/icons/scalable/apps/{icon}.svg"),
                        gio::ResourceLookupFlags::NONE,
                    )
                    .is_ok();

            assert!(found, "Icon '{icon}' must be present in bundled GResource");
        }
    }
}
