use std::env;
use std::path::Path;
use tempo_media::MediaEngine;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: probe_test <media_file_path>");
        std::process::exit(1);
    }

    let file_path = &args[1];
    println!("Probing file: {}", file_path);

    let start = std::time::Instant::now();
    match MediaEngine::probe(Path::new(file_path)) {
        Ok(info) => {
            let elapsed = start.elapsed();
            println!("Probe succeeded in {:.2?}:", elapsed);
            println!("  Path: {}", file_path);
            println!("  Media type: {:?}", info.media_type);
            println!("  Duration: {} ms ({} us)", info.duration_us / 1000, info.duration_us);
            println!("  Video: {:?}", info.video);
            println!("  Audio: {:?}", info.audio);
            println!("  Thumbnail extracted: {} bytes", info.thumbnail_rgba.as_ref().map(|t| t.len()).unwrap_or(0));
            assert!(elapsed.as_millis() < 100, "Probe must complete in < 100ms");
        }
        Err(e) => {
            eprintln!("Probe failed: {:?}", e);
            std::process::exit(1);
        }
    }
}
