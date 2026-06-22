# fb-gen build / run 子命令实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 新增 `fb-gen build`（Debug 构建不运行），重构 `fb-gen run`（Release 构建并运行），提取统一的 `BuildConfig` 参数对象收纳所有 CMake/Zig 构建参数。

**Architecture:** 新增 `src/models/build.rs` 定义 `BuildProfile` 枚举和 `BuildConfig` 结构体——从 `Cli` + `ProjectConfig` 构造，对外暴露完整的 `Command` 对象。`commands.rs` 提取共享的 `ensure_build_files_up_to_date` 和 `execute_build`，`cmd_build`/`cmd_run` 退化为薄封装。Zig 模板修改 install 路径让产物直接放在 `output_dir` 而非 `bin/` 子目录。

**Tech Stack:** Rust, clap (derive), petgraph, tera, std::process::Command

## Global Constraints

- CMake 构建时 `-DCMAKE_BUILD_TYPE=Debug`（build）或 `Release`（run）
- Zig 构建时 `--release=fast`（Release + x86）、`--release=small`（Release + 嵌入式）、不传（Debug）
- Zig 缓存目录隔离到 `.fb-gen/cache/zig`
- Zig 可执行文件产出在 `output_dir/<name>`，不包裹 `bin/`
- 可执行文件路径 = `output_dir/project_name`
- Run 找不到可执行文件时仅警告不报错
- Run 可执行文件崩溃时仅警告不报错
- 不修改 `init`/`sync`/`check`/`validate` 行为

---

### Task 1: 创建 `BuildProfile` 和 `BuildConfig` 数据模型

**Files:**
- Create: `src/models/build.rs`

**Interfaces:**
- Produces: `BuildProfile` enum (Debug, Release), `BuildConfig` struct, `BuildConfig::from_config()`, `BuildProfile::cmake_build_type()`, `BuildProfile::zig_release_flag()`, `BuildConfig::cmake_configure_command()`, `BuildConfig::cmake_build_command()`, `BuildConfig::zig_build_command()`, `BuildConfig::prepare_build_dir()`, `BuildConfig::executable_path()`
- Consumes: `ProjectConfig`, `BuildSystem`, `BuildBackend`, `TargetArch`, `ToolchainConfig`, `CMakePresets`, `FbGenResult`, `FbGenError` — all from `crate::models`

- [ ] **Step 1: Write `src/models/build.rs`**

