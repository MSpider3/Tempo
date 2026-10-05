//! Plugins in the interface: the list, the menu entries, and running a command.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gtk4::gio;
use tempo_plugin::{Plugin, RunInput};
use tempo_timeline::ReplaceTimelineCommand;

use crate::state::AppState;

pub struct Plugins {
    state: Rc<AppState>,
    pub list: RefCell<Vec<Plugin>>,
    /// Called after the list or the on/off switches change.
    changed: RefCell<Option<Box<dyn Fn()>>>,
}

/// Where the user's own plugins live.
pub fn user_dir() -> PathBuf {
    glib::user_data_dir().join("tempo").join("plugins")
}

/// Copy a plugin folder into the user's plugin folder. Blocking. Returns the plugin's id.
fn install_folder(from: &Path) -> Result<String, String> {
    let plugin = Plugin::load(from).map_err(|e| e.to_string())?;
    // The folder is named after the id, with anything unusual replaced.
    let folder: String = plugin.id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' }).collect();
    let target = user_dir().join(folder);
    std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
    for name in ["plugin.toml", "main.lua"] {
        std::fs::copy(from.join(name), target.join(name)).map_err(|e| e.to_string())?;
    }
    Ok(plugin.id)
}

impl Plugins {
    pub fn new(state: &Rc<AppState>) -> Rc<Self> {
        Rc::new(Self { state: state.clone(), list: RefCell::new(tempo_plugin::built_in()), changed: RefCell::new(None) })
    }

    pub fn connect_changed(&self, f: impl Fn() + 'static) {
        *self.changed.borrow_mut() = Some(Box::new(f));
    }

    fn notify(&self) {
        if let Some(f) = self.changed.borrow().as_ref() {
            f();
        }
    }

    pub fn is_enabled(&self, id: &str) -> bool {
        !self.state.settings.borrow().plugins_off.iter().any(|p| p == id)
    }

    pub fn set_enabled(self: &Rc<Self>, id: &str, on: bool) {
        {
            let mut settings = self.state.settings.borrow_mut();
            settings.plugins_off.retain(|p| p != id);
            if !on {
                settings.plugins_off.push(id.to_string());
            }
        }
        let snapshot = self.state.settings.borrow().clone();
        glib::MainContext::default().spawn_local(async move {
            let _ = gio::spawn_blocking(move || snapshot.save()).await;
        });
        self.notify();
    }

    /// Read the user's plugin folder again, on a worker.
    pub fn reload(self: &Rc<Self>) {
        let plugins = self.clone();
        glib::MainContext::default().spawn_local(async move {
            let found = gio::spawn_blocking(|| tempo_plugin::discover(&user_dir())).await.unwrap_or_default();
            let mut list = tempo_plugin::built_in();
            // A user plugin cannot take the id of a built-in one.
            list.extend(found.into_iter().filter(|p| !tempo_plugin::built_in().iter().any(|b| b.id == p.id)));
            *plugins.list.borrow_mut() = list;
            plugins.notify();
        });
    }

    /// Install a plugin from a folder the user picked. It starts switched off,
    /// so the user sees what it asks for before it can run.
    pub fn install(self: &Rc<Self>, folder: PathBuf) {
        let plugins = self.clone();
        glib::MainContext::default().spawn_local(async move {
            match gio::spawn_blocking(move || install_folder(&folder)).await {
                Ok(Ok(id)) => {
                    plugins.set_enabled(&id, false);
                    plugins.reload();
                    plugins.state.message("Plugin installed. Switch it on to use it.");
                }
                Ok(Err(e)) => plugins.state.message(format!("Could not install the plugin: {e}")),
                Err(_) => plugins.state.message("Could not install the plugin."),
            }
        });
    }

    /// Menu entries: (label, "plugin-id::command-id") for every command of every enabled plugin.
    pub fn commands(&self) -> Vec<(String, String)> {
        self.list
            .borrow()
            .iter()
            .filter(|p| self.is_enabled(&p.id))
            .flat_map(|p| p.commands.iter().map(move |c| (c.name.clone(), format!("{}::{}", p.id, c.id))))
            .collect()
    }

    /// Run a command on a worker against a copy of the timeline, then apply the
    /// result as a single undo step.
    pub fn run(self: &Rc<Self>, target: &str) {
        let Some((plugin_id, command_id)) = target.split_once("::") else { return };
        let found = self.list.borrow().iter().find(|p| p.id == plugin_id && self.is_enabled(&p.id)).and_then(|p| {
            p.commands.iter().find(|c| c.id == command_id).map(|c| (p.clone(), c.function.clone(), c.name.clone()))
        });
        let Some((plugin, function, name)) = found else { return };
        let Some(timeline) = self.state.with_timeline(|t| t.clone()) else { return };
        let loudness: HashMap<_, _> = self.state.waveforms.borrow().iter().map(|(id, peaks)| (*id, Arc::new(peaks.as_ref().clone()))).collect();
        let input = RunInput { timeline, playhead_us: self.state.player.position_us(), selection: self.state.selected_ids(), loudness };

        let plugins = self.clone();
        glib::MainContext::default().spawn_local(async move {
            let result = gio::spawn_blocking(move || tempo_plugin::run_command(&plugin, &function, input)).await;
            let state = &plugins.state;
            match result {
                Ok(Ok(output)) => {
                    if output.changed {
                        state.execute(Box::new(ReplaceTimelineCommand::new(name, output.timeline)));
                    }
                    for message in output.messages.into_iter().take(3) {
                        state.message(message);
                    }
                }
                Ok(Err(e)) => state.message(format!("{name} failed: {e}")),
                Err(_) => state.message(format!("{name} stopped unexpectedly.")),
            }
        });
    }
}
