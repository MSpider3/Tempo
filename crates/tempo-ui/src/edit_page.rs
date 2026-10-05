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
    pub source_slot: gtk::Box,
    vertical: Cell<bool>,
    upper: gtk::Box,
    lower: gtk::Box,
    effects_column: gtk::Box,
    effects: gtk::Box,
    media_open: Cell<bool>,
    effects_open: Cell<bool>,
    tools: [gtk::ToggleButton; 3],
    /// Snapping and linked-selection toggles.
    options: [gtk::ToggleButton; 2],
}

impl EditPage {
    pub fn new(state: &Rc<AppState>, viewer: &Rc<Viewer>) -> Rc<Self> {
        let s = state.clone();
        let media_pool = MediaPool::new(state, move |id| s.show_source(Some(id)));
        let inspector = Inspector::new(state);
        inspector.root.set_visible(false);

        let viewer_slot = gtk::Box::builder().hexpand(true).vexpand(true).build();
        // Holds the source viewer in dual-viewer mode.
        let source_slot = gtk::Box::builder().hexpand(true).vexpand(true).visible(false).css_classes(["tempo-sep-right"]).build();
        let upper = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        upper.append(&source_slot);
        upper.append(&viewer_slot);
        upper.append(&inspector.root);

        let timeline = TimelineArea::new(state, viewer);
        let (toolbar, tools, options) = toolbar(state, &timeline);
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
        let effects = effects_panel(state);
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
            source_slot,
            vertical: Cell::new(false),
            upper,
            lower,
            effects_column,
            effects,
            media_open: Cell::new(true),
            effects_open: Cell::new(false),
            tools,
            options,
        });
        page.layout_left();

        let p = page.clone();
        state.connect(move |change| {
            match change {
                Change::Options => p.sync_options(),
                Change::Project => p.layout_viewer(),
                _ => {}
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

    /// For a project taller than it is wide, give the viewer a full-height column
    /// at the right, as DaVinci Resolve does. Otherwise it sits above the timeline.
    fn layout_viewer(&self) {
        let vertical = self.state.with_project(|p| p.height > p.width).unwrap_or(false);
        if vertical == self.vertical.replace(vertical) {
            return;
        }
        let slot = &self.viewer_slot;
        if let Some(parent) = slot.parent().and_downcast::<gtk::Box>() {
            parent.remove(slot);
        }
        if vertical {
            slot.set_hexpand(false);
            slot.set_width_request(380);
            self.root.append(slot);
            // With the viewer gone from the upper area, the Inspector fills it.
            self.inspector.root.set_hexpand(true);
        } else {
            slot.set_hexpand(true);
            slot.set_width_request(-1);
            self.inspector.root.set_hexpand(false);
            self.upper.insert_child_after(slot, Some(&self.source_slot));
        }
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
        self.options[0].set_active(state.snapping.get());
        self.options[1].set_active(state.linked_selection.get());
    }
}

/// Effects: fades and titles. One click applies an item to the selected clip
/// or adds it at the playhead.
fn effects_panel(state: &Rc<AppState>) -> gtk::Box {
    let panel = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .width_request(300)
        .hexpand(false)
        .vexpand(true)
        .css_classes(["tempo-surface", "tempo-sep-right", "tempo-sep-top"])
        .build();
    let list = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).margin_top(8).margin_bottom(8).margin_start(10).margin_end(10).build();

    let heading = |text: &str, hint: &str| {
        let h = label(text, &["tempo-heading", "tempo-bright"]);
        h.set_xalign(0.0);
        h.set_margin_top(6);
        list.append(&h);
        let sub = label(hint, &["tempo-small", "tempo-dim"]);
        sub.set_xalign(0.0);
        list.append(&sub);
    };
    let item = |name: &str, tip: &str, icon: &str, run: Box<dyn Fn(&Rc<AppState>)>| {
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let cell = gtk::Box::builder().width_request(56).css_classes(["effect-icon"]).build();
        let image = gtk::Image::from_icon_name(icon);
        image.set_hexpand(true);
        image.set_halign(gtk::Align::Center);
        cell.append(&image);
        content.append(&cell);
        let text = label(name, &[]);
        text.set_hexpand(true);
        text.set_xalign(0.0);
        content.append(&text);
        let button = gtk::Button::builder().child(&content).tooltip_text(tip).css_classes(["effect-row"]).build();
        let s = state.clone();
        button.connect_clicked(move |_| run(&s));
        list.append(&button);
    };

    heading("Transitions", "Click to apply to the selected clip");
    item(
        "Cross Dissolve",
        "Dissolve from the previous clip into the selected clip (Ctrl+T)",
        "media-seek-backward-symbolic",
        Box::new(|s| actions::cross_dissolve(s, 0.5)),
    );
    item("Fade In", "Fade the selected clip in over half a second", "go-first-symbolic", Box::new(|s| actions::set_fade(s, true, 0.5)));
    item("Fade Out", "Fade the selected clip out over half a second", "go-last-symbolic", Box::new(|s| actions::set_fade(s, false, 0.5)));
    item(
        "Fade In and Out",
        "Fade the selected clip in and out",
        "media-playback-start-symbolic",
        Box::new(|s| {
            actions::set_fade(s, true, 0.5);
            actions::set_fade(s, false, 0.5);
        }),
    );
    heading("Titles", "Click to add at the playhead");
    item("Text", "A title in the middle of the picture", "document-edit-symbolic", Box::new(|s| actions::add_title(s, false)));
    item("Lower Third", "A name line near the bottom of the picture", "document-properties-symbolic", Box::new(|s| actions::add_title(s, true)));

    heading("Filters", "Click to apply to the selected clip");
    // Filters come from plugins, so this part is rebuilt when plugins change.
    let filters = gtk::Box::new(gtk::Orientation::Vertical, 6);
    list.append(&filters);
    let fill = {
        let state = state.clone();
        let filters = filters.clone();
        move || {
            while let Some(child) = filters.first_child() {
                filters.remove(&child);
            }
            for filter in state.filters.borrow().iter().cloned() {
                let text = label(&filter.name, &[]);
                text.set_xalign(0.0);
                let button = gtk::Button::builder().child(&text).tooltip_text(format!("Apply {} to the selected clip", filter.name)).css_classes(["effect-row"]).build();
                let s = state.clone();
                button.connect_clicked(move |_| actions::add_filter(&s, &filter));
                filters.append(&button);
            }
            if filters.first_child().is_none() {
                let none = label("No filter plugins are switched on", &["tempo-small", "tempo-dim"]);
                none.set_xalign(0.0);
                filters.append(&none);
            }
        }
    };
    fill();
    state.connect(move |change| {
        if matches!(change, Change::Plugins) {
            fill();
        }
    });

    panel.append(&gtk::ScrolledWindow::builder().child(&list).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build());
    panel
}

fn toolbar(state: &Rc<AppState>, timeline: &TimelineArea) -> (gtk::CenterBox, [gtk::ToggleButton; 3], [gtk::ToggleButton; 2]) {
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

    let link = text_toggle("Link", "Linked selection (Ctrl+Shift+L): a clip and its sound are selected and moved together");
    link.set_active(state.linked_selection.get());
    let s = state.clone();
    link.connect_toggled(move |b| {
        if s.linked_selection.replace(b.is_active()) != b.is_active() {
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
        link.upcast_ref(),
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
    (bar, [select, trim, blade], [snap, link])
}
