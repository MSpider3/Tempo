//! Beginner helpers: searchable "How Do I…?" task guide, auto-tracking First-Edit
//! Checklist, and per-panel `?` mini-guides referencing DaVinci Resolve equivalents.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use gtk4 as gtk;
use gtk4::gio;
use gtk4::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;

use crate::state::{AppState, Change};
use crate::util::label;

#[derive(Clone, Copy)]
pub struct TaskGuide {
    pub title: &'static str,
    pub resolve_equiv: &'static str,
    pub summary: &'static str,
    pub steps: &'static [&'static str],
    pub shortcuts: &'static [(&'static str, &'static str)],
}

pub const GUIDES: &[TaskGuide] = &[
    TaskGuide {
        title: "Import & Organize in Bins",
        resolve_equiv: "Media Pool Bins (Ctrl+I, Ctrl+Shift+N)",
        summary: "Bring video, audio, and images into your project without slow re-encoding, and keep them tidy in Bins.",
        steps: &[
            "Press Ctrl+I (or click + in Media Pool) to import clips, or drag files/folders straight in.",
            "Right-click empty space in Media Pool -> New Bin… (or Ctrl+Shift+N) to make folders like Clips or Timelines.",
            "Drag clips or timelines onto a Bin row, double-click a Bin to open it, and use the red bar on thumbnails to see what is already on your timeline.",
        ],
        shortcuts: &[("Ctrl+I", "Import Media"), ("Ctrl+Shift+N", "New Bin"), ("Ctrl+N", "New Timeline")],
    },
    TaskGuide {
        title: "Cut Out Mistakes",
        resolve_equiv: "Split Clip (Ctrl+\\) & Blade Mode (B)",
        summary: "Play to the mistake, slice before and after it, and remove the bad take.",
        steps: &[
            "Press Space (or J / K / L to shuttle) until the red playhead reaches the start of the mistake.",
            "Press Ctrl+\\ to split the clip right under the playhead (works even without selecting first), or press B for the Blade tool.",
            "Repeat at the end of the mistake, click the unwanted piece, and press BackSpace (leaves space) or Shift+Backspace / Delete (closes gap).",
        ],
        shortcuts: &[("Ctrl+\\", "Split Clip"), ("B", "Blade Mode"), ("Ctrl+Z", "Undo")],
    },
    TaskGuide {
        title: "Close Gaps",
        resolve_equiv: "Ripple Delete Gap & Ripple Trim",
        summary: "Snap later clips left so there are no unintended black pauses between scenes.",
        steps: &[
            "Click any empty gap between clips on a track to highlight it, then press Delete or BackSpace to close it.",
            "When deleting a clip itself, use Shift+Backspace or Delete to ripple everything after it left automatically.",
            "To chop the start or end of a clip right up to the playhead and pull later clips along, use Ctrl+Shift+[ or Ctrl+Shift+].",
        ],
        shortcuts: &[("Delete / Shift+Backspace", "Ripple Delete"), ("Ctrl+Shift+[", "Ripple Start to Playhead"), ("Ctrl+Shift+]", "Ripple End to Playhead")],
    },
    TaskGuide {
        title: "Add Transitions to Every Cut",
        resolve_equiv: "Select Edit (V), Ctrl+T, and Copy/Paste Transitions",
        summary: "Smooth out hard cuts with DaVinci Resolve's Cross Dissolve and Audio Crossfade.",
        steps: &[
            "Open Effects -> Video Transitions -> Dissolve (or Audio Transitions -> Crossfade) and drag onto a cut or clip edge.",
            "Or move near any cut, press V to highlight the nearest edit point, and press Ctrl+T (or Alt+T video-only / Shift+T audio-only).",
            "To copy one transition to every edit: select the transition, press Ctrl+C, tap Up/Down to jump between cuts, and press Ctrl+V.",
        ],
        shortcuts: &[("V", "Select Edit Point"), ("Ctrl+T", "Add Transition"), ("Ctrl+C / Ctrl+V", "Copy & Paste Transition")],
    },
    TaskGuide {
        title: "Add Titles & Markers",
        resolve_equiv: "Titles in Effects & Timeline Markers (M)",
        summary: "Drop on-screen text overlays and mark chapters before exporting to YouTube.",
        steps: &[
            "Open Effects -> Titles and click or drag Text / Lower Third onto your timeline at the playhead.",
            "Select the title clip and open Inspector (top-right) to customize Text, Size, Place, Font, Color, and Background box.",
            "Press M while playing to drop a marker; press M again (or Ctrl+M / double-click) on that spot to name it as a YouTube chapter.",
        ],
        shortcuts: &[("M", "Add / Re-edit Marker"), ("Ctrl+M", "Add & Edit Marker"), ("Shift+Up / Down", "Jump Markers")],
    },
    TaskGuide {
        title: "Export for YouTube",
        resolve_equiv: "Deliver Page (Shift+8), Presets & Render Queue",
        summary: "Render a crisp 1080p/4K MP4 with normalized YouTube loudness and chapter markers.",
        steps: &[
            "Press Shift+8 (or click Export at the bottom) and pick 16:9 YouTube 1080p (or 16:9 4K, or Custom for MKV/MOV/H.265).",
            "Optional: set Normalize -> YouTube (-14 LUFS) for consistent streaming volume and keep Chapters from markers checked.",
            "Click Add to Render Queue (Ctrl+Return). Click a single job card for Render 1, or click empty space for Render All.",
        ],
        shortcuts: &[("Shift+8", "Export Page"), ("Ctrl+Return", "Add to Render Queue"), ("Ctrl+Shift+Return", "Render All")],
    },
    TaskGuide {
        title: "Play, Shuttle & Inspect Faster",
        resolve_equiv: "J-K-L Shuttle (up to 64×) & Multi-Clip Inspector",
        summary: "Review long recordings fast on any laptop and tweak multiple clips at once.",
        steps: &[
            "Tap L repeatedly to speed forward (1× up to 64×), J to reverse, and K to stop. Press Shift+F for full-page viewer.",
            "Press Z to fit the picture or Ctrl+/ to toggle Loop playback around your In/Out range.",
            "Ctrl+click several clips on the timeline to batch-edit Opacity, Scale, Volume, or Fades together in the Inspector with one Ctrl+Z undo.",
        ],
        shortcuts: &[("J / K / L", "Shuttle up to 64×"), ("Shift+F", "Full-Page Viewer"), ("Ctrl+/", "Toggle Loop")],
    },
];

