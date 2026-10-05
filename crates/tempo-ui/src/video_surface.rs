//! The picture area. Draws the decoded layers with GTK's own renderer, so
//! scaling, placement and opacity are done by the GPU without extra copies.

use std::cell::{Cell, RefCell};

use glib::subclass::prelude::*;
use gtk4 as gtk;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;
use gtk4::{gdk, graphene};
use tempo_timeline::ClipProperties;

use crate::player::Layer;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct VideoSurface {
        pub layers: RefCell<Vec<(gdk::Texture, ClipProperties)>>,
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
            for (texture, props) in self.layers.borrow().iter() {
                let (tw, th) = (texture.width() as f32, texture.height() as f32);
                if tw <= 0.0 || th <= 0.0 {
                    continue;
                }
                let fit = (fw / tw).min(fh / th);
                let (dw, dh) = (tw * fit, th * fit);
                snapshot.save();
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
                snapshot.append_texture(texture, &graphene::Rect::new(-dw / 2.0, -dh / 2.0, dw, dh));
                if !opaque {
                    snapshot.pop();
                }
                snapshot.restore();
            }
            snapshot.pop();
        }
    }
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
        let textures = layers
            .into_iter()
            .map(|l| {
                let stride = l.frame.width as usize * 4;
                let bytes = glib::Bytes::from_owned(l.frame.data);
                let tex = gdk::MemoryTexture::new(
                    l.frame.width as i32,
                    l.frame.height as i32,
                    gdk::MemoryFormat::R8g8b8a8,
                    &bytes,
                    stride,
                );
                (tex.upcast::<gdk::Texture>(), l.props)
            })
            .collect();
        *self.imp().layers.borrow_mut() = textures;
        self.queue_draw();
    }
}
