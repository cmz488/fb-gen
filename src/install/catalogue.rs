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

impl PackageKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            PackageKind::Toolchain => "toolchain",
            PackageKind::McuSdk => "sdk",
            PackageKind::Middleware => "middleware",
        }
    }
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

// ── MSP430 group ─────────────────────────────────────────────

/// MSP430 GNU Toolchain (ultra-low-power 16-bit MCU).
pub const MSP430_ELF_GCC: Package = Package {
    id: "msp430-elf-gcc",
    name: "MSP430 GNU Toolchain",
    kind: PackageKind::Toolchain,
    version: "9.3.1.11",
    arch: None,
    downloads: PlatformDownloads {
        linux_x86_64: Some(Download {
            url: "https://software-dl.ti.com/msp430/msp430_public_sw/mcu/msp430/MSPGCC/latest/exports/msp430-gcc-full-linux-x64-installer-9.3.1.11.tar.xz",
            sha256: "TODO_REAL_SHA256",
        }),
        linux_aarch64: None,
        macos_arm64: None,
        macos_x86_64: None,
        windows_x86_64: None,
    },
    verify: "msp430-elf",
    dependencies: &[],
    scope: InstallScope::Global,
    cmake_metadata: None,
};

// ── MCU SDKs ─────────────────────────────────────────────────

/// STM32CubeF1 HAL — STM32F1xx Hardware Abstraction Layer.
pub const STM32F1_HAL: Package = Package {
    id: "stm32f1-hal",
    name: "STM32CubeF1 HAL",
    kind: PackageKind::McuSdk,
    version: "1.8.5",
    arch: Some(TargetArch::NoneEabi),
    downloads: PlatformDownloads {
        linux_x86_64: Some(Download {
            url: "https://github.com/STMicroelectronics/STM32CubeF1/archive/refs/tags/v1.8.5.tar.gz",
            sha256: "TODO_REAL_SHA256",
        }),
        linux_aarch64: None,
        macos_arm64: None,
        macos_x86_64: None,
        windows_x86_64: None,
    },
    verify: "",  // SDKs don't have a binary to verify
    dependencies: &[],
    scope: InstallScope::Global,
    cmake_metadata: Some(CmakePackageMeta {
        include_dirs: &[
            "Drivers/STM32F1xx_HAL_Driver/Inc",
            "Drivers/STM32F1xx_HAL_Driver/Inc/Legacy",
            "Drivers/CMSIS/Device/ST/STM32F1xx/Include",
            "Drivers/CMSIS/Include",
        ],
        source_globs: &[
            "Drivers/STM32F1xx_HAL_Driver/Src/*.c",
        ],
        compile_defines: &["USE_HAL_DRIVER", "STM32F103xB"],
        link_libraries: &[],
    }),
};

/// STM32CubeF4 HAL — STM32F4xx Hardware Abstraction Layer.
pub const STM32F4_HAL: Package = Package {
    id: "stm32f4-hal",
    name: "STM32CubeF4 HAL",
    kind: PackageKind::McuSdk,
    version: "1.28.0",
    arch: Some(TargetArch::NoneEabi),
    downloads: PlatformDownloads {
        linux_x86_64: Some(Download {
            url: "https://github.com/STMicroelectronics/STM32CubeF4/archive/refs/tags/v1.28.0.tar.gz",
            sha256: "TODO_REAL_SHA256",
        }),
        linux_aarch64: None,
        macos_arm64: None,
        macos_x86_64: None,
        windows_x86_64: None,
    },
    verify: "",
    dependencies: &[],
    scope: InstallScope::Global,
    cmake_metadata: Some(CmakePackageMeta {
        include_dirs: &[
            "Drivers/STM32F4xx_HAL_Driver/Inc",
            "Drivers/STM32F4xx_HAL_Driver/Inc/Legacy",
            "Drivers/CMSIS/Device/ST/STM32F4xx/Include",
            "Drivers/CMSIS/Include",
        ],
        source_globs: &[
            "Drivers/STM32F4xx_HAL_Driver/Src/*.c",
        ],
        compile_defines: &["USE_HAL_DRIVER", "STM32F407xx"],
        link_libraries: &[],
    }),
};

