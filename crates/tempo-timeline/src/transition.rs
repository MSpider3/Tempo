//! Transitions as objects on clip edges, as on DaVinci Resolve's Edit page.
//!
//! # Where a transition lives
//! A [`Transition`] belongs to one clip edge (`clip_id` + `edge`) and is kept in
//! `Track::transitions` of that clip's track, at most one per clip edge. It follows its
//! clip by id: moving, rippling or trimming clips never drops it.
//!
//! * **Cut** between clip A (outgoing) and clip B (incoming, `B.timeline_in ==
//!   A.timeline_out` on the same track): stored on **B, edge In**. `alignment` places it
//!   around the cut `c`, for a length `d`: `Centered` = `[c - d/2, c + d - d/2]`,
//!   `EndAtCut` = `[c - d, c]`, `StartAtCut` = `[c, c + d]`.
//! * **Head** of a clip with nothing touching before it: edge In on that clip, inside the
//!   clip from its start, `[in, in + d]`, whatever its alignment (fade from transparent,
//!   or from the colour for a dip). Resolve calls this Start on Edit.
//! * **Tail** of a clip: edge Out on that clip, always inside the clip and ending on its
//!   out point, `[out - d, out]` (fade to transparent or to the dip colour), whether or
//!   not a clip touches after it. A transition between two clips is always B's In.
//!
//! Never inert: an In transition whose clip has lost the clip before it becomes a head
//! fade over its span, and a cut transition again once a clip touches it.
//!
//! # Lengths
//! Lengths are whole frames ([`frame_floor_us`]); new ones default to
//! [`STANDARD_TRANSITION_US`] (1 s). The part of a cut transition before the cut shows
//! B's footage from before its In point (head handle); the part after it shows A's
//! footage after its Out point (tail handle). Each part also fits inside the clip it
//! covers. Head and tail transitions fit inside their clip. A clip's two ends share it:
//! what its In transition covers plus what lies at its end (its Out transition, or the
//! part of the next cut's transition before that cut) is at most its length.
//! [`Track::max_transition_us`] gives the longest allowed; nothing here trims clips to
//! make room. Saved data that breaks this is shrunk when drawn and in
//! [`Track::transition_span`]: the In edge keeps its length, the end gets what is left.
//!
//! # Rendering
//! [`Track::transition_layers`] turns a track's transitions into things the renderers
//! already draw, in this order, bottom to top:
//! 1. `clips`: the track's clips, with cross-dissolve head/tail transitions folded into
//!    their `fade_in_us` / `fade_out_us` (the clip's own fade is kept if longer).
//! 2. `mix`: extra clips (handle footage), drawn over the track with their fade ramps.
//!    For a cross dissolve on a cut they are B's head before the cut, fading in, and A's
//!    tail after it, fading out, so the picture is `A*(1-p) + B*p`, `p` going 0 to 1
//!    across the span. A fade longer than the extra clip gives a partial ramp on purpose.
//! 3. `color`: [`ColorLayer`]s of solid colour with an opacity ramp, for dips: over the
//!    first half A dissolves to the colour, over the second the colour to B (the extra
//!    clips show B or A at full opacity under the colour where the cut is off-centre).
//!    A dip at a head or tail fades from or to the colour.
//!
//! Ramps are linear; `Transition::ease` is stored for the renderers to apply. `Cut`
//! transitions draw nothing. Other old kinds mix like a cross dissolve, except the black
//! and white ones, which dip (see [`Transition::dip_color`]).

use std::collections::HashMap;
use uuid::Uuid;

use crate::types::{
    frame_to_us, Clip, ClipType, MediaSource, RationalFps, Track, Transition, TransitionAlignment, TransitionEase, TransitionEdge,
    TransitionKind,
};

/// Length of a new transition, 1 s as in Resolve.
pub const STANDARD_TRANSITION_US: i64 = 1_000_000;

/// The longest whole number of frames that fits in `us`, in microseconds.
pub fn frame_floor_us(us: i64, fps: RationalFps) -> i64 {
    if us <= 0 || fps.num == 0 || fps.den == 0 {
        return us.max(0);
    }
    let frames = (us as i128 * fps.num as i128 / (1_000_000 * fps.den as i128)) as i64;
    // frame_to_us rounds to the nearest microsecond, so a whole frame count can sit a
    // microsecond under its exact time and floor one frame short; take the next frame if it fits.
    if frame_to_us(frames + 1, fps) <= us {
        frame_to_us(frames + 1, fps)
    } else {
        frame_to_us(frames, fps)
    }
}

impl Transition {
    /// A new transition with a fresh id, no plugin, black and no ease.
    pub fn new(clip_id: Uuid, edge: TransitionEdge, kind: TransitionKind, duration_us: i64, alignment: TransitionAlignment) -> Self {
        Self {
            id: Uuid::new_v4(),
            clip_id,
            edge,
            kind,
            duration_us,
            alignment,
            plugin_id: None,
            plugin_params: None,
            color: [0.0; 3],
            ease: TransitionEase::None,
        }
    }

    /// The colour this transition dips through, or `None` when it mixes the pictures.
    pub fn dip_color(&self) -> Option<[f32; 3]> {
        match self.kind {
            TransitionKind::DipToColor => Some(self.color),
            TransitionKind::DipToBlack => Some([0.0; 3]),
            TransitionKind::DipToWhite | TransitionKind::FadeToWhite | TransitionKind::FadeFromWhite => Some([1.0; 3]),
            _ => None,
        }
    }
}

/// What kind of edit point a position on a track is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditKind {
    /// One clip ends where the next starts.
    Cut,
    /// A clip starts with nothing touching before it.
    Head,
    /// A clip ends with nothing touching after it.
    Tail,
}

/// An edit point and the clip edge a transition put there belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EditTarget {
    pub position_us: i64,
    pub clip_id: Uuid,
    pub edge: TransitionEdge,
    pub kind: EditKind,
}

