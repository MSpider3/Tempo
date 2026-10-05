//! Inspector: settings of the selected clip. Rows follow Resolve's layout:
//! label right-aligned, value fields to its right, reset per section.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4 as gtk;
use gtk4::prelude::*;
use tempo_timeline::{Clip, ClipType, Command, EditClipCommand, PropertyChange, SetClipPropertyCommand, TitleType};

use crate::state::{AppState, Change};
use crate::util::{label, tool_button};

/// Turns a new field value and the clip as it is now into the command that applies it.
type MakeCommand = Box<dyn Fn(f64, &Clip) -> Box<dyn Command>>;

pub struct Inspector {
    pub root: gtk::Box,
    state: Rc<AppState>,
    title: gtk::Label,
    body: gtk::Box,
    /// Set while an edit from the Inspector itself is applied, so the fields are
    /// not rebuilt under the user's hands.
    busy: Rc<Cell<bool>>,
}

fn property(make: impl Fn(f64, &Clip) -> PropertyChange + 'static) -> MakeCommand {
    Box::new(move |v, clip| Box::new(SetClipPropertyCommand::new(clip.id, make(v, clip))))
}

fn edit(name: &'static str, change: impl Fn(f64, &mut Clip) + 'static) -> MakeCommand {
    Box::new(move |v, clip| {
        let mut edited = clip.clone();
        change(v, &mut edited);
        Box::new(EditClipCommand::new(name, edited))
    })
}