/// STM32CubeH7 HAL — STM32H7xx Hardware Abstraction Layer.
pub const STM32H7_HAL: Package = Package {
    id: "stm32h7-hal",
    name: "STM32CubeH7 HAL",
    kind: PackageKind::McuSdk,
    version: "1.11.2",
    arch: Some(TargetArch::NoneEabi),
    downloads: PlatformDownloads {
        linux_x86_64: Some(Download {
            url: "https://github.com/STMicroelectronics/STM32CubeH7/archive/refs/tags/v1.11.2.tar.gz",
            sha256: "TODO_REAL_SHA256",
        }),
        linux_aarch64: None,
        macos_arm64: None,
        macos_x86_64: None,
        windows_x86_64: None,
    },
    verify: "",
    dependencies: &[],
    scope: InstallScope::Global,
    cmake_metadata: Some(CmakePackageMeta {
        include_dirs: &[
            "Drivers/STM32H7xx_HAL_Driver/Inc",
            "Drivers/STM32H7xx_HAL_Driver/Inc/Legacy",
            "Drivers/CMSIS/Device/ST/STM32H7xx/Include",
            "Drivers/CMSIS/Include",
        ],
        source_globs: &[
            "Drivers/STM32H7xx_HAL_Driver/Src/*.c",
        ],
        compile_defines: &["USE_HAL_DRIVER", "STM32H743xx"],
        link_libraries: &[],
    }),
};

/// Arduino-ESP32 core — Arduino API layer for all ESP32 series.
///
/// Covers Xtensa (ESP32/S2/S3) and RISC-V (ESP32-C3/C6/H2/P4).
/// Requires the corresponding toolchain installed first:
/// `xtensa-esp32-elf-gcc` or `riscv32-esp-elf-gcc`.
pub const ESP32_ARDUINO: Package = Package {
    id: "esp32-arduino",
    name: "ESP32 Arduino Core (all variants)",
    kind: PackageKind::McuSdk,
    version: "3.1.2",
    arch: None,  // cross-architecture: Xtensa + RISC-V
    downloads: PlatformDownloads {
        linux_x86_64: Some(Download {
            url: "https://github.com/espressif/arduino-esp32/archive/refs/tags/3.1.2.tar.gz",
            sha256: "TODO_REAL_SHA256",
        }),
        linux_aarch64: None,
        macos_arm64: None,
        macos_x86_64: None,
        windows_x86_64: None,
    },
    verify: "",
    dependencies: &[],
    scope: InstallScope::Global,
    cmake_metadata: Some(CmakePackageMeta {
        include_dirs: &[
            "cores/esp32",
            "variants/esp32",
        ],
        source_globs: &[
            "cores/esp32/*.c",
            "cores/esp32/*.cpp",
        ],
        compile_defines: &["ARDUINO=10819"],
        link_libraries: &[],
    }),
};

/// STM32CubeG0 HAL — STM32G0xx (Cortex-M0+ entry-level).
pub const STM32G0_HAL: Package = Package {
    id: "stm32g0-hal",
    name: "STM32CubeG0 HAL",
    kind: PackageKind::McuSdk,
    version: "1.6.2",
    arch: Some(TargetArch::NoneEabi),
    downloads: PlatformDownloads {
        linux_x86_64: Some(Download {
            url: "https://github.com/STMicroelectronics/STM32CubeG0/archive/refs/tags/v1.6.2.tar.gz",
            sha256: "TODO_REAL_SHA256",
        }),
        linux_aarch64: None,
        macos_arm64: None,
        macos_x86_64: None,
        windows_x86_64: None,
    },
    verify: "",
    dependencies: &[],
    scope: InstallScope::Global,
    cmake_metadata: Some(CmakePackageMeta {
        include_dirs: &[
            "Drivers/STM32G0xx_HAL_Driver/Inc",
            "Drivers/CMSIS/Device/ST/STM32G0xx/Include",
            "Drivers/CMSIS/Include",
        ],
        source_globs: &[
            "Drivers/STM32G0xx_HAL_Driver/Src/*.c",
        ],
        compile_defines: &["USE_HAL_DRIVER", "STM32G031xx"],
        link_libraries: &[],
    }),
};

