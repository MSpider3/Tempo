//! Real timeline export. The cut is described to FFmpeg as a filter graph, so
//! FFmpeg reads the original files, composites the tracks, mixes the audio and
//! encodes. No frames pass through this process.
//!
//! FFmpeg keeps every input of a graph open at once, and each one costs memory.
//! A timeline with many cuts is therefore rendered in sections of a few clips
//! each, which are then joined without encoding the picture again. Memory use
//! stays the same however long the timeline is.

use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};

use tempo_timeline::{Clip, ColorLayer, ClipProperties, ClipType, MediaType, PictureBlend, Project, TitleData, TitleType, TrackKind};

use crate::chapters::{ffmetadata, Chapter};
use crate::error::{ExportError, Result};

/// What to do when the output shape differs from the footage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fit {
    /// Keep the whole picture; add black bars.
    Fit,
    /// Fill the frame; crop the overflow.
    Fill,
}

#[derive(Debug, Clone)]
pub struct TimelineExport {
    pub width: u32,
    pub height: u32,
    /// Lower is better quality and a larger file (x264 CRF).
    pub crf: u8,
    pub audio_kbps: u32,
    pub fit: Fit,
    /// `None` exports the whole timeline.
    pub range: Option<(i64, i64)>,
    pub chapters: Vec<Chapter>,
    pub output: PathBuf,
    /// Try the graphics chip's H.264 encoder (VA-API) first. If it cannot be
    /// used, the export is done again with the software encoder.
    pub hardware: bool,
    /// Cap the picture at this many kb/s instead of using `crf`, to make a file fit a
    /// size limit (see `bitrate_for_size`). Always encoded on the processor.
    pub max_video_kbps: Option<u32>,
}

/// The picture bitrate that keeps `duration_us` of video plus `audio_kbps` of sound
/// under `target_bytes`, leaving 5 % for the container and the encoder's overshoot.
/// `None` when that would be under 200 kb/s, too little to watch.
pub fn bitrate_for_size(target_bytes: u64, duration_us: i64, audio_kbps: u32) -> Option<u32> {
    if duration_us <= 0 {
        return None;
    }
    let total_kbps = (target_bytes as f64 * 8.0 * 0.95 / 1000.0 / (duration_us as f64 / 1e6)).floor() as i64;
    u32::try_from(total_kbps - audio_kbps as i64).ok().filter(|kbps| *kbps >= 200)
}

fn secs(us: i64) -> String {
    format!("{:.6}", us as f64 / 1_000_000.0)
}

fn is_default_transform(p: &ClipProperties) -> bool {
    p.opacity >= 0.999
        && p.position_x == 0.0
        && p.position_y == 0.0
        && (p.scale_x - 1.0).abs() < 1e-4
        && (p.scale_y - 1.0).abs() < 1e-4
        && p.rotation == 0.0
}

/// A clip cut down to the export range, in range-relative time.
struct Piece<'a> {
    clip: &'a Clip,
    start: i64,
    end: i64,
    source_in: i64,
}

fn pieces<'a>(clips: &'a [Clip], r0: i64, r1: i64, kinds: &[ClipType]) -> Vec<Piece<'a>> {
    let mut out: Vec<Piece> = clips
        .iter()
        .filter(|c| c.properties.enabled && kinds.contains(&c.clip_type) && c.timeline_out > r0 && c.timeline_in < r1)
        .map(|c| {
            let start = c.timeline_in.max(r0);
            Piece { clip: c, start: start - r0, end: c.timeline_out.min(r1) - r0, source_in: c.source_offset_at(start) }
        })
        .collect();
    out.sort_by_key(|p| p.start);
    out
}

/// FFmpeg filters for the clip's own filters (colour and blur), to follow `format=rgba`.
fn effect_filters(props: &ClipProperties, height: f32) -> String {
    let fx = tempo_timeline::resolve_effects(&props.effects);
    let mut out = String::new();
    if !fx.color.is_identity() {
        let m = &fx.color.matrix;
        out.push_str(&format!(
            ",colorchannelmixer=rr={:.5}:rg={:.5}:rb={:.5}:ra={:.5}:gr={:.5}:gg={:.5}:gb={:.5}:ga={:.5}:br={:.5}:bg={:.5}:bb={:.5}:ba={:.5}",
            m[0][0], m[0][1], m[0][2], m[0][3], m[1][0], m[1][1], m[1][2], m[1][3], m[2][0], m[2][1], m[2][2], m[2][3]
        ));
        let o = &fx.color.offset;
        if o[..3].iter().any(|v| v.abs() > 1e-5) {
            // The mixer has no constant term, so add the offsets with a lookup table.
            out.push_str(&format!(
                ",lutrgb=r='clip(val+{:.2},0,255)':g='clip(val+{:.2},0,255)':b='clip(val+{:.2},0,255)'",
                o[0] * 255.0,
                o[1] * 255.0,
                o[2] * 255.0
            ));
        }
    }
    if fx.blur > 0.05 {
        // The radius is given for a 1080-line project, scaled to the output as the viewer does.
        out.push_str(&format!(",gblur=sigma={:.2}", tempo_timeline::blur_sigma(tempo_timeline::blur_radius_px(fx.blur, height)).max(0.1)));
    }
    out
}

/// `volume` is evaluated once per audio frame; 64-sample frames make a ramp rise in steps of
/// about a millisecond, which cannot be heard.
const SMOOTH_GAIN: &str = ",asetnsamples=n=64:p=0";

/// Sound fade of a piece as a per-frame `volume`, measured against the whole clip so an export
/// range that starts or ends inside a fade still hears the rest of the ramp.
fn audio_fade_filters(p: &Piece, r0: i64) -> String {
    let props = &p.clip.properties;
    let abs = p.start + r0;
    let mut terms = Vec::new();
    if props.fade_in_us > 0 {
        terms.push(format!("clip((t+({}))/{},0,1)", secs(abs - p.clip.timeline_in), secs(props.fade_in_us)));
    }
    if props.fade_out_us > 0 {
        terms.push(format!("clip(({}-t)/{},0,1)", secs(p.clip.timeline_out - abs), secs(props.fade_out_us)));
    }
    match terms.len() {
        0 => String::new(),
        1 => format!("{SMOOTH_GAIN},volume='{}':eval=frame", terms[0]),
        _ => format!("{SMOOTH_GAIN},volume='min({},{})':eval=frame", terms[0], terms[1]),
    }
}

/// Per-frame opacity of a picture: `opacity_at` takes the absolute timeline time of each of
/// the `frames` output frames from output frame `first`, and the value is set only where it
/// changes. Same functions as the player uses, so a ranged export shows what the player shows.
/// Needs rgba input. Empty when the opacity stays 1.
fn opacity_filters(name: &str, first: i64, frames: i64, r0: i64, project: &Project, opacity_at: impl Fn(i64) -> f32) -> String {
    per_frame(&format!("colorchannelmixer@{name}"), "aa", 1.0, first, frames, r0, project, |t| opacity_at(t).clamp(0.0, 1.0))
}

/// `filter` with its `param` set for each output frame from `value_at` (as `opacity_filters`),
/// sent only where it changes from `rest`, the value it starts at. Empty when it never does.
#[allow(clippy::too_many_arguments)]
fn per_frame(filter: &str, param: &str, rest: f32, first: i64, frames: i64, r0: i64, project: &Project, value_at: impl Fn(i64) -> f32) -> String {
    let (mut cmds, mut last) = (String::new(), rest);
    for i in 0..frames {
        let v = value_at(r0 + frame_time(first + i, project));
        if (v - last).abs() > 1e-4 {
            cmds.push_str(&format!("{} {filter} {param} {v:.4};", secs(frame_time(i, project))));
            last = v;
        }
    }
    if cmds.is_empty() {
        return String::new();
    }
    format!(",sendcmd=c='{cmds}',{filter}={param}={rest}")
}

/// A Blur Dissolve's per-frame blur on a picture, a Gaussian as for the clip's own blur filter
/// (its 1080-line radius scaled to `height`, as the viewer does). Empty when the clip has none.
fn transition_blur_filters(name: &str, clip: &Clip, first: i64, frames: i64, r0: i64, project: &Project, height: f32) -> String {
    if clip.properties.blur_ramps.is_empty() {
        return String::new();
    }
    per_frame(&format!("gblur@{name}"), "sigma", 0.0, first, frames, r0, project, |t| tempo_timeline::blur_sigma(tempo_timeline::blur_radius_px(clip.transition_blur(t), height)))
}

/// The FFmpeg `blend` mode for a picture blend; `None` for a plain overlay.
fn blend_mode(blend: PictureBlend) -> Option<&'static str> {
    match blend {
        PictureBlend::Normal => None,
        PictureBlend::Screen => Some("screen"),
        PictureBlend::Lighten => Some("lighten"),
    }
}

/// Gain ramps of audio transitions as `volume` filters evaluated per frame.
fn ramp_filters(p: &Piece, r0: i64, ramps: &std::collections::HashMap<uuid::Uuid, Vec<crate::audio_mix::Ramp>>) -> String {
    let mut out = String::new();
    for r in ramps.get(&p.clip.id).into_iter().flatten() {
        let (offset, span) = (secs(p.start + r0 - r.start_us), secs((r.end_us - r.start_us).max(1)));
        let x = format!("clip((t+({offset}))/{span},0,1)");
        let x = if r.rising { x } else { format!("(1-{x})") };
        let gain = if r.equal_power { format!("sin(1.5707963*{x})") } else { x };
        out.push_str(&format!("{SMOOTH_GAIN},volume='{gain}':eval=frame"));
    }
    out
}

