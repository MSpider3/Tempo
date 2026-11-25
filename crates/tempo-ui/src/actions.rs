//! Editing actions. Each builds commands and sends them through `AppState::execute`,
//! so every one of them can be undone.

use std::collections::HashMap;
use std::rc::Rc;

use tempo_timeline::{
    frame_to_us, us_to_frame, AddMarkerCommand, Clip, ClipType, Command, CompositeCommand, DeleteClipCommand, DeleteMarkerCommand,
    ClipEffect, EditClipCommand, EditKind, EditTarget, InsertClipCommand, Marker, MarkerColor, MediaSource, MediaType, MoveClipCommand,
    OverwriteClipCommand, PropertyChange, RationalFps, ReplaceClipCommand, SetClipPropertyCommand, SetTrackFlagCommand, SetTransitionCommand,
    SplitClipCommand, TitleData, TitleType, Timeline, Track, TrackFlag, TrackKind, Transition, TransitionAlignment, TransitionEdge, TransitionKind,
    TrimClipCommand, TrimEdge, RippleTrimCommand,
};
use uuid::Uuid;

use crate::player::Player;
use crate::state::{AppState, Change, EditSelection, Gap};

/// Still images get this length when placed on the timeline.
const STILL_DURATION_US: i64 = 5_000_000;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Place {
    Insert,
    Overwrite,
    Append,
    OnTop,
}

/// Build the clip(s) a source produces: picture on a video track, sound on an audio track.
pub(crate) fn clips_for_source(state: &AppState, source_id: Uuid, at_us: i64, video_track: Option<Uuid>, audio_track: Option<Uuid>) -> Vec<Clip> {
    clips_for_source_marked(state, source_id, at_us, video_track, audio_track, true)
}

/// `use_marks` off places the whole source whatever the viewer's marks say.
fn clips_for_source_marked(state: &AppState, source_id: Uuid, at_us: i64, video_track: Option<Uuid>, audio_track: Option<Uuid>, use_marks: bool) -> Vec<Clip> {
    let Some((media_type, duration, has_audio, name, conform_speed, ch_map)) = state
        .with_project(|p| {
            p.sources.get(&source_id).map(|s| {
                (
                    s.media_type,
                    s.duration_us,
                    s.audio_channels.is_some(),
                    s.display_name(),
                    s.conform_speed(),
                    s.audio_channel_map,
                )
            })
        })
        .flatten()
    else {
        return Vec::new();
    };

    // Honour In/Out marks set on this clip in the viewer's source mode.
    let marked = use_marks && state.source_clip.get() == Some(source_id);
    let full = if media_type == MediaType::Image { STILL_DURATION_US } else { duration.max(1) };
    // The marks are in viewer time, which runs at the conformed rate; the clip wants file time.
    let to_src = |v: i64| tempo_timeline::scale_us(v, conform_speed);
    let src_in = if marked { to_src(state.src_in.get().unwrap_or(0)).clamp(0, full - 1) } else { 0 };
    let src_out = if marked { state.src_out.get().map_or(full, to_src).clamp(src_in + 1, full) } else { full };
    let src_len = src_out - src_in;
    let timeline_len = if (conform_speed - 1.0).abs() < 1e-9 {
        src_len
    } else {
        tempo_timeline::scale_us(src_len, 1.0 / conform_speed).max(1)
    };

    let mut clips = Vec::new();
    let mut push = |track: Option<Uuid>, kind: ClipType| {
        if let Some(track) = track {
            let mut clip = Clip::new(track, source_id, kind, name.clone(), at_us, at_us + timeline_len, src_in, src_out);
            if matches!(kind, ClipType::Video | ClipType::Audio) {
                clip.properties.speed = conform_speed;
            }
            if kind == ClipType::Audio {
                clip.properties.audio_channel_map = ch_map;
            }
            clips.push(clip);
        }
    };
    match media_type {
        MediaType::Video => {
            push(video_track, ClipType::Video);
            if has_audio {
                push(audio_track, ClipType::Audio);
            }
        }
        MediaType::Image => push(video_track, ClipType::Image),
        MediaType::Audio => push(audio_track, ClipType::Audio),
    }
    // The picture and its sound stay together until the user unlinks them.
    if clips.len() > 1 {
        let link = Some(Uuid::new_v4());
        for c in &mut clips {
            c.properties.link = link;
        }
    }
    clips
}

/// Place a Media Pool source on the timeline. `track` overrides the destination
/// track of its kind (used when a clip is dropped on a particular row), and
/// `at_us` the place (a drop); without it the clip goes to the playhead. `use_marks`
/// off ignores the viewer's In/Out marks and places the whole clip.
pub fn place_source(state: &Rc<AppState>, source_id: Uuid, how: Place, at_us: Option<i64>, track: Option<Uuid>, use_marks: bool) {
    let dropped = at_us.is_some();
    let mut video = state.dest_track(TrackKind::Video);
    let mut audio = state.dest_track(TrackKind::Audio);
    let dropped_kind = track.and_then(|t| state.with_timeline(|tl| tl.find_track(t).map(|tr| tr.kind)).flatten());
    match dropped_kind {
        Some(TrackKind::Video) => video = track,
        Some(TrackKind::Audio) => audio = track,
        None => {}
    }

    let mut at = state.frame_floor(at_us.unwrap_or_else(|| state.playhead_us()));
    if how == Place::Append {
        at = state
            .with_timeline(|t| {
                t.tracks.iter().filter(|tr| Some(tr.id) == video || Some(tr.id) == audio).map(|tr| tr.duration_us()).max()
            })
            .flatten()
            .unwrap_or(0);
    }

    let mut clips = clips_for_source_marked(state, source_id, at, video, audio, use_marks);
    if clips.is_empty() {
        state.message("Nothing to place: pick a clip in the Media Pool first.");
        return;
    }

    if how == Place::OnTop {
        // Lowest video track that is free over the clip's range.
        let (start, end) = (clips[0].timeline_in, clips[0].timeline_out);
        let free = state
            .with_timeline(|t| {
                let mut tracks: Vec<_> = t.tracks.iter().filter(|tr| tr.kind == TrackKind::Video && !tr.locked).collect();
                tracks.sort_by_key(|tr| tr.kind_index);
                tracks.into_iter().find(|tr| !tr.has_collision(start, end, None)).map(|tr| tr.id)
            })
            .flatten();
        let Some(free) = free else {
            state.message("No free video track at the playhead.");
            return;
        };
        clips.retain(|c| c.clip_type != ClipType::Audio);
        for c in &mut clips {
            c.track_id = free;
        }
    }

    // A clip dropped on one row replaces only what is on that row. Its other
    // half (the sound of a picture, or the picture of a sound) goes to a track
    // that is free there, so it never wipes out something the user did not aim at.
    if let (true, Some(kind)) = (dropped && how == Place::Overwrite && clips.len() > 1, dropped_kind) {
        let other = clips.iter().position(|c| (c.clip_type == ClipType::Audio) != (kind == TrackKind::Audio));
        if let Some(i) = other {
            let (start, end, wanted) = (clips[i].timeline_in, clips[i].timeline_out, clips[i].track_id);
            let other_kind = if kind == TrackKind::Audio { TrackKind::Video } else { TrackKind::Audio };
            let free = state
                .with_timeline(|t| {
                    let mut tracks: Vec<_> = t.tracks.iter().filter(|tr| tr.kind == other_kind && !tr.locked).collect();
                    // The usual destination first, then the lowest track.
                    tracks.sort_by_key(|tr| (tr.id != wanted, tr.kind_index));
                    tracks.into_iter().find(|tr| !tr.has_collision(start, end, None)).map(|tr| tr.id)
                })
                .flatten();
            match free {
                Some(free) => clips[i].track_id = free,
                None => {
                    clips.remove(i);
                    for c in &mut clips {
                        c.properties.link = None;
                    }
                    state.message(if other_kind == TrackKind::Audio {
                        "No audio track is free there, so only the picture was placed."
                    } else {
                        "No video track is free there, so only the sound was placed."
                    });
                }
            }
        }
    }

    let first = clips[0].id;
    let end = clips[0].timeline_out;
    let len = end - clips[0].timeline_in;
    let mut commands: Vec<Box<dyn Command>> = Vec::new();
    if how == Place::Insert {
        // Make room on every unlocked track, not only the ones the clip lands
        // on, so that nothing later in the timeline drifts apart: cut whatever
        // lies across the playhead, then move everything after it to the right.
        let (across, mut later) = state
            .with_timeline(|t| {
                let unlocked = || t.tracks.iter().filter(|tr| !tr.locked);
                let across: Vec<Uuid> =
                    unlocked().flat_map(|tr| tr.clips.iter()).filter(|c| c.timeline_in < at && c.timeline_out > at).map(|c| c.id).collect();
                let later: Vec<(Uuid, Uuid, i64)> =
                    unlocked().flat_map(|tr| tr.clips.iter().filter(|c| c.timeline_in >= at).map(|c| (tr.id, c.id, c.timeline_in))).collect();
                (across, later)
            })
            .unwrap_or_default();
        for (track, split) in split_plan(state, &across, at) {
            later.push((track, split.second_clip_id(), at));
            commands.push(Box::new(split));
        }
        // Latest first, so no clip runs into the one after it.
        later.sort_by_key(|m| std::cmp::Reverse(m.2));
        for (track, id, from) in later {
            commands.push(Box::new(MoveClipCommand::new(id, track, track, from, from + len)));
        }
    }
    for c in clips {
        commands.push(match how {
            Place::Overwrite => Box::new(OverwriteClipCommand::new(c)),
            Place::Insert | Place::Append | Place::OnTop => Box::new(InsertClipCommand::new(c)),
        });
    }
    let name = match how {
        Place::Insert => "Insert",
        Place::Overwrite => "Overwrite",
        Place::Append => "Append",
        Place::OnTop => "Place on Top",
    };
    if state.execute(Box::new(CompositeCommand::new(name, commands))) {
        state.select(Some(first));
        // After a key edit the playhead goes to the end of the new clip, ready
        // for the next one. A dropped clip leaves the playhead where it was.
        if how != Place::OnTop && !dropped {
            state.player.seek_timeline(end);
        }
    }
}

/// The source the edit keys act on: the clip open in the viewer, else the Media Pool selection.
pub fn current_source(state: &AppState) -> Option<Uuid> {
    state.source_clip.get().or(state.media_selection.get())
}

pub fn place_current(state: &Rc<AppState>, how: Place) {
    match current_source(state) {
        Some(id) => place_source(state, id, how, None, None, true),
        None => state.message("Pick a clip in the Media Pool first."),
    }
}

fn selected(state: &AppState) -> Option<(Uuid, Clip)> {
    let id = state.selection.get()?;
    state.with_timeline(|t| t.find_clip(id).map(|(tr, c)| (tr.id, c.clone()))).flatten()
}

/// Every selected clip with its track, latest first. Working from the end of
/// the timeline keeps earlier positions valid while clips are removed or moved.
///
/// Clips on locked tracks are left out: a lock means "do not change this",
/// and one locked clip should not stop the rest of an edit.
fn all_selected(state: &AppState) -> Vec<(Uuid, Clip)> {
    let ids = state.selected_ids();
    let mut clips: Vec<(Uuid, Clip)> = state
        .with_timeline(|t| ids.iter().filter_map(|id| t.find_clip(*id)).filter(|(tr, _)| !tr.locked).map(|(tr, c)| (tr.id, c.clone())).collect())
        .unwrap_or_default();
    if clips.is_empty() && !ids.is_empty() {
        state.message("That track is locked. Click its lock in the track header to edit it.");
    }
    clips.sort_by_key(|(_, c)| std::cmp::Reverse(c.timeline_in));
    clips
}

/// Run one command per selected clip as a single undo step.
fn for_selected(state: &Rc<AppState>, name: &str, make: impl Fn(Uuid, &Clip) -> Option<Box<dyn Command>>) -> bool {
    let commands: Vec<Box<dyn Command>> = all_selected(state).iter().filter_map(|(track, clip)| make(*track, clip)).collect();
    !commands.is_empty() && state.execute(Box::new(CompositeCommand::new(name, commands)))
}

pub fn delete_selected(state: &Rc<AppState>, ripple: bool) {
    // A selected gap is closed, whichever delete key was pressed.
    if let Some(gap) = state.gap.get() {
        close_gap(state, gap);
        return;
    }
    if !ripple {
        for_selected(state, "Delete", |track, clip| Some(Box::new(DeleteClipCommand::new(track, clip.id)) as Box<dyn Command>));
        return;
    }

    // Delete, then close each hole across the whole timeline, so that later
    // clips on other tracks keep their place relative to the ones that move.
    let selected = all_selected(state);
    let Some(mut after) = state.with_timeline(|t| t.clone()) else { return };
    if selected.is_empty() {
        return;
    }
    let mut commands: Vec<Box<dyn Command>> = Vec::new();
    for (track, clip) in &selected {
        if DeleteClipCommand::new(*track, clip.id).execute(&mut after).is_ok() {
            commands.push(Box::new(DeleteClipCommand::new(*track, clip.id)));
        }
    }
    // One hole per distinct stretch (a picture and its sound share one), latest first.
    let mut holes: Vec<(i64, i64)> = selected.iter().map(|(_, c)| (c.timeline_in, c.timeline_out)).collect();
    holes.sort_unstable_by_key(|h| std::cmp::Reverse(*h));
    holes.dedup();
    let mut kept_gap = false;
    for (start, end) in holes {
        let tracks: Vec<Uuid> = selected.iter().filter(|(_, c)| (c.timeline_in, c.timeline_out) == (start, end)).map(|(t, _)| *t).collect();
        // Try the moves on a copy first; a hole that cannot be closed cleanly stays open.
        let mut trial = after.clone();
        let moves = plan_close(&after, &tracks, start, end).ok().filter(|moves| {
            moves.iter().all(|(track, id, from)| MoveClipCommand::new(*id, *track, *track, *from, *from - (end - start)).execute(&mut trial).is_ok())
        });
        match moves {
            Some(moves) => {
                after = trial;
                commands.extend(moves.into_iter().map(|(track, id, from)| Box::new(MoveClipCommand::new(id, track, track, from, from - (end - start))) as Box<dyn Command>));
            }
            None => kept_gap = true,
        }
    }
    if state.execute(Box::new(CompositeCommand::new("Ripple Delete", commands))) && kept_gap {
        state.message("Deleted. The gap was kept, because closing it would move a picture away from its sound on another track.");
    }
}

