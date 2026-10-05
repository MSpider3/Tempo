use cairo::{Context, FontSlant, FontWeight, Format, ImageSurface};
use crate::error::{RenderError, Result};
use crate::wgpu_engine::WgpuRenderer;
use wgpu::{
    Extent3d, ImageCopyTexture, ImageDataLayout, Origin3d, TextureAspect, TextureDescriptor,
    TextureDimension, TextureFormat, TextureUsages, TextureView, TextureViewDescriptor,
};

#[derive(Clone, Debug, PartialEq)]
pub enum TitleType {
    CenterTitle,
    LowerThird,
    Custom,
}

#[derive(Clone, Debug)]
pub struct TitleProperties {
    pub title_type: TitleType,
    pub text: String,
    pub subtitle: Option<String>,
    pub font_family: String,
    pub font_size: f64,
    pub text_color: [f64; 4], // RGBA in 0.0..1.0
    pub bg_color: [f64; 4],   // RGBA in 0.0..1.0
    pub pos_x: f64,           // 0.0..1.0 relative
    pub pos_y: f64,           // 0.0..1.0 relative
}

impl Default for TitleProperties {
    fn default() -> Self {
        Self {
            title_type: TitleType::CenterTitle,
            text: "Title Text".to_string(),
            subtitle: None,
            font_family: "Sans".to_string(),
            font_size: 64.0,
            text_color: [1.0, 1.0, 1.0, 1.0],
            bg_color: [0.0, 0.0, 0.0, 0.0],
            pos_x: 0.5,
            pos_y: 0.5,
        }
    }
}

impl TitleProperties {
    pub fn center_title(text: &str) -> Self {
        Self {
            title_type: TitleType::CenterTitle,
            text: text.to_string(),
            subtitle: None,
            font_family: "Sans".to_string(),
            font_size: 72.0,
            text_color: [1.0, 1.0, 1.0, 1.0],
            bg_color: [0.0, 0.0, 0.0, 0.4],
            pos_x: 0.5,
            pos_y: 0.5,
        }
    }

    pub fn lower_third(title: &str, subtitle: &str) -> Self {
        Self {
            title_type: TitleType::LowerThird,
            text: title.to_string(),
            subtitle: Some(subtitle.to_string()),
            font_family: "Sans".to_string(),
            font_size: 48.0,
            text_color: [1.0, 1.0, 1.0, 1.0],
            bg_color: [0.1, 0.1, 0.15, 0.85],
            pos_x: 0.08,
            pos_y: 0.82,
        }
    }
}

