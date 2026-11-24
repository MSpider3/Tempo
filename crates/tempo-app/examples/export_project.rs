//! Exports a project without opening the interface, for checks and debugging:
//!   cargo run -p tempo-app --example export_project -- in.tempo out.mp4 [start_s end_s]

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use tempo_export::{export_timeline, ContainerFormat, TimelineExport};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(input), Some(output)) = (args.first(), args.get(1)) else {
        return Err("usage: export_project <in.tempo> <out.mp4> [start_s end_s]".into());
    };
    let range = match (args.get(2), args.get(3)) {
        (Some(a), Some(b)) => Some(((a.parse::<f64>()? * 1e6) as i64, (b.parse::<f64>()? * 1e6) as i64)),
        _ => None,
    };
    let project = tempo_project::load_project(&PathBuf::from(input))?;
    let output_path = PathBuf::from(output);
    let container = match output_path.extension().and_then(|e| e.to_str()) {
        Some(ext) if ext.eq_ignore_ascii_case("mov") => ContainerFormat::Mov,
        Some(ext) if ext.eq_ignore_ascii_case("mkv") => ContainerFormat::Mkv,
        _ => ContainerFormat::Mp4,
    };
    let settings = TimelineExport {
        width: project.width,
        height: project.height,
        range,
        output: output_path,
        // `TEMPO_HW_EXPORT=1` tries the graphics chip's encoder first, for timing it.
        hardware: std::env::var_os("TEMPO_HW_EXPORT").is_some(),
        container,
        ..TimelineExport::default()
    };
    let started = std::time::Instant::now();
    export_timeline(&project, &settings, &AtomicBool::new(false), |_| {})?;
    println!("exported {} in {:.1} s", output, started.elapsed().as_secs_f64());
    Ok(())
}
