//! Application state shared by every widget on the GTK thread.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use tempo_timeline::{Clip, Command, CommandLog, Project, Timeline, TrackKind, Transition, TransitionEdge};
use uuid::Uuid;

use crate::player::{Player, Snapshot};

/// What changed, so listeners redraw only what they must.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    /// A project was opened or closed.
    Project,
    /// Clips, tracks or markers changed.
    Timeline,
    /// The Media Pool changed.
    Media,
    /// Timelines were renamed, duplicated or removed (Media Pool, viewer menu).
    Timelines,
    /// A different clip (or nothing) is selected.
    Selection,
    /// Tool, snapping, destination tracks, In/Out marks.
    Options,
    /// Saved / unsaved state flipped.
    Dirty,
    /// Waveform data for a source became available.
    Waveform,
    /// The set of enabled plugins changed.
    Plugins,
    /// Something the user should be told.
    Message(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tool {
    #[default]
    Select,
    Trim,
    Blade,
}

/// An empty stretch of a track between two clips (or before the first one).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gap {
    pub track: Uuid,
    pub start: i64,
    pub end: i64,
}

/// What `AppState::edit_selection` holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditSelection {
    /// The transition stored on `clip_id`'s `edge` (see `Track::transition_at_edit`).
    Transition { track_id: Uuid, clip_id: Uuid, edge: TransitionEdge },
    /// The edit point at `position_us` on a track: a cut, or a clip's head or tail
    /// (see `Track::edit_at`). Up/Down move it along with the playhead.
    Edit { track_id: Uuid, position_us: i64 },
}

impl EditSelection {
    /// Whether the transition or edit point is still on the timeline.
    pub fn exists_in(&self, t: &Timeline) -> bool {
        match *self {
            Self::Transition { track_id, clip_id, edge } => t.find_track(track_id).is_some_and(|tr| tr.transition_at_edit(clip_id, edge).is_some()),
            Self::Edit { track_id, position_us } => t.find_track(track_id).is_some_and(|tr| tr.edit_at(position_us).is_some()),
        }
    }
}

pub struct AppState {
    pub project: RefCell<Option<Project>>,
    pub path: RefCell<Option<PathBuf>>,
    log: RefCell<CommandLog>,
    /// The undo history of each timeline that is not open, so each keeps its own.
    parked_logs: RefCell<std::collections::HashMap<Uuid, CommandLog>>,
    /// The clip the Inspector shows: the one clicked last.
    pub selection: Cell<Option<Uuid>>,
    /// Other clips selected with it (Ctrl+click, linked partners, Select All).
    pub extra_selection: RefCell<Vec<Uuid>>,
    pub tool: Cell<Tool>,
    pub snapping: Cell<bool>,
    /// When on, selecting a clip also selects the clips linked to it.
    pub linked_selection: Cell<bool>,
    pub dirty: Cell<bool>,
    /// Set by every change; cleared when a rolling backup is taken.
    pub changed_since_backup: Cell<bool>,
    /// Whether hardware decoding and hardware encoding work here, found at start-up.
    pub hardware: Cell<(bool, bool)>,
    pub mark_in: Cell<Option<i64>>,
    pub mark_out: Cell<Option<i64>>,
    /// `kind_index` of the video / audio track that receives the next edit.
    pub dest_video: Cell<u32>,
    pub dest_audio: Cell<u32>,
    /// Media Pool item shown in the viewer's source mode, if any.
    pub source_clip: Cell<Option<Uuid>>,
    /// In / Out set on the source clip while it is open in the viewer.
    pub src_in: Cell<Option<i64>>,
    pub src_out: Cell<Option<i64>>,
    /// Item selected in the Media Pool.
    pub media_selection: Cell<Option<Uuid>>,
    /// The clip last copied or cut.
    /// The clips last copied or cut.
    pub clipboard: RefCell<Vec<Clip>>,
    /// The selected gap, if a gap rather than a clip is selected.
    pub gap: Cell<Option<Gap>>,
    /// A transition (Resolve's red outline) or an edit point (green bracket) selected
    /// instead of clips. Selecting clips or a gap clears it, and selecting it clears them.
    pub edit_selection: Cell<Option<EditSelection>>,
    /// The transition last copied (Ctrl+C with a transition selected); Ctrl+V puts it on
    /// the selected edit point. Kept apart from the clip clipboard, as in Resolve.
    pub transition_clipboard: RefCell<Option<Transition>>,
    pub settings: RefCell<crate::settings::Settings>,
    /// True while the source clip has its own viewer beside the timeline viewer.
    pub dual_viewer: Cell<bool>,
    /// The dual Source Viewer's own player, once that viewer has been built.
    pub source_player: RefCell<Option<Player>>,
    /// When on, playback loops over In..Out (if both set) or the whole timeline/source.
    pub loop_playback: Cell<bool>,
    /// When on, scrubbing or stepping plays a short snippet of audio at the playhead.
    pub scrub_audio: Cell<bool>,
    /// Viewer picture zoom: `None` fits the frame to the viewer, `Some(s)` scales by `s` (1.0 = 100%).
    pub viewer_zoom: Cell<Option<f64>>,
    /// The user's First-Edit Checklist steps 3 & 5, kept in marker files (`help::tick_step`).
    pub checklist_split: Cell<bool>,
    pub checklist_queued: Cell<bool>,
    /// Filters offered by the enabled plugins, kept current by `Plugins`.
    pub filters: RefCell<Vec<tempo_timeline::ClipEffect>>,
    /// Upload targets offered by the enabled plugins, kept current by `Plugins`.
    pub uploaders: RefCell<Vec<crate::plugins::Uploader>>,
    /// Opens a Media Pool clip in whichever viewer shows source clips.
    pub open_source: RefCell<Option<Rc<dyn Fn(Option<Uuid>)>>>,
    /// Shows the shortcut hints for where the pointer is (bottom bar).
    pub show_hint: RefCell<Option<Rc<dyn Fn(Option<crate::keybinds::HintZone>)>>>,
    /// Loudness of each source, 50 values a second, for drawing waveforms.
    pub waveforms: RefCell<std::collections::HashMap<Uuid, Rc<Vec<u8>>>>,
    pub player: Player,
    listeners: RefCell<Vec<Rc<dyn Fn(&Change)>>>,
}

