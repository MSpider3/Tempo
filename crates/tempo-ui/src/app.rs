//! Application start-up: style, dark scheme, main window.

use gtk4 as gtk;
use gtk4::prelude::*;
use libadwaita as adw;

use std::rc::Rc;

use serde_json::{json, Value};

use crate::state::EditSelection;
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
            // End-to-end tests click at screen positions: fullscreen puts the window at 0,0.
            if std::env::var_os("TEMPO_TEST_FULLSCREEN").is_some() {
                win.window.fullscreen();
            }
            win.window.present();
            dev_hooks(app, &win);
            test_control(app, &win);
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
        // An open menu is a window of its own, so it gets a picture of its own
        // beside the main one ("shot.png" → "shot.menu.png").
        if let Some(menu) = open_popover(window.upcast_ref()) {
            let (w, h) = (menu.width(), menu.height());
            let snapshot = gtk::Snapshot::new();
            gtk::WidgetPaintable::new(Some(&menu)).snapshot(&snapshot, w as f64, h as f64);
            let target = std::path::PathBuf::from(&path).with_extension("menu.png");
            let saved = snapshot.to_node().zip(menu.native().and_then(|n| n.renderer())).map(|(node, renderer)| renderer.render_texture(&node, None).save_to_png(&target));
            tracing::info!("menu screenshot: {}", matches!(saved, Some(Ok(()))));
        }
        app.quit();
    });
}

// ---- End-to-end test control ------------------------------------------------------

/// `TEMPO_TEST_CONTROL=/private-dir/control.sock` serves the end-to-end test channel
/// (docs/TESTING_E2E.md): one JSON reply line per command line. Nothing is opened
/// unless the variable is set; everything runs on the main loop without blocking it.
fn test_control(app: &adw::Application, win: &Rc<MainWindow>) {
    let Some(path) = std::env::var_os("TEMPO_TEST_CONTROL") else { return };
    let service = gio::SocketService::new();
    let address = gio::UnixSocketAddress::new(std::path::Path::new(&path));
    if let Err(e) = service.add_address(&address, gio::SocketType::Stream, gio::SocketProtocol::Default, None::<&glib::Object>) {
        tracing::error!("test control socket: {e}");
        return;
    }
    let (app, w) = (app.clone(), win.clone());
    service.connect_incoming(move |_, connection, _| {
        glib::MainContext::default().spawn_local(serve_test_client(app.clone(), w.clone(), connection.clone()));
        false
    });
    service.start();
    // The closure keeps the service alive as long as the window.
    win.window.connect_destroy(move |_| service.stop());
    tracing::info!("test control listening");
}

async fn serve_test_client(app: adw::Application, win: Rc<MainWindow>, connection: gio::SocketConnection) {
    let input = gio::DataInputStream::new(&connection.input_stream());
    let output = connection.output_stream();
    while let Ok(Some(line)) = input.read_line_utf8_future(glib::Priority::DEFAULT).await {
        let reply = match test_command(&app, &win, line.trim()).await {
            Ok(result) => serde_json::json!({ "ok": true, "result": result }),
            Err(error) => serde_json::json!({ "ok": false, "error": error }),
        };
        if output.write_all_future(format!("{reply}\n").into_bytes(), glib::Priority::DEFAULT).await.is_err() {
            break;
        }
    }
}

async fn test_command(app: &adw::Application, win: &Rc<MainWindow>, line: &str) -> Result<Value, String> {
    let (command, arg) = line.split_once(' ').unwrap_or((line, ""));
    match command {
        "ping" => Ok(json!("pong")),
        "state" => Ok(test_state(win)),
        "geometry" => Ok(test_geometry(win)),
        // Setup only: a GAction name or a `run_script_step` step.
        "action" if !arg.trim().is_empty() => {
            win.run_script_step(arg.trim());
            Ok(Value::Null)
        }
        "wait-idle" => {
            // Idle, one frame, idle: pending redraws and replies to them have run.
            let low = || glib::timeout_future_with_priority(glib::Priority::LOW, std::time::Duration::ZERO);
            low().await;
            glib::timeout_future(std::time::Duration::from_millis(20)).await;
            low().await;
            Ok(Value::Null)
        }
        "quit" => {
            app.quit();
            Ok(Value::Null)
        }
        other => Err(format!("unknown command '{other}' (ping, state, geometry, action <step>, wait-idle, quit)")),
    }
}

