//! The picture area. Draws the decoded layers with GTK's own renderer, so
//! scaling, placement and opacity are done by the GPU without extra copies.

use std::cell::{Cell, RefCell};

use glib::subclass::prelude::*;
use gtk4 as gtk;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use gtk4::{gdk, graphene, pango};
use tempo_timeline::{ClipProperties, TitleData, TitleType};

use crate::player::Layer;

/// What the surface keeps for each layer.
struct Drawn {
    texture: Option<gdk::Texture>,
    title: Option<TitleData>,
    props: ClipProperties,
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct VideoSurface {
        pub(super) layers: RefCell<Vec<Drawn>>,
        /// Project frame size; the picture is letterboxed to this shape.
        pub frame: Cell<(u32, u32)>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for VideoSurface {
        const NAME: &'static str = "TempoVideoSurface";
        type Type = super::VideoSurface;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for VideoSurface {}

    impl WidgetImpl for VideoSurface {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let widget = self.obj();
            let (w, h) = (widget.width() as f32, widget.height() as f32);
            let (pw, ph) = self.frame.get();
            if w <= 0.0 || h <= 0.0 || pw == 0 || ph == 0 {
                return;
            }
            let scale = (w / pw as f32).min(h / ph as f32);
            let (fw, fh) = (pw as f32 * scale, ph as f32 * scale);
            let (x0, y0) = ((w - fw) / 2.0, (h - fh) / 2.0);
            let frame_rect = graphene::Rect::new(x0, y0, fw, fh);

            snapshot.append_color(&gdk::RGBA::BLACK, &frame_rect);
            snapshot.push_clip(&frame_rect);
            for layer in self.layers.borrow().iter() {
                let props = &layer.props;
                snapshot.save();
                // Everything is placed relative to the centre of the frame.
                snapshot.translate(&graphene::Point::new(
                    x0 + fw / 2.0 + props.position_x * scale,
                    y0 + fh / 2.0 + props.position_y * scale,
                ));
                snapshot.rotate(props.rotation);
                snapshot.scale(props.scale_x, props.scale_y);
                let opaque = props.opacity >= 0.999;
                if !opaque {
                    snapshot.push_opacity(props.opacity.clamp(0.0, 1.0) as f64);
                }
                if let Some(texture) = &layer.texture {
                    let (tw, th) = (texture.width() as f32, texture.height() as f32);
                    if tw > 0.0 && th > 0.0 {
                        let fit = (fw / tw).min(fh / th);
                        let (dw, dh) = (tw * fit, th * fit);
                        // The clip's filters: a blur and a colour transform, done by the GPU.
                        let fx = tempo_timeline::resolve_effects(&props.effects);
                        let blur = fx.blur * fh / 1080.0;
                        if blur > 0.05 {
                            snapshot.push_blur(blur as f64);
                        }
                        let tinted = !fx.color.is_identity();
                        if tinted {
                            // graphene multiplies a row vector by the matrix, so the rows
                            // of our transform become its columns.
                            let m = &fx.color.matrix;
                            let mut flat = [0.0f32; 16];
                            for r in 0..4 {
                                for c in 0..4 {
                                    flat[c * 4 + r] = m[r][c];
                                }
                            }
                            let o = &fx.color.offset;
                            snapshot.push_color_matrix(&graphene::Matrix::from_float(flat), &graphene::Vec4::new(o[0], o[1], o[2], o[3]));
                        }
                        snapshot.append_texture(texture, &graphene::Rect::new(-dw / 2.0, -dh / 2.0, dw, dh));
                        if tinted {
                            snapshot.pop();
                        }
                        if blur > 0.05 {
                            snapshot.pop();
                        }
                    }
                }
                if let Some(title) = &layer.title {
                    super::draw_title(&widget, snapshot, title, scale, fh);
                }
                if !opaque {
                    snapshot.pop();
                }
                snapshot.restore();
            }
            snapshot.pop();
        }
    }
}

/// Draw a title with its centre at the origin (or in the lower third).
/// Sizes in `TitleData` are in project pixels; `scale` converts them to screen pixels.
fn draw_title(widget: &VideoSurface, snapshot: &gtk::Snapshot, title: &TitleData, scale: f32, frame_h: f32) {
    let layout = widget.create_pango_layout(Some(&title.text));
    let mut font = pango::FontDescription::from_string(&title.font_family);
    font.set_absolute_size((title.font_size * scale).max(1.0) as f64 * pango::SCALE as f64);
    font.set_weight(if title.font_bold { pango::Weight::Bold } else { pango::Weight::Normal });
    font.set_style(if title.font_italic { pango::Style::Italic } else { pango::Style::Normal });
    layout.set_font_description(Some(&font));
    layout.set_alignment(pango::Alignment::Center);
    let (tw, th) = layout.pixel_size();
    let (tw, th) = (tw as f32, th as f32);
    // A lower third sits at 80 % of the frame height.
    let y = match title.title_type {
        TitleType::CenterTitle => -th / 2.0,
        TitleType::LowerThird => frame_h * 0.30 - th / 2.0,
    };
    if let Some([r, g, b, a]) = title.background_color {
        let pad = title.background_padding * scale;
        snapshot.append_color(
            &gdk::RGBA::new(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a as f32 / 255.0),
            &graphene::Rect::new(-tw / 2.0 - pad, y - pad, tw + 2.0 * pad, th + 2.0 * pad),
        );
    }
    let [r, g, b, a] = title.color;
    snapshot.save();
    snapshot.translate(&graphene::Point::new(-tw / 2.0, y));
    snapshot.append_layout(&layout, &gdk::RGBA::new(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a as f32 / 255.0));
    snapshot.restore();
}

glib::wrapper! {
    pub struct VideoSurface(ObjectSubclass<imp::VideoSurface>)
        @extends gtk::Widget, @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for VideoSurface {
    fn default() -> Self {
        Self::new()
    }
}

impl VideoSurface {
    pub fn new() -> Self {
        let obj: Self = glib::Object::builder().build();
        obj.set_hexpand(true);
        obj.set_vexpand(true);
        obj.imp().frame.set((1920, 1080));
        obj.update_property(&[gtk::accessible::Property::Label("Viewer")]);
        obj
    }

    pub fn set_frame_size(&self, width: u32, height: u32) {
        self.imp().frame.set((width.max(1), height.max(1)));
        self.queue_draw();
    }

    pub fn set_layers(&self, layers: Vec<Layer>) {
        let drawn = layers
            .into_iter()
            .map(|l| {
                let texture = l.frame.map(|frame| {
                    let stride = frame.width as usize * 4;
                    let (w, h) = (frame.width as i32, frame.height as i32);
                    let bytes = glib::Bytes::from_owned(frame.data);
                    gdk::MemoryTexture::new(w, h, gdk::MemoryFormat::R8g8b8a8, &bytes, stride).upcast::<gdk::Texture>()
                });
                Drawn { texture, title: l.title, props: l.props }
            })
            .collect();
        *self.imp().layers.borrow_mut() = drawn;
        self.queue_draw();
    }
}
