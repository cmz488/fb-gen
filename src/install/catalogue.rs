//! Package catalogue — hard-coded package definitions for all installable components.
//!
//! Phase 1 contains only the ARM GNU toolchain.  Additional toolchains, MCU SDKs,
//! and middleware are added in later phases.

use crate::models::project::TargetArch;
use std::env::consts;

/// Package type: toolchain, MCU SDK, or middleware.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageKind {
    Toolchain,
    McuSdk,
    Middleware,
}

/// Installation scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallScope {
    Global,
    LocalProject,
}

/// A single download entry with URL and expected SHA256.
#[derive(Debug, Clone)]
pub struct Download {
    pub url: &'static str,
    pub sha256: &'static str,
}

/// Platform-specific download URLs.
#[derive(Debug, Clone, Default)]
pub struct PlatformDownloads {
    pub linux_x86_64: Option<Download>,
    pub linux_aarch64: Option<Download>,
    pub macos_arm64: Option<Download>,
    pub macos_x86_64: Option<Download>,
    pub windows_x86_64: Option<Download>,
}

impl PlatformDownloads {
    /// Return the `Download` entry for the current host platform, if any.
    pub fn for_current_platform(&self) -> Option<&Download> {
        match (consts::OS, consts::ARCH) {
            ("linux", "x86_64") => self.linux_x86_64.as_ref(),
            ("linux", "aarch64") => self.linux_aarch64.as_ref(),
            ("macos", "aarch64") | ("macos", "arm") => self.macos_arm64.as_ref(),
            ("macos", "x86_64") => self.macos_x86_64.as_ref(),
            ("windows", "x86_64") => self.windows_x86_64.as_ref(),
            _ => None,
        }
    }
}

/// CMake bridge metadata — tells `bridge.rs` how to inject an SDK package
/// into the generated `CMakeLists.txt`.
#[derive(Debug, Clone)]
pub struct CmakePackageMeta {
    /// Include directories relative to package root.
    pub include_dirs: &'static [&'static str],
    /// Source-file globs relative to package root (e.g. `"Drivers/.../Src/*.c"`).
    pub source_globs: &'static [&'static str],
    /// Preprocessor definitions to add for this package.
    pub compile_defines: &'static [&'static str],
    /// Library names to link (e.g. `["cmsis", "hal"]`).
    pub link_libraries: &'static [&'static str],
}

/// A package available for installation.
#[derive(Debug, Clone)]
pub struct Package {
    /// Stable identifier (e.g. `"arm-none-eabi-gcc"`).
    pub id: &'static str,
    /// Human-readable display name.
    pub name: &'static str,
    /// What kind of package this is.
    pub kind: PackageKind,
    /// Upstream version string.
    pub version: &'static str,
    /// Target architecture this package targets, if applicable.
    pub arch: Option<TargetArch>,
    /// Download URLs per host platform.
    pub downloads: PlatformDownloads,
    /// Substring expected in `{prefix}gcc --version` output after install.
    pub verify: &'static str,
    /// IDs of packages that must be installed first.
    pub dependencies: &'static [&'static str],
    /// Whether the package is global or per-project.
    pub scope: InstallScope,
    /// CMake bridge metadata (`None` for pure toolchain packages).
    pub cmake_metadata: Option<CmakePackageMeta>,
}

// ── Phase 1 Catalogue ────────────────────────────────────────────

/// ARM GNU Toolchain for Cortex-M bare-metal targets.
pub const ARM_NONE_EABI: Package = Package {
    id: "arm-none-eabi-gcc",
    name: "ARM GNU Toolchain (arm-none-eabi)",
    kind: PackageKind::Toolchain,
    version: "13.3.rel1",
    arch: Some(TargetArch::NoneEabi),
    downloads: PlatformDownloads {
        linux_x86_64: Some(Download {
            url: "https://developer.arm.com/-/media/Files/downloads/gnu/13.3.rel1/binrel/\
                  arm-gnu-toolchain-13.3.rel1-x86_64-arm-none-eabi.tar.xz",
            sha256: "TODO_REAL_SHA256",
        }),
        linux_aarch64: Some(Download {
            url: "https://developer.arm.com/-/media/Files/downloads/gnu/13.3.rel1/binrel/\
                  arm-gnu-toolchain-13.3.rel1-aarch64-arm-none-eabi.tar.xz",
            sha256: "TODO_REAL_SHA256",
        }),
        macos_arm64: None,
        macos_x86_64: None,
        windows_x86_64: None
    },
    verify: "arm-none-eabi",
    dependencies: &[],
    scope: InstallScope::Global,
    cmake_metadata: None,
};

/// Full catalogue — Phase 1 has only the ARM toolchain.
pub static CATALOGUE: &[&Package] = &[&ARM_NONE_EABI];


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_arm_none_eabi_definition() {
        assert_eq!(ARM_NONE_EABI.id, "arm-none-eabi-gcc");
        assert_eq!(ARM_NONE_EABI.kind, PackageKind::Toolchain);
        assert_eq!(ARM_NONE_EABI.version, "13.3.rel1");
        assert_eq!(ARM_NONE_EABI.arch, Some(TargetArch::NoneEabi));
        assert!(ARM_NONE_EABI.dependencies.is_empty());
        assert!(ARM_NONE_EABI.cmake_metadata.is_none());
    }

    #[test]
    fn test_platform_resolution_linux_x86_64() {
        let dl = ARM_NONE_EABI.downloads.for_current_platform();
        // On linux x86_64 this must resolve.
        if cfg!(target_os = "linux") && cfg!(target_arch = "x86_64") {
            assert!(dl.is_some());
            let d = dl.unwrap();
            assert!(d.url.contains("x86_64"));
            assert!(d.url.contains("arm-none-eabi"));
        }
    }

    #[test]
    fn test_platform_resolution_none_on_unsupported() {
        let empty = PlatformDownloads::default();
        assert!(empty.for_current_platform().is_none());
    }

    #[test]
    fn test_catalogue_not_empty() {
        assert!(!CATALOGUE.is_empty());
    }

    #[test]
    fn test_package_verify_field() {
        assert!(ARM_NONE_EABI.verify.contains("arm-none-eabi"));
    }
}