/// A full-frame solid colour drawn over a span, for dip transitions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorLayer {
    pub start_us: i64,
    pub end_us: i64,
    /// RGB, 0.0-1.0.
    pub color: [f32; 3],
    /// Opacity ramps up from 0 over this long after `start_us`, and down to 0 over
    /// `fade_out_us` before `end_us`, as `Clip::fade_factor` does for clips.
    pub fade_in_us: i64,
    pub fade_out_us: i64,
}

impl ColorLayer {
    /// Opacity at a timeline position: 0.0 outside the layer.
    pub fn opacity_at(&self, time_us: i64) -> f32 {
        if time_us < self.start_us || time_us >= self.end_us {
            return 0.0;
        }
        let (from_start, to_end) = (time_us - self.start_us, self.end_us - time_us);
        let mut factor = 1.0f32;
        if self.fade_in_us > 0 && from_start < self.fade_in_us {
            factor = factor.min(from_start as f32 / self.fade_in_us as f32);
        }
        if self.fade_out_us > 0 && to_end < self.fade_out_us {
            factor = factor.min(to_end as f32 / self.fade_out_us as f32);
        }
        factor
    }
}

/// A track's transitions as layers to draw; see the module docs for the order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TransitionLayers {
    pub clips: Vec<Clip>,
    pub mix: Vec<Clip>,
    pub color: Vec<ColorLayer>,
}