/// Case-insensitive filter across task title, Resolve equivalent, summary, steps, and shortcuts.
pub fn filter_guides(query: &str) -> Vec<&'static TaskGuide> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return GUIDES.iter().collect();
    }
    let terms: Vec<&str> = q.split_whitespace().collect();
    GUIDES
        .iter()
        .filter(|g| {
            terms.iter().all(|term| {
                g.title.to_lowercase().contains(term)
                    || g.resolve_equiv.to_lowercase().contains(term)
                    || g.summary.to_lowercase().contains(term)
                    || g.steps.iter().any(|s| s.to_lowercase().contains(term))
                    || g.shortcuts.iter().any(|(k, d)| k.to_lowercase().contains(term) || d.to_lowercase().contains(term))
            })
        })
        .collect()
}

pub fn show_how_do_i(
    parent: &impl IsA<gtk::Widget>,
    on_show_checklist: impl Fn() + 'static,
    on_show_shortcuts: impl Fn() + 'static,
) {
    let search = gtk::SearchEntry::builder()
        .placeholder_text("Search tasks or shortcuts (cut, gap, bin, title, youtube)…")
        .hexpand(true)
        .build();
    search.update_property(&[gtk::accessible::Property::Label("Search beginner guides")]);

    let cards = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .margin_top(6)
        .margin_bottom(6)
        .build();
    let scroller = gtk::ScrolledWindow::builder()
        .child(&cards)
        .vexpand(true)
        .min_content_height(360)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();

    let rebuild = {
        let cards = cards.clone();
        move |query: &str| {
            while let Some(c) = cards.first_child() {
                cards.remove(&c);
            }
            let matches = filter_guides(query);
            if matches.is_empty() {
                let empty = label("No matching guides — try 'cut', 'gap', 'bin', 'transition', 'title', or 'youtube'.", &["tempo-dim"]);
                empty.set_wrap(true);
                empty.set_margin_top(24);
                cards.append(&empty);
                return;
            }
            for guide in matches {
                cards.append(&guide_card(guide));
            }
        }
    };
    rebuild("");

    let rebuild_search = rebuild.clone();
    search.connect_search_changed(move |entry| {
        rebuild_search(entry.text().as_str());
    });

    let checklist_btn = gtk::Button::builder()
        .label("Show First-Edit Checklist")
        .css_classes(["pill-outline"])
        .build();
    let shortcuts_btn = gtk::Button::builder()
        .label("All Shortcuts (F1)")
        .css_classes(["pill-outline"])
        .build();
    let footer = gtk::Box::builder().spacing(8).halign(gtk::Align::End).build();
    footer.append(&checklist_btn);
    footer.append(&shortcuts_btn);

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .margin_top(12)
        .margin_bottom(14)
        .margin_start(16)
        .margin_end(16)
        .build();
    content.append(&search);
    content.append(&scroller);
    content.append(&footer);

    let header = adw::HeaderBar::builder().show_title(true).build();
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&content));

    let dialog = adw::Dialog::builder()
        .title("How Do I…?")
        .content_width(600)
        .content_height(560)
        .child(&view)
        .build();

    let d = dialog.clone();
    search.connect_stop_search(move |_| {
        d.close();
    });
    let d = dialog.clone();
    let on_checklist = Rc::new(on_show_checklist);
    checklist_btn.connect_clicked(move |_| {
        d.close();
        on_checklist();
    });
    let d = dialog.clone();
    let on_shortcuts = Rc::new(on_show_shortcuts);
    shortcuts_btn.connect_clicked(move |_| {
        d.close();
        on_shortcuts();
    });

    dialog.present(Some(parent));
    search.grab_focus();
}

