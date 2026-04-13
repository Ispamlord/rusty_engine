use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::NodeLibraryScope;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomNodeRegistration {
    pub type_name: String,
    pub config_path: String,
    pub impl_path: String,
    #[serde(default)]
    pub scope: NodeLibraryScope,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomNodeRegistry {
    #[serde(default = "default_registry_version")]
    pub version: u32,
    #[serde(default)]
    pub custom_nodes: Vec<CustomNodeRegistration>,
}

impl Default for CustomNodeRegistry {
    fn default() -> Self {
        Self {
            version: default_registry_version(),
            custom_nodes: Vec::new(),
        }
    }
}

impl CustomNodeRegistry {
    pub fn merged(project: &Self, global: &Self) -> Self {
        let mut merged = BTreeMap::new();

        for entry in &global.custom_nodes {
            merged.insert(entry.type_name.to_ascii_lowercase(), entry.clone());
        }
        for entry in &project.custom_nodes {
            merged.insert(entry.type_name.to_ascii_lowercase(), entry.clone());
        }

        let mut custom_nodes = merged.into_values().collect::<Vec<_>>();
        custom_nodes.sort_by(|a, b| a.type_name.cmp(&b.type_name));

        Self {
            version: project.version.max(global.version),
            custom_nodes,
        }
    }

    pub fn validate(&self) -> Result<(), CustomNodeRegistryError> {
        let mut seen = std::collections::BTreeSet::new();
        for node in &self.custom_nodes {
            if node.type_name.trim().is_empty() {
                return Err(CustomNodeRegistryError::Validation(
                    "custom node type_name cannot be empty".to_string(),
                ));
            }
            if node.config_path.trim().is_empty() {
                return Err(CustomNodeRegistryError::Validation(format!(
                    "custom node '{}' has empty config_path",
                    node.type_name
                )));
            }
            if node.impl_path.trim().is_empty() {
                return Err(CustomNodeRegistryError::Validation(format!(
                    "custom node '{}' has empty impl_path",
                    node.type_name
                )));
            }
            let key = node.type_name.to_ascii_lowercase();
            if !seen.insert(key) {
                return Err(CustomNodeRegistryError::Validation(format!(
                    "duplicate custom node type '{}'",
                    node.type_name
                )));
            }
        }

        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum CustomNodeRegistryError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("YAML parse error: {0}")]
    Parse(#[from] serde_yaml::Error),

    #[error("validation error: {0}")]
    Validation(String),
}

pub fn load_custom_node_registry(path: &Path) -> Result<CustomNodeRegistry, CustomNodeRegistryError> {
    let raw = fs::read_to_string(path)?;
    parse_custom_node_registry(&raw)
}

pub fn parse_custom_node_registry(raw: &str) -> Result<CustomNodeRegistry, CustomNodeRegistryError> {
    let registry = serde_yaml::from_str::<CustomNodeRegistry>(raw)?;
    registry.validate()?;
    Ok(registry)
}

pub fn save_custom_node_registry(
    path: &Path,
    registry: &CustomNodeRegistry,
) -> Result<(), CustomNodeRegistryError> {
    registry.validate()?;
    let encoded = serde_yaml::to_string(registry)?;
    fs::write(path, encoded)?;
    Ok(())
}

const fn default_registry_version() -> u32 {
    1
}
