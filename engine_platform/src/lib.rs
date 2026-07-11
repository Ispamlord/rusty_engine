use engine_core::BackendPreference;
use engine_render_api::BackendKind;
use thiserror::Error;

const WINDOWS_PRIORITY: [BackendKind; 3] =
    [BackendKind::Dx12, BackendKind::Vulkan, BackendKind::Dx11];
const LINUX_PRIORITY: [BackendKind; 1] = [BackendKind::Vulkan];

/// Host platform detected at compile time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimePlatform {
    Windows,
    Linux,
    Other,
}

impl RuntimePlatform {
    /// Returns the platform for which the current binary is being compiled.
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

/// Errors that can occur when selecting a render backend for the host platform.
#[derive(Debug, Error)]
pub enum PlatformError {
    #[error("no compatible backend available for {platform:?} with preference {preference:?}")]
    NoCompatibleBackend {
        platform: RuntimePlatform,
        preference: BackendPreference,
    },
}

/// Returns the default backend preference order for a given platform.
///
/// On Windows the order is DirectX 12, Vulkan, then DirectX 11. On Linux only
/// Vulkan is returned. For unsupported platforms the slice is empty.
pub fn default_backend_priority(platform: RuntimePlatform) -> &'static [BackendKind] {
    match platform {
        RuntimePlatform::Windows => &WINDOWS_PRIORITY,
        RuntimePlatform::Linux => &LINUX_PRIORITY,
        RuntimePlatform::Other => &[],
    }
}

/// Returns the set of backends that are nominally available on `platform`.
pub fn available_backends_for_platform(platform: RuntimePlatform) -> Vec<BackendKind> {
    match platform {
        RuntimePlatform::Windows => WINDOWS_PRIORITY.to_vec(),
        RuntimePlatform::Linux => LINUX_PRIORITY.to_vec(),
        RuntimePlatform::Other => Vec::new(),
    }
}

/// Selects a render backend from `available_backends` based on `preference`.
///
/// If `preference` is a specific backend, that backend is returned only if it
/// is present in `available_backends`. If `preference` is [`BackendPreference::Auto`],
/// the first backend from the platform's default priority list that is also in
/// `available_backends` is returned.
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

    #[test]
    fn auto_with_no_available_backends_fails() {
        let result = choose_backend(BackendPreference::Auto, &[], RuntimePlatform::Windows);
        assert!(
            matches!(result, Err(PlatformError::NoCompatibleBackend { .. })),
            "expected no compatible backend, got {result:?}"
        );
    }

    #[test]
    fn manual_preference_unavailable_fails() {
        let result = choose_backend(
            BackendPreference::Dx12,
            &[BackendKind::Vulkan],
            RuntimePlatform::Linux,
        );
        assert!(
            matches!(result, Err(PlatformError::NoCompatibleBackend { .. })),
            "expected no compatible backend, got {result:?}"
        );
    }

    #[test]
    fn other_platform_auto_fails() {
        let result = choose_backend(
            BackendPreference::Auto,
            &[BackendKind::Vulkan],
            RuntimePlatform::Other,
        );
        assert!(
            matches!(result, Err(PlatformError::NoCompatibleBackend { .. })),
            "expected no compatible backend, got {result:?}"
        );
    }

    #[test]
    fn available_backends_match_default_priority() {
        assert_eq!(
            available_backends_for_platform(RuntimePlatform::Windows),
            WINDOWS_PRIORITY.to_vec()
        );
        assert_eq!(
            available_backends_for_platform(RuntimePlatform::Linux),
            LINUX_PRIORITY.to_vec()
        );
        assert!(available_backends_for_platform(RuntimePlatform::Other).is_empty());
    }

    #[test]
    fn runtime_platform_current_matches_cfg() {
        let current = RuntimePlatform::current();
        #[cfg(target_os = "windows")]
        assert_eq!(current, RuntimePlatform::Windows);
        #[cfg(target_os = "linux")]
        assert_eq!(current, RuntimePlatform::Linux);
        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        assert_eq!(current, RuntimePlatform::Other);
    }
}