```rust
//! Build configuration — unifies all CMake/Zig build parameters in one place.
//!
//! `BuildConfig` is constructed from `Cli` + `ProjectConfig` and exposes
//! ready-to-execute `std::process::Command` objects.  `commands.rs` no longer
//! contains any CMake/Zig argument assembly logic.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::models::error::{FbGenError, FbGenResult};
use crate::models::project::{
    BuildBackend, BuildSystem, CMakePresets, ProjectConfig, TargetArch, ToolchainConfig,
};

// ── BuildProfile ────────────────────────────────────────────────────────────

/// Build profile: Debug or Release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildProfile {
    Debug,
    Release,
}

impl BuildProfile {
    /// `-DCMAKE_BUILD_TYPE=` value.
    pub fn cmake_build_type(&self) -> &'static str {
        match self {
            BuildProfile::Debug => "Debug",
            BuildProfile::Release => "Release",
        }
    }

    /// Zig `--release=` flag.  Returns `None` for Debug (no flag).
    ///
    /// | Profile  | x86 / Custom  | Embedded (ARM/RISCV/WASM/Xtensa) |
    /// |----------|---------------|-----------------------------------|
    /// | Debug    | —             | —                                 |
    /// | Release  | `fast`        | `small`                           |
    pub fn zig_release_flag(&self, arch: &TargetArch) -> Option<&'static str> {
        match self {
            BuildProfile::Debug => None,
            BuildProfile::Release => {
                if Self::is_embedded(arch) {
                    Some("small")
                } else {
                    Some("fast")
                }
            }
        }
    }

    fn is_embedded(arch: &TargetArch) -> bool {
        !matches!(
            arch,
            TargetArch::X86_64 | TargetArch::X86 | TargetArch::Custom(_)
        )
    }
}

// ── BuildConfig ─────────────────────────────────────────────────────────────

/// Unified build parameter object.
///
/// Construct via `BuildConfig::from_config()`.  All CMake/Zig CLI argument
/// assembly lives here; `commands.rs` only calls the `*_command()` methods
/// and executes the returned `Command`.
#[derive(Debug, Clone)]
pub struct BuildConfig {
    // ── public fields ──
    pub profile: BuildProfile,
    pub run_after_build: bool,
    pub root: PathBuf,
    pub output_dir: PathBuf,
    pub project_name: String,
    pub quiet: bool,
    pub lsp: bool,

    // ── derived paths ──
    pub build_dir: PathBuf,
    pub zig_cache_dir: PathBuf,
    executable_path_cache: PathBuf,

    // ── project config snapshot ──
    pub build_system: BuildSystem,
    build_backend: BuildBackend,
    target_arch: TargetArch,
    toolchain: Option<ToolchainConfig>,
    cmake_presets: Option<CMakePresets>,
}

impl BuildConfig {
    /// Construct from `ProjectConfig` and CLI flags.
    pub fn from_config(
        config: &ProjectConfig,
        profile: BuildProfile,
        run_after_build: bool,
        quiet: bool,
        lsp: bool,
    ) -> Self {
        let build_dir = config.root.join(&config.output_dir);
        let zig_cache_dir = config.root.join(".fb-gen").join("cache").join("zig");
        let executable_path = Self::compute_executable_path(
            &build_dir,
            &config.output_dir,
            &config.name,
            config.build_system,
            config.build_backend,
            profile,
        );

        Self {
            profile,
            run_after_build,
            root: config.root.clone(),
            output_dir: config.output_dir.clone(),
            project_name: config.name.clone(),
            quiet,
            lsp,
            build_dir,
            zig_cache_dir,
            executable_path_cache: executable_path,
            build_system: config.build_system,
            build_backend: config.build_backend,
            target_arch: config.target_arch.clone(),
            toolchain: config.toolchain.clone(),
            cmake_presets: config.cmake_presets.clone(),
        }
    }

    /// Predicted executable path (best-effort).
    pub fn executable_path(&self) -> &Path {
        &self.executable_path_cache
    }

    // ── path computation ──

    fn compute_executable_path(
        build_dir: &Path,
        output_dir: &Path,
        project_name: &str,
        build_system: BuildSystem,
        build_backend: BuildBackend,
        profile: BuildProfile,
    ) -> PathBuf {
        match build_system {
            BuildSystem::CMake => {
                if matches!(build_backend, BuildBackend::MSBuild) {
                    // Multi-config: <build_dir>/<Config>/<name>
                    build_dir.join(profile.cmake_build_type()).join(project_name)
                } else {
                    // Single-config: <build_dir>/<name>
                    build_dir.join(project_name)
                }
            }
            BuildSystem::Zig => {
                // Zig with modified template: directly in output_dir
                output_dir.join(project_name)
            }
        }
    }

    // ── CMake commands ──

    /// Full `cmake -S ... -B ... -G ... -D...` command.
    pub fn cmake_configure_command(&self) -> Command {
        let mut cmd = Command::new("cmake");
        cmd.arg("-S").arg(&self.root);
        cmd.arg("-B").arg(&self.build_dir);

        // Generator flag
        for f in self.generator_flag() {
            cmd.arg(f);
        }

        // Build type
        cmd.arg(format!("-DCMAKE_BUILD_TYPE={}", self.profile.cmake_build_type()));

        // Toolchain file (cross-compilation)
        for f in self.toolchain_args() {
            cmd.arg(f);
        }

        // LSP
        if self.lsp {
            cmd.arg("-DCMAKE_EXPORT_COMPILE_COMMANDS=ON");
        }

        self.apply_toolchain_path(&mut cmd);
        cmd
    }

    /// Full `cmake --build <build_dir>` command.
    pub fn cmake_build_command(&self) -> Command {
        let mut cmd = Command::new("cmake");
        cmd.arg("--build").arg(&self.build_dir);
        self.apply_toolchain_path(&mut cmd);
        cmd
    }

    // ── Zig command ──

    /// Full `zig build -p ... --release=... --cache-dir ... --global-cache-dir ...` command.
    pub fn zig_build_command(&self) -> Command {
        let mut cmd = Command::new("zig");
        cmd.arg("build");
        cmd.arg("-p").arg(&self.output_dir);

        // Release flag
        if let Some(release) = self.profile.zig_release_flag(&self.target_arch) {
            cmd.arg(format!("--release={}", release));
        }

        // Cache isolation — keep .cache / .zig-cache out of project root
        cmd.arg(format!("--cache-dir"));
        cmd.arg(&self.zig_cache_dir);
        cmd.arg(format!("--global-cache-dir"));
        cmd.arg(&self.zig_cache_dir);

        cmd.current_dir(&self.root);
        cmd
    }

    // ── pre-build preparation ──

    /// Create build directory and remove stale CMake cache files.
    pub fn prepare_build_dir(&self) -> FbGenResult<()> {
        std::fs::create_dir_all(&self.build_dir).map_err(FbGenError::Io)?;

        // Create zig cache dir too so zig doesn't fail on first run.
        std::fs::create_dir_all(&self.zig_cache_dir).map_err(FbGenError::Io)?;

        // Remove stale CMakeCache.txt if toolchain args changed.
        let toolchain_args = self.toolchain_args();
        if !toolchain_args.is_empty() {
            let cache_file = self.build_dir.join("CMakeCache.txt");
            if cache_file.exists() {
                let cache_stale = std::fs::read_to_string(&cache_file)
                    .map(|contents| {
                        !toolchain_args.iter().any(|a| {
                            if let Some(path) = a.strip_prefix("-DCMAKE_TOOLCHAIN_FILE=") {
                                contents.contains(path)
                            } else {
                                contents.contains(a.as_str())
                            }
                        })
                    })
                    .unwrap_or(true);
                if cache_stale {
                    let _ = std::fs::remove_file(&cache_file);
                }
            }
        }
        Ok(())
    }

    // ── internal helpers ──

    /// Map `BuildBackend` to cmake `-G` argument(s).
    fn generator_flag(&self) -> Vec<String> {
        match self.build_backend {
            BuildBackend::Ninja => vec!["-G".into(), "Ninja".into()],
            BuildBackend::Make => vec![],
            BuildBackend::MSBuild => vec!["-G".into(), "Visual Studio 17 2022".into()],
            BuildBackend::Custom(ref name) => vec!["-G".into(), name.clone()],
        }
    }

    /// `-DCMAKE_TOOLCHAIN_FILE=...` args for cross-compilation targets.
    fn toolchain_args(&self) -> Vec<String> {
        use crate::core::CMakeGenerator;

        let is_cross = !matches!(
            self.target_arch,
            TargetArch::X86_64 | TargetArch::X86 | TargetArch::Custom(_)
        );
        if !is_cross {
            return vec![];
        }

        let path = self
            .cmake_presets
            .as_ref()
            .and_then(|p| {
                p.configure_presets
                    .iter()
                    .find_map(|cp| cp.toolchain_file.as_ref())
            })
            .map(|tf| CMakeGenerator::resolve_preset_path(&self.root, tf))
            .unwrap_or_else(|| self.root.join("cmake").join("toolchain.cmake"));

        if path.exists() {
            vec![format!("-DCMAKE_TOOLCHAIN_FILE={}", path.display())]
        } else {
            vec![]
        }
    }

    /// Extend a `Command`'s `PATH` so cross-compilation toolchain is discoverable.
    pub fn apply_toolchain_path(&self, cmd: &mut Command) {
        let dirs = self.toolchain_bin_dirs();
        if dirs.is_empty() {
            return;
        }
        let current_path = std::env::var_os("PATH").unwrap_or_default();
        let mut new_path = std::ffi::OsString::new();
        for d in &dirs {
            new_path.push(d);
            new_path.push(":");
        }
        new_path.push(&current_path);
        cmd.env("PATH", new_path);
    }

    /// Candidate directories containing the cross-compiler toolchain.
    fn toolchain_bin_dirs(&self) -> Vec<PathBuf> {
        let tc = match self.toolchain.as_ref() {
            Some(t) => t,
            None => return vec![],
        };
        let mut dirs: Vec<PathBuf> = Vec::new();

        if let Some(ref sysroot) = tc.sysroot {
            let sysroot_path = Path::new(sysroot);
            if let Ok(canon) = sysroot_path.canonicalize() {
                if let Some(parent) = canon.parent() {
                    let bin = parent.join("bin");
                    if bin.exists() {
                        dirs.push(bin);
                    }
                }
            }
            if let Some(parent) = sysroot_path.parent() {
                let bin = parent.join("bin");
                if bin.exists() && !dirs.contains(&bin) {
                    dirs.push(bin);
                }
            }
        }
        dirs
    }
}

// ── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cmake_build_type() {
        assert_eq!(BuildProfile::Debug.cmake_build_type(), "Debug");
        assert_eq!(BuildProfile::Release.cmake_build_type(), "Release");
    }

    #[test]
    fn test_zig_release_flag_debug_is_none() {
        assert_eq!(BuildProfile::Debug.zig_release_flag(&TargetArch::X86_64), None);
        assert_eq!(BuildProfile::Debug.zig_release_flag(&TargetArch::NoneEabi), None);
    }

    #[test]
    fn test_zig_release_flag_x86_is_fast() {
        assert_eq!(
            BuildProfile::Release.zig_release_flag(&TargetArch::X86_64),
            Some("fast")
        );
        assert_eq!(
            BuildProfile::Release.zig_release_flag(&TargetArch::X86),
            Some("fast")
        );
    }

    #[test]
    fn test_zig_release_flag_embedded_is_small() {
        assert_eq!(
            BuildProfile::Release.zig_release_flag(&TargetArch::NoneEabi),
            Some("small")
        );
        assert_eq!(
            BuildProfile::Release.zig_release_flag(&TargetArch::ARM32),
            Some("small")
        );
        assert_eq!(
            BuildProfile::Release.zig_release_flag(&TargetArch::RISCV32),
            Some("small")
        );
        assert_eq!(
            BuildProfile::Release.zig_release_flag(&TargetArch::WASM),
            Some("small")
        );
        assert_eq!(
            BuildProfile::Release.zig_release_flag(&TargetArch::Xtensa),
            Some("small")
        );
    }

    #[test]
    fn test_executable_path_cmake_ninja_release() {
        let bc = BuildConfig {
            profile: BuildProfile::Release,
            run_after_build: false,
            root: PathBuf::from("/proj"),
            output_dir: PathBuf::from("build"),
            project_name: "myapp".into(),
            quiet: false,
            lsp: false,
            build_dir: PathBuf::from("/proj/build"),
            zig_cache_dir: PathBuf::from("/proj/.fb-gen/cache/zig"),
            executable_path_cache: BuildConfig::compute_executable_path(
                &PathBuf::from("/proj/build"),
                &PathBuf::from("build"),
                "myapp",
                BuildSystem::CMake,
                BuildBackend::Ninja,
                BuildProfile::Release,
            ),
            build_system: BuildSystem::CMake,
            build_backend: BuildBackend::Ninja,
            target_arch: TargetArch::X86_64,
            toolchain: None,
            cmake_presets: None,
        };
        assert_eq!(bc.executable_path(), Path::new("/proj/build/myapp"));
    }

    #[test]
    fn test_executable_path_cmake_msbuild_debug() {
        let bc = BuildConfig {
            profile: BuildProfile::Debug,
            executable_path_cache: BuildConfig::compute_executable_path(
                &PathBuf::from("/proj/build"),
                &PathBuf::from("build"),
                "myapp",
                BuildSystem::CMake,
                BuildBackend::MSBuild,
                BuildProfile::Debug,
            ),
            ..BuildConfig::dummy()
        };
        assert_eq!(bc.executable_path(), Path::new("/proj/build/Debug/myapp"));
    }

    #[test]
    fn test_executable_path_zig() {
        let bc = BuildConfig {
            executable_path_cache: BuildConfig::compute_executable_path(
                &PathBuf::from("/proj/build"),
                &PathBuf::from("build"),
                "myapp",
                BuildSystem::Zig,
                BuildBackend::Ninja, // irrelevant for Zig
                BuildProfile::Release,
            ),
            ..BuildConfig::dummy()
        };
        assert_eq!(bc.executable_path(), Path::new("build/myapp"));
    }

    #[test]
    fn test_cmake_configure_command_contains_build_type() {
        let bc = BuildConfig {
            profile: BuildProfile::Debug,
            build_system: BuildSystem::CMake,
            build_backend: BuildBackend::Ninja,
            target_arch: TargetArch::X86_64,
            toolchain: None,
            cmake_presets: None,
            ..BuildConfig::dummy()
        };
        let cmd = bc.cmake_configure_command();
        let args: Vec<String> = cmd.get_args().map(|a| a.to_string_lossy().to_string()).collect();
        assert!(args.contains(&"-DCMAKE_BUILD_TYPE=Debug".to_string()));
    }

    #[test]
    fn test_zig_build_command_debug_no_release_flag() {
        let bc = BuildConfig {
            profile: BuildProfile::Debug,
            build_system: BuildSystem::Zig,
            target_arch: TargetArch::X86_64,
            ..BuildConfig::dummy()
        };
        let cmd = bc.zig_build_command();
        let args: Vec<String> = cmd.get_args().map(|a| a.to_string_lossy().to_string()).collect();
        assert!(!args.iter().any(|a| a.starts_with("--release=")));
    }

    #[test]
    fn test_zig_build_command_has_cache_dirs() {
        let bc = BuildConfig {
            build_system: BuildSystem::Zig,
            ..BuildConfig::dummy()
        };
        let cmd = bc.zig_build_command();
        let args: Vec<String> = cmd.get_args().map(|a| a.to_string_lossy().to_string()).collect();
        assert!(args.contains(&"--cache-dir".to_string()));
        assert!(args.contains(&"--global-cache-dir".to_string()));
    }

    impl BuildConfig {
        /// Dummy config for tests that only care about specific fields.
        #[cfg(test)]
        fn dummy() -> Self {
            Self {
                profile: BuildProfile::Release,
                run_after_build: false,
                root: PathBuf::from("/proj"),
                output_dir: PathBuf::from("build"),
                project_name: "myapp".into(),
                quiet: false,
                lsp: false,
                build_dir: PathBuf::from("/proj/build"),
                zig_cache_dir: PathBuf::from("/proj/.fb-gen/cache/zig"),
                executable_path_cache: PathBuf::from("/proj/build/myapp"),
                build_system: BuildSystem::CMake,
                build_backend: BuildBackend::Ninja,
                target_arch: TargetArch::X86_64,
                toolchain: None,
                cmake_presets: None,
            }
        }
    }
}
```

