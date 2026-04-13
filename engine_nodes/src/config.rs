use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PredefinedNodeType {
    Transform2D,
    Sprite2D,
    Collider2D,
    AudioEmitter,
    Camera2D,
    Bool,
    I32,
    U32,
    F32,
    String,
    Vec2,
    Vec3,
    Flow,
    Data,
    Texture,
    Buffer,
    Audio,
    Event,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "NodeTypeDescriptorSerde", into = "NodeTypeDescriptorSerde")]
pub enum NodeTypeDescriptor {
    Predefined(PredefinedNodeType),
    Custom(String),
    Generic(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
enum NodeTypeDescriptorSerde {
    PredefinedMap {
        #[serde(rename = "Predefined")]
        predefined: PredefinedNodeType,
    },
    CustomMap {
        #[serde(rename = "Custom")]
        custom: String,
    },
    GenericMap {
        #[serde(rename = "Generic")]
        generic: String,
    },
    PredefinedScalar(PredefinedNodeType),
    Scalar(String),
}

impl From<NodeTypeDescriptorSerde> for NodeTypeDescriptor {
    fn from(value: NodeTypeDescriptorSerde) -> Self {
        match value {
            NodeTypeDescriptorSerde::PredefinedMap { predefined } => Self::Predefined(predefined),
            NodeTypeDescriptorSerde::CustomMap { custom } => Self::Custom(custom),
            NodeTypeDescriptorSerde::GenericMap { generic } => Self::Generic(generic),
            NodeTypeDescriptorSerde::PredefinedScalar(predefined) => Self::Predefined(predefined),
            NodeTypeDescriptorSerde::Scalar(value) => {
                if let Some(predefined) = parse_predefined_node_type(&value) {
                    Self::Predefined(predefined)
                } else {
                    Self::Custom(value)
                }
            }
        }
    }
}

impl From<NodeTypeDescriptor> for NodeTypeDescriptorSerde {
    fn from(value: NodeTypeDescriptor) -> Self {
        match value {
            NodeTypeDescriptor::Predefined(predefined) => Self::PredefinedMap { predefined },
            NodeTypeDescriptor::Custom(custom) => Self::CustomMap { custom },
            NodeTypeDescriptor::Generic(generic) => Self::GenericMap { generic },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeInputConfig {
    pub name: String,
    #[serde(rename = "type")]
    pub type_descriptor: NodeTypeDescriptor,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub default_value: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeOutputConfig {
    pub name: String,
    #[serde(rename = "type")]
    pub type_descriptor: NodeTypeDescriptor,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeConfigDocument {
    #[serde(default = "default_node_config_version")]
    pub version: u32,
    pub type_name: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub inputs: Vec<NodeInputConfig>,
    #[serde(default)]
    pub outputs: Vec<NodeOutputConfig>,
    #[serde(default)]
    pub default_impl_path: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

impl NodeConfigDocument {
    pub fn validate(&self) -> Result<(), NodeConfigError> {
        if self.type_name.trim().is_empty() {
            return Err(NodeConfigError::Validation(
                "type_name cannot be empty".to_string(),
            ));
        }

        validate_port_names(
            self.inputs.iter().map(|port| port.name.as_str()),
            "input",
        )?;
        validate_port_names(
            self.outputs.iter().map(|port| port.name.as_str()),
            "output",
        )?;

        Ok(())
    }
}

impl Default for NodeConfigDocument {
    fn default() -> Self {
        Self {
            version: default_node_config_version(),
            type_name: "CustomNode".to_string(),
            display_name: "Custom Node".to_string(),
            description: Some("User-authored node configuration".to_string()),
            category: Some("Gameplay".to_string()),
            inputs: vec![NodeInputConfig {
                name: "input".to_string(),
                type_descriptor: NodeTypeDescriptor::Predefined(PredefinedNodeType::Data),
                required: false,
                default_value: None,
                description: Some("Primary input".to_string()),
            }],
            outputs: vec![NodeOutputConfig {
                name: "output".to_string(),
                type_descriptor: NodeTypeDescriptor::Predefined(PredefinedNodeType::Data),
                description: Some("Primary output".to_string()),
            }],
            default_impl_path: Some("assets/nodes/custom_node.rhai".to_string()),
            tags: vec!["custom".to_string()],
        }
    }
}

#[derive(Debug, Error)]
pub enum NodeConfigError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("YAML parse error: {0}")]
    Parse(#[from] serde_yaml::Error),

    #[error("validation error: {0}")]
    Validation(String),
}

pub fn load_node_config(path: &Path) -> Result<NodeConfigDocument, NodeConfigError> {
    let raw = fs::read_to_string(path)?;
    parse_node_config(&raw)
}

pub fn parse_node_config(raw: &str) -> Result<NodeConfigDocument, NodeConfigError> {
    let document = serde_yaml::from_str::<NodeConfigDocument>(raw)?;
    document.validate()?;
    Ok(document)
}

pub fn save_node_config(path: &Path, config: &NodeConfigDocument) -> Result<(), NodeConfigError> {
    config.validate()?;
    let encoded = serde_yaml::to_string(config)?;
    fs::write(path, encoded)?;
    Ok(())
}

fn validate_port_names<'a>(names: impl Iterator<Item = &'a str>, kind: &str) -> Result<(), NodeConfigError> {
    let mut seen = std::collections::BTreeSet::new();

    for name in names {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(NodeConfigError::Validation(format!(
                "{kind} name cannot be empty"
            )));
        }
        if !seen.insert(trimmed.to_string()) {
            return Err(NodeConfigError::Validation(format!(
                "duplicate {kind} name '{trimmed}'"
            )));
        }
    }

    Ok(())
}

const fn default_node_config_version() -> u32 {
    1
}

fn parse_predefined_node_type(value: &str) -> Option<PredefinedNodeType> {
    match value.trim().to_ascii_lowercase().as_str() {
        "transform2d" => Some(PredefinedNodeType::Transform2D),
        "sprite2d" => Some(PredefinedNodeType::Sprite2D),
        "collider2d" => Some(PredefinedNodeType::Collider2D),
        "audioemitter" => Some(PredefinedNodeType::AudioEmitter),
        "camera2d" => Some(PredefinedNodeType::Camera2D),
        "bool" => Some(PredefinedNodeType::Bool),
        "i32" => Some(PredefinedNodeType::I32),
        "u32" => Some(PredefinedNodeType::U32),
        "f32" => Some(PredefinedNodeType::F32),
        "string" => Some(PredefinedNodeType::String),
        "vec2" => Some(PredefinedNodeType::Vec2),
        "vec3" => Some(PredefinedNodeType::Vec3),
        "flow" => Some(PredefinedNodeType::Flow),
        "data" => Some(PredefinedNodeType::Data),
        "texture" => Some(PredefinedNodeType::Texture),
        "buffer" => Some(PredefinedNodeType::Buffer),
        "audio" => Some(PredefinedNodeType::Audio),
        "event" => Some(PredefinedNodeType::Event),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
        use super::*;

        #[test]
        fn parses_map_style_type_descriptor() {
                let yaml = r#"
version: 1
type_name: ExampleNode
display_name: Example Node
inputs:
    - name: value
        type: { Predefined: F32 }
outputs:
    - name: result
        type: { Predefined: F32 }
"#;

                let config = parse_node_config(yaml).expect("config should parse");
                assert_eq!(config.inputs.len(), 1);
                assert!(matches!(
                        config.inputs[0].type_descriptor,
                        NodeTypeDescriptor::Predefined(PredefinedNodeType::F32)
                ));
        }

        #[test]
        fn parses_scalar_predefined_type_descriptor() {
                let yaml = r#"
version: 1
type_name: ExampleNode
display_name: Example Node
inputs:
    - name: value
        type: F32
outputs:
    - name: result
        type: F32
"#;

                let config = parse_node_config(yaml).expect("config should parse");
                assert_eq!(config.outputs.len(), 1);
                assert!(matches!(
                        config.outputs[0].type_descriptor,
                        NodeTypeDescriptor::Predefined(PredefinedNodeType::F32)
                ));
        }
}