/// A full-frame solid colour over `layer`'s span with its opacity ramps, as a strip as long
/// as the output. Returns its label.
fn color_strip(
    graph: &mut Vec<String>,
    ti: usize,
    layer: &ColorLayer,
    (r0, r1): (i64, i64),
    (w, h): (u32, u32),
    fps: &str,
    project: &Project,
) -> Option<String> {
    let (first, end) = (to_frame(layer.start_us.max(r0) - r0, project), to_frame(layer.end_us.min(r1) - r0, project));
    let frames = end - first;
    if frames <= 0 {
        return None;
    }
    let [r, g, b] = layer.color.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
    let fades = opacity_filters(&format!("dip{ti}"), first, frames, r0, project, |t| layer.opacity_at(t));
    let solid = format!("color=c=0x{r:02x}{g:02x}{b:02x}:s={w}x{h}:r={fps},trim=end_frame={frames},format=rgba{fades}");
    let out = format!("vt{ti}");
    if first > 0 {
        graph.push(format!("color=c=black@0.0:s={w}x{h}:r={fps},trim=end_frame={first},format=rgba[vg{ti}]"));
        graph.push(format!("{solid}[vc{ti}]"));
        graph.push(format!("[vg{ti}][vc{ti}]concat=n=2:v=1:a=0[{out}]"));
    } else {
        graph.push(format!("{solid}[{out}]"));
    }
    Some(out)
}

/// The `drawtext` filter for a title. The text itself goes in a file, which
/// avoids every quoting problem in the filter graph. `expansion=none` prints
/// the text as written: FFmpeg 9 refuses a title holding a lone `%` otherwise.
fn title_filter(title: &TitleData, props: &ClipProperties, text_file: &std::path::Path, sx: f32, sy: f32) -> String {
    let [r, g, b, a] = title.color;
    let alpha = a as f32 / 255.0 * props.opacity.clamp(0.0, 1.0);
    let size = (title.font_size * sy * props.scale_y.max(0.01)).max(1.0);
    let style = match (title.font_bold, title.font_italic) {
        (true, true) => "\\:bold\\:italic",
        (true, false) => "\\:bold",
        (false, true) => "\\:italic",
        (false, false) => "",
    };
    let y = match title.title_type {
        TitleType::CenterTitle => "(h-text_h)/2",
        TitleType::LowerThird => "h*0.8-text_h/2",
    };
    let mut f = format!(
        "drawtext=textfile='{}':expansion=none:font='{}{style}':fontsize={size:.1}:fontcolor=0x{r:02x}{g:02x}{b:02x}@{alpha:.3}:x=(w-text_w)/2+{:.1}:y={y}+{:.1}",
        text_file.display(),
        title.font_family.replace(['\'', ':', '\\'], ""),
        props.position_x * sx,
        props.position_y * sy,
    );
    if let Some([r, g, b, a]) = title.background_color {
        f.push_str(&format!(
            ":box=1:boxcolor=0x{r:02x}{g:02x}{b:02x}@{:.3}:boxborderw={:.0}",
            a as f32 / 255.0 * props.opacity.clamp(0.0, 1.0),
            title.background_padding * sy
        ));
    }
    f
}

/// Build the complete FFmpeg command line (without the program name).
/// Returns the arguments, the duration of the output in microseconds, and the
/// text files (path, contents) that must exist while FFmpeg runs.
pub fn build_ffmpeg_args(
    project: &Project,
    e: &TimelineExport,
    metadata_file: Option<&PathBuf>,
) -> Result<(Vec<String>, i64, Vec<(PathBuf, String)>)> {
    build_args(project, e, metadata_file, None)
}

/// The output frame a time falls on, counting from the start of the export.
/// The picture is laid out in whole frames, so cuts that are not on a frame
/// never add up to a drift between picture and sound.
fn to_frame(us: i64, project: &Project) -> i64 {
    let (num, den) = (project.timeline_fps().num.max(1) as i128, project.timeline_fps().den.max(1) as i128);
    ((us as i128 * num + den * 500_000) / (den * 1_000_000)) as i64
}

/// The time at which an output frame starts.
fn frame_time(frame: i64, project: &Project) -> i64 {
    let (num, den) = (project.timeline_fps().num.max(1) as i128, project.timeline_fps().den.max(1) as i128);
    ((frame as i128 * den * 1_000_000 + num / 2) / num) as i64
}

/// A section's length rounded to whole frames, so its picture and sound are
/// exactly as long as each other and sections join without a gap.
fn whole_frames(us: i64, project: &Project) -> i64 {
    frame_time(to_frame(us, project).max(1), project)
}