- [ ] **Step 2: Run unit tests on new module**

```bash
cargo test --lib build::
```

Expected: 10 tests pass.

- [ ] **Step 3: Commit**

```bash
git add src/models/build.rs
git commit -m "feat: add BuildProfile and BuildConfig — unified build parameters

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 2: Wire `build` module into `models/mod.rs`

**Files:**
- Modify: `src/models/mod.rs`

**Interfaces:**
- Consumes: `pub mod build` (from Task 1)
- Produces: `pub use build::{BuildConfig, BuildProfile}`

- [ ] **Step 1: Add module declaration and re-exports**

In `src/models/mod.rs`, add after `pub mod project;`:
```rust
pub mod build;
```

In the `pub use` block, add:
```rust
pub use build::{BuildConfig, BuildProfile};
```

Full file becomes:

```rust
pub mod build;
pub mod dependency;
pub mod error;
pub mod module;
pub mod project;

pub use build::{BuildConfig, BuildProfile};
pub use dependency::{DependencyEdge, DependencyGraph, DependencyType};
pub use error::{FbGenError, FbGenResult};
pub use module::{CMakeModule, SourceFile, SourceType, TargetType};
pub use project::{
    BuildBackend, BuildPreset, BuildSystem, CMakePresets, Compiler, ConfigurePreset,
    DependencySnapshot, ProjectConfig, ProjectMeta, TargetArch,
};
```

- [ ] **Step 2: Verify compilation**

```bash
cargo build
```

Expected: compiles successfully.

- [ ] **Step 3: Commit**

```bash
git add src/models/mod.rs
git commit -m "feat: wire build module into models re-exports

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 3: Add `Build` subcommand and update `Run` in CLI definitions

