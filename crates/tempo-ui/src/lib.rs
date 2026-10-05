//! tempo-ui: the GTK 4 interface. See docs/VISUAL_DESIGN.md and docs/UI_SPEC.md.

pub mod actions;
pub mod app;
pub mod dialogs;
pub mod edit_page;
pub mod export_page;
pub mod inspector;
pub mod keybinds;
pub mod media_pool;
pub mod player;
pub mod project_manager;
pub mod proxies;
pub mod settings;
pub mod state;
pub mod timeline;
pub mod util;
pub mod video_surface;
pub mod viewer;
pub mod waveforms;
pub mod window;

pub use app::TempoApp;
