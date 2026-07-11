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

impl WindowConfig {
    /// Validates window dimensions and other invariants.
    ///
    /// Returns `Ok(())` if the configuration is usable; otherwise returns a
    /// descriptive error. Zero or negative dimensions cannot be used to create
    /// a valid surface or viewport.
    pub fn validate(&self) -> Result<(), EngineCoreError> {
        if self.width == 0 {
            return Err(EngineCoreError::Config(
                "window width must be greater than zero".to_string(),
            ));
        }
        if self.height == 0 {
            return Err(EngineCoreError::Config(
                "window height must be greater than zero".to_string(),
            ));
        }
        Ok(())
    }
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

impl FixedStepConfig {
    /// Validates fixed-step parameters.
    ///
    /// A zero or negative frequency would cause division-by-zero when
    /// converting to a timestep, and a zero step limit would stall the fixed
    /// update loop.
    pub fn validate(&self) -> Result<(), EngineCoreError> {
        if self.hz <= 0.0 {
            return Err(EngineCoreError::Config(
                "fixed_step.hz must be greater than zero".to_string(),
            ));
        }
        if self.max_steps_per_frame == 0 {
            return Err(EngineCoreError::Config(
                "fixed_step.max_steps_per_frame must be greater than zero".to_string(),
            ));
        }
        if self.max_catch_up_seconds < 0.0 {
            return Err(EngineCoreError::Config(
                "fixed_step.max_catch_up_seconds must not be negative".to_string(),
            ));
        }
        Ok(())
    }
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

impl FramePacingConfig {
    /// Validates frame-pacing parameters.
    ///
    /// A target FPS of zero would cause division-by-zero when computing target
    /// frame time.
    pub fn validate(&self) -> Result<(), EngineCoreError> {
        if self.target_fps == 0 {
            return Err(EngineCoreError::Config(
                "frame_pacing.target_fps must be greater than zero".to_string(),
            ));
        }
        Ok(())
    }
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

impl PerfGateConfig {
    /// Validates performance-gate parameters.
    ///
    /// A non-positive frame-time threshold cannot be used as a meaningful
    /// performance budget.
    pub fn validate(&self) -> Result<(), EngineCoreError> {
        if self.max_frame_ms <= 0.0 {
            return Err(EngineCoreError::Config(
                "perf_gate.max_frame_ms must be greater than zero".to_string(),
            ));
        }
        Ok(())
    }
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

impl ShaderToolchainConfig {
    /// Validates shader toolchain configuration.
    ///
    /// Empty compiler paths or shader profiles are rejected because they would
    /// fail later when the asset pipeline tries to invoke the toolchain.
    pub fn validate(&self) -> Result<(), EngineCoreError> {
        for (name, value) in [
            ("glslc_path", &self.glslc_path),
            ("dxc_path", &self.dxc_path),
            ("fxc_path", &self.fxc_path),
            ("vulkan_profile", &self.vulkan_profile),
            ("dx12_profile", &self.dx12_profile),
            ("dx11_profile", &self.dx11_profile),
        ] {
            if value.trim().is_empty() {
                return Err(EngineCoreError::Config(format!(
                    "shader_toolchain.{name} must not be empty"
                )));
            }
        }
        Ok(())
    }
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

impl RecoveryPolicy {
    /// Validates recovery policy configuration.
    ///
    /// If any automatic recovery path is enabled, at least one recovery
    /// attempt must be allowed; otherwise recovery is enabled but cannot run.
    pub fn validate(&self) -> Result<(), EngineCoreError> {
        let any_recovery_enabled = self.recover_surface_out_of_date || self.recover_device_loss;
        if any_recovery_enabled && self.max_recovery_attempts == 0 {
            return Err(EngineCoreError::Config(
                "recovery_policy.max_recovery_attempts must be greater than zero when recovery is enabled".to_string(),
            ));
        }
        Ok(())
    }
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

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SchedulerTopologyBias {
    #[default]
    Balanced,
    PreferHighClock,
    PreferManyCore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchedulerTuningConfig {
    pub enabled: bool,
    #[serde(default)]
    pub topology_bias: SchedulerTopologyBias,
    pub reserve_main_thread: bool,
    pub min_workers: u32,
    pub max_workers: u32,
    pub script_parallel_min_jobs: u32,
}

impl SchedulerTuningConfig {
    /// Validates scheduler tuning configuration.
    ///
    /// Worker limits must form a valid range and there must be at least one
    /// worker so that the thread pool can be created.
    pub fn validate(&self) -> Result<(), EngineCoreError> {
        if self.min_workers == 0 {
            return Err(EngineCoreError::Config(
                "scheduler_tuning.min_workers must be greater than zero".to_string(),
            ));
        }
        if self.max_workers < self.min_workers {
            return Err(EngineCoreError::Config(
                "scheduler_tuning.max_workers must be greater than or equal to min_workers"
                    .to_string(),
            ));
        }
        Ok(())
    }
}

impl Default for SchedulerTuningConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            topology_bias: SchedulerTopologyBias::Balanced,
            reserve_main_thread: true,
            min_workers: 1,
            max_workers: 16,
            script_parallel_min_jobs: 2,
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
    #[serde(default)]
    pub scheduler_tuning: SchedulerTuningConfig,
}

impl EngineConfig {
    /// Validates the entire engine configuration.
    ///
    /// Recursively checks every nested config section for values that would
    /// cause undefined behavior or runtime failures (e.g. division by zero,
    /// impossible worker ranges, empty toolchain paths).
    pub fn validate(&self) -> Result<(), EngineCoreError> {
        self.window.validate()?;
        self.fixed_step.validate()?;
        self.frame_pacing.validate()?;
        self.perf_gate.validate()?;
        self.shader_toolchain.validate()?;
        self.recovery_policy.validate()?;
        self.scheduler_tuning.validate()?;
        Ok(())
    }
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
            scheduler_tuning: SchedulerTuningConfig::default(),
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

/// Loads an [`EngineConfig`] from a RON file and validates it.
///
/// Returns a parse error if the file is malformed and a validation error if
/// the parsed configuration contains values that cannot be used safely.
pub fn load_config_from_ron(
    path: impl AsRef<std::path::Path>,
) -> Result<EngineConfig, EngineCoreError> {
    let source = std::fs::read_to_string(path)?;
    let config = ron::from_str::<EngineConfig>(&source)
        .map_err(|err| EngineCoreError::ConfigParse(err.to_string()))?;
    config.validate()?;
    Ok(config)
}

/// Saves an [`EngineConfig`] to a RON file.
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
        assert_eq!(parsed.scheduler_tuning, SchedulerTuningConfig::default());
        parsed.validate().expect("legacy config should be valid");
    }

