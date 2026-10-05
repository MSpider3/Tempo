//! Viewer: header, picture, scrub bar and transport.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gtk4 as gtk;
use gtk4::prelude::*;

use crate::player::Snapshot;
use crate::state::{timecode, AppState, Change};
use crate::util::{label, tool_button};
use crate::video_surface::VideoSurface;

pub struct Viewer {
    pub root: gtk::Box,
    pub surface: VideoSurface,
    state: Rc<AppState>,
    name: gtk::Label,
    proxy_badge: gtk::Label,
    scrub: gtk::Scale,
    play: gtk::Button,
    position_listeners: RefCell<Vec<Box<dyn Fn(i64)>>>,
    seek_generation: Cell<u32>,
}

impl Viewer {
    pub fn new(state: &Rc<AppState>) -> Rc<Self> {
        let root = gtk::Box::builder().orientation(gtk::Orientation::Vertical).hexpand(true).vexpand(true).build();

        // Header: name in the middle, timecode and menu at the right.
        let name = label("", &["tempo-bright", "tempo-heading"]);
        let tc = label("00:00:00:00", &["tempo-timecode"]);
        let quality = gtk::MenuButton::builder()
            .icon_name("view-more-symbolic")
            .tooltip_text("Viewer options")
            .css_classes(["tool"])
            .build();
        quality.set_popover(Some(&quality_popover(state)));
        let end = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        end.append(&tc);
        end.append(&quality);
        let header = gtk::CenterBox::builder().css_classes(["viewer-header"]).build();
        header.set_center_widget(Some(&name));
        header.set_end_widget(Some(&end));
        root.append(&header);

        // Picture.
        let surface = VideoSurface::new();
        let proxy_badge = label("PROXY", &["badge", "warn"]);
        proxy_badge.set_halign(gtk::Align::End);
        proxy_badge.set_valign(gtk::Align::Start);
        proxy_badge.set_visible(false);
        let overlay = gtk::Overlay::builder().child(&surface).css_classes(["viewer-surround"]).build();
        overlay.add_overlay(&proxy_badge);
        root.append(&overlay);

        // Scrub bar.
        let scrub = gtk::Scale::builder()
            .orientation(gtk::Orientation::Horizontal)
            .adjustment(&gtk::Adjustment::new(0.0, 0.0, 1.0, 1.0, 1.0, 0.0))
            .draw_value(false)
            .tooltip_text("Scrub")
            .css_classes(["tempo-panel"])
            .build();
        root.append(&scrub);

        // Transport.
        let first = tool_button("media-skip-backward-symbolic", "Go to start (Home)");
        let reverse = tool_button("media-seek-backward-symbolic", "Play reverse (J)");
        let stop = tool_button("media-playback-stop-symbolic", "Stop (K)");
        let play = tool_button("media-playback-start-symbolic", "Play / stop (Space)");
        let last = tool_button("media-skip-forward-symbolic", "Go to end (End)");
        let centre = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        for b in [&first, &reverse, &stop, &play, &last] {
            centre.append(b);
        }
        let mark_in = tool_button("go-first-symbolic", "Mark In (I)");
        let mark_out = tool_button("go-last-symbolic", "Mark Out (O)");
        let marks = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        marks.append(&mark_in);
        marks.append(&mark_out);
        let transport = gtk::CenterBox::builder().css_classes(["viewer-transport"]).build();
        transport.set_center_widget(Some(&centre));
        transport.set_end_widget(Some(&marks));
        root.append(&transport);

        let viewer = Rc::new(Self {
            root,
            surface,
            state: state.clone(),
            name,
            proxy_badge,
            scrub,
            play,
            position_listeners: RefCell::new(Vec::new()),
            seek_generation: Cell::new(0),
        });

        let s = state.clone();
        first.connect_clicked(move |_| s.player.seek(0, true));
        let s = state.clone();
        last.connect_clicked(move |_| s.player.seek(s.player.duration_us(), true));
        let s = state.clone();
        reverse.connect_clicked(move |_| s.player.play(-1.0));
        let s = state.clone();
        stop.connect_clicked(move |_| s.player.pause());
        let v = viewer.clone();
        viewer.play.connect_clicked(move |_| v.toggle_play());
        let v = viewer.clone();
        mark_in.connect_clicked(move |_| v.mark(true));
        let v = viewer.clone();
        mark_out.connect_clicked(move |_| v.mark(false));

        // Dragging the scrub bar: keyframes while moving, exact frame when it rests.
        let v = viewer.clone();
        viewer.scrub.connect_change_value(move |_, _, value| {
            v.scrub_to(value as i64);
            glib::Propagation::Proceed
        });

        // One tick per display frame while the viewer is on screen. It only reads
        // a few atomics unless a new picture or position has arrived.
        let v = viewer.clone();
        let last_serial = Cell::new(0u64);
        let last_pos = Cell::new(-1i64);
        let last_playing = Cell::new(false);
        let last_duration = Cell::new(-1i64);
        viewer.surface.add_tick_callback(move |_, _| {
            let player = &v.state.player;
            let serial = player.serial();
            if serial != last_serial.replace(serial) {
                if let Some(layers) = player.take_layers() {
                    v.surface.set_layers(layers);
                }
            }
            let duration = player.duration_us();
            if duration != last_duration.replace(duration) {
                v.scrub.set_range(0.0, duration.max(1) as f64);
            }
            let pos = player.position_us();
            if pos != last_pos.replace(pos) {
                tc.set_text(&timecode(pos, v.state.fps()));
                v.scrub.set_value(pos as f64);
                for l in v.position_listeners.borrow().iter() {
                    l(pos);
                }
            }
            let playing = player.is_playing();
            if playing != last_playing.replace(playing) {
                v.play.set_icon_name(if playing { "media-playback-pause-symbolic" } else { "media-playback-start-symbolic" });
            }
            glib::ControlFlow::Continue
        });

        let v = viewer.clone();
        state.connect(move |change| {
            if matches!(change, Change::Project | Change::Timeline | Change::Media) {
                v.refresh();
            }
        });
        viewer.refresh();
        viewer
    }