/// Where a transition falls, its parts limited to the clips they cover.
enum Shape<'a> {
    Cut { outgoing: &'a Clip, incoming: &'a Clip, before: i64, after: i64 },
    Head(&'a Clip, i64),
    Tail(&'a Clip, i64),
}

fn is_still(clip: &Clip) -> bool {
    matches!(clip.clip_type, ClipType::Image | ClipType::Title)
}

/// Timeline length of the footage before the clip's In point.
fn head_room(clip: &Clip) -> i64 {
    if is_still(clip) {
        i64::MAX
    } else {
        clip.to_timeline_floor_us(clip.source_in).max(0)
    }
}

/// Timeline length of the footage after the clip's Out point. A source not in
/// `sources` is not limited.
fn tail_room(clip: &Clip, sources: &HashMap<Uuid, MediaSource>) -> i64 {
    match sources.get(&clip.source_id) {
        Some(s) if !is_still(clip) => clip.to_timeline_floor_us((s.duration_us - clip.source_out).max(0)),
        _ => i64::MAX,
    }
}

/// `len` of the clip's footage from before its In point, placed just before its start.
fn head_piece(clip: &Clip, len: i64, fade_in_us: i64, fade_out_us: i64) -> Clip {
    let mut piece = clip.clone();
    piece.timeline_in = clip.timeline_in - len;
    piece.timeline_out = clip.timeline_in;
    piece.source_in = (clip.source_in - clip.to_source_us(len)).max(0);
    piece.source_out = clip.source_in;
    set_fades(piece, fade_in_us, fade_out_us)
}

/// `len` of the clip's footage from after its Out point, placed just after its end.
fn tail_piece(clip: &Clip, len: i64, fade_in_us: i64, fade_out_us: i64) -> Clip {
    let mut piece = clip.clone();
    piece.timeline_in = clip.timeline_out;
    piece.timeline_out = clip.timeline_out + len;
    piece.source_in = clip.source_out;
    piece.source_out = clip.source_out + clip.to_source_us(len);
    set_fades(piece, fade_in_us, fade_out_us)
}

fn set_fades(mut clip: Clip, fade_in_us: i64, fade_out_us: i64) -> Clip {
    clip.properties.fade_in_us = fade_in_us;
    clip.properties.fade_out_us = fade_out_us;
    clip.properties.dissolve_in_us = 0;
    clip
}

impl Track {
    fn clip_ending_at(&self, position_us: i64) -> Option<&Clip> {
        self.clips.iter().find(|c| c.timeline_out == position_us)
    }

    fn clip_starting_at(&self, position_us: i64) -> Option<&Clip> {
        self.clips.iter().find(|c| c.timeline_in == position_us)
    }

    /// The transition on a clip edge, if any.
    pub fn transition_at_edit(&self, clip_id: Uuid, edge: TransitionEdge) -> Option<&Transition> {
        self.transitions.iter().find(|t| t.clip_id == clip_id && t.edge == edge)
    }

    /// The edit exactly at `position_us` and the clip edge a transition there belongs
    /// to: a cut gives the incoming clip's In, a lone head its In, a lone tail its Out.
    pub fn edit_at(&self, position_us: i64) -> Option<EditTarget> {
        let (clip, edge, kind) = match (self.clip_ending_at(position_us), self.clip_starting_at(position_us)) {
            (Some(_), Some(incoming)) => (incoming, TransitionEdge::In, EditKind::Cut),
            (None, Some(clip)) => (clip, TransitionEdge::In, EditKind::Head),
            (Some(clip), None) => (clip, TransitionEdge::Out, EditKind::Tail),
            (None, None) => return None,
        };
        Some(EditTarget { position_us, clip_id: clip.id, edge, kind })
    }

    /// Every edit point on the track, earliest first.
    pub fn edit_points(&self) -> Vec<EditTarget> {
        let mut at: Vec<i64> = self.clips.iter().flat_map(|c| [c.timeline_in, c.timeline_out]).collect();
        at.sort_unstable();
        at.dedup();
        at.into_iter().filter_map(|p| self.edit_at(p)).collect()
    }

    /// How much of `clip`, from its start, its In transition covers (head fade or the
    /// part of a cut transition after the cut). This edge has first claim on the clip.
    fn in_use(&self, clip: &Clip) -> i64 {
        let Some(t) = self.transition_at_edit(clip.id, TransitionEdge::In) else { return 0 };
        let d = t.duration_us.max(0);
        let inside = if self.clip_ending_at(clip.timeline_in).is_some() { d - before_cut(t) } else { d };
        inside.clamp(0, clip.duration_us())
    }

    /// How much of `clip`, up to its end, its Out transition or the part of the next
    /// cut's transition before that cut asks for, whichever is longer.
    fn out_use(&self, clip: &Clip) -> i64 {
        let tail = self.transition_at_edit(clip.id, TransitionEdge::Out).map_or(0, |t| t.duration_us);
        let next = self
            .clip_starting_at(clip.timeline_out)
            .and_then(|n| self.transition_at_edit(n.id, TransitionEdge::In))
            .map_or(0, before_cut);
        tail.max(next).clamp(0, clip.duration_us())
    }

    fn shape(&self, t: &Transition) -> Option<Shape<'_>> {
        let clip = self.find_clip(t.clip_id)?;
        let d = t.duration_us.max(0);
        // A clip's In edge keeps its length; what lies at its end gets what is left.
        if t.edge == TransitionEdge::Out {
            return Some(Shape::Tail(clip, d.min(clip.duration_us() - self.in_use(clip))));
        }
        let Some(outgoing) = self.clip_ending_at(clip.timeline_in) else {
            return Some(Shape::Head(clip, d.min(clip.duration_us())));
        };
        let before = before_cut(t);
        Some(Shape::Cut {
            outgoing,
            incoming: clip,
            before: before.min(outgoing.duration_us() - self.in_use(outgoing)).max(0),
            after: (d - before).min(clip.duration_us()),
        })
    }

    /// Timeline start and end of a transition on this track, `None` if its clip is not
    /// here. Limited to the clips it covers (sharing a clip with the transition at its
    /// other end, see `max_transition_us`), not to the footage there is.
    pub fn transition_span(&self, t: &Transition) -> Option<(i64, i64)> {
        Some(match self.shape(t)? {
            Shape::Cut { incoming, before, after, .. } => (incoming.timeline_in - before, incoming.timeline_in + after),
            Shape::Head(clip, len) => (clip.timeline_in, clip.timeline_in + len),
            Shape::Tail(clip, len) => (clip.timeline_out - len, clip.timeline_out),
        })
    }

    /// The longest transition, in whole frames, that fits on a clip edge with this
    /// alignment where the clips are now: limited by the handles (`sources` gives each
    /// source's length; a source not in it is not limited) and the clips' lengths, less
    /// what the transition at each clip's other end already covers: a clip's two
    /// transitions together never cover more than the clip. 0 when the clip is not on
    /// this track or nothing fits.
    pub fn max_transition_us(
        &self,
        clip_id: Uuid,
        edge: TransitionEdge,
        alignment: TransitionAlignment,
        sources: &HashMap<Uuid, MediaSource>,
        fps: RationalFps,
    ) -> i64 {
        let Some(clip) = self.find_clip(clip_id) else { return 0 };
        let longest = match (edge, self.clip_ending_at(clip.timeline_in)) {
            (TransitionEdge::Out, _) => clip.duration_us() - self.in_use(clip),
            (TransitionEdge::In, None) => clip.duration_us() - self.out_use(clip),
            (TransitionEdge::In, Some(outgoing)) => {
                let before = head_room(clip).min(outgoing.duration_us() - self.in_use(outgoing)).max(0);
                let after = tail_room(outgoing, sources).min(clip.duration_us() - self.out_use(clip)).max(0);
                match alignment {
                    TransitionAlignment::EndAtCut => before,
                    TransitionAlignment::StartAtCut => after,
                    // d/2 before the cut and the rest after it.
                    TransitionAlignment::Centered => before.saturating_mul(2).saturating_add(1).min(after.saturating_mul(2)),
                }
            }
        };
        frame_floor_us(longest, fps)
    }

    /// This track's clips and transitions as layers to draw (see the module docs).
    /// A transition longer than the footage there is shrinks to fit.
    pub fn transition_layers(&self, sources: &HashMap<Uuid, MediaSource>) -> TransitionLayers {
        let mut layers = TransitionLayers { clips: self.clips.clone(), ..TransitionLayers::default() };
        for t in self.transitions.iter().filter(|t| t.kind != TransitionKind::Cut) {
            let dip = t.dip_color();
            match self.shape(t) {
                Some(Shape::Cut { outgoing, incoming, before, after }) => {
                    let before = before.min(head_room(incoming));
                    let after = after.min(tail_room(outgoing, sources));
                    add_cut(&mut layers, outgoing, incoming, before, after, dip);
                }
                Some(Shape::Head(clip, len)) if len > 0 => match dip {
                    Some(color) => layers.color.push(ColorLayer { start_us: clip.timeline_in, end_us: clip.timeline_in + len, color, fade_in_us: 0, fade_out_us: len }),
                    None => {
                        if let Some(c) = layers.clips.iter_mut().find(|c| c.id == clip.id) {
                            c.properties.fade_in_us = c.properties.fade_in_us.max(len);
                        }
                    }
                },
                Some(Shape::Tail(clip, len)) if len > 0 => match dip {
                    Some(color) => layers.color.push(ColorLayer { start_us: clip.timeline_out - len, end_us: clip.timeline_out, color, fade_in_us: len, fade_out_us: 0 }),
                    None => {
                        if let Some(c) = layers.clips.iter_mut().find(|c| c.id == clip.id) {
                            c.properties.fade_out_us = c.properties.fade_out_us.max(len);
                        }
                    }
                },
                _ => {}
            }
        }
        // As `Clip::with_fade`: the fades of a clip never overlap, so every renderer agrees.
        for c in &mut layers.clips {
            let (dur, p) = (c.duration_us(), &mut c.properties);
            p.fade_in_us = p.fade_in_us.clamp(0, dur);
            p.fade_out_us = p.fade_out_us.clamp(0, dur - p.fade_in_us);
        }
        layers
    }

    /// Turns the old `ClipProperties::dissolve_in_us` of clips that touch the clip
    /// before them into cross dissolves ending on the cut, as they were drawn, and clears
    /// the field. Run on load.
    pub fn convert_legacy_dissolves(&mut self) {
        // Sound gets Resolve's audio Cross Fade, so it stays a sound transition.
        let kind = if self.kind == crate::types::TrackKind::Audio { TransitionKind::Crossfade } else { TransitionKind::CrossDissolve };
        let found: Vec<(Uuid, i64)> = self
            .clips
            .iter()
            .filter(|c| c.properties.dissolve_in_us > 0 && self.has_clip_ending_at(c.timeline_in))
            .map(|c| (c.id, c.properties.dissolve_in_us.min(head_room(c))))
            .collect();
        for (clip_id, d) in found {
            if let Some(clip) = self.find_clip_mut(clip_id) {
                clip.properties.dissolve_in_us = 0;
            }
            if d > 0 && self.transition_at_edit(clip_id, TransitionEdge::In).is_none() {
                self.transitions.push(Transition::new(clip_id, TransitionEdge::In, kind.clone(), d, TransitionAlignment::EndAtCut));
            }
        }
    }

    /// Removes and returns the transitions of a clip: on `edge`, or on both edges.
    pub(crate) fn take_transitions(&mut self, clip_id: Uuid, edge: Option<TransitionEdge>) -> Vec<Transition> {
        self.transitions.extract_if(.., |t| t.clip_id == clip_id && edge.is_none_or(|e| e == t.edge)).collect()
    }

    /// Gives the transition on `from`'s `edge` to clip `to`, as when a split hands the
    /// tail of a clip to its second part.
    pub(crate) fn hand_over_transition(&mut self, from: Uuid, to: Uuid, edge: TransitionEdge) {
        for t in self.transitions.iter_mut().filter(|t| t.clip_id == from && t.edge == edge) {
            t.clip_id = to;
        }
    }
}

