use image::{ImageBuffer, Rgba};
use std::path::Path;
use tempo_render::title::{render_title_to_rgba, TitleProperties};

fn main() {
    println!("=== Title Render Test ===");
    let width = 1920;
    let height = 1080;

    let output_dir = Path::new("target/test_output");
    std::fs::create_dir_all(output_dir).expect("Failed to create test_output dir");

    // 1. Center Title
    println!("Rendering Center Title (1920x1080)...");
    let center_props = TitleProperties::center_title("TEMPO CINEMATIC TITLE");
    let center_rgba = render_title_to_rgba(&center_props, width, height)
        .expect("Failed to render Center Title");

    let non_transparent_center = center_rgba.chunks(4).filter(|p| p[3] > 10).count();
    println!(
        "Center Title: {} non-transparent pixels rendered",
        non_transparent_center
    );
    assert!(
        non_transparent_center > 10_000,
        "Center Title pixel count too low: {}",
        non_transparent_center
    );

    let center_img: ImageBuffer<Rgba<u8>, _> =
        ImageBuffer::from_raw(width, height, center_rgba.clone())
            .expect("Failed to create center image buffer");
    let center_path = output_dir.join("title_center.png");
    center_img
        .save(&center_path)
        .expect("Failed to save center title PNG");
    println!("Saved Center Title to: {}", center_path.display());

    // 2. Lower Third
    println!("Rendering Lower Third (1920x1080)...");
    let lower_props = TitleProperties::lower_third("Mehul Golecha", "Lead Architect & Editor");
    let lower_rgba = render_title_to_rgba(&lower_props, width, height)
        .expect("Failed to render Lower Third");

    let non_transparent_lower = lower_rgba.chunks(4).filter(|p| p[3] > 10).count();
    println!(
        "Lower Third: {} non-transparent pixels rendered",
        non_transparent_lower
    );
    assert!(
        non_transparent_lower > 20_000,
        "Lower Third pixel count too low: {}",
        non_transparent_lower
    );

    let lower_img: ImageBuffer<Rgba<u8>, _> =
        ImageBuffer::from_raw(width, height, lower_rgba.clone())
            .expect("Failed to create lower third image buffer");
    let lower_path = output_dir.join("title_lower_third.png");
    lower_img
        .save(&lower_path)
        .expect("Failed to save lower third PNG");
    println!("Saved Lower Third to: {}", lower_path.display());

    println!("Title Render Test PASSED successfully!");
}
