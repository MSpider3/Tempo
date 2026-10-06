//! Editing actions. Each builds commands and sends them through `AppState::execute`,
//! so every one of them can be undone.

use std::rc::Rc;

use tempo_timeline::{
    AddMarkerCommand, Clip, ClipType, Command, CompositeCommand, DeleteClipCommand, DeleteMarkerCommand,
    ClipEffect, EditClipCommand, InsertClipCommand, Marker, MarkerColor, MediaType, MoveClipCommand, OverwriteClipCommand,
    PropertyChange, ReplaceClipCommand, SetClipPropertyCommand, SplitClipCommand, TitleData,
    TitleType, TrackKind, TrimClipCommand, TrimEdge,
};
use uuid::Uuid;

use crate::state::{AppState, Change, Gap};

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
fn clips_for_source(
    state: &AppState,
    source_id: Uuid,
    at_us: i64,
    video_track: Option<Uuid>,
    audio_track: Option<Uuid>,
) -> Vec<Clip> {
    let Some((media_type, duration, has_audio, name)) = state
        .with_project(|p| {
            p.sources.get(&source_id).map(|s| {
                let name = s.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                (s.media_type, s.duration_us, s.audio_channels.is_some(), name)
            })
        })
        .flatten()
    else {
        return Vec::new();
    };

    // Honour In/Out marks set on this clip in the viewer's source mode.
    let marked = state.source_clip.get() == Some(source_id);
    let full = if media_type == MediaType::Image { STILL_DURATION_US } else { duration.max(1) };
    let src_in = if marked { state.src_in.get().unwrap_or(0).clamp(0, full - 1) } else { 0 };
    let src_out = if marked { state.src_out.get().unwrap_or(full).clamp(src_in + 1, full) } else { full };
    let len = src_out - src_in;

    let mut clips = Vec::new();
    let mut push = |track: Option<Uuid>, kind: ClipType| {
        if let Some(track) = track {
            clips.push(Clip::new(track, source_id, kind, name.clone(), at_us, at_us + len, src_in, src_out));
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
/// `at_us` the place (a drop); without it the clip goes to the playhead.
pub fn place_source(state: &Rc<AppState>, source_id: Uuid, how: Place, at_us: Option<i64>, track: Option<Uuid>) {
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

    let mut clips = clips_for_source(state, source_id, at, video, audio);
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
        Some(id) => place_source(state, id, how, None, None),
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

/// Cross dissolve from the previous clip into each selected clip. The incoming
/// clip needs footage before its In point to dissolve from.
pub fn cross_dissolve(state: &Rc<AppState>, seconds: f64) {
    let wanted = (seconds.max(0.0) * 1_000_000.0) as i64;
    let frame = state.frame_us();
    let mut reason: Option<&str> = None;
    let mut commands: Vec<Box<dyn Command>> = Vec::new();
    for (track, clip) in all_selected(state) {
        // Length of the clip this one touches, if any.
        let previous = state
            .with_timeline(|t| t.find_track(track).and_then(|tr| tr.clips.iter().find(|c| c.timeline_out == clip.timeline_in).map(|c| c.duration_us())))
            .flatten();
        let Some(previous) = previous else {
            reason = Some("A cross dissolve needs a clip touching the start of the selected one.");
            continue;
        };
        let length = if wanted == 0 { 0 } else { wanted.min(clip.source_in).min(previous / 2) };
        if wanted > 0 && length < frame {
            reason = Some("This clip starts at the very beginning of its footage, so there is nothing to dissolve from. Trim its start a little first.");
            continue;
        }
        let mut edited = clip.clone();
        edited.properties.dissolve_in_us = length;
        if edited != clip {
            commands.push(Box::new(EditClipCommand::new("Cross Dissolve", edited)));
        }
    }
    if !commands.is_empty() {
        state.execute(Box::new(CompositeCommand::new("Cross Dissolve", commands)));
    } else if let Some(reason) = reason {
        state.message(reason);
    } else if state.selection.get().is_none() {
        state.message("Select the clip that should dissolve in.");
    }
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
    let earliest = copied.iter().map(|c| c.timeline_in).min().unwrap_or(0);
    let kinds: std::collections::HashSet<bool> = copied.iter().map(|c| c.clip_type == ClipType::Audio).collect();
    // A picture with its sound, or a single clip, follows the destination tracks.
    let to_destination = copied.len() == 1 || (copied.len() == 2 && kinds.len() == 2);
    let mut links: std::collections::HashMap<Uuid, Uuid> = Default::default();
    let mut pasted = Vec::new();
    for mut clip in copied {
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
    let Some(first) = pasted.first().map(|c| c.id) else { return };
    let end = pasted.iter().map(|c| c.timeline_out).max().unwrap_or(at);
    let commands: Vec<Box<dyn Command>> = pasted.into_iter().map(|c| Box::new(OverwriteClipCommand::new(c)) as Box<dyn Command>).collect();
    if state.execute(Box::new(CompositeCommand::new("Paste", commands))) {
        state.select(Some(first));
        state.player.seek_timeline(end);
    }
}

/// Replace the selected timeline clips with the current Media Pool clip, keeping
/// their place and length. A picture and its linked sound are both replaced.
pub fn replace_selected(state: &Rc<AppState>) {
    let Some(source) = current_source(state) else {
        state.message("Select a clip on the timeline and a clip in the Media Pool first.");
        return;
    };
    let Some((name, duration, media, has_audio)) = state
        .with_project(|p| {
            p.sources.get(&source).map(|s| {
                (s.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), s.duration_us, s.media_type, s.audio_channels.is_some())
            })
        })
        .flatten()
    else {
        return;
    };
    if state.selected_ids().is_empty() {
        state.message("Select a clip on the timeline first.");
        return;
    }
    let src_in = if state.source_clip.get() == Some(source) { state.src_in.get().unwrap_or(0) } else { 0 };
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
    } else {
        state.execute(Box::new(CompositeCommand::new("Razor", cuts)));
    }
}

/// Cut a clip at a time, together with the clips linked to it (the Blade tool).
pub fn split_at(state: &Rc<AppState>, clip_id: Uuid, at_us: i64) {
    let mut ids = vec![clip_id];
    ids.extend(state.linked_to(clip_id));
    let cuts = split_commands(state, &ids, at_us);
    if !cuts.is_empty() {
        state.execute(Box::new(CompositeCommand::new("Split", cuts)));
    }
}

/// Cut the selected clips at the playhead.
pub fn split_selected(state: &Rc<AppState>) {
    let ids = state.selected_ids();
    if ids.is_empty() {
        state.message("Select a clip first, or press Ctrl+B to cut every track at the playhead.");
        return;
    }
    let cuts = split_commands(state, &ids, state.playhead_us());
    if cuts.is_empty() {
        state.message("Move the playhead inside the selected clip first.");
    } else {
        state.execute(Box::new(CompositeCommand::new("Split", cuts)));
    }
}

/// Trim the start or end of the selected clips to the playhead. With `ripple`
/// the clips after them move up to close the gap.
pub fn trim_to_playhead(state: &Rc<AppState>, edge: TrimEdge, ripple: bool) {
    let pos = state.playhead_us();
    // Only clips the playhead is inside can be trimmed to it.
    let done = for_selected(state, if ripple { "Ripple Trim" } else { "Trim" }, |_, clip| {
        let delta = match edge {
            TrimEdge::In => pos - clip.timeline_in,
            TrimEdge::Out => pos - clip.timeline_out,
        };
        (pos > clip.timeline_in && pos < clip.timeline_out).then(|| -> Box<dyn Command> {
            if ripple {
                Box::new(tempo_timeline::RippleTrimCommand::new(clip.id, edge, delta))
            } else {
                Box::new(TrimClipCommand::new(clip.id, edge, delta))
            }
        })
    });
    if !done {
        state.message("Select a clip and put the playhead inside it first.");
    }
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

/// Take a file out of the Media Pool. Files still used on the timeline stay.
pub fn remove_source(state: &Rc<AppState>, source: Uuid) {
    let used = state.with_timeline(|t| t.tracks.iter().flat_map(|tr| tr.clips.iter()).any(|c| c.source_id == source)).unwrap_or(false);
    if used {
        state.message("This clip is used on the timeline. Delete it from the timeline first.");
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
    if state.is_selected(clip_id) {
        ids.extend(state.selected_ids().into_iter().filter(|id| *id != clip_id));
    }
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
                Box::new(tempo_timeline::RippleTrimCommand::new(id, edge, delta_us))
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
    let pos = state.playhead_us();
    let frame = state.frame_us();
    state.with_timeline(|t| t.markers.iter().find(|m| (m.position_us - pos).abs() < frame).cloned()).flatten()
}

pub fn add_marker(state: &Rc<AppState>) -> Option<Marker> {
    if let Some(existing) = marker_at_playhead(state) {
        return Some(existing);
    }
    let marker = Marker::new(state.frame_floor(state.playhead_us()), "", MarkerColor::Blue);
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

/// Jump to the next or previous cut on any track.
pub fn goto_edit(state: &Rc<AppState>, forward: bool) {
    let pos = state.player.position_us();
    let target = state
        .with_timeline(|t| {
            let points = t.tracks.iter().flat_map(|tr| tr.clips.iter()).flat_map(|c| [c.timeline_in, c.timeline_out]);
            if forward {
                points.filter(|p| *p > pos).min()
            } else {
                points.filter(|p| *p < pos).max()
            }
        })
        .flatten();
    if let Some(t) = target {
        state.player.pause();
        state.player.seek(t, true);
    }
}

pub fn step_frames(state: &Rc<AppState>, frames: i64) {
    let target = state.frame_step(state.player.position_us(), frames);
    state.player.pause();
    state.player.seek(target.min(state.player.duration_us()), true);
}

/// Set (or clear) In / Out. In source mode the marks belong to the source clip.
pub fn set_mark(state: &Rc<AppState>, is_in: bool, clear: bool) {
    let value = (!clear).then(|| state.player.position_us());
    let source = state.source_mode();
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
