//! Inspector: settings of the selected clip. Rows follow Resolve's layout:
//! label right-aligned, value fields to its right, reset per section.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4 as gtk;
use gtk4::glib;
use gtk4::prelude::*;
use tempo_timeline::{
    frame_to_us, us_to_frame, Clip, ClipType, Command, CompositeCommand, EditClipCommand, EditKind, PropertyChange, RationalFps, SetClipPropertyCommand,
    SetTransitionCommand, TitleData, TitleType, TrackKind, Transition, TransitionAlignment, TransitionEase, TransitionEdge, TransitionKind,
};
use uuid::Uuid;

use crate::state::{AppState, Change, EditSelection};
use crate::util::{label, tool_button};

/// Turns a new field value and the clip as it is now into the command that applies it.
type MakeCommand = Box<dyn Fn(f64, &Clip) -> Box<dyn Command>>;
/// Resets one clip's section fields back to defaults.
type ResetClipFn = Box<dyn Fn(&mut Clip)>;

const DEFAULT_TITLE_BG: [u8; 4] = [0, 0, 0, 170];
const FONT_PRESETS: [&str; 3] = ["Sans", "Serif", "Monospace"];

pub struct Inspector {
    pub root: gtk::Box,
    state: Rc<AppState>,
    title: gtk::Label,
    body: gtk::Box,
    scroller: gtk::ScrolledWindow,
    /// What the panel shows (a transition, or the selected clips): its scroll position
    /// is kept while this stays the same.
    shown: RefCell<Option<(Option<EditSelection>, Vec<Uuid>)>>,
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

fn rgba_from_bytes([r, g, b, a]: [u8; 4]) -> gtk::gdk::RGBA {
    gtk::gdk::RGBA::new(f32::from(r) / 255.0, f32::from(g) / 255.0, f32::from(b) / 255.0, f32::from(a) / 255.0)
}

fn bytes_from_rgba(c: &gtk::gdk::RGBA) -> [u8; 4] {
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    [byte(c.red()), byte(c.green()), byte(c.blue()), byte(c.alpha())]
}

fn is_mixed_f64(clips: &[Clip], get: impl Fn(&Clip) -> f64) -> bool {
    let Some(first) = clips.first().map(&get) else { return false };
    clips[1..].iter().any(|c| (get(c) - first).abs() > 1e-3)
}

/// A field value rounded to what the field shows. GtkSpinButton re-reads its text when it loses
/// focus; a stored value that differs from that text (0.7 opacity is 69.9999988 %) would fire
/// value-changed and write the shown value onto every selected clip.
fn round_to_digits(v: f64, digits: u32) -> f64 {
    let scale = 10f64.powi(digits as i32);
    (v * scale).round() / scale
}

/// The Place dropdown's index: none shown while the titles differ, so any pick applies to all.
fn place_selection(kind: TitleType, mixed: bool) -> u32 {
    match (mixed, kind) {
        (true, _) => gtk::INVALID_LIST_POSITION,
        (false, TitleType::LowerThird) => 1,
        (false, _) => 0,
    }
}

/// Whether a title entry holds an edit to commit. An entry left empty under a Mixed
/// placeholder (no `last` text) is no change.
fn title_entry_has_edit(last: Option<&str>, dirty: bool, text: &str) -> bool {
    match last {
        None => dirty && !text.is_empty(),
        Some(prev) => prev != text,
    }
}

/// Stands in for a `ColorDialogButton` while the selected titles' colors differ: that button
/// drops a pick equal to the color it shows, so the shown color could not be applied to all.
fn mixed_color_button(initial: [u8; 4], accessible: &str, pick: impl Fn([u8; 4]) + 'static) -> gtk::Button {
    let btn = gtk::Button::builder().label("Choose…").tooltip_text("Pick one color for all selected titles").build();
    btn.update_property(&[gtk::accessible::Property::Label(accessible)]);
    let pick = Rc::new(pick);
    btn.connect_clicked(move |b| {
        let pick = pick.clone();
        let parent = b.root().and_downcast::<gtk::Window>();
        gtk::ColorDialog::new().choose_rgba(parent.as_ref(), Some(&rgba_from_bytes(initial)), gtk::gio::Cancellable::NONE, move |res| {
            // An error here means the dialog was dismissed.
            if let Ok(c) = res {
                pick(bytes_from_rgba(&c));
            }
        });
    });
    btn
}

fn selected_clips(state: &AppState) -> Vec<Clip> {
    let ids = state.selected_ids();
    state
        .with_timeline(|t| ids.iter().filter_map(|id| t.find_clip(*id).map(|(_, c)| c.clone())).collect())
        .unwrap_or_default()
}

fn current_clips_for_edit(state: &AppState, ids: &[Uuid]) -> Vec<Clip> {
    state
        .with_timeline(|t| {
            let found: Vec<(bool, Clip)> = ids.iter().filter_map(|id| t.find_clip(*id).map(|(tr, c)| (tr.locked, c.clone()))).collect();
            let unlocked: Vec<Clip> = found.iter().filter(|(locked, _)| !*locked).map(|(_, c)| c.clone()).collect();
            if unlocked.is_empty() {
                found.into_iter().take(1).map(|(_, c)| c).collect()
            } else {
                unlocked
            }
        })
        .unwrap_or_default()
}

fn build_batch_command(label: &str, clips: &[Clip], make: impl Fn(&Clip) -> Box<dyn Command>) -> Option<Box<dyn Command>> {
    match clips {
        [] => None,
        [one] => Some(make(one)),
        many => Some(Box::new(CompositeCommand::new(label, many.iter().map(make).collect()))),
    }
}

fn mixed_badge_label(tooltip: &str) -> gtk::Label {
    let badge = label("Mixed", &["tempo-small", "tempo-dim"]);
    badge.set_tooltip_text(Some(tooltip));
    badge.set_valign(gtk::Align::Center);
    badge
}

fn unfocus_widget(widget: &gtk::Widget) {
    if let Some(window) = widget.root().and_downcast::<gtk::Window>() {
        gtk::prelude::GtkWindowExt::set_focus(&window, None::<&gtk::Widget>);
    }
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
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .build();
        let header = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(4)
            .css_classes(["inspector-title"])
            .build();
        let inspector_help = crate::help::panel_guide_button(
            "Inspector",
            "Inspector",
            "Adjust Transform, Opacity, Volume, Fades, Titles, or Transitions for the selected clip(s). Ctrl+click clips to batch-edit together.",
            &[
                ("Click title/value", "Type exact numbers or drag slider"),
                ("Reset ↺", "Revert section defaults"),
                ("Ctrl+Z", "Undo batch edits in one step"),
            ],
        );
        header.append(&title);
        header.append(&inspector_help);
        root.append(&header);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let scroller = gtk::ScrolledWindow::builder().child(&body).vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).build();
        root.append(&scroller);

