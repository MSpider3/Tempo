//! The timeline: one custom-drawn canvas (ruler, tracks, clips, markers, playhead)
//! plus a column of real widgets for the track headers.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use glib::subclass::prelude::*;
use gtk4 as gtk;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use gtk4::{gdk, gio, graphene, gsk, pango};
use tempo_timeline::{
    AddTrackCommand, AudioTrackType, Clip, ClipEffect, ClipType, Command, CompositeCommand, DeleteTrackCommand, EditClipCommand,
    FadeEdge, InsertClipCommand, MediaType, MoveClipCommand, SetTrackFlagCommand, SetTransitionCommand, Timeline, TitleData,
    TitleType, Track, TrackFlag, TrackKind, Transition, TransitionAlignment, TransitionEdge, TransitionKind, TrimEdge,
};
use uuid::Uuid;

use crate::actions::{self, Place};
use crate::keybinds::HintZone;
use crate::state::{AppState, Change, EditSelection, Tool};
use crate::util::{css_color, label};
use crate::viewer::Viewer;

/// Timecode over name: the tooltip of a marker.
fn marker_tip(timecode: &str, name: &str) -> String {
    if name.is_empty() {
        timecode.to_string()
    } else {
        format!("{timecode}\n{name}")
    }
}

pub const RULER_H: f32 = 48.0;
/// Empty space above the top video track, where a clip dragged there gets a new track.
const GHOST_BAND_H: f32 = VIDEO_H;
const VIDEO_H: f32 = 60.0;
const AUDIO_H: f32 = 54.0;
const MIN_TRACK_H: f32 = 36.0;
const MAX_TRACK_H: f32 = 140.0;
const DIVIDER_H: f32 = 2.0;

thread_local! {
    static TRACK_HEIGHTS: Cell<(f32, f32)> = const { Cell::new((VIDEO_H, AUDIO_H)) };
}

fn adjust_track_height(current: f32, delta: f32) -> f32 {
    (current + delta).clamp(MIN_TRACK_H, MAX_TRACK_H)
}
const NAME_BAR_H: f32 = 22.0;
const EDGE_GRAB_PX: f64 = 6.0;
/// A press becomes a drag only after the pointer has travelled this far.
const DRAG_START_PX: f64 = 4.0;
/// How wide a marker is to grab in the ruler, either side of its line.
const MARKER_GRAB_PX: f64 = 6.0;
/// Detail Zoom shows about this many seconds across the view.
const DETAIL_ZOOM_SECONDS: f64 = 4.0;
const SNAP_PX: f64 = 8.0;
/// Fade handles: how far from a clip's top corner they can be grabbed, and their size.
const FADE_GRAB_PX: f64 = 7.0;
const FADE_HANDLE_PX: f32 = 8.0;
/// How long an edit takes to settle on screen.
const TWEEN_US: i64 = 120_000;

fn ease_out(p: f64) -> f64 {
    let q = 1.0 - p.clamp(0.0, 1.0);
    1.0 - q * q * q
}

fn lerp_i(from: i64, to: i64, e: f64) -> i64 {
    from + ((to - from) as f64 * e).round() as i64
}

/// The new tracks a Media Pool source dragged to `y` asks for, as (video, audio):
/// a source with a picture over the band above the top video track, an audio-only
/// source below the last row. `rows_bottom` is where the last track row ends.
fn ghost_lanes(y: f64, rows_bottom: f64, media: MediaType, has_audio: bool) -> Option<(bool, bool)> {
    let in_band = (RULER_H as f64..(RULER_H + GHOST_BAND_H) as f64).contains(&y);
    match media {
        MediaType::Audio => (y >= rows_bottom).then_some((false, true)),
        MediaType::Video => in_band.then_some((true, has_audio)),
        MediaType::Image => in_band.then_some((true, false)),
    }
}

/// What a transition dragged from the Effects panel carries, before its kind.
pub const TRANSITION_PREFIX: &str = "transition:";
pub const MIN_PPS: f64 = 0.5;
pub const MAX_PPS: f64 = 400.0;

/// One track row. `y` is measured from the top of the canvas.
#[derive(Clone)]
pub struct Row {
    pub track_id: Uuid,
    pub kind: TrackKind,
    pub index: u32,
    pub y: f32,
    pub h: f32,
    pub locked: bool,
    pub enabled: bool,
    pub solo: bool,
}

/// How many times the timeline canvas has been redrawn; reported by `stats` for benchmarks.
static DRAWS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn draw_count() -> u64 {
    DRAWS.load(std::sync::atomic::Ordering::Relaxed)
}

/// Video tracks stack upward and audio tracks downward from the divider, as in Resolve.
pub fn rows(timeline: &Timeline) -> Vec<Row> {
    let (video_h, audio_h) = TRACK_HEIGHTS.with(|c| c.get());
    let mut video: Vec<_> = timeline.tracks.iter().filter(|t| t.kind == TrackKind::Video).collect();
    video.sort_by_key(|t| std::cmp::Reverse(t.kind_index));
    let mut audio: Vec<_> = timeline.tracks.iter().filter(|t| t.kind == TrackKind::Audio).collect();
    audio.sort_by_key(|t| t.kind_index);

    let mut out = Vec::new();
    let mut y = RULER_H + GHOST_BAND_H;
    for (list, h, is_video) in [(video, video_h, true), (audio, audio_h, false)] {
        for t in list {
            out.push(Row { track_id: t.id, kind: t.kind, index: t.kind_index, y, h, locked: t.locked, enabled: t.enabled, solo: t.solo });
            y += h;
        }
        if is_video {
            y += DIVIDER_H;
        }
    }
    out
}

/// The three zoom preset buttons left of the zoom slider.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ZoomPreset {
    Full,
    Detail,
    Custom,
}

/// Pixels per second that fit `seconds` across a view `width` pixels wide.
fn detail_pps(width: f64, seconds: f64) -> f64 {
    (width / seconds).clamp(MIN_PPS, MAX_PPS)
}

/// `ss:ff` with a leading `-` when negative, for the tooltip of a drag.
fn offset_label(us: i64, fps: f64) -> String {
    let fps_i = fps.round().max(1.0) as i64;
    let frames = (us.unsigned_abs() as f64 * fps / 1_000_000.0 + 0.1).floor() as i64;
    format!("{}{:02}:{:02}", if us < 0 { "-" } else { "" }, frames / fps_i, frames % fps_i)
}

/// Resolve's tooltip while a transition's edge is dragged: how far the edge under the
/// pointer has moved (right is +, left is -), over the new length. A transition centred
/// on a cut moves both edges, so the length changes twice as fast as the one edge.
fn length_tip(from: i64, to: i64, left_edge: bool, both_free: bool, fps: f64) -> String {
    let grown = (to - from) / if both_free { 2 } else { 1 };
    let moved = if left_edge { -grown } else { grown };
    format!("{}{}\n{}", if moved >= 0 { "+" } else { "" }, offset_label(moved, fps), offset_label(to, fps))
}

/// A centred transition moves both edges, so its length changes in whole 2-frame steps
/// (each edge moves whole frames); an odd step falls back one frame toward `from`. The
/// caller still caps the result at the longest length the footage allows.
fn even_steps(from: i64, to: i64, both_free: bool, frame_us: i64) -> i64 {
    let frames = ((to - from) as f64 / frame_us.max(1) as f64).round() as i64;
    if both_free && frames % 2 != 0 { to - frames.signum() * frame_us } else { to }
}

/// Size of a probe thumbnail (raw RGBA) from its byte count.
fn thumbnail_size(len: usize) -> Option<(usize, usize)> {
    [(160, 90), (320, 180), (96, 54)].into_iter().find(|(w, h)| w * h * 4 == len)
}

/// The name a transition box shows, as Resolve names them.
fn transition_name(kind: &TransitionKind) -> String {
    kind.resolve_name()
}

/// Top and height of the transition boxes in a row: below the fade handles, above the name bar.
fn transition_band(row: &Row) -> (f32, f32) {
    (row.y + 6.0, row.h - NAME_BAR_H - 10.0)
}

/// What dropping something from the Effects panel would do.
enum DropPlan {
    /// Put this transition on this row's track.
    Transition(Row, Transition),
    /// Fade this clip (the fade kinds).
    Fade(Row, Uuid),
}

/// The marker whose line is within `grab_px` of `x`, the nearest one.
fn marker_near(markers: &[tempo_timeline::Marker], x: f64, grab_px: f64, x_of: impl Fn(i64) -> f64) -> Option<&tempo_timeline::Marker> {
    let dist = |m: &&tempo_timeline::Marker| (x_of(m.position_us) - x).abs();
    markers.iter().filter(|m| dist(m) <= grab_px).min_by(|a, b| dist(a).total_cmp(&dist(b)))
}

#[derive(Clone)]
enum Drag {
    Scrub,
    /// A marker dragged along the ruler: where it was and where it would land.
    Marker { id: Uuid, from: i64, to: i64 },
    Move { clip: Uuid, from_track: Uuid, kind: TrackKind, orig_in: i64, len: i64, new_in: i64, to_track: Uuid, duplicate: bool },
    /// `roll` moves the cut between two touching clips (Shift in Trim mode).
    Trim { clip: Uuid, edge: TrimEdge, delta: i64, roll: bool },
    /// Dragging a fade handle: the fade length at the start of the drag and now.
    Fade { clip: Uuid, edge: FadeEdge, from: i64, to: i64 },
    /// Dragging a free edge of a transition (`left` or right; `both` when both edges are
    /// free, centred on a cut): its length then and now, and the most it can be.
    Transition { track: Uuid, clip: Uuid, edge: TransitionEdge, left: bool, both: bool, from: i64, to: i64, max: i64 },
}

mod imp {
    use super::*;

    pub struct TimelineCanvas {
        pub state: RefCell<Option<Rc<AppState>>>,
        pub viewer: RefCell<Option<Rc<Viewer>>>,
        pub pps: Cell<f64>,
        pub scroll: Cell<f64>,
        pub playhead: Cell<i64>,
        pub video_h: Cell<f32>,
        pub audio_h: Cell<f32>,
        pub heights_cb: RefCell<Option<Rc<dyn Fn()>>>,
        pub(super) drag: RefCell<Option<Drag>>,
        pub press: Cell<(f64, f64)>,
        /// True once the pointer has moved far enough for the press to be a drag.
        pub dragging: Cell<bool>,
        /// Modifier keys held when the pointer went down and when released.
        pub press_mods: Cell<gdk::ModifierType>,
        pub release_mods: Cell<gdk::ModifierType>,
        pub pointer_x: Cell<f64>,
        /// The clip under the pointer, which shows its fade handles.
        pub hover: Cell<Option<Uuid>>,
        pub snap_line: Cell<Option<i64>>,
        /// Where something dragged from the Effects panel would land: track, start, end.
        pub drop_at: Cell<Option<(Uuid, i64, i64)>>,
        /// A Media Pool clip is over where new tracks would be made for it: (video, audio).
        pub ghost: Cell<Option<(bool, bool)>>,
        /// Told whenever `ghost` changes, so the track headers can show their ghost rows.
        pub ghost_cb: RefCell<Option<Rc<dyn Fn(Option<(bool, bool)>)>>>,
        /// Where a Media Pool clip dragged over the timeline would land: start, end, whether
        /// it brings sound, and the pointer's y.
        pub preview: Cell<Option<(i64, i64, bool, f64)>>,
        /// The Media Pool source being dragged, and the textures of source thumbnails.
        pub drop_source: Cell<Option<Uuid>>,
        pub thumbs: RefCell<HashMap<Uuid, Option<gdk::Texture>>>,
        /// Edit animation: 1.0 at rest, else how far the move from the `from` values has run.
        pub tween_e: Cell<f64>,
        pub tween_t0: Cell<i64>,
        pub tween_on: Cell<bool>,
        pub tween_from: RefCell<HashMap<Uuid, (i64, i64)>>,
        /// Where every clip is in the model, as of the last change.
        pub tween_to: RefCell<HashMap<Uuid, (i64, i64)>>,
        pub ph_from: Cell<i64>,
        pub scroll_from: Cell<f64>,
        /// Edits before this time (monotonic us) are the end of a drag, already on screen.
        pub no_tween_until: Cell<i64>,
        pub hadj: gtk::Adjustment,
        pub zoom_listener: RefCell<Option<Box<dyn Fn(f64)>>>,
        /// Zoom-to-fit was asked for before the widget had a size.
        pub fit_pending: Cell<bool>,
        /// Which zoom preset button is pressed, and the zoom the user last set by hand.
        pub preset: Cell<ZoomPreset>,
        pub custom_pps: Cell<f64>,
        /// A small black tooltip drawn at the pointer: x, y, text.
        pub tip: RefCell<Option<(f64, f64, String)>>,
        /// The right-click menu while it is open.
        pub menu: RefCell<Option<gtk::PopoverMenu>>,
        /// The playhead, drawn on its own layer above the canvas so moving it does
        /// not redraw every clip.
        pub playhead_layer: RefCell<Option<super::PlayheadLayer>>,
    }

