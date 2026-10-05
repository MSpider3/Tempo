//! Helpers for running the `ffmpeg` program.

use std::os::unix::process::CommandExt;
use std::process::Command;
use std::sync::OnceLock;

/// The H.264 encoder this FFmpeg build has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum H264Encoder {
    /// Best quality for the size; missing from some distributions' FFmpeg.
    X264,
    /// Always present in Fedora's `ffmpeg-free`.
    OpenH264,
}

/// Which encoder to use, found once by asking `ffmpeg -encoders`.
pub fn h264_encoder() -> H264Encoder {
    static FOUND: OnceLock<H264Encoder> = OnceLock::new();
    *FOUND.get_or_init(|| {
        let listing = Command::new("ffmpeg").args(["-hide_banner", "-encoders"]).output();
        let text = listing.map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
        if text.contains(" libx264 ") || text.is_empty() {
            H264Encoder::X264
        } else {
            H264Encoder::OpenH264
        }
    })
}

/// Encoder arguments for a quality level given as an x264 CRF (18 high … 28 small).
/// `fast` trades size for speed (used for proxies).
pub fn h264_args(crf: u8, width: u32, height: u32, fast: bool) -> Vec<String> {
    match h264_encoder() {
        H264Encoder::X264 => vec![
            "-c:v".into(),
            "libx264".into(),
            "-preset".into(),
            if fast { "ultrafast" } else { "veryfast" }.into(),
            "-crf".into(),
            crf.min(51).to_string(),
        ],
        H264Encoder::OpenH264 => {
            // OpenH264 has no CRF mode, so pick a bitrate that gives similar quality:
            // about 0.1 bits per pixel per frame at CRF 23, doubling every 6 steps down.
            let pixels = width as f64 * height as f64;
            let bits_per_pixel = 0.1 * 2f64.powf((23.0 - crf as f64) / 6.0);
            let kbps = (pixels * 30.0 * bits_per_pixel / 1000.0).clamp(300.0, 60_000.0) as u32;
            vec!["-c:v".into(), "libopenh264".into(), "-b:v".into(), format!("{kbps}k")]
        }
    }
}

/// Make the child stop when Tempo does, so closing the app never leaves an
/// encode running in the background.
pub fn die_with_parent(command: &mut Command) {
    // SAFETY: `prctl` is async-signal-safe and touches only the child process.
    unsafe {
        command.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
            Ok(())
        });
    }
}
