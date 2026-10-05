//! Inspector: settings of the selected clip. Rows follow Resolve's layout:
//! label right-aligned, value fields to its right, reset per section.

use std::cell::Cell;
use std::rc::Rc;

use gtk4 as gtk;
use gtk4::prelude::*;
use tempo_timeline::{Clip, ClipProperties, ClipType, PropertyChange, SetClipPropertyCommand};

use crate::state::{AppState, Change};
use crate::util::{label, tool_button};

pub struct Inspector {
    pub root: gtk::Box,
    state: Rc<AppState>,
    title: gtk::Label,
    body: gtk::Box,
    /// Set while the widgets are being filled in, so that does not count as an edit.
    loading: Rc<Cell<bool>>,
}

impl Inspector {
    pub fn new(state: &Rc<AppState>) -> Rc<Self> {
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .width_request(300)
            .css_classes(["inspector", "tempo-sep-left"])
            .build();
        let title = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .css_classes(["inspector-title"])
            .build();
        root.append(&title);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.append(&gtk::ScrolledWindow::builder().child(&body).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build());

        let inspector = Rc::new(Self { root, state: state.clone(), title, body, loading: Rc::new(Cell::new(false)) });
        let i = inspector.clone();
        state.connect(move |change| {
            if matches!(change, Change::Selection | Change::Timeline | Change::Project) {
                i.rebuild();
            }
        });
        inspector.rebuild();
        inspector
    }

    fn rebuild(&self) {
        // Rebuilding while a field has focus would steal the caret mid-edit.
        if self.loading.get() {
            return;
        }
        while let Some(child) = self.body.first_child() {
            self.body.remove(&child);
        }
        let Some(clip) = self.state.selected_clip() else {
            self.title.set_text("Inspector");
            let hint = label("Select a clip to see its settings", &["empty-hint"]);
            hint.set_margin_top(32);
            self.body.append(&hint);
            return;
        };
        self.title.set_text(&clip.name);
        self.loading.set(true);
        match clip.clip_type {
            ClipType::Audio => self.audio_section(&clip),
            _ => {
                self.transform_section(&clip);
                self.composite_section(&clip);
            }
        }
        self.loading.set(false);
    }

    /// Section header with a reset button; returns the grid the rows go into.
    fn section(&self, name: &str, clip: &Clip, reset: Vec<PropertyChange>) -> gtk::Grid {
        let header = gtk::Box::builder().css_classes(["inspector-section"]).build();
        let title = label(name, &["tempo-heading"]);
        title.set_hexpand(true);
        title.set_xalign(0.0);
        header.append(&title);
        let reset_btn = tool_button("edit-undo-symbolic", &format!("Reset {name}"));
        let state = self.state.clone();
        let id = clip.id;
        let reset = std::cell::RefCell::new(Some(reset));
        reset_btn.connect_clicked(move |_| {
            // Each reset value is applied once; the rebuild that follows makes a fresh button.
            if let Some(changes) = reset.borrow_mut().take() {
                for change in changes {
                    state.execute(Box::new(SetClipPropertyCommand::new(id, change)));
                }
            }
        });
        header.append(&reset_btn);
        self.body.append(&header);

        let grid = gtk::Grid::builder().row_spacing(8).column_spacing(8).margin_top(10).margin_bottom(12).margin_start(12).margin_end(12).build();
        self.body.append(&grid);
        grid
    }

    /// One number row. `apply` turns the new value into a property change.
    #[allow(clippy::too_many_arguments)]
    fn number_row(
        &self,
        grid: &gtk::Grid,
        row: i32,
        name: &str,
        value: f64,
        (min, max, step, digits): (f64, f64, f64, u32),
        clip: &Clip,
        apply: impl Fn(f64, &ClipProperties) -> PropertyChange + 'static,
    ) {
        let name_label = label(name, &["tempo-dim"]);
        name_label.set_xalign(1.0);
        name_label.set_width_chars(10);
        grid.attach(&name_label, 0, row, 1, 1);

        let spin = gtk::SpinButton::builder()
            .adjustment(&gtk::Adjustment::new(value, min, max, step, step * 10.0, 0.0))
            .digits(digits)
            .numeric(true)
            .hexpand(true)
            .build();
        spin.update_property(&[gtk::accessible::Property::Label(name)]);
        let state = self.state.clone();
        let loading = self.loading.clone();
        let id = clip.id;
        spin.connect_value_changed(move |s| {
            if loading.get() {
                return;
            }
            let Some(current) = state.selected_clip().filter(|c| c.id == id) else { return };
            // Keep the Inspector as it is while this edit is applied, so the field keeps focus.
            loading.set(true);
            state.execute(Box::new(SetClipPropertyCommand::new(id, apply(s.value(), &current.properties))));
            loading.set(false);
        });
        grid.attach(&spin, 1, row, 1, 1);
    }

    fn transform_section(&self, clip: &Clip) {
        let p = &clip.properties;
        let grid = self.section(
            "Transform",
            clip,
            vec![
                PropertyChange::Scale { x: 1.0, y: 1.0 },
                PropertyChange::Position { x: 0.0, y: 0.0 },
                PropertyChange::Rotation(0.0),
            ],
        );
        self.number_row(&grid, 0, "Zoom", p.scale_x as f64, (0.1, 10.0, 0.05, 2), clip, |v, _| PropertyChange::Scale {
            x: v as f32,
            y: v as f32,
        });
        self.number_row(&grid, 1, "Position X", p.position_x as f64, (-8000.0, 8000.0, 10.0, 0), clip, |v, cur| {
            PropertyChange::Position { x: v as f32, y: cur.position_y }
        });
        self.number_row(&grid, 2, "Position Y", p.position_y as f64, (-8000.0, 8000.0, 10.0, 0), clip, |v, cur| {
            PropertyChange::Position { x: cur.position_x, y: v as f32 }
        });
        self.number_row(&grid, 3, "Rotation", p.rotation as f64, (-360.0, 360.0, 1.0, 1), clip, |v, _| {
            PropertyChange::Rotation(v as f32)
        });
    }

    fn composite_section(&self, clip: &Clip) {
        let grid = self.section("Composite", clip, vec![PropertyChange::Opacity(1.0)]);
        self.number_row(&grid, 0, "Opacity", clip.properties.opacity as f64 * 100.0, (0.0, 100.0, 5.0, 0), clip, |v, _| {
            PropertyChange::Opacity(v as f32 / 100.0)
        });
    }

    fn audio_section(&self, clip: &Clip) {
        let p = &clip.properties;
        let grid = self.section("Audio", clip, vec![PropertyChange::Volume(1.0), PropertyChange::Pan(0.0)]);
        self.number_row(&grid, 0, "Volume %", p.volume as f64 * 100.0, (0.0, 200.0, 5.0, 0), clip, |v, _| {
            PropertyChange::Volume(v as f32 / 100.0)
        });
        self.number_row(&grid, 1, "Pan", p.pan as f64 * 100.0, (-100.0, 100.0, 10.0, 0), clip, |v, _| {
            PropertyChange::Pan(v as f32 / 100.0)
        });
    }
}