    impl Default for TimelineCanvas {
        fn default() -> Self {
            Self {
                state: RefCell::new(None),
                viewer: RefCell::new(None),
                pps: Cell::new(20.0),
                scroll: Cell::new(0.0),
                playhead: Cell::new(0),
                video_h: Cell::new(VIDEO_H),
                audio_h: Cell::new(AUDIO_H),
                heights_cb: RefCell::new(None),
                drag: RefCell::new(None),
                press: Cell::new((0.0, 0.0)),
                dragging: Cell::new(false),
                press_mods: Cell::new(gdk::ModifierType::empty()),
                release_mods: Cell::new(gdk::ModifierType::empty()),
                pointer_x: Cell::new(0.0),
                hover: Cell::new(None),
                drop_at: Cell::new(None),
                ghost: Cell::new(None),
                ghost_cb: RefCell::new(None),
                preview: Cell::new(None),
                drop_source: Cell::new(None),
                thumbs: RefCell::new(HashMap::new()),
                tween_e: Cell::new(1.0),
                tween_t0: Cell::new(0),
                tween_on: Cell::new(false),
                tween_from: RefCell::new(HashMap::new()),
                tween_to: RefCell::new(HashMap::new()),
                ph_from: Cell::new(0),
                scroll_from: Cell::new(0.0),
                no_tween_until: Cell::new(0),
                snap_line: Cell::new(None),
                hadj: gtk::Adjustment::new(0.0, 0.0, 1.0, 40.0, 200.0, 1.0),
                zoom_listener: RefCell::new(None),
                fit_pending: Cell::new(false),
                preset: Cell::new(ZoomPreset::Full),
                custom_pps: Cell::new(20.0),
                tip: RefCell::new(None),
                menu: RefCell::new(None),
                playhead_layer: RefCell::new(None),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for TimelineCanvas {
        const NAME: &'static str = "TempoTimelineCanvas";
        type Type = super::TimelineCanvas;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for TimelineCanvas {
        fn dispose(&self) {
            if let Some(menu) = self.menu.take() {
                menu.unparent();
            }
        }
    }

    impl WidgetImpl for TimelineCanvas {
        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            self.parent_size_allocate(width, height, baseline);
            if self.fit_pending.get() {
                self.obj().zoom_fit();
            }
            self.obj().update_adjustment();
            // A popover belongs to this widget, so this widget places it.
            if let Some(menu) = self.menu.borrow().as_ref() {
                menu.present();
            }
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            self.obj().draw(snapshot);
        }
    }
}

glib::wrapper! {
    pub struct TimelineCanvas(ObjectSubclass<imp::TimelineCanvas>)
        @extends gtk::Widget, @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl TimelineCanvas {
    pub fn new(state: &Rc<AppState>, viewer: &Rc<Viewer>) -> Self {
        TRACK_HEIGHTS.with(|c| c.set((VIDEO_H, AUDIO_H)));
        let obj: Self = glib::Object::builder().build();
        obj.set_hexpand(true);
        obj.set_vexpand(true);
        obj.set_focusable(true);
        // Clips scrolled past the left edge must not be drawn over the track headers.
        obj.set_overflow(gtk::Overflow::Hidden);
        obj.update_property(&[gtk::accessible::Property::Label("Timeline")]);
        *obj.imp().state.borrow_mut() = Some(state.clone());
        *obj.imp().viewer.borrow_mut() = Some(viewer.clone());
        obj.install_controllers();

        let c = obj.downgrade();
        state.connect(move |change| {
            if let Some(c) = c.upgrade() {
                match change {
                    Change::Project => {
                        c.imp().scroll.set(0.0);
                        c.imp().tween_e.set(1.0);
                        *c.imp().tween_to.borrow_mut() = c.clip_spans();
                        c.update_adjustment();
                        c.redraw();
                    }
                    Change::Timeline => c.on_timeline_changed(),
                    Change::Selection | Change::Options | Change::Waveform => c.redraw(),
                    _ => {}
                }
            }
        });

        let c = obj.downgrade();
        viewer.connect_position(move |_| {
            if let Some(c) = c.upgrade() {
                // The timeline's own playhead, not the position inside a source clip.
                c.set_playhead(c.state().playhead_us());
            }
        });

        let c = obj.downgrade();
        obj.imp().hadj.connect_value_changed(move |adj| {
            if let Some(c) = c.upgrade() {
                if (c.imp().scroll.get() - adj.value()).abs() > 0.5 {
                    c.imp().scroll.set(adj.value());
                    c.imp().scroll_from.set(adj.value());
                    c.redraw();
                }
            }
        });
        obj
    }

    fn state(&self) -> Rc<AppState> {
        // Set once in `new`; the fallback is an empty state with no project.
        self.imp().state.borrow_mut().get_or_insert_with(AppState::new).clone()
    }

    pub fn hadjustment(&self) -> gtk::Adjustment {
        self.imp().hadj.clone()
    }

    pub fn pps(&self) -> f64 {
        self.imp().pps.get()
    }

    /// The scroll as drawn: it eases to a new value after an edit moved it.
    fn shown_scroll(&self) -> f64 {
        let imp = self.imp();
        let e = imp.tween_e.get();
        if e >= 1.0 { imp.scroll.get() } else { imp.scroll_from.get() + (imp.scroll.get() - imp.scroll_from.get()) * e }
    }

    fn shown_playhead(&self) -> i64 {
        let imp = self.imp();
        lerp_i(imp.ph_from.get(), imp.playhead.get(), imp.tween_e.get())
    }

    fn x_of(&self, us: i64) -> f64 {
        us as f64 / 1_000_000.0 * self.pps() - self.shown_scroll()
    }

    fn us_at(&self, x: f64) -> i64 {
        (((x + self.shown_scroll()) / self.pps()) * 1_000_000.0).max(0.0) as i64
    }

    fn clip_spans(&self) -> HashMap<Uuid, (i64, i64)> {
        let spans = |t: &Timeline| t.tracks.iter().flat_map(|tr| tr.clips.iter().map(|c| (c.id, (c.timeline_in, c.timeline_out)))).collect();
        self.state().with_timeline(spans).unwrap_or_default()
    }

    /// A clip's span as drawn: moving from where it was shown to where the model has it.
    fn tweened(&self, id: Uuid, tin: i64, tout: i64) -> (i64, i64) {
        let e = self.imp().tween_e.get();
        match self.imp().tween_from.borrow().get(&id) {
            Some(&(a, b)) if e < 1.0 => (lerp_i(a, tin, e), lerp_i(b, tout, e)),
            _ => (tin, tout),
        }
    }

    /// The timeline changed: ease clips, playhead and scroll from what was on screen to
    /// the new place, unless the user dragged it there or it is playing (drawing only).
    fn on_timeline_changed(&self) {
        let imp = self.imp();
        let state = self.state();
        let e = imp.tween_e.get();
        let (shown_scroll, shown_playhead) = (self.shown_scroll(), self.shown_playhead());
        let new = self.clip_spans();
        let still = imp.drag.borrow().is_some() || state.player.is_playing() || glib::monotonic_time() < imp.no_tween_until.get();
        let mut from = HashMap::new();
        if !still {
            let (old, was) = (imp.tween_to.borrow(), imp.tween_from.borrow());
            for (id, now) in &new {
                let Some(&before) = old.get(id) else { continue };
                let (a, b) = was.get(id).map_or(before, |&(a, b)| (lerp_i(a, before.0, e), lerp_i(b, before.1, e)));
                if (a, b) != *now {
                    from.insert(*id, (a, b));
                }
            }
        }
        // An edit may have moved the playhead (after an insert, say).
        imp.playhead.set(state.playhead_us());
        self.update_adjustment();
        let moved = !from.is_empty() || (!still && (shown_playhead != imp.playhead.get() || (shown_scroll - imp.scroll.get()).abs() > 0.5));
        *imp.tween_to.borrow_mut() = new;
        *imp.tween_from.borrow_mut() = from;
        imp.ph_from.set(shown_playhead);
        imp.scroll_from.set(shown_scroll);
        if moved {
            self.start_tween();
        } else {
            imp.tween_e.set(1.0);
        }
        // Keep destination track indices pointing at an existing track after deletions/undos.
        if let Some((v_fix, a_fix)) = state.with_timeline(|t| {
            let fix = |kind: TrackKind, cur: u32| {
                if t.tracks.iter().any(|tr| tr.kind == kind && tr.kind_index == cur) {
                    None
                } else {
                    Some(t.tracks.iter().filter(|tr| tr.kind == kind).map(|tr| tr.kind_index).max().unwrap_or(1))
                }
            };
            (fix(TrackKind::Video, state.dest_video.get()), fix(TrackKind::Audio, state.dest_audio.get()))
        }) {
            if let Some(v) = v_fix {
                state.dest_video.set(v);
            }
            if let Some(a) = a_fix {
                state.dest_audio.set(a);
            }
            if v_fix.is_some() || a_fix.is_some() {
                state.emit(Change::Options);
            }
        }
        self.redraw();
    }

    pub fn connect_heights(&self, f: impl Fn() + 'static) {
        *self.imp().heights_cb.borrow_mut() = Some(Rc::new(f));
    }

    pub fn adjust_track_heights(&self, delta_px: f32) {
        let imp = self.imp();
        let vh = adjust_track_height(imp.video_h.get(), delta_px);
        let ah = adjust_track_height(imp.audio_h.get(), delta_px);
        if (vh - imp.video_h.get()).abs() < f32::EPSILON && (ah - imp.audio_h.get()).abs() < f32::EPSILON {
            return;
        }
        imp.video_h.set(vh);
        imp.audio_h.set(ah);
        TRACK_HEIGHTS.with(|c| c.set((vh, ah)));
        if let Some(cb) = imp.heights_cb.borrow().as_ref() {
            cb();
        }
        self.queue_resize();
        self.redraw();
    }

    fn start_tween(&self) {
        let imp = self.imp();
        imp.tween_e.set(0.0);
        imp.tween_t0.set(0);
        if imp.tween_on.replace(true) {
            return;
        }
        self.add_tick_callback(|canvas, clock| {
            let imp = canvas.imp();
            let now = clock.frame_time();
            if imp.tween_t0.get() == 0 {
                imp.tween_t0.set(now);
            }
            let p = (now - imp.tween_t0.get()) as f64 / TWEEN_US as f64;
            imp.tween_e.set(ease_out(p));
            canvas.redraw();
            if p >= 1.0 {
                imp.tween_on.set(false);
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    fn content_width(&self) -> f64 {
        let dur = self.state().with_timeline(|t| t.tracks.iter().map(|tr| tr.duration_us()).max().unwrap_or(0)).unwrap_or(0);
        // Leave room after the last clip so there is always somewhere to drop.
        dur as f64 / 1_000_000.0 * self.pps() + self.width() as f64 * 0.5 + 200.0
    }

    fn update_adjustment(&self) {
        let imp = self.imp();
        let page = self.width().max(1) as f64;
        let upper = self.content_width().max(page);
        let value = imp.scroll.get().clamp(0.0, upper - page);
        imp.scroll.set(value);
        imp.hadj.configure(value, 0.0, upper, 40.0, page * 0.8, page);
    }

    fn set_scroll(&self, px: f64) {
        self.imp().scroll.set(px.max(0.0));
        self.update_adjustment();
        self.redraw();
    }

    /// Change zoom keeping the time under `anchor_x` in place.
    pub fn set_pps(&self, pps: f64, anchor_x: Option<f64>) {
        let anchor = anchor_x.unwrap_or(self.x_of(self.imp().playhead.get()).clamp(0.0, self.width() as f64));
        let t = self.us_at(anchor);
        self.imp().pps.set(pps.clamp(MIN_PPS, MAX_PPS));
        // Zooming by hand is the Custom Zoom, which the preset button returns to.
        self.imp().custom_pps.set(self.pps());
        self.imp().preset.set(ZoomPreset::Custom);
        self.set_scroll(t as f64 / 1_000_000.0 * self.pps() - anchor);
        self.notify_zoom();
    }

    pub fn preset(&self) -> ZoomPreset {
        self.imp().preset.get()
    }

    /// Zoom to `pps` with the playhead in the middle of the view.
    fn zoom_centred(&self, pps: f64, preset: ZoomPreset) {
        let imp = self.imp();
        imp.pps.set(pps.clamp(MIN_PPS, MAX_PPS));
        imp.preset.set(preset);
        self.set_scroll(imp.playhead.get() as f64 / 1_000_000.0 * self.pps() - self.width() as f64 / 2.0);
        self.notify_zoom();
    }

    /// Detail Zoom: a few seconds across the view, centred on the playhead.
    pub fn zoom_detail(&self) {
        self.zoom_centred(detail_pps(self.width() as f64, DETAIL_ZOOM_SECONDS), ZoomPreset::Detail);
    }

    /// Custom Zoom: back to the zoom last set by hand, centred on the playhead.
    pub fn zoom_custom(&self) {
        self.zoom_centred(self.imp().custom_pps.get(), ZoomPreset::Custom);
    }

    /// Called with the new pixels-per-second whenever the zoom changes.
    pub fn connect_zoom(&self, f: impl Fn(f64) + 'static) {
        *self.imp().zoom_listener.borrow_mut() = Some(Box::new(f));
    }

    fn notify_zoom(&self) {
        if let Some(f) = self.imp().zoom_listener.borrow().as_ref() {
            f(self.pps());
        }
    }

    pub fn zoom_fit(&self) {
        if self.width() < 100 {
            self.imp().fit_pending.set(true);
            self.imp().preset.set(ZoomPreset::Full);
            return;
        }
        self.imp().fit_pending.set(false);
        self.imp().preset.set(ZoomPreset::Full);
        let end = self.state().with_timeline(|t| t.tracks.iter().map(|tr| tr.duration_us()).max().unwrap_or(0)).unwrap_or(0);
        // An empty timeline shows its first minute.
        let dur = if end > 0 { end as f64 / 1_000_000.0 } else { 60.0 };
        self.imp().pps.set(((self.width() as f64 - 40.0) / dur).clamp(MIN_PPS, MAX_PPS));
        self.set_scroll(0.0);
        self.notify_zoom();
    }

    fn set_playhead(&self, us: i64) {
        if self.imp().playhead.replace(us) == us {
            return;
        }
        self.imp().ph_from.set(us);
        // Keep the playhead in view: while playing, and after a key moved it
        // (End, next cut, next marker). Not while the user drags it by hand.
        let scrubbing = matches!(*self.imp().drag.borrow(), Some(Drag::Scrub));
        let (x, w) = (self.x_of(us), self.width() as f64);
        if !scrubbing && w > 100.0 && (x > w - 20.0 || x < 0.0) {
            let lead = if self.state().player.is_playing() { 0.1 } else { 0.4 };
            self.set_scroll(us as f64 / 1_000_000.0 * self.pps() - w * lead);
            return;
        }
        // Only the playhead moved: the clips need not be drawn again.
        self.redraw_playhead();
    }

    fn row_at(&self, y: f64) -> Option<Row> {
        let state = self.state();
        let rows = state.with_timeline(rows).unwrap_or_default();
        rows.into_iter().find(|r| y >= r.y as f64 && y < (r.y + r.h) as f64)
    }

    /// Clip under a point, and which edge (if any) the pointer is close to.
    /// An edge can be grabbed from a few pixels either side of it, so the end of
    /// a clip is reachable from the empty space after it. At a cut, the left
    /// side of the line grabs the end of the left clip and the right side the
    /// start of the right clip.
    fn hit(&self, x: f64, y: f64) -> Option<(Row, Uuid, Option<TrimEdge>)> {
        let row = self.row_at(y)?;
        let t = self.us_at(x);
        let state = self.state();
        let clips: Vec<(Uuid, i64, i64)> = state
            .with_timeline(|tl| tl.find_track(row.track_id).map(|tr| tr.clips.iter().map(|c| (c.id, c.timeline_in, c.timeline_out)).collect()))
            .flatten()?;

        let mut best: Option<(f64, Uuid, TrimEdge)> = None;
        for (id, tin, tout) in &clips {
            for (edge_us, edge) in [(*tin, TrimEdge::In), (*tout, TrimEdge::Out)] {
                let edge_x = self.x_of(edge_us);
                let distance = (x - edge_x).abs();
                // Each edge is grabbed from its own clip's side of a shared cut.
                let own_side = match edge {
                    TrimEdge::In => x >= edge_x,
                    TrimEdge::Out => x < edge_x,
                };
                let shared = clips.iter().any(|(other, oin, oout)| other != id && (*oin == edge_us || *oout == edge_us));
                // A narrow clip keeps its middle half for moving: only its outer quarters trim.
                let width = self.x_of(*tout) - self.x_of(*tin);
                let reach = if (self.x_of(*tin)..self.x_of(*tout)).contains(&x) { EDGE_GRAB_PX.min(width / 4.0) } else { EDGE_GRAB_PX };
                if distance <= reach && (own_side || !shared) && best.as_ref().is_none_or(|(d, ..)| distance < *d) {
                    best = Some((distance, *id, edge));
                }
            }
        }
        if let Some((_, id, edge)) = best {
            return Some((row, id, Some(edge)));
        }
        clips.iter().find(|(_, tin, tout)| t >= *tin && t < *tout).map(|(id, ..)| (row, *id, None))
    }

    /// The nearest thing to stick to: a clip edge, a marker, the playhead or the
    /// start of the timeline. Clips in `ignore` (the ones being moved) do not count,
    /// or a clip would stick to where it and its linked partner already are.
    fn snap(&self, us: i64, ignore: &[Uuid]) -> (i64, bool) {
        self.snap_skipping(us, ignore, None)
    }

    /// `snap`, without the marker `skip_marker` (the one being dragged).
    fn snap_skipping(&self, us: i64, ignore: &[Uuid], skip_marker: Option<Uuid>) -> (i64, bool) {
        let state = self.state();
        if !state.snapping.get() {
            return (us, false);
        }
        let threshold = (SNAP_PX / self.pps() * 1_000_000.0) as i64;
        let playhead = self.imp().playhead.get();
        let best = state
            .with_timeline(|t| {
                let edges = t.tracks.iter().flat_map(|tr| tr.clips.iter()).filter(|c| !ignore.contains(&c.id)).flat_map(|c| [c.timeline_in, c.timeline_out]);
                let markers = t.markers.iter().filter(|m| Some(m.id) != skip_marker).map(|m| m.position_us);
                edges.chain(markers).chain([0, playhead]).filter(|p| (p - us).abs() <= threshold).min_by_key(|p| (p - us).abs())
            })
            .flatten();
        match best {
            Some(b) => (b, true),
            None => (us, false),
        }
    }

    /// The clips that move or trim together with `clip` in a drag.
    fn moving_with(&self, clip: Uuid) -> Vec<Uuid> {
        let state = self.state();
        if state.is_selected(clip) {
            state.selected_ids()
        } else {
            // An edge grabbed as an edit point trims its linked partner too (`actions::trim_clip`).
            std::iter::once(clip).chain(state.linked_to(clip)).collect()
        }
    }

    /// Screen position of a time on a track ("V1", "A2" …). For scripted checks.
    pub fn point_for(&self, seconds: f64, track: &str) -> Option<(f64, f64)> {
        let rows = self.state().with_timeline(rows).unwrap_or_default();
        let row = rows.into_iter().find(|r| {
            let prefix = if r.kind == TrackKind::Video { "V" } else { "A" };
            format!("{prefix}{}", r.index).eq_ignore_ascii_case(track)
        })?;
        Some((self.x_of((seconds * 1_000_000.0) as i64), (row.y + row.h / 2.0) as f64))
    }

    /// Run a pointer press, drag and release through the same handlers the mouse
    /// uses. For scripted checks of the editing gestures.
    /// Like `point_for`, but at the top edge of the row, where fade handles are.
    pub fn top_point_for(&self, seconds: f64, track: &str) -> Option<(f64, f64)> {
        let (x, y) = self.point_for(seconds, track)?;
        let row = self.row_at(y)?;
        Some((x, row.y as f64 + 3.0))
    }

    /// Like `point_for`, but low in the row, below the transition boxes, where a click
    /// on a clip edge selects the edit point.
    pub fn bottom_point_for(&self, seconds: f64, track: &str) -> Option<(f64, f64)> {
        let (x, y) = self.point_for(seconds, track)?;
        let row = self.row_at(y)?;
        Some((x, (row.y + row.h) as f64 - 8.0))
    }

    pub fn simulate_drag(&self, from: (f64, f64), to: (f64, f64), mods: gdk::ModifierType) {
        self.imp().press_mods.set(mods);
        self.imp().release_mods.set(mods);
        self.on_press(from.0, from.1);
        if from != to {
            // A few intermediate points, as a real drag would produce.
            for step in 1..=4 {
                let t = step as f64 / 4.0;
                self.on_drag(from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t);
            }
        }
        self.on_release();
    }

    /// A fade handle under the pointer: near a clip's top corner, or where its fade
    /// ends. Locked tracks have none.
    fn fade_hit(&self, x: f64, y: f64) -> Option<(Uuid, FadeEdge, i64)> {
        // Only the top strip of an unlocked row holds fade handles.
        self.row_at(y).filter(|r| !r.locked && y - (r.y as f64) < 2.0 * FADE_GRAB_PX)?;
        let (_, id, _) = [0.0, -FADE_GRAB_PX, FADE_GRAB_PX].into_iter().find_map(|dx| self.hit(x + dx, y))?;
        let (tin, tout, fade_in, fade_out) = self.state().with_timeline(|t| {
            t.find_clip(id).map(|(_, c)| (c.timeline_in, c.timeline_out, c.properties.fade_in_us, c.properties.fade_out_us))
        })??;
        [(FadeEdge::In, tin + fade_in, fade_in), (FadeEdge::Out, tout - fade_out, fade_out)]
            .into_iter()
            .filter(|(_, at, _)| (self.x_of(*at) - x).abs() <= FADE_GRAB_PX)
            .min_by(|a, b| (self.x_of(a.1) - x).abs().total_cmp(&(self.x_of(b.1) - x).abs()))
            .map(|(edge, _, length)| (id, edge, length))
    }

    /// The transition box under the pointer, and the free edge (`left`, `both` free) it
    /// is near, if any. An edge on the cut, or on the clip's end, stays put and is not free.
    fn transition_hit(&self, x: f64, y: f64) -> Option<(Row, Transition, Option<(bool, bool)>)> {
        let row = self.row_at(y)?;
        let (top, height) = transition_band(&row);
        if !(top as f64..=(top + height) as f64).contains(&y) {
            return None;
        }
        let found = self.state().with_timeline(|tl| {
            let track = tl.find_track(row.track_id)?;
            track.transitions.iter().filter(|t| t.kind != TransitionKind::Cut).find_map(|t| {
                let (start, end) = track.transition_span(t)?;
                let edit = actions::transition_edit_us(track, t)?;
                let (x0, x1) = (self.x_of(start), self.x_of(end));
                let reach = EDGE_GRAB_PX.min((x1 - x0) / 3.0);
                let (left, right) = (start != edit && (x - x0).abs() <= reach, end != edit && (x - x1).abs() <= reach);
                let edge = (left || right).then_some((left, start != edit && end != edit));
                (edge.is_some() || (x0..=x1).contains(&x)).then(|| (t.clone(), edge))
            })
        });
        found.flatten().map(|(t, edge)| (row, t, edge))
    }

    /// What dropping `kind` from the Effects panel at a point would do, or why it cannot.
    /// A transition goes on the edit nearest the pointer on its matching track kind (see
    /// `actions::fit_transition`); a fade on the clip under the pointer.
    fn plan_drop(&self, kind: &str, x: f64, y: f64) -> Result<DropPlan, &'static str> {
        const WHERE: &str = "Drop a transition on a cut, or on the start or end of a clip.";
        let row = self.row_at(y).ok_or(WHERE)?;
        if row.locked {
            return Err("That track is locked. Click its lock in the track header to edit it.");
        }
        let kind = match kind {
            "fade-in" | "fade-out" | "fade-in-out" => {
                if row.kind == TrackKind::Audio {
                    return Err("Video transitions go on video tracks.");
                }
                return self.hit(x, y).map(|(row, id, _)| DropPlan::Fade(row, id)).ok_or("Drop a fade on a clip.");
            }
            "crossfade" => TransitionKind::Crossfade,
            id => TransitionKind::from_effect_id(id).ok_or("Tempo does not know that transition.")?,
        };
        match (kind == TransitionKind::Crossfade, row.kind) {
            (true, TrackKind::Video) => return Err("Audio transitions go on audio tracks."),
            (false, TrackKind::Audio) => return Err("Video transitions go on video tracks."),
            _ => {}
        }
        let reach = (3.0 * EDGE_GRAB_PX / self.pps() * 1_000_000.0) as i64;
        let template = Transition::new(Uuid::nil(), TransitionEdge::In, kind, self.state().standard_transition_us(), TransitionAlignment::Centered);
        let at = self.us_at(x);
        self.state()
            .with_project(|p| {
                let track = p.timeline.find_track(row.track_id).ok_or(WHERE)?;
                let edit = actions::nearest_edit(track, at, reach).ok_or(WHERE)?;
                actions::fit_transition(track, &edit, &template, &p.sources, p.timeline_fps())
            })
            .unwrap_or(Err(WHERE))
            .map(|t| DropPlan::Transition(row, t))
    }

    fn plan_filter_drop(&self, filter_id: &str, x: f64, y: f64) -> Result<(Uuid, Uuid, i64, i64, ClipEffect), &'static str> {
        let state = self.state();
        let filter = state
            .filters
            .borrow()
            .iter()
            .find(|f| f.id == filter_id)
            .cloned()
            .ok_or("Unknown filter.")?;
        let (row, clip_id, _) = self.hit(x, y).ok_or("Drop a filter onto a video clip.")?;
        if row.locked {
            return Err("That track is locked. Click its lock in the track header to edit it.");
        }
        state
            .with_timeline(|t| {
                let (_, clip) = t.find_clip(clip_id)?;
                if matches!(clip.clip_type, ClipType::Audio | ClipType::Title) {
                    return Some(Err("Filters go on video or image clips."));
                }
                if clip.properties.effects.iter().any(|e| e.id == filter.id) {
                    return Some(Err("That clip already has this filter."));
                }
                Some(Ok((row.track_id, clip_id, clip.timeline_in, clip.timeline_out, filter)))
            })
            .flatten()
            .unwrap_or(Err("Drop a filter onto a video clip."))
    }

    fn drop_filter(&self, filter_id: &str, x: f64, y: f64) -> bool {
        let state = self.state();
        match self.plan_filter_drop(filter_id, x, y) {
            Err(why) => {
                state.message(why);
                false
            }
            Ok((_, clip_id, _, _, filter)) => {
                let edited = state
                    .with_timeline(|t| {
                        t.find_clip(clip_id).map(|(_, clip)| {
                            let mut c = clip.clone();
                            c.properties.effects.push(filter);
                            c
                        })
                    })
                    .flatten();
                let Some(edited) = edited else { return false };
                let done = state.execute(Box::new(EditClipCommand::new("Add Filter", edited)));
                if done {
                    state.select(Some(clip_id));
                }
                done
            }
        }
    }

    fn plan_title_drop(&self, kind: &str, x: f64, y: f64) -> Result<(Uuid, i64, i64, bool), &'static str> {
        const LENGTH_US: i64 = 5_000_000;
        let lower_third = match kind {
            "text" => false,
            "lower_third" => true,
            _ => return Err("Unknown title type."),
        };
        let state = self.state();
        let (snapped, _) = self.snap(self.us_at(x), &[]);
        let at = state.frame_floor(snapped.max(0));
        let dropped_video = self.row_at(y).filter(|r| r.kind == TrackKind::Video && !r.locked).map(|r| r.track_id);
        let free = state
            .with_timeline(|t| {
                let mut tracks: Vec<_> = t.tracks.iter().filter(|tr| tr.kind == TrackKind::Video && !tr.locked).collect();
                tracks.sort_by_key(|tr| (Some(tr.id) != dropped_video, tr.kind_index));
                tracks.into_iter().find_map(|tr| {
                    if tr.clip_at(at).is_some() {
                        return None;
                    }
                    let room = tr.clips.iter().map(|c| c.timeline_in).filter(|start| *start > at).min().map_or(LENGTH_US, |next| next - at);
                    (room >= 1_000_000).then(|| (tr.id, room.min(LENGTH_US)))
                })
            })
            .flatten();
        let (track_id, len) = free.ok_or("No free video track there for a title.")?;
        Ok((track_id, at, at + len, lower_third))
    }

    fn drop_title(&self, kind: &str, x: f64, y: f64) -> bool {
        let state = self.state();
        let (track, start, end, lower_third) = match self.plan_title_drop(kind, x, y) {
            Ok(plan) => plan,
            Err(why) => {
                state.message(why);
                return false;
            }
        };
        let length = end - start;
        let mut clip = Clip::new(
            track,
            Uuid::nil(),
            ClipType::Title,
            if lower_third { "Lower Third" } else { "Title" },
            start,
            end,
            0,
            length,
        );
        clip.title_data = Some(if lower_third {
            TitleData {
                title_type: TitleType::LowerThird,
                text: "Name".into(),
                font_size: 48.0,
                background_color: Some([0, 0, 0, 170]),
                ..Default::default()
            }
        } else {
            TitleData::default()
        });
        let id = clip.id;
        let done = state.execute(Box::new(InsertClipCommand::new(clip)));
        if done {
            state.select(Some(id));
        }
        done
    }

    fn clear_drop_preview(&self) {
        self.set_ghost(None);
        self.imp().drop_source.set(None);
        if self.imp().preview.take().is_some() {
            self.redraw();
        }
    }

    /// Where a drop would land: track and span. A fade shows its whole clip.
    fn drop_span(&self, plan: &DropPlan) -> Option<(Uuid, i64, i64)> {
        self.state()
            .with_timeline(|tl| match plan {
                DropPlan::Transition(row, t) => tl.find_track(row.track_id)?.transition_span(t).map(|(a, b)| (row.track_id, a, b)),
                DropPlan::Fade(row, id) => tl.find_clip(*id).map(|(_, c)| (row.track_id, c.timeline_in, c.timeline_out)),
            })
            .flatten()
    }

    /// Drop a transition from the Effects panel at a point, as Resolve does: on the edit
    /// nearest the pointer, replacing any transition there, and select it. Fades go on
    /// the clip under the pointer.
    pub fn drop_transition(&self, kind: &str, x: f64, y: f64) -> bool {
        let state = self.state();
        match self.plan_drop(kind, x, y) {
            Err(why) => {
                state.message(why);
                false
            }
            Ok(DropPlan::Fade(_, id)) => {
                state.select(Some(id));
                let seconds = actions::STANDARD_TRANSITION_S;
                if kind != "fade-out" {
                    actions::set_fade(&state, true, seconds);
                }
                if kind != "fade-in" {
                    actions::set_fade(&state, false, seconds);
                }
                true
            }
            Ok(DropPlan::Transition(row, t)) => {
                let (clip_id, edge) = (t.clip_id, t.edge);
                let placed = state.execute(Box::new(SetTransitionCommand::set(row.track_id, t)));
                if placed {
                    state.select_edit(Some(EditSelection::Transition { track_id: row.track_id, clip_id, edge }));
                }
                placed
            }
        }
    }

    fn install_controllers(&self) {
        // Press, drag, release.
        let drag = gtk::GestureDrag::new();
        let c = self.downgrade();
        drag.connect_drag_begin(move |gesture, x, y| {
            if let Some(c) = c.upgrade() {
                c.grab_focus();
                c.imp().press_mods.set(gesture.current_event_state());
                c.on_press(x, y);
            }
        });
        let c = self.downgrade();
        drag.connect_drag_update(move |_, dx, dy| {
            if let Some(c) = c.upgrade() {
                let (px, py) = c.imp().press.get();
                c.on_drag(px + dx, py + dy);
            }
        });
        let c = self.downgrade();
        drag.connect_drag_end(move |gesture, _, _| {
            if let Some(c) = c.upgrade() {
                c.imp().release_mods.set(gesture.current_event_state());
                c.on_release();
            }
        });
        self.add_controller(drag);

        // Right-click: a menu for the clip, gap or empty space under the pointer.
        let click = gtk::GestureClick::builder().button(gdk::BUTTON_SECONDARY).build();
        let c = self.downgrade();
        click.connect_pressed(move |gesture, _, x, y| {
            if let Some(c) = c.upgrade() {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                c.grab_focus();
                c.show_menu(x, y);
            }
        });
        self.add_controller(click);

        // Double-click a marker in the ruler to edit it, as in Resolve.
        let double = gtk::GestureClick::new();
        let c = self.downgrade();
        double.connect_pressed(move |_, presses, x, y| {
            let Some(c) = c.upgrade() else { return };
            if presses != 2 || y >= RULER_H as f64 {
                return;
            }
            let state = c.state();
            let marker = state.with_timeline(|t| marker_near(&t.markers, x, MARKER_GRAB_PX, |us| c.x_of(us)).cloned()).flatten();
            if let Some(marker) = marker {
                crate::dialogs::edit_marker(&c, &state, marker);
            }
        });
        self.add_controller(double);

        // Pointer shape near clip edges.
        let motion = gtk::EventControllerMotion::new();
        let c = self.downgrade();
        motion.connect_motion(move |_, x, y| {
            if let Some(c) = c.upgrade() {
                c.imp().pointer_x.set(x);
                c.hover_tip(x, y);
                let tool = c.state().tool.get();
                let hit = c.hit(x, y);
                if c.imp().hover.replace(hit.as_ref().map(|(_, id, _)| *id)) != hit.as_ref().map(|(_, id, _)| *id) {
                    c.redraw();
                }
                let on_fade = c.fade_hit(x, y).is_some();
                let on_transition = if on_fade { None } else { c.transition_hit(x, y).map(|(_, _, edge)| edge.is_some()) };
                let zone = if y < RULER_H as f64 {
                    Some(HintZone::Ruler)
                } else if on_fade {
                    Some(HintZone::FadeHandle)
                } else if on_transition.is_some() {
                    Some(HintZone::TransitionEdge)
                } else {
                    hit.as_ref().map(|(_, _, edge)| if edge.is_some() { HintZone::ClipEdge } else { HintZone::ClipBody })
                };
                c.state().hint(zone);
                let cursor = match (tool, hit) {
                    (Tool::Blade, Some(_)) => Some("crosshair"),
                    (Tool::Select | Tool::Trim, _) if on_fade || on_transition == Some(true) => Some("ew-resize"),
                    (_, _) if on_transition.is_some() => None,
                    (Tool::Select | Tool::Trim, Some((_, _, Some(_)))) => Some("col-resize"),
                    _ => None,
                };
                c.set_cursor_from_name(cursor);
            }
        });
        let c = self.downgrade();
        motion.connect_leave(move |_| {
            if let Some(c) = c.upgrade() {
                c.state().hint(None);
                c.hover_tip(-1.0, -1.0);
                if c.imp().hover.take().is_some() {
                    c.redraw();
                }
            }
        });
        self.add_controller(motion);

        // Shift+scroll resizes track heights, Alt+scroll zooms, anything else scrolls sideways.
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
        let c = self.downgrade();
        scroll.connect_scroll(move |ctl, dx, dy| {
            let Some(c) = c.upgrade() else { return glib::Propagation::Proceed };
            let mods = ctl.current_event_state();
            if mods.contains(gdk::ModifierType::SHIFT_MASK) {
                let raw = if dy.abs() >= dx.abs() { dy } else { dx };
                if raw != 0.0 {
                    c.adjust_track_heights((-raw as f32) * 8.0);
                }
            } else if mods.contains(gdk::ModifierType::ALT_MASK) {
                let factor = if dy < 0.0 { 1.2 } else { 1.0 / 1.2 };
                c.set_pps(c.pps() * factor, Some(c.imp().pointer_x.get()));
            } else {
                c.set_scroll(c.imp().scroll.get() + (dx + dy) * 60.0);
            }
            glib::Propagation::Stop
        });
        self.add_controller(scroll);

        // Clips dragged in from the Media Pool carry the source id as text,
        // transitions from the Effects panel TRANSITION_PREFIX and their kind,
        // filters FILTER:<id>, and titles TITLE:<kind>.
        let drop = gtk::DropTarget::new(glib::Type::STRING, gdk::DragAction::COPY);
        drop.set_preload(true);
        let c = self.downgrade();
        drop.connect_motion(move |target, x, y| {
            let Some(c) = c.upgrade() else { return gdk::DragAction::COPY };
            let text = target.value().and_then(|v| v.get::<String>().ok());
            match text.as_deref() {
                Some(s) if s.starts_with("timeline:") => {
                    c.clear_drop_preview();
                    if c.imp().drop_at.take().is_some() {
                        c.redraw();
                    }
                    gdk::DragAction::empty()
                }
                Some(s) if s.starts_with(TRANSITION_PREFIX) => {
                    c.clear_drop_preview();
                    let kind = &s[TRANSITION_PREFIX.len()..];
                    let plan = c.plan_drop(kind, x, y);
                    let at = plan.as_ref().ok().and_then(|p| c.drop_span(p));
                    if c.imp().drop_at.replace(at) != at {
                        c.redraw();
                    }
                    if plan.is_err() { gdk::DragAction::empty() } else { gdk::DragAction::COPY }
                }
                Some(s) if s.starts_with("FILTER:") => {
                    c.clear_drop_preview();
                    let filter_id = &s["FILTER:".len()..];
                    let plan = c.plan_filter_drop(filter_id, x, y);
                    let at = plan.as_ref().ok().map(|&(track_id, _, tin, tout, _)| (track_id, tin, tout));
                    if c.imp().drop_at.replace(at) != at {
                        c.redraw();
                    }
                    if plan.is_err() { gdk::DragAction::empty() } else { gdk::DragAction::COPY }
                }
                Some(s) if s.starts_with("TITLE:") => {
                    c.clear_drop_preview();
                    let title_kind = &s["TITLE:".len()..];
                    let plan = c.plan_title_drop(title_kind, x, y);
                    let at = plan.as_ref().ok().map(|&(track_id, tin, tout, _)| (track_id, tin, tout));
                    if c.imp().drop_at.replace(at) != at {
                        c.redraw();
                    }
                    if plan.is_err() { gdk::DragAction::empty() } else { gdk::DragAction::COPY }
                }
                Some(s) => {
                    let id = Uuid::parse_str(s).ok();
                    let ghost = id.and_then(|id| c.ghost_for(id, y));
                    c.set_ghost(ghost);
                    if let Some(id) = id {
                        c.imp().drop_source.set(Some(id));
                    }
                    let preview = id.and_then(|id| c.preview_for(id, x, y));
                    if c.imp().preview.replace(preview) != preview {
                        c.redraw();
                    }
                    if c.imp().drop_at.take().is_some() {
                        c.redraw();
                    }
                    if id.is_some() { gdk::DragAction::COPY } else { gdk::DragAction::empty() }
                }
                None => {
                    // On Wayland the dragged value can arrive a motion or two late: keep the last source
                    // until it does, so the landing box never loses its thumbnail and name meanwhile.
                    if let Some(id) = c.imp().drop_source.get() {
                        let ghost = c.ghost_for(id, y);
                        c.set_ghost(ghost);
                        let preview = c.preview_for(id, x, y);
                        if c.imp().preview.replace(preview) != preview {
                            c.redraw();
                        }
                    }
                    gdk::DragAction::COPY
                }
            }
        });
        let c = self.downgrade();
        drop.connect_leave(move |_| {
            if let Some(c) = c.upgrade() {
                c.set_ghost(None);
                c.imp().drop_source.set(None);
                if c.imp().drop_at.take().is_some() | c.imp().preview.take().is_some() {
                    c.redraw();
                }
            }
        });
        let c = self.downgrade();
        drop.connect_drop(move |_, value, x, y| {
            let Some(c) = c.upgrade() else { return false };
            c.imp().drop_at.set(None);
            c.imp().drop_source.set(None);
            c.set_ghost(None);
            c.imp().preview.set(None);
            let Ok(text) = value.get::<String>() else { return false };
            if text.starts_with("timeline:") {
                return false;
            }
            if let Some(kind) = text.strip_prefix(TRANSITION_PREFIX) {
                return c.drop_transition(kind, x, y);
            }
            if let Some(filter_id) = text.strip_prefix("FILTER:") {
                return c.drop_filter(filter_id, x, y);
            }
            if let Some(title_kind) = text.strip_prefix("TITLE:") {
                return c.drop_title(title_kind, x, y);
            }
            let Ok(id) = Uuid::parse_str(&text) else { return false };
            let (at, _) = c.snap(c.us_at(x), &[]);
            let was_empty = c.state().with_timeline(|t| t.tracks.iter().all(|tr| tr.clips.is_empty())).unwrap_or(false);
            let done = if let Some(lanes) = c.ghost_for(id, y) {
                c.drop_on_new_tracks(id, at, lanes)
            } else {
                let track = c.row_at(y).map(|r| r.track_id);
                actions::place_source(&c.state(), id, Place::Overwrite, Some(at), track, true);
                true
            };
            // The first clip on an empty timeline zooms the view to fit it, as Resolve does.
            if done && was_empty {
                c.zoom_fit();
            }
            done
        });
        self.add_controller(drop);
    }

    fn set_ghost(&self, ghost: Option<(bool, bool)>) {
        if self.imp().ghost.replace(ghost) != ghost {
            if let Some(cb) = self.imp().ghost_cb.borrow().clone() {
                cb(ghost);
            }
            self.redraw();
        }
    }

    /// Tell `f` whenever ghost tracks appear or go away.
    pub fn connect_ghost(&self, f: impl Fn(Option<(bool, bool)>) + 'static) {
        *self.imp().ghost_cb.borrow_mut() = Some(Rc::new(f));
    }

    /// Where a Media Pool source dragged to (x, y) would land: start, end, sound, y.
    fn preview_for(&self, id: Uuid, x: f64, y: f64) -> Option<(i64, i64, bool, f64)> {
        let state = self.state();
        let at = state.frame_floor(self.snap(self.us_at(x), &[]).0);
        let nil = Some(Uuid::nil());
        let clips = actions::clips_for_source(&state, id, at, nil, nil);
        let (first, last) = (clips.first()?, clips.last()?);
        Some((first.timeline_in, last.timeline_out, clips.iter().any(|c| c.clip_type == ClipType::Audio), y))
    }

    /// The boxes showing where the dragged clip will land, as Resolve draws them: the
    /// video lane in the video clip colour with the clip's thumbnail and name, the audio lane green.
    fn draw_drop_preview(&self, s: &gtk::Snapshot, rows: &[Row], (start, end, sound, y): (i64, i64, bool, f64), w: f32) {
        let ghost = self.imp().ghost.get();
        let audio_y = rows.last().map_or(RULER_H, |r| r.y + r.h);
        let dest_audio = self.state().dest_audio.get();
        // (top, height, is a video lane)
        let mut lanes: Vec<(f32, f32, bool)> = Vec::new();
        if let Some((video, audio)) = ghost {
            if video {
                lanes.push((RULER_H, GHOST_BAND_H, true));
            }
            if audio {
                lanes.push((audio_y, self.imp().audio_h.get(), false));
            }
        } else if let Some(row) = rows.iter().find(|r| y >= r.y as f64 && y < (r.y + r.h) as f64) {
            lanes.push((row.y, row.h, row.kind == TrackKind::Video));
            if sound && row.kind == TrackKind::Video {
                lanes.extend(rows.iter().find(|r| r.kind == TrackKind::Audio && r.index == dest_audio).map(|r| (r.y, r.h, false)));
            }
        }
        let (x0, x1) = ((self.x_of(start) as f32).max(-8.0), (self.x_of(end) as f32).min(w + 8.0));
        for (ly, lh, video) in lanes {
            let area = graphene::Rect::new(x0, ly + 1.0, (x1 - x0).max(2.0), lh - 3.0);
            let rounded = gsk::RoundedRect::from_rect(area, 3.0);
            if !video {
                s.append_color(&gdk::RGBA::new(0.25, 0.75, 0.35, 0.35), &area);
                s.append_border(&rounded, &[2.0; 4], &[gdk::RGBA::new(0.3, 0.85, 0.4, 1.0); 4]);
                continue;
            }
            let body = css_color(self, "tempo_clip_video_top");
            let bar = css_color(self, "tempo_clip_video");
            s.push_rounded_clip(&rounded);
            s.append_color(&gdk::RGBA::new(body.red(), body.green(), body.blue(), 0.85), &area);
            let bar_y = area.y() + area.height() - NAME_BAR_H;
            s.append_color(&gdk::RGBA::new(bar.red(), bar.green(), bar.blue(), 0.9), &graphene::Rect::new(area.x(), bar_y, area.width(), NAME_BAR_H));
            if let Some((texture, name)) = self.drop_source_look() {
                let side = bar_y - area.y();
                if let Some(texture) = texture {
                    s.push_clip(&graphene::Rect::new(area.x(), area.y(), area.width(), side));
                    s.append_texture(&texture, &graphene::Rect::new(area.x(), area.y(), side * 16.0 / 9.0, side));
                    s.pop();
                }
                self.text(s, &name, area.x() + 6.0, bar_y + 3.0, &gdk::RGBA::WHITE, true, area.width() - 10.0);
            }
            s.pop();
            s.append_border(&rounded, &[2.0; 4], &[css_color(self, "tempo_accent"); 4]);
        }
    }

    /// The dragged Media Pool source's file name and, when it has one, its thumbnail (made once).
    fn drop_source_look(&self) -> Option<(Option<gdk::Texture>, String)> {
        let id = self.imp().drop_source.get()?;
        let state = self.state();
        let (name, data) = state.with_project(|p| p.sources.get(&id).map(|src| (src.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), src.thumbnail_data.is_some()))).flatten()?;
        let mut cache = self.imp().thumbs.borrow_mut();
        if !cache.contains_key(&id) && data {
            let texture = state
                .with_project(|p| p.sources.get(&id).and_then(|src| src.thumbnail_data.clone()))
                .flatten()
                .and_then(|data| {
                    let (w, h) = thumbnail_size(data.len())?;
                    Some(gdk::MemoryTexture::new(w as i32, h as i32, gdk::MemoryFormat::R8g8b8a8, &glib::Bytes::from_owned(data), w * 4).upcast::<gdk::Texture>())
                });
            // A blob is there: cache even an unreadable one, so a bad size isn't re-copied every draw.
            cache.insert(id, texture);
        }
        Some((cache.get(&id).cloned().flatten(), name))
    }

    /// The new tracks a source dropped at `y` would get, if any.
    fn ghost_for(&self, id: Uuid, y: f64) -> Option<(bool, bool)> {
        let (media, sound) = self.state().with_project(|p| p.sources.get(&id).map(|s| (s.media_type, s.audio_channels.is_some()))).flatten()?;
        let bottom = self.state().with_timeline(|t| rows(t).last().map_or(0.0, |r| (r.y + r.h) as f64))?;
        ghost_lanes(y, bottom, media, sound)
    }

    /// Drop a source onto new tracks made for it (Resolve's ghost tracks): one undo step.
    fn drop_on_new_tracks(&self, id: Uuid, at: i64, (video, sound): (bool, bool)) -> bool {
        let state = self.state();
        let next = |kind: TrackKind| state.with_timeline(|t| t.tracks.iter().filter(|tr| tr.kind == kind).map(|tr| tr.kind_index).max().unwrap_or(0) + 1).unwrap_or(1);
        let at = state.frame_floor(at);
        let mut commands: Vec<Box<dyn Command>> = Vec::new();
        // New audio tracks copy the existing ones' type (Stereo when there are none).
        let audio_type = state.with_timeline(|t| t.tracks.iter().find(|tr| tr.kind == TrackKind::Audio).map(|tr| tr.audio_type)).flatten();
        let mut new_track = |kind: TrackKind| {
            let n = next(kind);
            let mut track = Track::new(kind, n, format!("{} {n}", if kind == TrackKind::Video { "Video" } else { "Audio" }), 0);
            if let (TrackKind::Audio, Some(t)) = (kind, audio_type) {
                track.audio_type = t;
            }
            let add = AddTrackCommand::new(track);
            let id = add.track_id();
            commands.push(Box::new(add));
            id
        };
        let video = video.then(|| new_track(TrackKind::Video));
        let audio = sound.then(|| new_track(TrackKind::Audio));
        // Built as a normal drop is: In/Out marks, still length and links.
        let clips = actions::clips_for_source(&state, id, at, video, audio);
        if clips.is_empty() {
            return false;
        }
        let first = clips[0].id;
        commands.extend(clips.into_iter().map(|c| Box::new(InsertClipCommand::new(c)) as Box<dyn Command>));
        let done = state.execute(Box::new(CompositeCommand::new("Add tracks and place clip", commands)));
        if done {
            state.select(Some(first));
        }
        done
    }

    /// The temporary track lanes shown while a clip is dragged above the top video track.
    fn draw_ghost_lanes(&self, s: &gtk::Snapshot, rows: &[Row], (video, sound): (bool, bool), w: f32) {
        let next = |kind: TrackKind| rows.iter().filter(|r| r.kind == kind).map(|r| r.index).max().unwrap_or(0) + 1;
        let (v, a) = (next(TrackKind::Video), next(TrackKind::Audio));
        let audio_y = rows.last().map_or(RULER_H, |r| r.y + r.h);
        let lane = |y: f32, h: f32, text: String| {
            let area = graphene::Rect::new(0.0, y, w, h);
            s.append_color(&css_color(self, "tempo_bg_surface"), &area);
            s.append_color(&css_color(self, "tempo_line_soft"), &graphene::Rect::new(0.0, y + h - 1.0, w, 1.0));
            self.text(s, &text, 8.0, y + h / 2.0 - 8.0, &css_color(self, "tempo_text_dim"), false, 160.0);
        };
        if video {
            lane(RULER_H, GHOST_BAND_H, format!("V{v}  Video {v}"));
        }
        if sound {
            lane(audio_y, self.imp().audio_h.get(), format!("A{a}  Audio {a}"));
        }
    }

    fn on_press(&self, x: f64, y: f64) {
        let imp = self.imp();
        imp.press.set((x, y));
        imp.dragging.set(false);
        let state = self.state();
        // Working on the timeline ends looking at a Media Pool clip.
        if state.source_mode() {
            state.show_source(None);
        }

        if y < RULER_H as f64 {
            // The playhead goes to the click either way; on a marker the drag moves the marker.
            let marker = state.with_timeline(|t| marker_near(&t.markers, x, MARKER_GRAB_PX, |us| self.x_of(us)).map(|m| m.id)).flatten();
            *imp.drag.borrow_mut() = Some(match marker.and_then(|id| state.with_timeline(|t| t.markers.iter().find(|m| m.id == id).map(|m| m.position_us)).flatten().map(|at| (id, at))) {
                Some((id, at)) => Drag::Marker { id, from: at, to: at },
                None => Drag::Scrub,
            });
            self.scrub(x);
            return;
        }

        let plain = !imp.press_mods.get().contains(gdk::ModifierType::CONTROL_MASK);
        if let Some((id, edge, length)) = self.fade_hit(x, y).filter(|_| plain && state.tool.get() != Tool::Blade) {
            if !state.is_selected(id) {
                state.select(Some(id));
            }
            *imp.drag.borrow_mut() = Some(Drag::Fade { clip: id, edge, from: length, to: length });
            return;
        }
        // A transition box: a click selects it, a free edge drags its length.
        if let Some((row, t, edge)) = self.transition_hit(x, y).filter(|_| plain && state.tool.get() != Tool::Blade) {
            state.select_edit(Some(EditSelection::Transition { track_id: row.track_id, clip_id: t.clip_id, edge: t.edge }));
            if let Some((left, both)) = edge.filter(|_| !row.locked) {
                let max = state
                    .with_project(|p| p.timeline.find_track(row.track_id).map(|tr| tr.max_transition_us(t.clip_id, t.edge, t.alignment, &p.sources, p.timeline_fps())))
                    .flatten()
                    .unwrap_or(t.duration_us);
                let from = t.duration_us;
                *imp.drag.borrow_mut() = Some(Drag::Transition { track: row.track_id, clip: t.clip_id, edge: t.edge, left, both, from, to: from, max });
            }
            return;
        }

        match (state.tool.get(), self.hit(x, y)) {
            (Tool::Blade, Some((row, id, _))) => {
                if !row.locked {
                    let (at, hit) = self.snap(self.us_at(x), &[id]);
                    actions::split_at(&state, id, if hit { at } else { state.frame_floor(at) });
                }
            }
            (tool, Some((row, id, edge))) => {
                let mods = imp.press_mods.get();
                if mods.contains(gdk::ModifierType::CONTROL_MASK) {
                    // Ctrl+click adds a clip to the selection or takes it out.
                    state.toggle_select(id);
                    return;
                }
                if let Some(edge) = edge {
                    // Resolve: a click on a clip's edge selects the edit point there, a drag trims.
                    let at = state.with_timeline(|t| t.find_clip(id).map(|(_, c)| if edge == TrimEdge::In { c.timeline_in } else { c.timeline_out })).flatten();
                    if let Some(at) = at {
                        state.select_edit(Some(EditSelection::Edit { track_id: row.track_id, position_us: at }));
                    }
                    if !row.locked {
                        let roll = tool == Tool::Trim && mods.contains(gdk::ModifierType::SHIFT_MASK);
                        *imp.drag.borrow_mut() = Some(Drag::Trim { clip: id, edge, delta: 0, roll });
                    }
                    return;
                }
                if !state.is_selected(id) {
                    state.select(Some(id));
                } else if state.selection.get() != Some(id) {
                    // Clicking another clip of a multi-selection makes it the primary one.
                    let rest: Vec<Uuid> = state.selected_ids().into_iter().filter(|s| *s != id).collect();
                    state.selection.set(Some(id));
                    *state.extra_selection.borrow_mut() = rest;
                    state.emit(Change::Selection);
                }
                if row.locked {
                    return;
                }
                let Some(clip) = state.selected_clip() else { return };
                *imp.drag.borrow_mut() = Some(Drag::Move {
                    clip: id,
                    from_track: row.track_id,
                    kind: row.kind,
                    orig_in: clip.timeline_in,
                    len: clip.duration_us(),
                    new_in: clip.timeline_in,
                    to_track: row.track_id,
                    duplicate: mods.contains(gdk::ModifierType::ALT_MASK),
                });
            }
            // Empty space: a gap that could be closed is selected, anything else clears the selection.
            (_, None) => match self.row_at(y).and_then(|row| actions::gap_at(&state, row.track_id, self.us_at(x))) {
                Some(gap) => state.select_gap(gap),
                None => state.select(None),
            },
        }
    }

    /// The right-click menu. Items run the same actions as the keys.
    pub fn show_menu(&self, x: f64, y: f64) {
        if y < RULER_H as f64 {
            return;
        }
        let state = self.state();
        let section = |items: &[(&str, &str)]| {
            let menu = gio::Menu::new();
            for (label, action) in items {
                let item = gio::MenuItem::new(Some(label), None);
                item.set_action_and_target_value(Some("win.do"), Some(&action.to_variant()));
                menu.append_item(&item);
            }
            menu
        };
        let menu = gio::Menu::new();
        if let Some((row, t, _)) = self.transition_hit(x, y) {
            state.select_edit(Some(EditSelection::Transition { track_id: row.track_id, clip_id: t.clip_id, edge: t.edge }));
            menu.append_section(None, &section(&[("Copy", "win.copy"), ("Delete", "win.delete")]));
            menu.append_section(None, &section(&[("Change Transition Duration…", "win.transition-duration")]));
            self.popup_menu(&menu, x, y);
            return;
        }
        match self.hit(x, y) {
            Some((_, id, _)) => {
                if !state.is_selected(id) {
                    state.select(Some(id));
                }
                let clips: Vec<tempo_timeline::Clip> = state
                    .with_timeline(|t| state.selected_ids().iter().filter_map(|id| t.find_clip(*id).map(|(_, c)| c.clone())).collect())
                    .unwrap_or_default();
                let first_link = clips.first().and_then(|c| c.properties.link);
                let linked = first_link.is_some() && clips.iter().all(|c| c.properties.link == first_link);
                let enabled = clips.first().is_some_and(|c| c.properties.enabled);

                menu.append_section(None, &section(&[("Cut", "win.cut"), ("Copy", "win.copy"), ("Paste at Playhead", "win.paste")]));
                menu.append_section(
                    None,
                    &section(&[("Split at Playhead", "win.split-clip"), ("Delete", "win.delete"), ("Delete and Close Gap", "win.ripple-delete")]),
                );
                let mut clip_items = Vec::new();
                if linked {
                    clip_items.push(("Unlink Picture and Sound", "win.link-toggle"));
                } else if clips.len() > 1 {
                    clip_items.push(("Link Selected Clips", "win.link-toggle"));
                }
                clip_items.push((if enabled { "Turn Clip Off" } else { "Turn Clip On" }, "win.clip-enable-toggle"));
                menu.append_section(None, &section(&clip_items));
                menu.append_section(None, &section(&[("Fade In", "win.fade-in"), ("Fade Out", "win.fade-out"), ("Add Transition", "win.transition-add")]));
                menu.append_section(None, &section(&[("Change Clip Speed…", "win.clip-speed")]));
            }
            None => {
                let gap = self.row_at(y).and_then(|row| actions::gap_at(&state, row.track_id, self.us_at(x)));
                match gap {
                    Some(gap) => {
                        state.select_gap(gap);
                        menu.append_section(None, &section(&[("Close Gap", "win.delete")]));
                    }
                    None => state.select(None),
                }
                menu.append_section(None, &section(&[("Paste at Playhead", "win.paste"), ("Add Marker at Playhead", "win.marker-add")]));
                menu.append_section(None, &section(&[("Select All", "win.select-all"), ("Import Media…", "win.import-media")]));
            }
        }

        self.popup_menu(&menu, x, y);
    }

    fn popup_menu(&self, menu: &gio::Menu, x: f64, y: f64) {
        if let Some(old) = self.imp().menu.take() {
            old.unparent();
        }
        let popover = gtk::PopoverMenu::from_model(Some(menu));
        popover.set_parent(self);
        popover.set_has_arrow(false);
        popover.set_halign(gtk::Align::Start);
        popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        *self.imp().menu.borrow_mut() = Some(popover.clone());
        popover.popup();
    }

    /// The two-line tooltip of the marker under the pointer in the ruler.
    fn hover_tip(&self, x: f64, y: f64) {
        let imp = self.imp();
        if imp.drag.borrow().is_some() {
            return;
        }
        let state = self.state();
        let tip = (y >= 0.0 && y < RULER_H as f64)
            .then(|| state.with_timeline(|t| marker_near(&t.markers, x, MARKER_GRAB_PX, |us| self.x_of(us)).map(|m| marker_tip(&state.position_tc(m.position_us), &m.name))).flatten())
            .flatten()
            .map(|text| (x, y, text));
        if *imp.tip.borrow() != tip {
            *imp.tip.borrow_mut() = tip;
            self.queue_draw();
        }
    }

    fn scrub(&self, x: f64) {
        let us = self.us_at(x).min(self.state().player.duration_us());
        if let Some(v) = self.imp().viewer.borrow().as_ref() {
            v.scrub_to(us);
        }
    }

    fn on_drag(&self, x: f64, y: f64) {
        let imp = self.imp();
        let (px, py) = imp.press.get();
        let state = self.state();
        if matches!(*imp.drag.borrow(), Some(Drag::Scrub)) {
            self.scrub(x);
            return;
        }
        // A click with a slightly unsteady hand is not a drag.
        if !imp.dragging.get() {
            if (x - px).abs() < DRAG_START_PX && (y - py).abs() < DRAG_START_PX {
                return;
            }
            imp.dragging.set(true);
        }
        let delta_us = ((x - px) / self.pps() * 1_000_000.0) as i64;
        let mut drag = imp.drag.borrow_mut();
        match drag.as_mut() {
            Some(Drag::Marker { id, from, to }) => {
                let (at, hit) = self.snap_skipping((*from + delta_us).max(0), &[], Some(*id));
                *to = if hit { at } else { state.frame_floor(at) };
                let name = state.with_timeline(|t| t.markers.iter().find(|m| m.id == *id).map(|m| m.name.clone())).flatten().unwrap_or_default();
                *imp.tip.borrow_mut() = Some((x, y, marker_tip(&state.position_tc(*to), &name)));
            }
            Some(Drag::Fade { edge, from, to, .. }) => {
                // Toward the middle of the clip lengthens the fade.
                *to = (*from + if *edge == FadeEdge::In { delta_us } else { -delta_us }).max(0);
            }
            Some(Drag::Transition { left, both, from, to, max, .. }) => {
                let fps = state.with_project(|p| p.timeline_fps()).unwrap_or_default();
                *to = even_steps(*from, actions::dragged_length(*from, delta_us, *left, *both, *max, fps), *both, state.frame_us()).min(*max);
                *imp.tip.borrow_mut() = Some((x, y, length_tip(*from, *to, *left, *both, state.fps())));
            }
            Some(Drag::Move { clip, kind, orig_in, len, new_in, to_track, duplicate, .. }) => {
                let raw = (*orig_in + delta_us).max(0);
                let moving = if *duplicate { Vec::new() } else { self.moving_with(*clip) };
                // Snap whichever end is closer to something.
                let (s_in, hit_in) = self.snap(raw, &moving);
                let (s_out, hit_out) = self.snap(raw + *len, &moving);
                let snapped = if hit_in {
                    imp.snap_line.set(Some(s_in));
                    s_in
                } else if hit_out {
                    imp.snap_line.set(Some(s_out));
                    s_out - *len
                } else {
                    imp.snap_line.set(None);
                    state.frame_floor(raw)
                };
                *new_in = snapped.max(0);
                *imp.tip.borrow_mut() = Some((x, y, offset_label(*new_in - *orig_in, state.fps())));
                if let Some(row) = self.row_at(y) {
                    if row.kind == *kind && !row.locked {
                        *to_track = row.track_id;
                    }
                }
            }
            Some(Drag::Trim { clip, edge, delta, roll }) => {
                // The clip's place, its footage, and the clips either side of it.
                let facts = state
                    .with_project(|p| {
                        let (track, c) = p.timeline.find_clip(*clip)?;
                        let still = matches!(c.clip_type, ClipType::Title | ClipType::Image);
                        let footage = p.sources.get(&c.source_id).map(|s| s.duration_us).filter(|d| !still && *d > 0);
                        let before = track.clips.iter().map(|o| o.timeline_out).filter(|o| *o <= c.timeline_in).max().unwrap_or(0);
                        let after = track.clips.iter().map(|o| o.timeline_in).filter(|o| *o >= c.timeline_out).min();
                        // Unused footage before and after the clip, in timeline time at its speed.
                        let head = c.to_timeline_floor_us(c.source_in);
                        let tail = footage.map(|d| c.to_timeline_floor_us((d - c.source_out).max(0)));
                        Some((c.timeline_in, c.timeline_out, head, tail, still, before, after))
                    })
                    .flatten();
                let Some((tin, tout, head, tail, still, before, after)) = facts else { return };
                let base = if *edge == TrimEdge::In { tin } else { tout };
                let moving = self.moving_with(*clip);
                let (target, hit) = self.snap(base + delta_us, &moving);
                imp.snap_line.set(hit.then_some(target));
                let target = if hit { target } else { state.frame_floor(target) };
                // Stop at what the clip can do, instead of refusing the whole trim on release:
                // one frame at least, no further than its footage, and (outside Trim
                // mode, where later clips make room) not into the clip next to it.
                let frame = state.frame_us();
                let makes_room = state.tool.get() == Tool::Trim && !*roll;
                let (low, high) = match edge {
                    TrimEdge::In => {
                        let footage_low = if still { 0 } else { tin - head };
                        (footage_low.max(if makes_room { 0 } else { before }), tout - frame)
                    }
                    TrimEdge::Out => {
                        let footage_high = tail.map_or(i64::MAX, |room| tout + room);
                        (tin + frame, footage_high.min(if makes_room { i64::MAX } else { after.unwrap_or(i64::MAX) }))
                    }
                };
                *delta = target.clamp(low, high.max(low)) - base;
                *imp.tip.borrow_mut() = Some((x, y, offset_label(*delta, state.fps())));
            }
            _ => return,
        }
        drop(drag);
        self.redraw();
    }

    fn on_release(&self) {
        let imp = self.imp();
        imp.snap_line.set(None);
        imp.tip.take();
        let release_mods = imp.release_mods.replace(gdk::ModifierType::empty());
        let drag = imp.drag.borrow_mut().take();
        imp.no_tween_until.set(glib::monotonic_time() + 100_000);
        let state = self.state();
        match drag {
            Some(Drag::Marker { id, from, to }) if to != from => {
                let moved = state.with_timeline(|t| t.markers.iter().find(|m| m.id == id).cloned()).flatten();
                if let Some(mut marker) = moved {
                    marker.position_us = to;
                    actions::update_marker(&state, marker);
                }
            }
            Some(Drag::Move { clip, from_track, orig_in, new_in, to_track, duplicate, .. }) => {
                let dup = duplicate || release_mods.contains(gdk::ModifierType::ALT_MASK);
                if dup {
                    if new_in != orig_in || to_track != from_track {
                        self.duplicate_dragged(&state, clip, from_track, to_track, new_in - orig_in);
                    }
                } else if state.selected_ids().len() > 1 {
                    // Several clips (or a clip and its sound) move together in time.
                    // Dragging to another row takes the clips of that kind with it.
                    let rows = state.with_timeline(rows).unwrap_or_default();
                    let index = |id: Uuid| rows.iter().find(|r| r.track_id == id).map(|r| (r.kind, r.index as i32));
                    let shift = match (index(from_track), index(to_track)) {
                        (Some((kind, from)), Some((_, to))) if from != to => Some((kind, to - from)),
                        _ => None,
                    };
                    actions::move_selected(&state, new_in - orig_in, shift);
                } else if new_in != orig_in || to_track != from_track {
                    state.execute(Box::new(MoveClipCommand::new(clip, from_track, to_track, orig_in, new_in)));
                }
            }
            Some(Drag::Fade { clip, edge, from, to }) if to != from => {
                let edited = state.with_project(|p| p.timeline.find_clip(clip).map(|(_, c)| c.with_fade(edge, to, p.timeline_fps()))).flatten();
                if let Some(edited) = edited {
                    state.execute(Box::new(EditClipCommand::new(if edge == FadeEdge::In { "Fade In" } else { "Fade Out" }, edited)));
                }
            }
            Some(Drag::Transition { track, clip, edge, from, to, .. }) if to != from => actions::set_transition_length(&state, track, clip, edge, to),
            // Trim mode ripples (later clips follow), or rolls the cut with Shift.
            // Selection mode leaves a gap.
            Some(Drag::Trim { clip, edge, delta, roll }) if delta != 0 => {
                let was = state.edit_selection.get();
                if roll {
                    if !actions::roll_cut(&state, clip, edge, delta) {
                        state.message("There is no clip touching that edge to roll against.");
                    }
                } else {
                    actions::trim_clip(&state, clip, edge, delta, state.tool.get() == Tool::Trim);
                }
                // The selected edit point moves with the edge it is on.
                if let Some(EditSelection::Edit { track_id, .. }) = was {
                    let now = state.with_timeline(|t| t.find_clip(clip).map(|(_, c)| if edge == TrimEdge::In { c.timeline_in } else { c.timeline_out })).flatten();
                    if let Some(position_us) = now {
                        state.select_edit(Some(EditSelection::Edit { track_id, position_us }));
                    }
                }
            }
            _ => {}
        }
        self.redraw();
    }

    fn duplicate_dragged(&self, state: &Rc<AppState>, primary: Uuid, from_track: Uuid, to_track: Uuid, delta_us: i64) {
        let ids = if state.is_selected(primary) {
            state.selected_ids()
        } else {
            std::iter::once(primary).chain(state.linked_to(primary)).collect()
        };
        let clips: Vec<(Uuid, Clip)> = state
            .with_timeline(|t| {
                ids.iter()
                    .filter_map(|id| t.find_clip(*id))
                    .filter(|(tr, _)| !tr.locked)
                    .map(|(tr, c)| (tr.id, c.clone()))
                    .collect()
            })
            .unwrap_or_default();
        if clips.is_empty() {
            return;
        }
        let earliest = clips.iter().map(|(_, c)| c.timeline_in).min().unwrap_or(0);
        let delta = delta_us.max(-earliest);
        let rows = state.with_timeline(rows).unwrap_or_default();
        let index = |id: Uuid| rows.iter().find(|r| r.track_id == id).map(|r| (r.kind, r.index as i32));
        let shift = match (index(from_track), index(to_track)) {
            (Some((kind, from)), Some((_, to))) if from != to => Some((kind, to - from)),
            _ => None,
        };
        if delta == 0 && shift.is_none() {
            return;
        }
        let targets: Option<Vec<Uuid>> = state
            .with_timeline(|t| {
                clips
                    .iter()
                    .map(|(track, _)| {
                        let from = t.find_track(*track)?;
                        match shift.filter(|(kind, _)| *kind == from.kind) {
                            Some((kind, row_delta)) => {
                                let want = from.kind_index as i32 + row_delta;
                                t.tracks.iter().find(|tr| tr.kind == kind && tr.kind_index as i32 == want && !tr.locked).map(|tr| tr.id)
                            }
                            None => Some(*track),
                        }
                    })
                    .collect()
            })
            .flatten();
        let Some(targets) = targets else {
            state.message("There is no free track there for every selected clip.");
            return;
        };
        let mut link_counts: HashMap<Uuid, usize> = HashMap::new();
        for (_, c) in &clips {
            if let Some(l) = c.properties.link {
                *link_counts.entry(l).or_insert(0) += 1;
            }
        }
        let mut new_links: HashMap<Uuid, Uuid> = HashMap::new();
        let mut new_ids = Vec::with_capacity(clips.len());
        let mut commands: Vec<Box<dyn Command>> = Vec::with_capacity(clips.len());
        for ((_, orig), target_track) in clips.iter().zip(targets) {
            let mut copy = orig.clone();
            copy.id = Uuid::new_v4();
            copy.track_id = target_track;
            copy.timeline_in = orig.timeline_in + delta;
            copy.timeline_out = copy.timeline_in + orig.duration_us();
            copy.properties.link = orig.properties.link.and_then(|old| {
                if link_counts.get(&old).copied().unwrap_or(0) > 1 {
                    Some(*new_links.entry(old).or_insert_with(Uuid::new_v4))
                } else {
                    None
                }
            });
            new_ids.push(copy.id);
            commands.push(Box::new(InsertClipCommand::new(copy)) as Box<dyn Command>);
        }
        if !commands.is_empty() && state.execute(Box::new(CompositeCommand::new("Duplicate clip", commands))) {
            if let Some(first) = new_ids.first().copied() {
                state.selection.set(Some(first));
                *state.extra_selection.borrow_mut() = new_ids.into_iter().skip(1).collect();
                state.emit(Change::Selection);
            }
        }
    }

    // ---- Drawing -------------------------------------------------------------

    fn text(&self, snapshot: &gtk::Snapshot, text: &str, x: f32, y: f32, color: &gdk::RGBA, bold: bool, max_w: f32) {
        if max_w < 12.0 {
            return;
        }
        let layout = self.create_pango_layout(Some(text));
        let mut font = layout.font_description().unwrap_or_else(|| self.pango_context().font_description().unwrap_or_default());
        font.set_size((font.size() as f32 * 0.88) as i32);
        if bold {
            font.set_weight(pango::Weight::Semibold);
        }
        layout.set_font_description(Some(&font));
        layout.set_width((max_w * pango::SCALE as f32) as i32);
        layout.set_ellipsize(pango::EllipsizeMode::End);
        snapshot.save();
        snapshot.translate(&graphene::Point::new(x, y));
        snapshot.append_layout(&layout, color);
        snapshot.restore();
    }

    /// Draw the loudness of a clip as vertical bars, one every two pixels, only
    /// for the part of the clip that is on screen.
    /// `speed` is the clip's: each pixel of timeline covers that much more (or less) sound.
    #[allow(clippy::too_many_arguments)]
    fn draw_waveform(&self, s: &gtk::Snapshot, peaks: &[u8], source_start_us: i64, speed: f64, x: f32, y: f32, width: f32, height: f32, view_w: f32) {
        const PEAKS_PER_SECOND: f64 = 50.0;
        let color = css_color(self, "tempo_clip_text");
        let color = gdk::RGBA::new(color.red(), color.green(), color.blue(), 0.85);
        let us_per_px = 1_000_000.0 / self.pps() * speed;
        let mid = y + height / 2.0;
        let mut px = (-x).max(0.0);
        let end = width.min(view_w - x);
        while px < end {
            let t0 = source_start_us as f64 + px as f64 * us_per_px;
            let a = (t0 / 1_000_000.0 * PEAKS_PER_SECOND) as usize;
            let b = (((t0 + 2.0 * us_per_px) / 1_000_000.0 * PEAKS_PER_SECOND) as usize).max(a + 1);
            let peak = peaks.get(a..b.min(peaks.len())).and_then(|p| p.iter().max()).copied().unwrap_or(0);
            let half = (peak as f32 / 255.0 * (height / 2.0 - 2.0)).max(0.5);
            s.append_color(&color, &graphene::Rect::new(x + px, mid - half, 1.0, half * 2.0));
            px += 2.0;
        }
    }

    fn draw(&self, s: &gtk::Snapshot) {
        DRAWS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let (w, h) = (self.width() as f32, self.height() as f32);
        let state = self.state();
        let imp = self.imp();
        let color = |name: &str| css_color(self, name);
        let rect = |x: f32, y: f32, w: f32, h: f32| graphene::Rect::new(x, y, w, h);

        s.append_color(&color("tempo_bg_surface"), &rect(0.0, 0.0, w, h));

        let project = state.project.borrow();
        let Some(project) = project.as_ref() else { return };
        let timeline = &project.timeline;
        let line = color("tempo_line");
        let soft = color("tempo_line_soft");
        let dim = color("tempo_text_dim");
        let accent = color("tempo_accent");
        let white = color("tempo_clip_text");

        // Ruler ticks and labels.
        let pps = self.pps();
        let step = [1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0, 1800.0, 3600.0]
            .into_iter()
            .find(|s| s * pps >= 120.0)
            .unwrap_or(3600.0);
        let scroll = self.shown_scroll();
        let first = ((scroll / pps) / step).floor() * step;
        let mut t = first;
        while (t * pps - scroll) < w as f64 {
            let x = (t * pps - scroll) as f32;
            s.append_color(&soft, &rect(x, RULER_H - 20.0, 1.0, 20.0));
            for i in 1..5 {
                let mx = x + (step * pps) as f32 * i as f32 / 5.0;
                s.append_color(&soft, &rect(mx, RULER_H - 7.0, 1.0, 7.0));
            }
            self.text(s, &state.position_tc((t * 1_000_000.0) as i64), x + 6.0, 24.0, &dim, false, 110.0);
            t += step;
        }
        s.append_color(&line, &rect(0.0, RULER_H - 1.0, w, 1.0));

        // In–Out band, and a flag in the ruler for each mark so that one alone shows too.
        if let (Some(a), Some(b)) = (state.mark_in.get(), state.mark_out.get()) {
            let (xa, xb) = (self.x_of(a.min(b)) as f32, self.x_of(a.max(b)) as f32);
            s.append_color(&gdk::RGBA::new(1.0, 1.0, 1.0, 0.08), &rect(xa, 0.0, xb - xa, h));
        }
        for (mark, is_in) in [(state.mark_in.get(), true), (state.mark_out.get(), false)] {
            let Some(x) = mark.map(|us| self.x_of(us) as f32).filter(|x| *x > -10.0 && *x < w + 10.0) else { continue };
            let white = color("tempo_clip_text");
            s.append_color(&white, &rect(x, RULER_H - 22.0, 1.0, 22.0));
            // The foot points into the marked stretch: right for In, left for Out.
            s.append_color(&white, &rect(if is_in { x } else { x - 6.0 }, RULER_H - 22.0, 7.0, 2.0));
            self.text(s, if is_in { "In" } else { "Out" }, x + 4.0, RULER_H - 20.0, &white, false, 30.0);
        }

        // Tracks and clips.
        let dragging = imp.drag.borrow().clone();
        // While several clips are dragged, how far they have moved.
        let multi_move = match &dragging {
            Some(Drag::Move { orig_in, new_in, duplicate: false, .. }) if state.selected_ids().len() > 1 => Some(*new_in - *orig_in),
            _ => None,
        };
        let rows = rows(timeline);
        // While several clips are dragged to another row: which kind, and how many rows.
        let row_shift = match &dragging {
            Some(Drag::Move { from_track, to_track, kind, duplicate: false, .. }) if multi_move.is_some() => {
                let index = |id: &Uuid| rows.iter().find(|r| r.track_id == *id).map(|r| r.index as i32);
                index(from_track).zip(index(to_track)).map(|(from, to)| (*kind, to - from)).filter(|(_, by)| *by != 0)
            }
            _ => None,
        };
        let shifted_y = |row: &Row| {
            row_shift
                .filter(|(kind, _)| *kind == row.kind)
                .and_then(|(kind, by)| rows.iter().find(|r| r.kind == kind && r.index as i32 == row.index as i32 + by))
                .map_or(row.y, |r| r.y)
        };
        // The selected edit point is bracketed on its track and the tracks linked to it there.
        let (edit_group, edit_at) = match state.edit_selection.get() {
            Some(EditSelection::Edit { track_id, position_us }) => (actions::edit_group(timeline, track_id, position_us, state.linked_selection.get()), position_us),
            _ => (Vec::new(), 0),
        };
        for row in &rows {
            s.append_color(&line, &rect(0.0, row.y + row.h - 1.0, w, 1.0));
            let Some(track) = timeline.find_track(row.track_id) else { continue };
            for clip in &track.clips {
                // A clip being moved is drawn at its new place below.
                let (mut tin, mut tout, mut y) = (clip.timeline_in, clip.timeline_out, row.y);
                (tin, tout) = self.tweened(clip.id, tin, tout);
                match &dragging {
                    Some(Drag::Move { clip: id, new_in, len, to_track, duplicate: false, .. }) if *id == clip.id => {
                        tin = *new_in;
                        tout = *new_in + *len;
                        if let Some(r) = rows.iter().find(|r| r.track_id == *to_track) {
                            y = r.y;
                        }
                    }
                    // The other selected clips follow the dragged one.
                    Some(Drag::Move { duplicate: false, .. }) if multi_move.is_some() && state.is_selected(clip.id) => {
                        let delta = multi_move.unwrap_or(0);
                        tin += delta;
                        tout += delta;
                        y = shifted_y(row);
                    }
                    Some(Drag::Trim { clip: id, edge, delta, .. }) if *id == clip.id => match edge {
                        TrimEdge::In => tin = (tin + delta).min(tout - 1),
                        TrimEdge::Out => tout = (tout + delta).max(tin + 1),
                    },
                    _ => {}
                }
                let (x0, x1) = (self.x_of(tin) as f32, self.x_of(tout) as f32);
                if x1 < 0.0 || x0 > w {
                    continue;
                }
                // Only the part on screen is drawn, however long the clip is.
                let (v0, v1) = (x0.max(-8.0), x1.min(w + 8.0));
                let body = rect(v0, y + 1.0, (v1 - v0).max(2.0), row.h - 3.0);
                let (top, bar) = match clip.clip_type {
                    ClipType::Audio => ("tempo_clip_audio_wave", "tempo_clip_audio"),
                    ClipType::Title => ("tempo_clip_title", "tempo_clip_title"),
                    _ => ("tempo_clip_video_top", "tempo_clip_video"),
                };
                // Like Resolve, a clip with adjustments or filters is a darker shade.
                let shaded = |name: &str| {
                    let c = color(name);
                    let k = if clip.properties.is_modified() { 0.82 } else { 1.0 };
                    gdk::RGBA::new(c.red() * k, c.green() * k, c.blue() * k, c.alpha())
                };
                let rounded = gsk::RoundedRect::from_rect(body, 3.0);
                s.push_rounded_clip(&rounded);
                s.append_color(&shaded(top), &body);
                let bar_y = body.y() + body.height() - NAME_BAR_H;
                if clip.clip_type == ClipType::Audio {
                    if let Some(peaks) = state.waveforms.borrow().get(&clip.source_id) {
                        self.draw_waveform(s, peaks, clip.source_offset_at(tin), clip.properties.speed, x0, body.y(), x1 - x0, bar_y - body.y(), w);
                    }
                }
                s.append_color(&shaded(bar), &rect(body.x(), bar_y, body.width(), NAME_BAR_H));
                // The name stays readable while the start of the clip is off screen.
                let mut name_x = x0.max(0.0) + 6.0;
                if clip.properties.link.is_some() && x1 - name_x > 40.0 {
                    // Two joined rings: this clip moves and is cut with its partner.
                    for dx in [0.0, 5.0] {
                        let ring = gsk::RoundedRect::from_rect(rect(name_x + dx, bar_y + 8.0, 8.0, 6.0), 3.0);
                        s.append_border(&ring, &[1.2; 4], &[white; 4]);
                    }
                    name_x += 18.0;
                }
                // A retimed clip says its speed, as Resolve marks it.
                let name = match clip.properties.speed {
                    s if s != 1.0 => format!("{}  {}%", clip.name, (s * 100.0).round()),
                    _ => clip.name.clone(),
                };
                self.text(s, &name, name_x, bar_y + 3.0, &white, true, x1 - name_x - 4.0);
                if !row.enabled || !clip.properties.enabled {
                    s.append_color(&gdk::RGBA::new(0.16, 0.16, 0.18, 0.6), &body);
                }
                // A clip whose file cannot be found plays black and silent: say so.
                if project.sources.get(&clip.source_id).is_some_and(|source| source.is_missing) {
                    let red = color("tempo_error");
                    s.append_color(&gdk::RGBA::new(red.red(), red.green(), red.blue(), 0.55), &body);
                    self.text(s, "Media offline", x0.max(0.0) + 6.0, body.y() + 4.0, &white, true, x1 - x0.max(0.0) - 10.0);
                }
                s.pop();
                // Fades: a ramp from the corner down to where the fade ends, and handles
                // on the clip under the pointer.
                let (mut fade_in, mut fade_out) = (clip.properties.fade_in_us, clip.properties.fade_out_us);
                if let Some(Drag::Fade { clip: id, edge, to, .. }) = &dragging {
                    if *id == clip.id {
                        let preview = clip.with_fade(*edge, *to, state.with_project(|p| p.timeline_fps()).unwrap_or_default());
                        (fade_in, fade_out) = (preview.properties.fade_in_us, preview.properties.fade_out_us);
                    }
                }
                let (top, bottom) = (body.y(), bar_y);
                for (corner, end, length) in [(x0, self.x_of(tin + fade_in) as f32, fade_in), (x1, self.x_of(tout - fade_out) as f32, fade_out)] {
                    if length <= 0 {
                        continue;
                    }
                    let shade = gsk::PathBuilder::new();
                    shade.move_to(corner, top);
                    shade.line_to(end, top);
                    shade.line_to(corner, bottom);
                    shade.close();
                    s.append_fill(&shade.to_path(), gsk::FillRule::Winding, &gdk::RGBA::new(0.0, 0.0, 0.0, 0.35));
                    let ramp = gsk::PathBuilder::new();
                    ramp.move_to(corner, bottom);
                    ramp.line_to(end, top);
                    s.append_stroke(&ramp.to_path(), &gsk::Stroke::new(1.5), &white);
                }
                let hovered = imp.hover.get() == Some(clip.id) || matches!(&dragging, Some(Drag::Fade { clip: id, .. }) if *id == clip.id);
                if hovered && !row.locked {
                    for at in [self.x_of(tin + fade_in) as f32, self.x_of(tout - fade_out) as f32] {
                        let x = (at - FADE_HANDLE_PX / 2.0).clamp(x0, (x1 - FADE_HANDLE_PX).max(x0));
                        let handle = gsk::RoundedRect::from_rect(rect(x, top + 1.0, FADE_HANDLE_PX, FADE_HANDLE_PX), 2.0);
                        s.append_color(&white, handle.bounds());
                        s.append_border(&handle, &[1.0; 4], &[gdk::RGBA::new(0.0, 0.0, 0.0, 0.6); 4]);
                    }
                }
                if state.is_selected(clip.id) {
                    s.append_border(&rounded, &[2.0; 4], &[accent; 4]);
                } else {
                    s.append_border(&rounded, &[1.0; 4], &[gdk::RGBA::new(0.0, 0.0, 0.0, 0.3); 4]);
                }
            }
            self.draw_transitions(s, row, track, &dragging, w);
            if edit_group.iter().any(|(id, _)| *id == row.track_id) {
                self.draw_edit_bracket(s, row, track, edit_at);
            }
            // Where something dragged from the Effects panel would land: a dark box.
            if let Some((_, a, b)) = imp.drop_at.get().filter(|(track, ..)| *track == row.track_id) {
                let (top, height) = transition_band(row);
                let area = rect(self.x_of(a) as f32, top, (self.x_of(b) - self.x_of(a)) as f32, height);
                s.append_color(&gdk::RGBA::new(0.0, 0.0, 0.0, 0.6), &area);
                s.append_border(&gsk::RoundedRect::from_rect(area, 2.0), &[1.0; 4], &[white; 4]);
            }
            if let Some(gap) = state.gap.get().filter(|g| g.track == row.track_id) {
                let (g0, g1) = ((self.x_of(gap.start) as f32).max(-8.0), (self.x_of(gap.end) as f32).min(w + 8.0));
                if g1 > g0 {
                    let area = rect(g0, row.y + 1.0, g1 - g0, row.h - 3.0);
                    s.append_color(&gdk::RGBA::new(1.0, 1.0, 1.0, 0.10), &area);
                    s.append_border(&gsk::RoundedRect::from_rect(area, 3.0), &[2.0; 4], &[accent; 4]);
                }
            }
            if row.locked {
                s.append_color(&gdk::RGBA::new(1.0, 1.0, 1.0, 0.06), &rect(0.0, row.y, w, row.h));
            }
        }
        if let Some(lanes) = imp.ghost.get() {
            self.draw_ghost_lanes(s, &rows, lanes, w);
        }
        if let Some(Drag::Move { clip, from_track, orig_in, new_in, to_track, duplicate: true, .. }) = &dragging {
            if *new_in != *orig_in || *to_track != *from_track {
                let delta = *new_in - *orig_in;
                let row_delta = rows
                    .iter()
                    .find(|r| r.track_id == *from_track)
                    .zip(rows.iter().find(|r| r.track_id == *to_track))
                    .map_or(0, |(f, t)| t.index as i32 - f.index as i32);
                let moving = self.moving_with(*clip);
                for r in &rows {
                    let Some(tr) = timeline.find_track(r.track_id) else { continue };
                    for c in &tr.clips {
                        if !moving.contains(&c.id) {
                            continue;
                        }
                        let target_row = if r.track_id == *from_track {
                            rows.iter().find(|dst| dst.track_id == *to_track)
                        } else if r.kind == rows.iter().find(|f| f.track_id == *from_track).map(|f| f.kind).unwrap_or(r.kind) && row_delta != 0 {
                            rows.iter().find(|dst| dst.kind == r.kind && dst.index as i32 == r.index as i32 + row_delta)
                        } else {
                            Some(r)
                        };
                        let Some(dst) = target_row else { continue };
                        let (x0, x1) = (self.x_of((c.timeline_in + delta).max(0)) as f32, self.x_of((c.timeline_out + delta).max(0)) as f32);
                        let area = rect(x0.max(-8.0), dst.y + 1.0, (x1.min(w + 8.0) - x0.max(-8.0)).max(2.0), dst.h - 3.0);
                        let fill = css_color(self, match c.clip_type {
                            ClipType::Audio => "tempo_clip_audio",
                            ClipType::Title => "tempo_clip_title",
                            _ => "tempo_clip_video",
                        });
                        s.append_color(&gdk::RGBA::new(fill.red(), fill.green(), fill.blue(), 0.55), &area);
                        s.append_border(&gsk::RoundedRect::from_rect(area, 3.0), &[2.0; 4], &[accent; 4]);
                    }
                }
            }
        }
        if let Some(p) = imp.preview.get() {
            self.draw_drop_preview(s, &rows, p, w);
        }
        // Divider between video and audio tracks.
        if let Some(first_audio) = rows.iter().find(|r| r.kind == TrackKind::Audio) {
            s.append_color(&soft, &rect(0.0, first_audio.y - DIVIDER_H, w, DIVIDER_H));
        }

        // Markers.
        let moving_marker = match &*imp.drag.borrow() {
            Some(Drag::Marker { id, to, .. }) => Some((*id, *to)),
            _ => None,
        };
        for m in &timeline.markers {
            let at = moving_marker.filter(|(id, _)| *id == m.id).map_or(m.position_us, |(_, to)| to);
            let x = self.x_of(at) as f32;
            if x < -10.0 || x > w + 10.0 {
                continue;
            }
            let c = gdk::RGBA::parse(m.color.hex()).unwrap_or(gdk::RGBA::WHITE);
            // A marker with a length is a bar from its start to its end, as in Resolve.
            if m.duration_us > 0 {
                let end = self.x_of(at + m.duration_us) as f32;
                let bar = gdk::RGBA::new(c.red(), c.green(), c.blue(), 0.45);
                s.append_color(&bar, &rect(x, 0.0, (end - x).max(1.0), 12.0));
            }
            // Markers live in the ruler only, so they are not mistaken for the playhead.
            s.append_color(&c, &rect(x - 4.0, 0.0, 9.0, 12.0));
            s.append_color(&c, &rect(x, 12.0, 1.0, RULER_H - 12.0));
            if !m.name.is_empty() {
                self.text(s, &m.name, x + 8.0, -2.0, &c, false, 140.0);
            }
        }

        // Snap guide. The playhead is drawn by `PlayheadLayer`.
        if let Some(us) = imp.snap_line.get() {
            s.append_color(&white, &rect(self.x_of(us) as f32, RULER_H, 1.0, h - RULER_H));
        }

        if let Some((x, y, text)) = imp.tip.borrow().as_ref() {
            self.draw_tip(s, *x as f32, *y as f32, text);
        }
    }

    /// A track's transitions, as Resolve draws them: a box over the span with a gradient
    /// showing the way the picture fades, its name, and a red outline when selected.
    fn draw_transitions(&self, s: &gtk::Snapshot, row: &Row, track: &Track, dragging: &Option<Drag>, view_w: f32) {
        let state = self.state();
        let (top, height) = transition_band(row);
        let light = gdk::RGBA::new(0.62, 0.62, 0.64, 1.0);
        let dark = gdk::RGBA::new(0.06, 0.06, 0.07, 1.0);
        for stored in track.transitions.iter().filter(|t| t.kind != TransitionKind::Cut) {
            // While an edge is dragged, the new length.
            let mut t = std::borrow::Cow::Borrowed(stored);
            if let Some(Drag::Transition { track: tr, clip, edge, to, .. }) = dragging {
                if *tr == track.id && *clip == t.clip_id && *edge == t.edge {
                    t.to_mut().duration_us = *to;
                }
            }
            let Some((start, end)) = track.transition_span(&t) else { continue };
            // The transition rides on the clip edge it belongs to while that eases.
            let shift = track.find_clip(t.clip_id).map_or(0, |c| {
                let (a, b) = self.tweened(c.id, c.timeline_in, c.timeline_out);
                if t.edge == TransitionEdge::In { a - c.timeline_in } else { b - c.timeline_out }
            });
            let (x0, x1) = (self.x_of(start + shift) as f32, self.x_of(end + shift) as f32);
            if x1 < 0.0 || x0 > view_w {
                continue;
            }
            let area = graphene::Rect::new(x0, top, (x1 - x0).max(2.0), height);
            // Picture to black at a tail, black to picture at a head; a dip goes through its colour.
            let into = t.edge == TransitionEdge::In && track.find_clip(t.clip_id).is_some_and(|c| !track.has_clip_ending_at(c.timeline_in));
            let mid = t.dip_color().map(|[r, g, b]| gdk::RGBA::new(r, g, b, 1.0));
            let stops: Vec<gsk::ColorStop> = match (t.edge, into, mid) {
                (TransitionEdge::Out, _, _) => vec![gsk::ColorStop::new(0.0, light), gsk::ColorStop::new(1.0, mid.unwrap_or(dark))],
                (_, true, _) => vec![gsk::ColorStop::new(0.0, mid.unwrap_or(dark)), gsk::ColorStop::new(1.0, light)],
                (_, false, Some(mid)) => vec![gsk::ColorStop::new(0.0, light), gsk::ColorStop::new(0.5, mid), gsk::ColorStop::new(1.0, light)],
                (_, false, None) => vec![gsk::ColorStop::new(0.0, light), gsk::ColorStop::new(1.0, dark)],
            };
            let rounded = gsk::RoundedRect::from_rect(area, 2.0);
            s.push_rounded_clip(&rounded);
            s.append_linear_gradient(&area, &graphene::Point::new(x0, top), &graphene::Point::new(x1, top), &stops);
            self.text(s, &transition_name(&t.kind), x0.max(0.0) + 4.0, top + (height - 16.0).max(0.0) / 2.0, &gdk::RGBA::WHITE, true, x1 - x0.max(0.0) - 8.0);
            if !t.enabled {
                // Switched off, as Resolve greys it: still there and selectable, but dim.
                s.append_color(&gdk::RGBA::new(0.16, 0.16, 0.18, 0.7), &area);
            }
            s.pop();
            let selected = state.edit_selection.get() == Some(EditSelection::Transition { track_id: track.id, clip_id: t.clip_id, edge: t.edge });
            if selected {
                s.append_border(&rounded, &[2.0; 4], &[css_color(self, "tempo_error"); 4]);
            } else {
                s.append_border(&rounded, &[1.0; 4], &[gdk::RGBA::new(0.0, 0.0, 0.0, 0.7); 4]);
            }
        }
    }

    /// Resolve's green bracket on a selected edit point: a bar on the edit, with feet
    /// toward the clip ending there and the clip starting there.
    fn draw_edit_bracket(&self, s: &gtk::Snapshot, row: &Row, track: &Track, at: i64) {
        let shift = track.clips.iter().find_map(|c| {
            let (a, b) = self.tweened(c.id, c.timeline_in, c.timeline_out);
            (c.timeline_in == at).then_some(a - at).or((c.timeline_out == at).then_some(b - at))
        });
        let x = self.x_of(at + shift.unwrap_or(0)) as f32;
        let green = css_color(self, "tempo_ok");
        let (top, bottom) = (row.y + 1.0, row.y + row.h - 2.0);
        let bar = |x: f32, y: f32, w: f32, h: f32| s.append_color(&green, &graphene::Rect::new(x, y, w, h));
        bar(x - 1.5, top, 3.0, bottom - top);
        if track.has_clip_ending_at(at) {
            bar(x - 7.0, top, 7.0, 2.0);
            bar(x - 7.0, bottom - 2.0, 7.0, 2.0);
        }
        if track.clips.iter().any(|c| c.timeline_in == at) {
            bar(x, top, 7.0, 2.0);
            bar(x, bottom - 2.0, 7.0, 2.0);
        }
    }

    /// A small black tooltip below and right of the pointer.
    fn draw_tip(&self, s: &gtk::Snapshot, x: f32, y: f32, text: &str) {
        let layout = self.create_pango_layout(Some(text));
        let (tw, th) = layout.pixel_size();
        let (bw, bh) = (tw as f32 + 12.0, th as f32 + 6.0);
        let bx = (x + 12.0).min(self.width() as f32 - bw - 2.0).max(2.0);
        let by = y + 18.0;
        let bounds = graphene::Rect::new(bx, by, bw, bh);
        s.push_rounded_clip(&gsk::RoundedRect::from_rect(bounds, 3.0));
        s.append_color(&gdk::RGBA::new(0.0, 0.0, 0.0, 0.9), &bounds);
        s.pop();
        s.save();
        s.translate(&graphene::Point::new(bx + 6.0, by + 3.0));
        s.append_layout(&layout, &gdk::RGBA::WHITE);
        s.restore();
    }

    fn draw_playhead(&self, s: &gtk::Snapshot) {
        let (w, h) = (self.width() as f32, self.height() as f32);
        let accent = css_color(self, "tempo_accent");
        let rect = |x, y, w, h| graphene::Rect::new(x, y, w, h);
        let px = self.x_of(self.shown_playhead()) as f32;
        if px >= -8.0 && px <= w + 8.0 {
            s.append_color(&accent, &rect(px, 0.0, 1.0, h));
            s.append_color(&accent, &rect(px - 6.0, RULER_H - 16.0, 13.0, 10.0));
            s.append_color(&accent, &rect(px - 3.0, RULER_H - 6.0, 7.0, 3.0));
        }
    }

    /// Redraw the canvas and the playhead above it.
    fn redraw(&self) {
        self.queue_draw();
        self.redraw_playhead();
    }

    fn redraw_playhead(&self) {
        if let Some(layer) = self.imp().playhead_layer.borrow().as_ref() {
            layer.queue_draw();
        }
    }

    /// The layer that shows this canvas's playhead; place it over the canvas.
    pub fn playhead_layer(&self) -> PlayheadLayer {
        let layer: PlayheadLayer = glib::Object::builder().build();
        layer.set_can_target(false);
        layer.set_overflow(gtk::Overflow::Hidden);
        layer.imp().canvas.set(Some(self));
        *self.imp().playhead_layer.borrow_mut() = Some(layer.clone());
        layer
    }
}

mod layer_imp {
    use super::*;

    #[derive(Default)]
    pub struct PlayheadLayer {
        pub canvas: glib::WeakRef<super::TimelineCanvas>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PlayheadLayer {
        const NAME: &'static str = "TempoPlayheadLayer";
        type Type = super::PlayheadLayer;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for PlayheadLayer {}

    impl WidgetImpl for PlayheadLayer {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            if let Some(canvas) = self.canvas.upgrade() {
                canvas.draw_playhead(snapshot);
            }
        }
    }
}

glib::wrapper! {
    pub struct PlayheadLayer(ObjectSubclass<layer_imp::PlayheadLayer>)
        @extends gtk::Widget, @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

/// Canvas, scrollbar, and the column of track headers to its left.
pub struct TimelineArea {
    pub root: gtk::Box,
    pub canvas: TimelineCanvas,
}

impl TimelineArea {
    pub fn new(state: &Rc<AppState>, viewer: &Rc<Viewer>) -> Self {
        let canvas = TimelineCanvas::new(state, viewer);

        let timecode_label = label("00:00:00:00", &["tempo-timecode-big"]);
        let tc_on_switch = timecode_label.clone();
        let timecode_box = gtk::Box::builder().css_classes(["timecode-box"]).halign(gtk::Align::Fill).build();
        timecode_label.set_halign(gtk::Align::Center);
        timecode_label.set_width_chars(11);
        timecode_box.append(&timecode_label);
        timecode_box.set_height_request(RULER_H as i32);

        let header_rows = gtk::Box::builder().orientation(gtk::Orientation::Vertical).vexpand(true).build();
        let headers = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .width_request(150)
            .hexpand(false)
            .css_classes(["track-headers"])
            .build();
        headers.append(&timecode_box);
        headers.append(&header_rows);

        let header_menu: Rc<RefCell<Option<gtk::PopoverMenu>>> = Rc::new(RefCell::new(None));
        let m = header_menu.clone();
        headers.connect_unrealize(move |_| {
            if let Some(old) = m.take() {
                old.unparent();
            }
        });

        // Track header context-menu actions.
        let group = gio::SimpleActionGroup::new();
        let add_video = gio::SimpleAction::new("add-video", None);
        let s = state.clone();
        add_video.connect_activate(move |_, _| {
            let next = s
                .with_timeline(|t| t.tracks.iter().filter(|tr| tr.kind == TrackKind::Video).map(|tr| tr.kind_index).max().unwrap_or(0) + 1)
                .unwrap_or(1);
            s.execute(Box::new(AddTrackCommand::new(Track::new(TrackKind::Video, next, format!("Video {next}"), 0))));
        });
        group.add_action(&add_video);

        for (action_name, audio_type) in [("add-audio-stereo", AudioTrackType::Stereo), ("add-audio-mono", AudioTrackType::Mono)] {
            let act = gio::SimpleAction::new(action_name, None);
            let s = state.clone();
            act.connect_activate(move |_, _| {
                let next = s
                    .with_timeline(|t| t.tracks.iter().filter(|tr| tr.kind == TrackKind::Audio).map(|tr| tr.kind_index).max().unwrap_or(0) + 1)
                    .unwrap_or(1);
                let mut track = Track::new(TrackKind::Audio, next, format!("Audio {next}"), 0);
                track.audio_type = audio_type;
                s.execute(Box::new(AddTrackCommand::new(track)));
            });
            group.add_action(&act);
        }

        let delete_track = gio::SimpleAction::new("delete", Some(glib::VariantTy::STRING));
        let s = state.clone();
        delete_track.connect_activate(move |_, target| {
            if let Some(id) = target.and_then(|v| v.str()).and_then(|txt| Uuid::parse_str(txt).ok()) {
                s.execute(Box::new(DeleteTrackCommand::new(id)));
            }
        });
        group.add_action(&delete_track);
        headers.insert_action_group("track", Some(&group));

        // The big timecode is the timeline's, also while a source clip is in the viewer.
        let s = state.clone();
        viewer.connect_position(move |_| timecode_label.set_text(&s.position_tc(s.playhead_us())));

        let right = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let stack = gtk::Overlay::builder().child(&canvas).build();
        stack.add_overlay(&canvas.playhead_layer());
        right.append(&stack);
        right.append(&gtk::Scrollbar::new(gtk::Orientation::Horizontal, Some(&canvas.hadjustment())));

        let root = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).vexpand(true).build();
        root.append(&headers);
        root.append(&right);
        root.append(&audio_meter(state));

        let header_track_map: Rc<RefCell<Vec<(gtk::Box, Uuid)>>> = Rc::new(RefCell::new(Vec::new()));
        let s = state.clone();
        let rows_box = header_rows.clone();
        let rows_map = header_track_map.clone();
        let rebuild = Rc::new(move || build_headers(&s, &rows_box, &rows_map));
        rebuild();
        let rebuild_ev = rebuild.clone();
        let s = state.clone();
        state.connect(move |change| {
            if matches!(change, Change::Project | Change::Timeline | Change::Options) {
                rebuild_ev();
            }
            // Switching timelines can leave the playhead at 0 -> 0, so no position event comes.
            if matches!(change, Change::Project | Change::Timeline) {
                tc_on_switch.set_text(&s.position_tc(s.playhead_us()));
            }
        });
        let rebuild_h = rebuild;
        let rows_for_h = header_rows.clone();
        canvas.connect_heights(move || {
            rebuild_h();
            rows_for_h.queue_resize();
        });

        // Shift+scroll on track headers adjusts track heights too.
        let h_scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
        let c_scroll = canvas.clone();
        h_scroll.connect_scroll(move |ctl, dx, dy| {
            if ctl.current_event_state().contains(gdk::ModifierType::SHIFT_MASK) {
                let raw = if dy.abs() >= dx.abs() { dy } else { dx };
                if raw != 0.0 {
                    c_scroll.adjust_track_heights((-raw as f32) * 8.0);
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        header_rows.add_controller(h_scroll);

        // Right-click on track headers / header column opens the track context menu.
        let click = gtk::GestureClick::builder().button(gdk::BUTTON_SECONDARY).build();
        let s = state.clone();
        let h_rows = header_rows.clone();
        let hdrs = headers.clone();
        let rows_map = header_track_map;
        let h_menu = header_menu;
        let del_act = delete_track;
        click.connect_pressed(move |gesture, _, x, y| {
            if s.project.borrow().is_none() {
                return;
            }
            gesture.set_state(gtk::EventSequenceState::Claimed);
            let pt_rows = graphene::Point::new(x as f32, y as f32);
            let track_id = rows_map.borrow().iter().find_map(|(box_w, id)| {
                box_w
                    .compute_bounds(&h_rows)
                    .filter(|b| {
                        pt_rows.x() >= b.x()
                            && pt_rows.x() < b.x() + b.width()
                            && pt_rows.y() >= b.y()
                            && pt_rows.y() < b.y() + b.height()
                    })
                    .map(|_| *id)
            });
            let can_delete = track_id
                .and_then(|id| {
                    s.with_timeline(|t| {
                        t.find_track(id)
                            .is_some_and(|tr| t.tracks.iter().filter(|o| o.kind == tr.kind).count() > 1)
                    })
                })
                .unwrap_or(false);
            del_act.set_enabled(can_delete);

            let menu = gio::Menu::new();
            let add_sec = gio::Menu::new();
            add_sec.append(Some("Add Video Track"), Some("track.add-video"));
            add_sec.append(Some("Add Audio Track (Stereo)"), Some("track.add-audio-stereo"));
            add_sec.append(Some("Add Audio Track (Mono)"), Some("track.add-audio-mono"));
            menu.append_section(None, &add_sec);

            let del_sec = gio::Menu::new();
            let del_item = gio::MenuItem::new(Some("Delete Track"), None);
            let target_id = track_id.unwrap_or(Uuid::nil());
            del_item.set_action_and_target_value(Some("track.delete"), Some(&target_id.to_string().to_variant()));
            del_sec.append_item(&del_item);
            menu.append_section(None, &del_sec);

            if let Some(old) = h_menu.take() {
                old.unparent();
            }
            let pt = h_rows.compute_point(&hdrs, &pt_rows).unwrap_or(pt_rows);
            let popover = gtk::PopoverMenu::from_model(Some(&menu));
            popover.set_parent(&hdrs);
            popover.set_has_arrow(false);
            popover.set_halign(gtk::Align::Start);
            popover.set_pointing_to(Some(&gdk::Rectangle::new(pt.x() as i32, pt.y() as i32, 1, 1)));
            *h_menu.borrow_mut() = Some(popover.clone());
            popover.popup();
        });
        header_rows.add_controller(click);

        // Ghost tracks get a header too while a clip is dragged toward them.
        canvas.connect_ghost(move |ghost| {
            let (video, audio) = ghost.unwrap_or_default();
            if let Some(w) = header_rows.first_child().and_then(|b| b.first_child()) {
                w.set_visible(video);
            }
            if let Some(w) = header_rows.last_child().and_then(|b| b.first_child()) {
                w.set_visible(audio);
            }
        });

        Self { root, canvas }
    }
}

fn build_headers(state: &Rc<AppState>, container: &gtk::Box, track_map: &RefCell<Vec<(gtk::Box, Uuid)>>) {
    track_map.borrow_mut().clear();
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
    let rows = state.with_timeline(rows).unwrap_or_default();
    let audio_h = TRACK_HEIGHTS.with(|c| c.get().1);
    let next = |kind: TrackKind| rows.iter().filter(|r| r.kind == kind).map(|r| r.index).max().unwrap_or(0) + 1;
    let channels = state.with_timeline(|t| t.tracks.iter().find(|tr| tr.kind == TrackKind::Audio).map(|tr| tr.audio_type.label())).flatten().unwrap_or("2.0");
    let ghost_header = |class: &str, h: i32, text: String, extra: Option<&str>| {
        let slot = gtk::Box::builder().height_request(h).build();
        let header = gtk::Box::builder().spacing(4).hexpand(true).css_classes(["track-header", class]).build();
        header.append(&gtk::Button::builder().label(text).css_classes(["tool", "track-name"]).valign(gtk::Align::Center).sensitive(false).build());
        if let Some(e) = extra {
            header.append(&label(e, &["tempo-dim"]));
        }
        header.set_visible(false);
        slot.append(&header);
        slot
    };
    container.append(&ghost_header("video", GHOST_BAND_H as i32, format!("V{}", next(TrackKind::Video)), None));
    let ghost_audio_n = next(TrackKind::Audio);
    let mut divider_done = false;
    for row in rows {
        if row.kind == TrackKind::Audio && !divider_done {
            divider_done = true;
            container.append(&gtk::Box::builder().css_classes(["track-divider"]).build());
        }
        let (prefix, class) = match row.kind {
            TrackKind::Video => ("V", "video"),
            TrackKind::Audio => ("A", "audio"),
        };
        let header = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(4)
            .height_request(row.h as i32)
            .css_classes(["track-header", class])
            .build();

        // Name box: outlined on the destination track; click to make it the destination.
        let is_dest = match row.kind {
            TrackKind::Video => state.dest_video.get() == row.index,
            TrackKind::Audio => state.dest_audio.get() == row.index,
        };
        let name = gtk::Button::builder()
            .label(format!("{prefix}{}", row.index))
            .tooltip_text("Make this the destination track for new clips")
            .css_classes(if is_dest { vec!["tool", "track-name", "destination"] } else { vec!["tool", "track-name"] })
            .valign(gtk::Align::Center)
            .build();
        let s = state.clone();
        name.connect_clicked(move |_| {
            match row.kind {
                TrackKind::Video => s.dest_video.set(row.index),
                TrackKind::Audio => s.dest_audio.set(row.index),
            }
            s.emit(Change::Options);
        });
        header.append(&name);
        if row.kind == TrackKind::Audio {
            let channels = state.with_timeline(|t| t.find_track(row.track_id).map(|tr| tr.audio_type.label())).flatten().unwrap_or("2.0");
            header.append(&label(channels, &["tempo-dim"]));
        }

        let lock = gtk::ToggleButton::builder()
            .icon_name(if row.locked { "changes-prevent-symbolic" } else { "changes-allow-symbolic" })
            .tooltip_text("Lock track")
            .active(row.locked)
            .css_classes(["tool"])
            .valign(gtk::Align::Center)
            .build();
        let s = state.clone();
        lock.connect_toggled(move |b| {
            s.execute(Box::new(SetTrackFlagCommand::new(row.track_id, TrackFlag::Locked, b.is_active())));
        });
        header.append(&lock);

        // Resolve has Solo on audio tracks only: while any is on, playback hears just those.
        if row.kind == TrackKind::Audio {
            let solo = gtk::ToggleButton::builder()
                .label("S")
                .tooltip_text("Solo: hear only this track while playing. Export is not affected")
                .active(row.solo)
                .css_classes(["tool", "track-solo"])
                .valign(gtk::Align::Center)
                .build();
            let s = state.clone();
            solo.connect_toggled(move |b| {
                s.execute(Box::new(SetTrackFlagCommand::new(row.track_id, TrackFlag::Solo, b.is_active())));
            });
            header.append(&solo);
        }

        let (icon_on, icon_off, tip) = match row.kind {
            TrackKind::Video => ("view-reveal-symbolic", "view-conceal-symbolic", "Show or hide this video track"),
            TrackKind::Audio => ("audio-volume-high-symbolic", "audio-volume-muted-symbolic", "Mute this audio track"),
        };
        let enable = gtk::ToggleButton::builder()
            .icon_name(if row.enabled { icon_on } else { icon_off })
            .tooltip_text(tip)
            .active(row.enabled)
            .css_classes(["tool"])
            .valign(gtk::Align::Center)
            .build();
        let s = state.clone();
        enable.connect_toggled(move |b| {
            s.execute(Box::new(SetTrackFlagCommand::new(row.track_id, TrackFlag::Enabled, b.is_active())));
        });
        header.append(&enable);
        // Controls sit left, as in Resolve, so Lock lines up on every row.
        header.append(&gtk::Box::builder().hexpand(true).build());

        track_map.borrow_mut().push((header.clone(), row.track_id));
        container.append(&header);
    }
    container.append(&ghost_header("audio", audio_h as i32, format!("A{ghost_audio_n}"), Some(channels)));
}

/// Two bars showing the output level, left and right. Redrawn only when the level changes.
fn audio_meter(state: &Rc<AppState>) -> gtk::DrawingArea {
    const FLOOR_DB: f32 = -48.0;
    let meter = gtk::DrawingArea::builder().content_width(26).vexpand(true).tooltip_text("Output level").build();
    meter.update_property(&[gtk::accessible::Property::Label("Output level")]);
    let shown = Rc::new(Cell::new((0.0f32, 0.0f32)));

    let levels = shown.clone();
    meter.set_draw_func(move |area, cr, w, h| {
        let paint = |name: &str| {
            let c = css_color(area, name);
            cr.set_source_rgba(c.red() as f64, c.green() as f64, c.blue() as f64, c.alpha() as f64);
        };
        paint("tempo_bg_panel");
        let _ = cr.paint();
        let (w, h) = (w as f64, h as f64);
        let bar_w = (w - 10.0) / 2.0;
        let (l, r) = levels.get();
        for (i, level) in [l, r].into_iter().enumerate() {
            let x = 4.0 + i as f64 * (bar_w + 2.0);
            paint("tempo_bg_deep");
            cr.rectangle(x, 4.0, bar_w, h - 8.0);
            let _ = cr.fill();
            // Height on a decibel scale; colour by how close to clipping.
            let db = 20.0 * level.max(1e-6).log10();
            let fraction = ((db - FLOOR_DB) / -FLOOR_DB).clamp(0.0, 1.0) as f64;
            if fraction > 0.0 {
                paint(if db > -6.0 { "tempo_error" } else if db > -12.0 { "tempo_warn" } else { "tempo_ok" });
                let bar_h = (h - 8.0) * fraction;
                cr.rectangle(x, h - 4.0 - bar_h, bar_w, bar_h);
                let _ = cr.fill();
            }
        }
    });

    let s = state.clone();
    meter.add_tick_callback(move |area, _| {
        let now = s.player.levels();
        let old = shown.get();
        // Fall back smoothly so short sounds stay visible for a moment.
        let next = (now.0.max(old.0 * 0.92), now.1.max(old.1 * 0.92));
        let next = (if next.0 < 0.004 { 0.0 } else { next.0 }, if next.1 < 0.004 { 0.0 } else { next.1 });
        if next != old {
            shown.set(next);
            area.queue_draw();
        }
        glib::ControlFlow::Continue
    });
    meter
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempo_timeline::{Marker, MarkerColor};

    #[test]
    fn ghost_lanes_follow_drop_position_and_source_kind() {
        let (band, below, mid) = (RULER_H as f64 + 1.0, 500.0, 200.0);
        assert_eq!(ghost_lanes(RULER_H as f64 - 1.0, below, MediaType::Video, true), None);
        assert_eq!(ghost_lanes(band, below, MediaType::Video, true), Some((true, true)));
        assert_eq!(ghost_lanes(band, below, MediaType::Video, false), Some((true, false)));
        assert_eq!(ghost_lanes(band, below, MediaType::Image, false), Some((true, false)));
        assert_eq!(ghost_lanes(band, below, MediaType::Audio, true), None);
        assert_eq!(ghost_lanes(mid, below, MediaType::Audio, true), None);
        assert_eq!(ghost_lanes(below, below, MediaType::Audio, true), Some((false, true)));
        assert_eq!(ghost_lanes(below, below, MediaType::Video, true), None);
    }

    #[test]
    fn tween_runs_from_old_to_new_and_eases_out() {
        assert_eq!(lerp_i(100, 200, 0.0), 100);
        assert_eq!(lerp_i(100, 200, 1.0), 200);
        assert_eq!(lerp_i(200, 100, 0.5), 150);
        assert_eq!((ease_out(0.0), ease_out(1.0), ease_out(2.0)), (0.0, 1.0, 1.0));
        // Fast first, slow last.
        assert!(ease_out(0.5) > 0.5);
    }

    #[test]
    fn length_tip_reports_the_edge_movement_then_the_length() {
        // Resolve: left edge of a 24-frame transition dragged 11 frames right.
        assert_eq!(length_tip(24 * 41_667, 13 * 41_667, true, false, 24.0), "+00:11\n00:13");
        // The right edge dragged 5 frames right lengthens it; left edge moving left is -.
        assert_eq!(length_tip(10 * 41_667, 15 * 41_667, false, false, 24.0), "+00:05\n00:15");
        assert_eq!(length_tip(10 * 41_667, 15 * 41_667, true, false, 24.0), "-00:05\n00:15");
        // Centred on a cut both edges move, so each moves half the length change.
        assert_eq!(length_tip(10 * 41_667, 14 * 41_667, false, true, 24.0), "+00:02\n00:14");
        // An odd change steps back to a whole-frame edge movement: 10 -> 15 becomes 10 -> 14.
        assert_eq!(even_steps(10 * 41_667, 15 * 41_667, true, 41_667), 14 * 41_667);
        assert_eq!(even_steps(10 * 41_667, 7 * 41_667, true, 41_667), 8 * 41_667);
        assert_eq!(even_steps(10 * 41_667, 15 * 41_667, false, 41_667), 15 * 41_667);
    }

    #[test]
    fn thumbnail_size_matches_probe_sizes() {
        assert_eq!(thumbnail_size(160 * 90 * 4), Some((160, 90)));
        assert_eq!(thumbnail_size(7), None);
    }

    #[test]
    fn ghost_band_is_a_full_video_track() {
        assert_eq!(GHOST_BAND_H, VIDEO_H);
    }

    #[test]
    fn offset_label_shows_sign_and_frames() {
        assert_eq!(offset_label(0, 24.0), "00:00");
        assert_eq!(offset_label(-(3 * 1_000_000 + 5 * 41_667), 24.0), "-03:05");
        assert_eq!(offset_label(1_000_000, 30.0), "01:00");
    }

    #[test]
    fn detail_zoom_fits_a_few_seconds() {
        assert_eq!(detail_pps(800.0, 4.0), 200.0);
        assert_eq!(detail_pps(100_000.0, 4.0), MAX_PPS);
    }

    #[test]
    fn marker_near_picks_the_closest_within_reach() {
        let markers = [Marker::new(1_000_000, "a", MarkerColor::Blue), Marker::new(1_100_000, "b", MarkerColor::Blue)];
        let x_of = |us: i64| us as f64 / 10_000.0; // 100 px per second
        assert_eq!(marker_near(&markers, 109.0, 6.0, x_of).map(|m| m.name.as_str()), Some("b"));
        assert!(marker_near(&markers, 50.0, 6.0, x_of).is_none());
    }

    #[test]
    fn marker_tip_has_timecode_over_name() {
        assert_eq!(marker_tip("00:00:01:00", "Intro"), "00:00:01:00\nIntro");
        assert_eq!(marker_tip("00:00:01:00", ""), "00:00:01:00");
    }

    #[test]
    fn track_heights_clamp_and_update_rows() {
        assert_eq!(adjust_track_height(60.0, -50.0), MIN_TRACK_H);
        assert_eq!(adjust_track_height(60.0, 200.0), MAX_TRACK_H);
        assert_eq!(adjust_track_height(60.0, 16.0), 76.0);

        TRACK_HEIGHTS.with(|c| c.set((90.0, 80.0)));
        let tl = Timeline::with_tracks("T", 1, 1);
        let r = rows(&tl);
        assert_eq!(r[0].h, 90.0);
        assert_eq!(r[1].h, 80.0);
        TRACK_HEIGHTS.with(|c| c.set((VIDEO_H, AUDIO_H)));
    }
}