/// The project and everything about the editing state a test asserts on.
fn test_state(win: &MainWindow) -> Value {
    let s = &win.state;
    let edit_selection = s.edit_selection.get().map(|e| match e {
        EditSelection::Transition { track_id, clip_id, edge } => json!({ "kind": "transition", "track_id": track_id, "clip_id": clip_id, "edge": format!("{edge:?}") }),
        EditSelection::Edit { track_id, position_us } => json!({ "kind": "edit", "track_id": track_id, "position_us": position_us }),
    });
    let playhead = s.playhead_us();
    json!({
        "project_loaded": s.project.borrow().is_some(),
        "path": s.path.borrow().as_ref().map(|p| p.display().to_string()),
        "project": s.project.borrow().as_ref().and_then(|p| serde_json::to_value(p).ok()),
        "selection": s.selected_ids(),
        "media_selection": s.media_selection.get(),
        "edit_selection": edit_selection,
        "selected_transition": crate::actions::selected_transition(s).map(|(track, t)| json!({ "track_id": track, "transition": t })),
        "transition_clipboard": s.transition_clipboard.borrow().clone(),
        "playhead_us": playhead,
        "playhead_tc": s.position_tc(playhead),
        "start_tc_us": s.start_tc_us(),
        "fps": s.fps(),
        "settings": s.settings.borrow().clone(),
        "tool": format!("{:?}", s.tool.get()),
        "dirty": s.dirty.get(),
        "playing": s.player.is_playing(),
        "title": win.window.title().map(|t| t.to_string()),
        "fullscreen": win.window.is_fullscreen(),
        "focus": gtk::prelude::GtkWindowExt::focus(&win.window).map(|w| w.type_().name().to_string()),
    })
}

/// Screen rectangles (window surface = screen when fullscreen) of every mapped widget,
/// plus the timeline's tracks, clips, transitions and markers.
fn test_geometry(win: &MainWindow) -> Value {
    let window: &gtk::Widget = win.window.upcast_ref();
    let (ox, oy) = win.window.surface_transform();
    let mut widgets = Vec::new();
    collect_widgets(window, window, (ox, oy), None, &mut widgets);
    json!({
        "window": [0.0, 0.0, win.window.width(), win.window.height()],
        "fullscreen": win.window.is_fullscreen(),
        "widgets": widgets,
        "timeline": timeline_geometry(win, (ox, oy)),
    })
}

fn rect_json(r: &gtk::graphene::Rect, (ox, oy): (f64, f64)) -> Value {
    json!([r.x() as f64 + ox, r.y() as f64 + oy, r.width(), r.height()])
}

/// Depth first; `native` is the widget whose surface `origin` is the screen offset of.
fn collect_widgets(widget: &gtk::Widget, native: &gtk::Widget, origin: (f64, f64), parent: Option<usize>, out: &mut Vec<Value>) {
    let mut child = widget.first_child();
    while let Some(c) = child {
        child = c.next_sibling();
        if !c.is_mapped() {
            continue;
        }
        // A popover is a surface of its own, placed relative to its parent's surface.
        let (native, origin) = match c.downcast_ref::<gtk::Popover>() {
            Some(popover) => {
                let place = popover.surface().and_then(|s| s.downcast::<gtk::gdk::Popup>().ok()).map(|p| (p.position_x() as f64, p.position_y() as f64));
                let (tx, ty) = popover.surface_transform();
                let (px, py) = place.unwrap_or_default();
                (c.clone(), (origin.0 + px + tx, origin.1 + py + ty))
            }
            None => (native.clone(), origin),
        };
        let Some(bounds) = c.compute_bounds(&native) else { continue };
        let id = out.len();
        out.push(widget_json(&c, id, parent, rect_json(&bounds, origin)));
        collect_widgets(&c, &native, origin, Some(id), out);
    }
}

fn widget_json(w: &gtk::Widget, id: usize, parent: Option<usize>, rect: Value) -> Value {
    let type_name = w.type_().name().to_string();
    let text = if let Some(l) = w.downcast_ref::<gtk::Label>() {
        Some(l.label().to_string())
    } else if let Some(e) = w.dynamic_cast_ref::<gtk::Editable>() {
        Some(e.text().to_string())
    } else if let Some(b) = w.downcast_ref::<gtk::Button>() {
        b.label().map(|l| l.to_string())
    } else if let Some(b) = w.downcast_ref::<gtk::CheckButton>() {
        b.label().map(|l| l.to_string())
    } else if let Some(d) = w.downcast_ref::<gtk::DropDown>() {
        d.selected_item().and_then(|i| i.downcast::<gtk::StringObject>().ok()).map(|s| s.string().to_string())
    } else {
        w.downcast_ref::<gtk::Image>().and_then(|i| i.icon_name()).map(|n| n.to_string())
    };
    let active = w
        .downcast_ref::<gtk::ToggleButton>()
        .map(|b| b.is_active())
        .or_else(|| w.downcast_ref::<gtk::CheckButton>().map(|b| b.is_active()))
        .or_else(|| w.downcast_ref::<gtk::Switch>().map(|b| b.is_active()));
    let value = w.downcast_ref::<gtk::Range>().map(|r| r.value()).or_else(|| w.downcast_ref::<gtk::SpinButton>().map(|s| s.value()));
    let name = w.widget_name();
    json!({
        "id": id,
        "parent": parent,
        "type": type_name,
        "name": (name != type_name).then(|| name.to_string()),
        "css": w.css_classes().iter().map(|c| c.to_string()).collect::<Vec<_>>(),
        "text": text,
        "tooltip": w.tooltip_text().map(|t| t.to_string()),
        "active": active,
        "value": value,
        "sensitive": w.is_sensitive(),
        "focus": w.has_focus(),
        "rect": rect,
    })
}

