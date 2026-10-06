//! Viewer: header, picture, scrub bar and transport.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gtk4 as gtk;
use gtk4::prelude::*;

use crate::player::{Player, Snapshot};
use crate::state::{timecode, AppState, Change};
use crate::util::{label, tool_button};
use crate::video_surface::VideoSurface;

pub struct Viewer {
    pub root: gtk::Box,
    pub surface: VideoSurface,
    state: Rc<AppState>,
    /// The playback this viewer shows and controls.
    player: Player,
    /// True for the second viewer of dual-viewer mode, which only shows source clips.
    source_only: bool,
    shown_source: Cell<Option<uuid::Uuid>>,
    /// Called when "Dual viewer" is switched in the menu.
    dual_listener: RefCell<Option<Box<dyn Fn(bool)>>>,
    name: gtk::Label,
    proxy_badge: gtk::Label,
    /// Shown while a Media Pool clip is in the viewer: the way back.
    back: gtk::Button,
    scrub: gtk::Scale,
    play: gtk::Button,
    position_listeners: RefCell<Vec<Box<dyn Fn(i64)>>>,
    seek_generation: Cell<u32>,
}

impl Viewer {
    /// The main viewer: shows the timeline, or a source clip in single-viewer mode.
    pub fn new(state: &Rc<AppState>) -> Rc<Self> {
        Self::build(state, state.player.clone(), false)
    }

    /// The source viewer of dual-viewer mode, with its own playback.
    pub fn new_source(state: &Rc<AppState>, player: Player) -> Rc<Self> {
        Self::build(state, player, true)
    }

    pub fn connect_dual(&self, f: impl Fn(bool) + 'static) {
        *self.dual_listener.borrow_mut() = Some(Box::new(f));
    }

    fn build(state: &Rc<AppState>, player: Player, source_only: bool) -> Rc<Self> {
        let root = gtk::Box::builder().orientation(gtk::Orientation::Vertical).hexpand(true).vexpand(true).build();

        // Header: name in the middle, timecode and menu at the right.
        let name = label("", &["tempo-bright", "tempo-heading"]);
        let tc = label("00:00:00:00", &["tempo-timecode"]);
        let end = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        end.append(&tc);
        let mut dual_check = None;
        if !source_only {
            // Only the main viewer has options of its own.
            let options = gtk::MenuButton::builder().icon_name("view-more-symbolic").tooltip_text("Viewer options").css_classes(["tool"]).build();
            let (popover, check) = viewer_menu();
            options.set_popover(Some(&popover));
            end.append(&options);
            dual_check = Some(check);
        }
        let header = gtk::CenterBox::builder().css_classes(["viewer-header"]).build();
        // Preview quality is the setting that matters most on a slow computer,
        // so it is in plain sight, not inside a menu.
        let (quality, sync_quality) = quality_button(state);
        let back = crate::util::text_button("Back to Timeline", "Stop looking at this Media Pool clip and show the timeline again (Q)");
        back.set_visible(false);
        let s = state.clone();
        back.connect_clicked(move |_| s.show_source(None));
        let start = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        start.append(&quality);
        start.append(&back);
        header.set_start_widget(Some(&start));
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
            player: player.clone(),
            source_only,
            shown_source: Cell::new(None),
            dual_listener: RefCell::new(None),
            name,
            proxy_badge,
            back,
            scrub,
            play,
            position_listeners: RefCell::new(Vec::new()),
            seek_generation: Cell::new(0),
        });

        let p = player.clone();
        first.connect_clicked(move |_| p.seek(0, true));
        let p = player.clone();
        last.connect_clicked(move |_| p.seek(p.duration_us(), true));
        let p = player.clone();
        reverse.connect_clicked(move |_| p.play(-1.0));
        let p = player.clone();
        stop.connect_clicked(move |_| p.pause());
        if let Some(check) = dual_check {
            let v = viewer.clone();
            check.connect_toggled(move |c| {
                if let Some(f) = v.dual_listener.borrow().as_ref() {
                    f(c.is_active());
                }
            });
        }
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
            let player = &v.player;
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
        let height = Cell::new(u32::MAX);
        state.connect(move |change| {
            if matches!(change, Change::Options) {
                v.show_marks();
                // The quality may have changed, here or in Preferences.
                let now = v.state.settings.borrow().playback_height;
                if height.replace(now) != now {
                    sync_quality(now);
                    if v.source_only {
                        v.player.set_max_height(now);
                    }
                    v.refresh();
                }
            }
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
        let player = &self.player;
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
        self.player.pause();
        self.player.seek(us, false);
        let generation = self.seek_generation.get().wrapping_add(1);
        self.seek_generation.set(generation);
        let v = self.clone();
        glib::timeout_add_local_once(Duration::from_millis(120), move || {
            if v.seek_generation.get() == generation {
                v.player.seek(us, true);
            }
        });
    }

    fn mark(&self, is_in: bool) {
        if self.source_only {
            // The source viewer's buttons mark the source clip at its own position.
            let pos = Some(self.player.position_us());
            if is_in {
                self.state.src_in.set(pos);
            } else {
                self.state.src_out.set(pos);
            }
            self.state.emit(Change::Options);
        } else {
            crate::actions::set_mark(&self.state, is_in, false);
        }
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
        let shown = if snapshot.is_some() { source } else { None };
        self.state.source_clip.set(shown);
        self.state.src_in.set(None);
        self.state.src_out.set(None);
        if self.source_only {
            self.shown_source.set(shown);
            self.player.pause();
            self.player.set_timeline(snapshot.unwrap_or_else(Snapshot::empty));
            self.player.seek(0, true);
        } else {
            self.player.set_source(snapshot);
        }
        self.refresh();
    }

    /// In and Out as ticks on the scrub bar: the source clip's own marks while
    /// one is shown, the timeline's otherwise.
    fn show_marks(&self) {
        let state = &self.state;
        let source = self.source_only || state.source_mode();
        let (mark_in, mark_out) = if source { (state.src_in.get(), state.src_out.get()) } else { (state.mark_in.get(), state.mark_out.get()) };
        self.scrub.clear_marks();
        for (mark, name) in [(mark_in, "In"), (mark_out, "Out")] {
            if let Some(us) = mark {
                self.scrub.add_mark(us as f64, gtk::PositionType::Top, Some(name));
            }
        }
    }

    fn refresh(&self) {
        let state = &self.state;
        self.back.set_visible(!self.source_only && state.source_mode());
        self.show_marks();
        let (w, h, title) = state
            .with_project(|p| {
                let source = if self.source_only { self.shown_source.get() } else { state.source_clip.get().filter(|_| state.source_mode()) };
                let title = match source.and_then(|id| p.sources.get(&id)) {
                    Some(s) => s.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                    None if self.source_only => "Source".to_string(),
                    None => "Timeline".to_string(),
                };
                (p.width, p.height, title)
            })
            .unwrap_or((1920, 1080, String::new()));
        self.surface.set_frame_size(w, h);
        self.name.set_text(&title);
        // Shown while any clip is previewed from its proxy.
        let proxied = state.with_project(|p| p.sources.values().any(|s| s.proxy_ready)).unwrap_or(false);
        let uses_proxies = state.settings.borrow().playback_height != 0;
        self.proxy_badge.set_visible(proxied && uses_proxies && !self.source_only && !state.source_mode());
    }
}

