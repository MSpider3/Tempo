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
    cancel: RefCell<Arc<AtomicBool>>,
    activity: RefCell<Option<Box<dyn Fn(Option<String>)>>>,
}

impl Proxies {
    pub fn new(state: &Rc<AppState>) -> Rc<Self> {
        let proxies = Rc::new(Self {
            state: state.clone(),
            seen: Default::default(),
            running: Cell::new(false),
            cancel: RefCell::new(Arc::new(AtomicBool::new(false))),
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
            Change::Media => p.start_next(),
            _ => {}
        });
        proxies
    }

    /// Called with a one-line status while a proxy is being made, `None` when idle.
    pub fn connect_activity(&self, f: impl Fn(Option<String>) + 'static) {
        *self.activity.borrow_mut() = Some(Box::new(f));
    }

    fn report(&self, text: Option<String>) {
        if let Some(f) = self.activity.borrow().as_ref() {
            f(text);
        }
    }

    fn start_next(self: &Rc<Self>) {
        if self.running.get() {
            return;
        }
        let next = self
            .state
            .with_project(|p| {
                p.sources
                    .values()
                    .filter(|s| !s.proxy_ready && tempo_proxy::needs_proxy(s) && !self.seen.borrow().contains(&s.id))
                    .min_by_key(|s| s.import_order)
                    .map(|s| {
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
            p.report(Some(format!("Making proxy: {label} {}%", shown.load(Ordering::Relaxed))));
            glib::ControlFlow::Continue
        });

        let p = self.clone();
        glib::MainContext::default().spawn_local(async move {
            let out = output.clone();
            let flag = cancel.clone();
            let result = gio::spawn_blocking(move || {
                // A proxy left by an earlier session is reused as it is.
                if out.exists() {
                    return Ok(());
                }
                tempo_proxy::generate_proxy(&input, &out, duration, &flag, |f| progress.store((f * 100.0) as u32, Ordering::Relaxed))
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