/// STM32CubeG4 HAL — STM32G4xx (Cortex-M4 + DSP, motor control).
pub const STM32G4_HAL: Package = Package {
    id: "stm32g4-hal",
    name: "STM32CubeG4 HAL",
    kind: PackageKind::McuSdk,
    version: "1.5.2",
    arch: Some(TargetArch::NoneEabi),
    downloads: PlatformDownloads {
        linux_x86_64: Some(Download {
            url: "https://github.com/STMicroelectronics/STM32CubeG4/archive/refs/tags/v1.5.2.tar.gz",
            sha256: "TODO_REAL_SHA256",
        }),
        linux_aarch64: None,
        macos_arm64: None,
        macos_x86_64: None,
        windows_x86_64: None,
    },
    verify: "",
    dependencies: &[],
    scope: InstallScope::Global,
    cmake_metadata: Some(CmakePackageMeta {
        include_dirs: &[
            "Drivers/STM32G4xx_HAL_Driver/Inc",
            "Drivers/CMSIS/Device/ST/STM32G4xx/Include",
            "Drivers/CMSIS/Include",
        ],
        source_globs: &[
            "Drivers/STM32G4xx_HAL_Driver/Src/*.c",
        ],
        compile_defines: &["USE_HAL_DRIVER", "STM32G474xx"],
        link_libraries: &[],
    }),
};

/// STM32CubeL4 HAL — STM32L4xx (Cortex-M4 ultra-low-power).
pub const STM32L4_HAL: Package = Package {
    id: "stm32l4-hal",
    name: "STM32CubeL4 HAL",
    kind: PackageKind::McuSdk,
    version: "1.17.3",
    arch: Some(TargetArch::NoneEabi),
    downloads: PlatformDownloads {
        linux_x86_64: Some(Download {
            url: "https://github.com/STMicroelectronics/STM32CubeL4/archive/refs/tags/v1.17.3.tar.gz",
            sha256: "TODO_REAL_SHA256",
        }),
        linux_aarch64: None,
        macos_arm64: None,
        macos_x86_64: None,
        windows_x86_64: None,
    },
    verify: "",
    dependencies: &[],
    scope: InstallScope::Global,
    cmake_metadata: Some(CmakePackageMeta {
        include_dirs: &[
            "Drivers/STM32L4xx_HAL_Driver/Inc",
            "Drivers/CMSIS/Device/ST/STM32L4xx/Include",
            "Drivers/CMSIS/Include",
        ],
        source_globs: &[
            "Drivers/STM32L4xx_HAL_Driver/Src/*.c",
        ],
        compile_defines: &["USE_HAL_DRIVER", "STM32L476xx"],
        link_libraries: &[],
    }),
};

/// nRF5 SDK — Nordic nRF52832/840 (Cortex-M4 + BLE).
pub const NRF52_SDK: Package = Package {
    id: "nrf52-sdk",
    name: "nRF5 SDK (nRF52832/840)",
    kind: PackageKind::McuSdk,
    version: "17.1.0",
    arch: Some(TargetArch::NoneEabi),
    downloads: PlatformDownloads {
        linux_x86_64: Some(Download {
            url: "https://github.com/NordicPlayground/nrf5-sdk/archive/refs/tags/v17.1.0.tar.gz",
            sha256: "TODO_REAL_SHA256",
        }),
        linux_aarch64: None,
        macos_arm64: None,
        macos_x86_64: None,
        windows_x86_64: None,
    },
    verify: "",
    dependencies: &[],
    scope: InstallScope::Global,
    cmake_metadata: Some(CmakePackageMeta {
        include_dirs: &[
            "components",
            "components/ble/common",
            "components/libraries/util",
        ],
        source_globs: &[
            "components/libraries/**/*.c",
        ],
        compile_defines: &["NRF52832_XXAA"],
        link_libraries: &[],
    }),
};

