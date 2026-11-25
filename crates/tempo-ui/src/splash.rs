//! Startup splash showing behind-the-scenes illustrations of classic memes
//! together with DaVinci Resolve beginner tips, decoded lazily and freed on exit.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use gtk4 as gtk;
use gtk4::glib;
use gtk4::graphene;
use gtk4::prelude::*;
use gtk4::subclass::prelude::*;

use crate::util::label;

struct SplashSlide {
    bytes: &'static [u8],
    title: &'static str,
    caption: &'static str,
    tip: &'static str,
}

const SLIDES: &[SplashSlide] = &[
    SplashSlide {
        bytes: include_bytes!("../../../assets/splash/bts_distracted.jpg"),
        title: "BTS: TAKE #47 — DISTRACTED LOOKBACK",
        caption: "Director yelled cut because the editor was checking out another timeline.",
        tip: "Resolve Tip: Press Ctrl+\\ to split the clip under the playhead without switching tools.",
    },
    SplashSlide {
        bytes: include_bytes!("../../../assets/splash/bts_this_is_fine.jpg"),
        title: "BTS: SET #2 — THIS IS FINE",
        caption: "Fake LED flames, real smoke machine — and Tempo autosaves so your timeline stays fine.",
        tip: "Resolve Tip: Press Shift+Backspace or Delete to remove a clip and close the gap.",
    },
    SplashSlide {
        bytes: include_bytes!("../../../assets/splash/bts_stonks_edit.jpg"),
        title: "BTS: GREEN SCREEN — STONKS",
        caption: "Keying the chroma backdrop in the edit bay while staying well under 2 GB RAM.",
        tip: "Resolve Tip: Tap L repeatedly to shuttle up to 64× speed; press K to stop.",
    },
    SplashSlide {
        bytes: include_bytes!("../../../assets/splash/bts_cat_dinner.jpg"),
        title: "BTS: STAGE B — DINNER SCENE",
        caption: "Waiting for the talent to hit his mark before dropping a Dip To Color Dissolve.",
        tip: "Resolve Tip: Press V to grab the nearest cut, then Ctrl+T to add a transition.",
    },
];

const SLIDE_MS: f64 = 4000.0;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct SplashCanvas {
        pub texture: RefCell<Option<gtk::gdk::Texture>>,
        pub elapsed_ms: Cell<f64>,
        pub slide_index: Cell<usize>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SplashCanvas {
        const NAME: &'static str = "TempoSplashCanvas";
        type Type = super::SplashCanvas;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for SplashCanvas {}

    impl WidgetImpl for SplashCanvas {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let widget = self.obj();
            let (w, h) = (widget.width() as f32, widget.height() as f32);
            if w <= 0.0 || h <= 0.0 {
                return;
            }
            let bounds = graphene::Rect::new(0.0, 0.0, w, h);
            snapshot.push_clip(&bounds);
            if let Some(tex) = self.texture.borrow().as_ref() {
                let (tw, th) = (tex.width() as f32, tex.height() as f32);
                if tw > 0.0 && th > 0.0 {
                    let t = (self.elapsed_ms.get() / SLIDE_MS).clamp(0.0, 1.0) as f32;
                    let (x, y, dw, dh) = super::ken_burns_rect(w, h, tw, th, t, self.slide_index.get());
                    snapshot.append_texture(tex, &graphene::Rect::new(x, y, dw, dh));
                }
            }
            snapshot.pop();
        }
    }
}

glib::wrapper! {
    pub struct SplashCanvas(ObjectSubclass<imp::SplashCanvas>)
        @extends gtk::Widget;
}

impl SplashCanvas {
    fn new() -> Self {
        let obj: Self = glib::Object::builder().build();
        obj.set_hexpand(true);
        obj.set_vexpand(true);
        obj
    }

    fn set_frame(&self, tex: Option<gtk::gdk::Texture>, slide_index: usize, elapsed_ms: f64) {
        let imp = self.imp();
        *imp.texture.borrow_mut() = tex;
        imp.slide_index.set(slide_index);
        imp.elapsed_ms.set(elapsed_ms);
        self.queue_draw();
    }

    fn clear(&self) {
        *self.imp().texture.borrow_mut() = None;
    }
}