/// How much of a cut transition lies before the cut, by its alignment.
fn before_cut(t: &Transition) -> i64 {
    let d = t.duration_us.max(0);
    match t.alignment {
        TransitionAlignment::Centered => d / 2,
        TransitionAlignment::EndAtCut => d,
        TransitionAlignment::StartAtCut => 0,
    }
}

fn add_cut(layers: &mut TransitionLayers, outgoing: &Clip, incoming: &Clip, before: i64, after: i64, dip: Option<[f32; 3]>) {
    let d = before + after;
    if d <= 0 {
        return;
    }
    let cut = incoming.timeline_in;
    let (start, end) = (cut - before, cut + after);
    let Some(color) = dip else {
        if before > 0 {
            layers.mix.push(head_piece(incoming, before, d, 0));
        }
        if after > 0 {
            layers.mix.push(tail_piece(outgoing, after, 0, d));
        }
        return;
    };
    // The colour is full at the middle; the picture under it switches there, not at the cut.
    let mid = start + d / 2;
    if mid < cut {
        layers.mix.push(head_piece(incoming, cut - mid, 0, 0));
    }
    if mid > cut {
        layers.mix.push(tail_piece(outgoing, mid - cut, 0, 0));
    }
    layers.color.push(ColorLayer { start_us: start, end_us: end, color, fade_in_us: mid - start, fade_out_us: end - mid });
}

#[cfg(test)]
mod tests {
    #[test]
    fn frame_floor_keeps_every_whole_frame_at_ntsc_rates() {
        for fps in [crate::RationalFps::FPS_29_97, crate::RationalFps::FPS_30] {
            for n in 0..2000 {
                let us = crate::frame_to_us(n, fps);
                assert_eq!(frame_floor_us(us, fps), us, "{n} frames at {}/{}", fps.num, fps.den);
            }
        }
    }

    use super::*;
    use crate::command::*;
    use crate::types::{MediaType, Timeline};
    use std::path::PathBuf;

    const S: i64 = 1_000_000;
    const FPS: RationalFps = RationalFps::FPS_25;

    /// Track 0 holds A [0, 4 s] (footage 1-5 s) and B [4, 8 s] (footage 2-6 s), both
    /// from 10 s sources, so A has 5 s of tail handle and B 2 s of head handle.
    fn setup() -> (Timeline, HashMap<Uuid, MediaSource>, Uuid, Uuid) {
        let mut tl = Timeline::new_default();
        let track = tl.tracks[0].id;
        let mut sources = HashMap::new();
        let mut clip = |name: &str, tin: i64, sin: i64| {
            let src = MediaSource::new(PathBuf::from(name), MediaType::Video, 10 * S);
            let c = Clip::new(track, src.id, ClipType::Video, name, tin, tin + 4 * S, sin, sin + 4 * S);
            sources.insert(src.id, src);
            c
        };
        let (a, b) = (clip("a", 0, S), clip("b", 4 * S, 2 * S));
        let ids = (a.id, b.id);
        tl.tracks[0].clips = vec![a, b];
        (tl, sources, ids.0, ids.1)
    }

    fn dissolve(clip: Uuid, edge: TransitionEdge, d: i64, alignment: TransitionAlignment) -> Transition {
        Transition::new(clip, edge, TransitionKind::CrossDissolve, d, alignment)
    }

    /// The picture at `t`, as one number: A's footage is 10, B's 20, white 100, drawn
    /// with their fade factors over black, in layer order.
    fn picture(layers: &TransitionLayers, t: i64) -> f32 {
        let mut v = 0.0;
        for c in layers.clips.iter().chain(&layers.mix).filter(|c| c.contains_point(t)) {
            let f = c.fade_factor(t);
            let value = if c.name == "a" { 10.0 } else { 20.0 };
            v = v * (1.0 - f) + value * f;
        }
        for l in &layers.color {
            let f = l.opacity_at(t);
            v = v * (1.0 - f) + l.color[0] * 100.0 * f;
        }
        v
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.01
    }

    #[test]
    fn spans_follow_the_alignment_around_a_cut() {
        let (tl, _, _, b) = setup();
        let track = &tl.tracks[0];
        let span = |al| track.transition_span(&dissolve(b, TransitionEdge::In, S, al));
        assert_eq!(span(TransitionAlignment::Centered), Some((3_500_000, 4_500_000)));
        assert_eq!(span(TransitionAlignment::EndAtCut), Some((3 * S, 4 * S)));
        assert_eq!(span(TransitionAlignment::StartAtCut), Some((4 * S, 5 * S)));
    }