/// Raspberry Pi Pico SDK — RP2040 (dual Cortex-M0+).
pub const RP2040_SDK: Package = Package {
    id: "rp2040-sdk",
    name: "Raspberry Pi Pico SDK",
    kind: PackageKind::McuSdk,
    version: "2.1.0",
    arch: Some(TargetArch::ARM32),
    downloads: PlatformDownloads {
        linux_x86_64: Some(Download {
            url: "https://github.com/raspberrypi/pico-sdk/archive/refs/tags/2.1.0.tar.gz",
            sha256: "TODO_REAL_SHA256",
        }),
        linux_aarch64: None,
        macos_arm64: None,
        macos_x86_64: None,
        windows_x86_64: None,
    },
    verify: "",
    dependencies: &[],
    scope: InstallScope::Global,
    cmake_metadata: Some(CmakePackageMeta {
        include_dirs: &[
            "src/common/pico_base/include",
            "src/rp2_common/hardware_gpio/include",
        ],
        source_globs: &[
            "src/rp2_common/hardware_*/**/*.c",
            "src/common/pico_*/**/*.c",
        ],
        compile_defines: &["PICO_BOARD=pico"],
        link_libraries: &[],
    }),
};

/// GD32F3 HAL — GD32F303 (Chinese Cortex-M4, STM32F103 pin-compatible).
pub const GD32F3_HAL: Package = Package {
    id: "gd32f3-hal",
    name: "GD32F3 HAL (GD32F303)",
    kind: PackageKind::McuSdk,
    version: "2.2.1",
    arch: Some(TargetArch::NoneEabi),
    downloads: PlatformDownloads {
        linux_x86_64: Some(Download {
            url: "https://github.com/honghaier250/GD32F303_Firmware_Library/archive/refs/tags/v2.2.1.tar.gz",
            sha256: "TODO_REAL_SHA256",
        }),
        linux_aarch64: None,
        macos_arm64: None,
        macos_x86_64: None,
        windows_x86_64: None,
    },
    verify: "",
    dependencies: &[],
    scope: InstallScope::Global,
    cmake_metadata: Some(CmakePackageMeta {
        include_dirs: &[
            "Firmware/CMSIS/GD/GD32F30x/Include",
            "Firmware/GD32F30x_standard_peripheral/Include",
            "Firmware/CMSIS/Include",
        ],
        source_globs: &[
            "Firmware/GD32F30x_standard_peripheral/Source/*.c",
        ],
        compile_defines: &["GD32F303"],
        link_libraries: &[],
    }),
};

// ── Middleware ─────────────────────────────────────────────────

/// FreeRTOS kernel — real-time operating system for microcontrollers.
pub const FREERTOS_KERNEL: Package = Package {
    id: "freertos-kernel",
    name: "FreeRTOS Kernel",
    kind: PackageKind::Middleware,
    version: "11.1.0",
    arch: None,  // Cross-architecture
    downloads: PlatformDownloads {
        linux_x86_64: Some(Download {
            url: "https://github.com/FreeRTOS/FreeRTOS-Kernel/archive/refs/tags/V11.1.0.tar.gz",
            sha256: "TODO_REAL_SHA256",
        }),
        linux_aarch64: None,
        macos_arm64: None,
        macos_x86_64: None,
        windows_x86_64: None,
    },
    verify: "",
    dependencies: &[],  // No hard dependency; works standalone
    scope: InstallScope::Global,
    cmake_metadata: Some(CmakePackageMeta {
        include_dirs: &["include"],
        source_globs: &[
            "*.c",
            "portable/GCC/ARM_CM3/*.c",
        ],
        compile_defines: &[],
        link_libraries: &[],
    }),
};