impl AppState {
    pub fn new() -> Rc<Self> {
        Rc::new(Self {
            project: RefCell::new(None),
            path: RefCell::new(None),
            log: RefCell::new(CommandLog::new()),
            parked_logs: RefCell::new(Default::default()),
            selection: Cell::new(None),
            extra_selection: RefCell::new(Vec::new()),
            tool: Cell::new(Tool::Select),
            snapping: Cell::new(true),
            linked_selection: Cell::new(true),
            dirty: Cell::new(false),
            changed_since_backup: Cell::new(false),
            hardware: Cell::new((false, false)),
            mark_in: Cell::new(None),
            mark_out: Cell::new(None),
            dest_video: Cell::new(1),
            dest_audio: Cell::new(1),
            source_clip: Cell::new(None),
            src_in: Cell::new(None),
            src_out: Cell::new(None),
            media_selection: Cell::new(None),
            clipboard: RefCell::new(Vec::new()),
            gap: Cell::new(None),
            edit_selection: Cell::new(None),
            transition_clipboard: RefCell::new(None),
            settings: RefCell::new(Default::default()),
            dual_viewer: Cell::new(false),
            source_player: RefCell::new(None),
            loop_playback: Cell::new(false),
            scrub_audio: Cell::new(false),
            viewer_zoom: Cell::new(None),
            checklist_split: Cell::new(false),
            checklist_queued: Cell::new(false),
            filters: RefCell::new(Vec::new()),
            uploaders: RefCell::new(Vec::new()),
            open_source: RefCell::new(None),
            show_hint: RefCell::new(None),
            waveforms: RefCell::new(Default::default()),
            player: Player::new(),
            listeners: RefCell::new(Vec::new()),
        })
    }

    pub fn connect(&self, f: impl Fn(&Change) + 'static) {
        self.listeners.borrow_mut().push(Rc::new(f));
    }

    pub fn emit(&self, change: Change) {
        if matches!(change, Change::Options) {
            self.sync_loop_range();
        }
        // Clone the list first: a listener may register another listener.
        let listeners: Vec<_> = self.listeners.borrow().clone();
        for l in listeners {
            l(&change);
        }
    }

    pub fn set_loop_playback(&self, on: bool) {
        if self.loop_playback.replace(on) != on {
            self.player.set_looping(on);
            self.sync_loop_range();
            self.emit(Change::Options);
        }
    }