/// `section` is set when this is one part of a longer export: `Some(true)` if
/// the export has sound, so that every part carries a sound stream.
fn build_args(
    project: &Project,
    e: &TimelineExport,
    metadata_file: Option<&PathBuf>,
    section: Option<bool>,
) -> Result<(Vec<String>, i64, Vec<(PathBuf, String)>)> {
    // A size limit always uses the processor's encoder, which keeps to a bitrate.
    let hardware = e.hardware && e.max_video_kbps.is_none();
    let mut text_files: Vec<(PathBuf, String)> = Vec::new();
    let sx = ((e.width.max(16) + 1) & !1) as f32 / project.output_size().0.max(1) as f32;
    let sy = ((e.height.max(16) + 1) & !1) as f32 / project.output_size().1.max(1) as f32;
    let timeline_end = project.timeline.tracks.iter().map(|t| t.duration_us()).max().unwrap_or(0);
    let (r0, r1) = e.range.unwrap_or((0, timeline_end));
    let (r0, r1) = (r0.max(0), r1.min(timeline_end));
    if r1 - r0 <= 0 {
        return Err(ExportError::InvalidParameter("There is nothing on the timeline to export.".into()));
    }
    let total = if section.is_some() { whole_frames(r1 - r0, project) } else { r1 - r0 };

    let (w, h) = ((e.width.max(16) + 1) & !1, (e.height.max(16) + 1) & !1);
    let fps = format!("{}/{}", project.timeline_fps().num.max(1), project.timeline_fps().den.max(1));
    let mut inputs: Vec<String> = Vec::new();
    let mut graph: Vec<String> = Vec::new();
    let mut n_inputs = 0usize;

    let source_path = |clip: &Clip| -> Result<(PathBuf, MediaType)> {
        let s = project
            .sources
            .get(&clip.source_id)
            .ok_or_else(|| ExportError::InvalidParameter(format!("Clip \"{}\" has no media.", clip.name)))?;
        if s.is_missing || !s.path.exists() {
            return Err(ExportError::InvalidParameter(format!("Media offline: {}", s.path.display())));
        }
        // Always the original file, never a proxy.
        Ok((s.path.clone(), s.media_type))
    };

    // ---- Video: one concatenated strip per track, then stacked bottom to top.
    let mut video_tracks: Vec<_> = project.timeline.tracks.iter().filter(|t| t.kind == TrackKind::Video && t.enabled).collect();
    video_tracks.sort_by_key(|t| t.kind_index);
    // Per track: its clips, the handle footage of its dissolves as one more strip, then the
    // solid colours of its dips (they decode nothing).
    enum Strip {
        Clips(Vec<Clip>, PictureBlend),
        Color(ColorLayer),
    }
    let mut video_strips: Vec<Strip> = Vec::new();
    for track in &video_tracks {
        let layers = track.transition_layers(&project.sources);
        video_strips.push(Strip::Clips(layers.clips, PictureBlend::Normal));
        // One strip per blend: a track's mix pieces never overlap in time, so the order
        // between these strips does not matter.
        for blend in [PictureBlend::Normal, PictureBlend::Screen, PictureBlend::Lighten] {
            let mix: Vec<Clip> = layers.mix.iter().filter(|c| c.properties.blend == blend).cloned().collect();
            if !mix.is_empty() {
                video_strips.push(Strip::Clips(mix, blend));
            }
        }
        video_strips.extend(layers.color.into_iter().map(Strip::Color));
    }
    let mut track_labels = Vec::new();
    for (ti, strip) in video_strips.iter().enumerate() {
        let (strip, blend) = match strip {
            Strip::Clips(clips, blend) => (clips, *blend),
            Strip::Color(layer) => {
                if let Some(label) = color_strip(&mut graph, ti, layer, (r0, r1), (w, h), &fps, project) {
                    track_labels.push((label, PictureBlend::Normal));
                }
                continue;
            }
        };
        let parts = pieces(strip, r0, r1, &[ClipType::Video, ClipType::Image, ClipType::Title]);
        if parts.is_empty() {
            continue;
        }
        let mut segs = Vec::new();
        // Everything on this strip is measured in output frames.
        let mut cursor = 0i64;
        let blank = |frames: i64| format!("color=c=black@0.0:s={w}x{h}:r={fps},trim=end_frame={frames},format=rgba");
        for p in &parts {
            let (first, end) = (to_frame(p.start, project).max(cursor), to_frame(p.end, project));
            let frames = end - first;
            if frames <= 0 {
                // Shorter than a frame: it has no picture of its own.
                continue;
            }
            if first > cursor {
                let label = format!("vg{ti}_{}", segs.len());
                graph.push(format!("{}[{label}]", blank(first - cursor)));
                segs.push(label);
            }
            let len = p.end - p.start;
            let fades = opacity_filters(&format!("fd{ti}_{}", segs.len()), first, frames, r0, project, |t| p.clip.fade_factor(t));
            if p.clip.clip_type == ClipType::Title {
                let Some(title) = &p.clip.title_data else { continue };
                let file = std::env::temp_dir().join(format!("tempo-title-{}-{}-{}.txt", std::process::id(), r0, text_files.len()));
                let label = format!("vc{ti}_{}", segs.len());
                graph.push(format!("{},{}{fades}[{label}]", blank(frames), title_filter(title, &p.clip.properties, &file, sx, sy)));
                text_files.push((file, title.text.clone()));
                segs.push(label);
                cursor = end;
                continue;
            }
            let (path, media) = source_path(p.clip)?;
            if media == MediaType::Image {
                inputs.extend(["-loop".into(), "1".into(), "-t".into(), secs(len)]);
            } else {
                // One frame more than needed, so rounding never leaves the clip short.
                inputs.extend(["-ss".into(), secs(p.source_in), "-t".into(), secs(p.clip.to_source_us(len + frame_time(1, project)))]);
            }
            inputs.extend(["-i".into(), path.to_string_lossy().into_owned()]);
            let idx = n_inputs;
            n_inputs += 1;

            let props = &p.clip.properties;
            let fit = match e.fit {
                Fit::Fit => format!("scale={w}:{h}:force_original_aspect_ratio=decrease"),
                Fit::Fill => format!("scale={w}:{h}:force_original_aspect_ratio=increase,crop={w}:{h}"),
            };
            let label = format!("vc{ti}_{}", segs.len());
            // Blur radii are in pixels of a 1080-line picture.
            let effects = effect_filters(props, h as f32);
            let blur = transition_blur_filters(&format!("bl{ti}_{}", segs.len()), p.clip, first, frames, r0, project, h as f32);
            // A retimed clip's pictures are spread out (or packed together) before the
            // frame rate is set, so slow motion repeats frames and fast motion skips them.
            let retime = if props.speed != 1.0 { format!("setpts=(PTS-STARTPTS)/{:.6},", props.speed) } else { String::new() };
            // If the file ends early its last picture is held, so the clip always
            // gives exactly the frames the timeline has for it.
            let head = format!(
                "[{idx}:v]{retime}fps={fps},tpad=stop_mode=clone:stop_duration=2,{fit},setsar=1,format=rgba{effects},trim=end_frame={frames},setpts=PTS-STARTPTS{blur}"
            );
            if is_default_transform(props) {
                graph.push(format!("{head},pad={w}:{h}:(ow-iw)/2:(oh-ih)/2:color=black@0.0{fades}[{label}]"));
            } else {
                // Zoom, rotate and fade the clip, then place it on a transparent canvas.
                let mut chain = head;
                if (props.scale_x - 1.0).abs() > 1e-4 || (props.scale_y - 1.0).abs() > 1e-4 {
                    chain.push_str(&format!(",scale=iw*{:.4}:ih*{:.4}", props.scale_x.max(0.01), props.scale_y.max(0.01)));
                }
                if props.rotation != 0.0 {
                    let a = props.rotation.to_radians();
                    chain.push_str(&format!(",rotate={a:.6}:c=none:ow=rotw({a:.6}):oh=roth({a:.6})"));
                }
                if props.opacity < 0.999 {
                    chain.push_str(&format!(",colorchannelmixer=aa={:.4}", props.opacity.clamp(0.0, 1.0)));
                }
                // Inspector positions are in project pixels; `sx`/`sy` scale them to the output size.
                graph.push(format!("{chain}[{label}s]"));
                graph.push(format!("{}[{label}b]", blank(frames)));
                graph.push(format!(
                    "[{label}b][{label}s]overlay=x=(W-w)/2+{:.2}:y=(H-h)/2+{:.2}:shortest=1,format=rgba{fades}[{label}]",
                    props.position_x * sx,
                    props.position_y * sy
                ));
            }
            segs.push(label);
            cursor = end;
        }
        if segs.is_empty() {
            continue;
        }
        let out = format!("vt{ti}");
        if segs.len() == 1 {
            graph.push(format!("[{}]null[{out}]", segs[0]));
        } else {
            let joined: String = segs.iter().map(|s| format!("[{s}]")).collect();
            graph.push(format!("{joined}concat=n={}:v=1:a=0[{out}]", segs.len()));
        }
        track_labels.push((out, blend));
    }

    graph.push(format!("color=c=black:s={w}x{h}:r={fps},trim=end_frame={}[base0]", to_frame(total, project).max(1)));
    let mut base = "base0".to_string();
    for (i, (label, blend)) in track_labels.iter().enumerate() {
        let next = format!("base{}", i + 1);
        if let Some(mode) = blend_mode(*blend) {
            // `under + alpha * (mode(under, picture) - under)`, as GTK blends in the viewer:
            // blend the colours, give the result the strip's alpha, lay it over the picture.
            graph.push(format!("[{base}]split[{next}u][{next}o]"));
            graph.push(format!("[{label}]split[{next}p][{next}q]"));
            graph.push(format!("[{next}u]format=gbrap[{next}ug]"));
            graph.push(format!("[{next}p]format=gbrap[{next}pg]"));
            graph.push(format!("[{next}ug][{next}pg]blend=all_mode={mode}:shortest=1[{next}m]"));
            graph.push(format!("[{next}q]alphaextract[{next}a]"));
            graph.push(format!("[{next}m][{next}a]alphamerge[{next}s]"));
            graph.push(format!("[{next}o][{next}s]overlay=eof_action=pass:format=auto[{next}]"));
        } else {
            graph.push(format!("[{base}][{label}]overlay=eof_action=pass:format=auto[{next}]"));
        }
        base = next;
    }
    if hardware {
        graph.push(format!("[{base}]format=nv12,hwupload[vout]"));
    } else {
        graph.push(format!("[{base}]format=yuv420p[vout]"));
    }

    // ---- Audio: one strip per track, then mixed.
    let mut audio_tracks: Vec<_> = project.timeline.tracks.iter().filter(|t| t.kind == TrackKind::Audio && t.enabled).collect();
    audio_tracks.sort_by_key(|t| t.kind_index);
    // Audio transitions are gain ramps on the clips (see `audio_mix`).
    let mut audio_strips: Vec<(Vec<Clip>, f32, bool)> = Vec::new();
    let mut ramps = std::collections::HashMap::new();
    for track in &audio_tracks {
        let built = crate::audio_mix::audio_strips(track, &project.sources);
        let mono = track.audio_type == tempo_timeline::types::AudioTrackType::Mono;
        audio_strips.extend(built.strips.into_iter().map(|s| (s, track.volume, mono)));
        ramps.extend(built.ramps);
    }
    let mut audio_labels = Vec::new();
    for (ti, (strip, track_volume, mono)) in audio_strips.iter().enumerate() {
        let parts = pieces(strip, r0, r1, &[ClipType::Audio]);
        if parts.is_empty() {
            continue;
        }
        let mut segs = Vec::new();
        let mut cursor = 0i64;
        for p in &parts {
            if p.start > cursor {
                let label = format!("ag{ti}_{}", segs.len());
                graph.push(format!("anullsrc=r=48000:cl=stereo,atrim=duration={}[{label}]", secs(p.start - cursor)));
                segs.push(label);
            }
            let len = p.end - p.start;
            let (path, _) = source_path(p.clip)?;
            inputs.extend(["-ss".into(), secs(p.source_in), "-t".into(), secs(p.clip.to_source_us(len)), "-i".into(), path.to_string_lossy().into_owned()]);
            let idx = n_inputs;
            n_inputs += 1;
            let props = &p.clip.properties;
            // Sound plays faster or slower with the picture, and its pitch moves with it,
            // as in Resolve on Linux and Windows and as Tempo's preview plays it.
            let retime = if props.speed != 1.0 { format!("asetrate={},aresample=48000,", (48_000.0 * props.speed).round() as u32) } else { String::new() };
            let volume = if props.muted { 0.0 } else { props.volume.max(0.0) * track_volume.max(0.0) };
            let (left, right) = (1.0 - props.pan.max(0.0), 1.0 + props.pan.min(0.0));
            let label = format!("ac{ti}_{}", segs.len());
            // A Mono track plays centred: both sides get the average.
            let centre = if *mono { ",pan=stereo|c0=0.5*c0+0.5*c1|c1=0.5*c0+0.5*c1" } else { "" };
            graph.push(format!(
                "[{idx}:a]aresample=48000,{retime}aformat=sample_fmts=fltp:channel_layouts=stereo{centre},atrim=duration={},asetpts=PTS-STARTPTS,\
                 volume={volume:.4},pan=stereo|c0={left:.4}*c0|c1={right:.4}*c1{}{}[{label}]",
                secs(len),
                audio_fade_filters(p, r0),
                ramp_filters(p, r0, &ramps)
            ));
            segs.push(label);
            cursor = p.end;
        }
        let out = format!("at{ti}");
        if segs.len() == 1 {
            graph.push(format!("[{}]anull[{out}]", segs[0]));
        } else {
            let joined: String = segs.iter().map(|s| format!("[{s}]")).collect();
            graph.push(format!("{joined}concat=n={}:v=0:a=1[{out}]", segs.len()));
        }
        audio_labels.push(out);
    }
    let has_audio = !audio_labels.is_empty() || section == Some(true);
    if audio_labels.is_empty() && has_audio {
        // A silent part of an export that has sound elsewhere.
        graph.push(format!("anullsrc=r=48000:cl=stereo,atrim=duration={}[aout]", secs(total)));
    } else if has_audio {
        let joined: String = audio_labels.iter().map(|s| format!("[{s}]")).collect();
        graph.push(format!("{joined}amix=inputs={}:normalize=0:duration=longest,alimiter=limit=0.97,apad[aout]", audio_labels.len()));
    }

    // ---- Command line.
    let mut args: Vec<String> = vec!["-y".into(), "-hide_banner".into(), "-nostats".into(), "-progress".into(), "pipe:1".into()];
    if hardware {
        args.extend(["-vaapi_device".into(), crate::ffmpeg::VAAPI_DEVICE.into()]);
    }
    args.extend(inputs);
    if let Some(meta) = metadata_file {
        args.extend(["-i".into(), meta.to_string_lossy().into_owned(), "-map_metadata".into(), n_inputs.to_string()]);
    }
    args.extend(["-filter_complex".into(), graph.join(";"), "-map".into(), "[vout]".into()]);
    if has_audio && section.is_some() {
        // Uncompressed in a section, so the joins are exact; encoded once at the end.
        args.extend(["-map".into(), "[aout]".into(), "-c:a".into(), "pcm_s16le".into()]);
    } else if has_audio {
        args.extend(["-map".into(), "[aout]".into(), "-c:a".into(), "aac".into(), "-b:a".into(), format!("{}k", e.audio_kbps.max(64))]);
    }
    match (e.max_video_kbps, hardware) {
        (Some(kbps), _) => args.extend(crate::ffmpeg::h264_capped_args(kbps)),
        // VA-API has no CRF; a fixed quantiser close to the CRF gives similar quality.
        (None, true) => args.extend(["-c:v".into(), "h264_vaapi".into(), "-qp".into(), (e.crf.min(51) as u32 + 2).to_string()]),
        (None, false) => args.extend(crate::ffmpeg::h264_args(e.crf, w, h, false)),
    }
    if section.is_some() {
        args.extend(["-f".into(), "matroska".into()]);
    } else {
        args.extend(["-movflags".into(), "+faststart".into()]);
    }
    args.extend(["-t".into(), secs(total), e.output.to_string_lossy().into_owned()]);
    Ok((args, total, text_files))
}

/// Most files one FFmpeg run may have open. Each costs about 25 MB with
/// high-resolution footage, on top of about 500 MB for the run itself.
const MAX_INPUTS: usize = 6;