    #[test]
    fn head_and_tail_transitions_lie_inside_their_clip() {
        let (mut tl, _, a, b) = setup();
        let track = &tl.tracks[0];
        // A's head has nothing before it, whatever the alignment says.
        assert_eq!(track.transition_span(&dissolve(a, TransitionEdge::In, S, TransitionAlignment::Centered)), Some((0, S)));
        assert_eq!(track.transition_span(&dissolve(b, TransitionEdge::Out, S, TransitionAlignment::EndAtCut)), Some((7 * S, 8 * S)));
        // An Out transition stays a tail fade even with a clip after it.
        assert_eq!(track.transition_span(&dissolve(a, TransitionEdge::Out, S, TransitionAlignment::Centered)), Some((3 * S, 4 * S)));
        // Once A is gone, B's cut transition becomes a fade from its start.
        tl.tracks[0].clips.retain(|c| c.id != a);
        let t = dissolve(b, TransitionEdge::In, S, TransitionAlignment::Centered);
        assert_eq!(tl.tracks[0].transition_span(&t), Some((4 * S, 5 * S)));
        assert_eq!(tl.tracks[0].transition_span(&dissolve(a, TransitionEdge::In, S, TransitionAlignment::Centered)), None);
    }

    #[test]
    fn edits_name_the_clip_edge_a_transition_belongs_to() {
        let (tl, _, a, b) = setup();
        let track = &tl.tracks[0];
        let kinds: Vec<_> = track.edit_points().iter().map(|e| (e.position_us, e.clip_id, e.edge, e.kind)).collect();
        assert_eq!(
            kinds,
            vec![
                (0, a, TransitionEdge::In, EditKind::Head),
                (4 * S, b, TransitionEdge::In, EditKind::Cut),
                (8 * S, b, TransitionEdge::Out, EditKind::Tail),
            ]
        );
        assert_eq!(track.edit_at(2 * S), None);
    }

    #[test]
    fn longest_transition_comes_from_the_handles() {
        let (mut tl, mut sources, a, b) = setup();
        let max = |tl: &Timeline, s: &HashMap<_, _>, clip, edge, al| tl.tracks[0].max_transition_us(clip, edge, al, s, FPS);
        // Before the cut: B's 2 s of head. After it: A's 5 s of tail, but only B's 4 s length.
        assert_eq!(max(&tl, &sources, b, TransitionEdge::In, TransitionAlignment::EndAtCut), 2 * S);
        assert_eq!(max(&tl, &sources, b, TransitionEdge::In, TransitionAlignment::StartAtCut), 4 * S);
        assert_eq!(max(&tl, &sources, b, TransitionEdge::In, TransitionAlignment::Centered), 4 * S);
        // Lone edges: the clip's own length.
        assert_eq!(max(&tl, &sources, a, TransitionEdge::In, TransitionAlignment::Centered), 4 * S);
        assert_eq!(max(&tl, &sources, b, TransitionEdge::Out, TransitionAlignment::EndAtCut), 4 * S);
        // A with only 0.5 s of tail: a centred transition can be 1 s, in whole frames.
        sources.get_mut(&tl.tracks[0].clips[0].source_id).unwrap().duration_us = 5_510_000;
        assert_eq!(max(&tl, &sources, b, TransitionEdge::In, TransitionAlignment::StartAtCut), 480_000);
        assert_eq!(max(&tl, &sources, b, TransitionEdge::In, TransitionAlignment::Centered), 1_000_000);
        // No head handle at all: nothing ends on the cut.
        tl.tracks[0].clips[1].source_in = 0;
        assert_eq!(max(&tl, &sources, b, TransitionEdge::In, TransitionAlignment::EndAtCut), 0);
        assert_eq!(max(&tl, &sources, Uuid::new_v4(), TransitionEdge::In, TransitionAlignment::EndAtCut), 0);
    }

    #[test]
    fn stills_have_endless_handles() {
        let (mut tl, sources, _, b) = setup();
        tl.tracks[0].clips[1].clip_type = ClipType::Image;
        tl.tracks[0].clips[1].source_in = 0;
        assert_eq!(tl.tracks[0].max_transition_us(b, TransitionEdge::In, TransitionAlignment::EndAtCut, &sources, FPS), 4 * S);
    }

    #[test]
    fn a_centred_dissolve_mixes_a_into_b() {
        let (mut tl, sources, _, b) = setup();
        tl.tracks[0].transitions.push(dissolve(b, TransitionEdge::In, S, TransitionAlignment::Centered));
        let layers = tl.tracks[0].transition_layers(&sources);
        assert_eq!(layers.mix.len(), 2);
        // B's head plays from its footage before the In point.
        assert_eq!((layers.mix[0].timeline_in, layers.mix[0].source_in, layers.mix[0].source_out), (3_500_000, 1_500_000, 2 * S));
        assert_eq!((layers.mix[1].timeline_out, layers.mix[1].source_in, layers.mix[1].source_out), (4_500_000, 5 * S, 5_500_000));
        for (t, want) in [(3 * S, 10.0), (3_750_000, 12.5), (4 * S, 15.0), (4_250_000, 17.5), (5 * S, 20.0)] {
            assert!(close(picture(&layers, t), want), "at {t}: {} want {want}", picture(&layers, t));
        }
    }

    #[test]
    fn end_and_start_on_edit_dissolves_mix_on_one_side() {
        let (mut tl, sources, _, b) = setup();
        tl.tracks[0].transitions.push(dissolve(b, TransitionEdge::In, S, TransitionAlignment::EndAtCut));
        let layers = tl.tracks[0].transition_layers(&sources);
        assert_eq!(layers.mix.len(), 1);
        assert!(close(picture(&layers, 3_500_000), 15.0));
        assert!(close(picture(&layers, 4 * S), 20.0));

        tl.tracks[0].transitions[0].alignment = TransitionAlignment::StartAtCut;
        let layers = tl.tracks[0].transition_layers(&sources);
        assert!(close(picture(&layers, 4 * S), 10.0));
        assert!(close(picture(&layers, 4_500_000), 15.0));
        assert!(close(picture(&layers, 5 * S), 20.0));
    }

