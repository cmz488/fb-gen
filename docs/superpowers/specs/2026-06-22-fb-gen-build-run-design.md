# fb-gen build / run 子命令设计

**日期**: 2026-06-22
**状态**: 已设计

## 概述

新增 `fb-gen build` 子命令（Debug profile 构建，不运行），重构 `fb-gen run`（Release profile 构建并运行）。同时将散落在 `commands.rs` 中的所有 CMake/Zig 构建参数统一收敛到 `BuildConfig` 对象中。

## 动机

- 当前只有 `run` 子命令，没有独立的 `build`
- `run` 构建但不运行可执行文件（没有实际运行行为）
- 所有 CMake/Zig 构建参数硬编码在 `commands.rs` 的 `cmd_run` 中，分散不易维护
- Zig 构建产生的 `.cache` / `.zig-cache` 污染项目根目录
- Zig 构建产物被包裹在 `bin/` 子目录，与 CMake 行为不一致

## CLI 接口

### Commands enum 变更

```rust
pub enum Commands {
    Init { ... },
    Sync,
    Check,
    Validate,
    /// 构建项目（Debug profile），不运行
    Build,
    /// 构建项目（Release profile）并运行可执行文件
    Run {
        /// 传递给可执行文件的额外参数
        #[arg(last = true)]
        args: Vec<String>,
    },
}
```

### 行为差异

| | `fb-gen build` | `fb-gen run` |
|---|---|---|
| CMake BUILD_TYPE | `Debug` | `Release` |
| Zig --release | 不传（debug 模式） | `fast`（x86）/ `small`（嵌入式） |
| 运行可执行文件 | 否 | 是 |
| 透传参数 | - | `fb-gen run -- <args>` |

## 数据模型

### BuildProfile

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildProfile {
    Debug,
    Release,
}
```

### BuildConfig

新增 `src/models/build.rs`，在 `models/mod.rs` 中 re-export。

收纳所有构建参数，对外暴露完整的可执行 `Command` 对象。`commands.rs` 不再包含 CMake/Zig 参数拼装逻辑。

```rust
#[derive(Debug, Clone)]
pub struct BuildConfig {
    // 基础信息
    pub profile: BuildProfile,
    pub run_after_build: bool,
    pub root: PathBuf,
    pub output_dir: PathBuf,
    pub project_name: String,
    pub quiet: bool,
    pub lsp: bool,

    // 派生路径
    pub build_dir: PathBuf,        // root / output_dir
    pub zig_cache_dir: PathBuf,    // root / .fb-gen/cache/zig
    pub executable_path: PathBuf,  // 见下文路径解析