fn guide_card(guide: &TaskGuide) -> gtk::Box {
    let card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .css_classes(["help-card"])
        .build();

    let head = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(8).build();
    let title = label(guide.title, &["tempo-heading", "tempo-bright"]);
    title.set_xalign(0.0);
    title.set_hexpand(true);
    let resolve = label(guide.resolve_equiv, &["tempo-small", "tempo-dim"]);
    resolve.set_xalign(1.0);
    head.append(&title);
    head.append(&resolve);
    card.append(&head);

    let summary = label(guide.summary, &["tempo-small", "tempo-dim"]);
    summary.set_xalign(0.0);
    summary.set_wrap(true);
    card.append(&summary);

    for (idx, step) in guide.steps.iter().enumerate() {
        let row = label(&format!("{}. {step}", idx + 1), &["tempo-small"]);
        row.set_xalign(0.0);
        row.set_wrap(true);
        card.append(&row);
    }

    let keys_row = gtk::Box::builder().spacing(6).margin_top(4).build();
    for (key, desc) in guide.shortcuts {
        let chip = gtk::Box::builder().spacing(4).css_classes(["help-shortcut-pill"]).build();
        chip.append(&label(key, &["tempo-small", "tempo-bright"]));
        chip.append(&label(desc, &["tempo-small", "tempo-dim"]));
        keys_row.append(&chip);
    }
    card.append(&keys_row);
    card
}

/// Tiny `?` popover button shared by Media Pool, Timeline, Inspector, and Export headers.
pub fn panel_guide_button(
    panel_name: &str,
    resolve_equiv: &str,
    summary: &str,
    tips: &[(&str, &str)],
) -> gtk::MenuButton {
    let box_ui = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .width_request(280)
        .margin_top(10)
        .margin_bottom(10)
        .margin_start(12)
        .margin_end(12)
        .build();

    let title = label(panel_name, &["tempo-heading", "tempo-bright"]);
    title.set_xalign(0.0);
    let equiv = label(&format!("DaVinci Resolve: {resolve_equiv}"), &["tempo-small", "tempo-dim"]);
    equiv.set_xalign(0.0);
    equiv.set_wrap(true);
    let body = label(summary, &["tempo-small"]);
    body.set_xalign(0.0);
    body.set_wrap(true);

    box_ui.append(&title);
    box_ui.append(&equiv);
    box_ui.append(&body);

    for (k, d) in tips {
        let line = gtk::Box::builder().spacing(6).build();
        let kl = label(k, &["tempo-small", "tempo-bright", "help-shortcut-pill"]);
        let dl = label(d, &["tempo-small", "tempo-dim"]);
        dl.set_xalign(0.0);
        dl.set_wrap(true);
        line.append(&kl);
        line.append(&dl);
        box_ui.append(&line);
    }

    let pop = gtk::Popover::builder().child(&box_ui).build();
    let btn = gtk::MenuButton::builder()
        .label("?")
        .popover(&pop)
        .tooltip_text(format!("{panel_name} guide"))
        .css_classes(["tool", "panel-help-btn"])
        .build();
    btn.update_property(&[gtk::accessible::Property::Label(&format!("{panel_name} guide"))]);
    btn
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChecklistStatus {
    pub imported: bool,
    pub placed: bool,
    pub split: bool,
    pub transition: bool,
    pub queued: bool,
}

impl ChecklistStatus {
    pub fn completed_count(self) -> usize {
        [self.imported, self.placed, self.split, self.transition, self.queued]
            .into_iter()
            .filter(|v| *v)
            .count()
    }

    pub fn for_state(state: &AppState) -> Self {
        let (imported, placed, transition) = state
            .with_project(|p| {
                let imported = !p.sources.is_empty();
                let placed = p.timeline.tracks.iter().any(|t| !t.clips.is_empty());
                let transition = p.timeline.tracks.iter().any(|t| !t.transitions.is_empty());
                (imported, placed, transition)
            })
            .unwrap_or((false, false, false));
        Self {
            imported,
            placed,
            split: state.checklist_split.get(),
            transition,
            queued: state.checklist_queued.get(),
        }
    }
}

pub fn checklist_dismissed_marker() -> PathBuf {
    glib::user_config_dir().join("tempo").join("checklist-dismissed")
}

/// Marker for a step the project itself cannot show ("split", "queued").
fn checklist_step_marker(step: &str) -> PathBuf {
    glib::user_config_dir().join("tempo").join(format!("checklist-{step}"))
}

/// Blocking: call from a worker.
fn write_marker(path: PathBuf) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, b"1\n");
}