/// Cover-scaled Ken Burns rectangle guaranteed to fill `(0, 0, w, h)` with no gaps.
fn ken_burns_rect(w: f32, h: f32, tw: f32, th: f32, t: f32, slide_index: usize) -> (f32, f32, f32, f32) {
    let t = t.clamp(0.0, 1.0);
    let base = (w / tw).max(h / th);
    let zoom = 1.0 + 0.07 * t;
    let (dw, dh) = (tw * base * zoom, th * base * zoom);
    let slack_x = (dw - w).max(0.0);
    let slack_y = (dh - h).max(0.0);
    let pan_x = if slide_index.is_multiple_of(2) {
        0.35 + 0.30 * t
    } else {
        0.65 - 0.30 * t
    };
    let pan_y = 0.45 + 0.10 * t;
    (-slack_x * pan_x, -slack_y * pan_y, dw, dh)
}

pub struct SplashScreen {
    pub root: gtk::Overlay,
    canvas: SplashCanvas,
    counter: gtk::Label,
    slide_title: gtk::Label,
    caption: gtk::Label,
    tip: gtk::Label,
    progress: gtk::ProgressBar,
    status: gtk::Label,
    skip_btn: gtk::Button,
    textures: RefCell<Vec<Option<gtk::gdk::Texture>>>,
    index: Cell<usize>,
    started: RefCell<Option<Instant>>,
    active: Cell<bool>,
    timer_running: Cell<bool>,
}

impl SplashScreen {
    pub fn new(on_skip: impl Fn() + 'static) -> Rc<Self> {
        let on_skip: Rc<dyn Fn()> = Rc::new(on_skip);
        let canvas = SplashCanvas::new();

        let brand = label("TEMPO", &["splash-brand"]);
        brand.set_xalign(0.0);
        let badge = label("BEHIND THE SCENES", &["splash-badge"]);
        badge.set_valign(gtk::Align::Center);
        let counter = label("1 / 4", &["splash-counter"]);
        counter.set_valign(gtk::Align::Center);
        let top_row = gtk::Box::builder().spacing(10).halign(gtk::Align::Start).build();
        top_row.append(&badge);
        top_row.append(&counter);

        let top_box = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).halign(gtk::Align::Start).build();
        top_box.append(&brand);
        top_box.append(&top_row);

        let slide_title = label("", &["splash-slide-title"]);
        slide_title.set_xalign(0.0);
        slide_title.set_wrap(true);
        let caption = label("", &["splash-caption"]);
        caption.set_xalign(0.0);
        caption.set_wrap(true);
        caption.set_max_width_chars(56);
        let tip = label("", &["splash-tip"]);
        tip.set_xalign(0.0);
        tip.set_wrap(true);
        tip.set_max_width_chars(60);

        let progress = gtk::ProgressBar::builder().width_request(340).halign(gtk::Align::Start).css_classes(["splash-progress"]).build();
        let status = label("STARTING MEDIA ENGINE", &["splash-status"]);
        status.set_xalign(0.0);