/// Split `[r0, r1)` into sections that each read at most `MAX_INPUTS` files.
/// Sections meet at the output frame nearest a clip edge, and never inside a
/// fade or dissolve, so a join is not visible or audible. One section means
/// "no need to split".
fn plan_sections(project: &Project, r0: i64, r1: i64) -> Vec<(i64, i64)> {
    let clips: Vec<&Clip> = project
        .timeline
        .tracks
        .iter()
        .filter(|t| t.enabled)
        .flat_map(|t| t.clips.iter())
        .filter(|c| c.properties.enabled && c.timeline_out > r0 && c.timeline_in < r1)
        .collect();
    let inputs = |a: i64, b: i64| clips.iter().filter(|c| c.clip_type != ClipType::Title && c.timeline_out > a && c.timeline_in < b).count();
    if inputs(r0, r1) <= MAX_INPUTS {
        return vec![(r0, r1)];
    }
    // Times a join must not fall inside.
    let busy: Vec<(i64, i64)> = clips
        .iter()
        .flat_map(|c| {
            let p = &c.properties;
            [
                (c.timeline_in, c.timeline_in + p.fade_in_us),
                (c.timeline_out - p.fade_out_us, c.timeline_out),
            ]
        })
        .chain(
            project
                .timeline
                .tracks
                .iter()
                .filter(|t| t.enabled)
                .flat_map(|t| t.transitions.iter().filter_map(|tr| t.transition_span(tr))),
        )
        .filter(|(a, b)| b > a)
        .collect();
    let mut edges: Vec<i64> = clips
        .iter()
        .flat_map(|c| [c.timeline_in, c.timeline_out])
        .map(|t| r0 + frame_time(to_frame(t - r0, project), project))
        .filter(|t| *t > r0 && *t < r1 && !busy.iter().any(|(a, b)| t > a && t < b))
        .collect();
    edges.sort_unstable();
    edges.dedup();

    let mut sections = Vec::new();
    let mut start = r0;
    while inputs(start, r1) > MAX_INPUTS {
        let mut later = edges.iter().copied().filter(|t| *t > start);
        let Some(first) = later.next() else { break };
        // The furthest edge that keeps the section within the limit; if even the
        // nearest one does not (many tracks), take it anyway.
        let end = later.take_while(|t| inputs(start, *t) <= MAX_INPUTS).last().unwrap_or(first);
        sections.push((start, end));
        start = end;
    }
    sections.push((start, r1));
    sections
}

/// Run the export. `progress` receives 0.0–1.0. Setting `cancel` stops FFmpeg
/// and removes the partial file. Blocking: call from a worker thread.
pub fn export_timeline(project: &Project, e: &TimelineExport, cancel: &AtomicBool, mut progress: impl FnMut(f32)) -> Result<()> {
    if e.hardware && e.max_video_kbps.is_none() {
        match run_ffmpeg(project, e, cancel, &mut progress) {
            Ok(()) => return Ok(()),
            Err(_) if cancel.load(Ordering::Relaxed) => return Err(ExportError::Ffmpeg("Cancelled".into())),
            Err(err) => {
                tracing::warn!("hardware encoder could not be used ({err}); exporting with the software encoder");
                let software = TimelineExport { hardware: false, ..e.clone() };
                return run_ffmpeg(project, &software, cancel, &mut progress);
            }
        }
    }
    run_ffmpeg(project, e, cancel, &mut progress)
}