/// Turn the selected clips off or on (Resolve's `D`).
pub fn toggle_enabled(state: &Rc<AppState>) {
    let Some((_, first)) = selected(state) else { return };
    let enable = !first.properties.enabled;
    for_selected(state, "Enable Clip", |_, clip| {
        Some(Box::new(SetClipPropertyCommand::new(clip.id, PropertyChange::Enabled(enable))) as Box<dyn Command>)
    });
}

/// Set the fade at the start (`fade_in`) or end of the selected clips, in seconds.
pub fn set_fade(state: &Rc<AppState>, fade_in: bool, seconds: f64) {
    let us = (seconds.max(0.0) * 1_000_000.0) as i64;
    let done = for_selected(state, "Fade", |_, clip| {
        let mut edited = clip.clone();
        // A fade cannot be longer than half the clip.
        let us = us.min(clip.duration_us() / 2);
        if fade_in {
            edited.properties.fade_in_us = us;
        } else {
            edited.properties.fade_out_us = us;
        }
        (edited != *clip).then(|| Box::new(EditClipCommand::new("Fade", edited)) as Box<dyn Command>)
    });
    if !done && state.selection.get().is_none() {
        state.message("Select a clip on the timeline first.");
    }
}

/// Put a filter on the selected picture clips. A clip holds each filter once.
pub fn add_filter(state: &Rc<AppState>, filter: &ClipEffect) {
    let done = for_selected(state, "Add Filter", |_, clip| {
        if matches!(clip.clip_type, ClipType::Audio | ClipType::Title) || clip.properties.effects.iter().any(|e| e.id == filter.id) {
            return None;
        }
        let mut edited = clip.clone();
        edited.properties.effects.push(filter.clone());
        Some(Box::new(EditClipCommand::new("Add Filter", edited)) as Box<dyn Command>)
    });
    if !done {
        state.message("Select a video clip that does not have this filter yet.");
    }
}

// ---- Transitions ---------------------------------------------------------------
//
// Transitions are objects on clip edges (see tempo_timeline::transition). Resolve's
// workflow: select an edit point (green) or a transition (red), Ctrl+T adds one,
// Ctrl+C copies a selected transition, Ctrl+V pastes it on the selected edit, Up/Down
// carry the edit selection from edit to edit.

/// Resolve's standard transition duration (Preferences, Editing): one second.
pub const STANDARD_TRANSITION_S: f64 = 1.0;

/// Resolve's Cross Fade lives on sound; every other kind on pictures.
fn is_audio_kind(kind: &TransitionKind) -> bool {
    matches!(kind, TransitionKind::Crossfade)
}

/// Where the edit a transition belongs to is: its clip's start (In) or end (Out).
pub(crate) fn transition_edit_us(track: &Track, t: &Transition) -> Option<i64> {
    let clip = track.find_clip(t.clip_id)?;
    Some(if t.edge == TransitionEdge::In { clip.timeline_in } else { clip.timeline_out })
}

/// `us` to the nearest whole frame.
fn whole_frames(us: i64, fps: RationalFps) -> i64 {
    frame_to_us(us_to_frame(us.max(0), fps), fps)
}

/// `template` (its kind, colour, ease and wanted length) fitted to an edit point as
/// Resolve places it: inside the clip at a head or a tail; on a cut centred when the
/// handles allow the whole length, else ending or starting on the cut, whichever fits
/// it (or, when none does, leaves the most), shortened to what fits. Whole frames.
pub(crate) fn fit_transition(
    track: &Track,
    edit: &EditTarget,
    template: &Transition,
    sources: &HashMap<Uuid, MediaSource>,
    fps: RationalFps,
) -> Result<Transition, &'static str> {
    use TransitionAlignment::*;
    let alignments: &[TransitionAlignment] = match edit.kind {
        EditKind::Head => &[StartAtCut],
        EditKind::Tail => &[EndAtCut],
        EditKind::Cut => &[Centered, EndAtCut, StartAtCut],
    };
    let frame = frame_to_us(1, fps);
    let wanted = whole_frames(template.duration_us, fps).max(frame);
    let room = |al: &TransitionAlignment| track.max_transition_us(edit.clip_id, edit.edge, *al, sources, fps);
    // `rev` so that on a tie `max_by_key` keeps the earlier, preferred alignment.
    let alignment = *alignments.iter().find(|al| room(al) >= wanted).or_else(|| alignments.iter().rev().max_by_key(|al| room(al))).unwrap_or(&Centered);
    let length = wanted.min(room(&alignment));
    if length < frame {
        return Err("There is not enough footage past the ends of these clips for a transition here.");
    }
    Ok(Transition { id: Uuid::new_v4(), clip_id: edit.clip_id, edge: edit.edge, alignment, duration_us: length, ..template.clone() })
}

/// The edit point of `track` nearest `at_us`: one within `reach_us`, or an edge of the
/// clip under `at_us`. Where a transition dropped there goes.
pub(crate) fn nearest_edit(track: &Track, at_us: i64, reach_us: i64) -> Option<EditTarget> {
    let under = track.clip_at(at_us).map(|c| (c.timeline_in, c.timeline_out));
    track
        .edit_points()
        .into_iter()
        .filter(|e| (e.position_us - at_us).abs() <= reach_us || under.is_some_and(|(a, b)| e.position_us == a || e.position_us == b))
        .min_by_key(|e| (e.position_us - at_us).abs())
}

/// The edit before (or after) `from_us` among `points`; the timeline's start and its
/// end (`end_us`) count as edits, as with Resolve's Up and Down.
pub(crate) fn neighbour_edit(points: impl IntoIterator<Item = i64>, from_us: i64, end_us: i64, forward: bool) -> Option<i64> {
    let all = points.into_iter().chain([0, end_us]);
    if forward {
        all.filter(|p| *p > from_us).min()
    } else {
        all.filter(|p| *p < from_us).max()
    }
}

/// The length of a transition while one of its free edges is dragged `delta_us` along
/// the timeline: dragging away from the middle lengthens it, and a transition with
/// both edges free (centred on a cut) changes on both sides at once. Whole frames,
/// from one frame to `max_us`.
pub(crate) fn dragged_length(from_us: i64, delta_us: i64, left_edge: bool, both_free: bool, max_us: i64, fps: RationalFps) -> i64 {
    let grow = if left_edge { -delta_us } else { delta_us } * if both_free { 2 } else { 1 };
    let frame = frame_to_us(1, fps);
    whole_frames(from_us + grow, fps).clamp(frame, max_us.max(frame))
}

/// The edit at `position_us` on `track_id`, and the same edit on the tracks whose clips
/// there are linked to its clips (picture and sound), when `linked` is on.
pub(crate) fn edit_group(t: &Timeline, track_id: Uuid, position_us: i64, linked: bool) -> Vec<(Uuid, TrackKind)> {
    let Some(track) = t.find_track(track_id) else { return Vec::new() };
    let at_edit = |c: &&Clip| c.timeline_in == position_us || c.timeline_out == position_us;
    let links: Vec<Uuid> = track.clips.iter().filter(at_edit).filter_map(|c| c.properties.link).collect();
    let mut group = vec![(track_id, track.kind)];
    if linked && !links.is_empty() {
        let partners = t.tracks.iter().filter(|o| o.id != track_id && o.clips.iter().filter(at_edit).any(|c| c.properties.link.is_some_and(|l| links.contains(&l))));
        group.extend(partners.map(|o| (o.id, o.kind)));
    }
    group
}

/// The selected transition and its track.
pub fn selected_transition(state: &AppState) -> Option<(Uuid, Transition)> {
    let Some(EditSelection::Transition { track_id, clip_id, edge }) = state.edit_selection.get() else { return None };
    let t = state.with_timeline(|t| t.find_track(track_id)?.transition_at_edit(clip_id, edge).cloned()).flatten()?;
    Some((track_id, t))
}

/// `template` fitted to the edit at `position_us` on a track (see `fit_transition`), or
/// why it cannot go there.
fn placed_transition(state: &AppState, track_id: Uuid, position_us: i64, template: &Transition) -> Result<Transition, &'static str> {
    state.with_project(|p| place_on(&p.timeline, &p.sources, p.timeline_fps(), track_id, position_us, template)).unwrap_or(Err("Open a project first."))
}

fn place_on(
    timeline: &Timeline,
    sources: &HashMap<Uuid, MediaSource>,
    fps: RationalFps,
    track_id: Uuid,
    position_us: i64,
    template: &Transition,
) -> Result<Transition, &'static str> {
    let track = timeline.find_track(track_id).ok_or("That track is no longer there.")?;
    if track.locked {
        return Err("That track is locked. Click its lock in the track header to edit it.");
    }
    if is_audio_kind(&template.kind) != (track.kind == TrackKind::Audio) {
        return Err(if track.kind == TrackKind::Audio { "Video transitions go on video tracks." } else { "Audio transitions go on audio tracks." });
    }
    let edit = track.edit_at(position_us).ok_or("There is no edit point there.")?;
    fit_transition(track, &edit, template, sources, fps)
}

/// The standard transitions for Add Transition, one per free edit, each fitted with the
/// ones before it already in place, so a short clip's two transitions share its length.
/// Returns them with the reason the last refused edit gave, if any.
fn fill_edits(
    timeline: &Timeline,
    sources: &HashMap<Uuid, MediaSource>,
    fps: RationalFps,
    edits: &[(Uuid, i64, TransitionKind)],
    length_us: i64,
) -> (Vec<(Uuid, Transition)>, Option<&'static str>) {
    let mut scratch = timeline.clone();
    let (mut placed, mut why) = (Vec::new(), None);
    for (track_id, position, kind) in edits {
        // No second transition on an edit that has one.
        let taken = scratch.find_track(*track_id).and_then(|track| {
            let edit = track.edit_at(*position)?;
            Some(track.transition_at_edit(edit.clip_id, edit.edge).is_some())
        });
        if taken != Some(false) {
            why = Some("There is already a transition there.");
            continue;
        }
        let template = Transition::new(Uuid::nil(), TransitionEdge::In, kind.clone(), length_us, TransitionAlignment::Centered);
        match place_on(&scratch, sources, fps, *track_id, *position, &template) {
            Ok(t) => {
                if SetTransitionCommand::set(*track_id, t.clone()).execute(&mut scratch).is_ok() {
                    placed.push((*track_id, t));
                }
            }
            Err(e) => why = Some(e),
        }
    }
    (placed, why)
}

/// Ctrl+C with a transition selected: copy the transition alone, as Resolve does.
pub fn copy_transition(state: &AppState) -> bool {
    let Some((_, t)) = selected_transition(state) else { return false };
    *state.transition_clipboard.borrow_mut() = Some(t);
    true
}

/// Backspace / Delete with a transition selected: remove only the transition.
pub fn delete_transition(state: &Rc<AppState>) -> bool {
    let Some((track, t)) = selected_transition(state) else { return false };
    state.execute(Box::new(SetTransitionCommand::remove(track, t.clip_id, t.edge)));
    true
}

/// Ctrl+V with an edit point selected and a transition copied: put the copy on that
/// edit (centred on a cut, inside the clip at a head or tail), replacing any there, as
/// one undo step. The edit stays selected, so Up, Ctrl+V can carry on.
pub fn paste_transition(state: &Rc<AppState>) -> bool {
    let Some(EditSelection::Edit { track_id, position_us }) = state.edit_selection.get() else { return false };
    let Some(copied) = state.transition_clipboard.borrow().clone() else { return false };
    match placed_transition(state, track_id, position_us, &copied) {
        Ok(t) => {
            state.execute(Box::new(SetTransitionCommand::set(track_id, t)));
        }
        Err(why) => state.message(why),
    }
    true
}

/// Change the length of a transition, limited to one frame and to what the handles allow.
pub fn set_transition_length(state: &Rc<AppState>, track_id: Uuid, clip_id: Uuid, edge: TransitionEdge, length_us: i64) {
    let changed = state
        .with_project(|p| {
            let track = p.timeline.find_track(track_id)?;
            let mut t = track.transition_at_edit(clip_id, edge)?.clone();
            let max = track.max_transition_us(clip_id, edge, t.alignment, &p.sources, p.timeline_fps());
            t.duration_us = dragged_length(length_us, 0, false, false, max, p.timeline_fps());
            Some(t)
        })
        .flatten();
    if let Some(t) = changed {
        state.execute(Box::new(SetTransitionCommand::set(track_id, t)));
    }
}

