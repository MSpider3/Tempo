//! Real timeline export. The cut is described to FFmpeg as a filter graph, so
//! FFmpeg reads the original files, composites the tracks, mixes the audio and
//! encodes in one pass. No frames pass through this process.

use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};

use tempo_timeline::{Clip, ClipProperties, ClipType, MediaType, Project, TrackKind};

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

/// Build the complete FFmpeg command line (without the program name).
/// Returns the arguments and the duration of the output in microseconds.
pub fn build_ffmpeg_args(project: &Project, e: &TimelineExport, metadata_file: Option<&PathBuf>) -> Result<(Vec<String>, i64)> {
    let timeline_end = project.timeline.tracks.iter().map(|t| t.duration_us()).max().unwrap_or(0);
    let (r0, r1) = e.range.unwrap_or((0, timeline_end));
    let (r0, r1) = (r0.max(0), r1.min(timeline_end));
    let total = r1 - r0;
    if total <= 0 {
        return Err(ExportError::InvalidParameter("There is nothing on the timeline to export.".into()));
    }

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
    let mut track_labels = Vec::new();
    for (ti, track) in video_tracks.iter().enumerate() {
        let parts = pieces(&track.clips, r0, r1, &[ClipType::Video, ClipType::Image]);
        if parts.is_empty() {
            continue;
        }
        let mut segs = Vec::new();
        let mut cursor = 0i64;
        let gap = |graph: &mut Vec<String>, segs: &mut Vec<String>, len: i64| {
            let label = format!("vg{ti}_{}", segs.len());
            graph.push(format!("color=c=black@0.0:s={w}x{h}:r={fps}:d={},format=rgba[{label}]", secs(len)));
            segs.push(label);
        };
        for p in &parts {
            if p.start > cursor {
                gap(&mut graph, &mut segs, p.start - cursor);
            }
            let len = p.end - p.start;
            let (path, media) = source_path(p.clip)?;
            if media == MediaType::Image {
                inputs.extend(["-loop".into(), "1".into(), "-t".into(), secs(len)]);
            } else {
                inputs.extend(["-ss".into(), secs(p.source_in), "-t".into(), secs(len)]);
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
            let head = format!("[{idx}:v]fps={fps},{fit},setsar=1,format=rgba,trim=duration={},setpts=PTS-STARTPTS", secs(len));
            if is_default_transform(props) {
                graph.push(format!("{head},pad={w}:{h}:(ow-iw)/2:(oh-ih)/2:color=black@0.0[{label}]"));
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
                // Inspector positions are in project pixels; scale them to the output size.
                let sx = w as f32 / project.width.max(1) as f32;
                let sy = h as f32 / project.height.max(1) as f32;
                graph.push(format!("{chain}[{label}s]"));
                graph.push(format!("color=c=black@0.0:s={w}x{h}:r={fps}:d={},format=rgba[{label}b]", secs(len)));
                graph.push(format!(
                    "[{label}b][{label}s]overlay=x=(W-w)/2+{:.2}:y=(H-h)/2+{:.2}:shortest=1,format=rgba[{label}]",
                    props.position_x * sx,
                    props.position_y * sy
                ));
            }
            segs.push(label);
            cursor = p.end;
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

    graph.push(format!("color=c=black:s={w}x{h}:r={fps}:d={}[base0]", secs(total)));
    let mut base = "base0".to_string();
    for (i, label) in track_labels.iter().enumerate() {
        let next = format!("base{}", i + 1);
        graph.push(format!("[{base}][{label}]overlay=eof_action=pass:format=auto[{next}]"));
        base = next;
    }
    graph.push(format!("[{base}]format=yuv420p[vout]"));

    // ---- Audio: one strip per track, then mixed.
    let mut audio_tracks: Vec<_> = project.timeline.tracks.iter().filter(|t| t.kind == TrackKind::Audio && t.enabled).collect();
    audio_tracks.sort_by_key(|t| t.kind_index);
    let mut audio_labels = Vec::new();
    for (ti, track) in audio_tracks.iter().enumerate() {
        let parts = pieces(&track.clips, r0, r1, &[ClipType::Audio]);
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
            let volume = if props.muted { 0.0 } else { props.volume.max(0.0) * track.volume.max(0.0) };
            let (left, right) = (1.0 - props.pan.max(0.0), 1.0 + props.pan.min(0.0));
            let label = format!("ac{ti}_{}", segs.len());
            graph.push(format!(
                "[{idx}:a]aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo,atrim=duration={},asetpts=PTS-STARTPTS,\
                 volume={volume:.4},pan=stereo|c0={left:.4}*c0|c1={right:.4}*c1[{label}]",
                secs(len)
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
    let has_audio = !audio_labels.is_empty();
    if has_audio {
        let joined: String = audio_labels.iter().map(|s| format!("[{s}]")).collect();
        graph.push(format!("{joined}amix=inputs={}:normalize=0:duration=longest,alimiter=limit=0.97,apad[aout]", audio_labels.len()));
    }

    // ---- Command line.
    let mut args: Vec<String> = vec!["-y".into(), "-hide_banner".into(), "-nostats".into(), "-progress".into(), "pipe:1".into()];
    args.extend(inputs);
    if let Some(meta) = metadata_file {
        args.extend(["-i".into(), meta.to_string_lossy().into_owned(), "-map_metadata".into(), n_inputs.to_string()]);
    }
    args.extend(["-filter_complex".into(), graph.join(";"), "-map".into(), "[vout]".into()]);
    if has_audio {
        args.extend(["-map".into(), "[aout]".into(), "-c:a".into(), "aac".into(), "-b:a".into(), format!("{}k", e.audio_kbps.max(64))]);
    }
    args.extend(crate::ffmpeg::h264_args(e.crf, w, h, false));
    args.extend([
        "-movflags".into(),
        "+faststart".into(),
        "-t".into(),
        secs(total),
        e.output.to_string_lossy().into_owned(),
    ]);
    Ok((args, total))
}

/// Run the export. `progress` receives 0.0–1.0. Setting `cancel` stops FFmpeg
/// and removes the partial file. Blocking: call from a worker thread.
pub fn export_timeline(project: &Project, e: &TimelineExport, cancel: &AtomicBool, mut progress: impl FnMut(f32)) -> Result<()> {
    if let Some(dir) = e.output.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let timeline_end = project.timeline.tracks.iter().map(|t| t.duration_us()).max().unwrap_or(0);
    let range_len = e.range.map(|(a, b)| b.min(timeline_end) - a.max(0)).unwrap_or(timeline_end);

    let metadata_file = if e.chapters.is_empty() {
        None
    } else {
        let path = std::env::temp_dir().join(format!("tempo-chapters-{}.txt", std::process::id()));
        std::fs::write(&path, ffmetadata(&e.chapters, range_len))?;
        Some(path)
    };
    let (args, total) = build_ffmpeg_args(project, e, metadata_file.as_ref())?;

    let mut command = Command::new("ffmpeg");
    command.args(&args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    crate::ffmpeg::die_with_parent(&mut command);
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
    if let Some(meta) = metadata_file {
        let _ = std::fs::remove_file(meta);
    }

    if cancelled || cancel.load(Ordering::Relaxed) {
        let _ = std::fs::remove_file(&e.output);
        return Err(ExportError::Ffmpeg("Cancelled".into()));
    }
    if !status.success() {
        let _ = std::fs::remove_file(&e.output);
        let reason = stderr_text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("FFmpeg failed").to_string();
        tracing::error!("ffmpeg export failed:\n{stderr_text}");
        return Err(ExportError::Ffmpeg(reason));
    }
    progress(1.0);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempo_timeline::{MediaSource, RationalFps};

    fn sample() -> Option<PathBuf> {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/media/sample_1080p_h264.mp4");
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
        TimelineExport { width: 640, height: 360, crf: 30, audio_kbps: 96, fit: Fit::Fit, range: None, chapters: Vec::new(), output }
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
        let (args, total) = build_ffmpeg_args(&project, &settings("/tmp/x.mp4".into()), None).unwrap();
        assert_eq!(total, 3_000_000);
        let graph = &args[args.iter().position(|a| a == "-filter_complex").unwrap() + 1];
        assert!(graph.contains("concat=n=3:v=1:a=0"), "two clips and one gap on the video track");
        assert!(graph.contains("anullsrc"), "silence fills the audio gap");
        assert!(graph.contains("[aout]"));

        let mut ranged = settings("/tmp/x.mp4".into());
        ranged.range = Some((500_000, 2_500_000));
        let (_, total) = build_ffmpeg_args(&project, &ranged, None).unwrap();
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