/// Preview quality choices: label, what it means, and the height the picture is
/// decoded at (0 is the original file at full size).
const QUALITIES: [(&str, &str, u32); 3] = [
    ("Full", "The original file at full size. Sharpest, and the heaviest.", 0),
    ("Half", "A small proxy copy at half size. Smooth on most computers.", 540),
    ("Quarter", "A proxy at quarter size. For slow computers and long timelines.", 270),
];

/// The "Preview: Half" button in the viewer header and a function that makes
/// it show a given setting.
fn quality_button(state: &Rc<AppState>) -> (gtk::MenuButton, impl Fn(u32)) {
    let list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).margin_top(8).margin_bottom(8).margin_start(8).margin_end(8).build();
    let heading = label("Preview quality", &["tempo-heading"]);
    heading.set_xalign(0.0);
    list.append(&heading);
    let note = label("Lower quality plays more smoothly. Export always uses full quality.", &["tempo-dim", "tempo-small"]);
    note.set_xalign(0.0);
    note.set_margin_bottom(6);
    list.append(&note);

    let button = gtk::MenuButton::builder()
        .tooltip_text("Preview quality: how sharp the picture is while you edit. Lower is smoother.")
        .css_classes(["tool", "text"])
        .build();
    let mut items: Vec<gtk::CheckButton> = Vec::new();
    for (name, about, height) in QUALITIES {
        let item = gtk::CheckButton::with_label(name);
        if let Some(first) = items.first() {
            item.set_group(Some(first));
        }
        let s = state.clone();
        let b = button.clone();
        item.connect_toggled(move |i| {
            if i.is_active() {
                s.set_playback_height(height);
                b.popdown();
            }
        });
        list.append(&item);
        let about = label(about, &["tempo-dim", "tempo-small"]);
        about.set_xalign(0.0);
        about.set_margin_start(28);
        about.set_margin_bottom(4);
        list.append(&about);
        items.push(item);
    }
    button.set_popover(Some(&gtk::Popover::builder().child(&list).build()));

    let b = button.clone();
    let sync = move |height: u32| {
        let index = QUALITIES.iter().position(|q| q.2 == height).unwrap_or(1);
        b.set_label(&format!("Preview: {}", QUALITIES[index].0));
        items[index].set_active(true);
    };
    sync(state.settings.borrow().playback_height);
    (button, sync)
}

/// The main viewer's menu: the dual-viewer switch.
fn viewer_menu() -> (gtk::Popover, gtk::CheckButton) {
    let list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).margin_top(8).margin_bottom(8).margin_start(8).margin_end(8).build();
    let check = gtk::CheckButton::with_label("Dual viewer");
    check.set_tooltip_text(Some("Show the source clip in its own viewer beside the timeline viewer"));
    list.append(&check);
    (gtk::Popover::builder().child(&list).build(), check)
}