    pub fn set_viewer_zoom(&self, zoom: Option<f64>) {
        if self.viewer_zoom.replace(zoom) != zoom {
            self.emit(Change::Options);
        }
    }

    pub fn sync_loop_range(&self) {
        let (in_us, out_us) = if self.source_mode() {
            (self.src_in.get(), self.src_out.get())
        } else {
            (self.mark_in.get(), self.mark_out.get())
        };
        self.player.set_loop_range(in_us, out_us);
    }

    pub fn message(&self, text: impl Into<String>) {
        let text = text.into();
        tracing::info!("message: {text}");
        self.emit(Change::Message(text));
    }

    /// Run `f` with the timeline, if a project is open.
    pub fn with_timeline<R>(&self, f: impl FnOnce(&Timeline) -> R) -> Option<R> {
        self.project.borrow().as_ref().map(|p| f(&p.timeline))
    }

    pub fn with_project<R>(&self, f: impl FnOnce(&Project) -> R) -> Option<R> {
        self.project.borrow().as_ref().map(f)
    }

    /// True when In/Out and the transport act on a source clip shown in the single viewer.
    pub fn source_mode(&self) -> bool {
        self.source_clip.get().is_some() && !self.dual_viewer.get()
    }

    /// The player of whichever viewer shows source clips right now.
    pub fn shown_source_player(&self) -> Player {
        self.source_player.borrow().clone().filter(|_| self.dual_viewer.get()).unwrap_or_else(|| self.player.clone())
    }

    pub fn show_source(&self, source: Option<Uuid>) {
        let open = self.open_source.borrow().clone();
        if let Some(open) = open {
            open(source);
        }
    }

    /// The timeline playhead: where edits happen, whatever the viewer shows.
    pub fn playhead_us(&self) -> i64 {
        self.player.timeline_position_us()
    }

    /// The number of the frame a time is in. A time within a tenth of a frame
    /// of the next frame counts as that frame, so positions stored to the
    /// microsecond never read one frame low.
    fn frame_index(&self, us: i64) -> i128 {
        let (num, den) = self.with_project(|p| (p.timeline_fps().num.max(1) as i128, p.timeline_fps().den.max(1) as i128)).unwrap_or((30, 1));
        (us.max(0) as i128 * num * 10 + den * 1_000_000) / (den * 10_000_000)
    }

    fn frame_start(&self, frame: i128) -> i64 {
        let (num, den) = self.with_project(|p| (p.timeline_fps().num.max(1) as i128, p.timeline_fps().den.max(1) as i128)).unwrap_or((30, 1));
        ((frame.max(0) * den * 1_000_000 + num / 2) / num) as i64
    }

    /// The start of the frame a time is in. Positions made with this stay on
    /// true frame times however far along the timeline they are.
    pub fn frame_floor(&self, us: i64) -> i64 {
        self.frame_start(self.frame_index(us))
    }

    /// The start of the frame `frames` frames away from the one `us` is in.
    pub fn frame_step(&self, us: i64, frames: i64) -> i64 {
        self.frame_start(self.frame_index(us) + frames as i128)
    }

    pub fn frame_us(&self) -> i64 {
        self.with_project(|p| p.timeline_fps().frame_duration_us()).unwrap_or(33_333).max(1)
    }

    pub fn fps(&self) -> f64 {
        self.with_project(|p| p.timeline_fps().to_f64()).unwrap_or(30.0)
    }

    /// The timecode a timeline position reads as: moved by the timeline's Start Timecode.
    /// Use `timecode` for lengths, which the Start Timecode does not move.
    pub fn position_tc(&self, us: i64) -> String {
        timecode(us + self.start_tc_us(), self.fps())
    }

    /// The open timeline's Start Timecode, 0 with no project. Subtract it from a typed timecode.
    pub fn start_tc_us(&self) -> i64 {
        self.with_project(|p| p.timeline.start_tc_us).unwrap_or(0)
    }

    /// Length of a new transition: the user's "Set as Default Duration", 1 s until set.
    pub fn standard_transition_us(&self) -> i64 {
        // A hand-edited settings file can hold anything; outside a frame..an hour means unset.
        match self.settings.borrow().transition_us {
            us if (1..=3_600_000_000).contains(&us) => us,
            _ => tempo_timeline::STANDARD_TRANSITION_US,
        }
    }