/// Tick a step the project cannot show and remember it for this user, like the dismissed flag.
pub fn tick_step(done: &Cell<bool>, step: &'static str) {
    if !done.replace(true) {
        glib::MainContext::default().spawn_local(async move {
            let _ = gio::spawn_blocking(move || write_marker(checklist_step_marker(step))).await;
        });
    }
}

/// Top-bar popover checklist that auto-ticks steps as the user edits in the real app.
pub struct FirstEditChecklist {
    pub button: gtk::MenuButton,
    button_label: gtk::Label,
    state: Rc<AppState>,
    rows: RefCell<Vec<(gtk::Label, gtk::Label)>>,
    dismissed: Cell<bool>,
}

impl FirstEditChecklist {
    pub fn new(state: &Rc<AppState>) -> Rc<Self> {
        let button_label = label("First Edit 0/5", &["tempo-small"]);
        let pop_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .width_request(300)
            .margin_top(10)
            .margin_bottom(10)
            .margin_start(12)
            .margin_end(12)
            .build();
        let heading = label("First-Edit Checklist", &["tempo-heading", "tempo-bright"]);
        heading.set_xalign(0.0);
        let subtitle = label("Complete these 5 Resolve steps to finish your first cut:", &["tempo-small", "tempo-dim"]);
        subtitle.set_xalign(0.0);
        subtitle.set_wrap(true);
        pop_box.append(&heading);
        pop_box.append(&subtitle);

        let steps = [
            ("Import a clip", "Ctrl+I or drag to Media Pool"),
            ("Drag a clip to the timeline", "Drop onto V1 / A1"),
            ("Split a clip", "Ctrl+\\ or Blade (B)"),
            ("Add a transition", "V + Ctrl+T or drag Dissolve"),
            ("Queue an export", "Shift+8 -> Add to Render Queue"),
        ];
        let mut rows = Vec::with_capacity(steps.len());
        for (title_text, hint_text) in steps {
            let row = gtk::Box::builder().spacing(8).build();
            let mark = label("○", &["tempo-small", "tempo-dim"]);
            mark.set_valign(gtk::Align::Start);
            let col = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(1).hexpand(true).build();
            let t = label(title_text, &["tempo-small"]);
            t.set_xalign(0.0);
            let h = label(hint_text, &["tempo-small", "tempo-dim"]);
            h.set_xalign(0.0);
            col.append(&t);
            col.append(&h);
            row.append(&mark);
            row.append(&col);
            pop_box.append(&row);
            rows.push((mark, t));
        }

        let dismiss_btn = gtk::Button::builder()
            .label("Dismiss checklist")
            .halign(gtk::Align::End)
            .css_classes(["link-text"])
            .build();
        pop_box.append(&dismiss_btn);

        let popover = gtk::Popover::builder().child(&pop_box).build();
        let button = gtk::MenuButton::builder()
            .child(&button_label)
            .popover(&popover)
            .tooltip_text("First-Edit Checklist — tracks your first 5 editing steps")
            .css_classes(["tool", "checklist-pill"])
            .build();
        button.update_property(&[gtk::accessible::Property::Label("First-Edit Checklist")]);

        let checklist = Rc::new(Self {
            button,
            button_label,
            state: state.clone(),
            rows: RefCell::new(rows),
            dismissed: Cell::new(false),
        });

        let c = checklist.clone();
        dismiss_btn.connect_clicked(move |_| {
            c.dismiss();
        });

        let c = checklist.clone();
        state.connect(move |change| {
            if matches!(change, Change::Project | Change::Media | Change::Timeline | Change::Options) {
                c.refresh();
            }
        });

        let c = checklist.clone();
        glib::MainContext::default().spawn_local(async move {
            let (dismissed, split, queued) = gio::spawn_blocking(|| {
                (
                    checklist_dismissed_marker().exists(),
                    checklist_step_marker("split").exists(),
                    checklist_step_marker("queued").exists(),
                )
            })
            .await
            .unwrap_or((false, false, false));
            if split || queued {
                c.state.checklist_split.set(c.state.checklist_split.get() || split);
                c.state.checklist_queued.set(c.state.checklist_queued.get() || queued);
                c.refresh();
            }
            if dismissed {
                c.dismissed.set(true);
                c.button.set_visible(false);
            }
        });

        checklist.refresh();
        checklist
    }