    #[test]
    fn a_dip_goes_through_the_colour() {
        let (mut tl, sources, _, b) = setup();
        let mut t = Transition::new(b, TransitionEdge::In, TransitionKind::DipToColor, S, TransitionAlignment::EndAtCut);
        t.color = [1.0, 1.0, 1.0];
        tl.tracks[0].transitions.push(t);
        let layers = tl.tracks[0].transition_layers(&sources);
        assert_eq!(layers.color.len(), 1);
        // White counts as 100: A at the start, all white at the middle, B at the end.
        for (t, want) in [(3 * S, 10.0), (3_250_000, 55.0), (3_500_000, 100.0), (3_750_000, 60.0), (4 * S, 20.0)] {
            assert!(close(picture(&layers, t), want), "at {t}: {} want {want}", picture(&layers, t));
        }
        // Black is the default colour.
        assert_eq!(Transition::new(b, TransitionEdge::In, TransitionKind::DipToColor, S, TransitionAlignment::Centered).dip_color(), Some([0.0; 3]));
    }

    #[test]
    fn lone_edges_fade_the_clip_or_dip_from_the_colour() {
        let (mut tl, sources, a, b) = setup();
        tl.tracks[0].transitions.push(dissolve(a, TransitionEdge::In, S, TransitionAlignment::StartAtCut));
        let mut dip = Transition::new(b, TransitionEdge::Out, TransitionKind::DipToColor, S, TransitionAlignment::EndAtCut);
        dip.color = [1.0, 0.0, 0.0];
        tl.tracks[0].transitions.push(dip);
        let layers = tl.tracks[0].transition_layers(&sources);
        assert!(layers.mix.is_empty());
        assert_eq!(layers.clips[0].properties.fade_in_us, S);
        assert!(close(picture(&layers, 500_000), 5.0));
        assert_eq!(layers.color, vec![ColorLayer { start_us: 7 * S, end_us: 8 * S, color: [1.0, 0.0, 0.0], fade_in_us: S, fade_out_us: 0 }]);
        assert!(close(picture(&layers, 7_500_000), 60.0));
    }

    #[test]
    fn a_dissolve_shrinks_to_the_footage_there_is() {
        let (mut tl, sources, _, b) = setup();
        tl.tracks[0].clips[1].source_in = 250_000;
        tl.tracks[0].transitions.push(dissolve(b, TransitionEdge::In, S, TransitionAlignment::Centered));
        let layers = tl.tracks[0].transition_layers(&sources);
        // 0.25 s before the cut and 0.5 s after: still continuous, A to B.
        assert_eq!(layers.mix[0].timeline_in, 3_750_000);
        assert!(close(picture(&layers, 3_750_000), 10.0));
        assert!(close(picture(&layers, 4 * S), 10.0 + 10.0 / 3.0));
        assert!(close(picture(&layers, 4_500_000), 20.0));
    }

    #[test]
    fn old_dissolves_become_transitions() {
        let (mut tl, _, a, b) = setup();
        tl.tracks[0].clips[1].properties.dissolve_in_us = 500_000;
        // A dissolve with nothing before it was never drawn and stays as it was.
        tl.tracks[0].clips[0].properties.dissolve_in_us = 500_000;
        tl.tracks[0].convert_legacy_dissolves();
        let track = &tl.tracks[0];
        assert_eq!(track.transitions.len(), 1);
        let t = track.transition_at_edit(b, TransitionEdge::In).unwrap();
        assert_eq!((t.kind.clone(), t.duration_us, t.alignment), (TransitionKind::CrossDissolve, 500_000, TransitionAlignment::EndAtCut));
        assert_eq!(track.find_clip(b).unwrap().properties.dissolve_in_us, 0);
        assert_eq!(track.find_clip(a).unwrap().properties.dissolve_in_us, 500_000);
        assert!(track.dissolve_leads().is_empty());
    }

    #[test]
    fn set_transition_adds_changes_and_removes_with_undo() {
        let (mut tl, _, _, b) = setup();
        let track = tl.tracks[0].id;
        let mut log = CommandLog::new();
        let t = dissolve(b, TransitionEdge::In, S, TransitionAlignment::Centered);
        log.execute(Box::new(SetTransitionCommand::set(track, t.clone())), &mut tl).unwrap();
        let mut longer = t.clone();
        longer.duration_us = 2 * S;
        longer.ease = TransitionEase::InOut;
        let change = SetTransitionCommand::set(track, longer.clone());
        log.execute(Box::new(change), &mut tl).unwrap();
        assert_eq!(tl.tracks[0].transitions, vec![longer]);
        log.execute(Box::new(SetTransitionCommand::remove(track, b, TransitionEdge::In)), &mut tl).unwrap();
        assert!(tl.tracks[0].transitions.is_empty());
        log.undo(&mut tl).unwrap();
        log.undo(&mut tl).unwrap();
        assert_eq!(tl.tracks[0].transitions, vec![t]);
        log.undo(&mut tl).unwrap();
        assert!(tl.tracks[0].transitions.is_empty());
        log.redo(&mut tl).unwrap();
        assert_eq!(tl.tracks[0].transitions.len(), 1);
    }

    #[test]
    fn set_transition_refuses_bad_requests() {
        let (mut tl, _, _, b) = setup();
        let track = tl.tracks[0].id;
        let none = SetTransitionCommand::remove(track, b, TransitionEdge::In);
        assert!(none.description() == "Remove Transition");
        let mut none = none;
        assert!(none.execute(&mut tl).is_err());
        assert!(SetTransitionCommand::set(track, dissolve(b, TransitionEdge::In, 0, TransitionAlignment::Centered)).execute(&mut tl).is_err());
        assert!(SetTransitionCommand::set(track, dissolve(Uuid::new_v4(), TransitionEdge::In, S, TransitionAlignment::Centered)).execute(&mut tl).is_err());
        tl.tracks[0].locked = true;
        assert!(SetTransitionCommand::set(track, dissolve(b, TransitionEdge::In, S, TransitionAlignment::Centered)).execute(&mut tl).is_err());
        assert!(tl.tracks[0].transitions.is_empty());
    }

