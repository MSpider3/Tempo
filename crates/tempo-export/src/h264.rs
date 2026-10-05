use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Instant;
use crate::error::{ExportError, Result};

#[derive(Debug, Clone)]
pub struct ExportSettings {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub crf: u8,
    pub output_path: PathBuf,
    pub audio_source_path: Option<PathBuf>,
    pub audio_start_sec: Option<f64>,
}

impl Default for ExportSettings {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            fps: 24.0,
            crf: 18,
            output_path: PathBuf::from("/tmp/test_export.mp4"),
            audio_source_path: None,
            audio_start_sec: None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ExportProgress {
    pub progress: f32, // 0.0 to 1.0
    pub current_frame: u64,
    pub total_frames: u64,
    pub elapsed_secs: f64,
}

pub fn export_raw_frames<F>(
    settings: &ExportSettings,
    total_frames: u64,
    mut frame_generator: F,
    mut progress_cb: Option<Box<dyn FnMut(ExportProgress)>>,
) -> Result<()>
where
    F: FnMut(u64) -> Vec<u8>,
{
    let width = settings.width.max(16);
    let height = settings.height.max(16);
    let fps = if settings.fps <= 0.0 { 24.0 } else { settings.fps };
    let crf = settings.crf.min(51);

    let output_str = settings.output_path.to_str().ok_or_else(|| {
        ExportError::InvalidParameter("Invalid output file path".into())
    })?;

    let mut args = vec![
        "-y".to_string(),
        "-f".to_string(), "rawvideo".to_string(),
        "-pixel_format".to_string(), "rgba".to_string(),
        "-video_size".to_string(), format!("{}x{}", width, height),
        "-framerate".to_string(), format!("{}", fps),
        "-i".to_string(), "pipe:0".to_string(),
    ];

    if let Some(ref audio_path) = settings.audio_source_path {
        if let Some(start_sec) = settings.audio_start_sec {
            args.push("-ss".to_string());
            args.push(format!("{:.6}", start_sec));
        }
        args.push("-i".to_string());
        args.push(audio_path.to_string_lossy().to_string());

        // Explicitly map rawvideo from pipe:0 and audio from audio_source_path
        args.push("-map".to_string());
        args.push("0:v:0".to_string());
        args.push("-map".to_string());
        args.push("1:a:0".to_string());
        args.push("-c:a".to_string());
        args.push("aac".to_string());
        args.push("-b:a".to_string());
        args.push("192k".to_string());
        args.push("-shortest".to_string());
    } else {
        // Explicitly map rawvideo from pipe:0
        args.push("-map".to_string());
        args.push("0:v:0".to_string());
    }

    args.extend([
        "-c:v".to_string(), "libx264".to_string(),
        "-preset".to_string(), "veryfast".to_string(),
        "-crf".to_string(), format!("{}", crf),
        "-pix_fmt".to_string(), "yuv420p".to_string(),
        output_str.to_string(),
    ]);

    let mut child = Command::new("ffmpeg")
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| ExportError::Ffmpeg(format!("Failed to spawn ffmpeg: {}", e)))?;

    let start_time = Instant::now();
    {
        let mut stdin = child.stdin.take().ok_or_else(|| {
            ExportError::Ffmpeg("Failed to open stdin to ffmpeg child process".into())
        })?;

        for frame_idx in 0..total_frames {
            let buffer = frame_generator(frame_idx);
            stdin.write_all(&buffer).map_err(ExportError::Io)?;

            if let Some(ref mut cb) = progress_cb {
                let prog = (frame_idx + 1) as f32 / total_frames as f32;
                cb(ExportProgress {
                    progress: prog,
                    current_frame: frame_idx + 1,
                    total_frames,
                    elapsed_secs: start_time.elapsed().as_secs_f64(),
                });
            }
        }
    }

    let output = child.wait_with_output().map_err(ExportError::Io)?;
    if !output.status.success() {
        let err_msg = String::from_utf8_lossy(&output.stderr);
        return Err(ExportError::Ffmpeg(format!("FFmpeg failed: {}", err_msg)));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_export_h264() {
        let output_path = PathBuf::from("/tmp/test_export.mp4");
        if output_path.exists() {
            let _ = std::fs::remove_file(&output_path);
        }

        let settings = ExportSettings {
            width: 1920,
            height: 1080,
            fps: 24.0,
            crf: 18,
            output_path: output_path.clone(),
            audio_source_path: None,
            audio_start_sec: None,
        };

        let duration_secs = 3.0;
        let total_frames = (settings.fps * duration_secs) as u64; // 72 frames
        let frame_size = (settings.width * settings.height * 4) as usize;

        // Generate dynamic video frames (gradient with noise/motion so file size > 100 KB)
        export_raw_frames(
            &settings,
            total_frames,
            |frame_idx| {
                let mut data = vec![0u8; frame_size];
                let offset = (frame_idx * 4) as u8;
                for y in 0..settings.height {
                    let y_byte = (y & 0xFF) as u8;
                    let row_start = (y * settings.width * 4) as usize;
                    for x in 0..settings.width {
                        let x_byte = (x & 0xFF) as u8;
                        let idx = row_start + (x * 4) as usize;
                        data[idx] = x_byte.wrapping_add(offset);
                        data[idx + 1] = y_byte.wrapping_add(offset);
                        data[idx + 2] = (x_byte ^ y_byte).wrapping_add(offset);
                        data[idx + 3] = 255;
                    }
                }
                data
            },
            None,
        )
        .expect("Export failed");

        // 1. Assert file exists
        assert!(output_path.exists(), "Output file /tmp/test_export.mp4 must exist");

        // 2. Assert size > 100 KB
        let metadata = std::fs::metadata(&output_path).expect("Failed to read metadata");
        let size_kb = metadata.len() / 1024;
        assert!(
            metadata.len() > 100 * 1024,
            "File size must be > 100 KB, got {} KB ({} bytes)",
            size_kb,
            metadata.len()
        );

        // 3. ffprobe reports 1920x1080 @ 24fps H.264
        let probe_output = Command::new("ffprobe")
            .args([
                "-v", "error",
                "-select_streams", "v:0",
                "-show_entries", "stream=width,height,codec_name,r_frame_rate",
                "-of", "default=noprint_wrappers=1",
                output_path.to_str().unwrap(),
            ])
            .output()
            .expect("Failed to execute ffprobe");

        let probe_text = String::from_utf8_lossy(&probe_output.stdout);
        assert!(
            probe_text.contains("codec_name=h264"),
            "Expected codec_name=h264, got: {}",
            probe_text
        );
        assert!(
            probe_text.contains("width=1920"),
            "Expected width=1920, got: {}",
            probe_text
        );
        assert!(
            probe_text.contains("height=1080"),
            "Expected height=1080, got: {}",
            probe_text
        );
        assert!(
            probe_text.contains("r_frame_rate=24/1"),
            "Expected r_frame_rate=24/1, got: {}",
            probe_text
        );
    }
}