/// lwIP — lightweight TCP/IP stack.
pub const LWIP: Package = Package {
    id: "lwip",
    name: "lwIP (Lightweight TCP/IP Stack)",
    kind: PackageKind::Middleware,
    version: "2.2.0",
    arch: None,
    downloads: PlatformDownloads {
        linux_x86_64: Some(Download {
            url: "https://github.com/lwip-tcpip/lwip/archive/refs/tags/STABLE-2_2_0_RELEASE.tar.gz",
            sha256: "TODO_REAL_SHA256",
        }),
        linux_aarch64: None,
        macos_arm64: None,
        macos_x86_64: None,
        windows_x86_64: None,
    },
    verify: "",
    dependencies: &[],
    scope: InstallScope::Global,
    cmake_metadata: Some(CmakePackageMeta {
        include_dirs: &["src/include"],
        source_globs: &[
            "src/core/*.c",
            "src/core/ipv4/*.c",
            "src/api/*.c",
            "src/netif/*.c",
        ],
        compile_defines: &[],
        link_libraries: &[],
    }),
};

/// FatFS — generic FAT/exFAT filesystem.
pub const FATFS: Package = Package {
    id: "fatfs",
    name: "FatFS (FAT/exFAT Filesystem)",
    kind: PackageKind::Middleware,
    version: "0.15",
    arch: None,
    downloads: PlatformDownloads {
        linux_x86_64: Some(Download {
            url: "https://github.com/abbrev/fatfs/archive/refs/tags/v0.15.tar.gz",
            sha256: "TODO_REAL_SHA256",
        }),
        linux_aarch64: None,
        macos_arm64: None,
        macos_x86_64: None,
        windows_x86_64: None,
    },
    verify: "",
    dependencies: &[],
    scope: InstallScope::Global,
    cmake_metadata: Some(CmakePackageMeta {
        include_dirs: &["source"],
        source_globs: &["source/*.c"],
        compile_defines: &[],
        link_libraries: &[],
    }),
};

/// mbed TLS — cryptographic and TLS library.
pub const MBEDTLS: Package = Package {
    id: "mbedtls",
    name: "mbed TLS (Cryptographic & TLS Library)",
    kind: PackageKind::Middleware,
    version: "3.6.0",
    arch: None,
    downloads: PlatformDownloads {
        linux_x86_64: Some(Download {
            url: "https://github.com/Mbed-TLS/mbedtls/archive/refs/tags/v3.6.0.tar.gz",
            sha256: "TODO_REAL_SHA256",
        }),
        linux_aarch64: None,
        macos_arm64: None,
        macos_x86_64: None,
        windows_x86_64: None,
    },
    verify: "",
    dependencies: &[],
    scope: InstallScope::Global,
    cmake_metadata: Some(CmakePackageMeta {
        include_dirs: &["include"],
        source_globs: &["library/*.c"],
        compile_defines: &[],
        link_libraries: &[],
    }),
};

/// LittleFS — fail-safe filesystem for SPI flash / SD cards.
pub const LITTLEFS: Package = Package {
    id: "littlefs",
    name: "LittleFS (Flash Filesystem)",
    kind: PackageKind::Middleware,
    version: "2.9.1",
    arch: None,
    downloads: PlatformDownloads {
        linux_x86_64: Some(Download {
            url: "https://github.com/littlefs-project/littlefs/archive/refs/tags/v2.9.1.tar.gz",
            sha256: "TODO_REAL_SHA256",
        }),
        linux_aarch64: None,
        macos_arm64: None,
        macos_x86_64: None,
        windows_x86_64: None,
    },
    verify: "",
    dependencies: &[],
    scope: InstallScope::Global,
    cmake_metadata: Some(CmakePackageMeta {
        include_dirs: &["."],
        source_globs: &["*.c"],
        compile_defines: &["LFS_NO_DEBUG"],
        link_libraries: &[],
    }),
};

/// LVGL — Light and Versatile Embedded Graphics Library (GUI for embedded displays).
pub const LVGL: Package = Package {
    id: "lvgl",
    name: "LVGL (Embedded GUI)",
    kind: PackageKind::Middleware,
    version: "9.3.0",
    arch: None,
    downloads: PlatformDownloads {
        linux_x86_64: Some(Download {
            url: "https://github.com/lvgl/lvgl/archive/refs/tags/v9.3.0.tar.gz",
            sha256: "TODO_REAL_SHA256",
        }),
        linux_aarch64: None,
        macos_arm64: None,
        macos_x86_64: None,
        windows_x86_64: None,
    },
    verify: "",
    dependencies: &[],
    scope: InstallScope::Global,
    cmake_metadata: Some(CmakePackageMeta {
        include_dirs: &["src"],
        source_globs: &["src/**/*.c"],
        compile_defines: &["LV_CONF_INCLUDE_SIMPLE"],
        link_libraries: &[],
    }),
};