    pub fn set_project(&self, project: Option<Project>, path: Option<PathBuf>) {
        self.player.pause();
        self.player.set_source(None);
        *self.project.borrow_mut() = project;
        *self.path.borrow_mut() = path;
        self.log.borrow_mut().clear();
        self.parked_logs.borrow_mut().clear();
        self.selection.set(None);
        self.extra_selection.borrow_mut().clear();
        self.gap.set(None);
        self.edit_selection.set(None);
        self.clipboard.borrow_mut().clear();
        self.source_clip.set(None);
        self.waveforms.borrow_mut().clear();
        self.media_selection.set(None);
        self.src_in.set(None);
        self.src_out.set(None);
        self.mark_in.set(None);
        self.mark_out.set(None);
        self.dest_video.set(1);
        self.dest_audio.set(1);
        self.loop_playback.set(false);
        self.scrub_audio.set(false);
        self.viewer_zoom.set(None);
        self.player.set_looping(false);
        self.player.set_scrub_audio(false);
        self.sync_loop_range();
        self.dirty.set(false);
        self.sync_player();
        self.player.seek(0, true);
        self.emit(Change::Project);
        // Loop, scrub and zoom were reset above: let the viewers show and apply that.
        self.emit(Change::Options);
    }

    /// Opens timeline `id`, as Resolve's timeline menu above the viewer does.
    pub fn open_timeline(&self, id: Uuid) {
        let from = self.with_timeline(|t| t.id);
        if from != Some(id) && self.project.borrow_mut().as_mut().is_some_and(|p| p.open_timeline(id)) {
            self.switched_timeline(from);
            // The file records which timeline is open, as Resolve reopens the last one.
            self.set_dirty(true);
        }
    }

    /// Adds `timeline` to the project and opens it (Create New Timeline).
    pub fn add_timeline(&self, timeline: Timeline) {
        let from = self.with_timeline(|t| t.id);
        if self.project.borrow_mut().as_mut().map(|p| p.add_timeline(timeline)).is_none() {
            return;
        }
        self.switched_timeline(from);
        self.set_dirty(true);
        // The Media Pool lists timelines too.
        self.emit(Change::Timelines);
    }

    /// Renames timeline `id`; false when the name is blank or taken.
    pub fn rename_timeline(&self, id: Uuid, name: &str) -> bool {
        let done = self.project.borrow_mut().as_mut().is_some_and(|p| p.rename_timeline(id, name));
        if done {
            self.set_dirty(true);
            self.emit(Change::Timelines);
        }
        done
    }

    /// Copies timeline `id` as "<name> Copy" without opening it.
    pub fn duplicate_timeline(&self, id: Uuid) -> Option<Uuid> {
        let copy = self.project.borrow_mut().as_mut().and_then(|p| p.duplicate_timeline(id));
        if copy.is_some() {
            self.set_dirty(true);
            self.emit(Change::Timelines);
        }
        copy
    }

    /// Deletes timeline `id` (never the open one) and its parked undo history.
    pub fn remove_timeline(&self, id: Uuid) -> bool {
        let done = self.project.borrow_mut().as_mut().is_some_and(|p| p.remove_timeline(id));
        if done {
            self.parked_logs.borrow_mut().remove(&id);
            self.set_dirty(true);
            self.emit(Change::Timelines);
        }
        done
    }

    /// After another timeline was opened: swap undo histories and start it from the top.
    fn switched_timeline(&self, from: Option<Uuid>) {
        self.player.pause();
        let to = self.with_timeline(|t| t.id);
        let log = to.and_then(|id| self.parked_logs.borrow_mut().remove(&id)).unwrap_or_default();
        let old = self.log.replace(log);
        if let Some(from) = from {
            self.parked_logs.borrow_mut().insert(from, old);
        }
        self.selection.set(None);
        self.extra_selection.borrow_mut().clear();
        self.gap.set(None);
        self.edit_selection.set(None);
        self.mark_in.set(None);
        self.mark_out.set(None);
        self.dest_video.set(1);
        self.dest_audio.set(1);
        self.sync_loop_range();
        self.sync_player();
        self.player.seek(0, true);
        self.emit(Change::Timeline);
        self.emit(Change::Selection);
    }

