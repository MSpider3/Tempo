use std::path::PathBuf;
use std::process::Command;

#[test]
fn test_proxy_generation() {
    let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let input_path = project_root.join("tests/media/sample_1080p_h264.mp4");
    let output_path = PathBuf::from("/tmp/sample_media_proxy.mp4");

    if output_path.exists() {
        let _ = std::fs::remove_file(&output_path);
    }

    // Generate proxy using standard ffmpeg parameters (H.264, veryfast, crf 18)
    let status = Command::new("ffmpeg")
        .args([
            "-y",
            "-i", input_path.to_str().unwrap(),
            "-c:v", "libx264",
            "-preset", "veryfast",
            "-crf", "18",
            "-pix_fmt", "yuv420p",
            "-c:a", "aac",
            output_path.to_str().unwrap(),
        ])
        .status()
        .expect("Failed to execute ffmpeg");

    assert!(status.success(), "ffmpeg failed to generate proxy");
    assert!(output_path.exists(), "Proxy file must exist");

    // Verify H.264
    let probe = Command::new("ffprobe")
        .args([
            "-v", "error",
            "-select_streams", "v:0",
            "-show_entries", "stream=codec_name",
            "-of", "default=noprint_wrappers=1",
            output_path.to_str().unwrap(),
        ])
        .output()
        .expect("ffprobe failed");

    let probe_text = String::from_utf8_lossy(&probe.stdout);
    assert!(probe_text.contains("codec_name=h264"), "Proxy must be H.264, got: {}", probe_text);

    // Verify duration matches original
    let orig_probe = Command::new("ffprobe")
        .args([
            "-v", "error",
            "-show_entries", "format=duration",
            "-of", "default=noprint_wrappers=1:nokey=1",
            input_path.to_str().unwrap(),
        ])
        .output()
        .expect("orig ffprobe failed");
    let orig_duration: f64 = String::from_utf8_lossy(&orig_probe.stdout).trim().parse().unwrap_or(0.0);

    let proxy_probe = Command::new("ffprobe")
        .args([
            "-v", "error",
            "-show_entries", "format=duration",
            "-of", "default=noprint_wrappers=1:nokey=1",
            output_path.to_str().unwrap(),
        ])
        .output()
        .expect("proxy ffprobe failed");
    let proxy_duration: f64 = String::from_utf8_lossy(&proxy_probe.stdout).trim().parse().unwrap_or(0.0);

    assert!(
        (orig_duration - proxy_duration).abs() < 0.2,
        "Duration must match original within 0.2s: orig={}, proxy={}",
        orig_duration,
        proxy_duration
    );
}
