//! Inspector: settings of the selected clip. Rows follow Resolve's layout:
//! label right-aligned, value fields to its right, reset per section.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4 as gtk;
use gtk4::glib;
use gtk4::prelude::*;
use tempo_timeline::{
    frame_to_us, us_to_frame, Clip, ClipType, Command, EditClipCommand, EditKind, PropertyChange, RationalFps, SetClipPropertyCommand, SetTransitionCommand,
    TitleType, TrackKind, Transition, TransitionAlignment, TransitionEase, TransitionEdge, TransitionKind,
};
use uuid::Uuid;

use crate::state::{AppState, Change, EditSelection};
use crate::util::{label, tool_button};

/// Turns a new field value and the clip as it is now into the command that applies it.
type MakeCommand = Box<dyn Fn(f64, &Clip) -> Box<dyn Command>>;

pub struct Inspector {
    pub root: gtk::Box,
    state: Rc<AppState>,
    title: gtk::Label,
    body: gtk::Box,
    scroller: gtk::ScrolledWindow,
    /// What the panel shows (a transition, or the selected clips): its scroll position
    /// is kept while this stays the same.
    shown: Cell<Option<(Option<EditSelection>, Option<Uuid>)>>,
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
        let scroller = gtk::ScrolledWindow::builder().child(&body).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build();
        root.append(&scroller);