/// Clicking a transition in the Effects panel: put `kind` on the selected edit point
/// (and the edits linked to it), or on both ends of each selected clip.
pub fn add_transition(state: &Rc<AppState>, kind: TransitionKind) {
    if is_audio_kind(&kind) {
        add_transitions(state, None, true);
    } else {
        add_transitions(state, Some(kind), false);
    }
}

/// Resolve's Add Transition keys: Ctrl+T both, Alt+T `video` only, Shift+T `audio` only.
/// The standard transition is a 1 s Cross Dissolve on pictures, a Cross Fade on sound.
pub fn add_standard_transitions(state: &Rc<AppState>, video: bool, audio: bool) {
    add_transitions(state, video.then_some(TransitionKind::CrossDissolve), audio);
}

/// The edits Add Transition works on: the selected edit point or transition with the
/// edits linked to it, else both ends of every selected clip. (track, kind, position).
fn edits_to_fill(state: &AppState) -> Vec<(Uuid, TrackKind, i64)> {
    let linked = state.linked_selection.get();
    let at = match state.edit_selection.get() {
        Some(EditSelection::Edit { track_id, position_us }) => Some((track_id, position_us)),
        Some(EditSelection::Transition { track_id, .. }) => {
            selected_transition(state).and_then(|(_, t)| state.with_timeline(|tl| transition_edit_us(tl.find_track(track_id)?, &t)).flatten()).map(|p| (track_id, p))
        }
        None => None,
    };
    let mut edits: Vec<(Uuid, TrackKind, i64)> = match at {
        Some((track_id, pos)) => state.with_timeline(|t| edit_group(t, track_id, pos, linked)).unwrap_or_default().into_iter().map(|(id, kind)| (id, kind, pos)).collect(),
        None => {
            let kinds = |id: Uuid| state.with_timeline(|t| t.find_track(id).map(|tr| tr.kind)).flatten().unwrap_or(TrackKind::Video);
            all_selected(state).into_iter().flat_map(|(track, c)| [(track, kinds(track), c.timeline_in), (track, kinds(track), c.timeline_out)]).collect()
        }
    };
    edits.sort_by_key(|(id, _, pos)| (*id, *pos));
    edits.dedup();
    edits
}

fn add_transitions(state: &Rc<AppState>, video: Option<TransitionKind>, audio: bool) {
    let edits = edits_to_fill(state);
    if edits.is_empty() {
        state.message("Select an edit point or a clip on the timeline first.");
        return;
    }
    let wanted: Vec<(Uuid, i64, TransitionKind)> = edits
        .into_iter()
        .filter_map(|(track_id, track_kind, position)| {
            let kind = match track_kind {
                TrackKind::Video => video.clone(),
                TrackKind::Audio => audio.then_some(TransitionKind::Crossfade),
            };
            kind.map(|k| (track_id, position, k))
        })
        .collect();
    if wanted.is_empty() {
        state.message(if audio && video.is_none() {
            "Audio transitions go on audio tracks."
        } else {
            "Video transitions go on video tracks."
        });
        return;
    }
    let length = state.standard_transition_us();
    let Some((placed, why)) = state.with_project(|p| fill_edits(&p.timeline, &p.sources, p.timeline_fps(), &wanted, length)) else { return };
    if placed.is_empty() {
        state.message(why.unwrap_or("There is already a transition there."));
        return;
    }
    let commands: Vec<Box<dyn Command>> = placed.into_iter().map(|(track_id, t)| Box::new(SetTransitionCommand::set(track_id, t)) as Box<dyn Command>).collect();
    state.execute(Box::new(CompositeCommand::new("Add Transition", commands)));
}

/// Link or unlink the selected clips (Resolve's Ctrl+Alt+L).
pub fn toggle_link(state: &Rc<AppState>) {
    let clips = all_selected(state);
    if clips.is_empty() {
        return;
    }
    // If every selected clip already shares one link, this unlinks them.
    let first = clips[0].1.properties.link;
    let link = if clips.len() > 1 && first.is_some() && clips.iter().all(|(_, c)| c.properties.link == first) { None } else { Some(Uuid::new_v4()) };
    let link = if clips.len() == 1 { None } else { link };
    for_selected(state, if link.is_some() { "Link Clips" } else { "Unlink Clips" }, |_, clip| {
        let mut edited = clip.clone();
        edited.properties.link = link;
        (edited != *clip).then(|| Box::new(EditClipCommand::new("Link", edited)) as Box<dyn Command>)
    });
}

/// Add a title clip at the playhead, on the lowest video track that is free there.
pub fn add_title(state: &Rc<AppState>, lower_third: bool) {
    const LENGTH_US: i64 = 5_000_000;
    let at = state.frame_floor(state.playhead_us());
    // Prefer the track above the main one, so the title sits over the picture.
    // If a later clip is in the way, the title is shortened to fit (at least a second).
    let free = state
        .with_timeline(|t| {
            let mut tracks: Vec<_> = t.tracks.iter().filter(|tr| tr.kind == TrackKind::Video && !tr.locked).collect();
            tracks.sort_by_key(|tr| if tr.kind_index == 1 { u32::MAX } else { tr.kind_index });
            tracks.into_iter().find_map(|tr| {
                if tr.clip_at(at).is_some() {
                    return None;
                }
                let room = tr.clips.iter().map(|c| c.timeline_in).filter(|start| *start > at).min().map_or(LENGTH_US, |next| next - at);
                (room >= 1_000_000).then(|| (tr.id, room.min(LENGTH_US)))
            })
        })
        .flatten();
    let Some((track, length)) = free else {
        state.message("No free video track at the playhead for a title.");
        return;
    };
    let mut clip = Clip::new(track, Uuid::nil(), ClipType::Title, if lower_third { "Lower Third" } else { "Title" }, at, at + length, 0, length);
    clip.title_data = Some(if lower_third {
        TitleData { title_type: TitleType::LowerThird, text: "Name".into(), font_size: 48.0, background_color: Some([0, 0, 0, 170]), ..Default::default() }
    } else {
        TitleData::default()
    });
    let id = clip.id;
    if state.execute(Box::new(InsertClipCommand::new(clip))) {
        state.select(Some(id));
    }
}

/// Copy the selected clips. With `cut` they are also removed, leaving a gap.
pub fn copy_selected(state: &Rc<AppState>, cut: bool) {
    let clips = all_selected(state);
    if clips.is_empty() {
        state.message("Select a clip on the timeline first.");
        return;
    }
    *state.clipboard.borrow_mut() = clips.iter().map(|(_, c)| c.clone()).collect();
    if cut {
        for_selected(state, "Cut", |track, clip| Some(Box::new(DeleteClipCommand::new(track, clip.id)) as Box<dyn Command>));
    }
}

/// Build pasted copies of `copied` starting at `at`, keeping multi-track offsets or
/// routing single / linked clips to the current destination tracks.
fn prepare_pasted_clips(state: &AppState, copied: &[Clip], at: i64) -> Vec<Clip> {
    let earliest = copied.iter().map(|c| c.timeline_in).min().unwrap_or(0);
    let kinds: std::collections::HashSet<bool> = copied.iter().map(|c| c.clip_type == ClipType::Audio).collect();
    // A picture with its sound, or a single clip, follows the destination tracks.
    let to_destination = copied.len() == 1 || (copied.len() == 2 && kinds.len() == 2);
    let mut links: std::collections::HashMap<Uuid, Uuid> = Default::default();
    let mut pasted = Vec::new();
    for clip in copied {
        let mut clip = clip.clone();
        let kind = if clip.clip_type == ClipType::Audio { TrackKind::Audio } else { TrackKind::Video };
        let own_track = state.with_timeline(|t| t.find_track(clip.track_id).is_some_and(|tr| !tr.locked)).unwrap_or(false);
        let track = if to_destination || !own_track { state.dest_track(kind) } else { Some(clip.track_id) };
        let Some(track) = track else { continue };
        let len = clip.duration_us();
        clip.id = Uuid::new_v4();
        clip.track_id = track;
        clip.timeline_in = at + (clip.timeline_in - earliest);
        clip.timeline_out = clip.timeline_in + len;
        // The copies are linked to each other, not to the clips they came from.
        clip.properties.link = clip.properties.link.map(|old| *links.entry(old).or_insert_with(Uuid::new_v4));
        pasted.push(clip);
    }
    pasted
}

/// Paste the copied clips at the playhead. One clip goes to the destination
/// track of its kind; several keep their own tracks and their spacing, and a
/// picture stays linked to its sound.
pub fn paste(state: &Rc<AppState>) {
    let copied = state.clipboard.borrow().clone();
    if copied.is_empty() {
        state.message("Nothing to paste: copy a clip first (Ctrl+C).");
        return;
    }
    let at = state.frame_floor(state.playhead_us());
    let pasted = prepare_pasted_clips(state, &copied, at);
    let Some(first) = pasted.first().map(|c| c.id) else { return };
    let end = pasted.iter().map(|c| c.timeline_out).max().unwrap_or(at);
    let commands: Vec<Box<dyn Command>> = pasted.into_iter().map(|c| Box::new(OverwriteClipCommand::new(c)) as Box<dyn Command>).collect();
    if state.execute(Box::new(CompositeCommand::new("Paste", commands))) {
        state.select(Some(first));
        state.player.seek_timeline(end);
    }
}

/// Paste Insert (Ctrl+Shift+V): make room at the playhead across all unlocked tracks,
/// splitting clips that cross it, then insert the copied clips there.
pub fn paste_insert(state: &Rc<AppState>) {
    let copied = state.clipboard.borrow().clone();
    if copied.is_empty() {
        state.message("Nothing to paste: copy a clip first (Ctrl+C).");
        return;
    }
    let at = state.frame_floor(state.playhead_us());
    let pasted = prepare_pasted_clips(state, &copied, at);
    let Some(first) = pasted.first().map(|c| c.id) else { return };
    let end = pasted.iter().map(|c| c.timeline_out).max().unwrap_or(at);
    let len = end - at;
    if len <= 0 {
        return;
    }
    let mut commands = ripple_open_commands(state, at, len);
    commands.extend(pasted.into_iter().map(|c| Box::new(InsertClipCommand::new(c)) as Box<dyn Command>));
    if state.execute(Box::new(CompositeCommand::new("Paste Insert", commands))) {
        state.select(Some(first));
        state.player.seek_timeline(end);
    }
}

/// Commands that open a gap of `len` microseconds at `at` across all unlocked tracks.
fn ripple_open_commands(state: &AppState, at: i64, len: i64) -> Vec<Box<dyn Command>> {
    let (across, mut later) = state
        .with_timeline(|t| {
            let unlocked = || t.tracks.iter().filter(|tr| !tr.locked);
            let across: Vec<Uuid> =
                unlocked().flat_map(|tr| tr.clips.iter()).filter(|c| c.timeline_in < at && c.timeline_out > at).map(|c| c.id).collect();
            let later: Vec<(Uuid, Uuid, i64)> =
                unlocked().flat_map(|tr| tr.clips.iter().filter(|c| c.timeline_in >= at).map(|c| (tr.id, c.id, c.timeline_in))).collect();
            (across, later)
        })
        .unwrap_or_default();
    let mut commands: Vec<Box<dyn Command>> = Vec::new();
    for (track, split) in split_plan(state, &across, at) {
        later.push((track, split.second_clip_id(), at));
        commands.push(Box::new(split));
    }
    later.sort_by_key(|m| std::cmp::Reverse(m.2));
    for (track, id, from) in later {
        commands.push(Box::new(MoveClipCommand::new(id, track, track, from, from + len)));
    }
    commands
}

/// Replace the selected timeline clips with the current Media Pool clip, keeping
/// their place and length. A picture and its linked sound are both replaced.
pub fn replace_selected(state: &Rc<AppState>) {
    let Some(source) = current_source(state) else {
        state.message("Select a clip on the timeline and a clip in the Media Pool first.");
        return;
    };
    let Some((name, duration, media, has_audio, speed)) = state
        .with_project(|p| {
            p.sources
                .get(&source)
                .map(|s| (s.display_name(), s.duration_us, s.media_type, s.audio_channels.is_some(), s.conform_speed()))
        })
        .flatten()
    else {
        return;
    };
    if state.selected_ids().is_empty() {
        state.message("Select a clip on the timeline first.");
        return;
    }
    // The In mark is in viewer (conformed) time; the command wants file time.
    let src_in = if state.source_clip.get() == Some(source) { tempo_timeline::scale_us(state.src_in.get().unwrap_or(0), speed) } else { 0 };
    let too_short = std::cell::Cell::new(false);
    let done = for_selected(state, "Replace", |track, clip| {
        // Like for like: a picture for a picture, a sound for a sound.
        let fits = match clip.clip_type {
            ClipType::Video => media == MediaType::Video,
            ClipType::Image => media == MediaType::Image,
            ClipType::Audio => has_audio,
            ClipType::Title => false,
        };
        if !fits {
            return None;
        }
        // A still picture lasts as long as it is asked to.
        if media != MediaType::Image && duration - src_in < clip.duration_us() {
            too_short.set(true);
            return None;
        }
        Some(Box::new(ReplaceClipCommand::new(track, clip.id, source, src_in, name.clone())) as Box<dyn Command>)
    });
    if !done {
        state.message(if too_short.get() {
            "The new clip is too short to replace this one."
        } else {
            "That clip cannot stand in for the selected one: pick a video for a video, a picture for a picture, or a clip with sound for a sound."
        });
    }
}