/// Renders title overlay text into an RGBA8 pixel buffer using Cairo.
pub fn render_title_to_rgba(
    props: &TitleProperties,
    width: u32,
    height: u32,
) -> Result<Vec<u8>> {
    let mut surface = ImageSurface::create(Format::ARgb32, width as i32, height as i32)
        .map_err(|e| RenderError::TitleError(format!("Cairo surface error: {:?}", e)))?;

    let cr = Context::new(&surface)
        .map_err(|e| RenderError::TitleError(format!("Cairo context error: {:?}", e)))?;

    // Clear transparent
    cr.set_operator(cairo::Operator::Clear);
    cr.paint().map_err(|e| RenderError::TitleError(format!("{:?}", e)))?;
    cr.set_operator(cairo::Operator::Over);

    match props.title_type {
        TitleType::CenterTitle => {
            cr.select_font_face(&props.font_family, FontSlant::Normal, FontWeight::Bold);
            cr.set_font_size(props.font_size);
            let extents = cr
                .text_extents(&props.text)
                .map_err(|e| RenderError::TitleError(format!("{:?}", e)))?;

            let x = (width as f64 - extents.width()) / 2.0 - extents.x_bearing();
            let y = (height as f64 - extents.height()) / 2.0 - extents.y_bearing();

            // Background pill if bg alpha > 0
            if props.bg_color[3] > 0.0 {
                let pad_x = 24.0;
                let pad_y = 16.0;
                cr.set_source_rgba(
                    props.bg_color[0],
                    props.bg_color[1],
                    props.bg_color[2],
                    props.bg_color[3],
                );
                cr.rectangle(
                    x + extents.x_bearing() - pad_x,
                    y + extents.y_bearing() - pad_y,
                    extents.width() + pad_x * 2.0,
                    extents.height() + pad_y * 2.0,
                );
                let _ = cr.fill();
            }

            // Draw text
            cr.set_source_rgba(
                props.text_color[0],
                props.text_color[1],
                props.text_color[2],
                props.text_color[3],
            );
            cr.move_to(x, y);
            let _ = cr.show_text(&props.text);
        }
        TitleType::LowerThird => {
            let base_x = (width as f64) * props.pos_x;
            let base_y = (height as f64) * props.pos_y;

            cr.select_font_face(&props.font_family, FontSlant::Normal, FontWeight::Bold);
            cr.set_font_size(props.font_size);
            let title_extents = cr
                .text_extents(&props.text)
                .map_err(|e| RenderError::TitleError(format!("{:?}", e)))?;

            let sub_font_size = props.font_size * 0.55;
            let sub_text = props.subtitle.as_deref().unwrap_or("");
            cr.set_font_size(sub_font_size);
            let sub_extents = cr
                .text_extents(sub_text)
                .map_err(|e| RenderError::TitleError(format!("{:?}", e)))?;

            let max_w = title_extents.width().max(sub_extents.width());
            let pad_x = 28.0;
            let pad_y = 18.0;
            let total_h = title_extents.height() + sub_extents.height() + 16.0;

            // Background banner
            if props.bg_color[3] > 0.0 {
                cr.set_source_rgba(
                    props.bg_color[0],
                    props.bg_color[1],
                    props.bg_color[2],
                    props.bg_color[3],
                );
                cr.rectangle(
                    base_x - pad_x,
                    base_y - pad_y,
                    max_w + pad_x * 2.0 + 8.0,
                    total_h + pad_y * 2.0,
                );
                let _ = cr.fill();

                // Accent vertical bar on left edge
                cr.set_source_rgba(0.9, 0.4, 0.1, 1.0); // Tempo amber/orange accent
                cr.rectangle(base_x - pad_x, base_y - pad_y, 6.0, total_h + pad_y * 2.0);
                let _ = cr.fill();
            }

            // Draw primary title
            cr.select_font_face(&props.font_family, FontSlant::Normal, FontWeight::Bold);
            cr.set_font_size(props.font_size);
            cr.set_source_rgba(
                props.text_color[0],
                props.text_color[1],
                props.text_color[2],
                props.text_color[3],
            );
            cr.move_to(base_x, base_y + title_extents.height());
            let _ = cr.show_text(&props.text);

            // Draw subtitle
            if !sub_text.is_empty() {
                cr.select_font_face(&props.font_family, FontSlant::Normal, FontWeight::Normal);
                cr.set_font_size(sub_font_size);
                cr.set_source_rgba(0.85, 0.85, 0.85, 0.95);
                cr.move_to(
                    base_x,
                    base_y + title_extents.height() + 14.0 + sub_extents.height(),
                );
                let _ = cr.show_text(sub_text);
            }
        }
        TitleType::Custom => {
            let x = (width as f64) * props.pos_x;
            let y = (height as f64) * props.pos_y;

            cr.select_font_face(&props.font_family, FontSlant::Normal, FontWeight::Normal);
            cr.set_font_size(props.font_size);
            cr.set_source_rgba(
                props.text_color[0],
                props.text_color[1],
                props.text_color[2],
                props.text_color[3],
            );
            cr.move_to(x, y);
            let _ = cr.show_text(&props.text);
        }
    }

    drop(cr);
    surface.flush();

    let data = surface
        .data()
        .map_err(|e| RenderError::TitleError(format!("{:?}", e)))?;

    // Convert Cairo ARgb32 (native LE: B, G, R, A) into standard RGBA (R, G, B, A)
    let pixel_count = (width * height) as usize;
    let mut rgba = vec![0u8; pixel_count * 4];

    for i in 0..pixel_count {
        let b = data[i * 4];
        let g = data[i * 4 + 1];
        let r = data[i * 4 + 2];
        let a = data[i * 4 + 3];

        rgba[i * 4] = r;
        rgba[i * 4 + 1] = g;
        rgba[i * 4 + 2] = b;
        rgba[i * 4 + 3] = a;
    }

    Ok(rgba)
}

/// Renders title overlay text and uploads it to a wgpu Texture & TextureView.
pub fn render_title_to_texture(
    renderer: &WgpuRenderer,
    props: &TitleProperties,
    width: u32,
    height: u32,
) -> Result<(wgpu::Texture, TextureView)> {
    let rgba = render_title_to_rgba(props, width, height)?;

    let texture = renderer.device.create_texture(&TextureDescriptor {
        label: Some("Title Overlay Texture"),
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
        &rgba,
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
    Ok((texture, view))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_center_title() {
        let props = TitleProperties::center_title("Test Center Title");
        let rgba = render_title_to_rgba(&props, 640, 360).expect("Title render failed");
        assert_eq!(rgba.len(), 640 * 360 * 4);

        // Verify that there are non-black / non-transparent pixels
        let non_transparent_count = rgba.chunks(4).filter(|p| p[3] > 0).count();
        assert!(
            non_transparent_count > 500,
            "Center title should render visible text pixels, got {}",
            non_transparent_count
        );
    }

    #[test]
    fn test_render_lower_third() {
        let props = TitleProperties::lower_third("Jane Doe", "Lead Director");
        let rgba = render_title_to_rgba(&props, 640, 360).expect("Lower third render failed");
        assert_eq!(rgba.len(), 640 * 360 * 4);

        let non_transparent_count = rgba.chunks(4).filter(|p| p[3] > 0).count();
        assert!(
            non_transparent_count > 1000,
            "Lower third should render banner and text pixels, got {}",
            non_transparent_count
        );
    }
}
