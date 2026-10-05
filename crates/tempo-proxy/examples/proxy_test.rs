use std::env;
use std::path::PathBuf;
use std::process::Command;
use tempo_proxy::ProxyEngine;

fn main() {
    let args: Vec<String> = env::args().collect();
    let mut input_path = None;
    let mut output_path = None;

    let mut i = 1;
    while i < args.len() {
        if args[i] == "--input" && i + 1 < args.len() {
            input_path = Some(PathBuf::from(&args[i + 1]));
            i += 2;
        } else if args[i] == "--output" && i + 1 < args.len() {
            output_path = Some(PathBuf::from(&args[i + 1]));
            i += 2;
        } else {
            i += 1;
        }
    }

    let input = input_path.unwrap_or_else(|| {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        manifest.parent().unwrap().parent().unwrap().join("tests/media/sample_1080p_h264.mp4")
    });

    let output = output_path.unwrap_or_else(|| PathBuf::from("/tmp/test.proxy.mp4"));

    println!("Generating proxy from {:?} to {:?}", input, output);
    ProxyEngine::generate_proxy_sync(&input, &output).expect("Failed to generate proxy");

    // Verify proxy file exists
    assert!(output.exists(), "Proxy file does not exist at {:?}", output);

    // Verify H.264
    let probe = Command::new("ffprobe")
        .args([
            "-v", "error",
            "-select_streams", "v:0",
            "-show_entries", "stream=codec_name",
            "-of", "default=noprint_wrappers=1",
            output.to_str().unwrap(),
        ])
        .output()
        .expect("ffprobe failed");

    let probe_text = String::from_utf8_lossy(&probe.stdout);
    assert!(probe_text.contains("codec_name=h264"), "Proxy must be H.264, got: {}", probe_text);

    println!("Proxy verification successful: {:?}", output);
}
