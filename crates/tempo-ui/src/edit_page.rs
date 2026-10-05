//! Edit page: Media Pool, viewer, Inspector, toolbar and timeline, arranged as
//! on DaVinci Resolve's Edit page.

use std::cell::Cell;
use std::rc::Rc;

use gtk4 as gtk;
use gtk4::prelude::*;

use crate::actions::{self, Place};
use crate::inspector::Inspector;
use crate::media_pool::MediaPool;
use crate::state::{AppState, Change, Tool};
use crate::timeline::{TimelineArea, MAX_PPS, MIN_PPS};
use crate::util::{divider, label, text_button, text_toggle, tool_button};
use crate::viewer::Viewer;

pub struct EditPage {
    pub root: gtk::Box,
    state: Rc<AppState>,
    pub media_pool: Rc<MediaPool>,
    pub inspector: Rc<Inspector>,
    pub timeline: TimelineArea,
    /// Holds the viewer while this page is showing.
    pub viewer_slot: gtk::Box,
    upper: gtk::Box,
    lower: gtk::Box,
    effects_column: gtk::Box,
    effects: gtk::Box,
    media_open: Cell<bool>,
    effects_open: Cell<bool>,
    tools: [gtk::ToggleButton; 3],
    snap: gtk::ToggleButton,
}

impl EditPage {
    pub fn new(state: &Rc<AppState>, viewer: &Rc<Viewer>) -> Rc<Self> {
        let v = viewer.clone();
        let media_pool = MediaPool::new(state, move |id| v.show_source(Some(id)));
        let inspector = Inspector::new(state);
        inspector.root.set_visible(false);

        let viewer_slot = gtk::Box::builder().hexpand(true).vexpand(true).build();
        let upper = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        upper.append(&viewer_slot);
        upper.append(&inspector.root);

        let timeline = TimelineArea::new(state, viewer);
        let (toolbar, tools, snap) = toolbar(state, &timeline);
        let lower = gtk::Box::new(gtk::Orientation::Vertical, 0);
        lower.append(&toolbar);
        lower.append(&timeline.root);

        let split = gtk::Paned::builder()
            .orientation(gtk::Orientation::Vertical)
            .start_child(&upper)
            .end_child(&lower)
            .resize_start_child(true)
            .resize_end_child(true)
            .shrink_start_child(false)
            .shrink_end_child(false)
            .hexpand(true)
            .build();
        // Give the viewer a little over half of the height once the size is known.
        let placed = Cell::new(false);
        split.connect_notify_local(Some("max-position"), move |p, _| {
            if p.max_position() > 200 && !placed.replace(true) {
                p.set_position((p.max_position() as f32 * 0.54) as i32);
            }
        });

        // Effects, when open, sits under the Media Pool in a full-height column.
        let effects = effects_panel();
        let effects_column = gtk::Box::builder().orientation(gtk::Orientation::Vertical).visible(false).build();

        let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        root.append(&effects_column);
        root.append(&split);

        let page = Rc::new(Self {
            root,
            state: state.clone(),
            media_pool,
            inspector,
            timeline,
            viewer_slot,
            upper,
            lower,
            effects_column,
            effects,
            media_open: Cell::new(true),
            effects_open: Cell::new(false),
            tools,
            snap,
        });
        page.layout_left();

        let p = page.clone();
        state.connect(move |change| {
            if matches!(change, Change::Options) {
                p.sync_options();
            }
        });
        page
    }

    pub fn set_media_pool_open(&self, open: bool) {
        self.media_open.set(open);
        self.layout_left();
    }

    pub fn set_effects_open(&self, open: bool) {
        self.effects_open.set(open);
        self.layout_left();
    }

    pub fn set_inspector_open(&self, open: bool) {
        self.inspector.root.set_visible(open);
    }

    /// Put the Media Pool beside the viewer, or above Effects in the left column.
    fn layout_left(&self) {
        let pool = &self.media_pool.root;
        if let Some(parent) = pool.parent().and_downcast::<gtk::Box>() {
            parent.remove(pool);
        }
        if let Some(parent) = self.effects.parent().and_downcast::<gtk::Box>() {
            parent.remove(&self.effects);
        }
        let (media, effects) = (self.media_open.get(), self.effects_open.get());
        if effects {
            if media {
                pool.set_vexpand(true);
                self.effects_column.append(pool);
            }
            self.effects_column.append(&self.effects);
        } else if media {
            pool.set_vexpand(true);
            self.upper.prepend(pool);
        }
        self.effects_column.set_visible(effects);
    }

    /// Hide everything except the viewer (cinema viewer), or bring it back.
    pub fn set_cinema(&self, on: bool) {
        self.lower.set_visible(!on);
        self.media_pool.root.set_visible(!on);
        self.effects_column.set_visible(!on && self.effects_open.get());
        if on {
            self.inspector.root.set_visible(false);
        }
    }

    fn sync_options(&self) {
        let state = &self.state;
        let active = match state.tool.get() {
            Tool::Select => 0,
            Tool::Trim => 1,
            Tool::Blade => 2,
        };
        self.tools[active].set_active(true);
        self.snap.set_active(state.snapping.get());
    }
}

