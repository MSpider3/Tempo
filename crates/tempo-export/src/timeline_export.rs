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

use tempo_timeline::{Clip, ClipProperties, ClipType, MediaType, Project, TitleData, TitleType, TrackKind};

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
            Piece { clip: c, start: start - r0, end: c.timeline_out.min(r1) - r0, source_in: c.source_in + (start - c.timeline_in) }
        })
        .collect();
    out.sort_by_key(|p| p.start);
    out
}

/// FFmpeg filters for the clip's own filters (colour and blur), to follow `format=rgba`.
fn effect_filters(props: &ClipProperties, sy: f32) -> String {
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
        // The radius is given for a 1080-line project; `sy` scales it to the output.
        out.push_str(&format!(",gblur=sigma={:.2}", (fx.blur * sy * 0.5).max(0.1)));
    }
    out
}

/// Fade filters for a piece. A fade is applied only where the piece still has
/// that end of its clip (an export range may have cut it off).
fn fade_filters(p: &Piece, r0: i64, audio: bool) -> String {
    let props = &p.clip.properties;
    let len = p.end - p.start;
    let (name, alpha) = if audio { ("afade", "") } else { ("fade", ":alpha=1") };
    let mut out = String::new();
    if props.fade_in_us > 0 && p.start + r0 == p.clip.timeline_in {
        out.push_str(&format!(",{name}=t=in:st=0:d={}{alpha}", secs(props.fade_in_us.min(len))));
    }
    if props.fade_out_us > 0 && p.end + r0 == p.clip.timeline_out {
        let d = props.fade_out_us.min(len);
        out.push_str(&format!(",{name}=t=out:st={}:d={}{alpha}", secs(len - d), secs(d)));
    }
    out
}

/// The `drawtext` filter for a title. The text itself goes in a file, which
/// avoids every quoting problem in the filter graph.
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
        "drawtext=textfile='{}':font='{}{style}':fontsize={size:.1}:fontcolor=0x{r:02x}{g:02x}{b:02x}@{alpha:.3}:x=(w-text_w)/2+{:.1}:y={y}+{:.1}",
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
    let (num, den) = (project.fps.num.max(1) as i128, project.fps.den.max(1) as i128);
    ((us as i128 * num + den * 500_000) / (den * 1_000_000)) as i64
}

