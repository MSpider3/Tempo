//! Audio transitions as gain ramps, shared by the live player and the export.
//!
//! A transition on an audio track ramps the outgoing clip down and the incoming clip up
//! over `Track::transition_span`, the incoming side using handle footage before the cut
//! and the outgoing side its footage after it (extra "pieces" in separate strips, since
//! they overlap the clips). A cut is equal power (sin/cos, Resolve's "Cross Fade +3 dB");
//! a head or tail transition is a plain linear fade. Video transitions never touch sound.

use std::collections::HashMap;

use tempo_timeline::{Clip, ClipType, MediaSource, Track, TransitionEdge, TransitionKind};
use uuid::Uuid;

/// A gain ramp over `[start_us, end_us]`, outside which it is 0 or 1 to match its end.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ramp {
    pub start_us: i64,
    pub end_us: i64,
    /// Rising (0 to 1) for the incoming side, falling for the outgoing one.
    pub rising: bool,
    pub equal_power: bool,
}

impl Ramp {
    /// Progress through the ramp for the gain of the side it applies to, 0.0 to 1.0.
    pub fn level(&self, p: f64) -> f64 {
        let x = if self.rising { p } else { 1.0 - p };
        if self.equal_power {
            (x * std::f64::consts::FRAC_PI_2).sin()
        } else {
            x
        }
    }

    pub fn gain_at(&self, time_us: i64) -> f32 {
        let span = (self.end_us - self.start_us).max(1) as f64;
        self.level(((time_us - self.start_us) as f64 / span).clamp(0.0, 1.0)) as f32
    }
}

/// One audio track as strips of clips (each strip plays at most one clip at a time) and
/// the ramps to apply to clips by id.
#[derive(Debug, Clone, Default)]
pub struct AudioStrips {
    pub strips: Vec<Vec<Clip>>,
    pub ramps: HashMap<Uuid, Vec<Ramp>>,
}

impl AudioStrips {
    /// The product of the ramps on a clip at a time.
    pub fn gain(ramps: &HashMap<Uuid, Vec<Ramp>>, clip: &Clip, time_us: i64) -> f32 {
        ramps.get(&clip.id).map_or(1.0, |rs| rs.iter().map(|r| r.gain_at(time_us)).product())
    }
}

fn head_room(clip: &Clip) -> i64 {
    clip.to_timeline_floor_us(clip.source_in).max(0)
}

fn tail_room(clip: &Clip, sources: &HashMap<Uuid, MediaSource>) -> i64 {
    sources.get(&clip.source_id).map_or(i64::MAX, |s| clip.to_timeline_floor_us((s.duration_us - clip.source_out).max(0)))
}

/// A copy of `clip` playing `[from, to)` of the timeline, from `source_in`.
fn piece(clip: &Clip, from: i64, to: i64, source_in: i64) -> Clip {
    let mut p = clip.clone();
    p.id = Uuid::new_v4();
    p.timeline_in = from;
    p.timeline_out = to;
    p.source_in = source_in;
    p.source_out = source_in + clip.to_source_us(to - from);
    p.properties.fade_in_us = 0;
    p.properties.fade_out_us = 0;
    p.properties.dissolve_in_us = 0;
    p
}

/// Builds the strips and ramps for an audio track. Strip 0 is the track's clips; the
/// next two hold the handle pieces before and after the cuts. (shortcut: pieces of two
/// transitions that overlap each other in one strip are not both heard; real overlaps need
/// transitions shorter than a clip, which the model limits.)
pub fn audio_strips(track: &Track, sources: &HashMap<Uuid, MediaSource>) -> AudioStrips {
    let mut out = AudioStrips { strips: vec![track.clips.clone(), Vec::new(), Vec::new()], ramps: HashMap::new() };
    let mut ramp = |id: Uuid, r: Ramp| out.ramps.entry(id).or_default().push(r);
    let (mut heads, mut tails) = (Vec::new(), Vec::new());
    for t in track.transitions.iter().filter(|t| t.kind != TransitionKind::Cut) {
        let Some(clip) = track.find_clip(t.clip_id) else { continue };
        let Some((start_us, end_us)) = track.transition_span(t) else { continue };
        if end_us <= start_us {
            continue;
        }
        let outgoing = (t.edge == TransitionEdge::In).then(|| track.clips.iter().find(|c| c.id != clip.id && c.timeline_out == clip.timeline_in)).flatten();
        let r = |rising, equal_power| Ramp { start_us, end_us, rising, equal_power };
        let Some(outgoing) = outgoing else {
            ramp(clip.id, r(t.edge == TransitionEdge::In, false));
            continue;
        };
        ramp(outgoing.id, r(false, true));
        ramp(clip.id, r(true, true));
        let cut = clip.timeline_in;
        let before = (cut - start_us).min(head_room(clip));
        if before > 0 && clip.clip_type == ClipType::Audio {
            let p = piece(clip, cut - before, cut, (clip.source_in - clip.to_source_us(before)).max(0));
            ramp(p.id, r(true, true));
            heads.push(p);
        }
        let after = (end_us - cut).min(tail_room(outgoing, sources));
        if after > 0 && outgoing.clip_type == ClipType::Audio {
            let p = piece(outgoing, cut, cut + after, outgoing.source_out);
            ramp(p.id, r(false, true));
            tails.push(p);
        }
    }
    out.strips[1] = heads;
    out.strips[2] = tails;
    out.strips.retain(|s| !s.is_empty());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_power_ramps_keep_constant_power() {
        let (up, down) = (Ramp { start_us: 0, end_us: 100, rising: true, equal_power: true }, Ramp { rising: false, ..Ramp { start_us: 0, end_us: 100, rising: true, equal_power: true } });
        for t in [0, 25, 50, 75, 100] {
            let (a, b) = (up.gain_at(t), down.gain_at(t));
            assert!((a * a + b * b - 1.0).abs() < 1e-5, "{t}: {a} {b}");
        }
        assert_eq!(down.gain_at(-5), 1.0);
        assert_eq!(up.gain_at(500), 1.0);
    }
}