impl Inspector {
    pub fn new(state: &Rc<AppState>) -> Rc<Self> {
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .width_request(300)
            .hexpand(false)
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

        let inspector = Rc::new(Self { root, state: state.clone(), title, body, busy: Rc::new(Cell::new(false)) });
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
        if self.busy.get() {
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
        let count = self.state.selected_ids().len();
        self.title.set_text(&if count > 1 { format!("{} (+{} more)", clip.name, count - 1) } else { clip.name.clone() });
        match clip.clip_type {
            ClipType::Audio => self.audio_section(&clip),
            ClipType::Title => {
                self.title_section(&clip);
                self.transform_section(&clip);
                self.composite_section(&clip);
            }
            _ => {
                self.transform_section(&clip);
                self.composite_section(&clip);
            }
        }
        if !matches!(clip.clip_type, ClipType::Audio | ClipType::Title) {
            self.filter_sections(&clip);
        }
        self.fade_section(&clip);
    }

    /// Run a command for the clip without the Inspector rebuilding itself.
    fn apply(state: &Rc<AppState>, busy: &Cell<bool>, id: uuid::Uuid, make: impl FnOnce(&Clip) -> Box<dyn Command>) {
        let Some(current) = state.selected_clip().filter(|c| c.id == id) else { return };
        busy.set(true);
        state.execute(make(&current));
        busy.set(false);
    }

    /// Section header with a reset button; returns the grid the rows go into.
    fn section(&self, name: &str, clip: &Clip, reset: Vec<(f64, MakeCommand)>) -> gtk::Grid {
        let header = gtk::Box::builder().css_classes(["inspector-section"]).build();
        let title = label(name, &["tempo-heading"]);
        title.set_hexpand(true);
        title.set_xalign(0.0);
        header.append(&title);
        if !reset.is_empty() {
            let reset_btn = tool_button("edit-undo-symbolic", &format!("Reset {name}"));
            let state = self.state.clone();
            let id = clip.id;
            let reset = RefCell::new(Some(reset));
            reset_btn.connect_clicked(move |_| {
                // The rebuild after the first command replaces this button, so take the list once.
                for (value, make) in reset.borrow_mut().take().into_iter().flatten() {
                    if let Some(current) = state.selected_clip().filter(|c| c.id == id) {
                        state.execute(make(value, &current));
                    }
                }
            });
            header.append(&reset_btn);
        }
        self.body.append(&header);

        let grid = gtk::Grid::builder().row_spacing(8).column_spacing(8).margin_top(10).margin_bottom(12).margin_start(12).margin_end(12).build();
        self.body.append(&grid);
        grid
    }

    fn row_label(grid: &gtk::Grid, row: i32, name: &str) -> gtk::Label {
        let name_label = label(name, &["tempo-dim"]);
        name_label.set_xalign(1.0);
        name_label.set_width_chars(10);
        grid.attach(&name_label, 0, row, 1, 1);
        name_label
    }

    /// One number row. Typing or stepping the field is one undo step. Dragging
    /// left or right on the row's name changes the value live, and the whole
    /// drag is one undo step.
    fn number_row(&self, grid: &gtk::Grid, row: i32, name: &str, value: f64, (min, max, step, digits): (f64, f64, f64, u32), clip: &Clip, make: MakeCommand) {
        let name_label = Self::row_label(grid, row, name);
        name_label.set_cursor_from_name(Some("ew-resize"));
        name_label.set_tooltip_text(Some("Drag left or right to change"));
        let make = Rc::new(make);
        let dragging = Rc::new(Cell::new(false));
        let spin = gtk::SpinButton::builder()
            .adjustment(&gtk::Adjustment::new(value, min, max, step, step * 10.0, 0.0))
            .digits(digits)
            .numeric(true)
            .hexpand(true)
            .build();
        spin.update_property(&[gtk::accessible::Property::Label(name)]);
        let state = self.state.clone();
        let busy = self.busy.clone();
        let id = clip.id;
        let (m, d) = (make.clone(), dragging.clone());
        spin.connect_value_changed(move |s| {
            // During a drag the value is previewed below, not recorded step by step.
            if !d.get() {
                Self::apply(&state, &busy, id, |current| m(s.value(), current));
            }
        });
        grid.attach(&spin, 1, row, 1, 1);

        let drag = gtk::GestureDrag::new();
        let start: Rc<RefCell<Option<(f64, Clip)>>> = Rc::new(RefCell::new(None));
        let (state, busy, sp, st, dr) = (self.state.clone(), self.busy.clone(), spin.clone(), start.clone(), dragging.clone());
        drag.connect_drag_begin(move |_, _, _| {
            if let Some(original) = state.selected_clip().filter(|c| c.id == id) {
                *st.borrow_mut() = Some((sp.value(), original));
                dr.set(true);
                busy.set(true);
            }
        });
        let (state, sp, st, m) = (self.state.clone(), spin.clone(), start.clone(), make.clone());
        drag.connect_drag_update(move |_, dx, _| {
            let Some((from, _)) = st.borrow().as_ref().map(|(v, c)| (*v, c.id)) else { return };
            // Four pixels per step: fine enough to aim, quick enough to travel.
            let value = (from + (dx / 4.0).round() * step).clamp(min, max);
            if (value - sp.value()).abs() < f64::EPSILON {
                return;
            }
            sp.set_value(value);
            if let Some(current) = state.selected_clip().filter(|c| c.id == id) {
                state.apply_unlogged(m(value, &current));
            }
        });
        let (state, busy, name) = (self.state.clone(), self.busy.clone(), name.to_string());
        drag.connect_drag_end(move |_, _, _| {
            if let Some((_, original)) = start.borrow_mut().take() {
                state.commit_preview(&name, original);
            }
            dragging.set(false);
            busy.set(false);
        });
        name_label.add_controller(drag);
    }

    fn transform_section(&self, clip: &Clip) {
        let p = &clip.properties;
        let zoom = || property(|v, _| PropertyChange::Scale { x: v as f32, y: v as f32 });
        let pos_x = || property(|v, c| PropertyChange::Position { x: v as f32, y: c.properties.position_y });
        let pos_y = || property(|v, c| PropertyChange::Position { x: c.properties.position_x, y: v as f32 });
        let rotation = || property(|v, _| PropertyChange::Rotation(v as f32));
        let grid = self.section("Transform", clip, vec![(1.0, zoom()), (0.0, pos_x()), (0.0, pos_y()), (0.0, rotation())]);
        self.number_row(&grid, 0, "Zoom", p.scale_x as f64, (0.1, 10.0, 0.05, 2), clip, zoom());
        self.number_row(&grid, 1, "Position X", p.position_x as f64, (-8000.0, 8000.0, 10.0, 0), clip, pos_x());
        self.number_row(&grid, 2, "Position Y", p.position_y as f64, (-8000.0, 8000.0, 10.0, 0), clip, pos_y());
        self.number_row(&grid, 3, "Rotation", p.rotation as f64, (-360.0, 360.0, 1.0, 1), clip, rotation());
    }

    fn composite_section(&self, clip: &Clip) {
        let opacity = || property(|v, _| PropertyChange::Opacity(v as f32 / 100.0));
        let grid = self.section("Composite", clip, vec![(100.0, opacity())]);
        self.number_row(&grid, 0, "Opacity", clip.properties.opacity as f64 * 100.0, (0.0, 100.0, 5.0, 0), clip, opacity());
    }

    fn audio_section(&self, clip: &Clip) {
        let p = &clip.properties;
        let volume = || property(|v, _| PropertyChange::Volume(v as f32 / 100.0));
        let pan = || property(|v, _| PropertyChange::Pan(v as f32 / 100.0));
        let grid = self.section("Audio", clip, vec![(100.0, volume()), (0.0, pan())]);
        self.number_row(&grid, 0, "Volume %", p.volume as f64 * 100.0, (0.0, 200.0, 5.0, 0), clip, volume());
        self.number_row(&grid, 1, "Pan", p.pan as f64 * 100.0, (-100.0, 100.0, 10.0, 0), clip, pan());
    }

    /// One section per filter on the clip: its settings and a button to take it off.
    fn filter_sections(&self, clip: &Clip) {
        for (index, effect) in clip.properties.effects.iter().enumerate() {
            let reset = effect
                .params
                .iter()
                .enumerate()
                .map(|(p, param)| (param.default as f64, Self::filter_param(index, p)))
                .collect();
            let grid = self.section(&effect.name, clip, reset);
            for (p, param) in effect.params.iter().enumerate() {
                let range = (param.min as f64, param.max as f64, ((param.max - param.min) as f64 / 100.0).max(0.01), 2);
                self.number_row(&grid, p as i32, &param.name, param.value as f64, range, clip, Self::filter_param(index, p));
            }
            let remove = gtk::Button::builder().label("Remove Filter").halign(gtk::Align::End).build();
            let state = self.state.clone();
            let id = clip.id;
            let effect_id = effect.id.clone();
            remove.connect_clicked(move |_| {
                let Some(mut edited) = state.selected_clip().filter(|c| c.id == id) else { return };
                edited.properties.effects.retain(|e| e.id != effect_id);
                state.execute(Box::new(EditClipCommand::new("Remove Filter", edited)));
            });
            grid.attach(&remove, 0, effect.params.len() as i32, 2, 1);
        }
    }

    fn filter_param(effect: usize, param: usize) -> MakeCommand {
        edit("Filter", move |v, c| {
            if let Some(p) = c.properties.effects.get_mut(effect).and_then(|e| e.params.get_mut(param)) {
                p.value = (v as f32).clamp(p.min, p.max);
            }
        })
    }

    /// Fade lengths in seconds, at the start and end of the clip.
    fn fade_section(&self, clip: &Clip) {
        let p = &clip.properties;
        let fade_in = || edit("Fade In", |v, c| c.properties.fade_in_us = ((v * 1e6) as i64).min(c.duration_us() / 2));
        let fade_out = || edit("Fade Out", |v, c| c.properties.fade_out_us = ((v * 1e6) as i64).min(c.duration_us() / 2));
        let grid = self.section("Fades", clip, vec![(0.0, fade_in()), (0.0, fade_out())]);
        let max = clip.duration_us() as f64 / 2e6;
        self.number_row(&grid, 0, "Fade In s", p.fade_in_us as f64 / 1e6, (0.0, max, 0.1, 1), clip, fade_in());
        self.number_row(&grid, 1, "Fade Out s", p.fade_out_us as f64 / 1e6, (0.0, max, 0.1, 1), clip, fade_out());
        // Shown only once a dissolve exists; it is added from the Effects panel or with Ctrl+T.
        if p.dissolve_in_us > 0 {
            let dissolve = edit("Cross Dissolve", |v, c| c.properties.dissolve_in_us = ((v * 1e6) as i64).min(c.source_in));
            self.number_row(&grid, 2, "Dissolve s", p.dissolve_in_us as f64 / 1e6, (0.0, clip.source_in as f64 / 1e6, 0.1, 1), clip, dissolve);
        }
    }

    fn title_section(&self, clip: &Clip) {
        let Some(title) = clip.title_data.clone() else { return };
        let grid = self.section("Title", clip, Vec::new());

        // Text: applied when Enter is pressed or the field loses focus.
        Self::row_label(&grid, 0, "Text");
        let entry = gtk::Entry::builder().text(&title.text).hexpand(true).build();
        entry.update_property(&[gtk::accessible::Property::Label("Title text")]);
        let commit = {
            let state = self.state.clone();
            let busy = self.busy.clone();
            let id = clip.id;
            move |e: &gtk::Entry| {
                let text = e.text().to_string();
                Self::apply(&state, &busy, id, |current| {
                    let mut edited = current.clone();
                    if let Some(t) = edited.title_data.as_mut() {
                        t.text = text.clone();
                    }
                    edited.name = text.chars().take(40).collect();
                    Box::new(EditClipCommand::new("Title Text", edited))
                });
            }
        };
        entry.connect_activate(commit.clone());
        let focus = gtk::EventControllerFocus::new();
        let e = entry.clone();
        let last = RefCell::new(title.text.clone());
        focus.connect_leave(move |_| {
            // Only when the text really changed, so leaving the field is not an undo step.
            if *last.borrow() != e.text().as_str() {
                *last.borrow_mut() = e.text().to_string();
                commit(&e);
            }
        });
        entry.add_controller(focus);
        grid.attach(&entry, 1, 0, 1, 1);

        self.number_row(
            &grid,
            1,
            "Size",
            title.font_size as f64,
            (8.0, 400.0, 4.0, 0),
            clip,
            edit("Title Size", |v, c| {
                if let Some(t) = c.title_data.as_mut() {
                    t.font_size = v as f32;
                }
            }),
        );

        Self::row_label(&grid, 2, "Place");
        let place = gtk::DropDown::from_strings(&["Centre", "Lower third"]);
        place.set_selected(if title.title_type == TitleType::LowerThird { 1 } else { 0 });
        let state = self.state.clone();
        let busy = self.busy.clone();
        let id = clip.id;
        place.connect_selected_notify(move |d| {
            let kind = if d.selected() == 1 { TitleType::LowerThird } else { TitleType::CenterTitle };
            Self::apply(&state, &busy, id, |current| {
                let mut edited = current.clone();
                if let Some(t) = edited.title_data.as_mut() {
                    t.title_type = kind;
                }
                Box::new(EditClipCommand::new("Title Place", edited))
            });
        });
        grid.attach(&place, 1, 2, 1, 1);
    }
}
