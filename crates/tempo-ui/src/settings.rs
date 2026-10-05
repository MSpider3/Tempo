//! User settings, kept in `~/.config/tempo/settings.json`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Seconds between autosaves.
    pub autosave_seconds: u32,
    /// Height the preview is decoded at; 0 is full size.
    pub playback_height: u32,
    /// Make small proxy files for heavy footage in the background.
    pub auto_proxies: bool,
    /// Use the graphics chip to decode video when it can. Off by default until
    /// it has been tried on more machines.
    pub hardware_decode: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { autosave_seconds: 120, playback_height: 540, auto_proxies: true, hardware_decode: false }
    }
}

fn path() -> PathBuf {
    glib::user_config_dir().join("tempo").join("settings.json")
}

impl Settings {
    /// Blocking: call from a worker. Missing or unreadable files give the defaults.
    pub fn load() -> Self {
        std::fs::read_to_string(path()).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
    }

    /// Blocking: call from a worker.
    pub fn save(&self) -> std::io::Result<()> {
        let path = path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self).unwrap_or_default())
    }
}

/// Folders Tempo fills with files it can make again: waveforms and proxies.
pub fn cache_dir() -> PathBuf {
    glib::user_cache_dir().join("tempo")
}

/// Total size of the cache in bytes. Blocking.
pub fn cache_size() -> u64 {
    fn size(dir: &std::path::Path) -> u64 {
        std::fs::read_dir(dir)
            .map(|entries| {
                entries
                    .flatten()
                    .map(|e| match e.metadata() {
                        Ok(m) if m.is_dir() => size(&e.path()),
                        Ok(m) => m.len(),
                        Err(_) => 0,
                    })
                    .sum()
            })
            .unwrap_or(0)
    }
    size(&cache_dir())
}

#[cfg(test)]
mod tests {
    use super::Settings;

    #[test]
    fn unknown_or_missing_fields_fall_back_to_defaults() {
        let s: Settings = serde_json::from_str(r#"{"autosave_seconds": 300, "some_future_field": 1}"#).unwrap();
        assert_eq!(s.autosave_seconds, 300);
        assert_eq!(s.playback_height, 540);
        assert!(s.auto_proxies);
    }
}