/// TinyUSB — USB device/host stack (HID, CDC, MSC, MIDI).
pub const TINYUSB: Package = Package {
    id: "tinyusb",
    name: "TinyUSB (USB Stack)",
    kind: PackageKind::Middleware,
    version: "0.17.0",
    arch: None,
    downloads: PlatformDownloads {
        linux_x86_64: Some(Download {
            url: "https://github.com/hathach/tinyusb/archive/refs/tags/0.17.0.tar.gz",
            sha256: "TODO_REAL_SHA256",
        }),
        linux_aarch64: None,
        macos_arm64: None,
        macos_x86_64: None,
        windows_x86_64: None,
    },
    verify: "",
    dependencies: &[],
    scope: InstallScope::Global,
    cmake_metadata: Some(CmakePackageMeta {
        include_dirs: &["src"],
        source_globs: &["src/*.c", "src/portable/st/stm32_fsdev/*.c"],
        compile_defines: &[],
        link_libraries: &[],
    }),
};

/// cJSON — ultra-lightweight JSON parser for IoT.
pub const CJSON: Package = Package {
    id: "cjson",
    name: "cJSON (JSON Parser)",
    kind: PackageKind::Middleware,
    version: "1.7.18",
    arch: None,
    downloads: PlatformDownloads {
        linux_x86_64: Some(Download {
            url: "https://github.com/DaveGamble/cJSON/archive/refs/tags/v1.7.18.tar.gz",
            sha256: "TODO_REAL_SHA256",
        }),
        linux_aarch64: None,
        macos_arm64: None,
        macos_x86_64: None,
        windows_x86_64: None,
    },
    verify: "",
    dependencies: &[],
    scope: InstallScope::Global,
    cmake_metadata: Some(CmakePackageMeta {
        include_dirs: &["."],
        source_globs: &["*.c"],
        compile_defines: &["CJSON_HIDE_SYMBOLS"],
        link_libraries: &[],
    }),
};

/// NanoPB — Protocol Buffers for microcontrollers.
pub const NANOPB: Package = Package {
    id: "nanopb",
    name: "NanoPB (Protocol Buffers)",
    kind: PackageKind::Middleware,
    version: "0.4.9",
    arch: None,
    downloads: PlatformDownloads {
        linux_x86_64: Some(Download {
            url: "https://github.com/nanopb/nanopb/archive/refs/tags/0.4.9.tar.gz",
            sha256: "TODO_REAL_SHA256",
        }),
        linux_aarch64: None,
        macos_arm64: None,
        macos_x86_64: None,
        windows_x86_64: None,
    },
    verify: "",
    dependencies: &[],
    scope: InstallScope::Global,
    cmake_metadata: Some(CmakePackageMeta {
        include_dirs: &["."],
        source_globs: &["*.c"],
        compile_defines: &["PB_ENABLE_MALLOC"],
        link_libraries: &[],
    }),
};

/// CMSIS-DSP — ARM DSP library (FFT, filters, matrix ops).
pub const CMSIS_DSP: Package = Package {
    id: "cmsis-dsp",
    name: "CMSIS-DSP (ARM DSP Library)",
    kind: PackageKind::Middleware,
    version: "1.15.1",
    arch: Some(TargetArch::NoneEabi),
    downloads: PlatformDownloads {
        linux_x86_64: Some(Download {
            url: "https://github.com/ARM-software/CMSIS-DSP/archive/refs/tags/v1.15.1.tar.gz",
            sha256: "TODO_REAL_SHA256",
        }),
        linux_aarch64: None,
        macos_arm64: None,
        macos_x86_64: None,
        windows_x86_64: None,
    },
    verify: "",
    dependencies: &[],
    scope: InstallScope::Global,
    cmake_metadata: Some(CmakePackageMeta {
        include_dirs: &["Include"],
        source_globs: &["Source/**/*.c"],
        compile_defines: &["ARM_MATH_CM4"],
        link_libraries: &[],
    }),
};

