use image::{ImageBuffer, Rgba};
use std::env;
use std::fs::create_dir_all;
use std::path::Path;
use tempo_media::FfmpegDecoder;
use tempo_render::{LayerDesc, WgpuRenderer};
use wgpu::{
    Extent3d, ImageCopyTexture, ImageDataLayout, Origin3d, TextureAspect, TextureDescriptor,
    TextureDimension, TextureFormat, TextureUsages, TextureViewDescriptor,
};

fn main() {
    println!("=== Color Correction Render Test ===");
    let args: Vec<String> = env::args().collect();
    let media_path = if args.len() > 1 && !args[1].starts_with("--") {
        args[1].clone()
    } else {
        "tests/media/sample_1080p_h264.mp4".to_string()
    };

    println!("Media source: {}", media_path);

    let renderer = WgpuRenderer::new().expect("Failed to initialize WgpuRenderer");
    let mut decoder = FfmpegDecoder::open(Path::new(&media_path)).expect("Failed to open media file");

    let width = decoder.video_width();
    let height = decoder.video_height();
    let original_frame = decoder
        .decode_video_frame(0)
        .expect("Failed to decode reference frame");

    let ref_dir = Path::new("tests/render_references");
    create_dir_all(ref_dir).expect("Failed to create tests/render_references dir");
    let out_dir = Path::new("target/test_output");
    create_dir_all(out_dir).expect("Failed to create target/test_output dir");

    // Upload original frame to GPU texture
    let texture = renderer.device.create_texture(&TextureDescriptor {
        label: Some("Color Correction Input Texture"),
        size: Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: TextureFormat::Rgba8Unorm,
        usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
        view_formats: &[],
    });

    renderer.queue.write_texture(
        ImageCopyTexture {
            texture: &texture,
            mip_level: 0,
            origin: Origin3d::ZERO,
            aspect: TextureAspect::All,
        },
        &original_frame.data,
        ImageDataLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(height),
        },
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    let view = texture.create_view(&TextureViewDescriptor::default());

    // Test Case 1: Neutral / Identity (lift=0, gamma=1, gain=1)
    println!("Testing Case 1: Neutral Pass...");
    let layer_neutral = LayerDesc::new(&view);
    let neutral_rgba = renderer
        .composite_layers(width, height, &[layer_neutral])
        .expect("Failed neutral render pass");

    // Test Case 2: Warm Cinematic Grade
    // Lift: boost red shadows (+0.08, 0.0, -0.05)
    // Gamma: warm midtones (1.1, 1.0, 0.9)
    // Gain: golden highlights (1.2, 1.05, 0.85)
    println!("Testing Case 2: Warm Grade...");
    let warm_lift = [0.08, 0.0, -0.05];
    let warm_gamma = [1.1, 1.0, 0.9];
    let warm_gain = [1.2, 1.05, 0.85];
    let layer_warm = LayerDesc::new(&view).with_color_correction(warm_lift, warm_gamma, warm_gain);
    let warm_rgba = renderer
        .composite_layers(width, height, &[layer_warm])
        .expect("Failed warm render pass");

    // Test Case 3: Cool High-Contrast Grade
    // Lift: crush shadows, cool tint (-0.05, -0.05, 0.08)
    // Gamma: contrasty cool (0.9, 0.95, 1.1)
    // Gain: icy highlights (0.9, 1.0, 1.25)
    println!("Testing Case 3: Cool Grade...");
    let cool_lift = [-0.05, -0.05, 0.08];
    let cool_gamma = [0.9, 0.95, 1.1];
    let cool_gain = [0.9, 1.0, 1.25];
    let layer_cool = LayerDesc::new(&view).with_color_correction(cool_lift, cool_gamma, cool_gain);
    let cool_rgba = renderer
        .composite_layers(width, height, &[layer_cool])
        .expect("Failed cool render pass");

    // Verify color shifts:
    // Warm: total red > neutral red, total blue < neutral blue
    let mut neutral_r_sum: u64 = 0;
    let mut neutral_b_sum: u64 = 0;
    let mut warm_r_sum: u64 = 0;
    let mut warm_b_sum: u64 = 0;
    let mut cool_r_sum: u64 = 0;
    let mut cool_b_sum: u64 = 0;

    for i in 0..(width * height) as usize {
        neutral_r_sum += neutral_rgba[i * 4] as u64;
        neutral_b_sum += neutral_rgba[i * 4 + 2] as u64;

        warm_r_sum += warm_rgba[i * 4] as u64;
        warm_b_sum += warm_rgba[i * 4 + 2] as u64;

        cool_r_sum += cool_rgba[i * 4] as u64;
        cool_b_sum += cool_rgba[i * 4 + 2] as u64;
    }

    println!("Channel Sums Check:");
    println!("  Neutral: Red={}, Blue={}", neutral_r_sum, neutral_b_sum);
    println!("  Warm:    Red={}, Blue={}", warm_r_sum, warm_b_sum);
    println!("  Cool:    Red={}, Blue={}", cool_r_sum, cool_b_sum);

    assert!(
        warm_r_sum > neutral_r_sum,
        "Warm grade should increase red channel!"
    );
    assert!(
        cool_b_sum > neutral_b_sum,
        "Cool grade should increase blue channel!"
    );

    // Save test outputs
    let warm_img: ImageBuffer<Rgba<u8>, _> =
        ImageBuffer::from_raw(width, height, warm_rgba.clone()).unwrap();
    warm_img
        .save(out_dir.join("color_warm_output.png"))
        .unwrap();

    let cool_img: ImageBuffer<Rgba<u8>, _> =
        ImageBuffer::from_raw(width, height, cool_rgba.clone()).unwrap();
    cool_img
        .save(out_dir.join("color_cool_output.png"))
        .unwrap();

    // If reference files do not exist, save them as ground truth
    let ref_warm_path = ref_dir.join("color_warm_ref.png");
    let ref_cool_path = ref_dir.join("color_cool_ref.png");

    if !ref_warm_path.exists() {
        println!("Generating reference file: {}", ref_warm_path.display());
        warm_img.save(&ref_warm_path).unwrap();
    }
    if !ref_cool_path.exists() {
        println!("Generating reference file: {}", ref_cool_path.display());
        cool_img.save(&ref_cool_path).unwrap();
    }

    // Compare with reference file
    let ref_warm_img = image::open(&ref_warm_path)
        .expect("Failed to open warm reference")
        .to_rgba8();
    let diff_warm_pixels = warm_rgba
        .iter()
        .zip(ref_warm_img.as_raw().iter())
        .filter(|(a, b)| (a.abs_diff(**b)) > 1)
        .count();
    println!(
        "Warm grade comparison to reference: {} differing subpixels (>1)",
        diff_warm_pixels
    );
    assert!(
        diff_warm_pixels == 0,
        "Warm grade output must match reference PNG!"
    );

    let ref_cool_img = image::open(&ref_cool_path)
        .expect("Failed to open cool reference")
        .to_rgba8();
    let diff_cool_pixels = cool_rgba
        .iter()
        .zip(ref_cool_img.as_raw().iter())
        .filter(|(a, b)| (a.abs_diff(**b)) > 1)
        .count();
    println!(
        "Cool grade comparison to reference: {} differing subpixels (>1)",
        diff_cool_pixels
    );
    assert!(
        diff_cool_pixels == 0,
        "Cool grade output must match reference PNG!"
    );

    println!("\nColor Correction Render Test PASSED!");
}
