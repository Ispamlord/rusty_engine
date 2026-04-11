use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BackendPreference {
    Auto,
    Vulkan,
    Dx12,
    Dx11,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowConfig {
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub resizable: bool,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            title: "Rusty Engine".to_string(),
            width: 1280,
            height: 720,
            resizable: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FixedStepConfig {
    pub hz: f32,
    pub max_steps_per_frame: u32,
    pub max_catch_up_seconds: f32,
}

impl Default for FixedStepConfig {
    fn default() -> Self {
        Self {
            hz: 60.0,
            max_steps_per_frame: 8,
            max_catch_up_seconds: 0.25,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FramePacingConfig {
    pub target_fps: u32,
    pub sleep_enabled: bool,
}

impl Default for FramePacingConfig {
    fn default() -> Self {
        Self {
            target_fps: 60,
            sleep_enabled: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PerfGateConfig {
    pub enabled: bool,
    pub max_frame_ms: f32,
    pub warmup_frames: u32,
    pub sample_frames: u32,
}

impl Default for PerfGateConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_frame_ms: 16.67,
            warmup_frames: 30,
            sample_frames: 180,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShaderToolchainConfig {
    pub glslc_path: String,
    pub dxc_path: String,
    pub fxc_path: String,
    pub vulkan_profile: String,
    pub dx12_profile: String,
    pub dx11_profile: String,
    pub strict: bool,
}

impl Default for ShaderToolchainConfig {
    fn default() -> Self {
        Self {
            glslc_path: "glslc".to_string(),
            dxc_path: "dxc".to_string(),
            fxc_path: "fxc".to_string(),
            vulkan_profile: "spirv1.5".to_string(),
            dx12_profile: "cs_6_6".to_string(),
            dx11_profile: "cs_5_0".to_string(),
            strict: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryPolicy {
    pub recover_surface_out_of_date: bool,
    pub recover_device_loss: bool,
    pub max_recovery_attempts: u32,
}

impl Default for RecoveryPolicy {
    fn default() -> Self {
        Self {
            recover_surface_out_of_date: true,
            recover_device_loss: true,
            max_recovery_attempts: 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EngineConfig {
    pub window: WindowConfig,
    pub vsync: bool,
    pub backend_preference: BackendPreference,
    pub enable_validation: bool,
    pub enable_debug: bool,
    #[serde(default)]
    pub fixed_step: FixedStepConfig,
    #[serde(default)]
    pub frame_pacing: FramePacingConfig,
    #[serde(default)]
    pub perf_gate: PerfGateConfig,
    #[serde(default)]
    pub shader_toolchain: ShaderToolchainConfig,
    #[serde(default)]
    pub recovery_policy: RecoveryPolicy,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            window: WindowConfig::default(),
            vsync: true,
            backend_preference: BackendPreference::Auto,
            enable_validation: true,
            enable_debug: true,
            fixed_step: FixedStepConfig::default(),
            frame_pacing: FramePacingConfig::default(),
            perf_gate: PerfGateConfig::default(),
            shader_toolchain: ShaderToolchainConfig::default(),
            recovery_policy: RecoveryPolicy::default(),
        }
    }
}

#[derive(Debug, Error)]
pub enum EngineCoreError {
    #[error("unsupported backend preference: {0:?}")]
    UnsupportedBackendPreference(BackendPreference),

    #[error("configuration error: {0}")]
    Config(String),

    #[error("configuration io error: {0}")]
    ConfigIo(#[from] std::io::Error),

    #[error("configuration parse error: {0}")]
    ConfigParse(String),
}

pub fn load_config_from_ron(
    path: impl AsRef<std::path::Path>,
) -> Result<EngineConfig, EngineCoreError> {
    let source = std::fs::read_to_string(path)?;
    ron::from_str::<EngineConfig>(&source)
        .map_err(|err| EngineCoreError::ConfigParse(err.to_string()))
}

pub fn save_config_to_ron(
    path: impl AsRef<std::path::Path>,
    config: &EngineConfig,
) -> Result<(), EngineCoreError> {
    let encoded =
        ron::to_string(config).map_err(|err| EngineCoreError::ConfigParse(err.to_string()))?;
    std::fs::write(path, encoded)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_legacy_config_with_defaults() {
        let legacy = r#"
            (
                window: (title: "Legacy", width: 800, height: 600, resizable: true),
                vsync: true,
                backend_preference: Auto,
                enable_validation: true,
                enable_debug: false,
            )
        "#;

        let parsed: EngineConfig = ron::from_str(legacy).expect("legacy config should parse");
        assert_eq!(parsed.fixed_step, FixedStepConfig::default());
        assert_eq!(parsed.recovery_policy, RecoveryPolicy::default());
    }
}
