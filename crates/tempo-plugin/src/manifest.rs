use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("Failed to read manifest file: {0}")]
    Io(#[from] std::io::Error),
    #[error("Failed to parse TOML manifest: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("Missing required field: {0}")]
    MissingField(&'static str),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginManifest {
    pub plugin: PluginInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub min_tempo: Option<String>,
    #[serde(default = "default_wasm_file")]
    pub wasm_file: String,
    #[serde(default)]
    pub r#type: PluginType,
    #[serde(default)]
    pub permissions: PluginPermissions,
    #[serde(default)]
    pub effects: Vec<EffectDeclaration>,
    #[serde(default)]
    pub panels: Vec<PanelDeclaration>,
    #[serde(default)]
    pub formats: Vec<FormatDeclaration>,
}

fn default_wasm_file() -> String {
    "plugin.wasm".to_string()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct PluginType {
    #[serde(default)]
    pub effect: bool,
    #[serde(default)]
    pub ui_panel: bool,
    #[serde(default)]
    pub io_format: bool,
    #[serde(default)]
    pub compute: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginPermissions {
    #[serde(default = "default_timeline_perm")]
    pub timeline: String, // "none" | "read-only" | "read-write"
    #[serde(default = "default_fs_perm")]
    pub filesystem: String, // "none" | "project-dir" | "user-selected" | "unrestricted"
    #[serde(default)]
    pub network: bool,
    #[serde(default)]
    pub compute: bool,
}

impl Default for PluginPermissions {
    fn default() -> Self {
        Self {
            timeline: default_timeline_perm(),
            filesystem: default_fs_perm(),
            network: false,
            compute: false,
        }
    }
}

fn default_timeline_perm() -> String {
    "read-only".to_string()
}

fn default_fs_perm() -> String {
    "none".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EffectDeclaration {
    pub id: String,
    pub name: String,
    pub r#type: String, // "transition" | "filter"
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub thumbnail: Option<String>,
    #[serde(default)]
    pub wgsl_file: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PanelDeclaration {
    pub id: String,
    pub title: String,
    #[serde(default = "default_panel_position")]
    pub position: String, // "left" | "right" | "bottom"
    #[serde(default)]
    pub default_open: bool,
}

fn default_panel_position() -> String {
    "bottom".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FormatDeclaration {
    pub name: String,
    pub extensions: Vec<String>,
    #[serde(default)]
    pub mime_types: Vec<String>,
    #[serde(default = "default_format_direction")]
    pub direction: String, // "import" | "export" | "both"
}

fn default_format_direction() -> String {
    "import".to_string()
}

impl PluginManifest {
    pub fn from_toml(content: &str) -> Result<Self, ManifestError> {
        let manifest: PluginManifest = toml::from_str(content)?;
        if manifest.plugin.id.is_empty() {
            return Err(ManifestError::MissingField("plugin.id"));
        }
        if manifest.plugin.name.is_empty() {
            return Err(ManifestError::MissingField("plugin.name"));
        }
        Ok(manifest)
    }

    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, ManifestError> {
        let content = std::fs::read_to_string(path)?;
        Self::from_toml(&content)
    }

    pub fn id(&self) -> &str {
        &self.plugin.id
    }

    pub fn name(&self) -> &str {
        &self.plugin.name
    }

    pub fn r#type(&self) -> &PluginType {
        &self.plugin.r#type
    }

    pub fn permissions(&self) -> &PluginPermissions {
        &self.plugin.permissions
    }

    pub fn effects(&self) -> &[EffectDeclaration] {
        &self.plugin.effects
    }

    pub fn panels(&self) -> &[PanelDeclaration] {
        &self.plugin.panels
    }

    pub fn formats(&self) -> &[FormatDeclaration] {
        &self.plugin.formats
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_manifest() {
        let toml_str = r#"
[plugin]
id          = "dev.tempo.glow"
name        = "Glow Effect"
version     = "1.0.0"
description = "Adds glowing transitions"
author      = "Tempo Team"

[plugin.type]
effect = true

[plugin.permissions]
timeline = "read-write"

[[plugin.effects]]
id = "glow-transition"
name = "Glow"
type = "transition"
wgsl_file = "assets/glow.wgsl"
"#;
        let manifest = PluginManifest::from_toml(toml_str).unwrap();
        assert_eq!(manifest.id(), "dev.tempo.glow");
        assert_eq!(manifest.name(), "Glow Effect");
        assert!(manifest.r#type().effect);
        assert!(!manifest.r#type().ui_panel);
        assert_eq!(manifest.permissions().timeline, "read-write");
        assert_eq!(manifest.effects().len(), 1);
        assert_eq!(manifest.effects()[0].id, "glow-transition");
    }
}