    /// Hand the playback worker a fresh copy of the timeline.
    pub fn sync_player(&self) {
        if let Some(p) = self.project.borrow().as_ref() {
            // "Full" preview quality reads the original files; the others use proxies.
            let proxies = self.settings.borrow().playback_height != 0;
            self.player.set_timeline(Snapshot::from_project(p, proxies));
        }
    }

    fn after_edit(&self) {
        self.sync_player();
        // Forget selected clips that no longer exist.
        let gone = |id: &Uuid| self.with_timeline(|t| t.find_clip(*id).is_none()).unwrap_or(true);
        let before = self.selected_ids();
        // An edit may have filled or moved the gap, so it is not kept.
        let had_gap = self.gap.take().is_some();
        // Nor a transition or edit point that is gone.
        let edit_gone = self.edit_selection.get().is_some_and(|sel| !self.with_timeline(|t| sel.exists_in(t)).unwrap_or(false));
        if edit_gone {
            self.edit_selection.set(None);
        }
        let had_gap = had_gap || edit_gone;
        self.extra_selection.borrow_mut().retain(|id| !gone(id));
        if self.selection.get().is_some_and(|id| gone(&id)) {
            let next = self.extra_selection.borrow_mut().pop();
            self.selection.set(next);
        }
        if had_gap || before != self.selected_ids() {
            self.emit(Change::Selection);
        }
        self.set_dirty(true);
        self.emit(Change::Timeline);
    }

    /// Show the shortcut hints for a part of the window, or none.
    pub fn hint(&self, zone: Option<crate::keybinds::HintZone>) {
        let show = self.show_hint.borrow().clone();
        if let Some(show) = show {
            show(zone);
        }
    }

    pub fn set_dirty(&self, dirty: bool) {
        if dirty {
            self.changed_since_backup.set(true);
        }
        if self.dirty.replace(dirty) != dirty {
            self.emit(Change::Dirty);
        }
    }

    /// Every timeline change goes through here so it can be undone.
    pub fn execute(&self, command: Box<dyn Command>) -> bool {
        let result = {
            let mut project = self.project.borrow_mut();
            let Some(project) = project.as_mut() else { return false };
            self.log.borrow_mut().execute(command, &mut project.timeline)
        };
        match result {
            Ok(()) => {
                self.after_edit();
                true
            }
            Err(e) => {
                self.message(friendly_error(&e));
                false
            }
        }
    }

    /// Apply a command without recording it, to preview a change while the user
    /// is still dragging. Follow it with `commit_preview`.
    pub fn apply_unlogged(&self, mut command: Box<dyn Command>) {
        let applied = self.project.borrow_mut().as_mut().is_some_and(|p| command.execute(&mut p.timeline).is_ok());
        if applied {
            self.sync_player();
            self.emit(Change::Timeline);
        }
    }

    /// End a previewed drag: put the clip back as it was, then record the final
    /// state as one undo step.
    pub fn commit_preview(&self, name: &str, original: Clip) {
        let id = original.id;
        let Some(now) = self.with_timeline(|t| t.find_clip(id).map(|(_, c)| c.clone())).flatten() else { return };
        if now == original {
            return;
        }
        if let Some(clip) = self.project.borrow_mut().as_mut().and_then(|p| p.timeline.find_clip_mut(id)) {
            *clip = original;
        }
        self.execute(Box::new(tempo_timeline::EditClipCommand::new(name, now)));
    }

    pub fn undo(&self) {
        self.history_step(true);
    }

    pub fn redo(&self) {
        self.history_step(false);
    }

    fn history_step(&self, undo: bool) {
        let result = {
            let mut project = self.project.borrow_mut();
            let Some(project) = project.as_mut() else { return };
            let mut log = self.log.borrow_mut();
            if undo {
                log.undo(&mut project.timeline)
            } else {
                log.redo(&mut project.timeline)
            }
        };
        match result {
            Ok(()) => self.after_edit(),
            Err(e) => self.message(friendly_error(&e)),
        }
    }

    /// Every selected clip, the primary one first.
    pub fn selected_ids(&self) -> Vec<Uuid> {
        self.selection.get().into_iter().chain(self.extra_selection.borrow().iter().copied()).collect()
    }

    pub fn is_selected(&self, id: Uuid) -> bool {
        self.selection.get() == Some(id) || self.extra_selection.borrow().contains(&id)
    }

