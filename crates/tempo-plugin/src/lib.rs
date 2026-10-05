//! tempo-plugin: WASM plugin sandbox and runtime using wasmtime.

pub mod host_api;
pub mod lua_engine;
pub mod manager;
pub mod manifest;
pub mod memory;

pub use host_api::{register_host_functions, HostState};
pub use lua_engine::LuaEngine;
pub use manager::{ActivePlugin, PluginError, PluginManager};
pub use manifest::{
    EffectDeclaration, FormatDeclaration, ManifestError, PanelDeclaration, PluginInfo,
    PluginManifest, PluginPermissions, PluginType,
};
pub use memory::{
    read_wasm_string, write_wasm_bytes, WasmClipInfo, WasmMediaInfo,
};