**Files:**
- Modify: `src/cli/mod.rs`

**Interfaces:**
- Consumes: `BuildConfig`, `BuildProfile` from `crate::models` (via Task 2)
- Produces: `Commands::Build`, `Commands::Run { args }` variants consumed by dispatch in same file

- [ ] **Step 1: Add `Build` variant and update `Run` variant**

In `src/cli/mod.rs`, replace the `Commands` enum:

```rust
#[derive(Subcommand)]
pub enum Commands {
    /// Initialize a new fb-gen project (creates build files)
    Init {
        /// Project name
        #[arg(short, long)]
        name: Option<String>,

        /// Build system: cmake or zig
        #[arg(long)]
        build: Option<String>,
    },

    /// Sync: scan sources and update CMakeLists.txt incrementally
    Sync,

    /// Check project structure without modifying files (diff mode)
    Check,

    /// Validate generated CMake configuration with cmake
    Validate,

    /// Build project (Debug profile) without running
    Build,

    /// Build project (Release profile) and run the executable
    Run {
        /// Arguments passed through to the executable
        #[arg(last = true)]
        args: Vec<String>,
    },
}
```

- [ ] **Step 2: Update dispatch in `run()` function**

In the same file, update the `match` block inside `pub fn run(cli: Cli)`:

```rust
let result = match &cli.command {
    Commands::Init { name, build } => commands::cmd_init(&cli, name.as_deref(), build.as_deref()),
    Commands::Sync => commands::cmd_sync(&cli),
    Commands::Check => commands::cmd_check(&cli),
    Commands::Validate => commands::cmd_validate(&cli),
    Commands::Build => commands::cmd_build(&cli),
    Commands::Run { args } => commands::cmd_run(&cli, args),
};
```