    /// Clips linked to `id` (its sound or its picture), if linked selection is on.
    pub fn linked_to(&self, id: Uuid) -> Vec<Uuid> {
        if !self.linked_selection.get() {
            return Vec::new();
        }
        self.with_timeline(|t| {
            let Some(link) = t.find_clip(id).and_then(|(_, c)| c.properties.link) else { return Vec::new() };
            t.tracks.iter().flat_map(|tr| tr.clips.iter()).filter(|c| c.id != id && c.properties.link == Some(link)).map(|c| c.id).collect()
        })
        .unwrap_or_default()
    }

    /// Select one clip (and what is linked to it), or nothing.
    pub fn select(&self, clip: Option<Uuid>) {
        let before = self.selected_ids();
        let had_gap = self.gap.take().is_some() | self.edit_selection.take().is_some();
        self.selection.set(clip);
        *self.extra_selection.borrow_mut() = clip.map(|id| self.linked_to(id)).unwrap_or_default();
        if had_gap || before != self.selected_ids() {
            self.emit(Change::Selection);
        }
    }

    /// Select an explicit list of clips (first is primary), clearing any gap or edit selection.
    pub fn select_ids(&self, ids: Vec<Uuid>) {
        let mut unique = Vec::with_capacity(ids.len());
        for id in ids {
            if !unique.contains(&id) {
                unique.push(id);
            }
        }
        let before = self.selected_ids();
        let had_special = self.gap.take().is_some() | self.edit_selection.take().is_some();
        self.selection.set(if unique.is_empty() { None } else { Some(unique.remove(0)) });
        *self.extra_selection.borrow_mut() = unique;
        if had_special || before != self.selected_ids() {
            self.emit(Change::Selection);
        }
    }

    /// Select an empty stretch of a track instead of a clip.
    pub fn select_gap(&self, gap: Gap) {
        self.selection.set(None);
        self.extra_selection.borrow_mut().clear();
        self.edit_selection.set(None);
        self.gap.set(Some(gap));
        self.emit(Change::Selection);
    }

    /// Select a transition or an edit point (or nothing) in place of clips and gaps.
    pub fn select_edit(&self, edit: Option<EditSelection>) {
        self.selection.set(None);
        self.extra_selection.borrow_mut().clear();
        self.gap.set(None);
        self.edit_selection.set(edit);
        self.emit(Change::Selection);
    }

    /// Change the preview quality: the height the picture is decoded at, 0 for
    /// the original files at full size. Applied at once and remembered.
    pub fn set_playback_height(&self, height: u32) {
        if self.settings.borrow().playback_height == height {
            return;
        }
        self.settings.borrow_mut().playback_height = height;
        self.player.set_max_height(height);
        self.sync_player();
        self.emit(Change::Options);
        let snapshot = self.settings.borrow().clone();
        glib::MainContext::default().spawn_local(async move {
            if let Ok(Err(e)) = gtk4::gio::spawn_blocking(move || snapshot.save()).await {
                tracing::warn!("could not save settings: {e}");
            }
        });
    }

    /// Add a clip to the selection, or take it out (Ctrl+click).
    pub fn toggle_select(&self, id: Uuid) {
        self.gap.set(None);
        let mut group = vec![id];
        group.extend(self.linked_to(id));
        if self.is_selected(id) {
            let mut rest: Vec<Uuid> = self.selected_ids().into_iter().filter(|s| !group.contains(s)).collect();
            self.selection.set(if rest.is_empty() { None } else { Some(rest.remove(0)) });
            *self.extra_selection.borrow_mut() = rest;
        } else {
            let mut all = self.selected_ids();
            all.retain(|s| !group.contains(s));
            self.selection.set(Some(id));
            all.extend(group.into_iter().skip(1));
            *self.extra_selection.borrow_mut() = all;
        }
        self.emit(Change::Selection);
    }

    pub fn select_all(&self) {
        let mut all: Vec<Uuid> = self
            .with_timeline(|t| t.tracks.iter().filter(|tr| !tr.locked).flat_map(|tr| tr.clips.iter().map(|c| c.id)).collect())
            .unwrap_or_default();
        self.gap.set(None);
        self.selection.set(if all.is_empty() { None } else { Some(all.remove(0)) });
        *self.extra_selection.borrow_mut() = all;
        self.emit(Change::Selection);
    }