    pub fn refresh(&self) {
        let st = ChecklistStatus::for_state(&self.state);
        let flags = [st.imported, st.placed, st.split, st.transition, st.queued];
        let done = st.completed_count();
        self.button_label.set_text(&if done == flags.len() {
            format!("First Edit {done}/5 ✓")
        } else {
            format!("First Edit {done}/5")
        });
        if done == flags.len() {
            self.button.add_css_class("checklist-done");
        } else {
            self.button.remove_css_class("checklist-done");
        }
        for ((mark, title_lbl), is_done) in self.rows.borrow().iter().zip(flags) {
            if is_done {
                mark.set_text("✓");
                mark.remove_css_class("tempo-dim");
                mark.add_css_class("checklist-done");
                title_lbl.add_css_class("tempo-bright");
            } else {
                mark.set_text("○");
                mark.remove_css_class("checklist-done");
                mark.add_css_class("tempo-dim");
                title_lbl.remove_css_class("tempo-bright");
            }
        }
    }

    pub fn dismiss(&self) {
        self.dismissed.set(true);
        if let Some(p) = self.button.popover() {
            p.popdown();
        }
        self.button.set_visible(false);
        glib::MainContext::default().spawn_local(async move {
            let _ = gio::spawn_blocking(|| write_marker(checklist_dismissed_marker())).await;
        });
    }

    pub fn show(&self) {
        self.dismissed.set(false);
        self.button.set_visible(true);
        self.refresh();
        glib::MainContext::default().spawn_local(async move {
            let _ = gio::spawn_blocking(|| {
                let _ = std::fs::remove_file(checklist_dismissed_marker());
            })
            .await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempo_timeline::{
        Clip, ClipType, MediaSource, MediaType, Project, RationalFps, Transition,
        TransitionAlignment, TransitionEdge, TransitionKind,
    };

    #[test]
    fn opening_another_project_keeps_the_split_and_queue_ticks() {
        // Arrange
        let state = AppState::new();
        state.checklist_split.set(true);
        state.checklist_queued.set(true);

        // Act
        state.set_project(Some(Project::new("t", 1920, 1080, RationalFps::FPS_30)), None);

        // Assert
        let st = ChecklistStatus::for_state(&state);
        assert!(st.split && st.queued);
    }

    #[test]
    fn guide_search_filters_by_title_steps_and_shortcuts() {
        assert_eq!(filter_guides("").len(), GUIDES.len());
        let yt = filter_guides("youtube");
        assert!(yt.iter().any(|g| g.title == "Export for YouTube"));
        let bin = filter_guides("Ctrl+Shift+N");
        assert_eq!(bin.len(), 1);
        assert_eq!(bin[0].title, "Import & Organize in Bins");
        assert!(filter_guides("zzznonexistent").is_empty());
    }

    #[test]
    fn checklist_progress_tracks_all_five_steps() {
        let mut p = Project::new("Test", 1920, 1080, RationalFps::FPS_30);
        let state = AppState::new();
        state.set_project(Some(p.clone()), None);
        assert_eq!(ChecklistStatus::for_state(&state).completed_count(), 0);

        let src = MediaSource::new(std::path::PathBuf::from("/tmp/a.mp4"), MediaType::Video, 4_000_000);
        let src_id = src.id;
        p.sources.insert(src_id, src);
        let v1 = p.timeline.tracks[0].id;
        let clip = Clip::new(v1, src_id, ClipType::Video, "a", 0, 2_000_000, 0, 2_000_000);
        let clip_id = clip.id;
        p.timeline.tracks[0].clips.push(clip);
        p.timeline.tracks[0].transitions.push(Transition::new(
            clip_id,
            TransitionEdge::In,
            TransitionKind::CrossDissolve,
            1_000_000,
            TransitionAlignment::StartAtCut,
        ));
        state.set_project(Some(p), None);
        state.checklist_split.set(true);
        state.checklist_queued.set(true);

        let st = ChecklistStatus::for_state(&state);
        assert_eq!(st.completed_count(), 5);
        assert!(st.imported && st.placed && st.split && st.transition && st.queued);
    }
}
