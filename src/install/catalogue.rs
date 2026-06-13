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

// ── Xtensa group (ESP32 / S2 / S3) ────────────────────────────

/// Xtensa toolchain for ESP32 (LX6 core).
pub const XTENSA_ESP32_ELF: Package = Package {
    id: "xtensa-esp32-elf-gcc",
    name: "Xtensa ESP32 Toolchain",
    kind: PackageKind::Toolchain,
    version: "esp-14.2.0_20240906",
    arch: Some(TargetArch::Xtensa),
    downloads: PlatformDownloads {
        linux_x86_64: Some(Download {
            url: "https://github.com/espressif/crosstool-NG/releases/download/\
                  esp-14.2.0_20240906/xtensa-esp-elf-gcc14_2_0-esp-14.2.0_20240906-x86_64-linux-gnu.tar.xz",
            sha256: "TODO_REAL_SHA256",
        }),
        linux_aarch64: Some(Download {
            url: "https://github.com/espressif/crosstool-NG/releases/download/\
                  esp-14.2.0_20240906/xtensa-esp-elf-gcc14_2_0-esp-14.2.0_20240906-aarch64-linux-gnu.tar.xz",
            sha256: "TODO_REAL_SHA256",
        }),
        macos_arm64: Some(Download {
            url: "https://github.com/espressif/crosstool-NG/releases/download/\
                  esp-14.2.0_20240906/xtensa-esp-elf-gcc14_2_0-esp-14.2.0_20240906-aarch64-apple-darwin.tar.xz",
            sha256: "TODO_REAL_SHA256",
        }),
        macos_x86_64: None,
        windows_x86_64: None,
    },
    verify: "xtensa-esp32-elf",
    dependencies: &[],
    scope: InstallScope::Global,
    cmake_metadata: None,
};

// ── RISC-V group (ESP32-C3/C6/H2/P4) ──────────────────────────

/// RISC-V toolchain for ESP32-C3/C6/H2/P4 (RV32).
pub const RISCV32_ESP_ELF: Package = Package {
    id: "riscv32-esp-elf-gcc",
    name: "RISC-V ESP32 Toolchain",
    kind: PackageKind::Toolchain,
    version: "esp-14.2.0_20240906",
    arch: Some(TargetArch::RISCV32),
    downloads: PlatformDownloads {
        linux_x86_64: Some(Download {
            url: "https://github.com/espressif/crosstool-NG/releases/download/\
                  esp-14.2.0_20240906/riscv32-esp-elf-gcc14_2_0-esp-14.2.0_20240906-x86_64-linux-gnu.tar.xz",
            sha256: "TODO_REAL_SHA256",
        }),
        linux_aarch64: Some(Download {
            url: "https://github.com/espressif/crosstool-NG/releases/download/\
                  esp-14.2.0_20240906/riscv32-esp-elf-gcc14_2_0-esp-14.2.0_20240906-aarch64-linux-gnu.tar.xz",
            sha256: "TODO_REAL_SHA256",
        }),
        macos_arm64: Some(Download {
            url: "https://github.com/espressif/crosstool-NG/releases/download/\
                  esp-14.2.0_20240906/riscv32-esp-elf-gcc14_2_0-esp-14.2.0_20240906-aarch64-apple-darwin.tar.xz",
            sha256: "TODO_REAL_SHA256",
        }),
        macos_x86_64: None,
        windows_x86_64: None,
    },
    verify: "riscv32-esp-elf",
    dependencies: &[],
    scope: InstallScope::Global,
    cmake_metadata: None,
};

// ── RISC-V group (riscv64-unknown-elf) ─────────────────────────

/// RISC-V GNU Toolchain for 64-bit bare-metal (riscv64-unknown-elf).
pub const RISCV64_UNKNOWN_ELF: Package = Package {
    id: "riscv64-unknown-elf-gcc",
    name: "RISC-V GNU Toolchain (riscv64-unknown-elf)",
    kind: PackageKind::Toolchain,
    version: "2024.04.12",
    arch: Some(TargetArch::RISCV64),
    downloads: PlatformDownloads {
        linux_x86_64: Some(Download {
            url: "https://github.com/riscv-collab/riscv-gnu-toolchain/releases/download/\
                  2024.04.12/riscv64-unknown-elf-toolchain-13.3.0-2024.04.12-x86_64-linux-ubuntu14.tar.gz",
            sha256: "TODO_REAL_SHA256",
        }),
        linux_aarch64: None,
        macos_arm64: None,
        macos_x86_64: None,
        windows_x86_64: None,
    },
    verify: "riscv64-unknown-elf",
    dependencies: &[],
    scope: InstallScope::Global,
    cmake_metadata: None,
};