    pub fn selected_clip(&self) -> Option<Clip> {
        let id = self.selection.get()?;
        self.with_timeline(|t| t.find_clip(id).map(|(_, c)| c.clone())).flatten()
    }

    /// Id of the destination track of the given kind.
    pub fn dest_track(&self, kind: TrackKind) -> Option<Uuid> {
        let index = match kind {
            TrackKind::Video => self.dest_video.get(),
            TrackKind::Audio => self.dest_audio.get(),
        };
        self.with_timeline(|t| {
            t.tracks.iter().find(|tr| tr.kind == kind && tr.kind_index == index).map(|tr| tr.id)
        })
        .flatten()
    }
}

fn friendly_error(e: &tempo_timeline::TimelineError) -> String {
    use tempo_timeline::TimelineError as E;
    match e {
        E::TrackLocked(_) => "That track is locked.".into(),
        E::ClipCollision(..) => "There is already a clip there.".into(),
        E::InvalidTimeRange(..) => "The clip cannot be trimmed that far.".into(),
        E::InvalidSplitPosition(..) => "The clip cannot be cut there.".into(),
        E::NothingToUndo => "Nothing to undo.".into(),
        E::NothingToRedo => "Nothing to redo.".into(),
        other => other.to_string(),
    }
}

/// `HH:MM:SS:FF` for a position, using the project frame rate.
pub fn timecode(us: i64, fps: f64) -> String {
    let fps_i = fps.round().max(1.0) as i64;
    // A position on a frame is stored to the microsecond, a hair before the
    // exact frame time; the allowance keeps it from reading one frame low.
    let total_frames = (us.max(0) as f64 * fps / 1_000_000.0 + 0.1).floor() as i64;
    let frames = total_frames % fps_i;
    let secs = total_frames / fps_i;
    format!("{:02}:{:02}:{:02}:{:02}", secs / 3600, (secs / 60) % 60, secs % 60, frames)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_helpers_follow_the_timelines_own_rate() {
        let state = AppState::new();
        let mut project = Project::new("t", 1920, 1080, tempo_timeline::RationalFps::FPS_30);
        project.timeline.fps = Some(tempo_timeline::RationalFps::FPS_25);
        *state.project.borrow_mut() = Some(project);
        assert_eq!(state.frame_us(), 40_000);
        assert_eq!(state.fps(), 25.0);
        assert_eq!(state.frame_step(0, 25), 1_000_000);
        assert_eq!(state.frame_floor(1_030_000), 1_000_000);
    }

    #[test]
    fn shown_source_player_is_the_source_viewers_only_in_dual_mode() {
        let state = AppState::new();
        let source = Player::new();
        *state.source_player.borrow_mut() = Some(source.clone());
        state.shown_source_player().seek(1_000_000, false);
        assert_eq!(state.player.position_us(), 1_000_000);
        assert_eq!(source.position_us(), 0);

        state.dual_viewer.set(true);
        state.shown_source_player().seek(2_000_000, false);
        assert_eq!(source.position_us(), 2_000_000);
        assert_eq!(state.player.position_us(), 1_000_000);
    }

    #[test]
    fn opening_a_project_announces_the_reset_viewer_options() {
        let state = AppState::new();
        state.loop_playback.set(true);
        state.viewer_zoom.set(Some(1.0));
        let seen = Rc::new(std::cell::RefCell::new(Vec::new()));
        let s = state.clone();
        let log = seen.clone();
        state.connect(move |change| {
            if matches!(change, Change::Options) {
                log.borrow_mut().push((s.loop_playback.get(), s.viewer_zoom.get()));
            }
        });
        state.set_project(None, None);
        assert_eq!(*seen.borrow(), vec![(false, None)]);
    }

    #[test]
    fn timecode_uses_project_rate() {
        assert_eq!(timecode(0, 30.0), "00:00:00:00");
        assert_eq!(timecode(1_000_000, 24.0), "00:00:01:00");
        assert_eq!(timecode(1_500_000, 24.0), "00:00:01:12");
        assert_eq!(timecode(3_661_000_000, 30.0), "01:01:01:00");
        // One frame at 30 fps is stored as 33 333 µs and must read as frame 1.
        assert_eq!(timecode(33_333, 30.0), "00:00:00:01");
        assert_eq!(timecode(966_667, 30.0), "00:00:00:29");
        assert_eq!(timecode(41_708, 23.976), "00:00:00:01");
    }
}
