//! tempo-proxy: small, easy-to-decode copies of heavy footage.
//!
//! A proxy is used only for preview. It is a quarter of the pixels of 1080p,
//! has a keyframe twice a second so scrubbing is quick, and is made in the
//! background at the lowest CPU priority. Export always reads the original.

use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};

use tempo_timeline::{MediaSource, MediaType};

/// Height of proxy files.
pub const PROXY_HEIGHT: u32 = 540;

/// Footage at or below this height that is already H.264 plays well as it is.
const LIGHT_HEIGHT: u32 = 720;

/// Whether a source is worth a proxy on a weak machine.
pub fn needs_proxy(source: &MediaSource) -> bool {
    if source.media_type != MediaType::Video || source.is_missing {
        return false;
    }
    let height = source.video_height.unwrap_or(0);
    let light_codec = matches!(source.video_codec.as_deref(), Some("h264" | "mpeg4" | "mpeg2video" | "mjpeg"));
    height > LIGHT_HEIGHT || !light_codec
}

/// Make a proxy of `input` at `output`. Blocking: call from a worker thread.
/// `progress` receives 0.0–1.0. Setting `cancel` stops the work; an unfinished
/// file is never left at `output`.
pub fn generate_proxy(
    input: &Path,
    output: &Path,
    duration_us: i64,
    cancel: &AtomicBool,
    mut progress: impl FnMut(f32),
) -> Result<(), String> {
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    // Write beside the target and rename at the end, so a crash never leaves a
    // half-written file that looks finished.
    let partial = output.with_extension("part.mp4");

    let mut child = Command::new("nice")
        .args(["-n", "19", "ffmpeg", "-y", "-hide_banner", "-nostats", "-progress", "pipe:1", "-i"])
        .arg(input)
        .args([
            "-map", "0:v:0", "-map", "0:a:0?",
            "-vf", &format!("scale=-2:{PROXY_HEIGHT}"),
            "-c:v", "libx264", "-preset", "ultrafast", "-crf", "28", "-pix_fmt", "yuv420p",
            // A keyframe every half second keeps seeking cheap.
            "-g", "15", "-bf", "0",
            // Leave cores free for playback while the proxy is made.
            "-threads", "2",
            "-c:a", "aac", "-b:a", "128k",
        ])
        .arg(&partial)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Could not start ffmpeg: {e}"))?;

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
                progress((us as f32 / duration_us.max(1) as f32).clamp(0.0, 1.0));
            }
        }
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    let stderr_text = errors.join().unwrap_or_default();

    if cancelled || cancel.load(Ordering::Relaxed) {
        let _ = std::fs::remove_file(&partial);
        return Err("Cancelled".into());
    }
    if !status.success() {
        let _ = std::fs::remove_file(&partial);
        tracing::warn!("proxy generation failed for {}:\n{stderr_text}", input.display());
        return Err(stderr_text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("ffmpeg failed").to_string());
    }
    std::fs::rename(&partial, output).map_err(|e| e.to_string())?;
    progress(1.0);
    Ok(())
}

/// Kept for `examples/proxy_test.rs`.
pub struct ProxyEngine;

impl ProxyEngine {
    pub fn generate_proxy_sync(input: &Path, output: &Path) -> Result<(), String> {
        generate_proxy(input, output, 1, &AtomicBool::new(false), |_| {})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn media(name: &str) -> Option<PathBuf> {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/media").join(name);
        (p.exists() && Command::new("ffmpeg").arg("-version").output().is_ok()).then_some(p)
    }

    #[test]
    fn heavy_footage_needs_a_proxy_and_light_footage_does_not() {
        let mut s = MediaSource::new("a.mp4".into(), MediaType::Video, 1_000_000);
        s.video_height = Some(720);
        s.video_codec = Some("h264".into());
        assert!(!needs_proxy(&s));
        s.video_height = Some(1080);
        assert!(needs_proxy(&s));
        s.video_height = Some(720);
        s.video_codec = Some("hevc".into());
        assert!(needs_proxy(&s));
        s.media_type = MediaType::Audio;
        assert!(!needs_proxy(&s));
    }

    #[test]
    fn proxy_is_small_h264_with_progress() {
        let Some(input) = media("sample_1080p_h264.mp4") else { return };
        let output = std::env::temp_dir().join(format!("tempo-proxy-test-{}.mp4", std::process::id()));
        let mut last = 0.0;
        generate_proxy(&input, &output, 5_000_000, &AtomicBool::new(false), |p| last = p).expect("proxy");
        assert_eq!(last, 1.0);
        let probe = Command::new("ffprobe")
            .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=codec_name,height", "-of", "default=nw=1"])
            .arg(&output)
            .output()
            .expect("ffprobe");
        let text = String::from_utf8_lossy(&probe.stdout);
        let _ = std::fs::remove_file(&output);
        assert!(text.contains("codec_name=h264") && text.contains("height=540"), "{text}");
    }

    #[test]
    fn cancel_leaves_no_file() {
        let Some(input) = media("sample_1080p_h264.mp4") else { return };
        let output = std::env::temp_dir().join(format!("tempo-proxy-cancel-{}.mp4", std::process::id()));
        assert!(generate_proxy(&input, &output, 5_000_000, &AtomicBool::new(true), |_| {}).is_err());
        assert!(!output.exists() && !output.with_extension("part.mp4").exists());
    }
}
