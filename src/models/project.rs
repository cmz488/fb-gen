use crate::models::module::CMakeModule;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Supported C/C++ compilers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Compiler {
    GCC,
    Clang,
    Zig,
    MSVC,
    Custom(String),
}

/// Supported target architectures.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TargetArch {
    X86_64,
    X86,
    ARM64,
    ARM32,
    RISCV64,
    /// 32-bit RISC-V (e.g. ESP32-C3/C6/H2/P4).
    RISCV32,
    WASM,
    /// ARM Cortex-M bare-metal (arm-none-eabi).
    NoneEabi,
    /// Xtensa (e.g. ESP32, ESP32-S2, ESP32-S3).
    Xtensa,
    Custom(String),
}

/// CMake build system backend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum BuildBackend {
    #[default]
    Ninja,
    Make,
    MSBuild,
    Custom(String),
}

/// Build system for code generation — CMake or Zig.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum BuildSystem {
    #[default]
    CMake,
    Zig,
}

/// Structured toolchain configuration for embedded cross-compilation targets.
/// `None` when the target architecture does not need a toolchain file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolchainConfig {
    /// CPU / chip model (e.g. "cortex-m3", "cortex-m4").
    /// Generates the `-mcpu=` flag. Required for NoneEabi.
    pub cpu: String,

    /// Floating-point ABI: "soft", "softfp", or "hard". Skipped when empty.
    /// Generates the `-mfloat-abi=` flag.
    pub float_abi: String,

    /// FPU unit (e.g. "fpv4-sp-d16", "fpv5-d16"). Skipped when empty.
    /// Generates the `-mfpu=` flag. ARM-only.
    pub fpu: String,

    /// RISC-V architecture string (e.g. "rv32imac", "rv64gc").
    /// Generates the `-march=` flag. RISC-V only; ignored for ARM / Xtensa.
    #[serde(default)]
    pub march: String,

    /// RISC-V ABI string (e.g. "ilp32", "lp64").
    /// Generates the `-mabi=` flag. RISC-V only; ignored for ARM / Xtensa.
    #[serde(default)]
    pub mabi: String,

    /// Raw compiler/linker flags (e.g. "-mthumb", "-mlongcalls").
    /// Appended verbatim to TARGET_FLAGS.
    pub extra_flags: String,

    /// Toolchain prefix, e.g. `"arm-none-eabi-"`. Also serves as the
    /// gate for toolchain generation: when non-empty, ARM32, ARM64, and
    /// RISCV64 targets also produce a toolchain file.
    #[serde(default)]
    pub prefix: String,

    /// Sysroot returned by `<prefix>gcc -print-sysroot`. `None` when
    /// the compiler reports an empty sysroot (common for bare-metal).
    /// When `Some`, `CMAKE_SYSROOT` and `CMAKE_FIND_ROOT_PATH` are set.
    #[serde(default)]
    pub sysroot: Option<String>,

    /// Extra entries for `CMAKE_FIND_ROOT_PATH` beyond the sysroot itself.
    /// Appended after `${CMAKE_SYSROOT}` in the generated toolchain file.
    #[serde(default)]
    pub find_root_path: Vec<String>,

    /// Device-specific compile definitions (e.g. `"STM32F103xB"`, `"USE_HAL_DRIVER"`).
    /// Each entry is emitted as `-D<define>` in `TARGET_FLAGS`.
    #[serde(default)]
    pub device_defines: Vec<String>,
}

impl Default for ToolchainConfig {
    fn default() -> Self {
        Self {
            cpu: String::new(),
            float_abi: String::new(),
            fpu: String::new(),
            march: String::new(),
            mabi: String::new(),
            extra_flags: String::new(),
            prefix: String::new(),
            sysroot: None,
            find_root_path: Vec::new(),
            device_defines: Vec::new(),
        }
    }
}

/// Top-level project configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectConfig {
    pub name: String,
    pub version: String,
    pub root: PathBuf,
    pub language: String,
    pub c_standard: String,
    pub cpp_standard: String,
    pub target_arch: TargetArch,
    pub compiler: Compiler,
    pub build_backend: BuildBackend,
    pub cmake_min_version: String,
    pub exclude_dirs: Vec<String>,
    pub output_dir: PathBuf,
    pub enable_watch: bool,
    pub modules: Vec<CMakeModule>,
    pub generated_at: String,
    pub cmake_presets: Option<CMakePresets>,
    pub toolchain_files: Vec<PathBuf>,
    /// Toolchain configuration for cross-compilation targets.
    /// `None` when the target architecture does not require a toolchain file.
    pub toolchain: Option<ToolchainConfig>,
    /// Build system: CMake (default) or Zig.
    #[serde(default)]
    pub build_system: BuildSystem,
}

impl Default for ProjectConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            version: String::from("0.1.0"),
            root: PathBuf::from("."),
            language: String::from("CXX"),
            c_standard: String::from("11"),
            cpp_standard: String::from("17"),
            target_arch: TargetArch::X86_64,
            compiler: Compiler::GCC,
            build_backend: BuildBackend::default(),
            cmake_min_version: String::from("3.16"),
            exclude_dirs: vec![],
            output_dir: PathBuf::from("build"),
            enable_watch: false,
            modules: vec![],
            generated_at: String::new(),
            cmake_presets: None,
            toolchain_files: vec![],
            toolchain: None,
            build_system: BuildSystem::default(),
    }
    }
}

/// A snapshot of the full dependency structure at a point in time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencySnapshot {
    pub nodes: Vec<String>,
    pub edges: Vec<(String, String, crate::models::dependency::DependencyType)>,
}

/// Runtime metadata about the project, for caching and incremental updates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectMeta {
    pub config: ProjectConfig,
    pub modules: Vec<CMakeModule>,
    pub dependency_graph: DependencySnapshot,
    pub file_checksums: HashMap<String, String>,
    pub last_sync: String,
}

/// Parsed content of CMakePresets.json.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CMakePresets {
    pub version: u32,
    pub configure_presets: Vec<ConfigurePreset>,
    pub build_presets: Vec<BuildPreset>,
}

/// A configure preset from CMakePresets.json.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigurePreset {
    pub name: String,
    pub generator: Option<String>,
    pub toolchain_file: Option<String>,
    pub binary_dir: Option<String>,
    /// Name of another configure preset to inherit from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherits: Option<String>,
    /// Whether this preset should be hidden in GUI tools.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
    /// Cache variable overrides applied when this preset is selected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_variables: Option<HashMap<String, serde_json::Value>>,
}

/// A build preset from CMakePresets.json.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildPreset {
    pub name: String,
    pub configure_preset: String,
}