// ── ARM Linux userspace toolchains ─────────────────────────────

/// ARM GNU Toolchain for Linux userspace (arm-linux-gnueabihf).
pub const ARM_LINUX_GNUEABIHF: Package = Package {
    id: "arm-linux-gnueabihf-gcc",
    name: "ARM GNU Toolchain (arm-linux-gnueabihf)",
    kind: PackageKind::Toolchain,
    version: "13.3.rel1",
    arch: Some(TargetArch::ARM32),
    downloads: PlatformDownloads {
        linux_x86_64: Some(Download {
            url: "https://developer.arm.com/-/media/Files/downloads/gnu/13.3.rel1/binrel/\
                  arm-gnu-toolchain-13.3.rel1-x86_64-arm-none-linux-gnueabihf.tar.xz",
            sha256: "TODO_REAL_SHA256",
        }),
        linux_aarch64: Some(Download {
            url: "https://developer.arm.com/-/media/Files/downloads/gnu/13.3.rel1/binrel/\
                  arm-gnu-toolchain-13.3.rel1-aarch64-arm-none-linux-gnueabihf.tar.xz",
            sha256: "TODO_REAL_SHA256",
        }),
        macos_arm64: None,
        macos_x86_64: None,
        windows_x86_64: None,
    },
    verify: "arm-linux-gnueabihf",
    dependencies: &[],
    scope: InstallScope::Global,
    cmake_metadata: None,
};

/// ARM GNU Toolchain for Linux userspace (aarch64-linux-gnu).
pub const AARCH64_LINUX_GNU: Package = Package {
    id: "aarch64-linux-gnu-gcc",
    name: "ARM GNU Toolchain (aarch64-linux-gnu)",
    kind: PackageKind::Toolchain,
    version: "13.3.rel1",
    arch: Some(TargetArch::ARM64),
    downloads: PlatformDownloads {
        linux_x86_64: Some(Download {
            url: "https://developer.arm.com/-/media/Files/downloads/gnu/13.3.rel1/binrel/\
                  arm-gnu-toolchain-13.3.rel1-x86_64-aarch64-none-linux-gnu.tar.xz",
            sha256: "TODO_REAL_SHA256",
        }),
        linux_aarch64: Some(Download {
            url: "https://developer.arm.com/-/media/Files/downloads/gnu/13.3.rel1/binrel/\
                  arm-gnu-toolchain-13.3.rel1-aarch64-aarch64-none-linux-gnu.tar.xz",
            sha256: "TODO_REAL_SHA256",
        }),
        macos_arm64: None,
        macos_x86_64: None,
        windows_x86_64: None,
    },
    verify: "aarch64-linux-gnu",
    dependencies: &[],
    scope: InstallScope::Global,
    cmake_metadata: None,
};