/// The time at which an output frame starts.
fn frame_time(frame: i64, project: &Project) -> i64 {
    let (num, den) = (project.fps.num.max(1) as i128, project.fps.den.max(1) as i128);
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
    let mut text_files: Vec<(PathBuf, String)> = Vec::new();
    let sx = ((e.width.max(16) + 1) & !1) as f32 / project.width.max(1) as f32;
    let sy = ((e.height.max(16) + 1) & !1) as f32 / project.height.max(1) as f32;
    let timeline_end = project.timeline.tracks.iter().map(|t| t.duration_us()).max().unwrap_or(0);
    let (r0, r1) = e.range.unwrap_or((0, timeline_end));
    let (r0, r1) = (r0.max(0), r1.min(timeline_end));
    if r1 - r0 <= 0 {
        return Err(ExportError::InvalidParameter("There is nothing on the timeline to export.".into()));
    }
    let total = if section.is_some() { whole_frames(r1 - r0, project) } else { r1 - r0 };

    let (w, h) = ((e.width.max(16) + 1) & !1, (e.height.max(16) + 1) & !1);
    let fps = format!("{}/{}", project.fps.num.max(1), project.fps.den.max(1));
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
    // A cross dissolve is one more strip right above its track, holding the
    // lead-ins of the clips that dissolve in.
    let mut video_strips: Vec<Vec<Clip>> = Vec::new();
    for track in &video_tracks {
        video_strips.push(track.clips.clone());
        let leads = track.dissolve_leads();
        if !leads.is_empty() {
            video_strips.push(leads);
        }
    }
    let mut track_labels = Vec::new();
    for (ti, strip) in video_strips.iter().enumerate() {
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
            let fades = fade_filters(p, r0, false);
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
                inputs.extend(["-ss".into(), secs(p.source_in), "-t".into(), secs(len + frame_time(1, project))]);
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
            let effects = effect_filters(props, h as f32 / 1080.0);
            // If the file ends early its last picture is held, so the clip always
            // gives exactly the frames the timeline has for it.
            let head = format!(
                "[{idx}:v]fps={fps},tpad=stop_mode=clone:stop_duration=2,{fit},setsar=1,format=rgba{effects},trim=end_frame={frames},setpts=PTS-STARTPTS"
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
        track_labels.push(out);
    }

    graph.push(format!("color=c=black:s={w}x{h}:r={fps},trim=end_frame={}[base0]", to_frame(total, project).max(1)));
    let mut base = "base0".to_string();
    for (i, label) in track_labels.iter().enumerate() {
        let next = format!("base{}", i + 1);
        graph.push(format!("[{base}][{label}]overlay=eof_action=pass:format=auto[{next}]"));
        base = next;
    }
    if e.hardware {
        graph.push(format!("[{base}]format=nv12,hwupload[vout]"));
    } else {
        graph.push(format!("[{base}]format=yuv420p[vout]"));
    }

    // ---- Audio: one strip per track, then mixed.
    let mut audio_tracks: Vec<_> = project.timeline.tracks.iter().filter(|t| t.kind == TrackKind::Audio && t.enabled).collect();
    audio_tracks.sort_by_key(|t| t.kind_index);
    // For sound, the outgoing clip fades out while the incoming one fades in.
    let mut audio_strips: Vec<(Vec<Clip>, f32)> = Vec::new();
    for track in &audio_tracks {
        let leads = track.dissolve_leads();
        let mut clips = track.clips.clone();
        for lead in &leads {
            if let Some(out) = clips.iter_mut().find(|c| c.timeline_out == lead.timeline_out) {
                out.properties.fade_out_us = out.properties.fade_out_us.max(lead.duration_us());
            }
        }
        audio_strips.push((clips, track.volume));
        if !leads.is_empty() {
            audio_strips.push((leads, track.volume));
        }
    }
    let mut audio_labels = Vec::new();
    for (ti, (strip, track_volume)) in audio_strips.iter().enumerate() {
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
            inputs.extend(["-ss".into(), secs(p.source_in), "-t".into(), secs(len), "-i".into(), path.to_string_lossy().into_owned()]);
            let idx = n_inputs;
            n_inputs += 1;
            let props = &p.clip.properties;
            let volume = if props.muted { 0.0 } else { props.volume.max(0.0) * track_volume.max(0.0) };
            let (left, right) = (1.0 - props.pan.max(0.0), 1.0 + props.pan.min(0.0));
            let label = format!("ac{ti}_{}", segs.len());
            graph.push(format!(
                "[{idx}:a]aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo,atrim=duration={},asetpts=PTS-STARTPTS,\
                 volume={volume:.4},pan=stereo|c0={left:.4}*c0|c1={right:.4}*c1{}[{label}]",
                secs(len),
                fade_filters(p, r0, true)
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
    if e.hardware {
        args.extend(["-vaapi_device".into(), "/dev/dri/renderD128".into()]);
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
    if e.hardware {
        // VA-API has no CRF; a fixed quantiser close to the CRF gives similar quality.
        args.extend(["-c:v".into(), "h264_vaapi".into(), "-qp".into(), (e.crf.min(51) as u32 + 2).to_string()]);
    } else {
        args.extend(crate::ffmpeg::h264_args(e.crf, w, h, false));
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
                (c.timeline_in - p.dissolve_in_us, c.timeline_in + p.dissolve_in_us),
            ]
        })
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
    if e.hardware {
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
    if let Some(stdout) = child.stdout.take() {
        for line in BufReader::new(stdout).lines().map_while(|l| l.ok()) {
            if cancel.load(Ordering::Relaxed) {
                cancelled = true;
                let _ = child.kill();
                break;
            }
            if let Some(us) = line.strip_prefix("out_time_us=").and_then(|v| v.trim().parse::<i64>().ok()) {
                progress((us as f32 / total.max(1) as f32).clamp(0.0, 1.0));
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
        TimelineExport { width: 640, height: 360, crf: 30, audio_kbps: 96, fit: Fit::Fit, range: None, chapters: Vec::new(), output, hardware: false }
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
        assert!(graph.contains("drawtext=textfile=") && graph.contains("fade=t=in") && graph.contains("afade=t=out"));
        assert_eq!(files.len(), 1);
        // Awkward characters in the title must not break the filter graph.
        export_timeline(&project, &settings(out.clone()), &AtomicBool::new(false), |_| {}).unwrap();
        assert!(out.exists());
        let _ = std::fs::remove_file(&out);
    }

    #[test]
    fn cross_dissolve_adds_a_lead_in_strip_and_crossfades_sound() {
        let Some(path) = sample() else { return };
        let mut project = project_with_clip(path);
        // Make the two pieces touch at 1 s, then dissolve into the second.
        for track in &mut project.timeline.tracks {
            if let Some(second) = track.clips.get_mut(1) {
                second.timeline_in = 1_000_000;
                second.timeline_out = 2_000_000;
                second.properties.dissolve_in_us = 400_000;
            }
        }
        let out = std::env::temp_dir().join(format!("tempo-export-dissolve-{}.mp4", std::process::id()));
        let (args, total, _) = build_ffmpeg_args(&project, &settings(out.clone()), None).unwrap();
        assert_eq!(total, 2_000_000);
        let graph = &args[args.iter().position(|a| a == "-filter_complex").unwrap() + 1];
        // Video: a second strip with the fading lead-in, overlaid on the first.
        assert!(graph.contains("[vt1]") && graph.contains("fade=t=in:st=0:d=0.400000:alpha=1"), "{graph}");
        // Sound: the outgoing clip fades out and the lead-in fades in, then both are mixed.
        assert!(graph.contains("afade=t=out") && graph.contains("afade=t=in") && graph.contains("amix=inputs=2"), "{graph}");
        if Command::new("ffmpeg").arg("-version").output().is_ok() {
            export_timeline(&project, &settings(out.clone()), &AtomicBool::new(false), |_| {}).unwrap();
            assert!(out.exists());
            let _ = std::fs::remove_file(&out);
        }
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
            for clip in &mut track.clips {
                clip.properties.dissolve_in_us = 100_000;
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