        let inspector = Rc::new(Self { root, state: state.clone(), title, body, scroller, shown: Cell::new(None), busy: Rc::new(Cell::new(false)) });
        let i = inspector.clone();
        state.connect(move |change| {
            if matches!(change, Change::Selection | Change::Timeline | Change::Project) {
                // Editing keeps the panel where the user is; something else selected starts at the top.
                let now = Some((i.state.edit_selection.get(), i.state.selected_ids().first().copied()));
                let at = if i.shown.replace(now) == now { i.scroller.vadjustment().value() } else { 0.0 };
                // An edit from the curve graph rebuilds the panel; keep the keyboard on the new graph
                // so the next Delete removes a key, not the transition.
                let graph_focused = i.root.root().and_then(|r| r.focus()).is_some_and(|f| f.has_css_class(crate::keybinds::OWNS_DELETE_KEYS) && f.is_ancestor(&i.body));
                i.rebuild();
                if graph_focused {
                    if let Some(graph) = descendant_with_class(i.body.upcast_ref(), crate::keybinds::OWNS_DELETE_KEYS) {
                        graph.grab_focus();
                    }
                }
                let adjustment = i.scroller.vadjustment();
                glib::idle_add_local_once(move || adjustment.set_value(at));
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
        if let Some(EditSelection::Transition { track_id, clip_id, edge }) = self.state.edit_selection.get() {
            if self.transition_panel(track_id, clip_id, edge) {
                return;
            }
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
        // Enter or Escape finishes typing and hands the keyboard back to the
        // editor, so Space plays and Delete deletes again without a click.
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_released(|controller, key, _, _| {
            if matches!(key, gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter | gtk::gdk::Key::Escape) {
                if let Some(window) = controller.widget().and_then(|w| w.root()).and_downcast::<gtk::Window>() {
                    gtk::prelude::GtkWindowExt::set_focus(&window, None::<&gtk::Widget>);
                }
            }
        });
        spin.add_controller(keys);

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
    }

    /// Resolve's transition panel for the selected transition. Returns false when the
    /// transition is gone, so the caller shows the clip instead. Every change is one
    /// `SetTransitionCommand`; the panel is rebuilt from the timeline afterwards.
    fn transition_panel(&self, track_id: Uuid, clip_id: Uuid, edge: TransitionEdge) -> bool {
        let Some(view) = TransitionView::read(&self.state, (track_id, clip_id, edge)) else { return false };
        let t = &view.transition;
        self.title.set_text(&format!("Transition - {}", kind_name(&t.kind)));
        let sel = (track_id, clip_id, edge);
        let state = &self.state;
        // Resolve's section header with an on/off switch; off plays the edit as a cut.
        let header = gtk::Box::builder().css_classes(["inspector-section"]).build();
        let heading = label(if view.track_kind == TrackKind::Audio { "Audio" } else { "Video" }, &["tempo-heading"]);
        heading.set_hexpand(true);
        heading.set_xalign(0.0);
        header.append(&heading);
        let switch = gtk::Switch::builder().active(t.enabled).valign(gtk::Align::Center).tooltip_text("Off: this edit plays as a cut").build();
        switch.update_property(&[gtk::accessible::Property::Label("Transition on or off")]);
        let s = state.clone();
        switch.connect_active_notify(move |sw| {
            let on = sw.is_active();
            edit_transition(&s, sel, |t| t.enabled = on);
        });
        header.append(&switch);
        self.body.append(&header);
        let grid = gtk::Grid::builder().row_spacing(8).column_spacing(8).margin_top(10).margin_bottom(12).margin_start(12).margin_end(12).sensitive(t.enabled).build();
        self.body.append(&grid);

        // Audio transitions (Cross Fade) have no type choice and no colour.
        let video = has_video_controls(view.track_kind);
        if video {
            Self::row_label(&grid, 0, "Transition Type");
            let names = TransitionKind::VIDEO_DISSOLVES.map(|k| k.resolve_name());
            let kinds = gtk::DropDown::from_strings(&names.iter().map(String::as_str).collect::<Vec<_>>());
            let at = TransitionKind::VIDEO_DISSOLVES.iter().position(|k| *k == t.kind).unwrap_or(2);
            kinds.set_selected(at as u32);
            let s = state.clone();
            kinds.connect_selected_notify(move |d| {
                let Some(kind) = TransitionKind::VIDEO_DISSOLVES.get(d.selected() as usize).cloned() else { return };
                edit_transition(&s, sel, |t| t.kind = kind);
            });
            grid.attach(&kinds, 1, 0, 1, 1);
        }

        Self::row_label(&grid, 1, "Duration");
        let frames = us_to_frame(t.duration_us, view.fps);
        let seconds = gtk::SpinButton::builder().adjustment(&gtk::Adjustment::new(t.duration_us as f64 / 1e6, 0.0, 3600.0, 0.1, 1.0, 0.0)).digits(1).numeric(true).hexpand(true).build();
        seconds.update_property(&[gtk::accessible::Property::Label("Duration in seconds")]);
        let frame_spin = gtk::SpinButton::builder().adjustment(&gtk::Adjustment::new(frames as f64, 1.0, 1_000_000.0, 1.0, 10.0, 0.0)).numeric(true).build();
        frame_spin.update_property(&[gtk::accessible::Property::Label("Duration in frames")]);
        let (s, fps, shown_tenths) = (state.clone(), view.fps, (t.duration_us as f64 / 1e5).round());
        seconds.connect_value_changed(move |sp| {
            // Showing one decimal rounds the value; only a real edit changes the length.
            if (sp.value() * 10.0).round() == shown_tenths {
                return;
            }
            let frames = seconds_to_frames(sp.value(), fps);
            edit_transition(&s, sel, |t| t.duration_us = frame_to_us(frames, fps));
        });
        let (s, fps) = (state.clone(), view.fps);
        frame_spin.connect_value_changed(move |sp| {
            let frames = sp.value() as i64;
            edit_transition(&s, sel, |t| t.duration_us = frame_to_us(frames, fps));
        });
        let durations = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        durations.append(&seconds);
        durations.append(&label("Seconds", &["tempo-dim"]));
        durations.append(&frame_spin);
        durations.append(&label("Frames", &["tempo-dim"]));
        durations.set_tooltip_text(Some(&format!("Longest {:.2} s ({} frames)", view.max_us as f64 / 1e6, us_to_frame(view.max_us, view.fps))));
        grid.attach(&durations, 1, 1, 1, 1);

        let default_button = gtk::Button::builder().label("Set as Default Duration").halign(gtk::Align::Start).tooltip_text("New transitions get this length").build();
        let (s, duration_us) = (state.clone(), t.duration_us);
        default_button.connect_clicked(move |_| {
            s.settings.borrow_mut().transition_us = duration_us;
            let snapshot = s.settings.borrow().clone();
            glib::MainContext::default().spawn_local(async move {
                if let Ok(Err(e)) = gtk::gio::spawn_blocking(move || snapshot.save()).await {
                    tracing::warn!("could not save settings: {e}");
                }
            });
            s.message(format!("New transitions are {:.2} s long", duration_us as f64 / 1e6));
        });
        grid.attach(&default_button, 1, 3, 1, 1);

        Self::row_label(&grid, 4, "Alignment");
        let aligns = gtk::Box::builder().css_classes(["linked"]).build();
        let mut group: Option<gtk::ToggleButton> = None;
        for (name, icon, alignment) in [("Start on Edit", "format-justify-left-symbolic", TransitionAlignment::StartAtCut), ("Center on Edit", "format-justify-center-symbolic", TransitionAlignment::Centered), ("End on Edit", "format-justify-right-symbolic", TransitionAlignment::EndAtCut)] {
            let button = gtk::ToggleButton::builder().icon_name(icon).tooltip_text(name).hexpand(true).sensitive(alignment_enabled(view.edit_kind, alignment)).build();
            button.update_property(&[gtk::accessible::Property::Label(name)]);
            button.set_group(group.as_ref());
            button.set_active(t.alignment == alignment);
            let s = state.clone();
            button.connect_toggled(move |b| {
                if b.is_active() {
                    edit_transition(&s, sel, |t| t.alignment = alignment);
                }
            });
            group.get_or_insert_with(|| button.clone());
            aligns.append(&button);
        }
        grid.attach(&aligns, 1, 4, 1, 1);

        if video {
            Self::ratio_row(&grid, 5, "Start Ratio", t.start_ratio, sel, state, |t, r| t.start_ratio = r, 0.0);
            Self::ratio_row(&grid, 6, "End Ratio", t.end_ratio, sel, state, |t, r| t.end_ratio = r, 1.0);
            Self::ease_row(&grid, 7, t, sel, state);
            Self::curve_row(&grid, 8, t, sel, state);
            grid.attach(&curve_graph(t, sel, state), 0, 9, 2, 1);
        }
        if video && t.kind == TransitionKind::DipToColor {
            Self::row_label(&grid, 10, "Color");
            let button = gtk::ColorDialogButton::new(Some(gtk::ColorDialog::new()));
            button.set_rgba(&gtk::gdk::RGBA::new(t.color[0], t.color[1], t.color[2], 1.0));
            let s = state.clone();
            button.connect_rgba_notify(move |b| {
                let c = b.rgba();
                edit_transition(&s, sel, |t| t.color = [c.red(), c.green(), c.blue()]);
            });
            grid.attach(&button, 1, 10, 1, 1);
        }
        true
    }

    /// Slider and field sharing one adjustment, with a reset arrow. Returns the adjustment,
    /// a flag that is set while the program (not the user) moves it, and the row.
    /// `commit` runs once the user has been still for a moment, so a drag is one undo step.
    fn slider_row(adj: &gtk::Adjustment, digits: u32, label_text: &str, commit: impl Fn(f64) + 'static) -> (Rc<Cell<bool>>, gtk::Box, gtk::Button) {
        const QUIET_MS: u64 = 300;
        let spin = gtk::SpinButton::builder().adjustment(adj).digits(digits).numeric(true).build();
        spin.update_property(&[gtk::accessible::Property::Label(label_text)]);
        let scale = gtk::Scale::builder().adjustment(adj).hexpand(true).draw_value(false).build();
        scale.update_property(&[gtk::accessible::Property::Label(label_text)]);
        let reset = tool_button("edit-undo-symbolic", &format!("Reset {label_text}"));
        let refreshing = Rc::new(Cell::new(false));
        let pending: Rc<RefCell<Option<glib::SourceId>>> = Rc::default();
        let commit = Rc::new(commit);
        let (flag, adj_value) = (refreshing.clone(), adj.clone());
        adj.connect_value_changed(move |_| {
            if flag.get() {
                return;
            }
            if let Some(id) = pending.borrow_mut().take() {
                id.remove();
            }
            let (commit, pending_slot, adj) = (commit.clone(), pending.clone(), adj_value.clone());
            let id = glib::timeout_add_local_once(std::time::Duration::from_millis(QUIET_MS), move || {
                pending_slot.borrow_mut().take();
                commit(adj.value());
            });
            *pending.borrow_mut() = Some(id);
        });
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        row.append(&scale);
        row.append(&spin);
        row.append(&reset);
        (refreshing, row, reset)
    }

    /// Start / End Ratio: percent field and slider for a 0.0-1.0 ratio.
    fn ratio_row(grid: &gtk::Grid, row: i32, name: &str, ratio: f32, sel: TransitionRef, state: &Rc<AppState>, set: fn(&mut Transition, f32), default_ratio: f32) {
        Self::row_label(grid, row, name);
        let adj = gtk::Adjustment::new(f64::from(ratio_to_percent(ratio)), 0.0, 100.0, 1.0, 10.0, 0.0);
        let s = state.clone();
        let (_, cell, reset) = Self::slider_row(&adj, 0, name, move |v| {
            let ratio = percent_to_ratio(v);
            edit_transition(&s, sel, |t| set(t, ratio));
        });
        let s = state.clone();
        reset.connect_clicked(move |_| edit_transition(&s, sel, |t| set(t, default_ratio)));
        grid.attach(&cell, 1, row, 1, 1);
    }

    fn ease_row(grid: &gtk::Grid, row: i32, t: &Transition, sel: TransitionRef, state: &Rc<AppState>) {
        const EASES: [TransitionEase; 4] = [TransitionEase::None, TransitionEase::In, TransitionEase::Out, TransitionEase::InOut];
        Self::row_label(grid, row, "Ease");
        let drop = gtk::DropDown::from_strings(&["None", "In", "Out", "In & Out"]);
        drop.set_selected(EASES.iter().position(|e| *e == t.ease).unwrap_or(0) as u32);
        // Curve keys set the shape; the panel is rebuilt after Reset Curve, which re-enables this.
        if !t.curve.is_empty() {
            drop.set_sensitive(false);
            drop.set_tooltip_text(Some("The curve keys set the shape. Reset the curve to use Ease."));
        }
        let s = state.clone();
        drop.connect_selected_notify(move |d| {
            let ease = EASES.get(d.selected() as usize).copied().unwrap_or_default();
            edit_transition(&s, sel, |t| t.ease = ease);
        });
        grid.attach(&drop, 1, row, 1, 1);
    }

    /// Transition Curve at the playhead: shows the mix there; typing keys it.
    fn curve_row(grid: &gtk::Grid, row: i32, t: &Transition, sel: TransitionRef, state: &Rc<AppState>) {
        const OUTSIDE: &str = "Move the playhead into the transition to key its curve.";
        Self::row_label(grid, row, "Transition Curve");
        let adj = gtk::Adjustment::new(0.0, 0.0, 1.0, 0.01, 0.1, 0.0);
        let s = state.clone();
        let (refreshing, cell, reset) = Self::slider_row(&adj, 3, "Transition Curve", move |value| {
            let Some(u) = TransitionView::read(&s, sel).and_then(|v| playhead_fraction(s.playhead_us(), v.span?)) else { return };
            edit_transition(&s, sel, |t| *t = t.with_curve_key(u, value as f32));
        });
        reset.set_tooltip_text(Some("Reset Curve"));
        reset.set_sensitive(!t.curve.is_empty());
        let s = state.clone();
        reset.connect_clicked(move |_| edit_transition(&s, sel, |t| t.curve.clear()));
        // Show the curve where the playhead is; insensitive outside the transition.
        let refresh = {
            let (cell, adj, refreshing, s) = (cell.clone(), adj.clone(), refreshing.clone(), state.clone());
            move || {
                let view = TransitionView::read(&s, sel);
                let u = view.as_ref().and_then(|v| playhead_fraction(s.playhead_us(), v.span?));
                for child in std::iter::successors(cell.first_child(), |w| w.next_sibling()).filter(|w| !w.is::<gtk::Button>()) {
                    child.set_sensitive(u.is_some());
                }
                cell.set_tooltip_text(if u.is_some() { None } else { Some(OUTSIDE) });
                if let (Some(u), Some(v)) = (u, view) {
                    if cell.focus_child().is_none() {
                        refreshing.set(true);
                        adj.set_value(f64::from(v.transition.curve_at(u)));
                        refreshing.set(false);
                    }
                }
            }
        };
        refresh();
        // The panel is rebuilt on every edit; the poll ends with the row.
        let weak = cell.downgrade();
        glib::timeout_add_local(std::time::Duration::from_millis(100), move || match weak.upgrade() {
            Some(c) if c.is_mapped() => {
                refresh();
                glib::ControlFlow::Continue
            }
            Some(_) => glib::ControlFlow::Continue,
            None => glib::ControlFlow::Break,
        });
        grid.attach(&cell, 1, row, 1, 1);
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

/// The transition being edited and what limits it, read from the project.
struct TransitionView {
    transition: Transition,
    /// Where the transition lies on the timeline, in µs.
    span: Option<(i64, i64)>,
    max_us: i64,
    edit_kind: EditKind,
    track_kind: TrackKind,
    fps: RationalFps,
}

type TransitionRef = (Uuid, Uuid, TransitionEdge);

impl TransitionView {
    fn read(state: &AppState, (track_id, clip_id, edge): TransitionRef) -> Option<Self> {
        state
            .with_project(|p| {
                let track = p.timeline.find_track(track_id)?;
                let transition = track.transition_at_edit(clip_id, edge)?.clone();
                let clip = track.find_clip(clip_id)?;
                let edit_kind = if edge == TransitionEdge::Out { EditKind::Tail } else { track.edit_at(clip.timeline_in)?.kind };
                let max_us = track.max_transition_us(clip_id, edge, transition.alignment, &p.sources, p.timeline_fps());
                let span = track.transition_span(&transition);
                Some(Self { transition, span, max_us, edit_kind, track_kind: track.kind, fps: p.timeline_fps() })
            })
            .flatten()
    }
}

/// Apply `change` to the transition as one undo step. The length stays whole frames,
/// at least one, and within what the new alignment allows.
fn edit_transition(state: &Rc<AppState>, (track_id, clip_id, edge): TransitionRef, change: impl FnOnce(&mut Transition)) {
    let Some(view) = TransitionView::read(state, (track_id, clip_id, edge)) else { return };
    let mut edited = view.transition.clone();
    change(&mut edited);
    let max_us = state
        .with_project(|p| p.timeline.find_track(track_id).map_or(0, |t| t.max_transition_us(clip_id, edge, edited.alignment, &p.sources, p.timeline_fps())))
        .unwrap_or(0);
    if max_us <= 0 {
        state.message("No room for a transition here");
        return;
    }
    edited.duration_us = clamp_transition_us(edited.duration_us, max_us, view.fps);
    if edited != view.transition {
        state.execute(Box::new(SetTransitionCommand::set(track_id, edited)));
    }
}

fn percent_to_ratio(percent: f64) -> f32 {
    (percent / 100.0).clamp(0.0, 1.0) as f32
}

fn ratio_to_percent(ratio: f32) -> f32 {
    (ratio * 100.0).clamp(0.0, 100.0)
}

/// How far through the transition the playhead is, 0.0-1.0; `None` outside it.
fn playhead_fraction(playhead_us: i64, (start, end): (i64, i64)) -> Option<f32> {
    (end > start && (start..=end).contains(&playhead_us)).then(|| (playhead_us - start) as f32 / (end - start) as f32)
}

fn kind_name(kind: &TransitionKind) -> String {
    kind.resolve_name()
}

const GRAPH_HEIGHT: i32 = 110;
const GRAPH_PAD: f64 = 10.0;
const KEY_HIT_PX: f64 = 9.0;
/// Keys closer than this in place would be merged by `with_curve_key`.
const MIN_KEY_GAP: f32 = 0.02;

/// Pixel of a (place, value) point in a graph `w` x `h` px; value 1 is at the top.
fn graph_point(u: f32, v: f32, w: f64, h: f64) -> (f64, f64) {
    (GRAPH_PAD + f64::from(u) * (w - 2.0 * GRAPH_PAD), GRAPH_PAD + (1.0 - f64::from(v)) * (h - 2.0 * GRAPH_PAD))
}

/// The (place, value) under a pixel, clamped to 0..1.
fn graph_value(x: f64, y: f64, w: f64, h: f64) -> (f32, f32) {
    let u = (x - GRAPH_PAD) / (w - 2.0 * GRAPH_PAD).max(1.0);
    let v = 1.0 - (y - GRAPH_PAD) / (h - 2.0 * GRAPH_PAD).max(1.0);
    (u.clamp(0.0, 1.0) as f32, v.clamp(0.0, 1.0) as f32)
}

/// The key nearest to the pixel, if within reach.
fn hit_key(keys: &[[f32; 2]], x: f64, y: f64, w: f64, h: f64) -> Option<usize> {
    let dist = |k: &[f32; 2]| {
        let (kx, ky) = graph_point(k[0], k[1], w, h);
        (kx - x).hypot(ky - y)
    };
    keys.iter().enumerate().map(|(i, k)| (i, dist(k))).filter(|(_, d)| *d <= KEY_HIT_PX).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(i, _)| i)
}

/// Key `index` moved to (u, v), clamped to 0..1 and kept between its neighbours.
fn move_key(keys: &[[f32; 2]], index: usize, u: f32, v: f32) -> Vec<[f32; 2]> {
    let mut moved = keys.to_vec();
    if let Some(key) = moved.get_mut(index) {
        let low = if index == 0 { 0.0 } else { keys[index - 1][0] + MIN_KEY_GAP };
        let high = keys.get(index + 1).map_or(1.0, |n| n[0] - MIN_KEY_GAP);
        *key = [u.clamp(low, high.max(low)), v.clamp(0.0, 1.0)];
    }
    moved
}

/// A key added at (u, v); `None` when another key is too close.
fn add_key(keys: &[[f32; 2]], u: f32, v: f32) -> Option<Vec<[f32; 2]>> {
    if keys.iter().any(|k| (k[0] - u).abs() < MIN_KEY_GAP) {
        return None;
    }
    let mut added = keys.to_vec();
    added.push([u, v]);
    added.sort_by(|a, b| a[0].total_cmp(&b[0]));
    Some(added)
}

/// The curve editor under the Transition Curve row. Edits are one undo step each
/// (a drag commits on release); the panel is rebuilt afterwards.
/// The first widget under `w` (or `w` itself) with the CSS class.
fn descendant_with_class(w: &gtk::Widget, class: &str) -> Option<gtk::Widget> {
    if w.has_css_class(class) {
        return Some(w.clone());
    }
    let mut child = w.first_child();
    while let Some(c) = child {
        if let Some(found) = descendant_with_class(&c, class) {
            return Some(found);
        }
        child = c.next_sibling();
    }
    None
}

fn curve_graph(t: &Transition, sel: TransitionRef, state: &Rc<AppState>) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::builder().content_height(GRAPH_HEIGHT).hexpand(true).focusable(true).can_focus(true).css_classes([crate::keybinds::OWNS_DELETE_KEYS]).build();
    area.set_tooltip_text(Some("Drag a key to move it. Double-click to add one. Right-click or Delete removes the selected key."));
    area.update_property(&[gtk::accessible::Property::Label("Transition curve graph")]);
    let shape = Rc::new(RefCell::new(t.clone()));
    let selected: Rc<Cell<Option<usize>>> = Rc::default();
    let size = |a: &gtk::DrawingArea| (f64::from(a.width()), f64::from(a.height()));

    let (sh, selected_draw, s) = (shape.clone(), selected.clone(), state.clone());
    area.set_draw_func(move |a, cr, w, h| {
        let (w, h) = (f64::from(w), f64::from(h));
        let paint = |name: &str| {
            let c = crate::util::css_color(a, name);
            cr.set_source_rgba(f64::from(c.red()), f64::from(c.green()), f64::from(c.blue()), f64::from(c.alpha()));
        };
        paint("tempo_bg_inset");
        cr.rectangle(0.0, 0.0, w, h);
        let _ = cr.fill();
        let (x0, y0) = graph_point(0.0, 1.0, w, h);
        let (x1, y1) = graph_point(1.0, 0.0, w, h);
        paint("tempo_line_soft");
        cr.set_line_width(1.0);
        cr.rectangle(x0, y0, x1 - x0, y1 - y0);
        let _ = cr.stroke();
        let shape = sh.borrow();
        if let Some(u) = TransitionView::read(&s, sel).and_then(|v| playhead_fraction(s.playhead_us(), v.span?)) {
            paint("tempo_accent");
            let (px, _) = graph_point(u, 0.0, w, h);
            cr.move_to(px.round() + 0.5, y0);
            cr.line_to(px.round() + 0.5, y1);
            let _ = cr.stroke();
        }
        paint("tempo_text");
        cr.set_line_width(2.0);
        for i in 0..=64 {
            let u = i as f32 / 64.0;
            let (x, y) = graph_point(u, shape.curve_at(u), w, h);
            if i == 0 { cr.move_to(x, y) } else { cr.line_to(x, y) }
        }
        let _ = cr.stroke();
        for (i, k) in shape.curve.iter().enumerate() {
            let (x, y) = graph_point(k[0], k[1], w, h);
            paint(if selected_draw.get() == Some(i) { "tempo_accent" } else { "tempo_text_bright" });
            cr.arc(x, y, 4.0, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
        }
    });

    // Redraw for the playhead; ends with the widget.
    let weak = area.downgrade();
    glib::timeout_add_local(std::time::Duration::from_millis(100), move || match weak.upgrade() {
        Some(a) => {
            if a.is_mapped() {
                a.queue_draw();
            }
            glib::ControlFlow::Continue
        }
        None => glib::ControlFlow::Break,
    });

    let commit = {
        let s = state.clone();
        move |keys: Vec<[f32; 2]>| edit_transition(&s, sel, |t| t.curve = keys)
    };

    // Drag a key; commit on release.
    let drag = gtk::GestureDrag::builder().button(1).build();
    let moving: Rc<Cell<Option<(usize, f64, f64)>>> = Rc::default();
    let (sh, mv, selected_c, a) = (shape.clone(), moving.clone(), selected.clone(), area.clone());
    drag.connect_drag_begin(move |_, x, y| {
        let (w, h) = size(&a);
        let hit = hit_key(&sh.borrow().curve, x, y, w, h);
        mv.set(hit.map(|i| (i, x, y)));
        selected_c.set(hit);
        a.grab_focus();
        a.queue_draw();
    });
    let (sh, mv, a) = (shape.clone(), moving.clone(), area.clone());
    drag.connect_drag_update(move |_, dx, dy| {
        let Some((i, x, y)) = mv.get() else { return };
        let (w, h) = size(&a);
        let (u, v) = graph_value(x + dx, y + dy, w, h);
        let moved = move_key(&sh.borrow().curve, i, u, v);
        sh.borrow_mut().curve = moved;
        a.queue_draw();
    });
    let (sh, mv, commit_drag, original) = (shape.clone(), moving, commit.clone(), t.curve.clone());
    drag.connect_drag_end(move |_, dx, dy| {
        let Some(_) = mv.take() else { return };
        let keys = sh.borrow().curve.clone();
        if (dx != 0.0 || dy != 0.0) && keys != original {
            commit_drag(keys);
        }
    });
    area.add_controller(drag);

    // Double-click on empty space adds a key on the curve there.
    let click = gtk::GestureClick::builder().button(1).build();
    let (sh, a, commit_add) = (shape.clone(), area.clone(), commit.clone());
    click.connect_pressed(move |_, presses, x, y| {
        let (w, h) = size(&a);
        let shape = sh.borrow();
        if presses != 2 || hit_key(&shape.curve, x, y, w, h).is_some() {
            return;
        }
        let (u, _) = graph_value(x, y, w, h);
        if let Some(keys) = add_key(&shape.curve, u, shape.curve_at(u)) {
            drop(shape);
            commit_add(keys);
        }
    });
    area.add_controller(click);

    // Right-click on a key removes it.
    let right = gtk::GestureClick::builder().button(3).build();
    let (sh, a, commit_remove) = (shape.clone(), area.clone(), commit.clone());
    right.connect_pressed(move |_, _, x, y| {
        let (w, h) = size(&a);
        let hit = hit_key(&sh.borrow().curve, x, y, w, h);
        if let Some(i) = hit {
            let mut keys = sh.borrow().curve.clone();
            keys.remove(i);
            commit_remove(keys);
        }
    });
    area.add_controller(right);

    // Delete or BackSpace removes the selected key.
    let keys_ctl = gtk::EventControllerKey::new();
    let sh = shape;
    keys_ctl.connect_key_pressed(move |_, key, _, _| {
        use gtk::gdk::Key;
        let Some(i) = selected.get().filter(|_| matches!(key, Key::Delete | Key::BackSpace)) else { return glib::Propagation::Proceed };
        let mut keys = sh.borrow().curve.clone();
        if i < keys.len() {
            keys.remove(i);
            selected.set(None);
            commit(keys);
        }
        glib::Propagation::Stop
    });
    area.add_controller(keys_ctl);
    area
}

/// The Type dropdown and Color are video-only.
fn has_video_controls(kind: TrackKind) -> bool {
    kind == TrackKind::Video
}

fn seconds_to_frames(seconds: f64, fps: RationalFps) -> i64 {
    (seconds * fps.to_f64()).round() as i64
}

/// A length in whole frames, from 1 frame up to `max_us`.
fn clamp_transition_us(duration_us: i64, max_us: i64, fps: RationalFps) -> i64 {
    let frames = us_to_frame(duration_us, fps).max(1);
    frame_to_us(frames, fps).min(max_us)
}

/// A head transition can only start on the edit, a tail one only end on it.
fn alignment_enabled(kind: EditKind, alignment: TransitionAlignment) -> bool {
    match kind {
        EditKind::Cut => true,
        EditKind::Head => alignment == TransitionAlignment::StartAtCut,
        EditKind::Tail => alignment == TransitionAlignment::EndAtCut,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn ratios_and_playhead_fraction_map_as_expected() {
        assert_eq!(percent_to_ratio(50.0), 0.5);
        assert_eq!(percent_to_ratio(250.0), 1.0);
        assert_eq!(ratio_to_percent(0.25), 25.0);
        assert_eq!(playhead_fraction(150, (100, 200)), Some(0.5));
        assert_eq!(playhead_fraction(99, (100, 200)), None);
        assert_eq!(playhead_fraction(201, (100, 200)), None);
        assert_eq!(playhead_fraction(5, (5, 5)), None);
    }

    #[test]
    fn audio_transition_panel_is_cross_fade_without_video_controls() {
        assert_eq!(super::kind_name(&super::TransitionKind::Crossfade), "Cross Fade");
        assert!(!super::has_video_controls(super::TrackKind::Audio));
        assert!(super::has_video_controls(super::TrackKind::Video));
    }

    use super::*;

    #[test]
    fn graph_pixels_map_both_ways_and_clamp() {
        let (x, y) = graph_point(0.5, 0.25, 220.0, 110.0);
        assert_eq!((x, y), (110.0, 10.0 + 0.75 * 90.0));
        let (u, v) = graph_value(x, y, 220.0, 110.0);
        assert!((u - 0.5).abs() < 1e-6 && (v - 0.25).abs() < 1e-6);
        assert_eq!(graph_value(-50.0, 500.0, 220.0, 110.0), (0.0, 0.0));
        assert_eq!(graph_value(900.0, -5.0, 220.0, 110.0), (1.0, 1.0));
    }

    #[test]
    fn hit_key_picks_the_nearest_key_within_reach() {
        let keys = [[0.25, 0.5], [0.75, 0.5]];
        let (x, y) = graph_point(0.75, 0.5, 220.0, 110.0);
        assert_eq!(hit_key(&keys, x + 3.0, y - 2.0, 220.0, 110.0), Some(1));
        assert_eq!(hit_key(&keys, x + 30.0, y, 220.0, 110.0), None);
        assert_eq!(hit_key(&[], x, y, 220.0, 110.0), None);
    }

    #[test]
    fn a_key_stays_between_its_neighbours_and_inside_the_graph() {
        let keys = [[0.2, 0.3], [0.5, 0.5], [0.8, 0.7]];
        assert_eq!(move_key(&keys, 1, 0.9, 2.0)[1], [0.8 - MIN_KEY_GAP, 1.0]);
        assert_eq!(move_key(&keys, 1, 0.0, -1.0)[1], [0.2 + MIN_KEY_GAP, 0.0]);
        assert_eq!(move_key(&keys, 0, -1.0, 0.4)[0], [0.0, 0.4]);
        assert_eq!(move_key(&keys, 2, 2.0, 0.4)[2], [1.0, 0.4]);
        assert_eq!(move_key(&keys, 9, 0.5, 0.5), keys.to_vec());
    }

    #[test]
    fn adding_a_key_keeps_order_and_refuses_a_crowded_place() {
        let keys = [[0.2, 0.3], [0.8, 0.7]];
        assert_eq!(add_key(&keys, 0.5, 0.5), Some(vec![[0.2, 0.3], [0.5, 0.5], [0.8, 0.7]]));
        assert_eq!(add_key(&keys, 0.21, 0.5), None);
        assert_eq!(add_key(&[], 0.5, 0.5), Some(vec![[0.5, 0.5]]));
    }

    const FPS_30: RationalFps = RationalFps { num: 30, den: 1 };

    #[test]
    fn seconds_and_frames_are_linked_in_whole_frames() {
        assert_eq!(seconds_to_frames(1.0, FPS_30), 30);
        assert_eq!(seconds_to_frames(0.52, FPS_30), 16);
        assert_eq!(us_to_frame(frame_to_us(16, FPS_30), FPS_30), 16);
    }

    #[test]
    fn length_is_clamped_to_one_frame_and_the_maximum() {
        let max = frame_to_us(45, FPS_30);
        assert_eq!(clamp_transition_us(0, max, FPS_30), frame_to_us(1, FPS_30));
        assert_eq!(clamp_transition_us(5_000_000, max, FPS_30), max);
        assert_eq!(clamp_transition_us(frame_to_us(12, FPS_30), max, FPS_30), frame_to_us(12, FPS_30));
    }

    #[test]
    fn head_and_tail_allow_only_their_alignment() {
        use TransitionAlignment::*;
        assert!([StartAtCut, Centered, EndAtCut].iter().all(|a| alignment_enabled(EditKind::Cut, *a)));
        assert_eq!([StartAtCut, Centered, EndAtCut].map(|a| alignment_enabled(EditKind::Head, a)), [true, false, false]);
        assert_eq!([StartAtCut, Centered, EndAtCut].map(|a| alignment_enabled(EditKind::Tail, a)), [false, false, true]);
    }
}