/// Cuts of each of `ids` at `at`, with the track each is on (clips that do not
/// span `at`, or sit on a locked track, are left alone). When a picture and
/// its sound are both cut, the two later halves become a linked pair of their own.
fn split_plan(state: &AppState, ids: &[Uuid], at: i64) -> Vec<(Uuid, SplitClipCommand)> {
    state
        .with_timeline(|t| {
            let targets: Vec<(Uuid, Uuid, Option<Uuid>)> = ids
                .iter()
                .filter_map(|id| t.find_clip(*id))
                .filter(|(track, c)| !track.locked && at > c.timeline_in && at < c.timeline_out)
                .map(|(track, c)| (track.id, c.id, c.properties.link))
                .collect();
            let mut new_links: std::collections::HashMap<Uuid, Uuid> = Default::default();
            targets
                .iter()
                .map(|(track, id, link)| {
                    let command = SplitClipCommand::new(*track, *id, at);
                    let shared = link.filter(|l| targets.iter().filter(|(_, _, other)| *other == Some(*l)).count() > 1);
                    let command = match shared {
                        Some(l) => command.with_second_link(*new_links.entry(l).or_insert_with(Uuid::new_v4)),
                        None => command,
                    };
                    (*track, command)
                })
                .collect()
        })
        .unwrap_or_default()
}

fn split_commands(state: &AppState, ids: &[Uuid], at: i64) -> Vec<Box<dyn Command>> {
    split_plan(state, ids, at).into_iter().map(|(_, command)| Box::new(command) as Box<dyn Command>).collect()
}

/// Cut every unlocked clip under the playhead (Resolve's Razor).
pub fn razor(state: &Rc<AppState>) {
    let pos = state.playhead_us();
    let under: Vec<Uuid> = state
        .with_timeline(|t| t.tracks.iter().flat_map(|tr| tr.clips.iter()).filter(|c| pos > c.timeline_in && pos < c.timeline_out).map(|c| c.id).collect())
        .unwrap_or_default();
    let cuts = split_commands(state, &under, pos);
    if cuts.is_empty() {
        state.message("There is no clip under the playhead to cut.");
    } else if state.execute(Box::new(CompositeCommand::new("Razor", cuts))) {
        crate::help::tick_step(&state.checklist_split, "split");
        state.emit(Change::Options);
    }
}

/// Cut a clip at a time, together with the clips linked to it (the Blade tool).
pub fn split_at(state: &Rc<AppState>, clip_id: Uuid, at_us: i64) {
    let mut ids = vec![clip_id];
    ids.extend(state.linked_to(clip_id));
    let cuts = split_commands(state, &ids, at_us);
    if !cuts.is_empty() && state.execute(Box::new(CompositeCommand::new("Split", cuts))) {
        crate::help::tick_step(&state.checklist_split, "split");
        state.emit(Change::Options);
    }
}

/// Cut the selected clips at the playhead, or unlocked clips under the playhead when nothing is selected.
pub fn split_selected(state: &Rc<AppState>) {
    let pos = state.playhead_us();
    let ids = state.selected_ids();
    let had_selection = !ids.is_empty();
    let targets = if had_selection {
        ids
    } else {
        state.with_timeline(|t| clips_under(t, pos).into_iter().map(|(_, c)| c.id).collect()).unwrap_or_default()
    };
    let cuts = split_commands(state, &targets, pos);
    if cuts.is_empty() {
        state.message(if had_selection {
            "Move the playhead inside the selected clip first."
        } else {
            "There is no clip under the playhead to cut."
        });
    } else if state.execute(Box::new(CompositeCommand::new("Split", cuts))) {
        crate::help::tick_step(&state.checklist_split, "split");
        state.emit(Change::Options);
    }
}

fn can_join(left: &Clip, right: &Clip) -> bool {
    left.timeline_out == right.timeline_in
        && left.source_id == right.source_id
        && left.clip_type == right.clip_type
        && (left.properties.speed - right.properties.speed).abs() < 1e-6
        && (matches!(left.clip_type, ClipType::Image | ClipType::Title) || (left.source_out - right.source_in).abs() <= 1)
        && left.title_data == right.title_data
}

/// Join adjacent continuous same-source clips on the same track (Resolve's Alt+\).
pub fn join_clip(state: &Rc<AppState>) {
    let pos = state.playhead_us();
    let frame = state.frame_us();
    let selected = state.selected_ids();
    let edit_sel = state.edit_selection.get();
    let linked = state.linked_selection.get();

    let commands: Vec<Box<dyn Command>> = state
        .with_timeline(|t| {
            let mut per_track: HashMap<Uuid, Vec<usize>> = HashMap::new();
            let push_pair = |map: &mut HashMap<Uuid, Vec<usize>>, tid: Uuid, idx: usize| {
                let list = map.entry(tid).or_default();
                if !list.contains(&idx) {
                    list.push(idx);
                }
            };

            if let Some(EditSelection::Edit { track_id, position_us }) = edit_sel {
                for (tid, _) in edit_group(t, track_id, position_us, linked) {
                    let Some(tr) = t.find_track(tid).filter(|tr| !tr.locked) else { continue };
                    for (i, w) in tr.clips.windows(2).enumerate() {
                        if w[0].timeline_out == position_us && can_join(&w[0], &w[1]) {
                            push_pair(&mut per_track, tid, i);
                        }
                    }
                }
            } else if !selected.is_empty() {
                for tr in t.tracks.iter().filter(|tr| !tr.locked) {
                    for (i, w) in tr.clips.windows(2).enumerate() {
                        if selected.contains(&w[0].id) && selected.contains(&w[1].id) && can_join(&w[0], &w[1]) {
                            push_pair(&mut per_track, tr.id, i);
                        }
                    }
                }
                if per_track.is_empty() {
                    let collect_near = |tol: i64, map: &mut HashMap<Uuid, Vec<usize>>| {
                        for tr in t.tracks.iter().filter(|tr| !tr.locked) {
                            for (i, w) in tr.clips.windows(2).enumerate() {
                                if (selected.contains(&w[0].id) || selected.contains(&w[1].id))
                                    && (w[0].timeline_out - pos).abs() <= tol
                                    && can_join(&w[0], &w[1])
                                {
                                    push_pair(map, tr.id, i);
                                }
                            }
                        }
                    };
                    collect_near(0, &mut per_track);
                    if per_track.is_empty() {
                        collect_near(frame, &mut per_track);
                    }
                }
            } else {
                let collect_at = |tol: i64, map: &mut HashMap<Uuid, Vec<usize>>| {
                    for tr in t.tracks.iter().filter(|tr| !tr.locked) {
                        for (i, w) in tr.clips.windows(2).enumerate() {
                            if (w[0].timeline_out - pos).abs() <= tol && can_join(&w[0], &w[1]) {
                                push_pair(map, tr.id, i);
                            }
                        }
                    }
                };
                collect_at(0, &mut per_track);
                if per_track.is_empty() {
                    collect_at(frame, &mut per_track);
                }
            }

            let mut cmds: Vec<Box<dyn Command>> = Vec::new();
            for tr in &t.tracks {
                let Some(mut idxs) = per_track.remove(&tr.id) else { continue };
                idxs.sort_unstable();
                idxs.dedup();
                let mut runs: Vec<(usize, usize)> = Vec::new();
                for idx in idxs {
                    match runs.last_mut() {
                        Some((_, end)) if idx == *end => *end = idx + 1,
                        _ => runs.push((idx, idx + 1)),
                    }
                }
                for (start, end) in runs.into_iter().rev() {
                    let first = &tr.clips[start];
                    let last = &tr.clips[end];
                    let moved_tail = tr.transition_at_edit(last.id, TransitionEdge::Out).cloned().map(|mut trans| {
                        trans.clip_id = first.id;
                        trans
                    });
                    for k in (start + 1..=end).rev() {
                        cmds.push(Box::new(DeleteClipCommand::new(tr.id, tr.clips[k].id)));
                    }
                    let mut joined = first.clone();
                    joined.timeline_out = last.timeline_out;
                    joined.source_out = last.source_out;
                    joined.properties.fade_out_us = last.properties.fade_out_us;
                    cmds.push(Box::new(EditClipCommand::new("Join Clip", joined)));
                    if let Some(tail) = moved_tail {
                        cmds.push(Box::new(SetTransitionCommand::set(tr.id, tail)));
                    }
                }
            }
            cmds
        })
        .unwrap_or_default();

    if commands.is_empty() {
        state.message("No adjacent continuous clips to join here.");
    } else {
        state.execute(Box::new(CompositeCommand::new("Join Clip", commands)));
    }
}

/// Ripple Overwrite (Shift+F10): replace the selected clip (or clip under the playhead)
/// with the current source clip and shift later clips on all unlocked tracks by the length difference.
pub fn ripple_overwrite(state: &Rc<AppState>) {
    let Some(source_id) = current_source(state) else {
        state.message("Pick a clip in the Media Pool first.");
        return;
    };
    let pos = state.frame_floor(state.playhead_us());
    let targets = {
        let sel = all_selected(state);
        if !sel.is_empty() {
            sel
        } else {
            let dest_v = state.dest_track(TrackKind::Video);
            let dest_a = state.dest_track(TrackKind::Audio);
            let primary = state
                .with_timeline(|t| {
                    dest_v
                        .and_then(|id| t.find_track(id).filter(|tr| !tr.locked))
                        .and_then(|tr| tr.clip_at(pos))
                        .or_else(|| dest_a.and_then(|id| t.find_track(id).filter(|tr| !tr.locked)).and_then(|tr| tr.clip_at(pos)))
                        .map(|c| c.id)
                })
                .flatten();
            match primary {
                Some(id) => {
                    let mut ids = vec![id];
                    ids.extend(state.linked_to(id));
                    state
                        .with_timeline(|t| {
                            ids.iter().filter_map(|cid| t.find_clip(*cid)).filter(|(tr, _)| !tr.locked).map(|(tr, c)| (tr.id, c.clone())).collect()
                        })
                        .unwrap_or_default()
                }
                None => Vec::new(),
            }
        }
    };
    if targets.is_empty() {
        place_source(state, source_id, Place::Insert, None, None, true);
        return;
    }

    let old_in = targets.iter().map(|(_, c)| c.timeline_in).min().unwrap_or(pos);
    let old_out = targets.iter().map(|(_, c)| c.timeline_out).max().unwrap_or(pos);
    let old_len = old_out - old_in;

    let video_track = targets
        .iter()
        .find(|(_, c)| c.clip_type != ClipType::Audio)
        .map(|(tr, _)| *tr)
        .or_else(|| state.dest_track(TrackKind::Video));
    let audio_track = targets
        .iter()
        .find(|(_, c)| c.clip_type == ClipType::Audio)
        .map(|(tr, _)| *tr)
        .or_else(|| state.dest_track(TrackKind::Audio));

    let new_clips = clips_for_source(state, source_id, old_in, video_track, audio_track);
    if new_clips.is_empty() {
        state.message("Pick a clip in the Media Pool first.");
        return;
    }
    let first_id = new_clips[0].id;
    let new_out = new_clips[0].timeline_out;
    let delta = (new_out - old_in) - old_len;
    let target_ids: Vec<Uuid> = targets.iter().map(|(_, c)| c.id).collect();
    let target_tracks: Vec<Uuid> = targets.iter().map(|(tr, _)| *tr).collect();

    let mut commands: Vec<Box<dyn Command>> = targets
        .iter()
        .map(|(tr, c)| Box::new(DeleteClipCommand::new(*tr, c.id)) as Box<dyn Command>)
        .collect();

    if delta != 0 {
        let mut later: Vec<(Uuid, Uuid, i64)> = state
            .with_timeline(|t| {
                t.tracks
                    .iter()
                    .filter(|tr| !tr.locked)
                    .flat_map(|tr| {
                        tr.clips
                            .iter()
                            .filter(|c| !target_ids.contains(&c.id) && c.timeline_in >= old_out)
                            .map(|c| (tr.id, c.id, c.timeline_in))
                    })
                    .collect()
            })
            .unwrap_or_default();
        if delta > 0 {
            later.sort_by_key(|m| std::cmp::Reverse(m.2));
        } else {
            later.sort_by_key(|m| m.2);
        }
        for (track, id, from) in later {
            commands.push(Box::new(MoveClipCommand::new(id, track, track, from, from + delta)));
        }
    }

    for clip in new_clips {
        if target_tracks.contains(&clip.track_id) {
            commands.push(Box::new(InsertClipCommand::new(clip)));
        } else {
            commands.push(Box::new(OverwriteClipCommand::new(clip)));
        }
    }

    if state.execute(Box::new(CompositeCommand::new("Ripple Overwrite", commands))) {
        state.select(Some(first_id));
        state.player.seek_timeline(new_out);
    }
}