fn run_ffmpeg(project: &Project, e: &TimelineExport, cancel: &AtomicBool, progress: &mut dyn FnMut(f32)) -> Result<()> {
    if let Some(dir) = e.output.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let timeline_end = project.timeline.tracks.iter().map(|t| t.duration_us()).max().unwrap_or(0);
    let range_len = e.range.map(|(a, b)| b.min(timeline_end) - a.max(0)).unwrap_or(timeline_end);

    let metadata_file = if e.chapters.is_empty() {
        None
    } else {
        // Numbered, so two exports in one process never share the file.
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!("tempo-chapters-{}-{}.txt", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::write(&path, ffmetadata(&e.chapters, range_len))?;
        Some(path)
    };
    let (r0, r1) = e.range.map(|(a, b)| (a.max(0), b.min(timeline_end))).unwrap_or((0, timeline_end));
    let sections = plan_sections(project, r0, r1);
    let result = if sections.len() > 1 {
        run_sections(project, e, &sections, metadata_file.as_ref(), cancel, progress)
    } else {
        build_ffmpeg_args(project, e, metadata_file.as_ref()).and_then(|(args, total, text_files)| run_one(&args, total, &text_files, cancel, progress))
    };
    if let Some(meta) = metadata_file {
        let _ = std::fs::remove_file(meta);
    }
    let cancelled = cancel.load(Ordering::Relaxed);
    if result.is_err() || cancelled {
        let _ = std::fs::remove_file(&e.output);
    }
    if cancelled {
        return Err(ExportError::Ffmpeg("Cancelled".into()));
    }
    result?;
    progress(1.0);
    Ok(())
}

/// Render each section to a file beside the output, then join them. The picture
/// is copied as it is; only the sound is encoded in the joining step.
fn run_sections(
    project: &Project,
    e: &TimelineExport,
    sections: &[(i64, i64)],
    metadata_file: Option<&PathBuf>,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(f32),
) -> Result<()> {
    let dir = e.output.with_extension("parts");
    std::fs::create_dir_all(&dir)?;
    let result = render_and_join(project, e, sections, metadata_file, &dir, cancel, progress);
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn render_and_join(
    project: &Project,
    e: &TimelineExport,
    sections: &[(i64, i64)],
    metadata_file: Option<&PathBuf>,
    dir: &std::path::Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(f32),
) -> Result<()> {
    let (r0, r1) = (sections[0].0, sections[sections.len() - 1].1);
    let whole = (r1 - r0).max(1) as f32;
    let has_audio = project
        .timeline
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Audio && t.enabled)
        .flat_map(|t| t.clips.iter())
        .any(|c| c.properties.enabled && c.clip_type == ClipType::Audio && c.timeline_out > r0 && c.timeline_in < r1);

    let mut list = String::new();
    for (i, (a, b)) in sections.iter().enumerate() {
        let name = format!("{i:05}.mkv");
        let part = TimelineExport { range: Some((*a, *b)), chapters: Vec::new(), output: dir.join(&name), ..e.clone() };
        let (args, total, text_files) = build_args(project, &part, None, Some(has_audio))?;
        // Rendering is nearly all of the work; the last 2 % is the join.
        let (done, share) = ((*a - r0) as f32 / whole, (*b - *a) as f32 / whole);
        run_one(&args, total, &text_files, cancel, &mut |f| progress((done + f * share) * 0.98))?;
        // Names are relative to the list, so no path needs escaping.
        list.push_str(&format!("file '{name}'\n"));
    }
    let list_file = dir.join("list.txt");
    std::fs::write(&list_file, list)?;

    let mut args: Vec<String> = vec!["-y".into(), "-hide_banner".into(), "-nostats".into(), "-progress".into(), "pipe:1".into()];
    args.extend(["-f".into(), "concat".into(), "-safe".into(), "0".into(), "-i".into(), list_file.to_string_lossy().into_owned()]);
    if let Some(meta) = metadata_file {
        args.extend(["-i".into(), meta.to_string_lossy().into_owned(), "-map_metadata".into(), "1".into()]);
    }
    args.extend(["-map".into(), "0:v:0".into(), "-c:v".into(), "copy".into()]);
    if has_audio {
        args.extend(["-map".into(), "0:a:0".into(), "-c:a".into(), "aac".into(), "-b:a".into(), format!("{}k", e.audio_kbps.max(64))]);
    }
    args.extend(["-movflags".into(), "+faststart".into(), e.output.to_string_lossy().into_owned()]);
    run_one(&args, r1 - r0, &[], cancel, &mut |f| progress(0.98 + f * 0.02))
}

/// Run FFmpeg once and follow its progress. The title text files are written
/// first and removed afterwards.
fn run_one(args: &[String], total: i64, text_files: &[(PathBuf, String)], cancel: &AtomicBool, progress: &mut dyn FnMut(f32)) -> Result<()> {
    for (path, text) in text_files {
        std::fs::write(path, text)?;
    }

    let mut command = Command::new("ffmpeg");
    command.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    crate::ffmpeg::die_with_parent(&mut command);
    // Leave the processor to the interface when both want it.
    crate::ffmpeg::low_priority(&mut command, 10);
    let mut child = command
        .spawn()
        .map_err(|err| ExportError::Ffmpeg(format!("Could not start ffmpeg: {err}")))?;

    // Drain stderr on its own thread so FFmpeg never blocks on a full pipe.
    let stderr = child.stderr.take();
    let errors = std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(mut s) = stderr {
            let _ = s.read_to_string(&mut text);
        }
        text
    });

    let mut cancelled = false;
    // FFmpeg 9 can report a slightly earlier time than its last line; the bar never goes back.
    let mut shown = 0.0f32;
    if let Some(stdout) = child.stdout.take() {
        for line in BufReader::new(stdout).lines().map_while(|l| l.ok()) {
            if cancel.load(Ordering::Relaxed) {
                cancelled = true;
                let _ = child.kill();
                break;
            }
            if let Some(us) = line.strip_prefix("out_time_us=").and_then(|v| v.trim().parse::<i64>().ok()) {
                let f = (us as f32 / total.max(1) as f32).clamp(0.0, 1.0);
                if f > shown {
                    shown = f;
                    progress(f);
                }
            }
        }
    }
    let status = child.wait()?;
    let stderr_text = errors.join().unwrap_or_default();
    for (path, _) in text_files {
        let _ = std::fs::remove_file(path);
    }

    if cancelled || cancel.load(Ordering::Relaxed) {
        return Err(ExportError::Ffmpeg("Cancelled".into()));
    }
    if !status.success() {
        let reason = stderr_text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("FFmpeg failed").to_string();
        tracing::error!("ffmpeg export failed:\n{stderr_text}");
        return Err(ExportError::Ffmpeg(reason));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempo_timeline::{MediaSource, RationalFps};

    fn sample() -> Option<PathBuf> {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/test-media/sample_1080p_h264.mp4");
        p.exists().then_some(p)
    }

    fn project_with_clip(path: PathBuf) -> Project {
        let mut project = Project::new("t", 1920, 1080, RationalFps::FPS_30);
        let mut source = MediaSource::new(path, MediaType::Video, 5_000_000);
        source.audio_channels = Some(2);
        let v = project.timeline.tracks.iter().find(|t| t.kind == TrackKind::Video).map(|t| t.id).unwrap();
        let a = project.timeline.tracks.iter().find(|t| t.kind == TrackKind::Audio).map(|t| t.id).unwrap();
        // Two pieces with a one-second gap between them.
        for (tin, sin) in [(0, 0), (2_000_000, 1_000_000)] {
            for (track, kind) in [(v, ClipType::Video), (a, ClipType::Audio)] {
                let clip = Clip::new(track, source.id, kind, "c", tin, tin + 1_000_000, sin, sin + 1_000_000);
                project.timeline.find_track_mut(track).unwrap().clips.push(clip);
            }
        }
        project.sources.insert(source.id, source);
        project
    }

    fn settings(output: PathBuf) -> TimelineExport {
        TimelineExport { width: 640, height: 360, crf: 30, audio_kbps: 96, fit: Fit::Fit, range: None, chapters: Vec::new(), output, hardware: false, max_video_kbps: None }
    }

    #[test]
    fn frame_helpers_use_the_timelines_own_rate() {
        let mut project = Project::new("t", 1920, 1080, RationalFps::FPS_30);
        project.timeline.fps = Some(RationalFps::FPS_25);
        assert_eq!(to_frame(1_000_000, &project), 25);
        assert_eq!(frame_time(25, &project), 1_000_000);
        assert_eq!(whole_frames(520_000, &project), 520_000); // 13 frames at 25 fps, not 16 at 30
    }

    #[test]
    fn a_25_fps_timeline_in_a_30_fps_project_exports_at_25() {
        // build_args only checks that the media file exists, so an empty file will do.
        let media = std::env::temp_dir().join(format!("tempo-fps-{}.mp4", std::process::id()));
        std::fs::write(&media, b"").unwrap();
        let mut project = project_with_clip(media.clone());
        project.timeline.fps = Some(RationalFps::FPS_25);
        let (args, _, _) = build_args(&project, &settings(media.with_extension("out.mp4")), None, None).unwrap();
        let _ = std::fs::remove_file(&media);
        let graph = args.join(" ");
        assert!(graph.contains("r=25/1") && !graph.contains("r=30/1"), "{graph}");
    }

    #[test]
    fn bitrate_fits_the_size_with_headroom() {
        // 10 MB over 60 s: 1266 kb/s in all, less 96 for sound.
        assert_eq!(bitrate_for_size(10_000_000, 60_000_000, 96), Some(1170));
        assert_eq!(bitrate_for_size(10_000_000, 10_000_000, 96), Some(7504));
        // Too long to look like anything at 10 MB.
        assert_eq!(bitrate_for_size(10_000_000, 300_000_000, 96), None);
        assert_eq!(bitrate_for_size(10_000_000, 0, 96), None);
    }

    #[test]
    fn size_capped_export_stays_under_ten_megabytes() {
        if Command::new("ffmpeg").arg("-version").output().is_err() {
            return;
        }
        // Moving grain is the hardest thing to squeeze; a test pattern would pass too easily.
        let dir = std::env::temp_dir().join(format!("tempo-discord-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("grain.mp4");
        let made = Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=1280x720:rate=30:duration=40", "-f", "lavfi", "-i", "sine=duration=40"])
            .args(["-vf", "noise=alls=20:allf=t", "-c:v", "libx264", "-preset", "ultrafast", "-crf", "18", "-c:a", "aac", "-shortest"])
            .arg(&input)
            .status()
            .is_ok_and(|s| s.success());
        if !made {
            return; // no x264 here
        }
        let mut project = Project::new("t", 1280, 720, RationalFps::FPS_30);
        let mut source = MediaSource::new(input, MediaType::Video, 40_000_000);
        source.audio_channels = Some(1);
        let v = project.timeline.tracks.iter().find(|t| t.kind == TrackKind::Video).map(|t| t.id).unwrap();
        let a = project.timeline.tracks.iter().find(|t| t.kind == TrackKind::Audio).map(|t| t.id).unwrap();
        for (track, kind) in [(v, ClipType::Video), (a, ClipType::Audio)] {
            project.timeline.find_track_mut(track).unwrap().clips.push(Clip::new(track, source.id, kind, "c", 0, 40_000_000, 0, 40_000_000));
        }
        project.sources.insert(source.id, source);
        let output = dir.join("discord.mp4");
        let kbps = bitrate_for_size(10_000_000, 40_000_000, 96).unwrap();
        let e = TimelineExport { width: 1280, height: 720, max_video_kbps: Some(kbps), ..settings(output.clone()) };
        export_timeline(&project, &e, &AtomicBool::new(false), |_| {}).expect("export");
        let size = std::fs::metadata(&output).unwrap().len();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(size <= 10_000_000, "{size} bytes");
        assert!(size >= 6_000_000, "{size} bytes: the cap, not the quality setting, should decide the size");
    }

    #[test]
    fn empty_timeline_is_an_error() {
        let project = Project::new("t", 1920, 1080, RationalFps::FPS_30);
        assert!(build_ffmpeg_args(&project, &settings("/tmp/x.mp4".into()), None).is_err());
    }

    #[test]
    fn graph_covers_gaps_audio_and_range() {
        let Some(path) = sample() else { return };
        let project = project_with_clip(path);
        let (args, total, _) = build_ffmpeg_args(&project, &settings("/tmp/x.mp4".into()), None).unwrap();
        assert_eq!(total, 3_000_000);
        let graph = &args[args.iter().position(|a| a == "-filter_complex").unwrap() + 1];
        assert!(graph.contains("concat=n=3:v=1:a=0"), "two clips and one gap on the video track");
        assert!(graph.contains("anullsrc"), "silence fills the audio gap");
        assert!(graph.contains("[aout]"));

        let mut ranged = settings("/tmp/x.mp4".into());
        ranged.range = Some((500_000, 2_500_000));
        let (_, total, _) = build_ffmpeg_args(&project, &ranged, None).unwrap();
        assert_eq!(total, 2_000_000);
    }

    #[test]
    fn exports_a_playable_file() {
        let Some(path) = sample() else { return };
        if Command::new("ffmpeg").arg("-version").output().is_err() {
            return;
        }
        let project = project_with_clip(path);
        let out = std::env::temp_dir().join(format!("tempo-export-test-{}.mp4", std::process::id()));
        let mut e = settings(out.clone());
        e.chapters = vec![Chapter { start_us: 0, title: "Intro".into() }, Chapter { start_us: 2_000_000, title: "Next".into() }];
        let mut last = 0.0;
        export_timeline(&project, &e, &AtomicBool::new(false), |p| last = p).unwrap();
        assert_eq!(last, 1.0);

        let probe = Command::new("ffprobe")
            .args(["-v", "error", "-show_entries", "format=duration:stream=codec_type", "-show_chapters", "-of", "default=nw=1"])
            .arg(&out)
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&probe.stdout);
        let _ = std::fs::remove_file(&out);
        assert!(text.contains("codec_type=video") && text.contains("codec_type=audio"), "{text}");
        assert!(text.contains("title=Next"), "chapters are embedded: {text}");
        let duration: f64 = text.lines().find_map(|l| l.strip_prefix("duration=")).and_then(|d| d.parse().ok()).unwrap_or(0.0);
        assert!((duration - 3.0).abs() < 0.2, "duration was {duration}");
    }

    /// Where the loudest sound is in a file, in seconds (mono, 48 kHz).
    fn loudest_at(path: &std::path::Path) -> f64 {
        let pcm = Command::new("ffmpeg").args(["-v", "error", "-i"]).arg(path).args(["-f", "s16le", "-ac", "1", "-ar", "48000", "-"]).output().unwrap().stdout;
        let samples: Vec<i16> = pcm.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
        let at = samples.iter().enumerate().max_by_key(|(_, s)| s.unsigned_abs()).map(|(i, _)| i).unwrap_or(0);
        at as f64 / 48_000.0
    }

    #[test]
    fn retimed_clip_exports_at_its_speed() {
        let click = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/test-media/sample_click.mp4");
        if !click.exists() || Command::new("ffmpeg").arg("-version").output().is_err() {
            return;
        }
        // 3 s of source with a click at 1 s, at double speed: 1.5 s, click at 0.5 s.
        let mut project = Project::new("t", 1920, 1080, RationalFps::FPS_30);
        let mut source = MediaSource::new(click, MediaType::Video, 3_000_000);
        source.audio_channels = Some(2);
        for kind in [TrackKind::Video, TrackKind::Audio] {
            let track = project.timeline.tracks.iter().find(|t| t.kind == kind).map(|t| t.id).unwrap();
            let clip_type = if kind == TrackKind::Video { ClipType::Video } else { ClipType::Audio };
            let mut clip = Clip::new(track, source.id, clip_type, "c", 0, 1_500_000, 0, 3_000_000);
            clip.properties.speed = 2.0;
            project.timeline.find_track_mut(track).unwrap().clips.push(clip);
        }
        project.sources.insert(source.id, source);
        let out = std::env::temp_dir().join(format!("tempo-speed-{}.mp4", std::process::id()));
        export_timeline(&project, &settings(out.clone()), &AtomicBool::new(false), |_| {}).unwrap();
        let probe = Command::new("ffprobe").args(["-v", "error", "-show_entries", "format=duration", "-of", "default=nw=1:nk=1"]).arg(&out).output().unwrap();
        let duration: f64 = String::from_utf8_lossy(&probe.stdout).trim().parse().unwrap_or(0.0);
        let click_at = loudest_at(&out);
        let _ = std::fs::remove_file(&out);
        assert!((duration - 1.5).abs() < 0.1, "duration was {duration}");
        assert!((click_at - 0.5).abs() < 0.02, "click at {click_at} s");
    }

    #[test]
    fn hardware_export_runs_on_the_graphics_chip() {
        let Some(path) = sample() else { return };
        if !crate::ffmpeg::vaapi_h264_available() {
            return;
        }
        let out = std::env::temp_dir().join(format!("tempo-hw-export-{}.mp4", std::process::id()));
        let e = TimelineExport { hardware: true, ..settings(out.clone()) };
        // run_ffmpeg, not export_timeline: no quiet fallback to the processor here.
        let result = run_ffmpeg(&project_with_clip(path), &e, &AtomicBool::new(false), &mut |_| {});
        let exists = out.exists();
        let _ = std::fs::remove_file(&out);
        result.expect("hardware export");
        assert!(exists);
    }

    #[test]
    fn size_cap_wins_over_the_hardware_setting() {
        let Some(path) = sample() else { return };
        if Command::new("ffmpeg").arg("-version").output().is_err() {
            return;
        }
        let out = std::env::temp_dir().join(format!("tempo-capped-hw-{}.mp4", std::process::id()));
        let e = TimelineExport { hardware: true, max_video_kbps: Some(2_000), ..settings(out.clone()) };
        let result = run_ffmpeg(&project_with_clip(path), &e, &AtomicBool::new(false), &mut |_| {});
        let _ = std::fs::remove_file(&out);
        result.expect("a size-capped export uses the processor's encoder, whatever the hardware setting");
    }

    #[test]
    fn exports_titles_and_fades() {
        let Some(path) = sample() else { return };
        if Command::new("ffmpeg").arg("-version").output().is_err() {
            return;
        }
        let mut project = project_with_clip(path);
        let v2 = project.timeline.tracks.iter().find(|t| t.kind == TrackKind::Video && t.kind_index == 2).map(|t| t.id).unwrap();
        let mut title = Clip::new(v2, uuid::Uuid::nil(), ClipType::Title, "Title", 0, 2_000_000, 0, 2_000_000);
        title.title_data = Some(TitleData { text: "It's 100%: a \"test\"".into(), ..Default::default() });
        title.properties.fade_in_us = 500_000;
        project.timeline.find_track_mut(v2).unwrap().clips.push(title);
        for track in &mut project.timeline.tracks {
            for clip in &mut track.clips {
                clip.properties.fade_out_us = 300_000;
            }
        }
        let out = std::env::temp_dir().join(format!("tempo-export-title-{}.mp4", std::process::id()));
        let (args, _, files) = build_ffmpeg_args(&project, &settings(out.clone()), None).unwrap();
        let graph = &args[args.iter().position(|a| a == "-filter_complex").unwrap() + 1];
        assert!(graph.contains("drawtext=textfile=") && graph.contains("colorchannelmixer@fd") && graph.contains("volume='min(") || graph.contains("volume='clip("));
        assert_eq!(files.len(), 1);
        // Awkward characters in the title must not break the filter graph.
        export_timeline(&project, &settings(out.clone()), &AtomicBool::new(false), |_| {}).unwrap();
        assert!(out.exists());
        let _ = std::fs::remove_file(&out);
    }

    /// A 4 s clip of one colour with a sine tone, made with FFmpeg. `None` without FFmpeg.
    fn solid_clip(dir: &std::path::Path, name: &str, color: &str, hz: u32) -> Option<PathBuf> {
        let path = dir.join(name);
        let ok = Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", &format!("color=c={color}:s=320x180:r=30:d=4"), "-f", "lavfi", "-i", &format!("sine=frequency={hz}:duration=4")])
            .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"])
            .arg(&path)
            .status()
            .ok()?
            .success();
        ok.then_some(path)
    }

    /// Two touching 2 s clips (source 1-3 s of a red and a blue 4 s file, so each has a 1 s
    /// handle each side) on the first track of `kind`, plus the transition built by `make`
    /// on the second clip's In edge.
    fn two_clip_project(dir: &std::path::Path, kind: TrackKind, make: impl FnOnce(uuid::Uuid) -> Option<tempo_timeline::Transition>) -> Option<Project> {
        two_colour_project(dir, kind, ["red", "blue"], make)
    }

    /// `two_clip_project` with clips of these two colours.
    fn two_colour_project(dir: &std::path::Path, kind: TrackKind, colors: [&str; 2], make: impl FnOnce(uuid::Uuid) -> Option<tempo_timeline::Transition>) -> Option<Project> {
        let mut project = Project::new("t", 320, 180, RationalFps::FPS_30);
        let clip_type = if kind == TrackKind::Video { ClipType::Video } else { ClipType::Audio };
        let track = project.timeline.tracks.iter().find(|t| t.kind == kind).map(|t| t.id)?;
        let mut second = None;
        for (i, (name, color, hz)) in [("a.mp4", colors[0], 440), ("b.mp4", colors[1], 660)].into_iter().enumerate() {
            let mut source = MediaSource::new(solid_clip(dir, name, color, hz)?, MediaType::Video, 4_000_000);
            source.audio_channels = Some(2);
            let clip = Clip::new(track, source.id, clip_type, name, i as i64 * 2_000_000, (i as i64 + 1) * 2_000_000, 1_000_000, 3_000_000);
            second = Some(clip.id);
            project.timeline.find_track_mut(track)?.clips.push(clip);
            project.sources.insert(source.id, source);
        }
        project.timeline.find_track_mut(track)?.transitions.extend(make(second?));
        Some(project)
    }

    fn export_to(project: &Project, dir: &std::path::Path) -> PathBuf {
        let out = dir.join("out.mp4");
        export_timeline(project, &settings(out.clone()), &AtomicBool::new(false), |_| {}).unwrap();
        out
    }

    /// The colour of the frame at `at` seconds.
    fn pixel_at(file: &std::path::Path, at: f64) -> [u8; 3] {
        let out = Command::new("ffmpeg")
            .args(["-v", "error", "-ss", &at.to_string(), "-i"])
            .arg(file)
            .args(["-frames:v", "1", "-vf", "scale=1:1", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
            .output()
            .unwrap();
        out.stdout[..3].try_into().unwrap()
    }

    fn transition(kind: tempo_timeline::TransitionKind, edge: tempo_timeline::TransitionEdge) -> impl FnOnce(uuid::Uuid) -> Option<tempo_timeline::Transition> {
        move |clip| Some(tempo_timeline::Transition::new(clip, edge, kind, 1_000_000, tempo_timeline::TransitionAlignment::Centered))
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tempo-tr-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn has_ffmpeg() -> bool {
        Command::new("ffmpeg").arg("-version").output().is_ok()
    }

    #[test]
    fn centred_cross_dissolve_mid_frame_mixes_both_clips() {
        use tempo_timeline::{TransitionEdge::In, TransitionKind::CrossDissolve};
        if !has_ffmpeg() {
            return;
        }
        let dir = scratch("mix");
        let Some(project) = two_clip_project(&dir, TrackKind::Video, transition(CrossDissolve, In)) else { return };
        let out = export_to(&project, &dir);
        let (before, mid, after) = (pixel_at(&out, 1.2), pixel_at(&out, 2.0), pixel_at(&out, 2.8));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(before[0] > 200 && before[2] < 50, "{before:?}");
        assert!(after[2] > 200 && after[0] < 50, "{after:?}");
        assert!((90..170).contains(&mid[0]) && (90..170).contains(&mid[2]) && mid[1] < 40, "{mid:?}");
    }

    fn graph_of(project: &Project) -> String {
        let (args, _, _) = build_ffmpeg_args(project, &settings(std::env::temp_dir().join("tempo-graph.mp4")), None).unwrap();
        args[args.iter().position(|a| a == "-filter_complex").unwrap() + 1].clone()
    }

    #[test]
    fn additive_dissolve_middle_is_brighter_than_either_picture() {
        use tempo_timeline::{TransitionEdge::In, TransitionKind::AdditiveDissolve};
        if !has_ffmpeg() {
            return;
        }
        // Arrange: grey 96 into grey 128; a cross dissolve's middle is 112, the brighter one 128.
        let dir = scratch("additive");
        let Some(project) = two_colour_project(&dir, TrackKind::Video, ["0x606060", "0x808080"], transition(AdditiveDissolve, In)) else { return };
        // Act
        let graph = graph_of(&project);
        let out = export_to(&project, &dir);
        let (before, mid, after) = (pixel_at(&out, 1.2), pixel_at(&out, 2.0), pixel_at(&out, 2.8));
        let _ = std::fs::remove_dir_all(&dir);
        // Assert: screen of the two is about 176.
        assert!(graph.contains("blend=all_mode=screen") && graph.contains("alphamerge"), "{graph}");
        assert!((88..104).contains(&before[0]) && (120..136).contains(&after[0]), "{before:?} {after:?}");
        assert!((160..192).contains(&mid[0]), "{mid:?}");
    }

    #[test]
    fn non_additive_dissolve_middle_takes_the_brighter_of_each_channel() {
        use tempo_timeline::{TransitionEdge::In, TransitionKind::NonAdditiveDissolve};
        if !has_ffmpeg() {
            return;
        }
        // Arrange: red into blue; a cross dissolve's middle is half of each.
        let dir = scratch("nonadditive");
        let Some(project) = two_clip_project(&dir, TrackKind::Video, transition(NonAdditiveDissolve, In)) else { return };
        // Act
        let graph = graph_of(&project);
        let out = export_to(&project, &dir);
        let (before, mid, after) = (pixel_at(&out, 1.2), pixel_at(&out, 2.0), pixel_at(&out, 2.8));
        let _ = std::fs::remove_dir_all(&dir);
        // Assert: at least as bright as both pictures in every channel.
        assert!(graph.contains("blend=all_mode=lighten"), "{graph}");
        assert!((0..3).all(|c| mid[c] as i32 + 16 >= before[c].max(after[c]) as i32), "{before:?} {mid:?} {after:?}");
        assert!(mid[0] > 200 && mid[2] > 200 && mid[1] < 40, "{mid:?}");
    }

    /// Mean squared difference between neighbouring pixels of the frame at `at` seconds, in
    /// grey: fine detail. (A blur keeps the plain mean difference across a step edge.)
    fn detail_at(file: &std::path::Path, at: f64) -> f64 {
        let out = Command::new("ffmpeg")
            .args(["-v", "error", "-ss", &at.to_string(), "-i"])
            .arg(file)
            .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "gray", "-"])
            .output()
            .unwrap();
        let px = &out.stdout;
        px.windows(2).map(|w| (w[0] as f64 - w[1] as f64).powi(2)).sum::<f64>() / px.len().max(1) as f64
    }

    #[test]
    fn blur_dissolve_middle_has_less_detail_than_a_cross_dissolve() {
        use tempo_timeline::{Transition, TransitionAlignment, TransitionEdge::In, TransitionKind};
        let Some(path) = sample() else { return };
        if !has_ffmpeg() {
            return;
        }
        // Arrange: the test pattern cut to itself at 2 s, 1 s of handle each side.
        let project_with = |kind: TransitionKind| {
            let mut project = Project::new("t", 1920, 1080, RationalFps::FPS_30);
            let source = MediaSource::new(path.clone(), MediaType::Video, 5_000_000);
            let track = project.timeline.tracks.iter_mut().find(|t| t.kind == TrackKind::Video).unwrap();
            let a = Clip::new(track.id, source.id, ClipType::Video, "a", 0, 2_000_000, 1_000_000, 3_000_000);
            let b = Clip::new(track.id, source.id, ClipType::Video, "b", 2_000_000, 4_000_000, 1_000_000, 3_000_000);
            track.transitions.push(Transition::new(b.id, In, kind, 1_000_000, TransitionAlignment::Centered));
            track.clips = vec![a, b];
            project.sources.insert(source.id, source);
            project
        };
        let (blur, cross) = (project_with(TransitionKind::BlurDissolve), project_with(TransitionKind::CrossDissolve));
        let (dir_blur, dir_cross) = (scratch("blur"), scratch("blur-cross"));
        // Act
        let graph = graph_of(&blur);
        let (out_blur, out_cross) = (export_to(&blur, &dir_blur), export_to(&cross, &dir_cross));
        let (blurred, sharp) = (detail_at(&out_blur, 2.0), detail_at(&out_cross, 2.0));
        let (blur_edge, cross_edge) = (detail_at(&out_blur, 1.2), detail_at(&out_cross, 1.2));
        let _ = (std::fs::remove_dir_all(&dir_blur), std::fs::remove_dir_all(&dir_cross));
        // Assert: blurred hard at the middle, and the same picture outside the transition.
        assert!(graph.contains("gblur@bl") && !graph_of(&cross).contains("gblur@"), "{graph}");
        // The peak at the cut, sized as the viewer sizes it: GTK's radius for a 360-line
        // picture, as FFmpeg's sigma.
        let peak = tempo_timeline::blur_sigma(tempo_timeline::blur_radius_px(tempo_timeline::BLUR_DISSOLVE_PEAK, 360.0));
        assert!(graph.contains(&format!("0.000000 gblur@bl0_1 sigma {peak:.4};")), "{graph}");
        assert!(blurred < sharp * 0.6, "blur {blurred} vs cross {sharp}");
        assert!((blur_edge - cross_edge).abs() < cross_edge * 0.05, "{blur_edge} vs {cross_edge}");
    }

    #[test]
    fn dip_to_color_midpoint_is_the_dip_colour() {
        use tempo_timeline::{TransitionEdge::In, TransitionKind::DipToColor};
        if !has_ffmpeg() {
            return;
        }
        let dir = scratch("dip");
        let Some(project) = two_clip_project(&dir, TrackKind::Video, |clip| {
            let mut t = transition(DipToColor, In)(clip)?;
            t.color = [0.0, 1.0, 0.0];
            Some(t)
        }) else {
            return;
        };
        let out = export_to(&project, &dir);
        let (before, mid) = (pixel_at(&out, 1.2), pixel_at(&out, 2.0));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(before[0] > 200, "{before:?}");
        assert!(mid[1] > 215 && mid[0] < 45 && mid[2] < 45, "{mid:?}");
    }

    #[test]
    fn head_transition_fades_in_from_black() {
        use tempo_timeline::{TransitionEdge::In, TransitionKind::CrossDissolve};
        if !has_ffmpeg() {
            return;
        }
        let dir = scratch("head");
        // The first clip has nothing before it, so its In transition is a head fade.
        let Some(mut project) = two_clip_project(&dir, TrackKind::Video, |_| None) else { return };
        let first = project.timeline.tracks.iter().find(|t| t.kind == TrackKind::Video).unwrap().clips[0].id;
        project.timeline.tracks.iter_mut().find(|t| t.kind == TrackKind::Video).unwrap().transitions.extend(transition(CrossDissolve, In)(first));
        let out = export_to(&project, &dir);
        let (start, half, full) = (pixel_at(&out, 0.0), pixel_at(&out, 0.5), pixel_at(&out, 1.5));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(start[0] < 25, "{start:?}");
        assert!((90..170).contains(&half[0]), "{half:?}");
        assert!(full[0] > 200, "{full:?}");
    }

    /// An export that starts inside a ramp still shows the rest of it.
    #[test]
    fn ranged_export_keeps_the_fade_it_starts_inside() {
        use tempo_timeline::{TransitionEdge::In, TransitionKind::{CrossDissolve, DipToColor}};
        if !has_ffmpeg() {
            return;
        }
        let dir = scratch("ranged");
        // Head fade 0-1 s on the first clip; range 0.5-1.5 s starts half way up the ramp.
        let Some(mut project) = two_clip_project(&dir, TrackKind::Video, |_| None) else { return };
        let track = project.timeline.tracks.iter_mut().find(|t| t.kind == TrackKind::Video).unwrap();
        let first = track.clips[0].id;
        let second = track.clips[1].id;
        track.transitions.extend(transition(CrossDissolve, In)(first));
        let mut dip = transition(DipToColor, In)(second).unwrap();
        dip.color = [0.0, 1.0, 0.0];
        track.transitions.push(dip);
        let settings = TimelineExport { range: Some((500_000, 2_200_000)), ..settings(dir.join("out.mp4")) };
        export_timeline(&project, &settings, &AtomicBool::new(false), |_| {}).unwrap();
        let out = dir.join("out.mp4");
        // Export time 0 is 0.5 s on the timeline: red at about half. 1.5 s on the timeline
        // (export 1.0) is where the dip starts; export 1.5 is the dip's middle.
        let (start, mid) = (pixel_at(&out, 0.0), pixel_at(&out, 1.5));
        let _ = std::fs::remove_dir_all(&dir);
        assert!((70..180).contains(&start[0]), "{start:?}");
        assert!(mid[1] > 150 && mid[0] < 80, "{mid:?}");
    }

    /// Dip with Ease In, 1.5-2.5 s around the cut at 2.0 s; the sampled green must follow
    /// `ColorLayer::opacity_at`, in a full export and in ranges that start or end inside it.
    #[test]
    fn eased_dip_follows_opacity_at_whole_and_ranged() {
        use tempo_timeline::{TransitionEase, TransitionEdge::In, TransitionKind::DipToColor};
        if !has_ffmpeg() {
            return;
        }
        let dir = scratch("ease");
        let Some(project) = two_clip_project(&dir, TrackKind::Video, |clip| {
            let mut t = transition(DipToColor, In)(clip)?;
            t.color = [0.0, 1.0, 0.0];
            t.ease = TransitionEase::In;
            Some(t)
        }) else {
            return;
        };
        let track = project.timeline.tracks.iter().find(|t| t.kind == TrackKind::Video).unwrap();
        let layer = track.transition_layers(&project.sources).color.remove(0);
        // (range start, range end, timeline seconds to sample)
        for (range, sample) in [(None, 1.75), (Some((0, 1_800_000)), 1.7), (Some((1_800_000, 3_000_000)), 2.1), (Some((1_700_000, 2_300_000)), 1.9)] {
            let out = dir.join("out.mp4");
            let settings = TimelineExport { range, ..settings(out.clone()) };
            export_timeline(&project, &settings, &AtomicBool::new(false), |_| {}).unwrap();
            let at = sample - range.map_or(0.0, |r| r.0 as f64 / 1e6);
            let px = pixel_at(&out, at);
            let expect = layer.opacity_at((sample * 1e6) as i64) * 255.0;
            assert!((px[1] as f32 - expect).abs() < 30.0, "{range:?} at {sample}: green {} expected {expect}", px[1]);
        }
        let _ = std::fs::remove_dir_all(&dir);
        // Sanity: the ease really bends the ramp away from a straight line at the quarter point.
        assert!((layer.opacity_at(1_750_000) - 0.5).abs() > 0.05, "{}", layer.opacity_at(1_750_000));
    }

    #[test]
    fn audio_fades_are_smooth_and_survive_a_range_cut() {
        let Some(path) = sample() else { return };
        let mut project = project_with_clip(path);
        for clip in project.timeline.tracks.iter_mut().filter(|t| t.kind == TrackKind::Audio).flat_map(|t| t.clips.iter_mut().take(1)) {
            clip.properties.fade_in_us = 800_000;
        }
        let out = std::env::temp_dir().join("tempo-export-afade.mp4");
        for range in [None, Some((300_000, 900_000))] {
            let e = TimelineExport { range, ..settings(out.clone()) };
            let (args, _, _) = build_ffmpeg_args(&project, &e, None).unwrap();
            let graph = &args[args.iter().position(|a| a == "-filter_complex").unwrap() + 1];
            assert!(graph.contains("asetnsamples=n=64:p=0,volume='clip((t+("), "{range:?}: {graph}");
        }
    }

    #[test]
    fn mono_audio_track_is_centred_in_the_graph() {
        let Some(path) = sample() else { return };
        let mut project = project_with_clip(path);
        let a = project.timeline.tracks.iter().find(|t| t.kind == TrackKind::Audio).map(|t| t.id).unwrap();
        let out = std::env::temp_dir().join("tempo-export-mono.mp4");
        let graph_of = |project: &Project| {
            let (args, _, _) = build_ffmpeg_args(project, &settings(out.clone()), None).unwrap();
            args[args.iter().position(|a| a == "-filter_complex").unwrap() + 1].clone()
        };
        assert!(!graph_of(&project).contains("c0=0.5*c0+0.5*c1"));
        project.timeline.find_track_mut(a).unwrap().audio_type = tempo_timeline::types::AudioTrackType::Mono;
        assert!(graph_of(&project).contains("c0=0.5*c0+0.5*c1|c1=0.5*c0+0.5*c1"));
    }

    #[test]
    fn audio_crossfade_has_no_gap_in_level() {
        use tempo_timeline::{TransitionEdge::In, TransitionKind::Crossfade};
        if !has_ffmpeg() {
            return;
        }
        let dir = scratch("xfade");
        let Some(project) = two_clip_project(&dir, TrackKind::Audio, transition(Crossfade, In)) else { return };
        let out = export_to(&project, &dir);
        let pcm = Command::new("ffmpeg").args(["-v", "error", "-i"]).arg(&out).args(["-vn", "-ac", "1", "-ar", "8000", "-f", "s16le", "-"]).output().unwrap().stdout;
        let _ = std::fs::remove_dir_all(&dir);
        let samples: Vec<f64> = pcm.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]) as f64).collect();
        // RMS of each 50 ms window.
        let rms: Vec<f64> = samples.chunks_exact(400).map(|w| (w.iter().map(|v| v * v).sum::<f64>() / 400.0).sqrt()).collect();
        let nominal = rms[10..20].iter().sum::<f64>() / 10.0;
        // The crossfade runs 1.5-2.5 s: windows 30..50. Two different tones add in power.
        let worst = rms[32..48].iter().cloned().fold(f64::MAX, f64::min);
        assert!(nominal > 1000.0 && worst > nominal * 0.75, "nominal {nominal}, worst {worst}");
    }

    #[test]
    fn audio_ramps_reach_the_graph_and_video_transitions_do_not_touch_sound() {
        use tempo_timeline::{TransitionEdge::In, TransitionKind::CrossDissolve};
        let dir = std::env::temp_dir();
        let mut project = Project::new("t", 320, 180, RationalFps::FPS_30);
        let mut source = MediaSource::new(dir.join("x.mp4"), MediaType::Video, 4_000_000);
        source.audio_channels = Some(2);
        let (v, a) = (
            project.timeline.tracks.iter().find(|t| t.kind == TrackKind::Video).map(|t| t.id).unwrap(),
            project.timeline.tracks.iter().find(|t| t.kind == TrackKind::Audio).map(|t| t.id).unwrap(),
        );
        for track in [v, a] {
            let kind = if track == v { ClipType::Video } else { ClipType::Audio };
            for i in 0..2 {
                let clip = Clip::new(track, source.id, kind, "c", i * 2_000_000, (i + 1) * 2_000_000, 1_000_000, 3_000_000);
                project.timeline.find_track_mut(track).unwrap().clips.push(clip);
            }
            let second = project.timeline.find_track_mut(track).unwrap().clips[1].id;
            project.timeline.find_track_mut(track).unwrap().transitions.extend(transition(CrossDissolve, In)(second));
        }
        // Sound comes only from the audio track's own transition: its clips and two handles.
        let audio = crate::audio_mix::audio_strips(project.timeline.find_track_mut(a).unwrap(), &project.sources);
        assert_eq!(audio.strips.len(), 3, "clips, head handle, tail handle");
        assert_eq!(audio.ramps.len(), 4);
    }

    #[test]
    fn clip_filters_reach_the_filter_graph_and_export() {
        use tempo_timeline::{Amount, ClipEffect, EffectOp};
        let Some(path) = sample() else { return };
        let mut project = project_with_clip(path);
        let v = project.timeline.tracks.iter_mut().find(|t| t.kind == TrackKind::Video).unwrap();
        v.clips[0].properties.effects = vec![ClipEffect {
            id: "t/look".into(),
            name: "Look".into(),
            params: Vec::new(),
            ops: vec![EffectOp::Saturation(Amount::Value(0.0)), EffectOp::Brightness(Amount::Value(0.1)), EffectOp::Blur(Amount::Value(6.0))],
        }];
        let out = std::env::temp_dir().join(format!("tempo-export-fx-{}.mp4", std::process::id()));
        let (args, _, _) = build_ffmpeg_args(&project, &settings(out.clone()), None).unwrap();
        let graph = &args[args.iter().position(|a| a == "-filter_complex").unwrap() + 1];
        assert!(graph.contains("colorchannelmixer=rr=0.21260") && graph.contains("lutrgb=") && graph.contains("gblur="), "{graph}");
        // Only the first clip has the filter.
        assert_eq!(graph.matches("colorchannelmixer").count(), 1);
        if Command::new("ffmpeg").arg("-version").output().is_ok() {
            export_timeline(&project, &settings(out.clone()), &AtomicBool::new(false), |_| {}).unwrap();
            assert!(out.exists());
            let _ = std::fs::remove_file(&out);
        }
    }

    /// Nine frames at 30 fps, on the timeline's frame grid.
    const CUT: i64 = 9 * 33_333;

    /// A timeline of `n` clips of nine frames, picture and sound, with a gap in the middle.
    fn project_with_many_cuts(path: PathBuf, n: i64) -> Project {
        let mut project = project_with_clip(path);
        for track in &mut project.timeline.tracks {
            track.clips.clear();
        }
        let source = *project.sources.keys().next().unwrap();
        let v = project.timeline.tracks.iter().find(|t| t.kind == TrackKind::Video).map(|t| t.id).unwrap();
        let a = project.timeline.tracks.iter().find(|t| t.kind == TrackKind::Audio).map(|t| t.id).unwrap();
        for i in 0..n {
            // Clip 7 is left out to make a gap.
            if i == 7 {
                continue;
            }
            for (track, kind) in [(v, ClipType::Video), (a, ClipType::Audio)] {
                let clip = Clip::new(track, source, kind, "c", i * CUT, (i + 1) * CUT, i * 100_000, i * 100_000 + CUT);
                project.timeline.find_track_mut(track).unwrap().clips.push(clip);
            }
        }
        project
    }

    #[test]
    fn a_long_cut_is_planned_in_sections_that_avoid_fades() {
        let mut project = project_with_many_cuts(PathBuf::from("/nonexistent.mp4"), 30);
        let end = 30 * CUT;
        // Few clips: one section. Many: several, covering the range with no holes.
        assert_eq!(plan_sections(&project, 0, 3 * CUT), vec![(0, 3 * CUT)]);
        let sections = plan_sections(&project, 0, end);
        assert!(sections.len() >= 4, "{sections:?}");
        assert_eq!((sections[0].0, sections[sections.len() - 1].1), (0, end));
        assert!(sections.windows(2).all(|w| w[0].1 == w[1].0 && w[0].1 > w[0].0));
        // No section reads more than the limit.
        for (a, b) in &sections {
            let inputs = project.timeline.tracks.iter().flat_map(|t| t.clips.iter()).filter(|c| c.timeline_out > *a && c.timeline_in < *b).count();
            assert!(inputs <= MAX_INPUTS, "{inputs} inputs in {a}..{b}");
        }
        // A join never lands inside a fade: fade every clip across its edges and
        // the only places left are the two sides of the gap.
        for track in &mut project.timeline.tracks {
            let ids: Vec<_> = track.clips.iter().filter(|c| track.clips.iter().any(|p| p.timeline_out == c.timeline_in)).map(|c| c.id).collect();
            for id in ids {
                let t = tempo_timeline::Transition::new(id, tempo_timeline::TransitionEdge::In, tempo_timeline::TransitionKind::CrossDissolve, 200_000, tempo_timeline::TransitionAlignment::Centered);
                track.transitions.push(t);
            }
        }
        let joins: Vec<i64> = plan_sections(&project, 0, end).iter().skip(1).map(|s| s.0).collect();
        assert!(!joins.is_empty() && joins.iter().all(|t| [63, 72].contains(&to_frame(*t, &project))), "{joins:?}");
    }

    #[test]
    fn a_long_cut_exports_in_sections_with_picture_and_sound_in_step() {
        let Some(path) = sample() else { return };
        if Command::new("ffmpeg").arg("-version").output().is_err() {
            return;
        }
        let project = project_with_many_cuts(path, 16);
        assert!(plan_sections(&project, 0, 16 * CUT).len() > 1);
        let out = std::env::temp_dir().join(format!("tempo-export-sections-{}.mp4", std::process::id()));
        let mut e = settings(out.clone());
        e.chapters = vec![Chapter { start_us: 0, title: "Intro".into() }, Chapter { start_us: 2_000_000, title: "Next".into() }];
        let mut seen = Vec::new();
        export_timeline(&project, &e, &AtomicBool::new(false), |p| seen.push(p)).unwrap();
        assert_eq!(seen.last(), Some(&1.0));
        assert!(seen.windows(2).all(|w| w[1] >= w[0]), "progress never goes back");
        assert!(!out.with_extension("parts").exists(), "the section files are removed");

        let probe = Command::new("ffprobe")
            .args(["-v", "error", "-show_entries", "stream=codec_type,codec_name,duration,nb_frames", "-show_chapters", "-of", "default=nw=1"])
            .arg(&out)
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&probe.stdout);
        let _ = std::fs::remove_file(&out);
        assert!(text.contains("codec_name=h264") && text.contains("codec_name=aac") && text.contains("title=Next"), "{text}");
        // 16 × 9 frames is 144; picture and sound are the same length.
        assert!(text.contains("nb_frames=144"), "{text}");
        let durations: Vec<f64> = text.lines().filter_map(|l| l.strip_prefix("duration=")).filter_map(|d| d.parse().ok()).collect();
        assert!(durations.len() >= 2 && durations.iter().all(|d| (d - 4.8).abs() < 0.06), "{durations:?}");
    }

    #[test]
    fn hardware_export_falls_back_to_software_when_it_cannot_run() {
        let Some(path) = sample() else { return };
        if Command::new("ffmpeg").arg("-version").output().is_err() {
            return;
        }
        let project = project_with_clip(path);
        let out = std::env::temp_dir().join(format!("tempo-export-hw-{}.mp4", std::process::id()));
        let mut e = settings(out.clone());
        e.hardware = true;
        // Whether or not this machine has a working VA-API encoder, a file must come out.
        export_timeline(&project, &e, &AtomicBool::new(false), |_| {}).unwrap();
        assert!(out.exists());
        let _ = std::fs::remove_file(&out);
    }

    #[test]
    fn cancel_removes_the_partial_file() {
        let Some(path) = sample() else { return };
        if Command::new("ffmpeg").arg("-version").output().is_err() {
            return;
        }
        let project = project_with_clip(path);
        let out = std::env::temp_dir().join(format!("tempo-export-cancel-{}.mp4", std::process::id()));
        let cancel = AtomicBool::new(true);
        assert!(export_timeline(&project, &settings(out.clone()), &cancel, |_| {}).is_err());
        assert!(!out.exists());
    }
}