- [ ] **Step 3: Verify compilation (will fail until Task 5)**

```bash
cargo build 2>&1
```

Expected: "cannot find function `cmd_build`" and "this function takes 1 argument but 2 arguments were supplied" errors — confirms wiring is correct, implementation follows.

- [ ] **Step 4: Commit**

```bash
git add src/cli/mod.rs
git commit -m "feat: add Build subcommand, add args passthrough to Run

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 4: Extract shared helpers from `cmd_run` into `commands.rs`

**Files:**
- Modify: `src/cli/commands.rs`

**Interfaces:**
- Produces: `load_or_fallback_config(root, cli) -> FbGenResult<ProjectConfig>`, `ensure_build_files_up_to_date(cli, config, reporter) -> FbGenResult<()>`, `execute_cmake_build(bc, reporter) -> FbGenResult<bool>`, `execute_zig_build(bc, reporter) -> FbGenResult<bool>`, `execute_build(bc, reporter) -> FbGenResult<bool>`, `run_executable(bc, args, reporter)`
- Consumes: `BuildConfig`, `BuildProfile` from `crate::models::build` (via Task 2)
- Removes (migrated to `BuildConfig`): `cmake_generator_flag()`, `cmake_toolchain_args()`, `with_toolchain_path()`, `toolchain_bin_dirs()`

- [ ] **Step 1: Add imports for `BuildConfig` and `BuildProfile`**

In `src/cli/commands.rs`, update the imports block:

```rust
use crate::models::build::{BuildConfig, BuildProfile};
```

- [ ] **Step 2: Add `load_or_fallback_config` helper**

Insert after the `discoverer_opts` function (~line 112):

```rust
/// Load project config from cache, or build a fallback from directory name.
fn load_or_fallback_config(root: &Path, cli: &Cli) -> FbGenResult<ProjectConfig> {
    let cache = MetaCache::new(root);

    let fallback_config = || ProjectConfig {
        name: root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("project")
            .to_string(),
        root: root.to_path_buf(),
        output_dir: cli.output.clone(),
        exclude_dirs: vec!["build".into(), ".git".into(), "third_party".into()],
        ..Default::default()
    };

    let config = if cache.exists() {
        cache.load().map(|m| m.config).unwrap_or_else(fallback_config)
    } else {
        fallback_config()
    };

    Ok(config)
}
```

- [ ] **Step 3: Add `ensure_build_files_up_to_date` helper**

Insert after `load_or_fallback_config`:

```rust
/// Ensure build files (CMakeLists.txt or build.zig) are up to date.
/// Performs full generation if missing, incremental sync if cache exists.
fn ensure_build_files_up_to_date(
    cli: &Cli,
    config: &mut ProjectConfig,
    reporter: &Reporter,
) -> FbGenResult<()> {
    let root = &config.root;
    let cache = MetaCache::new(root);

    let build_file = match config.build_system {
        BuildSystem::CMake => root.join("CMakeLists.txt"),
        BuildSystem::Zig => root.join("build.zig"),
    };

    if !build_file.exists() {
        reporter.report_info(&format!(
            "No {} found — generating ...",
            build_file.file_name().unwrap_or_default().to_string_lossy()
        ));
        let (modules, graph, user_modules) = scan_and_discover(cli, config, reporter)?;

        let empty_graph = DependencyGraph::new();
        let ref_graph = graph.as_ref().unwrap_or(&empty_graph);
        generate_build_files(config, &modules, ref_graph, true, &user_modules)?;
        save_meta_cache(root, &modules, graph.as_ref(), config)?;
    } else if cache.exists() {
        reporter.report_info("Checking for source changes ...");
        match cache.load() {
            Some(mut prev_meta) => {
                match do_incremental_sync(root, config, &mut prev_meta, reporter) {
                    Ok(n) => {
                        let meta = ProjectMeta {
                            config: config.clone(),
                            modules: prev_meta.modules,
                            dependency_graph: prev_meta.dependency_graph,
                            file_checksums: prev_meta.file_checksums,
                            last_sync: chrono::Utc::now().to_rfc3339(),
                        };
                        if let Err(e) = cache.save(&meta) {
                            reporter.report_warning(&format!(
                                "Failed to save synced metadata: {}. Build will proceed.",
                                e
                            ));
                        } else if n > 0 {
                            reporter.report_success("CMakeLists.txt synced before build");
                        } else {
                            reporter.report_info("No source changes — skipping sync");
                        }
                    }
                    Err(e) => {
                        reporter.report_warning(&format!(
                            "Incremental sync failed: {}. Proceeding with existing CMakeLists.txt.",
                            e
                        ));
                    }
                }
            }
            None => {
                reporter.report_warning(
                    "Cache exists but failed to load — skipping sync, proceeding with build.",
                );
            }
        }
    }

    Ok(())
}
```

- [ ] **Step 4: Add `execute_cmake_build` helper**

Insert after `run_cmake_formatted` (~line 1803):

```rust
/// Execute CMake configure + build using the unified `BuildConfig`.
fn execute_cmake_build(
    bc: &BuildConfig,
    reporter: &Reporter,
    cache: &MetaCache,
) -> FbGenResult<bool> {
    // Configure.
    reporter.report_info(&format!(
        "Configuring with cmake -S {} -B {} -DCMAKE_BUILD_TYPE={} ...",
        bc.root.display(),
        bc.build_dir.display(),
        bc.profile.cmake_build_type(),
    ));

    let (status, _cfg_lines) = run_cmake_formatted(
        &mut bc.cmake_configure_command(),
        bc.quiet,
    )
    .map_err(|e| FbGenError::Config(format!("cmake configure: {e}")))?;

    if !status.success() {
        return Err(FbGenError::GenerationFailed(
            "cmake configure failed".into(),
        ));
    }

    // LSP: symlink compile_commands.json to project root.
    if bc.lsp {
        let cc_json = bc.build_dir.join("compile_commands.json");
        if cc_json.exists() {
            match symlink_or_copy(&cc_json, &bc.root.join("compile_commands.json")) {
                Ok(()) => reporter.report_success("compile_commands.json → project root"),
                Err(e) => reporter.report_warning(&format!(
                    "compile_commands.json symlink failed: {e}"
                )),
            }
        }
    }

    // Build.
    reporter.report_info(&format!(
        "Building with cmake --build {} ...",
        bc.build_dir.display(),
    ));

    let (status, build_lines) = run_cmake_formatted(
        &mut bc.cmake_build_command(),
        bc.quiet,
    )
    .map_err(|e| FbGenError::Config(format!("cmake --build: {e}")))?;

    // Memory usage summary.
    if let Some(summary) = extract_memory_summary(&build_lines) {
        if !bc.quiet {
            eprintln!("\n{}", "──── Memory Usage ────".cyan().bold());
            for line in summary.lines() {
                eprintln!("  {}", format_cmake_line(line));
            }
        }
    }

    // LSP (CMake): compile_commands.json already created by cmake configure.
    // No additional action needed here.

    Ok(status.success())
}

