//! The timeline: one custom-drawn canvas (ruler, tracks, clips, markers, playhead)
//! plus a column of real widgets for the track headers.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use glib::subclass::prelude::*;
use gtk4 as gtk;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use gtk4::{gdk, graphene, gsk, pango};
use tempo_timeline::{
    ClipType, MarkerColor, MoveClipCommand, RippleTrimCommand, SetTrackFlagCommand, Timeline, TrackFlag, TrackKind,
    TrimClipCommand, TrimEdge,
};
use uuid::Uuid;

use crate::actions::{self, Place};
use crate::state::{timecode, AppState, Change, Tool};
use crate::util::{css_color, label};
use crate::viewer::Viewer;

pub const RULER_H: f32 = 48.0;
const VIDEO_H: f32 = 60.0;
const AUDIO_H: f32 = 54.0;
const DIVIDER_H: f32 = 2.0;
const NAME_BAR_H: f32 = 22.0;
const EDGE_GRAB_PX: f64 = 6.0;
const SNAP_PX: f64 = 8.0;
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
}

/// Video tracks stack upward and audio tracks downward from the divider, as in Resolve.
pub fn rows(timeline: &Timeline) -> Vec<Row> {
    let mut video: Vec<_> = timeline.tracks.iter().filter(|t| t.kind == TrackKind::Video).collect();
    video.sort_by_key(|t| std::cmp::Reverse(t.kind_index));
    let mut audio: Vec<_> = timeline.tracks.iter().filter(|t| t.kind == TrackKind::Audio).collect();
    audio.sort_by_key(|t| t.kind_index);

    let mut out = Vec::new();
    let mut y = RULER_H;
    for (list, h) in [(video, VIDEO_H), (audio, AUDIO_H)] {
        for t in list {
            out.push(Row { track_id: t.id, kind: t.kind, index: t.kind_index, y, h, locked: t.locked, enabled: t.enabled });
            y += h;
        }
        if h == VIDEO_H {
            y += DIVIDER_H;
        }
    }
    out
}

#[derive(Clone)]
enum Drag {
    Scrub,
    Move { clip: Uuid, from_track: Uuid, kind: TrackKind, orig_in: i64, len: i64, new_in: i64, to_track: Uuid },
    Trim { clip: Uuid, edge: TrimEdge, delta: i64 },
}

mod imp {
    use super::*;

    pub struct TimelineCanvas {
        pub state: RefCell<Option<Rc<AppState>>>,
        pub viewer: RefCell<Option<Rc<Viewer>>>,
        pub pps: Cell<f64>,
        pub scroll: Cell<f64>,
        pub playhead: Cell<i64>,
        pub(super) drag: RefCell<Option<Drag>>,
        pub press: Cell<(f64, f64)>,
        pub pointer_x: Cell<f64>,
        pub snap_line: Cell<Option<i64>>,
        pub hadj: gtk::Adjustment,
        pub zoom_listener: RefCell<Option<Box<dyn Fn(f64)>>>,
        /// Zoom-to-fit was asked for before the widget had a size.
        pub fit_pending: Cell<bool>,
    }

    impl Default for TimelineCanvas {
        fn default() -> Self {
            Self {
                state: RefCell::new(None),
                viewer: RefCell::new(None),
                pps: Cell::new(20.0),
                scroll: Cell::new(0.0),
                playhead: Cell::new(0),
                drag: RefCell::new(None),
                press: Cell::new((0.0, 0.0)),
                pointer_x: Cell::new(0.0),
                snap_line: Cell::new(None),
                hadj: gtk::Adjustment::new(0.0, 0.0, 1.0, 40.0, 200.0, 1.0),
                zoom_listener: RefCell::new(None),
                fit_pending: Cell::new(false),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for TimelineCanvas {
        const NAME: &'static str = "TempoTimelineCanvas";
        type Type = super::TimelineCanvas;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for TimelineCanvas {}

    impl WidgetImpl for TimelineCanvas {
        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            self.parent_size_allocate(width, height, baseline);
            if self.fit_pending.get() {
                self.obj().zoom_fit();
            }
            self.obj().update_adjustment();
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
        let obj: Self = glib::Object::builder().build();
        obj.set_hexpand(true);
        obj.set_vexpand(true);
        obj.set_focusable(true);
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
                        c.update_adjustment();
                        c.queue_draw();
                    }
                    Change::Timeline => {
                        c.update_adjustment();
                        c.queue_draw();
                    }
                    Change::Selection | Change::Options | Change::Waveform => c.queue_draw(),
                    _ => {}
                }
            }
        });

