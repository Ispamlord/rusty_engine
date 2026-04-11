use engine_core::BackendPreference;
use engine_render_api::BackendKind;
use thiserror::Error;

const WINDOWS_PRIORITY: [BackendKind; 3] =
    [BackendKind::Dx12, BackendKind::Vulkan, BackendKind::Dx11];
const LINUX_PRIORITY: [BackendKind; 1] = [BackendKind::Vulkan];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimePlatform {
    Windows,
    Linux,
    Other,
}

impl RuntimePlatform {
    pub fn current() -> Self {
        if cfg!(target_os = "windows") {
            Self::Windows
        } else if cfg!(target_os = "linux") {
            Self::Linux
        } else {
            Self::Other
        }
    }
}

#[derive(Debug, Error)]
pub enum PlatformError {
    #[error("no compatible backend available for {platform:?} with preference {preference:?}")]
    NoCompatibleBackend {
        platform: RuntimePlatform,
        preference: BackendPreference,
    },
}

pub fn default_backend_priority(platform: RuntimePlatform) -> &'static [BackendKind] {
    match platform {
        RuntimePlatform::Windows => &WINDOWS_PRIORITY,
        RuntimePlatform::Linux => &LINUX_PRIORITY,
        RuntimePlatform::Other => &[],
    }
}

pub fn available_backends_for_platform(platform: RuntimePlatform) -> Vec<BackendKind> {
    match platform {
        RuntimePlatform::Windows => WINDOWS_PRIORITY.to_vec(),
        RuntimePlatform::Linux => LINUX_PRIORITY.to_vec(),
        RuntimePlatform::Other => Vec::new(),
    }
}

pub fn choose_backend(
    preference: BackendPreference,
    available_backends: &[BackendKind],
    platform: RuntimePlatform,
) -> Result<BackendKind, PlatformError> {
    if let Some(requested) = map_preference(preference) {
        if available_backends.contains(&requested) {
            return Ok(requested);
        }

        return Err(PlatformError::NoCompatibleBackend {
            platform,
            preference,
        });
    }

    for candidate in default_backend_priority(platform) {
        if available_backends.contains(candidate) {
            return Ok(*candidate);
        }
    }

    Err(PlatformError::NoCompatibleBackend {
        platform,
        preference,
    })
}

fn map_preference(preference: BackendPreference) -> Option<BackendKind> {
    match preference {
        BackendPreference::Auto => None,
        BackendPreference::Vulkan => Some(BackendKind::Vulkan),
        BackendPreference::Dx12 => Some(BackendKind::Dx12),
        BackendPreference::Dx11 => Some(BackendKind::Dx11),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_auto_prefers_dx12_then_vulkan_then_dx11() {
        let selected = choose_backend(
            BackendPreference::Auto,
            &[BackendKind::Dx11, BackendKind::Vulkan],
            RuntimePlatform::Windows,
        )
        .expect("backend should be selected");

        assert_eq!(selected, BackendKind::Vulkan);

        let selected = choose_backend(
            BackendPreference::Auto,
            &[BackendKind::Dx11],
            RuntimePlatform::Windows,
        )
        .expect("backend should be selected");

        assert_eq!(selected, BackendKind::Dx11);
    }

    #[test]
    fn linux_auto_uses_vulkan() {
        let selected = choose_backend(
            BackendPreference::Auto,
            &[BackendKind::Vulkan, BackendKind::Dx11],
            RuntimePlatform::Linux,
        )
        .expect("backend should be selected");

        assert_eq!(selected, BackendKind::Vulkan);
    }

    #[test]
    fn manual_override_is_respected() {
        let selected = choose_backend(
            BackendPreference::Dx11,
            &[BackendKind::Dx11, BackendKind::Dx12],
            RuntimePlatform::Windows,
        )
        .expect("backend should be selected");

        assert_eq!(selected, BackendKind::Dx11);
    }
}