fn effects_panel() -> gtk::Box {
    let panel = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .width_request(300)
        .vexpand(true)
        .css_classes(["tempo-surface", "tempo-sep-right", "tempo-sep-top"])
        .build();
    let title = label("Effects", &["tempo-heading", "tempo-bright"]);
    title.set_xalign(0.0);
    title.set_margin_top(8);
    title.set_margin_start(10);
    panel.append(&title);
    let hint = label("No transitions, titles or filters are installed yet.", &["empty-hint"]);
    hint.set_wrap(true);
    hint.set_margin_top(24);
    hint.set_margin_start(16);
    hint.set_margin_end(16);
    panel.append(&hint);
    panel
}

fn toolbar(state: &Rc<AppState>, timeline: &TimelineArea) -> (gtk::CenterBox, [gtk::ToggleButton; 3], gtk::ToggleButton) {
    // Edit modes: exactly one is active.
    let select = text_toggle("Select", "Selection mode (A): click, move and trim clips");
    let trim = text_toggle("Trim", "Trim mode (T): drag clip edges");
    let blade = text_toggle("Blade", "Blade mode (B): click a clip to cut it");
    for b in [&select, &trim, &blade] {
        b.add_css_class("accent");
    }
    trim.set_group(Some(&select));
    blade.set_group(Some(&select));
    select.set_active(true);
    for (button, tool) in [(&select, Tool::Select), (&trim, Tool::Trim), (&blade, Tool::Blade)] {
        let s = state.clone();
        button.connect_toggled(move |b| {
            if b.is_active() && s.tool.replace(tool) != tool {
                s.emit(Change::Options);
            }
        });
    }

    let insert = text_button("Insert", "Insert the selected Media Pool clip at the playhead, pushing later clips right (F9)");
    let s = state.clone();
    insert.connect_clicked(move |_| actions::place_current(&s, Place::Insert));
    let overwrite = text_button("Overwrite", "Place the selected Media Pool clip at the playhead, replacing what is there (F10)");
    let s = state.clone();
    overwrite.connect_clicked(move |_| actions::place_current(&s, Place::Overwrite));

    let snap = text_toggle("Snap", "Snapping (N): clips stick to each other, markers and the playhead");
    snap.set_active(state.snapping.get());
    let s = state.clone();
    snap.connect_toggled(move |b| {
        if s.snapping.replace(b.is_active()) != b.is_active() {
            s.emit(Change::Options);
        }
    });

    let marker = text_button("Marker", "Add a marker at the playhead (M)");
    let s = state.clone();
    marker.connect_clicked(move |_| {
        actions::add_marker(&s);
    });

    // Zoom: fit, minus, slider (logarithmic), plus.
    let canvas = timeline.canvas.clone();
    let fit = tool_button("zoom-fit-best-symbolic", "Zoom to fit (Shift+Z)");
    let c = canvas.clone();
    fit.connect_clicked(move |_| c.zoom_fit());
    let minus = tool_button("zoom-out-symbolic", "Zoom out (Ctrl+-)");
    let c = canvas.clone();
    minus.connect_clicked(move |_| c.set_pps(c.pps() / 1.4, None));
    let plus = tool_button("zoom-in-symbolic", "Zoom in (Ctrl+=)");
    let c = canvas.clone();
    plus.connect_clicked(move |_| c.set_pps(c.pps() * 1.4, None));

    let to_slider = |pps: f64| (pps / MIN_PPS).ln() / (MAX_PPS / MIN_PPS).ln();
    let zoom = gtk::Scale::builder()
        .adjustment(&gtk::Adjustment::new(to_slider(canvas.pps()), 0.0, 1.0, 0.02, 0.1, 0.0))
        .draw_value(false)
        .width_request(120)
        .tooltip_text("Timeline zoom")
        .build();
    let c = canvas.clone();
    zoom.connect_change_value(move |_, _, v| {
        c.set_pps(MIN_PPS * (MAX_PPS / MIN_PPS).powf(v.clamp(0.0, 1.0)), None);
        glib::Propagation::Proceed
    });
    let z = zoom.clone();
    canvas.connect_zoom(move |pps| z.set_value(to_slider(pps)));

    let centre = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    for w in [
        select.upcast_ref::<gtk::Widget>(),
        trim.upcast_ref(),
        blade.upcast_ref(),
        divider().upcast_ref(),
        insert.upcast_ref(),
        overwrite.upcast_ref(),
        divider().upcast_ref(),
        snap.upcast_ref(),
        divider().upcast_ref(),
        marker.upcast_ref(),
        divider().upcast_ref(),
        fit.upcast_ref(),
        minus.upcast_ref(),
        zoom.upcast_ref(),
        plus.upcast_ref(),
    ] {
        centre.append(w);
    }

    let bar = gtk::CenterBox::builder().css_classes(["timeline-toolbar"]).build();
    bar.set_center_widget(Some(&centre));
    (bar, [select, trim, blade], snap)
}