/// Swap each selected clip with its neighbor before (`forward = false`) or after (`forward = true`) on its track.
pub fn swap_clips(state: &Rc<AppState>, forward: bool) {
    let selected = all_selected(state);
    if selected.is_empty() {
        state.message("Select a clip on the timeline first.");
        return;
    }
    let commands: Vec<Box<dyn Command>> = state
        .with_timeline(|t| {
            let mut cmds: Vec<Box<dyn Command>> = Vec::new();
            let mut handled_tracks: Vec<Uuid> = Vec::new();
            for (track_id, clip) in &selected {
                if handled_tracks.contains(track_id) {
                    continue;
                }
                let Some(track) = t.find_track(*track_id).filter(|tr| !tr.locked) else { continue };
                let Some(idx) = track.clips.iter().position(|c| c.id == clip.id) else { continue };
                let neighbor_idx = if forward {
                    (idx + 1 < track.clips.len()).then_some(idx + 1)
                } else {
                    idx.checked_sub(1)
                };
                let Some(n_idx) = neighbor_idx else { continue };
                let (left, right) = if forward { (&track.clips[idx], &track.clips[n_idx]) } else { (&track.clips[n_idx], &track.clips[idx]) };
                handled_tracks.push(*track_id);
                let left_len = left.duration_us();
                let right_in = right.timeline_in;
                let right_out = right.timeline_out;
                let left_in = left.timeline_in;
                let temp = track.duration_us().max(right_out) + left_len + right.duration_us() + 1_000_000;
                cmds.push(Box::new(MoveClipCommand::new(left.id, *track_id, *track_id, left_in, temp)));
                cmds.push(Box::new(MoveClipCommand::new(right.id, *track_id, *track_id, right_in, left_in)));
                cmds.push(Box::new(MoveClipCommand::new(left.id, *track_id, *track_id, temp, right_out - left_len)));
            }
            cmds
        })
        .unwrap_or_default();

    if commands.is_empty() {
        state.message("No clip there to swap with.");
    } else {
        state.execute(Box::new(CompositeCommand::new("Swap Clips", commands)));
    }
}

/// Adjust selected audio clips' volume by `db` decibels. A nudge never goes past the Inspector's
/// 200%, and never pulls a clip already above that (set by a plugin or an older project) down.
pub fn adjust_volume_db(state: &Rc<AppState>, db: f32) {
    let factor = 10.0f32.powf(db / 20.0);
    let done = for_selected(state, "Adjust Volume", |_, clip| {
        if clip.clip_type != ClipType::Audio {
            return None;
        }
        let cur = clip.properties.volume;
        let next = if db > 0.0 && cur < 0.001 { 0.001 * factor } else { cur * factor }.clamp(0.0, cur.max(2.0));
        Some(Box::new(SetClipPropertyCommand::new(clip.id, PropertyChange::Volume(next))) as Box<dyn Command>)
    });
    if !done && state.selected_ids().is_empty() {
        state.message("Select an audio clip first.");
    }
}

/// Toggle enabled state of Video track `kind_index` (1-based, Ctrl+Shift+1..4).
pub fn toggle_video_track(state: &Rc<AppState>, kind_index: u32) {
    let target = state
        .with_timeline(|t| t.tracks.iter().find(|tr| tr.kind == TrackKind::Video && tr.kind_index == kind_index).map(|tr| (tr.id, tr.enabled)))
        .flatten();
    if let Some((track_id, enabled)) = target {
        state.execute(Box::new(SetTrackFlagCommand::new(track_id, TrackFlag::Enabled, !enabled)));
    }
}

/// Trim the start or end of the selected clips to the playhead. With `ripple`
/// the clips after them move up to close the gap.
pub fn trim_to_playhead(state: &Rc<AppState>, edge: TrimEdge, ripple: bool) {
    let pos = state.playhead_us();
    let name = if ripple { "Ripple Trim" } else { "Trim" };
    // Only clips the playhead is inside can be trimmed to it.
    let make = |_: Uuid, clip: &Clip| {
        let delta = match edge {
            TrimEdge::In => pos - clip.timeline_in,
            TrimEdge::Out => pos - clip.timeline_out,
        };
        (pos > clip.timeline_in && pos < clip.timeline_out).then(|| -> Box<dyn Command> {
            if ripple {
                Box::new(RippleTrimCommand::new(clip.id, edge, delta))
            } else {
                Box::new(TrimClipCommand::new(clip.id, edge, delta))
            }
        })
    };
    // As Resolve: with nothing selected, the clips under the playhead are trimmed.
    let done = if state.selected_ids().is_empty() {
        let under = state.with_timeline(|t| clips_under(t, pos)).unwrap_or_default();
        let commands: Vec<Box<dyn Command>> = under.iter().filter_map(|(track, clip)| make(*track, clip)).collect();
        !commands.is_empty() && state.execute(Box::new(CompositeCommand::new(name, commands)))
    } else {
        for_selected(state, name, make)
    };
    if !done {
        state.message("Put the playhead inside a clip first.");
    }
}

/// The clips the playhead is strictly inside, on unlocked tracks, latest first.
fn clips_under(timeline: &Timeline, pos: i64) -> Vec<(Uuid, Clip)> {
    timeline
        .tracks
        .iter()
        .filter(|t| !t.locked)
        .flat_map(|t| t.clips.iter().filter(|c| pos > c.timeline_in && pos < c.timeline_out).map(|c| (t.id, c.clone())))
        .collect()
}

/// Make the selected clips fade in up to the playhead, or fade out from it.
pub fn fade_to_playhead(state: &Rc<AppState>, fade_in: bool) {
    let pos = state.playhead_us();
    let done = for_selected(state, "Fade", |_, clip| {
        if pos <= clip.timeline_in || pos >= clip.timeline_out {
            return None;
        }
        let mut edited = clip.clone();
        if fade_in {
            edited.properties.fade_in_us = pos - clip.timeline_in;
        } else {
            edited.properties.fade_out_us = clip.timeline_out - pos;
        }
        Some(Box::new(EditClipCommand::new("Fade", edited)) as Box<dyn Command>)
    });
    if !done {
        state.message("Select a clip and put the playhead inside it first.");
    }
}

/// Select the clip under the playhead: on the destination video track if it
/// has one there, else on the highest track that does.
pub fn select_at_playhead(state: &Rc<AppState>) {
    let pos = state.playhead_us();
    let dest = state.dest_track(TrackKind::Video);
    let found = state
        .with_timeline(|t| {
            let mut tracks: Vec<_> = t.tracks.iter().collect();
            // Destination first, then video above audio, higher tracks first.
            tracks.sort_by_key(|tr| (Some(tr.id) != dest, tr.kind != TrackKind::Video, std::cmp::Reverse(tr.kind_index)));
            tracks.into_iter().find_map(|tr| tr.clip_at(pos).map(|c| c.id))
        })
        .flatten();
    match found {
        Some(id) => state.select(Some(id)),
        None => state.message("There is no clip under the playhead."),
    }
}

/// The gap under a point on a track: the empty stretch that has a clip after it.
pub fn gap_at(state: &AppState, track: Uuid, at_us: i64) -> Option<Gap> {
    state
        .with_timeline(|t| {
            let tr = t.find_track(track)?;
            if tr.clip_at(at_us).is_some() {
                return None;
            }
            let end = tr.clips.iter().map(|c| c.timeline_in).filter(|start| *start > at_us).min()?;
            let start = tr.clips.iter().map(|c| c.timeline_out).filter(|out| *out <= at_us).max().unwrap_or(0);
            (end > start).then_some(Gap { track, start, end })
        })
        .flatten()
}

/// The moves that close `[start, end)`: every clip after it, on every unlocked
/// track that is empty there, goes left by its length. `tracks` are the tracks
/// the hole is known to be on. Fails when that would pull a picture away from
/// its sound on a track that cannot move.
fn plan_close(t: &tempo_timeline::Timeline, tracks: &[Uuid], start: i64, end: i64) -> Result<Vec<(Uuid, Uuid, i64)>, &'static str> {
    let moving: Vec<&tempo_timeline::Track> =
        t.tracks.iter().filter(|tr| !tr.locked && (tracks.contains(&tr.id) || !tr.has_collision(start, end, None))).collect();
    if !tracks.iter().all(|id| moving.iter().any(|tr| tr.id == *id)) {
        return Err("That track is locked.");
    }
    let shifted: Vec<(Uuid, &Clip)> = moving.iter().flat_map(|tr| tr.clips.iter().filter(|c| c.timeline_in >= end).map(|c| (tr.id, c))).collect();
    let left_behind = shifted.iter().any(|(_, c)| {
        c.properties.link.is_some_and(|link| {
            t.tracks
                .iter()
                .filter(|tr| !moving.iter().any(|m| m.id == tr.id))
                .flat_map(|tr| tr.clips.iter())
                .any(|other| other.properties.link == Some(link) && other.timeline_in >= end)
        })
    });
    if left_behind {
        return Err("Another track has a clip in this gap, so closing it would move a picture away from its sound. Move or unlink that clip first.");
    }
    let mut moves: Vec<(Uuid, Uuid, i64)> = shifted.iter().map(|(track, c)| (*track, c.id, c.timeline_in)).collect();
    // Earliest first, so no clip runs into the one before it.
    moves.sort_by_key(|m| m.2);
    Ok(moves)
}

/// Close a gap: everything after it moves left by its length. Other tracks
/// that are empty over the same stretch move too, so nothing drifts apart.
pub fn close_gap(state: &Rc<AppState>, gap: Gap) {
    let len = gap.end - gap.start;
    match state.with_timeline(|t| plan_close(t, &[gap.track], gap.start, gap.end)) {
        Some(Ok(moves)) => {
            let commands: Vec<Box<dyn Command>> = moves
                .into_iter()
                .map(|(track, id, from)| Box::new(MoveClipCommand::new(id, track, track, from, from - len)) as Box<dyn Command>)
                .collect();
            if !commands.is_empty() {
                state.execute(Box::new(CompositeCommand::new("Delete Gap", commands)));
            }
        }
        Some(Err(reason)) => state.message(reason),
        None => {}
    }
}

/// Take a file out of the Media Pool. Files still used on any timeline stay.
pub fn remove_source(state: &Rc<AppState>, source: Uuid) {
    let used_on = state
        .with_project(|p| {
            std::iter::once(&p.timeline)
                .chain(&p.timelines)
                .find(|t| t.tracks.iter().flat_map(|tr| tr.clips.iter()).any(|c| c.source_id == source))
                .map(|t| t.name.clone())
        })
        .flatten();
    if let Some(timeline) = used_on {
        state.message(format!("This clip is used on {timeline}. Delete it from that timeline first."));
        return;
    }
    let removed = state.project.borrow_mut().as_mut().is_some_and(|p| p.sources.remove(&source).is_some());
    if removed {
        if state.media_selection.get() == Some(source) {
            state.media_selection.set(None);
        }
        if state.source_clip.get() == Some(source) {
            state.show_source(None);
        }
        state.sync_player();
        state.set_dirty(true);
        state.emit(Change::Media);
    }
}

pub fn nudge(state: &Rc<AppState>, frames: i64) {
    // Measured from the first selected clip, so it lands exactly on a frame.
    let earliest = all_selected(state).iter().map(|(_, c)| c.timeline_in).min();
    if let Some(from) = earliest {
        move_selected_by(state, state.frame_step(from, frames) - from);
    }
}

/// Move every selected clip by the same amount of time, as one undo step.
pub fn move_selected_by(state: &Rc<AppState>, delta_us: i64) -> bool {
    move_selected(state, delta_us, None)
}