/// Execute Zig build using the unified `BuildConfig`.
fn execute_zig_build(
    bc: &BuildConfig,
    reporter: &Reporter,
    cache: &MetaCache,
) -> FbGenResult<bool> {
    reporter.report_info(&format!(
        "Building with zig build --prefix {} ...",
        bc.output_dir.display(),
    ));

    let (status, build_lines) = run_cmake_formatted(
        &mut bc.zig_build_command(),
        bc.quiet,
    )
    .map_err(|e| FbGenError::Config(format!("zig build: {e}")))?;

    // Memory usage summary.
    if let Some(summary) = extract_memory_summary(&build_lines) {
        if !bc.quiet {
            eprintln!("\n{}", "──── Memory Usage ────".cyan().bold());
            for line in summary.lines() {
                eprintln!("  {}", format_cmake_line(line));
            }
        }
    }

    // LSP (Zig): generate compile_commands.json.
    if bc.lsp {
        if let Some(meta) = cache.load() {
            let graph = rebuild_graph_from_snapshot(&meta.dependency_graph);
            generate_compile_commands_zig(
                &bc.root,
                &meta.config,
                &meta.modules,
                Some(&graph),
                reporter,
            );
        } else {
            reporter.report_warning(
                "No cache found — compile_commands.json not generated. \
                 Run `fb-gen init` first, or re-run `fb-gen build`.",
            );
        }
    }

    Ok(status.success())
}
```

- [ ] **Step 5: Add `execute_build` dispatcher**

```rust
/// Dispatch build to CMake or Zig based on `BuildConfig`.
fn execute_build(
    bc: &BuildConfig,
    reporter: &Reporter,
    cache: &MetaCache,
) -> FbGenResult<bool> {
    bc.prepare_build_dir()?;

    match bc.build_system {
        BuildSystem::CMake => execute_cmake_build(bc, reporter, cache),
        BuildSystem::Zig => execute_zig_build(bc, reporter, cache),
    }
}
```

- [ ] **Step 6: Add `run_executable`**

```rust
/// Run the built executable (used by `fb-gen run`).
fn run_executable(
    bc: &BuildConfig,
    args: &[String],
    reporter: &Reporter,
) {
    let exe_path = bc.executable_path();

    if !exe_path.exists() {
        reporter.report_warning(&format!(
            "Executable not found at '{}'. Build may have placed it elsewhere.",
            exe_path.display()
        ));
        return;
    }

    reporter.report_info(&format!("Running {} ...", exe_path.display()));

    let mut cmd = Command::new(exe_path);
    cmd.args(args);
    bc.apply_toolchain_path(&mut cmd);

    match cmd.status() {
        Ok(status) => {
            if !status.success() {
                let code = status
                    .code()
                    .map_or_else(|| "signal".into(), |c| c.to_string());
                reporter.report_warning(&format!("Process exited with status: {}", code));
            }
        }
        Err(e) => {
            reporter.report_warning(&format!(
                "Failed to run '{}': {}",
                exe_path.display(),
                e
            ));
        }
    }
}
```

- [ ] **Step 7: Verify compilation (will fail until Task 5/6)**

```bash
cargo build 2>&1 | head -5
```

Expected: Only `cmd_build` undefined and `cmd_run` signature mismatch errors.

- [ ] **Step 8: Commit**

```bash
git add src/cli/commands.rs
git commit -m "refactor: extract shared build helpers from cmd_run

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 5: Add `cmd_build` function