    /// Called with the playhead position whenever it moves.
    pub fn connect_position(&self, f: impl Fn(i64) + 'static) {
        self.position_listeners.borrow_mut().push(Box::new(f));
    }

    pub fn toggle_play(&self) {
        let player = &self.state.player;
        if player.is_playing() {
            player.pause();
        } else {
            if player.position_us() >= player.duration_us() {
                player.seek(0, true);
            }
            player.play(1.0);
        }
    }

    /// Seek cheaply now, then exactly once the pointer has rested for a moment.
    pub fn scrub_to(self: &Rc<Self>, us: i64) {
        self.state.player.pause();
        self.state.player.seek(us, false);
        let generation = self.seek_generation.get().wrapping_add(1);
        self.seek_generation.set(generation);
        let v = self.clone();
        glib::timeout_add_local_once(Duration::from_millis(120), move || {
            if v.seek_generation.get() == generation {
                v.state.player.seek(us, true);
            }
        });
    }

    fn mark(&self, is_in: bool) {
        crate::actions::set_mark(&self.state, is_in, false);
    }

    /// Show a Media Pool clip (source mode) or go back to the timeline.
    pub fn show_source(&self, source: Option<uuid::Uuid>) {
        let snapshot = source.and_then(|id| {
            self.state
                .with_project(|p| {
                    p.sources.get(&id).map(|s| {
                        let has_video = s.media_type != tempo_timeline::MediaType::Audio;
                        Snapshot::from_source(id, s.path.clone(), s.duration_us, p.fps.frame_duration_us(), has_video, s.audio_channels.is_some())
                    })
                })
                .flatten()
        });
        self.state.source_clip.set(if snapshot.is_some() { source } else { None });
        self.state.src_in.set(None);
        self.state.src_out.set(None);
        self.state.player.set_source(snapshot);
        self.refresh();
    }

    fn refresh(&self) {
        let state = &self.state;
        let (w, h, title) = state
            .with_project(|p| {
                let title = match state.source_clip.get().and_then(|id| p.sources.get(&id)) {
                    Some(s) => s.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                    None => "Timeline".to_string(),
                };
                (p.width, p.height, title)
            })
            .unwrap_or((1920, 1080, String::new()));
        self.surface.set_frame_size(w, h);
        self.name.set_text(&title);
        // Shown while any clip is previewed from its proxy.
        let proxied = state.with_project(|p| p.sources.values().any(|s| s.proxy_ready)).unwrap_or(false);
        self.proxy_badge.set_visible(proxied && state.source_clip.get().is_none());
    }
}

/// Playback quality. Lower settings decode to a smaller picture, which costs less CPU.
fn quality_popover(state: &Rc<AppState>) -> gtk::Popover {
    let list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).margin_top(8).margin_bottom(8).margin_start(8).margin_end(8).build();
    list.append(&label("Playback quality", &["tempo-dim", "tempo-small"]));
    let mut group: Option<gtk::CheckButton> = None;
    for (text, height) in [("Full", 0u32), ("Half", 540), ("Quarter", 270)] {
        let item = gtk::CheckButton::with_label(text);
        if let Some(g) = &group {
            item.set_group(Some(g));
        } else {
            group = Some(item.clone());
        }
        item.set_active(height == 540);
        let s = state.clone();
        item.connect_toggled(move |b| {
            if b.is_active() {
                s.player.set_max_height(height);
            }
        });
        list.append(&item);
    }
    gtk::Popover::builder().child(&list).build()
}