        let inspector = Rc::new(Self { root, state: state.clone(), title, body, scroller, shown: RefCell::new(None), busy: Rc::new(Cell::new(false)) });
        let i = inspector.clone();
        state.connect(move |change| {
            if matches!(change, Change::Selection | Change::Timeline | Change::Project) {
                if i.busy.get() {
                    return;
                }
                // Editing keeps the panel where the user is; something else selected starts at the top.
                let now = Some((i.state.edit_selection.get(), i.state.selected_ids()));
                let same = *i.shown.borrow() == now;
                *i.shown.borrow_mut() = now;
                let at = if same { i.scroller.vadjustment().value() } else { 0.0 };
                // An edit from the curve graph rebuilds the panel; keep the keyboard on the new graph
                // so the next Delete removes a key, not the transition.
                let prev_focus = i.root.root().and_then(|r| r.focus());
                let had_no_focus = prev_focus.is_none();
                let in_body = prev_focus.as_ref().is_some_and(|f| f.is_ancestor(&i.body));
                let graph_focused = prev_focus.as_ref().is_some_and(|f| f.has_css_class(crate::keybinds::OWNS_DELETE_KEYS) && f.is_ancestor(&i.body));
                if !graph_focused && in_body {
                    if let Some(win) = i.root.root().and_downcast::<gtk::Window>() {
                        gtk::prelude::GtkWindowExt::set_focus(&win, None::<&gtk::Widget>);
                    }
                }
                i.rebuild();
                if graph_focused {
                    if let Some(graph) = descendant_with_class(i.body.upcast_ref(), crate::keybinds::OWNS_DELETE_KEYS) {
                        graph.grab_focus();
                    }
                } else if in_body || had_no_focus {
                    // Prevent GTK from falling back onto the first text entry when a focused
                    // widget in the rebuilt body is removed (or when focus was unset), which would hijack Ctrl+Z.
                    if let Some(win) = i.root.root().and_downcast::<gtk::Window>() {
                        gtk::prelude::GtkWindowExt::set_focus(&win, None::<&gtk::Widget>);
                    }
                }
                let adjustment = i.scroller.vadjustment();
                let (root, body) = (i.root.clone(), i.body.clone());
                glib::idle_add_local_once(move || {
                    adjustment.set_value(at);
                    if !graph_focused && (in_body || had_no_focus) {
                        if let Some(win) = root.root().and_downcast::<gtk::Window>() {
                            if gtk::prelude::GtkWindowExt::focus(&win).is_some_and(|f| f.is_ancestor(&body)) {
                                gtk::prelude::GtkWindowExt::set_focus(&win, None::<&gtk::Widget>);
                            }
                        }
                    }
                });
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
        let clips = selected_clips(&self.state);
        if clips.is_empty() {
            self.title.set_text("Inspector");
            let hint = label("Select a clip to see its settings", &["empty-hint"]);
            hint.set_margin_top(32);
            self.body.append(&hint);
            return;
        }
        self.title.set_text(&if clips.len() > 1 {
            format!("{} (+{} more)", clips[0].name, clips.len() - 1)
        } else {
            clips[0].name.clone()
        });

        if clips.len() == 1 {
            let clip = &clips[0];
            match clip.clip_type {
                ClipType::Audio => self.audio_section(&clips),
                ClipType::Title => {
                    self.title_section(&clips);
                    self.transform_section(&clips);
                    self.composite_section(&clips);
                }
                _ => {
                    self.transform_section(&clips);
                    self.composite_section(&clips);
                }
            }
            if !matches!(clip.clip_type, ClipType::Audio | ClipType::Title) {
                self.filter_sections(clip);
            }
            self.fade_section(&clips);
            return;
        }

        let title_clips: Vec<Clip> = clips.iter().filter(|c| c.clip_type == ClipType::Title && c.title_data.is_some()).cloned().collect();
        let visual_clips: Vec<Clip> = clips.iter().filter(|c| c.clip_type != ClipType::Audio).cloned().collect();
        let audio_clips: Vec<Clip> = clips.iter().filter(|c| c.clip_type == ClipType::Audio).cloned().collect();
        let picture_clips: Vec<Clip> = clips.iter().filter(|c| !matches!(c.clip_type, ClipType::Audio | ClipType::Title)).cloned().collect();

        if !title_clips.is_empty() && title_clips.len() == clips.len() {
            self.title_section(&title_clips);
        }
        if !visual_clips.is_empty() {
            self.transform_section(&visual_clips);
            self.composite_section(&visual_clips);
        }
        if !audio_clips.is_empty() {
            self.audio_section(&audio_clips);
        }
        if picture_clips.len() == 1 {
            self.filter_sections(&picture_clips[0]);
        }
        self.fade_section(&clips);
    }

    /// Run a command for `ids` (single command or atomic `CompositeCommand`) without the
    /// Inspector rebuilding itself.
    fn apply_clips(state: &Rc<AppState>, busy: &Cell<bool>, ids: &[Uuid], label: &str, make: impl Fn(&Clip) -> Box<dyn Command>) -> bool {
        let current = current_clips_for_edit(state, ids);
        let Some(cmd) = build_batch_command(label, &current, make) else { return false };
        busy.set(true);
        let ok = state.execute(cmd);
        busy.set(false);
        ok
    }

    /// Section header with a reset button; returns the grid the rows go into.
    fn section(&self, name: &str, clips: &[Clip], reset: Option<(&'static str, ResetClipFn)>) -> gtk::Grid {
        let header = gtk::Box::builder().css_classes(["inspector-section"]).build();
        let title = label(name, &["tempo-heading"]);
        title.set_hexpand(true);
        title.set_xalign(0.0);
        header.append(&title);
        if let Some((cmd_name, reset_fn)) = reset {
            let reset_btn = tool_button("edit-undo-symbolic", &format!("Reset {name}"));
            let state = self.state.clone();
            let ids: Vec<Uuid> = clips.iter().map(|c| c.id).collect();
            let reset_fn = RefCell::new(Some(reset_fn));
            reset_btn.connect_clicked(move |_| {
                // The rebuild after the command replaces this button, so take the callback once.
                let Some(reset_fn) = reset_fn.borrow_mut().take() else { return };
                let current = current_clips_for_edit(&state, &ids);
                if let Some(cmd) = build_batch_command(cmd_name, &current, |clip| {
                    let mut edited = clip.clone();
                    reset_fn(&mut edited);
                    Box::new(EditClipCommand::new(cmd_name, edited))
                }) {
                    state.execute(cmd);
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

    /// One number row. Typing or stepping the field is one undo step (across all `clips`).
    /// Dragging left or right on the row's name changes the value live, and the whole drag
    /// is one undo step.
    #[allow(clippy::too_many_arguments)]
    fn number_row(
        &self,
        grid: &gtk::Grid,
        row: i32,
        name: &str,
        clips: &[Clip],
        get: impl Fn(&Clip) -> f64,
        (min, max, step, digits): (f64, f64, f64, u32),
        make: MakeCommand,
    ) {
        let name_label = Self::row_label(grid, row, name);
        name_label.set_cursor_from_name(Some("ew-resize"));
        name_label.set_tooltip_text(Some("Drag left or right to change"));
        let first = round_to_digits(clips.first().map(&get).unwrap_or(0.0), digits);
        let mixed = is_mixed_f64(clips, &get);
        let ids: Rc<Vec<Uuid>> = Rc::new(clips.iter().map(|c| c.id).collect());
        let make = Rc::new(make);
        let dragging = Rc::new(Cell::new(false));
        let spin = gtk::SpinButton::builder()
            .adjustment(&gtk::Adjustment::new(first, min, max, step, step * 10.0, 0.0))
            .digits(digits)
            .numeric(true)
            .hexpand(true)
            .build();
        spin.update_property(&[gtk::accessible::Property::Label(name)]);
        let mixed_badge = mixed.then(|| mixed_badge_label("Selected clips have different values"));
        if let Some(badge) = &mixed_badge {
            let box_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            box_row.append(&spin);
            box_row.append(badge);
            grid.attach(&box_row, 1, row, 1, 1);
        } else {
            grid.attach(&spin, 1, row, 1, 1);
        }

        let state = self.state.clone();
        let busy = self.busy.clone();
        let (m, d, i_ids, badge_vc) = (make.clone(), dragging.clone(), ids.clone(), mixed_badge.clone());
        let name_str = name.to_string();
        spin.connect_value_changed(move |s| {
            // During a drag the value is previewed below, not recorded step by step.
            if !d.get() && Self::apply_clips(&state, &busy, &i_ids, &name_str, |current| m(s.value(), current)) {
                if let Some(b) = &badge_vc {
                    b.set_visible(false);
                }
            }
        });
        // If values were mixed and the user confirms the shown value with Enter, apply it to all clips.
        if let Some(badge_act) = &mixed_badge {
            let (state, busy, i_ids, m, badge_act, name_act) = (
                self.state.clone(),
                self.busy.clone(),
                ids.clone(),
                make.clone(),
                badge_act.clone(),
                name.to_string(),
            );
            spin.connect_activate(move |s| {
                if badge_act.is_visible() && Self::apply_clips(&state, &busy, &i_ids, &name_act, |current| m(s.value(), current)) {
                    badge_act.set_visible(false);
                }
            });
        }

        // Enter or Escape finishes typing and hands the keyboard back to the
        // editor, so Space plays and Delete deletes again without a click.
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_released(|controller, key, _, _| {
            if matches!(key, gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter | gtk::gdk::Key::Escape) {
                if let Some(widget) = controller.widget() {
                    unfocus_widget(&widget);
                }
            }
        });
        spin.add_controller(keys);

        let drag = gtk::GestureDrag::new();
        let start: Rc<RefCell<Option<(f64, Vec<Clip>)>>> = Rc::new(RefCell::new(None));
        let (state, busy, sp, st, dr, d_ids) = (self.state.clone(), self.busy.clone(), spin.clone(), start.clone(), dragging.clone(), ids.clone());
        drag.connect_drag_begin(move |_, _, _| {
            let originals = current_clips_for_edit(&state, &d_ids);
            if !originals.is_empty() {
                *st.borrow_mut() = Some((sp.value(), originals));
                dr.set(true);
                busy.set(true);
            }
        });
        let (state, sp, st, m, badge_du, name_du) = (self.state.clone(), spin.clone(), start.clone(), make.clone(), mixed_badge.clone(), name.to_string());
        drag.connect_drag_update(move |_, dx, _| {
            let borrow = st.borrow();
            let Some((from, originals)) = borrow.as_ref() else { return };
            // Four pixels per step: fine enough to aim, quick enough to travel.
            let value = (from + (dx / 4.0).round() * step).clamp(min, max);
            if (value - sp.value()).abs() < f64::EPSILON {
                return;
            }
            sp.set_value(value);
            if let Some(cmd) = build_batch_command(&name_du, originals, |orig| m(value, orig)) {
                state.apply_unlogged(cmd);
            }
            if let Some(b) = &badge_du {
                b.set_visible(false);
            }
        });
        let (state, busy, name, m_end) = (self.state.clone(), self.busy.clone(), name.to_string(), make);
        drag.connect_drag_end(move |_, _, _| {
            if let Some((_, originals)) = start.borrow_mut().take() {
                let final_val = spin.value();
                let changed = state
                    .with_timeline(|t| originals.iter().any(|orig| t.find_clip(orig.id).is_some_and(|(_, now)| now != orig)))
                    .unwrap_or(false);
                if changed {
                    {
                        let mut project = state.project.borrow_mut();
                        if let Some(p) = project.as_mut() {
                            for orig in &originals {
                                if let Some(slot) = p.timeline.find_clip_mut(orig.id) {
                                    *slot = orig.clone();
                                }
                            }
                        }
                    }
                    if let Some(cmd) = build_batch_command(&name, &originals, |orig| m_end(final_val, orig)) {
                        state.execute(cmd);
                    }
                }
            }
            dragging.set(false);
            busy.set(false);
        });
        name_label.add_controller(drag);
    }

    fn transform_section(&self, clips: &[Clip]) {
        let zoom = || property(|v, _| PropertyChange::Scale { x: v as f32, y: v as f32 });
        let pos_x = || property(|v, c| PropertyChange::Position { x: v as f32, y: c.properties.position_y });
        let pos_y = || property(|v, c| PropertyChange::Position { x: c.properties.position_x, y: v as f32 });
        let rotation = || property(|v, _| PropertyChange::Rotation(v as f32));
        let reset: ResetClipFn = Box::new(|c| {
            c.properties.scale_x = 1.0;
            c.properties.scale_y = 1.0;
            c.properties.position_x = 0.0;
            c.properties.position_y = 0.0;
            c.properties.rotation = 0.0;
        });
        let grid = self.section("Transform", clips, Some(("Reset Transform", reset)));
        self.number_row(&grid, 0, "Zoom", clips, |c| c.properties.scale_x as f64, (0.1, 10.0, 0.05, 2), zoom());
        self.number_row(&grid, 1, "Position X", clips, |c| c.properties.position_x as f64, (-8000.0, 8000.0, 10.0, 0), pos_x());
        self.number_row(&grid, 2, "Position Y", clips, |c| c.properties.position_y as f64, (-8000.0, 8000.0, 10.0, 0), pos_y());
        self.number_row(&grid, 3, "Rotation", clips, |c| c.properties.rotation as f64, (-360.0, 360.0, 1.0, 1), rotation());
    }

    fn composite_section(&self, clips: &[Clip]) {
        let opacity = || property(|v, _| PropertyChange::Opacity(v as f32 / 100.0));
        let reset: ResetClipFn = Box::new(|c| c.properties.opacity = 1.0);
        let grid = self.section("Composite", clips, Some(("Reset Composite", reset)));
        self.number_row(&grid, 0, "Opacity", clips, |c| c.properties.opacity as f64 * 100.0, (0.0, 100.0, 5.0, 0), opacity());
    }

    fn audio_section(&self, clips: &[Clip]) {
        let volume = || property(|v, _| PropertyChange::Volume(v as f32 / 100.0));
        let pan = || property(|v, _| PropertyChange::Pan(v as f32 / 100.0));
        let reset: ResetClipFn = Box::new(|c| {
            c.properties.volume = 1.0;
            c.properties.pan = 0.0;
        });
        let grid = self.section("Audio", clips, Some(("Reset Audio", reset)));
        self.number_row(&grid, 0, "Volume %", clips, |c| c.properties.volume as f64 * 100.0, (0.0, 200.0, 5.0, 0), volume());
        self.number_row(&grid, 1, "Pan", clips, |c| c.properties.pan as f64 * 100.0, (-100.0, 100.0, 10.0, 0), pan());
    }

    /// One section per filter on the clip: its settings and a button to take it off.
    fn filter_sections(&self, clip: &Clip) {
        let single = [clip.clone()];
        for (index, effect) in clip.properties.effects.iter().enumerate() {
            let reset: ResetClipFn = Box::new(move |c| {
                if let Some(eff) = c.properties.effects.get_mut(index) {
                    for p in &mut eff.params {
                        p.value = p.default;
                    }
                }
            });
            let grid = self.section(&effect.name, &single, Some(("Reset Filter", reset)));
            for (p, param) in effect.params.iter().enumerate() {
                let range = (param.min as f64, param.max as f64, ((param.max - param.min) as f64 / 100.0).max(0.01), 2);
                self.number_row(
                    &grid,
                    p as i32,
                    &param.name,
                    &single,
                    move |c| c.properties.effects.get(index).and_then(|e| e.params.get(p)).map_or(0.0, |pm| pm.value as f64),
                    range,
                    Self::filter_param(index, p),
                );
            }
            let remove = gtk::Button::builder().label("Remove Filter").halign(gtk::Align::End).build();
            let state = self.state.clone();
            let id = clip.id;
            let effect_id = effect.id.clone();
            remove.connect_clicked(move |_| {
                let Some(mut edited) = state.with_timeline(|t| t.find_clip(id).map(|(_, c)| c.clone())).flatten() else { return };
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

    /// Fade lengths in seconds, at the start and end of the clip(s).
    fn fade_section(&self, clips: &[Clip]) {
        let fade_in = || edit("Fade In", |v, c| c.properties.fade_in_us = ((v * 1e6) as i64).min(c.duration_us() / 2));
        let fade_out = || edit("Fade Out", |v, c| c.properties.fade_out_us = ((v * 1e6) as i64).min(c.duration_us() / 2));
        let reset: ResetClipFn = Box::new(|c| {
            c.properties.fade_in_us = 0;
            c.properties.fade_out_us = 0;
        });
        let grid = self.section("Fades", clips, Some(("Reset Fades", reset)));
        let max = clips.iter().map(|c| c.duration_us()).max().unwrap_or(0).max(0) as f64 / 2e6;
        self.number_row(&grid, 0, "Fade In s", clips, |c| c.properties.fade_in_us as f64 / 1e6, (0.0, max, 0.1, 1), fade_in());
        self.number_row(&grid, 1, "Fade Out s", clips, |c| c.properties.fade_out_us as f64 / 1e6, (0.0, max, 0.1, 1), fade_out());
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

    fn title_section(&self, clips: &[Clip]) {
        let titles: Vec<&TitleData> = clips.iter().filter_map(|c| c.title_data.as_ref()).collect();
        let Some(&first) = titles.first() else { return };
        let ids: Rc<Vec<Uuid>> = Rc::new(clips.iter().map(|c| c.id).collect());
        let text_mixed = titles.len() > 1 && titles[1..].iter().any(|t| t.text != first.text);
        let place_mixed = titles.len() > 1 && titles[1..].iter().any(|t| t.title_type != first.title_type);
        let font_mixed = titles.len() > 1 && titles[1..].iter().any(|t| t.font_family != first.font_family);
        let color_mixed = titles.len() > 1 && titles[1..].iter().any(|t| t.color != first.color);
        let bg_mixed = titles.len() > 1 && titles[1..].iter().any(|t| t.background_color != first.background_color);

        let grid = self.section("Title", clips, None);

        // Text: applied when Enter is pressed or the field loses focus.
        Self::row_label(&grid, 0, "Text");
        let text_entry = gtk::Entry::builder().hexpand(true).build();
        text_entry.update_property(&[gtk::accessible::Property::Label("Title text")]);
        {
            let (state, busy, ids) = (self.state.clone(), self.busy.clone(), ids.clone());
            bind_title_entry(&text_entry, first.text.clone(), text_mixed, move |text| {
                apply_title(&state, &busy, &ids, "Title Text", |edited, t| {
                    t.text = text.clone();
                    edited.name = text.chars().take(40).collect();
                })
            });
        }
        grid.attach(&text_entry, 1, 0, 1, 1);

        self.number_row(
            &grid,
            1,
            "Size",
            clips,
            |c| c.title_data.as_ref().map_or(72.0, |t| t.font_size as f64),
            (8.0, 400.0, 4.0, 0),
            edit("Title Size", |v, c| {
                if let Some(t) = c.title_data.as_mut() {
                    t.font_size = v as f32;
                }
            }),
        );

        Self::row_label(&grid, 2, "Place");
        let place = gtk::DropDown::from_strings(&["Centre", "Lower third"]);
        place.set_selected(place_selection(first.title_type, place_mixed));
        let place_badge = place_mixed.then(|| mixed_badge_label("Selected titles have different placement"));
        if let Some(badge) = &place_badge {
            let box_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            place.set_hexpand(true);
            box_row.append(&place);
            box_row.append(badge);
            grid.attach(&box_row, 1, 2, 1, 1);
        } else {
            grid.attach(&place, 1, 2, 1, 1);
        }
        {
            let (state, busy, ids, badge) = (self.state.clone(), self.busy.clone(), ids.clone(), place_badge);
            place.connect_selected_notify(move |d| {
                let kind = if d.selected() == 1 { TitleType::LowerThird } else { TitleType::CenterTitle };
                if apply_title(&state, &busy, &ids, "Title Place", |_, t| t.title_type = kind) {
                    if let Some(b) = &badge {
                        b.set_visible(false);
                    }
                }
            });
        }

        Self::row_label(&grid, 3, "Font");
        let font_entry = gtk::Entry::builder()
            .hexpand(true)
            .tooltip_text("Font family (e.g. Sans, Serif, Monospace)")
            .build();
        font_entry.update_property(&[gtk::accessible::Property::Label("Title font family")]);
        let trigger_font = {
            let (state, busy, ids) = (self.state.clone(), self.busy.clone(), ids.clone());
            bind_title_entry(&font_entry, first.font_family.clone(), font_mixed, move |raw| {
                let trimmed = raw.trim();
                let family = if trimmed.is_empty() { "Sans".to_string() } else { trimmed.to_string() };
                apply_title(&state, &busy, &ids, "Title Font", |_, t| t.font_family = family.clone())
            })
        };
        let preset_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .margin_top(4)
            .margin_bottom(4)
            .margin_start(4)
            .margin_end(4)
            .build();
        let preset_popover = gtk::Popover::builder().child(&preset_box).build();
        for preset in FONT_PRESETS {
            let btn = gtk::Button::builder().label(preset).css_classes(["tool", "text"]).build();
            let (pop, entry_w, trigger) = (preset_popover.downgrade(), font_entry.downgrade(), trigger_font.clone());
            btn.connect_clicked(move |_| {
                if let Some(p) = pop.upgrade() {
                    p.popdown();
                }
                if let Some(e) = entry_w.upgrade() {
                    e.set_text(preset);
                    trigger(&e);
                }
            });
            preset_box.append(&btn);
        }
        let preset_btn = gtk::MenuButton::builder()
            .icon_name("view-more-symbolic")
            .tooltip_text("Font presets")
            .popover(&preset_popover)
            .css_classes(["tool"])
            .build();
        let font_row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        font_row.append(&font_entry);
        font_row.append(&preset_btn);
        grid.attach(&font_row, 1, 3, 1, 1);

        Self::row_label(&grid, 4, "Color");
        let color_badge = color_mixed.then(|| mixed_badge_label("Selected titles have different colors"));
        let apply_color = {
            let (state, busy, ids, badge) = (self.state.clone(), self.busy.clone(), ids.clone(), color_badge.clone());
            let last_color = Cell::new(first.color);
            let is_mixed = Cell::new(color_mixed);
            move |next: [u8; 4]| {
                if !is_mixed.get() && last_color.get() == next {
                    return;
                }
                if apply_title(&state, &busy, &ids, "Title Color", |_, t| t.color = next) {
                    last_color.set(next);
                    is_mixed.set(false);
                    if let Some(badge) = &badge {
                        badge.set_visible(false);
                    }
                }
            }
        };
        let color_btn: gtk::Widget = if color_mixed {
            mixed_color_button(first.color, "Title color", apply_color).upcast()
        } else {
            let btn = gtk::ColorDialogButton::new(Some(gtk::ColorDialog::new()));
            btn.set_rgba(&rgba_from_bytes(first.color));
            btn.update_property(&[gtk::accessible::Property::Label("Title color")]);
            btn.connect_rgba_notify(move |b| apply_color(bytes_from_rgba(&b.rgba())));
            btn.upcast()
        };
        if let Some(badge) = &color_badge {
            let row_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            row_box.append(&color_btn);
            row_box.append(badge);
            grid.attach(&row_box, 1, 4, 1, 1);
        } else {
            grid.attach(&color_btn, 1, 4, 1, 1);
        }

        Self::row_label(&grid, 5, "Background");
        let bg_toggle = gtk::CheckButton::builder()
            .active(first.background_color.is_some())
            // Some titles have a box and some do not: show neither state.
            .inconsistent(titles[1..].iter().any(|t| t.background_color.is_some() != first.background_color.is_some()))
            .tooltip_text("Show background box (uncheck for transparent)")
            .build();
        bg_toggle.update_property(&[gtk::accessible::Property::Label("Title background box")]);
        let bg_badge = bg_mixed.then(|| mixed_badge_label("Selected titles have different backgrounds"));
        let bg_color = Rc::new(Cell::new(first.background_color.unwrap_or(DEFAULT_TITLE_BG)));
        let apply_bg: Rc<dyn Fn(Option<[u8; 4]>)> = {
            let (state, busy, ids, badge) = (self.state.clone(), self.busy.clone(), ids, bg_badge.clone());
            let last_bg = Cell::new(first.background_color);
            let is_mixed = Cell::new(bg_mixed);
            Rc::new(move |next| {
                if !is_mixed.get() && last_bg.get() == next {
                    return;
                }
                if apply_title(&state, &busy, &ids, "Title Background", |_, t| t.background_color = next) {
                    last_bg.set(next);
                    is_mixed.set(false);
                    if let Some(badge) = &badge {
                        badge.set_visible(false);
                    }
                }
            })
        };
        let pick_bg = {
            let (apply, color, toggle) = (apply_bg.clone(), bg_color.clone(), bg_toggle.clone());
            move |c: [u8; 4]| {
                color.set(c);
                if toggle.is_active() {
                    // Every title gets a box now, so the "values differ" dash is stale.
                    toggle.set_inconsistent(false);
                    apply(Some(c));
                }
            }
        };
        let bg_btn: gtk::Widget = if bg_mixed && first.background_color.is_some() {
            mixed_color_button(bg_color.get(), "Title background color", pick_bg).upcast()
        } else {
            let btn = gtk::ColorDialogButton::new(Some(gtk::ColorDialog::new()));
            btn.set_rgba(&rgba_from_bytes(bg_color.get()));
            btn.update_property(&[gtk::accessible::Property::Label("Title background color")]);
            btn.connect_rgba_notify(move |b| pick_bg(bytes_from_rgba(&b.rgba())));
            btn.upcast()
        };
        bg_btn.set_sensitive(first.background_color.is_some());
        let bg_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        bg_row.append(&bg_toggle);
        bg_row.append(&bg_btn);
        if let Some(badge) = &bg_badge {
            bg_row.append(badge);
        }
        grid.attach(&bg_row, 1, 5, 1, 1);
        bg_toggle.connect_toggled(move |chk| {
            chk.set_inconsistent(false);
            let on = chk.is_active();
            bg_btn.set_sensitive(on);
            apply_bg(on.then(|| bg_color.get()));
        });
    }
}

fn apply_title(state: &Rc<AppState>, busy: &Cell<bool>, ids: &[Uuid], label: &'static str, change: impl Fn(&mut Clip, &mut TitleData)) -> bool {
    Inspector::apply_clips(state, busy, ids, label, |current| {
        let mut edited = current.clone();
        if let Some(mut t) = edited.title_data.take() {
            change(&mut edited, &mut t);
            edited.title_data = Some(t);
        }
        Box::new(EditClipCommand::new(label, edited))
    })
}

/// Wire Enter, Escape, and focus-leave on a title `GtkEntry`; returns a trigger
/// callback that commits the entry's current text if needed.
fn bind_title_entry(entry: &gtk::Entry, initial: String, mixed: bool, commit: impl Fn(String) -> bool + 'static) -> Rc<dyn Fn(&gtk::Entry)> {
    if mixed {
        entry.set_placeholder_text(Some("Mixed"));
    } else {
        entry.set_text(&initial);
    }
    let last = Rc::new(RefCell::new(if mixed { None } else { Some(initial) }));
    let dirty = Rc::new(Cell::new(false));
    {
        let dirty = dirty.clone();
        entry.connect_changed(move |_| dirty.set(true));
    }
    let commit = Rc::new(commit);
    let apply_now: Rc<dyn Fn(&gtk::Entry)> = Rc::new({
        let (last, dirty, commit) = (last.clone(), dirty.clone(), commit);
        move |e: &gtk::Entry| {
            let text = e.text().to_string();
            if !title_entry_has_edit(last.borrow().as_deref(), dirty.get(), &text) {
                dirty.set(false);
                return;
            }
            let prev_last = last.borrow().clone();
            *last.borrow_mut() = Some(text.clone());
            dirty.set(false);
            if commit(text) {
                e.set_placeholder_text(None);
            } else {
                *last.borrow_mut() = prev_last;
            }
        }
    });
    {
        let apply = apply_now.clone();
        entry.connect_activate(move |e| {
            apply(e);
            unfocus_widget(e.upcast_ref());
        });
    }
    {
        let (last, dirty, entry_w) = (last.clone(), dirty.clone(), entry.downgrade());
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                if let Some(e) = entry_w.upgrade() {
                    if let Some(prev) = last.borrow().as_ref() {
                        e.set_text(prev);
                    } else {
                        e.set_text("");
                    }
                    dirty.set(false);
                    unfocus_widget(e.upcast_ref());
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        entry.add_controller(keys);
    }
    {
        let focus = gtk::EventControllerFocus::new();
        let (e, apply) = (entry.clone(), apply_now.clone());
        focus.connect_leave(move |_| apply(&e));
        entry.add_controller(focus);
    }
    apply_now
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

/// Where a key value lands on the graph: the mix runs from Start Ratio to End Ratio.
fn to_level(v: f32, (start, end): (f32, f32)) -> f32 {
    start + (end - start) * v
}

/// The key value that sits at graph `level`; `fallback` when the ratios are equal (the
/// level then says nothing about the value).
fn from_level(level: f32, (start, end): (f32, f32), fallback: f32) -> f32 {
    if (end - start).abs() < 1e-4 { fallback } else { ((level - start) / (end - start)).clamp(0.0, 1.0) }
}

/// The curve as drawn: `steps + 1` (place, level) samples of what playback mixes.
fn sample_drawn_curve(t: &Transition, steps: usize) -> Vec<(f32, f32)> {
    (0..=steps).map(|i| i as f32 / steps as f32).map(|u| (u, t.progress_at(u))).collect()
}

thread_local! {
    /// The selected curve key per transition, kept across the panel rebuild an edit causes.
    static KEPT_KEY: Cell<Option<(TransitionRef, usize)>> = const { Cell::new(None) };
}

/// The kept key for `sel` if it still exists among `len` keys.
fn kept_selection<R: PartialEq>(kept: Option<(R, usize)>, sel: &R, len: usize) -> Option<usize> {
    kept.filter(|(r, i)| r == sel && *i < len).map(|(_, i)| i)
}

/// The key nearest to the pixel, if within reach.
fn hit_key(keys: &[[f32; 2]], levels: (f32, f32), x: f64, y: f64, w: f64, h: f64) -> Option<usize> {
    let dist = |k: &[f32; 2]| {
        let (kx, ky) = graph_point(k[0], to_level(k[1], levels), w, h);
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
    let selected: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(kept_selection(KEPT_KEY.get(), &sel, t.curve.len())));
    let levels = (t.start_ratio, t.end_ratio);
    let select = {
        let selected = selected.clone();
        move |i: Option<usize>| {
            selected.set(i);
            KEPT_KEY.set(i.map(|i| (sel, i)));
        }
    };
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
        // Faint guides at Start and End Ratio.
        cr.set_dash(&[3.0, 3.0], 0.0);
        for ratio in [shape.start_ratio, shape.end_ratio] {
            let (_, y) = graph_point(0.0, ratio, w, h);
            cr.move_to(x0, y);
            cr.line_to(x1, y);
            let _ = cr.stroke();
        }
        cr.set_dash(&[], 0.0);
        if let Some(u) = TransitionView::read(&s, sel).and_then(|v| playhead_fraction(s.playhead_us(), v.span?)) {
            paint("tempo_accent");
            let (px, _) = graph_point(u, 0.0, w, h);
            cr.move_to(px.round() + 0.5, y0);
            cr.line_to(px.round() + 0.5, y1);
            let _ = cr.stroke();
        }
        paint("tempo_text");
        cr.set_line_width(2.0);
        for (i, (u, level)) in sample_drawn_curve(&shape, 64).into_iter().enumerate() {
            let (x, y) = graph_point(u, level, w, h);
            if i == 0 { cr.move_to(x, y) } else { cr.line_to(x, y) }
        }
        let _ = cr.stroke();
        for (i, k) in shape.curve.iter().enumerate() {
            let (x, y) = graph_point(k[0], to_level(k[1], (shape.start_ratio, shape.end_ratio)), w, h);
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
    let (sh, mv, select_c, a) = (shape.clone(), moving.clone(), select.clone(), area.clone());
    drag.connect_drag_begin(move |_, x, y| {
        let (w, h) = size(&a);
        let hit = hit_key(&sh.borrow().curve, levels, x, y, w, h);
        mv.set(hit.map(|i| (i, x, y)));
        select_c(hit);
        a.grab_focus();
        a.queue_draw();
    });
    let (sh, mv, a) = (shape.clone(), moving.clone(), area.clone());
    drag.connect_drag_update(move |_, dx, dy| {
        let Some((i, x, y)) = mv.get() else { return };
        let (w, h) = size(&a);
        let (u, level) = graph_value(x + dx, y + dy, w, h);
        let old = sh.borrow().curve.get(i).map_or(0.0, |k| k[1]);
        let moved = move_key(&sh.borrow().curve, i, u, from_level(level, levels, old));
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
    let (sh, a, commit_add, select_a) = (shape.clone(), area.clone(), commit.clone(), select.clone());
    click.connect_pressed(move |_, presses, x, y| {
        let (w, h) = size(&a);
        let shape = sh.borrow();
        if presses != 2 || hit_key(&shape.curve, levels, x, y, w, h).is_some() {
            return;
        }
        let (u, _) = graph_value(x, y, w, h);
        if let Some(keys) = add_key(&shape.curve, u, shape.curve_at(u)) {
            drop(shape);
            select_a(None);
            commit_add(keys);
        }
    });
    area.add_controller(click);

    // Right-click on a key removes it.
    let right = gtk::GestureClick::builder().button(3).build();
    let (sh, a, commit_remove, select_r) = (shape.clone(), area.clone(), commit.clone(), select.clone());
    right.connect_pressed(move |_, _, x, y| {
        let (w, h) = size(&a);
        let hit = hit_key(&sh.borrow().curve, levels, x, y, w, h);
        if let Some(i) = hit {
            select_r(None);
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
            select(None);
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
    fn a_number_field_starts_at_the_value_it_shows_so_leaving_it_writes_nothing() {
        let opacity = f64::from(0.7f32) * 100.0;
        assert_ne!(opacity, 70.0);
        assert_eq!(round_to_digits(opacity, 0), 70.0);
        assert_eq!(round_to_digits(f64::from(1.1f32), 2), 1.1);
        assert_eq!(round_to_digits(0.625, 1), 0.6);
        assert_eq!(round_to_digits(50.0, 0), 50.0);
    }

    #[test]
    fn an_empty_mixed_title_entry_commits_nothing() {
        assert!(!title_entry_has_edit(None, true, ""));
        assert!(!title_entry_has_edit(None, false, "Hi"));
        assert!(title_entry_has_edit(None, true, "Hi"));
        assert!(title_entry_has_edit(Some("Old"), true, ""));
        assert!(!title_entry_has_edit(Some("Old"), true, "Old"));
    }

    #[test]
    fn a_mixed_place_dropdown_shows_no_choice_so_any_pick_applies() {
        assert_eq!(place_selection(TitleType::CenterTitle, true), gtk::INVALID_LIST_POSITION);
        assert_eq!(place_selection(TitleType::LowerThird, true), gtk::INVALID_LIST_POSITION);
        assert_eq!(place_selection(TitleType::LowerThird, false), 1);
        assert_eq!(place_selection(TitleType::CenterTitle, false), 0);
    }

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
    fn drawn_curve_runs_from_start_ratio_to_end_ratio() {
        let t = Transition { start_ratio: 0.2, end_ratio: 0.6, ..Transition::new(Uuid::nil(), TransitionEdge::In, TransitionKind::CrossDissolve, 1, TransitionAlignment::Centered) };
        let drawn = sample_drawn_curve(&t, 4);
        assert_eq!(drawn.len(), 5);
        assert!((drawn[0].1 - 0.2).abs() < 1e-6 && (drawn[2].1 - 0.4).abs() < 1e-6 && (drawn[4].1 - 0.6).abs() < 1e-6);
    }

    #[test]
    fn key_levels_round_trip_through_the_ratios_and_flat_ratios_keep_the_value() {
        assert!((to_level(0.5, (0.2, 0.6)) - 0.4).abs() < 1e-6);
        assert!((from_level(0.4, (0.2, 0.6), 0.9) - 0.5).abs() < 1e-6);
        assert_eq!(from_level(0.4, (0.3, 0.3), 0.9), 0.9);
        assert_eq!(from_level(0.0, (0.2, 0.6), 0.9), 0.0);
    }

    #[test]
    fn hit_key_finds_a_key_drawn_at_its_ratio_level() {
        let keys = [[0.5, 0.8]];
        let (x, y) = graph_point(0.5, 0.4, 220.0, 110.0);
        assert_eq!(hit_key(&keys, (0.0, 0.5), x, y, 220.0, 110.0), Some(0));
        assert_eq!(hit_key(&keys, (0.0, 1.0), x, y, 220.0, 110.0), None);
    }

    #[test]
    fn selection_is_kept_for_the_same_transition_only_and_while_the_key_exists() {
        assert_eq!(kept_selection(Some((1, 2)), &1, 3), Some(2));
        assert_eq!(kept_selection(Some((1, 2)), &9, 3), None);
        assert_eq!(kept_selection(Some((1, 2)), &1, 2), None);
        assert_eq!(kept_selection::<i32>(None, &1, 3), None);
    }

    #[test]
    fn a_moved_key_keeps_its_index_so_the_kept_selection_still_points_at_it() {
        let keys = [[0.2, 0.3], [0.8, 0.7]];
        let moved = move_key(&keys, 0, 0.5, 0.9);
        assert_eq!(kept_selection(Some((7, 0)), &7, moved.len()), Some(0));
        assert_eq!(moved[0], [0.5, 0.9]);
    }

    #[test]
    fn hit_key_picks_the_nearest_key_within_reach() {
        let keys = [[0.25, 0.5], [0.75, 0.5]];
        let (x, y) = graph_point(0.75, 0.5, 220.0, 110.0);
        assert_eq!(hit_key(&keys, (0.0, 1.0), x + 3.0, y - 2.0, 220.0, 110.0), Some(1));
        assert_eq!(hit_key(&keys, (0.0, 1.0), x + 30.0, y, 220.0, 110.0), None);
        assert_eq!(hit_key(&[], (0.0, 1.0), x, y, 220.0, 110.0), None);
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

    #[test]
    fn rgba_byte_conversion_round_trips() {
        for bytes in [[0, 0, 0, 0], [255, 255, 255, 255], [12, 128, 200, 170]] {
            let rgba = rgba_from_bytes(bytes);
            assert_eq!(bytes_from_rgba(&rgba), bytes);
        }
    }

    #[test]
    fn mixed_detection_distinguishes_equal_and_differing_clips() {
        let track_id = Uuid::new_v4();
        let mut a = Clip::new(track_id, Uuid::nil(), ClipType::Video, "A", 0, 2_000_000, 0, 2_000_000);
        let b = Clip::new(track_id, Uuid::nil(), ClipType::Video, "B", 2_000_000, 4_000_000, 0, 2_000_000);
        assert!(!is_mixed_f64(&[a.clone(), b.clone()], |c| c.properties.opacity as f64));
        a.properties.opacity = 0.5;
        assert!(is_mixed_f64(&[a, b], |c| c.properties.opacity as f64));
        assert!(!is_mixed_f64(&[], |c| c.properties.opacity as f64));
    }

    #[test]
    fn batch_command_updates_and_undoes_all_selected_clips_atomically() {
        use tempo_timeline::{CommandLog, Timeline};
        let mut tl = Timeline::new_default();
        let v1 = tl.tracks[0].id;
        let a1 = tl.tracks[2].id;
        let mut vc1 = Clip::new(v1, Uuid::nil(), ClipType::Video, "V1a", 0, 2_000_000, 0, 2_000_000);
        vc1.properties.opacity = 0.4;
        let vc2 = Clip::new(v1, Uuid::nil(), ClipType::Video, "V1b", 2_000_000, 4_000_000, 0, 2_000_000);
        let ac1 = Clip::new(a1, Uuid::nil(), ClipType::Audio, "A1a", 0, 2_000_000, 0, 2_000_000);
        let (vc1_id, vc2_id, ac1_id) = (vc1.id, vc2.id, ac1.id);
        tl.tracks[0].clips.push(vc1.clone());
        tl.tracks[0].clips.push(vc2.clone());
        tl.tracks[2].clips.push(ac1.clone());

        let mut log = CommandLog::new();
        let cmd = build_batch_command("Opacity", &[vc1, vc2], |c| {
            Box::new(SetClipPropertyCommand::new(c.id, PropertyChange::Opacity(0.75)))
        })
        .expect("batch command built");
        log.execute(cmd, &mut tl).expect("batch execute succeeds");

        assert!((tl.find_clip(vc1_id).expect("vc1").1.properties.opacity - 0.75).abs() < 1e-6);
        assert!((tl.find_clip(vc2_id).expect("vc2").1.properties.opacity - 0.75).abs() < 1e-6);
        assert!((tl.find_clip(ac1_id).expect("ac1").1.properties.volume - 1.0).abs() < 1e-6);

        log.undo(&mut tl).expect("single undo restores both clips");
        assert!((tl.find_clip(vc1_id).expect("vc1").1.properties.opacity - 0.4).abs() < 1e-6);
        assert!((tl.find_clip(vc2_id).expect("vc2").1.properties.opacity - 1.0).abs() < 1e-6);

        log.redo(&mut tl).expect("single redo restores batch edit");
        assert!((tl.find_clip(vc1_id).expect("vc1").1.properties.opacity - 0.75).abs() < 1e-6);
        assert!((tl.find_clip(vc2_id).expect("vc2").1.properties.opacity - 0.75).abs() < 1e-6);
    }

    #[test]
    fn title_data_font_and_color_edits_undo_and_redo() {
        use tempo_timeline::{CommandLog, Timeline};
        let mut tl = Timeline::new_default();
        let v1 = tl.tracks[0].id;
        let mut title_clip = Clip::new(v1, Uuid::nil(), ClipType::Title, "Title", 0, 5_000_000, 0, 5_000_000);
        title_clip.title_data = Some(TitleData::default());
        let id = title_clip.id;
        tl.tracks[0].clips.push(title_clip.clone());

        let mut log = CommandLog::new();
        let mut edited = title_clip;
        if let Some(t) = edited.title_data.as_mut() {
            t.font_family = "Monospace".into();
            t.color = [255, 128, 0, 255];
            t.background_color = Some([10, 20, 30, 170]);
        }
        log.execute(Box::new(EditClipCommand::new("Title Style", edited)), &mut tl).expect("execute title edit");

        let updated = tl.find_clip(id).expect("title clip").1.title_data.as_ref().expect("title data");
        assert_eq!(updated.font_family, "Monospace");
        assert_eq!(updated.color, [255, 128, 0, 255]);
        assert_eq!(updated.background_color, Some([10, 20, 30, 170]));

        log.undo(&mut tl).expect("undo title edit");
        let reverted = tl.find_clip(id).expect("title clip").1.title_data.as_ref().expect("title data");
        assert_eq!(reverted.font_family, "Sans");
        assert_eq!(reverted.color, [255, 255, 255, 255]);
        assert_eq!(reverted.background_color, None);
    }
}