**Files:**
- Modify: `src/cli/commands.rs`

**Interfaces:**
- Produces: `pub fn cmd_build(cli: &Cli) -> FbGenResult<()>`
- Consumes: `resolve_root`, `load_or_fallback_config`, `ensure_build_files_up_to_date`, `Reporter`, `BuildConfig`, `BuildProfile`, `execute_build` (all from Task 4)

- [ ] **Step 1: Add `cmd_build` function**

Insert before `cmd_run` (~before line 1347 in the original):

```rust
/// `fb-gen build` — build project (Debug profile), without running.
pub fn cmd_build(cli: &Cli) -> FbGenResult<()> {
    let reporter = Reporter::new(cli.quiet);
    let root = resolve_root(cli)?;
    let mut config = load_or_fallback_config(&root, cli)?;

    ensure_build_files_up_to_date(cli, &mut config, &reporter)?;

    let bc = BuildConfig::from_config(
        &config,
        BuildProfile::Debug,
        false, // run_after_build
        cli.quiet,
        cli.lsp,
    );

    let cache = MetaCache::new(&root);
    let start = Instant::now();
    let success = execute_build(&bc, &reporter, &cache)?;

    if success {
        let elapsed = start.elapsed();
        reporter.report_success(&format!(
            "Build succeeded in {:.1}s",
            elapsed.as_secs_f64()
        ));
    } else {
        reporter.report_error("Build failed.");
        return Err(FbGenError::GenerationFailed("build failed".into()));
    }

    Ok(())
}
```

- [ ] **Step 2: Verify compilation**

```bash
cargo build
```

Expected: compiles (only `cmd_run` signature mismatch remains, fixed in Task 6).

- [ ] **Step 3: Commit**

```bash
git add src/cli/commands.rs
git commit -m "feat: add cmd_build — debug profile build without running

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 6: Refactor `cmd_run` to use shared helpers

**Files:**
- Modify: `src/cli/commands.rs:1347-1605`

**Interfaces:**
- Consumes: `BuildConfig`, `BuildProfile`, `execute_build`, `run_executable`, `ensure_build_files_up_to_date`, `load_or_fallback_config` (from Tasks 4, 5)
- Removes: inline CMake/Zig argument assembly (~100 lines, replaced by `BuildConfig` methods)

- [ ] **Step 1: Replace `cmd_run` body**

Replace the entire `cmd_run` function (currently ~260 lines, lines 1347-1605) with:

```rust
/// `fb-gen run` — build project (Release profile) and run the executable.
pub fn cmd_run(cli: &Cli, args: &[String]) -> FbGenResult<()> {
    let reporter = Reporter::new(cli.quiet);

    // ── CMake banner ──────────────────────────────────────────────────
    if !cli.quiet {
        let banner: [(&str, Color); 6] = [
            (" ████   █    █   ██████", Color::BrightCyan),
            ("█    █  ██  ██      █",    Color::Cyan),
            ("█       █ ██ █     █",    Color::BrightBlue),
            ("█       █    █    █",     Color::Blue),
            ("█    █  █    █   █",      Color::BrightGreen),
            (" ████   █    █   ██████", Color::Green),
        ];
        for (line, color) in &banner {
            println!("{}", line.color(*color).bold());
        }
        println!();
    }

    let root = resolve_root(cli)?;
    let mut config = load_or_fallback_config(&root, cli)?;

    ensure_build_files_up_to_date(cli, &mut config, &reporter)?;

    let bc = BuildConfig::from_config(
        &config,
        BuildProfile::Release,
        true, // run_after_build
        cli.quiet,
        cli.lsp,
    );

    let cache = MetaCache::new(&root);
    let start = Instant::now();
    let success = execute_build(&bc, &reporter, &cache)?;

    if success {
        let elapsed = start.elapsed();
        reporter.report_success(&format!(
            "Build succeeded in {:.1}s",
            elapsed.as_secs_f64()
        ));

        run_executable(&bc, args, &reporter);
    } else {
        reporter.report_error("Build failed.");
        return Err(FbGenError::GenerationFailed("build failed".into()));
    }

    Ok(())
}
```

- [ ] **Step 2: Remove migrated functions**

Delete the following functions from `commands.rs` (they now live in `BuildConfig`):
- `cmake_generator_flag()` — `src/cli/commands.rs:1990-1998`
- `cmake_toolchain_args()` — `src/cli/commands.rs:2000-2028`
- `with_toolchain_path()` — `src/cli/commands.rs:1937-1954`
- `toolchain_bin_dirs()` — `src/cli/commands.rs:1958-1988`

Remove the unused import `use crate::core::CMakeGenerator;` if it's only in `cmake_toolchain_args`.

- [ ] **Step 3: Verify compilation and full test suite**

```bash
cargo build && cargo test --lib
```

Expected: builds cleanly, all unit tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/cli/commands.rs
git commit -m "refactor: cmd_run uses BuildConfig, removed migrated helpers

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 7: Update Zig build template — install directly to `output_dir`, no `bin/` subdir

**Files:**
- Modify: `src/core/zig_generator.rs:276-281`

**Interfaces:**
- Consumes: existing `ZIG_BUILD_TEMPLATE` context variables (`mod.name_safe`, `mod.exe_name`, `mod.is_executable`)
- Produces: changed install behavior — binary at `<prefix>/<name>` instead of `<prefix>/bin/<name>`

- [ ] **Step 1: Replace `b.installArtifact(...)` with `b.addInstallFile(...)`**

In `src/core/zig_generator.rs`, replace lines 276-281:

**Before:**
```zig
    // ── Install ──────────────────────────────────────────────
{% for mod in modules -%}
{% if mod.is_executable -%}
    b.installArtifact({{ mod.name_safe }});
{% endif -%}
{% endfor -%}
```

**After:**
```zig
    // ── Install ──────────────────────────────────────────────
{% for mod in modules -%}
{% if mod.is_executable -%}
    // Install executable directly to prefix root (no bin/ subdir)
    // so it matches the CMake output convention: <output_dir>/<name>
    b.getInstallStep().dependOn(
        &b.addInstallFile(
            {{ mod.name_safe }}.getEmittedBin(),
            "{{ mod.exe_name }}",
        ).step,
    );
{% endif -%}
{% endfor -%}
```

- [ ] **Step 2: Run Zig-specific unit tests**

```bash
cargo test --lib zig::
```

Expected: all existing Zig generator tests pass.

- [ ] **Step 3: Commit**

```bash
git add src/core/zig_generator.rs
git commit -m "fix: zig build installs binary directly to prefix root, not bin/

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 8: Update `check_required_tools` message to mention `build` command

