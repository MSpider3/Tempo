//! Exports a project without opening the interface, for checks and debugging:
//!   cargo run -p tempo-app --example export_project -- in.tempo out.mp4 [start_s end_s]

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use tempo_export::{export_timeline, Fit, TimelineExport};

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
    let settings = TimelineExport {
        width: project.width,
        height: project.height,
        crf: 23,
        audio_kbps: 160,
        fit: Fit::Fit,
        range,
        chapters: Vec::new(),
        output: PathBuf::from(output),
        hardware: false,
    };
    let started = std::time::Instant::now();
    export_timeline(&project, &settings, &AtomicBool::new(false), |_| {})?;
    println!("exported {} in {:.1} s", output, started.elapsed().as_secs_f64());
    Ok(())
}