        let c = obj.downgrade();
        viewer.connect_position(move |pos| {
            if let Some(c) = c.upgrade() {
                c.set_playhead(pos);
            }
        });

        let c = obj.downgrade();
        obj.imp().hadj.connect_value_changed(move |adj| {
            if let Some(c) = c.upgrade() {
                if (c.imp().scroll.get() - adj.value()).abs() > 0.5 {
                    c.imp().scroll.set(adj.value());
                    c.queue_draw();
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

    fn x_of(&self, us: i64) -> f64 {
        us as f64 / 1_000_000.0 * self.pps() - self.imp().scroll.get()
    }

    fn us_at(&self, x: f64) -> i64 {
        (((x + self.imp().scroll.get()) / self.pps()) * 1_000_000.0).max(0.0) as i64
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
        self.queue_draw();
    }

    /// Change zoom keeping the time under `anchor_x` in place.
    pub fn set_pps(&self, pps: f64, anchor_x: Option<f64>) {
        let anchor = anchor_x.unwrap_or(self.x_of(self.imp().playhead.get()).clamp(0.0, self.width() as f64));
        let t = self.us_at(anchor);
        self.imp().pps.set(pps.clamp(MIN_PPS, MAX_PPS));
        self.set_scroll(t as f64 / 1_000_000.0 * self.pps() - anchor);
        self.notify_zoom();
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
            return;
        }
        self.imp().fit_pending.set(false);
        let end = self.state().with_timeline(|t| t.tracks.iter().map(|tr| tr.duration_us()).max().unwrap_or(0)).unwrap_or(0);
        // An empty timeline shows its first minute.
        let dur = if end > 0 { end as f64 / 1_000_000.0 } else { 60.0 };
        self.imp().pps.set(((self.width() as f64 - 40.0) / dur).clamp(MIN_PPS, MAX_PPS));
        self.set_scroll(0.0);
        self.notify_zoom();
    }

    fn set_playhead(&self, us: i64) {
        self.imp().playhead.set(us);
        // Follow the playhead while playing.
        if self.state().player.is_playing() {
            let x = self.x_of(us);
            let w = self.width() as f64;
            if x > w - 20.0 || x < 0.0 {
                self.set_scroll(us as f64 / 1_000_000.0 * self.pps() - w * 0.1);
                return;
            }
        }
        self.queue_draw();
    }

    fn row_at(&self, y: f64) -> Option<Row> {
        let state = self.state();
        let rows = state.with_timeline(rows).unwrap_or_default();
        rows.into_iter().find(|r| y >= r.y as f64 && y < (r.y + r.h) as f64)
    }

    /// Clip under a point, and which edge (if any) the pointer is close to.
    fn hit(&self, x: f64, y: f64) -> Option<(Row, Uuid, Option<TrimEdge>)> {
        let row = self.row_at(y)?;
        let t = self.us_at(x);
        let state = self.state();
        let (id, tin, tout) = state
            .with_timeline(|tl| {
                tl.find_track(row.track_id).and_then(|tr| tr.clip_at(t)).map(|c| (c.id, c.timeline_in, c.timeline_out))
            })
            .flatten()?;
        let edge = if (x - self.x_of(tin)).abs() <= EDGE_GRAB_PX {
            Some(TrimEdge::In)
        } else if (x - self.x_of(tout)).abs() <= EDGE_GRAB_PX {
            Some(TrimEdge::Out)
        } else {
            None
        };
        Some((row, id, edge))
    }

    fn snap(&self, us: i64, ignore: Option<Uuid>) -> (i64, bool) {
        let state = self.state();
        if !state.snapping.get() {
            return (us, false);
        }
        let threshold = (SNAP_PX / self.pps() * 1_000_000.0) as i64;
        let playhead = self.imp().playhead.get();
        let mut best = state.with_timeline(|t| t.find_snap_point(us, threshold, ignore)).flatten();
        if (playhead - us).abs() <= threshold && best.is_none_or(|b| (b - us).abs() > (playhead - us).abs()) {
            best = Some(playhead);
        }
        match best {
            Some(b) => (b, true),
            None => (us, false),
        }
    }

    fn install_controllers(&self) {
        // Press, drag, release.
        let drag = gtk::GestureDrag::new();
        let c = self.downgrade();
        drag.connect_drag_begin(move |_, x, y| {
            if let Some(c) = c.upgrade() {
                c.grab_focus();
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
        drag.connect_drag_end(move |_, _, _| {
            if let Some(c) = c.upgrade() {
                c.on_release();
            }
        });
        self.add_controller(drag);

        // Pointer shape near clip edges.
        let motion = gtk::EventControllerMotion::new();
        let c = self.downgrade();
        motion.connect_motion(move |_, x, y| {
            if let Some(c) = c.upgrade() {
                c.imp().pointer_x.set(x);
                let tool = c.state().tool.get();
                let cursor = match (tool, c.hit(x, y)) {
                    (Tool::Blade, Some(_)) => Some("crosshair"),
                    (Tool::Select | Tool::Trim, Some((_, _, Some(_)))) => Some("col-resize"),
                    _ => None,
                };
                c.set_cursor_from_name(cursor);
            }
        });
        self.add_controller(motion);

        // Alt+scroll zooms, anything else scrolls sideways.
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
        let c = self.downgrade();
        scroll.connect_scroll(move |ctl, dx, dy| {
            let Some(c) = c.upgrade() else { return glib::Propagation::Proceed };
            if ctl.current_event_state().contains(gdk::ModifierType::ALT_MASK) {
                let factor = if dy < 0.0 { 1.2 } else { 1.0 / 1.2 };
                c.set_pps(c.pps() * factor, Some(c.imp().pointer_x.get()));
            } else {
                c.set_scroll(c.imp().scroll.get() + (dx + dy) * 60.0);
            }
            glib::Propagation::Stop
        });
        self.add_controller(scroll);

        // Clips dragged in from the Media Pool carry the source id as text.
        let drop = gtk::DropTarget::new(glib::Type::STRING, gdk::DragAction::COPY);
        let c = self.downgrade();
        drop.connect_drop(move |_, value, x, y| {
            let Some(c) = c.upgrade() else { return false };
            let Some(id) = value.get::<String>().ok().and_then(|s| Uuid::parse_str(&s).ok()) else { return false };
            let (at, _) = c.snap(c.us_at(x), None);
            let track = c.row_at(y).map(|r| r.track_id);
            actions::place_source(&c.state(), id, Place::Overwrite, Some(at), track);
            true
        });
        self.add_controller(drop);
    }

    fn on_press(&self, x: f64, y: f64) {
        let imp = self.imp();
        imp.press.set((x, y));
        let state = self.state();

        if y < RULER_H as f64 {
            *imp.drag.borrow_mut() = Some(Drag::Scrub);
            self.scrub(x);
            return;
        }

        match (state.tool.get(), self.hit(x, y)) {
            (Tool::Blade, Some((row, id, _))) => {
                if !row.locked {
                    let (at, _) = self.snap(self.us_at(x), Some(id));
                    actions::split_at(&state, id, at - at % state.frame_us());
                }
            }
            (_, Some((row, id, edge))) => {
                state.select(Some(id));
                if row.locked {
                    return;
                }
                let Some(clip) = state.selected_clip() else { return };
                *imp.drag.borrow_mut() = Some(match edge {
                    Some(edge) => Drag::Trim { clip: id, edge, delta: 0 },
                    None => Drag::Move {
                        clip: id,
                        from_track: row.track_id,
                        kind: row.kind,
                        orig_in: clip.timeline_in,
                        len: clip.duration_us(),
                        new_in: clip.timeline_in,
                        to_track: row.track_id,
                    },
                });
            }
            (_, None) => state.select(None),
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
        let (px, _) = imp.press.get();
        let delta_us = ((x - px) / self.pps() * 1_000_000.0) as i64;
        let frame = self.state().frame_us();
        let mut drag = imp.drag.borrow_mut();
        match drag.as_mut() {
            Some(Drag::Scrub) => {
                drop(drag);
                self.scrub(x);
                return;
            }
            Some(Drag::Move { clip, kind, orig_in, len, new_in, to_track, .. }) => {
                let raw = (*orig_in + delta_us).max(0);
                // Snap whichever end is closer to something.
                let (s_in, hit_in) = self.snap(raw, Some(*clip));
                let (s_out, hit_out) = self.snap(raw + *len, Some(*clip));
                let snapped = if hit_in {
                    imp.snap_line.set(Some(s_in));
                    s_in
                } else if hit_out {
                    imp.snap_line.set(Some(s_out));
                    s_out - *len
                } else {
                    imp.snap_line.set(None);
                    raw - raw % frame
                };
                *new_in = snapped.max(0);
                if let Some(row) = self.row_at(y) {
                    if row.kind == *kind && !row.locked {
                        *to_track = row.track_id;
                    }
                }
            }
            Some(Drag::Trim { clip, edge, delta }) => {
                let Some((tin, tout)) = self
                    .state()
                    .with_timeline(|t| t.find_clip(*clip).map(|(_, c)| (c.timeline_in, c.timeline_out)))
                    .flatten()
                else {
                    return;
                };
                let base = if *edge == TrimEdge::In { tin } else { tout };
                let (target, hit) = self.snap(base + delta_us, Some(*clip));
                imp.snap_line.set(hit.then_some(target));
                let target = if hit { target } else { target - target % frame };
                *delta = target - base;
            }
            None => return,
        }
        drop(drag);
        self.queue_draw();
    }

    fn on_release(&self) {
        let imp = self.imp();
        imp.snap_line.set(None);
        let drag = imp.drag.borrow_mut().take();
        let state = self.state();
        match drag {
            Some(Drag::Move { clip, from_track, orig_in, new_in, to_track, .. }) => {
                if new_in != orig_in || to_track != from_track {
                    state.execute(Box::new(MoveClipCommand::new(clip, from_track, to_track, orig_in, new_in)));
                }
            }
            // Trim mode ripples: later clips follow. Selection mode leaves a gap.
            Some(Drag::Trim { clip, edge, delta }) if delta != 0 => {
                if state.tool.get() == Tool::Trim {
                    state.execute(Box::new(RippleTrimCommand::new(clip, edge, delta)));
                } else {
                    state.execute(Box::new(TrimClipCommand::new(clip, edge, delta)));
                }
            }
            _ => {}
        }
        self.queue_draw();
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
    #[allow(clippy::too_many_arguments)]
    fn draw_waveform(&self, s: &gtk::Snapshot, peaks: &[u8], source_start_us: i64, x: f32, y: f32, width: f32, height: f32, view_w: f32) {
        const PEAKS_PER_SECOND: f64 = 50.0;
        let color = css_color(self, "tempo_clip_text");
        let color = gdk::RGBA::new(color.red(), color.green(), color.blue(), 0.85);
        let us_per_px = 1_000_000.0 / self.pps();
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
        let (w, h) = (self.width() as f32, self.height() as f32);
        let state = self.state();
        let imp = self.imp();
        let color = |name: &str| css_color(self, name);
        let rect = |x: f32, y: f32, w: f32, h: f32| graphene::Rect::new(x, y, w, h);

        s.append_color(&color("tempo_bg_surface"), &rect(0.0, 0.0, w, h));

        let project = state.project.borrow();
        let Some(project) = project.as_ref() else { return };
        let timeline = &project.timeline;
        let fps = project.fps.to_f64();
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
        let first = ((imp.scroll.get() / pps) / step).floor() * step;
        let mut t = first;
        while (t * pps - imp.scroll.get()) < w as f64 {
            let x = (t * pps - imp.scroll.get()) as f32;
            s.append_color(&soft, &rect(x, RULER_H - 20.0, 1.0, 20.0));
            for i in 1..5 {
                let mx = x + (step * pps) as f32 * i as f32 / 5.0;
                s.append_color(&soft, &rect(mx, RULER_H - 7.0, 1.0, 7.0));
            }
            self.text(s, &timecode((t * 1_000_000.0) as i64, fps), x + 6.0, 24.0, &dim, false, 110.0);
            t += step;
        }
        s.append_color(&line, &rect(0.0, RULER_H - 1.0, w, 1.0));

        // In–Out band.
        if let (Some(a), Some(b)) = (state.mark_in.get(), state.mark_out.get()) {
            let (xa, xb) = (self.x_of(a.min(b)) as f32, self.x_of(a.max(b)) as f32);
            s.append_color(&gdk::RGBA::new(1.0, 1.0, 1.0, 0.08), &rect(xa, 0.0, xb - xa, h));
        }

        // Tracks and clips.
        let dragging = imp.drag.borrow().clone();
        let rows = rows(timeline);
        let selected = state.selection.get();
        for row in &rows {
            s.append_color(&line, &rect(0.0, row.y + row.h - 1.0, w, 1.0));
            let Some(track) = timeline.find_track(row.track_id) else { continue };
            for clip in &track.clips {
                // A clip being moved is drawn at its new place below.
                let (mut tin, mut tout, mut y) = (clip.timeline_in, clip.timeline_out, row.y);
                match &dragging {
                    Some(Drag::Move { clip: id, new_in, len, to_track, .. }) if *id == clip.id => {
                        tin = *new_in;
                        tout = *new_in + *len;
                        if let Some(r) = rows.iter().find(|r| r.track_id == *to_track) {
                            y = r.y;
                        }
                    }
                    Some(Drag::Trim { clip: id, edge, delta }) if *id == clip.id => match edge {
                        TrimEdge::In => tin = (tin + delta).min(tout - 1),
                        TrimEdge::Out => tout = (tout + delta).max(tin + 1),
                    },
                    _ => {}
                }
                let (x0, x1) = (self.x_of(tin) as f32, self.x_of(tout) as f32);
                if x1 < 0.0 || x0 > w {
                    continue;
                }
                let body = rect(x0, y + 1.0, (x1 - x0).max(2.0), row.h - 3.0);
                let (top, bar) = match clip.clip_type {
                    ClipType::Audio => ("tempo_clip_audio_wave", "tempo_clip_audio"),
                    ClipType::Title => ("tempo_clip_title", "tempo_clip_title"),
                    _ => ("tempo_clip_video_top", "tempo_clip_video"),
                };
                let rounded = gsk::RoundedRect::from_rect(body, 3.0);
                s.push_rounded_clip(&rounded);
                s.append_color(&color(top), &body);
                let bar_y = body.y() + body.height() - NAME_BAR_H;
                if clip.clip_type == ClipType::Audio {
                    if let Some(peaks) = state.waveforms.borrow().get(&clip.source_id) {
                        self.draw_waveform(s, peaks, clip.source_in + (tin - clip.timeline_in), body.x(), body.y(), body.width(), bar_y - body.y(), w);
                    }
                }
                s.append_color(&color(bar), &rect(body.x(), bar_y, body.width(), NAME_BAR_H));
                self.text(s, &clip.name, body.x() + 6.0, bar_y + 3.0, &white, true, body.width() - 10.0);
                if !row.enabled || !clip.properties.enabled {
                    s.append_color(&gdk::RGBA::new(0.16, 0.16, 0.18, 0.6), &body);
                }
                s.pop();
                if selected == Some(clip.id) {
                    s.append_border(&rounded, &[2.0; 4], &[accent; 4]);
                } else {
                    s.append_border(&rounded, &[1.0; 4], &[gdk::RGBA::new(0.0, 0.0, 0.0, 0.3); 4]);
                }
            }
            if row.locked {
                s.append_color(&gdk::RGBA::new(1.0, 1.0, 1.0, 0.06), &rect(0.0, row.y, w, row.h));
            }
        }
        // Divider between video and audio tracks.
        if let Some(first_audio) = rows.iter().find(|r| r.kind == TrackKind::Audio) {
            s.append_color(&soft, &rect(0.0, first_audio.y - DIVIDER_H, w, DIVIDER_H));
        }

        // Markers.
        for m in &timeline.markers {
            let x = self.x_of(m.position_us) as f32;
            if x < -10.0 || x > w + 10.0 {
                continue;
            }
            let c = color(match m.color {
                MarkerColor::Blue => "tempo_marker_blue",
                MarkerColor::Green => "tempo_marker_green",
                MarkerColor::Yellow => "tempo_marker_yellow",
                MarkerColor::Red => "tempo_marker_red",
                MarkerColor::Orange => "tempo_marker_orange",
                MarkerColor::Purple => "tempo_marker_purple",
            });
            // Markers live in the ruler only, so they are not mistaken for the playhead.
            s.append_color(&c, &rect(x - 4.0, 0.0, 9.0, 12.0));
            s.append_color(&c, &rect(x, 12.0, 1.0, RULER_H - 12.0));
            if !m.name.is_empty() {
                self.text(s, &m.name, x + 8.0, -2.0, &c, false, 140.0);
            }
        }

        // Snap guide and playhead.
        if let Some(us) = imp.snap_line.get() {
            s.append_color(&white, &rect(self.x_of(us) as f32, RULER_H, 1.0, h - RULER_H));
        }
        let px = self.x_of(imp.playhead.get()) as f32;
        if px >= -8.0 && px <= w + 8.0 {
            s.append_color(&accent, &rect(px, 0.0, 1.0, h));
            s.append_color(&accent, &rect(px - 6.0, RULER_H - 16.0, 13.0, 10.0));
            s.append_color(&accent, &rect(px - 3.0, RULER_H - 6.0, 7.0, 3.0));
        }
    }
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
        let timecode_box = gtk::Box::builder().css_classes(["timecode-box"]).halign(gtk::Align::Fill).build();
        timecode_label.set_halign(gtk::Align::Center);
        timecode_label.set_width_chars(11);
        timecode_box.append(&timecode_label);
        timecode_box.set_height_request(RULER_H as i32);

        let header_rows = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let headers = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .width_request(150)
            .hexpand(false)
            .css_classes(["track-headers"])
            .build();
        headers.append(&timecode_box);
        headers.append(&header_rows);

        let s = state.clone();
        viewer.connect_position(move |pos| timecode_label.set_text(&timecode(pos, s.fps())));

        let right = gtk::Box::new(gtk::Orientation::Vertical, 0);
        right.append(&canvas);
        right.append(&gtk::Scrollbar::new(gtk::Orientation::Horizontal, Some(&canvas.hadjustment())));

        let root = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).vexpand(true).build();
        root.append(&headers);
        root.append(&right);
        root.append(&audio_meter(state));

        let s = state.clone();
        let rebuild = move || build_headers(&s, &header_rows);
        rebuild();
        state.connect(move |change| {
            if matches!(change, Change::Project | Change::Timeline | Change::Options) {
                rebuild();
            }
        });

        Self { root, canvas }
    }
}

fn build_headers(state: &Rc<AppState>, container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
    let rows = state.with_timeline(rows).unwrap_or_default();
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
        header.append(&gtk::Box::builder().hexpand(true).build());

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

        container.append(&header);
    }
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