    // 项目配置快照
    build_system: BuildSystem,
    build_backend: BuildBackend,
    target_arch: TargetArch,
    toolchain: Option<ToolchainConfig>,
    cmake_presets: Option<CMakePresets>,
}
```

### 可执行文件路径解析

```rust
pub fn executable_path(&self) -> PathBuf {
    match self.build_system {
        BuildSystem::CMake => {
            if matches!(self.build_backend, BuildBackend::MSBuild) {
                // Multi-config generator: build_dir/<Config>/<name>
                self.build_dir.join(self.profile.cmake_build_type()).join(&self.project_name)
            } else {
                // Single-config: build_dir/<name>
                self.build_dir.join(&self.project_name)
            }
        }
        BuildSystem::Zig => {
            // Zig: 产物直接放在 output_dir（不在 bin/ 子目录）
            self.output_dir.join(&self.project_name)
        }
    }
}
```

### 构建参数完整迁移表

所有参数从 `commands.rs` 迁移到 `BuildConfig` 方法中：

| # | 参数 | BuildConfig 方法 |
|---|---|---|
| 1 | `cmake -S <root>` | `cmake_configure_command()` |
| 2 | `cmake -B <build_dir>` | `cmake_configure_command()` |
| 3 | `cmake -G <generator>` | 内部 `generator_flag()` — 从 `cmake_generator_flag()` 迁移 |
| 4 | `-DCMAKE_BUILD_TYPE=Debug\|Release` | 新增，`cmake_configure_command()` 内部 |
| 5 | `-DCMAKE_TOOLCHAIN_FILE=...` | 内部 `toolchain_args()` — 从 `cmake_toolchain_args()` 迁移 |
| 6 | `-DCMAKE_EXPORT_COMPILE_COMMANDS=ON` | `cmake_configure_command()` 当 `lsp=true` |
| 7 | PATH 追加 toolchain bin | 内部 `apply_toolchain_path()` — 从 `with_toolchain_path()` + `toolchain_bin_dirs()` 迁移 |
| 8 | `cmake --build <build_dir>` | `cmake_build_command()` |
| 9 | cmake --build PATH 追加 | `cmake_build_command()` 内部 |
| 10 | `zig build` | `zig_build_command()` |
| 11 | `zig build -p <output_dir>` | `zig_build_command()` 内部 |
| 12 | `--release=small`（嵌入式） | `zig_build_command()` 内部，`BuildProfile::zig_release_flag()` 决议 |
| 13 | `--release=fast`（x86 Release） | 新增，同上 |
| 14 | Debug 无 --release | 新增，同上 |
| 15 | `--cache-dir <zig_cache_dir>` | 新增，隔离到 `.fb-gen/cache/zig` |
| 16 | `--global-cache-dir <zig_cache_dir>` | 新增，同上 |
| 17 | `current_dir(root)` | `zig_build_command()` 内部 |
| 18 | CMakeCache.txt 过期清理 | `prepare_build_dir()` — 从 `cmd_run` 迁移 |
| 19 | `create_dir_all(build_dir)` | `prepare_build_dir()` |
| 20 | Zig 产物不包裹 bin/ | 修改 `ZIG_BUILD_TEMPLATE`，`dest_dir` 指向 prefix 根 |
| 21 | 可执行文件路径 | `executable_path()` |
| 22 | 构建输出格式化 | 保留在 `commands.rs`（`BuildConfig` 只产出 `Command`） |

## 实现结构

### 文件变更

| 文件 | 变更 |
|---|---|
| `src/models/build.rs` | **新增** — `BuildProfile`, `BuildConfig` |
| `src/models/mod.rs` | 新增 `pub mod build;` + re-export |
| `src/cli/mod.rs` | `Commands` enum 加 `Build`，`Run` 加 `args` 字段 |
| `src/cli/commands.rs` | 新增 `cmd_build`，重构 `cmd_run`，新增 `ensure_build_files_up_to_date`，新增 `execute_build`，新增 `run_executable`，移除 `cmake_generator_flag`/`cmake_toolchain_args`/`with_toolchain_path`/`toolchain_bin_dirs`（迁移到 `BuildConfig`） |
| `src/core/zig_generator.rs` | 修改 `ZIG_BUILD_TEMPLATE`，产物安装到 prefix 根目录 |
| `src/lib.rs` | `build` module 已通过 `models` re-export 暴露 |

### commands.rs 重构后流程

```
cmd_build(cli)
  → resolve_root
  → load_or_fallback_config
  → ensure_build_files_up_to_date       (gen/sync 复用逻辑)
  → BuildConfig::from_config(..., Debug, run_after=false)
  → execute_build(bc, reporter)
  → Ok

cmd_run(cli, args)
  → resolve_root
  → load_or_fallback_config
  → ensure_build_files_up_to_date
  → BuildConfig::from_config(..., Release, run_after=true)
  → execute_build(bc, reporter)
  → run_executable(bc, args, reporter)   (仅当构建成功)
  → Ok
```

### 构建执行统一路径

```rust
fn execute_build(bc: &BuildConfig, reporter: &Reporter) -> FbGenResult<bool> {
    bc.prepare_build_dir()?;

    match bc.build_system {
        BuildSystem::CMake => execute_cmake_build(bc, reporter),
        BuildSystem::Zig  => execute_zig_build(bc, reporter),
    }
}
```

## 错误处理

| 场景 | 行为 |
|---|---|
| 构建文件不存在 | 自动执行完整生成 |
| 增量同步失败 | 警告 + 继续使用已有构建文件 |
| cmake configure 失败 | `Err`，终止 |
| cmake build / zig build 失败 | `Err`，终止，不尝试运行 |
| 可执行文件找不到 | 警告（不阻塞） |
| 可执行文件运行崩溃 | 警告 + 退出码（不报 `Err`） |
| 交叉编译目标 | 不阻止 — 用户可能用 QEMU / 远程 runner |

## 不涉及

- 不在 `build`/`run` 中修改 `ProjectConfig` 持久化
- 不新增 CLI 全局参数
- 不修改 `init`/`sync`/`check`/`validate` 的行为

## 测试策略

- **单元测试**：`BuildProfile::cmake_build_type()`、`zig_release_flag()` 各组合
- **单元测试**：`BuildConfig::cmake_configure_command()` / `cmake_build_command()` / `zig_build_command()` 产出的 `Command` 参数正确性
- **单元测试**：`executable_path()` 各组合（CMake+Ninja, CMake+MSBuild, Zig）
- **集成测试**：`fb-gen build` 在示例项目中产出 Debug 构建
- **集成测试**：`fb-gen run` 在示例项目中产出 Release 构建并执行
- **集成测试**：`fb-gen run -- <args>` 参数透传