        let bottom_left = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).hexpand(true).valign(gtk::Align::End).build();
        bottom_left.append(&slide_title);
        bottom_left.append(&caption);
        bottom_left.append(&tip);
        bottom_left.append(&progress);
        bottom_left.append(&status);

        let skip_btn = gtk::Button::builder()
            .label("Skip")
            .tooltip_text("Skip loading screen (click or press any key)")
            .css_classes(["pill-outline", "splash-skip"])
            .build();
        skip_btn.update_property(&[gtk::accessible::Property::Label("Skip loading screen")]);
        let skip_cb = on_skip.clone();
        skip_btn.connect_clicked(move |_| skip_cb());

        let skip_hint = label("Click or press any key to skip", &["splash-skip-hint"]);
        let bottom_right = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).halign(gtk::Align::End).valign(gtk::Align::End).build();
        bottom_right.append(&skip_btn);
        bottom_right.append(&skip_hint);

        let bottom_row = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(24).valign(gtk::Align::End).build();
        bottom_row.append(&bottom_left);
        bottom_row.append(&bottom_right);

        let spacer = gtk::Box::builder().vexpand(true).build();
        let scrim = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .hexpand(true)
            .vexpand(true)
            .css_classes(["splash-scrim"])
            .build();
        scrim.append(&top_box);
        scrim.append(&spacer);
        scrim.append(&bottom_row);

        let root = gtk::Overlay::builder().css_classes(["loading", "splash"]).build();
        root.set_child(Some(&canvas));
        root.add_overlay(&scrim);

        let click = gtk::GestureClick::builder().button(gtk::gdk::BUTTON_PRIMARY).build();
        let skip_cb = on_skip;
        click.connect_released(move |_, _, _, _| skip_cb());
        root.add_controller(click);

        let initial = if std::env::var_os("TEMPO_SHOW_SPLASH").is_some() {
            0
        } else {
            SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| (d.as_millis() as usize) % SLIDES.len())
        };

        let splash = Rc::new(Self {
            root,
            canvas,
            counter,
            slide_title,
            caption,
            tip,
            progress,
            status,
            skip_btn,
            textures: RefCell::new(vec![None; SLIDES.len()]),
            index: Cell::new(initial),
            started: RefCell::new(None),
            active: Cell::new(false),
            timer_running: Cell::new(false),
        });
        splash.apply_slide(initial, 0.0, false);
        splash
    }

    pub fn activate(self: &Rc<Self>) {
        self.active.set(true);
        *self.started.borrow_mut() = Some(Instant::now());
        let idx = self.index.get() % SLIDES.len();
        self.apply_slide(idx, 0.0, true);
        self.skip_btn.grab_focus();
        if self.timer_running.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
        glib::timeout_add_local(Duration::from_millis(50), move || {
            let Some(sp) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if !sp.active.get() {
                sp.timer_running.set(false);
                return glib::ControlFlow::Break;
            }
            if !sp.root.is_mapped() {
                return glib::ControlFlow::Continue;
            }
            let elapsed_ms = sp.started.borrow().map_or(0.0, |t| t.elapsed().as_secs_f64() * 1000.0);
            if elapsed_ms >= SLIDE_MS {
                let next = (sp.index.get() + 1) % SLIDES.len();
                *sp.started.borrow_mut() = Some(Instant::now());
                sp.apply_slide(next, 0.0, true);
            } else {
                let idx = sp.index.get();
                let tex = sp.ensure_texture(idx);
                sp.canvas.set_frame(tex, idx, elapsed_ms);
            }
            glib::ControlFlow::Continue
        });
    }

    /// Release decoded textures immediately once the user leaves the splash so 0 MB stays in RAM.
    pub fn release(&self) {
        self.active.set(false);
        self.timer_running.set(false);
        self.canvas.clear();
        for slot in self.textures.borrow_mut().iter_mut() {
            *slot = None;
        }
    }

    pub fn set_status(&self, text: &str) {
        self.status.set_text(text);
    }

    pub fn set_progress(&self, fraction: f64) {
        self.progress.set_fraction(fraction.clamp(0.0, 1.0));
    }

    fn ensure_texture(&self, idx: usize) -> Option<gtk::gdk::Texture> {
        let idx = idx % SLIDES.len();
        let mut cache = self.textures.borrow_mut();
        if cache.len() != SLIDES.len() {
            *cache = vec![None; SLIDES.len()];
        }
        if let Some(existing) = &cache[idx] {
            return Some(existing.clone());
        }
        let bytes = glib::Bytes::from_static(SLIDES[idx].bytes);
        let decoded = gtk::gdk::Texture::from_bytes(&bytes).ok();
        cache[idx] = decoded.clone();
        decoded
    }

    fn apply_slide(&self, idx: usize, elapsed_ms: f64, decode: bool) {
        let idx = idx % SLIDES.len();
        self.index.set(idx);
        let slide = &SLIDES[idx];
        self.counter.set_text(&format!("{} / {}", idx + 1, SLIDES.len()));
        self.slide_title.set_text(slide.title);
        self.caption.set_text(slide.caption);
        self.tip.set_text(slide.tip);
        let tex = if decode { self.ensure_texture(idx) } else { None };
        self.canvas.set_frame(tex, idx, elapsed_ms);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_bts_slides_are_valid_jpegs_with_captions() {
        assert_eq!(SLIDES.len(), 4);
        for slide in SLIDES {
            assert!(slide.bytes.len() > 20_000, "{} should be non-trivial JPEG", slide.title);
            assert_eq!(&slide.bytes[..2], &[0xFF, 0xD8], "{} must start with JPEG SOI", slide.title);
            assert!(!slide.title.is_empty());
            assert!(!slide.caption.is_empty());
            assert!(!slide.tip.is_empty());
        }
    }

    #[test]
    fn ken_burns_rect_always_covers_viewport() {
        for (w, h) in [(1366.0, 768.0), (1920.0, 1080.0), (1024.0, 640.0), (800.0, 900.0)] {
            for idx in 0..4 {
                for t in [0.0, 0.5, 1.0] {
                    let (x, y, dw, dh) = ken_burns_rect(w, h, 1280.0, 720.0, t, idx);
                    assert!(x <= 0.01, "left edge covered: x={x} w={w}");
                    assert!(y <= 0.01, "top edge covered: y={y} h={h}");
                    assert!(x + dw >= w - 0.1, "right edge covered: {} >= {w}", x + dw);
                    assert!(y + dh >= h - 0.1, "bottom edge covered: {} >= {h}", y + dh);
                }
            }
        }
    }
}