/// Move every selected clip in time and, with `shift`, the clips of one kind
/// up or down by a number of tracks. All of it is one undo step.
pub fn move_selected(state: &Rc<AppState>, delta_us: i64, shift: Option<(TrackKind, i32)>) -> bool {
    let mut clips = all_selected(state);
    if clips.is_empty() || (delta_us == 0 && shift.is_none()) {
        return false;
    }
    // Nothing may move before the start of the timeline.
    let earliest = clips.iter().map(|(_, c)| c.timeline_in).min().unwrap_or(0);
    let delta = delta_us.max(-earliest);
    // Move the clips at the leading side first, so they never run into each other.
    if delta < 0 {
        clips.reverse();
    }
    // Where each clip's track ends up: the same track, or the one `shift` rows away.
    let targets: Option<Vec<Uuid>> = state
        .with_timeline(|t| {
            clips
                .iter()
                .map(|(track, _)| {
                    let from = t.find_track(*track)?;
                    match shift.filter(|(kind, _)| *kind == from.kind) {
                        Some((kind, rows)) => {
                            let want = from.kind_index as i32 + rows;
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
        return false;
    };
    let commands: Vec<Box<dyn Command>> = clips
        .iter()
        .zip(targets)
        .filter(|((track, _), target)| delta != 0 || track != target)
        .map(|((track, c), target)| Box::new(MoveClipCommand::new(c.id, *track, target, c.timeline_in, c.timeline_in + delta)) as Box<dyn Command>)
        .collect();
    !commands.is_empty() && state.execute(Box::new(CompositeCommand::new("Move", commands)))
}

/// Trim one edge of a clip; with `ripple`, later clips follow. Clips linked to
/// it are trimmed the same way while linked selection is on.
pub fn trim_clip(state: &Rc<AppState>, clip_id: Uuid, edge: TrimEdge, delta_us: i64, ripple: bool) {
    let mut ids = vec![clip_id];
    // A clip edge grabbed as an edit point leaves the clip unselected; its linked partners still follow.
    let others = if state.is_selected(clip_id) { state.selected_ids() } else { state.linked_to(clip_id) };
    ids.extend(others.into_iter().filter(|id| *id != clip_id));
    let link = state.with_timeline(|t| t.find_clip(clip_id).and_then(|(_, c)| c.properties.link)).flatten();
    // Only clips linked to this one follow; other selected clips are left alone.
    let partners: Vec<Uuid> = state
        .with_timeline(|t| {
            ids.iter()
                .copied()
                .filter(|id| *id == clip_id || (link.is_some() && t.find_clip(*id).is_some_and(|(_, c)| c.properties.link == link)))
                .collect()
        })
        .unwrap_or_default();
    let commands: Vec<Box<dyn Command>> = partners
        .into_iter()
        .map(|id| -> Box<dyn Command> {
            if ripple {
                Box::new(RippleTrimCommand::new(id, edge, delta_us))
            } else {
                Box::new(TrimClipCommand::new(id, edge, delta_us))
            }
        })
        .collect();
    state.execute(Box::new(CompositeCommand::new(if ripple { "Ripple Trim" } else { "Trim" }, commands)));
}

/// Roll a cut: move the point where two touching clips meet, keeping the total
/// length. Clips linked to the one being rolled roll with it, so picture and
/// sound stay together.
pub fn roll_cut(state: &Rc<AppState>, clip_id: Uuid, edge: TrimEdge, delta_us: i64) -> bool {
    let other_edge = if edge == TrimEdge::Out { TrimEdge::In } else { TrimEdge::Out };
    // (clip, neighbour) pairs: the dragged clip, then any linked clips that also have a neighbour.
    let pairs: Vec<(Uuid, Uuid)> = state
        .with_timeline(|t| {
            let link = t.find_clip(clip_id).and_then(|(_, c)| c.properties.link).filter(|_| state.linked_selection.get());
            t.tracks
                .iter()
                .flat_map(|track| track.clips.iter().map(move |c| (track, c)))
                .filter(|(_, c)| c.id == clip_id || (link.is_some() && c.properties.link == link))
                .filter_map(|(track, c)| {
                    let touching = match edge {
                        TrimEdge::Out => track.clips.iter().find(|n| n.timeline_in == c.timeline_out),
                        TrimEdge::In => track.clips.iter().find(|n| n.timeline_out == c.timeline_in),
                    };
                    touching.map(|n| (c.id, n.id))
                })
                .collect()
        })
        .unwrap_or_default();
    if !pairs.iter().any(|(c, _)| *c == clip_id) {
        return false;
    }
    let mut commands: Vec<Box<dyn Command>> = Vec::new();
    for (clip, neighbour) in pairs {
        // Shrink first, then grow into the freed space, so the two never overlap.
        let clip_grows = (edge == TrimEdge::Out) == (delta_us > 0);
        let (shrink, grow) = if clip_grows { ((neighbour, other_edge), (clip, edge)) } else { ((clip, edge), (neighbour, other_edge)) };
        commands.push(Box::new(TrimClipCommand::new(shrink.0, shrink.1, delta_us)));
        commands.push(Box::new(TrimClipCommand::new(grow.0, grow.1, delta_us)));
    }
    state.execute(Box::new(CompositeCommand::new("Roll", commands)))
}

/// Move the selected clips one track up (`+1`) or down (`-1`). The clips of
/// the clicked clip's kind move; a picture's sound stays on its own track.
pub fn move_track(state: &Rc<AppState>, step: i32) {
    let Some((track, _)) = selected(state) else {
        state.message("Select a clip on the timeline first.");
        return;
    };
    let kind = state.with_timeline(|t| t.find_track(track).map(|tr| tr.kind)).flatten();
    if let Some(kind) = kind {
        move_selected(state, 0, Some((kind, step)));
    }
}

// ---- Markers ---------------------------------------------------------------

pub fn marker_at_playhead(state: &AppState) -> Option<Marker> {
    let (pos, frame) = (state.playhead_us(), state.frame_us());
    state.with_timeline(|t| marker_on_frame(&t.markers, pos, frame).cloned()).flatten()
}

/// The marker on the same frame as `pos`.
pub fn marker_on_frame(markers: &[Marker], pos: i64, frame_us: i64) -> Option<&Marker> {
    markers.iter().find(|m| (m.position_us - pos).abs() < frame_us)
}

/// "Marker N" for a new marker: one more than the markers already there, and not a name in use.
pub fn default_marker_name(others: &[Marker]) -> String {
    (others.len() + 1..)
        .map(|n| format!("Marker {n}"))
        .find(|name| !others.iter().any(|m| &m.name == name))
        .unwrap_or_default()
}

/// `named`: Ctrl+M (the popup) names it "Marker N" from the start, as Resolve does; M and
/// the Marker button leave it unnamed, an editing note that is never exported as a chapter.
pub fn add_marker(state: &Rc<AppState>, named: bool) -> Option<Marker> {
    if let Some(existing) = marker_at_playhead(state) {
        return Some(existing);
    }
    let others = state.with_timeline(|t| t.markers.clone()).unwrap_or_default();
    let name = if named { default_marker_name(&others) } else { String::new() };
    let marker = Marker::new(state.frame_floor(state.playhead_us()), name, MarkerColor::Blue);
    state.execute(Box::new(AddMarkerCommand::new(marker.clone()))).then_some(marker)
}

pub fn delete_marker(state: &Rc<AppState>, id: Uuid) {
    state.execute(Box::new(DeleteMarkerCommand::new(id)));
}

/// Replace a marker with an edited copy as one undo step.
pub fn update_marker(state: &Rc<AppState>, marker: Marker) {
    let cmds: Vec<Box<dyn Command>> =
        vec![Box::new(DeleteMarkerCommand::new(marker.id)), Box::new(AddMarkerCommand::new(marker))];
    state.execute(Box::new(CompositeCommand::new("Modify Marker", cmds)));
}

pub fn goto_marker(state: &Rc<AppState>, forward: bool) {
    let pos = state.player.position_us();
    let target = state
        .with_timeline(|t| {
            if forward {
                t.markers.iter().map(|m| m.position_us).find(|p| *p > pos)
            } else {
                t.markers.iter().rev().map(|m| m.position_us).find(|p| *p < pos)
            }
        })
        .flatten();
    if let Some(t) = target {
        state.player.pause();
        state.player.seek(t, true);
    }
}

// ---- Navigation and marks ----------------------------------------------------

/// Up / Down (Resolve's Previous / Next Edit), measured from the playhead. With an edit
/// point or a transition selected, the playhead goes to the neighbouring edit on its
/// track and that edit
/// becomes the selection, so Up, Ctrl+V, Up, Ctrl+V… pastes on every cut. Otherwise
/// the playhead goes to the next or previous cut on any track.
pub fn goto_edit(state: &Rc<AppState>, forward: bool) {
    // Previous / Next Edit are timeline commands: they count from the timeline playhead and
    // move it, whatever the viewer shows. A selected edit or transition only says which
    // track to follow; without one every track counts.
    let at = state.playhead_us();
    let track = match state.edit_selection.get() {
        Some(EditSelection::Edit { track_id, .. } | EditSelection::Transition { track_id, .. }) => Some(track_id),
        None => None,
    };
    let next = state
        .with_timeline(|t| {
            let points: Vec<i64> = match track {
                Some(id) => t.find_track(id)?.edit_points().into_iter().map(|e| e.position_us).collect(),
                None => t.tracks.iter().flat_map(|tr| tr.clips.iter()).flat_map(|c| [c.timeline_in, c.timeline_out]).collect(),
            };
            neighbour_edit(points, at, t.duration_us(), forward)
        })
        .flatten();
    let Some(pos) = next else { return };
    if let Some(track_id) = track {
        state.select_edit(Some(EditSelection::Edit { track_id, position_us: pos }));
    }
    state.player.pause();
    state.player.seek_timeline(pos);
}

pub fn step_frames(state: &Rc<AppState>, player: &Player, frames: i64) {
    let target = state.frame_step(player.position_us(), frames);
    player.pause();
    player.seek(target.min(player.duration_us()), true);
}

/// Set (or clear) In / Out at `player`'s position. With `source` the marks belong to the source clip.
pub fn set_mark(state: &Rc<AppState>, player: &Player, source: bool, is_in: bool, clear: bool) {
    let value = (!clear).then(|| player.position_us());
    match (source, is_in) {
        (true, true) => state.src_in.set(value),
        (true, false) => state.src_out.set(value),
        (false, true) => state.mark_in.set(value),
        (false, false) => state.mark_out.set(value),
    }
    state.emit(Change::Options);
}

/// Mark In/Out around the clip under the playhead on the destination video track.
pub fn mark_clip(state: &Rc<AppState>) {
    let pos = state.playhead_us();
    let Some(track) = state.dest_track(TrackKind::Video) else { return };
    let range = state
        .with_timeline(|t| t.find_track(track).and_then(|tr| tr.clip_at(pos)).map(|c| (c.timeline_in, c.timeline_out)))
        .flatten();
    if let Some((a, b)) = range {
        state.mark_in.set(Some(a));
        state.mark_out.set(Some(b));
        state.emit(Change::Options);
    }
}

/// Mark In/Out around the span of the currently selected clips (`Shift+A`).
pub fn mark_selection(state: &Rc<AppState>) {
    let ids = state.selected_ids();
    let range = state
        .with_timeline(|t| {
            let mut lo = i64::MAX;
            let mut hi = i64::MIN;
            for id in &ids {
                if let Some((_, c)) = t.find_clip(*id) {
                    lo = lo.min(c.timeline_in);
                    hi = hi.max(c.timeline_out);
                }
            }
            (lo < hi).then_some((lo, hi))
        })
        .flatten();
    match range {
        Some((a, b)) => {
            state.mark_in.set(Some(a));
            state.mark_out.set(Some(b));
            state.emit(Change::Options);
        }
        None => state.message("Select a clip on the timeline first."),
    }
}

/// Select the edit point nearest the playhead on an unlocked track (`V`).
pub fn select_nearest_edit(state: &Rc<AppState>) {
    let pos = state.playhead_us();
    let dest_v = state.dest_track(TrackKind::Video);
    let dest_a = state.dest_track(TrackKind::Audio);
    let found = state
        .with_timeline(|t| {
            t.tracks
                .iter()
                .filter(|tr| !tr.locked)
                .flat_map(|tr| {
                    let prio = (Some(tr.id) != dest_v, tr.kind != TrackKind::Video, Some(tr.id) != dest_a, tr.kind_index);
                    tr.edit_points().into_iter().map(move |e| ((e.position_us - pos).abs(), prio, tr.id, e.position_us))
                })
                .min_by_key(|(dist, prio, _, _)| (*dist, *prio))
                .map(|(_, _, track_id, position_us)| (track_id, position_us))
        })
        .flatten();
    match found {
        Some((track_id, position_us)) => state.select_edit(Some(EditSelection::Edit { track_id, position_us })),
        None => state.message("There is no edit point on an unlocked track."),
    }
}

/// Select clips forward (`Y` / `Alt+Y`) or backward (`Ctrl+Y` / `Ctrl+Alt+Y`) from the playhead.
pub fn select_range_relative(state: &Rc<AppState>, forward: bool, all_tracks: bool) {
    let pos = state.playhead_us();
    let ids = state
        .with_timeline(|t| {
            let target_track = if all_tracks {
                None
            } else {
                match state.edit_selection.get() {
                    Some(EditSelection::Edit { track_id, .. } | EditSelection::Transition { track_id, .. }) => Some(track_id),
                    None => state
                        .selection
                        .get()
                        .and_then(|id| t.find_clip(id).map(|(tr, _)| tr.id))
                        .or_else(|| state.dest_track(TrackKind::Video))
                        .or_else(|| state.dest_track(TrackKind::Audio)),
                }
            };
            let mut matched: Vec<(i64, u32, Uuid)> = t
                .tracks
                .iter()
                .filter(|tr| !tr.locked && target_track.is_none_or(|id| tr.id == id))
                .flat_map(|tr| {
                    tr.clips.iter().filter_map(move |c| {
                        let hit = if forward { c.timeline_out > pos } else { c.timeline_in < pos };
                        hit.then_some((c.timeline_in, tr.sort_order, c.id))
                    })
                })
                .collect();
            matched.sort_by_key(|(tin, order, _)| (*tin, *order));
            matched.into_iter().map(|(_, _, id)| id).collect::<Vec<_>>()
        })
        .unwrap_or_default();
    // Linked Selection: each matched clip brings its linked clips (it stays first, so the primary is on the track).
    let ids: Vec<Uuid> = ids.into_iter().flat_map(|id| std::iter::once(id).chain(state.linked_to(id))).collect();
    state.select_ids(ids);
}

/// Jump to the first (`last = false`) or last (`last = true`) frame of the clip under the playhead.
pub fn first_or_last_frame(state: &AppState, player: &Player, source: bool, last: bool) {
    if source {
        let target = if last { state.frame_step(player.duration_us(), -1).max(0) } else { 0 };
        player.pause();
        player.seek(target, true);
        return;
    }
    let pos = state.playhead_us();
    let primary = state.selection.get();
    let dest = state.dest_track(TrackKind::Video);
    let span = state
        .with_timeline(|t| {
            if let Some(id) = primary {
                if let Some((_, c)) = t.find_clip(id) {
                    if pos >= c.timeline_in && pos <= c.timeline_out {
                        return Some((c.timeline_in, c.timeline_out));
                    }
                }
            }
            let mut tracks: Vec<_> = t.tracks.iter().collect();
            tracks.sort_by_key(|tr| (Some(tr.id) != dest, tr.kind != TrackKind::Video, std::cmp::Reverse(tr.kind_index)));
            tracks
                .iter()
                .find_map(|tr| tr.clip_at(pos).map(|c| (c.timeline_in, c.timeline_out)))
                .or_else(|| tracks.iter().find_map(|tr| tr.clips.iter().find(|c| c.timeline_out == pos).map(|c| (c.timeline_in, c.timeline_out))))
        })
        .flatten();
    if let Some((cin, cout)) = span {
        let target = if last { state.frame_step(cout, -1).max(cin) } else { cin };
        state.player.pause();
        state.player.seek(target, true);
    }
}

/// Play In to Out (`Alt+/`).
pub fn play_in_to_out(state: &AppState, player: &Player, source: bool) -> bool {
    let (in_us, out_us) = if source {
        (state.src_in.get(), state.src_out.get())
    } else {
        (state.mark_in.get(), state.mark_out.get())
    };
    let dur = player.duration_us();
    if in_us.is_none() && out_us.is_none() {
        state.message("Mark In and Out first (I and O).");
        return false;
    }
    let start = in_us.unwrap_or(0).clamp(0, dur);
    let end = out_us.unwrap_or(dur).clamp(0, dur);
    if end <= start {
        state.message("Out point must be after In point.");
        return false;
    }
    player.play_range(1.0, start, end);
    true
}

const PLAY_AROUND_ROLL_US: i64 = 2_000_000;

/// Timeline/source `(start_us, end_us)` around the selection, edit point, or playhead (`/`).
pub fn play_around_bounds(state: &AppState, player: &Player, source: bool) -> Option<(i64, i64)> {
    let dur = player.duration_us();
    if dur <= 0 {
        return None;
    }
    if source {
        let pos = player.position_us();
        let start = (pos - PLAY_AROUND_ROLL_US).max(0);
        let end = (pos + PLAY_AROUND_ROLL_US).min(dur);
        return (end > start).then_some((start, end));
    }
    let selected = state.selected_ids();
    let range = state.with_timeline(|t| {
        if !selected.is_empty() {
            let mut lo = i64::MAX;
            let mut hi = i64::MIN;
            for id in &selected {
                if let Some((_, c)) = t.find_clip(*id) {
                    lo = lo.min(c.timeline_in);
                    hi = hi.max(c.timeline_out);
                }
            }
            if lo < hi {
                return (lo, hi);
            }
        }
        match state.edit_selection.get() {
            Some(EditSelection::Edit { position_us, .. }) => (position_us, position_us),
            Some(EditSelection::Transition { track_id, clip_id, edge }) => t
                .find_track(track_id)
                .and_then(|tr| tr.transition_at_edit(clip_id, edge).and_then(|x| tr.transition_span(x)))
                .unwrap_or((state.playhead_us(), state.playhead_us())),
            None => (state.playhead_us(), state.playhead_us()),
        }
    });
    let (lo, hi) = range?;
    let start = (lo - PLAY_AROUND_ROLL_US).max(0);
    let end = (hi + PLAY_AROUND_ROLL_US).min(dur);
    (end > start).then_some((start, end))
}

/// Play around current selection, edit point, or playhead (`/`).
pub fn play_around(state: &AppState, player: &Player, source: bool) -> bool {
    let Some((start, end)) = play_around_bounds(state, player, source) else { return false };
    player.play_range(1.0, start, end);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_names_count_up_and_skip_taken_ones() {
        let m = |name: &str| Marker::new(0, name, MarkerColor::Blue);
        assert_eq!(default_marker_name(&[]), "Marker 1");
        assert_eq!(default_marker_name(&[m("Marker 1"), m("Section")]), "Marker 3");
        assert_eq!(default_marker_name(&[m("Marker 2"), m("Intro")]), "Marker 3");
        assert_eq!(default_marker_name(&[m("Marker 2")]), "Marker 3");
        assert_eq!(default_marker_name(&[m("Marker 1"), m("Marker 3")]), "Marker 4");
    }

    #[test]
    fn marker_is_found_only_on_the_same_frame() {
        let markers = [Marker::new(1_000_000, "a", MarkerColor::Red)];
        assert!(marker_on_frame(&markers, 1_020_000, 33_333).is_some());
        assert!(marker_on_frame(&markers, 1_040_000, 33_333).is_none());
    }

    const S: i64 = 1_000_000;
    const FPS: RationalFps = RationalFps::FPS_25;

    /// A video track with A [0, 4 s] (footage 1-5 s) and B [4, 8 s] (footage `b_in` on),
    /// both from 10 s files: A has 5 s of tail handle, B `b_in` of head handle.
    fn cut(b_in: i64) -> (Track, HashMap<Uuid, MediaSource>) {
        let mut track = Track::new(TrackKind::Video, 1, "Video 1", 0);
        let mut sources = HashMap::new();
        for (name, tin, sin) in [("a", 0, S), ("b", 4 * S, b_in)] {
            let src = MediaSource::new(std::path::PathBuf::from(name), MediaType::Video, 10 * S);
            track.clips.push(Clip::new(track.id, src.id, ClipType::Video, name, tin, tin + 4 * S, sin, sin + 4 * S));
            sources.insert(src.id, src);
        }
        (track, sources)
    }

    fn standard(d: i64) -> Transition {
        Transition::new(Uuid::nil(), TransitionEdge::In, TransitionKind::DipToColor, d, TransitionAlignment::EndAtCut)
    }

    fn fit(track: &Track, sources: &HashMap<Uuid, MediaSource>, at: i64, d: i64) -> Result<(TransitionAlignment, i64, TransitionEdge), &'static str> {
        let edit = track.edit_at(at).expect("an edit");
        fit_transition(track, &edit, &standard(d), sources, FPS).map(|t| (t.alignment, t.duration_us, t.edge))
    }

    #[test]
    fn a_transition_is_centred_on_a_cut_when_the_handles_allow() {
        let (track, sources) = cut(2 * S);
        assert_eq!(fit(&track, &sources, 4 * S, S), Ok((TransitionAlignment::Centered, S, TransitionEdge::In)));
        // The kind, colour and ease come from the template; the clip edge from the edit.
        let t = fit_transition(&track, &track.edit_at(4 * S).unwrap(), &standard(S), &sources, FPS).unwrap();
        assert_eq!((t.kind, t.clip_id), (TransitionKind::DipToColor, track.clips[1].id));
    }

    #[test]
    fn trim_without_a_selection_takes_the_clips_under_the_playhead_on_unlocked_tracks() {
        let (track, _) = cut(2 * S);
        let mut locked = track.clone();
        locked.id = Uuid::new_v4();
        locked.locked = true;
        let mut timeline = Timeline::new();
        timeline.tracks = vec![track, locked];
        let under = clips_under(&timeline, 5 * S);
        assert_eq!(under.len(), 1);
        assert_eq!(under[0].1.timeline_in, 4 * S);
        // On a cut the playhead is inside neither clip.
        assert!(clips_under(&timeline, 4 * S).is_empty());
    }

    #[test]
    fn add_transition_on_a_short_clip_fits_its_two_transitions_together() {
        // A [0, 4 s], a 0.6 s B, C after it: Ctrl+T on B puts a transition on both its cuts.
        let (mut track, mut sources) = cut(2 * S);
        track.clips[1].timeline_out = 4_600_000;
        track.clips[1].source_out = 2_600_000;
        let src = MediaSource::new(std::path::PathBuf::from("c"), MediaType::Video, 10 * S);
        track.clips.push(Clip::new(track.id, src.id, ClipType::Video, "c", 4_600_000, 8 * S, 2 * S, 5_400_000));
        sources.insert(src.id, src);
        let id = track.id;
        let mut timeline = Timeline::new();
        timeline.tracks = vec![track];
        let wanted = [(id, 4 * S, TransitionKind::CrossDissolve), (id, 4_600_000, TransitionKind::CrossDissolve)];
        let (placed, _) = fill_edits(&timeline, &sources, FPS, &wanted, tempo_timeline::STANDARD_TRANSITION_US);
        assert_eq!(placed.len(), 2);
        // What is stored is what is drawn: put both on, and each span is as long as its duration.
        let track = &mut timeline.tracks[0];
        track.transitions = placed.iter().map(|(_, t)| t.clone()).collect();
        for t in &track.transitions {
            let (start, end) = track.transition_span(t).expect("a span");
            assert_eq!(end - start, t.duration_us, "{t:?}");
        }
    }

    #[test]
    fn short_handles_move_the_transition_off_centre_then_shorten_it() {
        // No footage before B's In point: only "start on edit" fits.
        let (track, sources) = cut(0);
        assert_eq!(fit(&track, &sources, 4 * S, S), Ok((TransitionAlignment::StartAtCut, S, TransitionEdge::In)));
        // Half a second of head: a 3 s transition does not fit centred (1 s at most) but starts on the cut.
        let (track, sources) = cut(S / 2);
        assert_eq!(fit(&track, &sources, 4 * S, 3 * S), Ok((TransitionAlignment::StartAtCut, 3 * S, TransitionEdge::In)));
        // 6 s fits nowhere: the alignment with the most room, shortened (B is 4 s long).
        assert_eq!(fit(&track, &sources, 4 * S, 6 * S), Ok((TransitionAlignment::StartAtCut, 4 * S, TransitionEdge::In)));
    }

    #[test]
    fn no_footage_past_either_clip_refuses_the_transition() {
        let (track, mut sources) = cut(0);
        // A ends where its file ends.
        sources.get_mut(&track.clips[0].source_id).unwrap().duration_us = 5 * S;
        assert!(fit(&track, &sources, 4 * S, S).is_err());
    }

    #[test]
    fn heads_and_tails_get_their_transition_inside_the_clip() {
        let (track, sources) = cut(2 * S);
        assert_eq!(fit(&track, &sources, 0, S), Ok((TransitionAlignment::StartAtCut, S, TransitionEdge::In)));
        assert_eq!(fit(&track, &sources, 8 * S, S), Ok((TransitionAlignment::EndAtCut, S, TransitionEdge::Out)));
        // Lengths are whole frames (25 fps: 40 ms).
        assert_eq!(fit(&track, &sources, 8 * S, 1_010_000).map(|f| f.1), Ok(S));
    }

    #[test]
    fn a_drop_goes_to_the_nearest_edit_in_reach() {
        let (track, _) = cut(2 * S);
        let at = |us, reach| nearest_edit(&track, us, reach).map(|e| (e.position_us, e.kind));
        assert_eq!(at(3_900_000, 200_000), Some((4 * S, EditKind::Cut)));
        // Over the middle of a clip: the nearer of its own edges, whatever the reach.
        assert_eq!(at(S, 0), Some((0, EditKind::Head)));
        assert_eq!(at(7 * S, 0), Some((8 * S, EditKind::Tail)));
        // Past the end of the last clip: only within reach.
        assert_eq!(at(9 * S, S / 2), None);
        assert_eq!(at(9 * S, 3 * S / 2), Some((8 * S, EditKind::Tail)));
    }

    #[test]
    fn a_copied_transition_is_placed_by_the_edit_it_lands_on_not_the_timeline_it_came_from() {
        let (track, sources) = cut(2 * S);
        let mut timeline = Timeline::new();
        let track_id = track.id;
        let b_id = track.clips[1].id;
        timeline.tracks.push(track);
        // Copied on another timeline: its clip id means nothing here.
        let copied = Transition::new(Uuid::new_v4(), TransitionEdge::Out, TransitionKind::DipToColor, S, TransitionAlignment::EndAtCut);
        let t = place_on(&timeline, &sources, FPS, track_id, 4 * S, &copied).expect("fits");
        assert_eq!((t.clip_id, t.kind.clone(), t.duration_us), (b_id, TransitionKind::DipToColor, S));
        // An audio transition does not go on a video track.
        let fade = Transition::new(Uuid::new_v4(), TransitionEdge::In, TransitionKind::Crossfade, S, TransitionAlignment::Centered);
        assert_eq!(place_on(&timeline, &sources, FPS, track_id, 4 * S, &fade), Err("Audio transitions go on audio tracks."));
    }

    #[test]
    fn up_and_down_step_through_edits_and_the_timeline_ends() {
        let points = [0, 4 * S, 8 * S];
        assert_eq!(neighbour_edit(points, 4 * S, 10 * S, false), Some(0));
        assert_eq!(neighbour_edit(points, 4 * S, 10 * S, true), Some(8 * S));
        // The end of the timeline counts as an edit, even past this track's last clip.
        assert_eq!(neighbour_edit(points, 8 * S, 10 * S, true), Some(10 * S));
        assert_eq!(neighbour_edit(points, 10 * S, 10 * S, true), None);
        assert_eq!(neighbour_edit(points, 0, 10 * S, false), None);
        // A track starting later still lets Up reach the timeline start.
        assert_eq!(neighbour_edit([2 * S], 2 * S, 10 * S, false), Some(0));
    }

    #[test]
    fn dragging_an_edge_changes_the_length_in_whole_frames() {
        // The left edge dragged 0.2 s left: 0.2 s longer, or 0.4 s when centred.
        assert_eq!(dragged_length(S, -200_000, true, false, 3 * S, FPS), 1_200_000);
        assert_eq!(dragged_length(S, -200_000, true, true, 3 * S, FPS), 1_400_000);
        // The right edge dragged right lengthens, left shortens.
        assert_eq!(dragged_length(S, 200_000, false, false, 3 * S, FPS), 1_200_000);
        assert_eq!(dragged_length(S, -200_000, false, true, 3 * S, FPS), 600_000);
        // A quarter of a frame rounds away.
        assert_eq!(dragged_length(S, 10_000, false, false, 3 * S, FPS), S);
        // At least one frame, at most what the handles allow.
        assert_eq!(dragged_length(S, 5 * S, true, false, 3 * S, FPS), 40_000);
        assert_eq!(dragged_length(S, -5 * S, true, true, 1_500_000, FPS), 1_500_000);
    }

    #[test]
    fn an_edit_takes_its_linked_tracks_along() {
        let mut t = Timeline::new_default();
        let (v1, a1, v2) = (t.tracks[0].id, t.tracks[2].id, t.tracks[1].id);
        let link = Some(Uuid::new_v4());
        let mut pic = Clip::new(v1, Uuid::new_v4(), ClipType::Video, "p", 0, 4 * S, 0, 4 * S);
        let mut snd = Clip::new(a1, pic.source_id, ClipType::Audio, "s", 0, 4 * S, 0, 4 * S);
        (pic.properties.link, snd.properties.link) = (link, link);
        t.tracks[0].clips.push(pic);
        t.tracks[2].clips.push(snd);
        t.tracks[1].clips.push(Clip::new(v2, Uuid::new_v4(), ClipType::Video, "o", 0, 4 * S, 0, 4 * S));
        assert_eq!(edit_group(&t, v1, 4 * S, true), vec![(v1, TrackKind::Video), (a1, TrackKind::Audio)]);
        assert_eq!(edit_group(&t, a1, 0, true), vec![(a1, TrackKind::Audio), (v1, TrackKind::Video)]);
        assert_eq!(edit_group(&t, v1, 4 * S, false), vec![(v1, TrackKind::Video)]);
    }

    #[test]
    fn only_a_marker_added_for_the_popup_gets_a_default_name() {
        // Arrange: unnamed markers are editing notes and never become chapters (UI_SPEC 7.4).
        let state = AppState::new();
        *state.project.borrow_mut() = Some(tempo_timeline::Project::new("t", 1920, 1080, tempo_timeline::RationalFps::FPS_30));

        // Act
        let note = add_marker(&state, false).map(|m| m.name);
        state.player.seek_timeline(1_000_000);
        let popup = add_marker(&state, true).map(|m| m.name);

        // Assert
        assert_eq!(note.as_deref(), Some(""));
        assert_eq!(popup.as_deref(), Some("Marker 2"));
    }

    #[test]
    fn a_source_is_placed_whole_when_the_marks_are_not_to_be_used() {
        // Arrange: a 10 s source with In/Out marks on it in the viewer.
        let state = AppState::new();
        let mut project = tempo_timeline::Project::new("t", 1920, 1080, tempo_timeline::RationalFps::FPS_30);
        let src = MediaSource::new(std::path::PathBuf::from("a"), MediaType::Video, 10 * S);
        let (id, track) = (src.id, project.timeline.tracks.iter().find(|t| t.kind == TrackKind::Video).map(|t| t.id));
        project.sources.insert(id, src);
        *state.project.borrow_mut() = Some(project);
        state.source_clip.set(Some(id));
        state.src_in.set(Some(2 * S));
        state.src_out.set(Some(5 * S));

        // Act
        let marked = clips_for_source_marked(&state, id, 0, track, None, true);
        let whole = clips_for_source_marked(&state, id, 0, track, None, false);

        // Assert
        assert_eq!((marked[0].source_in, marked[0].source_out), (2 * S, 5 * S));
        assert_eq!((whole[0].source_in, whole[0].source_out), (0, 10 * S));
    }

    #[test]
    fn marks_on_a_conformed_source_are_read_back_in_file_time() {
        // Arrange: a 10 s 60 fps file conformed to 24 fps (0.4x) plays 25 s long in the viewer.
        let state = AppState::new();
        let mut project = tempo_timeline::Project::new("t", 1920, 1080, RationalFps::FPS_30);
        let mut src = MediaSource::new(std::path::PathBuf::from("a"), MediaType::Video, 10 * S);
        src.video_fps = Some(RationalFps::FPS_60);
        src.conform_fps = Some(RationalFps::FPS_24);
        let (id, track) = (src.id, project.timeline.tracks.iter().find(|t| t.kind == TrackKind::Video).map(|t| t.id));
        project.sources.insert(id, src);
        *state.project.borrow_mut() = Some(project);
        state.source_clip.set(Some(id));
        state.src_in.set(Some(5 * S));
        state.src_out.set(Some(7 * S));

        // Act
        let clips = clips_for_source_marked(&state, id, 0, track, None, true);

        // Assert: viewer 5..7 s is file 2..2.8 s, and it lasts 2 s on the timeline.
        assert_eq!((clips[0].source_in, clips[0].source_out), (2 * S, 2_800_000));
        assert_eq!(clips[0].duration_us(), 2 * S);
    }

    fn setup_two_track_project() -> (Rc<AppState>, Uuid, Uuid) {
        let state = AppState::new();
        let mut project = tempo_timeline::Project::new("t", 1920, 1080, RationalFps::FPS_30);
        let v1 = project.timeline.tracks.iter().find(|t| t.kind == TrackKind::Video && t.kind_index == 1).unwrap().id;
        let a1 = project.timeline.tracks.iter().find(|t| t.kind == TrackKind::Audio && t.kind_index == 1).unwrap().id;
        let src = MediaSource::new(std::path::PathBuf::from("clip.mp4"), MediaType::Video, 12 * S);
        for (i, (tin, sin)) in [(0, 0), (4 * S, 4 * S), (8 * S, 8 * S)].into_iter().enumerate() {
            let link = Some(Uuid::new_v4());
            let mut vc = Clip::new(v1, src.id, ClipType::Video, format!("v{i}"), tin, tin + 4 * S, sin, sin + 4 * S);
            let mut ac = Clip::new(a1, src.id, ClipType::Audio, format!("a{i}"), tin, tin + 4 * S, sin, sin + 4 * S);
            vc.properties.link = link;
            ac.properties.link = link;
            project.timeline.find_track_mut(v1).unwrap().clips.push(vc);
            project.timeline.find_track_mut(a1).unwrap().clips.push(ac);
        }
        project.sources.insert(src.id, src);
        state.set_project(Some(project), None);
        (state, v1, a1)
    }

    #[test]
    fn replace_reads_the_in_mark_of_a_conformed_source_in_file_time() {
        // Arrange: a 10 s 60 fps file conformed to 24 fps, In marked at 5 s of viewer time.
        let (state, v1, _) = setup_two_track_project();
        let mut src = MediaSource::new(std::path::PathBuf::from("b.mp4"), MediaType::Video, 10 * S);
        src.video_fps = Some(RationalFps::FPS_60);
        src.conform_fps = Some(RationalFps::FPS_24);
        let sid = src.id;
        state.project.borrow_mut().as_mut().unwrap().sources.insert(sid, src);
        state.source_clip.set(Some(sid));
        state.src_in.set(Some(5 * S));
        let target = state.with_timeline(|t| t.find_track(v1).unwrap().clips[0].id).unwrap();
        state.select(Some(target));

        // Act
        replace_selected(&state);

        // Assert
        let clip = state.with_timeline(|t| t.find_clip(target).unwrap().1.clone()).unwrap();
        assert_eq!((clip.source_id, clip.source_in), (sid, 2 * S));
    }

    #[test]
    fn split_without_selection_cuts_under_playhead_and_join_reunites_with_undo() {
        let (state, v1, a1) = setup_two_track_project();
        state.player.seek_timeline(2 * S);
        split_selected(&state);
        assert_eq!(state.with_timeline(|t| t.find_track(v1).unwrap().clips.len()), Some(4));
        assert_eq!(state.with_timeline(|t| t.find_track(a1).unwrap().clips.len()), Some(4));

        // Alt+\ with nothing selected joins the continuous split at the playhead in one undo step.
        join_clip(&state);
        assert_eq!(state.with_timeline(|t| t.find_track(v1).unwrap().clips.len()), Some(3));
        assert_eq!(state.with_timeline(|t| t.find_track(a1).unwrap().clips.len()), Some(3));
        state.undo();
        assert_eq!(state.with_timeline(|t| t.find_track(v1).unwrap().clips.len()), Some(4));
        assert_eq!(state.with_timeline(|t| t.find_track(a1).unwrap().clips.len()), Some(4));
    }

    #[test]
    fn track_and_all_selection_follow_playhead_and_mark_selection_sets_in_out() {
        let (state, v1, a1) = setup_two_track_project();
        state.player.seek_timeline(4 * S);
        let on = |track: Uuid, id: Uuid| state.with_timeline(|t| t.find_track(track).unwrap().find_clip(id).is_some()).unwrap();
        // Linked Selection (on by default): Y takes the 2 V1 clips and their A1 sound.
        select_range_relative(&state, true, false);
        let sel = state.selected_ids();
        assert_eq!(sel.len(), 4);
        assert!(on(v1, sel[0]));
        assert_eq!(sel.iter().filter(|id| on(v1, **id)).count(), 2);

        state.linked_selection.set(false);
        select_range_relative(&state, true, false);
        let sel = state.selected_ids();
        assert_eq!(sel.len(), 2);
        assert!(sel.iter().all(|id| on(v1, *id)));
        state.linked_selection.set(true);

        select_range_relative(&state, true, true);
        assert_eq!(state.selected_ids().len(), 4);

        select_range_relative(&state, false, false);
        let sel = state.selected_ids();
        assert_eq!(sel.len(), 2);
        assert!(on(v1, sel[0]) && on(a1, sel[1]));

        select_range_relative(&state, false, true);
        assert_eq!(state.selected_ids().len(), 2);
        assert!(state.with_timeline(|t| t.find_track(a1).unwrap().find_clip(state.selected_ids()[1]).is_some()).unwrap());

        mark_selection(&state);
        assert_eq!((state.mark_in.get(), state.mark_out.get()), (Some(0), Some(4 * S)));
    }

    #[test]
    fn select_nearest_edit_swap_paste_insert_volume_and_video_track_toggle() {
        let (state, v1, a1) = setup_two_track_project();
        state.player.seek_timeline(4_100_000);
        select_nearest_edit(&state);
        assert_eq!(state.edit_selection.get(), Some(EditSelection::Edit { track_id: v1, position_us: 4 * S }));

        // Swap first clip forward with second clip on V1.
        let first_id = state.with_timeline(|t| t.find_track(v1).unwrap().clips[0].id).unwrap();
        state.select(Some(first_id));
        swap_clips(&state, true);
        assert_eq!(state.with_timeline(|t| t.find_clip(first_id).unwrap().1.timeline_in), Some(4 * S));
        state.undo();
        assert_eq!(state.with_timeline(|t| t.find_clip(first_id).unwrap().1.timeline_in), Some(0));

        // Paste insert pushes later clips right by clip length.
        state.select(Some(first_id));
        copy_selected(&state, false);
        state.player.seek_timeline(4 * S);
        paste_insert(&state);
        assert_eq!(state.with_timeline(|t| t.find_track(v1).unwrap().clips.len()), Some(4));
        assert_eq!(state.with_timeline(|t| t.duration_us()), Some(16 * S));
        state.undo();
        assert_eq!(state.with_timeline(|t| t.duration_us()), Some(12 * S));

        // Adjust volume +3 dB only modifies Audio clip in linked selection.
        let audio_id = state.with_timeline(|t| t.find_track(a1).unwrap().clips[0].id).unwrap();
        state.select(Some(first_id));
        adjust_volume_db(&state, 3.0);
        let vol = state.with_timeline(|t| t.find_clip(audio_id).unwrap().1.properties.volume).unwrap();
        assert!((vol - 10.0f32.powf(0.15)).abs() < 1e-4);
        // Never past the Inspector's 200% ceiling.
        for _ in 0..5 {
            adjust_volume_db(&state, 3.0);
        }
        assert_eq!(state.with_timeline(|t| t.find_clip(audio_id).unwrap().1.properties.volume), Some(2.0));
        // A clip already above 200% is not turned down by Up, and Down still lowers it.
        state.execute(Box::new(SetClipPropertyCommand::new(audio_id, PropertyChange::Volume(3.0))));
        adjust_volume_db(&state, 3.0);
        assert_eq!(state.with_timeline(|t| t.find_clip(audio_id).unwrap().1.properties.volume), Some(3.0));
        adjust_volume_db(&state, -3.0);
        let vol = state.with_timeline(|t| t.find_clip(audio_id).unwrap().1.properties.volume).unwrap();
        assert!(vol < 3.0);

        // Toggle video track 1 enabled state and undo.
        toggle_video_track(&state, 1);
        assert_eq!(state.with_timeline(|t| t.find_track(v1).unwrap().enabled), Some(false));
        state.undo();
        assert_eq!(state.with_timeline(|t| t.find_track(v1).unwrap().enabled), Some(true));
    }

    #[test]
    fn clip_attributes_on_source_propagate_to_placed_clips() {
        let state = AppState::new();
        let mut project = tempo_timeline::Project::new("t", 1920, 1080, RationalFps::FPS_30);
        let mut src = MediaSource::new(std::path::PathBuf::from("/tmp/raw.mp4"), MediaType::Video, 4 * S);
        src.video_fps = Some(RationalFps::FPS_60);
        src.audio_channels = Some(2);
        src.display_name = Some("B-Roll Slow".into());
        src.conform_fps = Some(RationalFps::FPS_30);
        src.audio_channel_map = tempo_timeline::AudioChannelMap::LeftToBoth;
        let sid = src.id;
        project.sources.insert(sid, src);
        state.set_project(Some(project), None);

        place_source(&state, sid, Place::Insert, Some(0), None, false);
        state.with_timeline(|t| {
            let vclip = &t.tracks.iter().find(|tr| tr.kind == TrackKind::Video && tr.kind_index == 1).unwrap().clips[0];
            assert_eq!(vclip.name, "B-Roll Slow");
            assert_eq!(vclip.duration_us(), 8 * S);
            assert!((vclip.properties.speed - 0.5).abs() < 1e-6);

            let aclip = &t.tracks.iter().find(|tr| tr.kind == TrackKind::Audio && tr.kind_index == 1).unwrap().clips[0];
            assert_eq!(aclip.name, "B-Roll Slow");
            assert_eq!(aclip.duration_us(), 8 * S);
            assert_eq!(aclip.properties.audio_channel_map, tempo_timeline::AudioChannelMap::LeftToBoth);
        });
    }
}
