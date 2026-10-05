use crate::host_api::{register_host_functions, HostState};
use crate::manifest::PluginManifest;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tempo_timeline::command::CommandLog;
use tempo_timeline::Timeline;
use thiserror::Error;
use tracing::{error, info};
use wasmtime::{Config, Engine, Instance, Linker, Module, Store, Trap};

#[derive(Debug, Error)]
pub enum PluginError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Wasmtime error: {0}")]
    Wasmtime(#[from] wasmtime::Error),
    #[error("Plugin trapped: {0}")]
    Trapped(String),
    #[error("Plugin memory limit exceeded")]
    MemoryLimitExceeded,
    #[error("Plugin initialization returned non-zero error: {0}")]
    InitFailed(i32),
    #[error("Plugin not found: {0}")]
    NotFound(String),
    #[error("Manifest error: {0}")]
    Manifest(String),
}

pub struct ActivePlugin {
    pub manifest: Option<PluginManifest>,
    pub store: Store<HostState>,
    pub instance: Instance,
    pub is_enabled: bool,
}

pub struct PluginManager {
    engine: Engine,
    linker: Linker<HostState>,
    plugins: HashMap<String, ActivePlugin>,
    timeline: Option<Arc<Mutex<Timeline>>>,
    command_log: Option<Arc<Mutex<CommandLog>>>,
    registered_effects: Arc<Mutex<HashMap<String, String>>>,
    logs: Arc<Mutex<Vec<(i32, String)>>>,
}

impl PluginManager {
    pub fn new() -> Result<Self, PluginError> {
        let mut config = Config::new();
        config.wasm_component_model(false);
        config.consume_fuel(false);

        let engine = Engine::new(&config)?;
        let mut linker = Linker::new(&engine);
        register_host_functions(&mut linker)?;

        Ok(Self {
            engine,
            linker,
            plugins: HashMap::new(),
            timeline: None,
            command_log: None,
            registered_effects: Arc::new(Mutex::new(HashMap::new())),
            logs: Arc::new(Mutex::new(Vec::new())),
        })
    }

    pub fn set_timeline(&mut self, timeline: Arc<Mutex<Timeline>>, command_log: Arc<Mutex<CommandLog>>) {
        self.timeline = Some(timeline);
        self.command_log = Some(command_log);
    }

    pub fn registered_effects(&self) -> Arc<Mutex<HashMap<String, String>>> {
        self.registered_effects.clone()
    }

    pub fn logs(&self) -> Arc<Mutex<Vec<(i32, String)>>> {
        self.logs.clone()
    }

    pub fn load_wasm_bytes(&mut self, id: &str, wasm_bytes: &[u8]) -> Result<(), PluginError> {
        let module = Module::new(&self.engine, wasm_bytes)?;

        let host_state = HostState {
            timeline: self.timeline.clone(),
            command_log: self.command_log.clone(),
            playhead_us: 0,
            registered_effects: self.registered_effects.clone(),
            logs: self.logs.clone(),
            notifications: Arc::new(Mutex::new(Vec::new())),
            limits: wasmtime::StoreLimitsBuilder::new()
                .memory_size(256 * 1024 * 1024)
                .build(),
        };

        let mut store = Store::new(&self.engine, host_state);
        store.limiter(|state| &mut state.limits);

        let instance = match self.linker.instantiate(&mut store, &module) {
            Ok(inst) => inst,
            Err(e) => {
                if let Some(trap) = e.downcast_ref::<Trap>() {
                    return Err(PluginError::Trapped(trap.to_string()));
                }
                return Err(PluginError::Wasmtime(e));
            }
        };

        self.plugins.insert(
            id.to_string(),
            ActivePlugin {
                manifest: None,
                store,
                instance,
                is_enabled: false,
            },
        );

        Ok(())
    }

    pub fn load_plugin_file<P: AsRef<Path>>(&mut self, id: &str, wasm_path: P) -> Result<(), PluginError> {
        let bytes = std::fs::read(wasm_path)?;
        self.load_wasm_bytes(id, &bytes)
    }

    pub fn enable(&mut self, id: &str) -> Result<(), PluginError> {
        let plugin = self
            .plugins
            .get_mut(id)
            .ok_or_else(|| PluginError::NotFound(id.to_string()))?;

        if let Ok(init_func) = plugin.instance.get_typed_func::<(), i32>(&mut plugin.store, "plugin_init") {
            let res = init_func.call(&mut plugin.store, ());
            match res {
                Ok(code) => {
                    if code != 0 {
                        return Err(PluginError::InitFailed(code));
                    }
                }
                Err(e) => {
                    error!("Plugin '{}' trapped during plugin_init: {:#}", id, e);
                    return Err(PluginError::Trapped(format!("{:#}", e)));
                }
            }
        }

        plugin.is_enabled = true;
        info!("Plugin '{}' enabled successfully", id);
        Ok(())
    }

    pub fn disable(&mut self, id: &str) -> Result<(), PluginError> {
        let plugin = self
            .plugins
            .get_mut(id)
            .ok_or_else(|| PluginError::NotFound(id.to_string()))?;

        if let Ok(shutdown_func) = plugin.instance.get_typed_func::<(), ()>(&mut plugin.store, "plugin_shutdown") {
            let _ = shutdown_func.call(&mut plugin.store, ());
        }

        plugin.is_enabled = false;
        info!("Plugin '{}' disabled", id);
        Ok(())
    }

    pub fn is_enabled(&self, id: &str) -> bool {
        self.plugins.get(id).map_or(false, |p| p.is_enabled)
    }

    pub fn unload(&mut self, id: &str) {
        if let Ok(()) = self.disable(id) {
            self.plugins.remove(id);
        }
    }
}