    /// Setup with a head transition on A, a cut transition on B and a tail one on B.
    fn with_transitions() -> (Timeline, Uuid, Uuid) {
        let (mut tl, _, a, b) = setup();
        tl.tracks[0].transitions = vec![
            dissolve(a, TransitionEdge::In, S, TransitionAlignment::StartAtCut),
            dissolve(b, TransitionEdge::In, S, TransitionAlignment::Centered),
            dissolve(b, TransitionEdge::Out, S, TransitionAlignment::EndAtCut),
        ];
        (tl, a, b)
    }

    fn edges(tl: &Timeline, track: usize) -> Vec<(Uuid, TransitionEdge)> {
        let mut e: Vec<_> = tl.tracks[track].transitions.iter().map(|t| (t.clip_id, t.edge)).collect();
        e.sort_by_key(|(c, edge)| (*c, *edge == TransitionEdge::Out));
        e
    }

    #[test]
    fn deleting_a_clip_removes_its_transitions_until_undo() {
        for ripple in [false, true] {
            let (mut tl, a, b) = with_transitions();
            let before = edges(&tl, 0);
            let track = tl.tracks[0].id;
            let mut log = CommandLog::new();
            let cmd: Box<dyn Command> = if ripple { Box::new(RippleDeleteCommand::new(track, b)) } else { Box::new(DeleteClipCommand::new(track, b)) };
            log.execute(cmd, &mut tl).unwrap();
            assert_eq!(edges(&tl, 0), vec![(a, TransitionEdge::In)]);
            log.undo(&mut tl).unwrap();
            assert_eq!(edges(&tl, 0), before);
        }
    }

    #[test]
    fn splitting_hands_the_tail_transition_to_the_second_part() {
        let (mut tl, _, b) = with_transitions();
        let before = edges(&tl, 0);
        let track = tl.tracks[0].id;
        let split = SplitClipCommand::new(track, b, 6 * S);
        let second = split.second_clip_id();
        let mut log = CommandLog::new();
        log.execute(Box::new(split), &mut tl).unwrap();
        let t = &tl.tracks[0];
        assert!(t.transition_at_edit(b, TransitionEdge::In).is_some());
        assert!(t.transition_at_edit(b, TransitionEdge::Out).is_none());
        assert_eq!(t.transition_span(t.transition_at_edit(second, TransitionEdge::Out).unwrap()), Some((7 * S, 8 * S)));
        log.undo(&mut tl).unwrap();
        assert_eq!(edges(&tl, 0), before);
    }

    #[test]
    fn moving_a_clip_to_another_track_takes_its_transitions() {
        let (mut tl, a, b) = with_transitions();
        let (v1, v2) = (tl.tracks[0].id, tl.tracks[1].id);
        let mut log = CommandLog::new();
        log.execute(Box::new(MoveClipCommand::new(b, v1, v2, 4 * S, 10 * S)), &mut tl).unwrap();
        assert_eq!(edges(&tl, 0), vec![(a, TransitionEdge::In)]);
        assert_eq!(tl.tracks[1].transitions.len(), 2);
        // Alone on V2, B's cut transition is now a fade from its start: never inert.
        let t = tl.tracks[1].transition_at_edit(b, TransitionEdge::In).unwrap();
        assert_eq!(tl.tracks[1].transition_span(t), Some((10 * S, 11 * S)));
        log.undo(&mut tl).unwrap();
        assert_eq!(tl.tracks[0].transitions.len(), 3);
        assert!(tl.tracks[1].transitions.is_empty());
        // Within a track it keeps them, attached by id.
        log.execute(Box::new(MoveClipCommand::new(b, v1, v1, 4 * S, 9 * S)), &mut tl).unwrap();
        assert_eq!(tl.tracks[0].transitions.len(), 3);
    }

    #[test]
    fn overwriting_drops_covered_edges_and_hands_over_a_cut_tail() {
        let (mut tl, a, b) = with_transitions();
        let before = edges(&tl, 0);
        let track = tl.tracks[0].id;
        let mut log = CommandLog::new();
        // Over the cut: A's tail and B's head are covered, A's head stays.
        let over = Clip::new(track, Uuid::new_v4(), ClipType::Video, "n", 3 * S, 5 * S, 0, 2 * S);
        log.execute(Box::new(OverwriteClipCommand::new(over)), &mut tl).unwrap();
        let mut want = vec![(a, TransitionEdge::In), (b, TransitionEdge::Out)];
        want.sort_by_key(|(c, edge)| (*c, *edge == TransitionEdge::Out));
        assert_eq!(edges(&tl, 0), want);
        log.undo(&mut tl).unwrap();
        assert_eq!(edges(&tl, 0), before);
        // Inside B: B is cut in two and its tail transition goes to the second part.
        let inside = OverwriteClipCommand::new(Clip::new(track, Uuid::new_v4(), ClipType::Video, "n", 5 * S, 6 * S, 0, S));
        log.execute(Box::new(inside), &mut tl).unwrap();
        let right = tl.tracks[0].clips.iter().find(|c| c.timeline_in == 6 * S).unwrap().id;
        assert!(tl.tracks[0].transition_at_edit(right, TransitionEdge::Out).is_some());
        assert!(tl.tracks[0].transition_at_edit(b, TransitionEdge::In).is_some());
        // Covering all of A removes both its edges.
        log.undo(&mut tl).unwrap();
        log.execute(Box::new(OverwriteClipCommand::new(Clip::new(track, Uuid::new_v4(), ClipType::Video, "n", 0, 4 * S, 0, 4 * S))), &mut tl).unwrap();
        assert!(tl.tracks[0].transition_at_edit(a, TransitionEdge::In).is_none());
    }

    #[test]
    fn a_ripple_insert_inside_a_clip_hands_over_its_tail() {
        let (mut tl, _, b) = with_transitions();
        let before = edges(&tl, 0);
        let track = tl.tracks[0].id;
        let mut log = CommandLog::new();
        let insert = Clip::new(track, Uuid::new_v4(), ClipType::Video, "n", 6 * S, 7 * S, 0, S);
        log.execute(Box::new(InsertClipCommand::new_ripple(insert)), &mut tl).unwrap();
        let right = tl.tracks[0].clips.iter().find(|c| c.timeline_in == 7 * S).unwrap().id;
        assert!(tl.tracks[0].transition_at_edit(right, TransitionEdge::Out).is_some());
        assert!(tl.tracks[0].transition_at_edit(b, TransitionEdge::Out).is_none());
        log.undo(&mut tl).unwrap();
        assert_eq!(edges(&tl, 0), before);
    }