/// Full catalogue — all available toolchain packages.
pub static CATALOGUE: &[&Package] = &[
    &ARM_NONE_EABI,
    &XTENSA_ESP32_ELF,
    &RISCV32_ESP_ELF,
    &RISCV64_UNKNOWN_ELF,
    &ARM_LINUX_GNUEABIHF,
    &AARCH64_LINUX_GNU,
];


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
    fn test_xtensa_esp32_elf_definition() {
        assert_eq!(XTENSA_ESP32_ELF.id, "xtensa-esp32-elf-gcc");
        assert_eq!(XTENSA_ESP32_ELF.kind, PackageKind::Toolchain);
        assert_eq!(XTENSA_ESP32_ELF.arch, Some(TargetArch::Xtensa));
        assert!(XTENSA_ESP32_ELF.downloads.linux_x86_64.is_some());
        assert!(XTENSA_ESP32_ELF.downloads.linux_aarch64.is_some());
        assert!(XTENSA_ESP32_ELF.downloads.macos_arm64.is_some());
        assert!(XTENSA_ESP32_ELF.downloads.macos_x86_64.is_none());
        assert!(XTENSA_ESP32_ELF.downloads.windows_x86_64.is_none());
        assert!(XTENSA_ESP32_ELF.verify.contains("xtensa-esp32-elf"));
    }

    #[test]
    fn test_riscv32_esp_elf_definition() {
        assert_eq!(RISCV32_ESP_ELF.id, "riscv32-esp-elf-gcc");
        assert_eq!(RISCV32_ESP_ELF.kind, PackageKind::Toolchain);
        assert_eq!(RISCV32_ESP_ELF.arch, Some(TargetArch::RISCV32));
        assert!(RISCV32_ESP_ELF.downloads.linux_x86_64.is_some());
        assert!(RISCV32_ESP_ELF.downloads.linux_aarch64.is_some());
        assert!(RISCV32_ESP_ELF.downloads.macos_arm64.is_some());
        assert!(RISCV32_ESP_ELF.verify.contains("riscv32-esp-elf"));
    }

    #[test]
    fn test_riscv64_unknown_elf_definition() {
        assert_eq!(RISCV64_UNKNOWN_ELF.id, "riscv64-unknown-elf-gcc");
        assert_eq!(RISCV64_UNKNOWN_ELF.kind, PackageKind::Toolchain);
        assert_eq!(RISCV64_UNKNOWN_ELF.arch, Some(TargetArch::RISCV64));
        assert!(RISCV64_UNKNOWN_ELF.downloads.linux_x86_64.is_some());
        assert!(RISCV64_UNKNOWN_ELF.downloads.linux_aarch64.is_none());
        assert!(RISCV64_UNKNOWN_ELF.verify.contains("riscv64-unknown-elf"));
    }

    #[test]
    fn test_arm_linux_gnueabihf_definition() {
        assert_eq!(ARM_LINUX_GNUEABIHF.id, "arm-linux-gnueabihf-gcc");
        assert_eq!(ARM_LINUX_GNUEABIHF.kind, PackageKind::Toolchain);
        assert_eq!(ARM_LINUX_GNUEABIHF.arch, Some(TargetArch::ARM32));
        assert!(ARM_LINUX_GNUEABIHF.downloads.linux_x86_64.is_some());
        assert!(ARM_LINUX_GNUEABIHF.downloads.linux_aarch64.is_some());
        assert!(ARM_LINUX_GNUEABIHF.verify.contains("arm-linux-gnueabihf"));
    }

    #[test]
    fn test_aarch64_linux_gnu_definition() {
        assert_eq!(AARCH64_LINUX_GNU.id, "aarch64-linux-gnu-gcc");
        assert_eq!(AARCH64_LINUX_GNU.kind, PackageKind::Toolchain);
        assert_eq!(AARCH64_LINUX_GNU.arch, Some(TargetArch::ARM64));
        assert!(AARCH64_LINUX_GNU.downloads.linux_x86_64.is_some());
        assert!(AARCH64_LINUX_GNU.downloads.linux_aarch64.is_some());
        assert!(AARCH64_LINUX_GNU.verify.contains("aarch64-linux-gnu"));
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
    fn test_catalogue_contains_all_expected() {
        let ids: Vec<&str> = CATALOGUE.iter().map(|p| p.id).collect();
        assert!(ids.contains(&"arm-none-eabi-gcc"));
        assert!(ids.contains(&"xtensa-esp32-elf-gcc"));
        assert!(ids.contains(&"riscv32-esp-elf-gcc"));
        assert!(ids.contains(&"riscv64-unknown-elf-gcc"));
        assert!(ids.contains(&"arm-linux-gnueabihf-gcc"));
        assert!(ids.contains(&"aarch64-linux-gnu-gcc"));
    }

    #[test]
    fn test_catalogue_entries_have_required_fields() {
        for pkg in CATALOGUE.iter() {
            assert!(!pkg.id.is_empty(), "id must not be empty");
            assert!(!pkg.name.is_empty(), "name must not be empty");
            assert!(!pkg.version.is_empty(), "version must not be empty");
            assert!(!pkg.verify.is_empty(), "verify must not be empty");
            assert_eq!(pkg.kind, PackageKind::Toolchain);
            assert!(pkg.arch.is_some(), "arch must be set for toolchains");
            assert!(pkg.dependencies.is_empty());
            assert!(pkg.cmake_metadata.is_none());
        }
    }

    #[test]
    fn test_package_verify_field() {
        assert!(ARM_NONE_EABI.verify.contains("arm-none-eabi"));
    }
}