/// Full catalogue — all available toolchain packages.
pub static CATALOGUE: &[&Package] = &[
    &ARM_NONE_EABI,
    &XTENSA_ESP32_ELF,
    &RISCV32_ESP_ELF,
    &RISCV64_UNKNOWN_ELF,
    &ARM_LINUX_GNUEABIHF,
    &AARCH64_LINUX_GNU,
    &MSP430_ELF_GCC,
    &STM32F1_HAL,
    &STM32F4_HAL,
    &STM32H7_HAL,
    &STM32G0_HAL,
    &STM32G4_HAL,
    &STM32L4_HAL,
    &NRF52_SDK,
    &RP2040_SDK,
    &GD32F3_HAL,
    &ESP32_ARDUINO,
    &FREERTOS_KERNEL,
    &LWIP,
    &FATFS,
    &MBEDTLS,
    &LITTLEFS,
    &LVGL,
    &TINYUSB,
    &CJSON,
    &NANOPB,
    &CMSIS_DSP,
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
            assert!(pkg.dependencies.is_empty());
            match pkg.kind {
                PackageKind::Toolchain => {
                    assert!(!pkg.verify.is_empty(), "toolchain verify must not be empty");
                    // arch may be None for toolchains with custom targets (e.g. MSP430)
                    // that can't be expressed via TargetArch in const context
                    assert!(pkg.cmake_metadata.is_none());
                }
                PackageKind::McuSdk | PackageKind::Middleware => {
                    assert!(pkg.verify.is_empty(), "SDK/middleware verify should be empty");
                    assert!(pkg.cmake_metadata.is_some(), "SDK/middleware must have cmake_metadata");
                }
            }
        }
    }

    #[test]
    fn test_package_verify_field() {
        assert!(ARM_NONE_EABI.verify.contains("arm-none-eabi"));
    }

    #[test]
    fn test_stm32f1_hal_has_cmake_metadata() {
        assert_eq!(STM32F1_HAL.kind, PackageKind::McuSdk);
        let meta = STM32F1_HAL.cmake_metadata.as_ref().unwrap();
        assert!(!meta.include_dirs.is_empty());
        assert!(!meta.source_globs.is_empty());
        assert!(meta.compile_defines.contains(&"USE_HAL_DRIVER"));
    }

    #[test]
    fn test_freertos_is_middleware() {
        assert_eq!(FREERTOS_KERNEL.kind, PackageKind::Middleware);
        assert!(FREERTOS_KERNEL.cmake_metadata.is_some());
        assert!(FREERTOS_KERNEL.dependencies.is_empty());
        // FreeRTOS has include dirs and source files.
        let meta = FREERTOS_KERNEL.cmake_metadata.as_ref().unwrap();
        assert!(!meta.include_dirs.is_empty());
        assert!(!meta.source_globs.is_empty());
    }

    #[test]
    fn test_middleware_packages_are_in_catalogue() {
        let middleware_ids: Vec<&str> = CATALOGUE.iter()
            .filter(|p| p.kind == PackageKind::Middleware)
            .map(|p| p.id)
            .collect();
        assert_eq!(middleware_ids.len(), 10);
        assert!(middleware_ids.contains(&"freertos-kernel"));
        assert!(middleware_ids.contains(&"lwip"));
        assert!(middleware_ids.contains(&"fatfs"));
        assert!(middleware_ids.contains(&"mbedtls"));
        assert!(middleware_ids.contains(&"littlefs"));
        assert!(middleware_ids.contains(&"lvgl"));
        assert!(middleware_ids.contains(&"tinyusb"));
        assert!(middleware_ids.contains(&"cjson"));
        assert!(middleware_ids.contains(&"nanopb"));
        assert!(middleware_ids.contains(&"cmsis-dsp"));
    }
}