    #[test]
    fn default_config_passes_validation() {
        EngineConfig::default()
            .validate()
            .expect("default config should be valid");
    }

    #[test]
    fn window_zero_width_is_invalid() {
        let mut config = EngineConfig::default();
        config.window.width = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn window_zero_height_is_invalid() {
        let mut config = EngineConfig::default();
        config.window.height = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn fixed_step_zero_hz_is_invalid() {
        let mut config = EngineConfig::default();
        config.fixed_step.hz = 0.0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn fixed_step_negative_hz_is_invalid() {
        let mut config = EngineConfig::default();
        config.fixed_step.hz = -1.0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn fixed_step_zero_max_steps_is_invalid() {
        let mut config = EngineConfig::default();
        config.fixed_step.max_steps_per_frame = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn fixed_step_negative_catch_up_is_invalid() {
        let mut config = EngineConfig::default();
        config.fixed_step.max_catch_up_seconds = -0.1;
        assert!(config.validate().is_err());
    }

    #[test]
    fn frame_pacing_zero_target_fps_is_invalid() {
        let mut config = EngineConfig::default();
        config.frame_pacing.target_fps = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn perf_gate_non_positive_budget_is_invalid() {
        let mut config = EngineConfig::default();
        config.perf_gate.max_frame_ms = 0.0;
        assert!(config.validate().is_err());

        config.perf_gate.max_frame_ms = -1.0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn shader_toolchain_empty_path_is_invalid() {
        let mut config = EngineConfig::default();
        config.shader_toolchain.glslc_path = "   ".to_string();
        assert!(config.validate().is_err());
    }

    #[test]
    fn recovery_enabled_with_zero_attempts_is_invalid() {
        let mut config = EngineConfig::default();
        config.recovery_policy.max_recovery_attempts = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn recovery_disabled_with_zero_attempts_is_valid() {
        let mut config = EngineConfig::default();
        config.recovery_policy.recover_surface_out_of_date = false;
        config.recovery_policy.recover_device_loss = false;
        config.recovery_policy.max_recovery_attempts = 0;
        assert!(config.validate().is_ok());
    }

    #[test]
    fn scheduler_zero_min_workers_is_invalid() {
        let mut config = EngineConfig::default();
        config.scheduler_tuning.min_workers = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn scheduler_max_below_min_is_invalid() {
        let mut config = EngineConfig::default();
        config.scheduler_tuning.max_workers = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = std::env::temp_dir();
        let path = dir.join("rusty_engine_core_config_roundtrip.ron");
        let _ = std::fs::remove_file(&path);

        let config = EngineConfig::default();
        save_config_to_ron(&path, &config).expect("save should succeed");
        let loaded = load_config_from_ron(&path).expect("load should succeed");
        assert_eq!(loaded, config);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_missing_file_returns_io_error() {
        let path = std::env::temp_dir().join("rusty_engine_core_missing_config.ron");
        let _ = std::fs::remove_file(&path);
        let result = load_config_from_ron(&path);
        assert!(matches!(result, Err(EngineCoreError::ConfigIo(_))));
    }

    #[test]
    fn load_invalid_ron_returns_parse_error() {
        let dir = std::env::temp_dir();
        let path = dir.join("rusty_engine_core_invalid_config.ron");
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, "not valid ron").expect("write should succeed");

        let result = load_config_from_ron(&path);
        assert!(
            matches!(result, Err(EngineCoreError::ConfigParse(_))),
            "expected parse error, got {result:?}"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_invalid_values_returns_validation_error() {
        let dir = std::env::temp_dir();
        let path = dir.join("rusty_engine_core_invalid_values.ron");
        let _ = std::fs::remove_file(&path);
        std::fs::write(
            &path,
            r#"(
                window: (title: "Test", width: 0, height: 720, resizable: true),
                vsync: true,
                backend_preference: Auto,
                enable_validation: true,
                enable_debug: true,
            )"#,
        )
        .expect("write should succeed");

        let result = load_config_from_ron(&path);
        assert!(
            matches!(result, Err(EngineCoreError::Config(_))),
            "expected validation error, got {result:?}"
        );

        let _ = std::fs::remove_file(&path);
    }
}