    #[test]
    fn frame_floor_never_rounds_up() {
        assert_eq!(frame_floor_us(1_039_999, FPS), 1_000_000);
        assert_eq!(frame_floor_us(1_040_000, FPS), 1_040_000);
        assert!(frame_floor_us(1_000_000, RationalFps::FPS_23_976) <= 1_000_000);
        assert_eq!(frame_floor_us(-5, FPS), 0);
    }

    /// V1 with A [0, 4 s], B [4, 4.6 s], C [4.6, 8 s], each with 2 s of handle on both sides.
    fn short_middle_clip() -> (Timeline, HashMap<Uuid, MediaSource>, [Uuid; 3]) {
        let mut tl = Timeline::new_default();
        let track = tl.tracks[0].id;
        let mut sources = HashMap::new();
        let mut ids = [Uuid::nil(); 3];
        for (i, (tin, tout)) in [(0, 4 * S), (4 * S, 4_600_000), (4_600_000, 8 * S)].into_iter().enumerate() {
            let src = MediaSource::new(PathBuf::from("m"), MediaType::Video, 20 * S);
            let c = Clip::new(track, src.id, ClipType::Video, "m", tin, tout, 2 * S, 2 * S + tout - tin);
            sources.insert(src.id, src);
            ids[i] = c.id;
            tl.tracks[0].clips.push(c);
        }
        (tl, sources, ids)
    }

    fn assert_no_overlap(clips: &[Clip]) {
        for (i, x) in clips.iter().enumerate() {
            for y in &clips[i + 1..] {
                assert!(x.timeline_out <= y.timeline_in || y.timeline_out <= x.timeline_in, "{:?} overlaps {:?}", (x.timeline_in, x.timeline_out), (y.timeline_in, y.timeline_out));
            }
        }
    }

    #[test]
    fn both_cuts_of_a_short_clip_share_its_length() {
        let (mut tl, sources, [_, b, c]) = short_middle_clip();
        tl.tracks[0].transitions.push(dissolve(b, TransitionEdge::In, S, TransitionAlignment::Centered));
        // 0.5 s of B is used by its first cut, so the second can only put 0.1 s into B.
        assert_eq!(tl.tracks[0].max_transition_us(c, TransitionEdge::In, TransitionAlignment::Centered, &sources, FPS), 200_000);
        assert_eq!(tl.tracks[0].max_transition_us(c, TransitionEdge::In, TransitionAlignment::EndAtCut, &sources, FPS), 80_000);
        // And the first cut can only put 0.6 s - what the second uses into B.
        tl.tracks[0].transitions.push(dissolve(c, TransitionEdge::In, 200_000, TransitionAlignment::Centered));
        assert_eq!(tl.tracks[0].max_transition_us(b, TransitionEdge::In, TransitionAlignment::StartAtCut, &sources, FPS), 480_000);
        assert_eq!(tl.tracks[0].max_transition_us(b, TransitionEdge::Out, TransitionAlignment::EndAtCut, &sources, FPS), 80_000);
    }

    #[test]
    fn saved_cuts_that_overlap_in_a_short_clip_are_shrunk() {
        let (mut tl, sources, [_, b, c]) = short_middle_clip();
        tl.tracks[0].transitions.push(dissolve(b, TransitionEdge::In, S, TransitionAlignment::Centered));
        tl.tracks[0].transitions.push(dissolve(c, TransitionEdge::In, S, TransitionAlignment::Centered));
        let layers = tl.tracks[0].transition_layers(&sources);
        assert_no_overlap(&layers.mix);
        // The earlier cut keeps its length; the later one gets what is left of B.
        let track = &tl.tracks[0];
        assert_eq!(track.transition_span(&track.transitions[0]), Some((3_500_000, 4_500_000)));
        assert_eq!(track.transition_span(&track.transitions[1]), Some((4_500_000, 5_100_000)));
    }

    #[test]
    fn head_and_tail_of_a_lone_clip_never_overlap() {
        let (mut tl, sources, a, b) = setup();
        tl.tracks[0].clips.retain(|c| c.id != b);
        tl.tracks[0].clips[0].timeline_out = 1_500_000;
        tl.tracks[0].transitions.push(dissolve(a, TransitionEdge::In, S, TransitionAlignment::StartAtCut));
        // 0.5 s left is 12.5 frames: 12 whole ones.
        assert_eq!(tl.tracks[0].max_transition_us(a, TransitionEdge::Out, TransitionAlignment::EndAtCut, &sources, FPS), 480_000);
        tl.tracks[0].transitions.push(dissolve(a, TransitionEdge::Out, S, TransitionAlignment::EndAtCut));
        let layers = tl.tracks[0].transition_layers(&sources);
        let p = &layers.clips[0].properties;
        assert_eq!((p.fade_in_us, p.fade_out_us), (S, 500_000));
        // The clip's own fade is kept within what the transition leaves too.
        tl.tracks[0].transitions.pop();
        tl.tracks[0].clips[0].properties.fade_out_us = 1_200_000;
        let p = &tl.tracks[0].transition_layers(&sources).clips[0].properties;
        assert_eq!((p.fade_in_us, p.fade_out_us), (S, 500_000));
    }

    #[test]
    fn old_dissolves_on_sound_become_cross_fades() {
        let (mut tl, _, _, b) = setup();
        tl.tracks[0].kind = crate::types::TrackKind::Audio;
        tl.tracks[0].clips[1].properties.dissolve_in_us = 500_000;
        tl.tracks[0].convert_legacy_dissolves();
        assert_eq!(tl.tracks[0].transition_at_edit(b, TransitionEdge::In).map(|t| t.kind.clone()), Some(TransitionKind::Crossfade));
    }
}
