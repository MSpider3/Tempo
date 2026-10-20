//! Background proxies for heavy footage. One at a time, lowest priority.
//! Editing never waits for a proxy: the original plays until it is ready.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use gtk4::gio;
use uuid::Uuid;

use crate::state::{AppState, Change};

pub struct Proxies {
    state: Rc<AppState>,
    /// Sources already tried in this session (done, failed or running).
    seen: RefCell<HashSet<Uuid>>,
    running: Cell<bool>,
    busy: Cell<bool>,
    cancel: RefCell<Arc<AtomicBool>>,
    /// Set while the editor is playing or exporting: proxy work stands still.
    hold: Arc<AtomicBool>,
    activity: RefCell<Option<Box<dyn Fn(Option<String>)>>>,
}

impl Proxies {
    pub fn new(state: &Rc<AppState>) -> Rc<Self> {
        let proxies = Rc::new(Self {
            state: state.clone(),
            seen: Default::default(),
            running: Cell::new(false),
            busy: Cell::new(false),
            cancel: RefCell::new(Arc::new(AtomicBool::new(false))),
            hold: Arc::new(AtomicBool::new(false)),
            activity: RefCell::new(None),
        });
        let p = proxies.clone();
        state.connect(move |change| match change {
            Change::Project => {
                // A different project: stop work that belongs to the old one.
                p.cancel.borrow().store(true, Ordering::Relaxed);
                p.seen.borrow_mut().clear();
                p.start_next();
            }
            // Proxies are made only for footage on the timeline, so placing a clip may start one.
            Change::Media | Change::Timeline => p.start_next(),
            _ => {}
        });
        proxies
    }

    /// Called with a one-line status while a proxy is being made, `None` when idle.
    pub fn connect_activity(&self, f: impl Fn(Option<String>) + 'static) {
        *self.activity.borrow_mut() = Some(Box::new(f));
    }

    /// Hold proxy work (for example while an export runs). Playback holds it by itself.
    pub fn set_busy(&self, busy: bool) {
        self.busy.set(busy);
    }

    fn report(&self, text: Option<String>) {
        if let Some(f) = self.activity.borrow().as_ref() {
            f(text);
        }
    }

    fn start_next(self: &Rc<Self>) {
        // `TEMPO_NO_PROXY=1` turns proxies off, to measure playback of original files.
        let off = !self.state.settings.borrow().auto_proxies || std::env::var_os("TEMPO_NO_PROXY").is_some();
        if self.running.get() || off {
            return;
        }
        let next = self
            .state
            .with_project(|p| {
                next_proxy(p, &self.seen.borrow()).and_then(|id| p.sources.get(&id)).map(|s| {
                        let name = s.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    (s.id, s.path.clone(), p.proxy_dir.join(format!("{}.mp4", s.id)), s.duration_us, name)
                })
            })
            .flatten();
        let Some((id, input, output, duration, name)) = next else {
            self.report(None);
            return;
        };
        self.seen.borrow_mut().insert(id);
        self.running.set(true);

        let cancel = Arc::new(AtomicBool::new(false));
        *self.cancel.borrow_mut() = cancel.clone();
        let progress = Arc::new(AtomicU32::new(0));

        // Show progress a couple of times a second while this proxy is made.
        let p = self.clone();
        let shown = progress.clone();
        let label = name.clone();
        glib::timeout_add_local(Duration::from_millis(500), move || {
            if !p.running.get() {
                return glib::ControlFlow::Break;
            }
            // Playback and export come first; the proxy carries on afterwards.
            let hold = p.busy.get() || p.state.player.is_playing();
            p.hold.store(hold, Ordering::Relaxed);
            let percent = shown.load(Ordering::Relaxed);
            p.report(Some(if hold { format!("Proxy paused: {label} {percent}%") } else { format!("Making proxy: {label} {percent}%") }));
            glib::ControlFlow::Continue
        });

        let p = self.clone();
        glib::MainContext::default().spawn_local(async move {
            let out = output.clone();
            let flag = cancel.clone();
            let hold = p.hold.clone();
            let result = gio::spawn_blocking(move || {
                // A proxy left by an earlier session is reused as it is.
                if out.exists() {
                    return Ok(());
                }
                tempo_proxy::generate_proxy(&input, &out, duration, &flag, &hold, |f| progress.store((f * 100.0) as u32, Ordering::Relaxed))
            })
            .await;
            p.running.set(false);
            match result {
                Ok(Ok(())) if !cancel.load(Ordering::Relaxed) => {
                    let updated = p
                        .state
                        .project
                        .borrow_mut()
                        .as_mut()
                        .and_then(|proj| proj.sources.get_mut(&id))
                        .map(|s| {
                            s.proxy_path = Some(output);
                            s.proxy_ready = true;
                        })
                        .is_some();
                    if updated {
                        p.state.sync_player();
                        p.state.set_dirty(true);
                        p.state.emit(Change::Media);
                    }
                }
                Ok(Err(e)) if e != "Cancelled" => tracing::warn!("no proxy for {name}: {e}"),
                _ => {}
            }
            p.start_next();
        });
    }
}

/// The next source to make a proxy for: heavy footage that is on the timeline, has no
/// proxy yet and was not tried this session. The first imported goes first. Footage
/// that is only in the Media Pool is never transcoded.
fn next_proxy(project: &tempo_timeline::Project, seen: &HashSet<Uuid>) -> Option<Uuid> {
    let used: HashSet<Uuid> = project.timeline.tracks.iter().flat_map(|t| t.clips.iter().map(|c| c.source_id)).collect();
    project
        .sources
        .values()
        .filter(|s| used.contains(&s.id) && !s.proxy_ready && tempo_proxy::needs_proxy(s) && !seen.contains(&s.id))
        .min_by_key(|s| s.import_order)
        .map(|s| s.id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempo_timeline::{Clip, ClipType, MediaSource, MediaType, Project, RationalFps};

    fn heavy(order: u32) -> MediaSource {
        let mut s = MediaSource::new(format!("{order}.mp4").into(), MediaType::Video, 1_000_000);
        s.video_height = Some(2160);
        s.video_codec = Some("h264".into());
        s.import_order = order;
        s
    }

    #[test]
    fn only_footage_on_the_timeline_gets_a_proxy() {
        let mut project = Project::new("p", 1920, 1080, RationalFps::FPS_30);
        let (pool_only, placed) = (heavy(0), heavy(1));
        let track = project.timeline.tracks[0].id;
        project.timeline.tracks[0].clips.push(Clip::new(track, placed.id, ClipType::Video, "c", 0, 1_000_000, 0, 1_000_000));
        let placed_id = placed.id;
        project.sources.insert(pool_only.id, pool_only);
        project.sources.insert(placed.id, placed);
        assert_eq!(next_proxy(&project, &HashSet::new()), Some(placed_id));
        assert_eq!(next_proxy(&project, &HashSet::from([placed_id])), None);
    }
}