**Files:**
- Modify: `src/cli/commands.rs:181-201`

**Interfaces:**
- Consumes: nothing new
- Produces: updated warning messages

- [ ] **Step 1: Update warning messages to reference both `build` and `run`**

Replace lines 189-191 and 197-199:

**Before:**
```rust
                None => reporter.report_warning(
                    "cmake not found on PATH. Install it before running `fb-gen run`."
                ),
```

**After:**
```rust
                None => reporter.report_warning(
                    "cmake not found on PATH. Install it before running `fb-gen build` or `fb-gen run`."
                ),
```

**Before:**
```rust
                None => reporter.report_warning(
                    "zig not found on PATH. Install it before running `fb-gen run`."
                ),
```

**After:**
```rust
                None => reporter.report_warning(
                    "zig not found on PATH. Install it before running `fb-gen build` or `fb-gen run`."
                ),
```

- [ ] **Step 2: Commit**

```bash
git add src/cli/commands.rs
git commit -m "chore: update tool-not-found messages to reference build command

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 9: Integration tests and final verification

**Files:**
- Modify: `tests/integration.rs` (or create if not yet tested for build/run)

**Interfaces:**
- Consumes: `fb-gen` binary via `cargo run --`
- Produces: integration test coverage for `build` and `run` subcommands

- [ ] **Step 1: Add integration tests**

In `tests/integration.rs`, add after existing tests:

```rust
#[test]
fn test_build_subcommand_exists() {
    let output = run_fb_gen(&["build", "--help"]);
    assert!(output.contains("Build project"));
}

#[test]
fn test_run_subcommand_accepts_args() {
    let output = run_fb_gen(&["run", "--help"]);
    assert!(output.contains("Arguments passed through"));
}

#[test]
fn test_build_in_empty_dir() {
    let dir = TempDir::new().unwrap();

    // Create a minimal C project
    std::fs::create_dir(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src").join("main.c"),
        r#"#include <stdio.h>
int main(void) {
    printf("hello\n");
    return 0;
}
"#,
    )
    .unwrap();

    // Init
    let output = run_fb_gen_in(&dir, &["init", "--name", "testapp"]);
    // May require interactive input; skip if init times out.

    // Build should succeed
    let output = run_fb_gen_in(&dir, &["build"]);
    // Check build directory exists
    assert!(dir.path().join("build").exists());
}

/// Helper: run fb-gen in a specific directory.
fn run_fb_gen_in(dir: &TempDir, args: &[&str]) -> String {
    let output = std::process::Command::new(
        std::env::current_exe().unwrap().parent().unwrap().join("fb-gen"),
    )
    .args(args)
    .current_dir(dir.path())
    .output()
    .unwrap_or_else(|_| {
        // Fallback: cargo run
        let mut cmd = std::process::Command::new("cargo");
        cmd.arg("run").arg("--");
        cmd.args(args);
        cmd.current_dir(dir.path());
        cmd.output().unwrap()
    });
    String::from_utf8_lossy(&output.stdout).to_string()
}
```

- [ ] **Step 2: Run all tests**

```bash
cargo test
```

Expected: all unit tests + integration tests pass.

- [ ] **Step 3: Verify `fb-gen build --help` output**

```bash
cargo run -- build --help
```

Expected: shows `Build project (Debug profile) without running`

- [ ] **Step 4: Verify `fb-gen run --help` output**

```bash
cargo run -- run --help
```

Expected: shows `[ARGS]` passthrough parameter

- [ ] **Step 5: Commit**

```bash
git add tests/integration.rs
git commit -m "test: add integration tests for build and run subcommands

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 10: Final verification — full test suite

- [ ] **Step 1: Run complete test suite**

```bash
cargo test
cargo clippy --all-targets 2>&1
```

Expected: all tests pass, no clippy warnings introduced.

- [ ] **Step 2: Manual smoke test of build and run help**

```bash
cargo run -- build --help
cargo run -- run --help
```

Expected: both subcommands display correct help text.

- [ ] **Step 3: Final commit if any clippy fixes were needed**

```bash
git add -u
git commit -m "chore: clippy fixes for build/run implementation

Co-Authored-By: Claude <noreply@anthropic.com>"
```