fn timeline_geometry(win: &MainWindow, origin: (f64, f64)) -> Value {
    let canvas = &win.edit.timeline.canvas;
    if !canvas.is_mapped() {
        return Value::Null;
    }
    let window = &win.window;
    let at = |x: f64, y: f64| canvas.compute_point(window, &gtk::graphene::Point::new(x as f32, y as f32)).map(|p| (p.x() as f64 + origin.0, p.y() as f64 + origin.1)).unwrap_or_default();
    let rect = |x0: f64, y0: f64, x1: f64, y1: f64| {
        let (a, b) = (at(x0, y0), at(x1, y1));
        json!([a.0, a.1, b.0 - a.0, b.1 - a.1])
    };
    let width = canvas.width() as f64;
    let rows = win.state.with_timeline(crate::timeline::rows).unwrap_or_default();
    let ruler_h = crate::timeline::RULER_H as f64;
    let first_y = rows.first().map_or(ruler_h, |r| r.y as f64);
    let name = |r: &crate::timeline::Row| format!("{}{}", if r.kind == tempo_timeline::TrackKind::Video { "V" } else { "A" }, r.index);
    let first = rows.first().map(name).unwrap_or_default();
    let x = |name: &str, us: i64| canvas.point_for(us as f64 / 1e6, name).map_or(0.0, |p| p.0);
    let (mut tracks, mut clips, mut transitions) = (Vec::new(), Vec::new(), Vec::new());
    win.state.with_timeline(|t| {
        for row in &rows {
            let Some(track) = t.find_track(row.track_id) else { continue };
            let n = name(row);
            let (top, bottom) = (row.y as f64, (row.y + row.h) as f64);
            tracks.push(json!({ "name": n, "id": track.id, "rect": rect(0.0, top, width, bottom) }));
            for c in &track.clips {
                let (x0, x1) = (x(&n, c.timeline_in), x(&n, c.timeline_out));
                clips.push(json!({ "id": c.id, "name": c.name, "track": n, "in_us": c.timeline_in, "out_us": c.timeline_out, "rect": rect(x0, top, x1, bottom), "edit_y": at(0.0, bottom - 8.0).1 }));
            }
            for tr in &track.transitions {
                let Some((a, b)) = track.transition_span(tr) else { continue };
                transitions.push(json!({ "id": tr.id, "clip_id": tr.clip_id, "kind": format!("{:?}", tr.kind), "track": n, "start_us": a, "end_us": b, "rect": rect(x(&n, a), top + 6.0, x(&n, b), bottom - 26.0) }));
            }
        }
    });
    let markers = win.state.with_timeline(|t| t.markers.iter().map(|m| json!({ "id": m.id, "position_us": m.position_us, "x": at(x(&first, m.position_us), 0.0).0 })).collect::<Vec<_>>()).unwrap_or_default();
    json!({
        "canvas": rect(0.0, 0.0, width, canvas.height() as f64),
        "ruler": rect(0.0, 0.0, width, ruler_h),
        "ghost_band": rect(0.0, ruler_h, width, first_y),
        "pps": canvas.pps(),
        "playhead_x": at(x(&first, win.state.playhead_us()), 0.0).0,
        "tracks": tracks,
        "clips": clips,
        "transitions": transitions,
        "markers": markers,
    })
}

/// The first popover that is showing anywhere below `widget`.
fn open_popover(widget: &gtk::Widget) -> Option<gtk::Popover> {
    let mut child = widget.first_child();
    while let Some(c) = child {
        if let Some(popover) = c.downcast_ref::<gtk::Popover>().filter(|p| p.is_mapped()) {
            return Some(popover.clone());
        }
        if let Some(found) = open_popover(&c) {
            return Some(found);
        }
        child = c.next_sibling();
    }
    None
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
