//! Waveform data for audio clips: worked out once per file in the background
//! and kept on disk, so reopening a project costs nothing.

use std::cell::Cell;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::AtomicBool;

use gtk4::gio;

use crate::state::{AppState, Change};

/// Must match the drawing code in the timeline.
const PEAKS_PER_SECOND: u32 = 50;

pub struct Waveforms {
    state: Rc<AppState>,
    running: Cell<bool>,
}

/// Cache file for a media file. The name changes if the file's size or date does.
fn cache_path(media: &Path) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    media.hash(&mut hasher);
    if let Ok(meta) = std::fs::metadata(media) {
        meta.len().hash(&mut hasher);
        meta.modified().ok().hash(&mut hasher);
    }
    glib::user_cache_dir().join("tempo").join("waveforms").join(format!("{:016x}.peaks", hasher.finish()))
}

/// Load from the cache, or decode the file and fill the cache. Blocking.
fn load_or_compute(media: &Path) -> Option<Vec<u8>> {
    let cache = cache_path(media);
    if let Ok(bytes) = std::fs::read(&cache) {
        return Some(bytes);
    }
    let peaks = tempo_media::waveform_peaks(media, PEAKS_PER_SECOND, &AtomicBool::new(false)).ok()?;
    if let Some(dir) = cache.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&cache, &peaks);
    Some(peaks)
}

impl Waveforms {
    pub fn new(state: &Rc<AppState>) -> Rc<Self> {
        let waveforms = Rc::new(Self { state: state.clone(), running: Cell::new(false) });
        let w = waveforms.clone();
        state.connect(move |change| {
            if matches!(change, Change::Project | Change::Media) {
                w.start_next();
            }
        });
        waveforms
    }

    /// One file at a time, so this never competes with itself for the disk.
    fn start_next(self: &Rc<Self>) {
        if self.running.get() {
            return;
        }
        let next = self
            .state
            .with_project(|p| {
                let known = self.state.waveforms.borrow();
                p.sources
                    .values()
                    .filter(|s| s.audio_channels.is_some() && !s.is_missing && !known.contains_key(&s.id))
                    .min_by_key(|s| s.import_order)
                    .map(|s| (s.id, s.path.clone()))
            })
            .flatten();
        let Some((id, path)) = next else { return };
        self.running.set(true);
        let w = self.clone();
        glib::MainContext::default().spawn_local(async move {
            let peaks = gio::spawn_blocking(move || load_or_compute(&path)).await.ok().flatten();
            w.running.set(false);
            // An empty entry records a failure, so the file is not tried again and again.
            let still_open = w.state.with_project(|p| p.sources.contains_key(&id)).unwrap_or(false);
            if still_open {
                w.state.waveforms.borrow_mut().insert(id, Rc::new(peaks.unwrap_or_default()));
                w.state.emit(Change::Waveform);
            }
            w.start_next();
        });
    }
}
