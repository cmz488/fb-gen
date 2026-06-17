# fb-gen — Fast Build Generate

<div align="center">
<code style="color:#00d4ff;font-weight:bold;"> ████   █    █   ██████</code><br>
<code style="color:#00bcd4;font-weight:bold;">█    █  ██  ██      █</code><br>
<code style="color:#5c9ce6;font-weight:bold;">█       █ ██ █     █</code><br>
<code style="color:#4a7fd4;font-weight:bold;">█       █    █    █</code><br>
<code style="color:#5cdb5c;font-weight:bold;">█    █  █    █   █</code><br>
<code style="color:#00a651;font-weight:bold;"> ████   █    █   ██████</code>
</div>
<p align="center" style="color:gray;">
  <b>C/C++ 构建系统自动生成工具</b>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/Language-rust-orange?logo=rust&logoColor=white" alt="Rust">
  <img src="https://img.shields.io/badge/Platform-Linux%20%7C%20Windows%20%7C%20macOS-informational?logo=linux&logoColor=white" alt="Platform">
  <img src="https://img.shields.io/badge/License-MIT-blue?logo=opensourceinitiative&logoColor=white" alt="License">
  <img src="https://img.shields.io/github/stars/cmz488/fb-gen?style=flat&label=Stars&color=FFC700&logo=github&logoColor=white" alt="Stars">
  <img src="https://img.shields.io/github/forks/cmz488/fb-gen?style=flat&label=Forks&color=60adff&logo=git-fork&logoColor=white" alt="Forks">
  <img src="https://img.shields.io/github/v/release/cmz488/fb-gen?color=32cd32&label=Release&logo=github-actions&logoColor=white" alt="Release">
  <img src="https://img.shields.io/github/last-commit/cmz488/fb-gen?color=rebeccapurple&logo=git&logoColor=white" alt="Last Commit">
  <img src="https://img.shields.io/github/commit-activity/m/cmz488/fb-gen?style=flat&color=FF69B4&logo=github" alt="Commit Activity">
  <img src="https://img.shields.io/github/languages/code-size/cmz488/fb-gen?style=flat&color=blueviolet" alt="Code Size">
</p>

自动扫描 C/C++ 项目目录结构，智能生成多模块 **CMake** 或 **Zig Build** 构建系统。

## 简介

`fb-gen` (Fast Build Generate) 是一个命令行工具，用于将 C/C++ 项目自动转化为结构清晰的构建系统。
你是否厌烦了 CMake 琐碎的脚本语言（它居然是图灵完备的），想拥有 Cargo 和 Rust 的优雅体验？本工具模仿 `mod.rs` 的构建方法，把每一个含有 `*.c` / `*.h` / `*.cpp` / `*.hpp` 的独立文件夹视为一个项目模块，根据模块间依赖图，生成构建配置文件。

**支持两种构建系统后端：**

- **CMake** — 传统的 CMakeLists.txt，兼容广泛的工具链和 IDE
- **Zig Build** — 使用 Zig 作为跨平台构建系统和 C/C++ 编译器，一套工具链搞定所有平台，特别适合嵌入式/裸机项目

**核心理念：**

- **发现即结构** — 每个包含 `.c`/`.cpp` 源文件的子目录自动成为一个独立的构建模块
- **零配置假定** — 无需额外配置文件，通过扫描目录结构和文件内容智能推断模块边界与依赖关系
- **内容感知** — 解析 `#include` 指令，自动建立模块间依赖图
- **增量友好** — 文件结构变化时可快速重新生成配置，适合活跃开发的代码仓库
- **多架构交叉编译** — 内置对 ARM Cortex-M、RISC-V、Xtensa (ESP32)、WASM 等嵌入式目标的一流支持

## 安装

```sh
# 从源码编译
git clone https://github.com/cmz488/fb-gen.git
cd fb-gen
cargo build --release

# 安装到系统路径
cargo install --path .
```

## 快速开始

```sh
# 初始化一个 CMake 项目（默认）
fb-gen init

# 初始化一个 Zig Build 项目
fb-gen init --build zig

# 运行完整流水线：扫描 → 分析 → 生成 → 构建
fb-gen run

# 启用 LSP 支持（生成 compile_commands.json）
fb-gen run --lsp
```

## 命令用法

### `fb-gen init`

初始化新项目，通过交互式问答收集项目配置，生成构建文件。

```sh
fb-gen init                          # 在当前目录初始化（CMake）
fb-gen init --name MyProject         # 指定项目名称
fb-gen init --build zig              # 使用 Zig 构建系统
fb-gen init --build zig --name ESP32 # Zig 项目并指定名称
```

交互式配置包括：项目名称、编程语言、C/C++ 标准、目标架构、编译器、构建后端（Ninja/Make/MSBuild/Custom）、排除目录等。

### `fb-gen sync`

增量同步：对比缓存快照，仅重新处理变更的源文件，更新构建配置。

```sh
fb-gen sync
fb-gen sync --root /path/to/project
fb-gen sync --lsp                  # 同时更新 compile_commands.json
```

### `fb-gen check`

检查项目结构：对比当前目录结构与已有构建文件的差异，以 diff 形式输出。

```sh
fb-gen check
fb-gen check --root /path/to/project
```

### `fb-gen validate`

验证当前项目配置的正确性。

```sh
fb-gen validate
```

### `fb-gen run`

运行完整的扫描 → 分析 → 生成 → 构建流水线。

```sh
fb-gen run
fb-gen run --lang C --no-deps
fb-gen run --watch               # 启用文件监控，自动增量更新
fb-gen run --lsp                 # 构建后生成 compile_commands.json
```

## 全局选项

| 选项 | 说明 | 默认值 |
|------|------|--------|
| `-r, --root <PATH>` | 项目根目录 | `.` |
| `--exclude <DIRS>` | 排除目录（逗号分隔） | — |
| `--lang <C\|CXX>` | 编程语言 | `CXX` |
| `--no-deps` | 跳过依赖扫描 | — |
| `-o, --output <DIR>` | 生成文件输出目录 | `build` |
| `-w, --watch` | 启用文件监控，自动增量更新 | — |
| `-v, --verbose` | 详细输出（`-vv` / `-vvv` 更详细） | — |
| `-q, --quiet` | 静默模式，仅输出错误 | — |
| `--lsp` | 生成 `compile_commands.json` 供 clangd 等 LSP 使用 | — |

## Zig 构建系统

`fb-gen` 支持使用 [Zig](https://ziglang.org/) 作为构建系统和交叉编译器。Zig 自带完整的 C/C++ 交叉编译工具链，无需单独安装目标平台的 GCC/Clang 工具链。

### 为什么选择 Zig？

- **单一工具链** — `zig cc` / `zig c++` 可以交叉编译到 ARM、RISC-V、Xtensa、WASM 等所有目标，无需安装多个 GCC 版本
- **可复现构建** — Zig 编译器是静态链接的单文件二进制，团队成员只需同一 Zig 版本即可获得一致的构建结果
- **内置 C/C++ 支持** — Zig 捆绑了 clang，原生支持编译 C 和 C++ 代码
- **嵌入式友好** — 对裸机/RTOS 项目有优秀的支持

### 使用方式

```sh
# 初始化 Zig 项目
fb-gen init --build zig

# 运行（扫描 → 生成 build.zig → zig build）
fb-gen run
```

`fb-gen init --build zig` 会生成 `build.zig`（而非 `CMakeLists.txt`）。`fb-gen run` 会自动调用 `zig build` 完成构建。

### 支持的交叉编译目标

| 目标架构 | 说明 | Zig 目标三元组 |
|----------|------|---------------|
| `X86_64` | x86-64 Linux (GNU) | `x86_64-linux-gnu` |
| `X86` | x86 Linux (GNU) | `x86-linux-gnu` |
| `ARM64` | AArch64 裸机 | `aarch64-freestanding-none` |
| `ARM32` | ARM 裸机 | `arm-freestanding-none` |
| `NoneEabi` | ARM Cortex-M 裸机 (arm-none-eabi) | `thumb-freestanding-eabi` |
| `RISCV64` | 64 位 RISC-V 裸机 | `riscv64-freestanding-none` |
| `RISCV32` | 32 位 RISC-V 裸机 (ESP32-C3/C6/H2/P4) | `riscv32-freestanding-none` |
| `Xtensa` | Xtensa 裸机 (ESP32/S2/S3) | `xtensa-freestanding-none` |
| `WASM` | WebAssembly 裸机 | `wasm32-freestanding-none` |
| `Custom` | 自定义目标（默认为 x86_64-linux） | 自定义 |

对于交叉编译目标，`fb-gen run` 会自动添加 `--release=small` 优化选项以控制二进制体积（适合 MCU Flash/RAM 限制）。CPU 型号可以通过工具链配置指定（如 `cortex-m3` → Zig 的 `cortex_m3`）。

## LSP 支持

`--lsp` 标志会在命令完成后自动生成 `compile_commands.json`，供 clangd 等语言服务器使用，在编辑器中提供准确的代码补全、跳转和诊断。

```sh
fb-gen run --lsp      # 构建后生成 compile_commands.json
fb-gen sync --lsp     # 增量同步后生成
fb-gen init --lsp     # 初始化后生成
```

**CMake 项目：** 基于项目模块元数据生成标准 clang 编译命令。

**Zig 项目：** 使用 `zig c++` / `zig cc` 作为驱动，自动注入：
- 正确的 `-target` 目标三元组
- `-std=` 语言标准标志
- `-mcpu=` CPU 型号（交叉编译时）
- `--sysroot` 系统根目录（交叉编译时）
- Zig 的 C 系统头文件路径
- 完整的传递依赖 include 路径

## 模块发现规则

- 每个目录含 ≥1 个 `.c`/`.cpp` 文件 → 一个构建模块
- 检测到 `int main(` 或 `void main(` → 可执行文件目标
- 头文件（`.h`/`.hpp`）归属于其目录所在模块
- 汇编文件（`.s`/`.S`）和链接脚本（`.ld`）同样被识别
- 纯头文件目录 → HeaderOnly 目标
- 汇编 + 头文件 → StaticLibrary 目标
- 其他模块默认 → StaticLibrary 目标
- 默认排除目录：`build`, `.git`, `third_party`, `cmake-build-*`, `.idea`, `.vscode`

## 依赖分析策略

对于每个 `#include "xxx/yyy.h"` 指令：
1. 提取第一个路径段（`xxx`），匹配已知模块名
2. 尝试模块全名匹配（如 `src_core`）
3. 回退到目录短名匹配（如 `core` 匹配路径以 `core` 结尾的模块）
4. 匹配成功 → 建立 PUBLIC 依赖边

尖括号引用（`#include <...>`）被忽略。汇编文件（`.S`）同样被扫描（经过 C 预处理器）。

## 架构

```
CLI 交互层 (clap)
  └── 编排与协调层 (workflow / pipeline / reporter / cache / watcher)
        └── 核心业务逻辑层
              ├── ModuleDiscoverer — 模块发现
              ├── DependencyAnalyzer — 依赖分析（petgraph）
              ├── CMakeGenerator — CMakeLists.txt 生成（Tera 模板）
              ├── ZigGenerator — build.zig 生成（Tera 模板）
              └── ConfigInferrer — C++ 标准特性推断
                    └── 外部依赖层 (walkdir / tera / serde)
```

**关键数据流：**

```
FffScanner::scan_source_files()  →  Vec<SourceFile>
  ↓
ModuleDiscoverer::discover()     →  Vec<CMakeModule>
  ↓
DependencyAnalyzer::analyze()    →  DependencyGraph (petgraph DiGraph)
  ↓
CMakeGenerator / ZigGenerator    →  CMakeLists.txt / build.zig
  ↓
MetaCache::save()                →  .fb-gen/cache/{project,modules,checksums}.json
```

## 技术栈

| 组件 | 用途 |
|------|------|
| [clap](https://crates.io/crates/clap) | 命令行解析 |
| [tera](https://crates.io/crates/tera) | 构建文件模板渲染（CMakeLists.txt / build.zig） |
| [petgraph](https://crates.io/crates/petgraph) | 模块依赖图（拓扑排序、循环检测） |
| [walkdir](https://crates.io/crates/walkdir) | 递归目录遍历 |
| [regex](https://crates.io/crates/regex) | `#include` 指令解析、C++ 特性检测 |
| [serde](https://crates.io/crates/serde) / [serde_json](https://crates.io/crates/serde_json) / [serde_yaml](https://crates.io/crates/serde_yaml) | 序列化与配置存储 |
| [anyhow](https://crates.io/crates/anyhow) / [thiserror](https://crates.io/crates/thiserror) | 错误处理 |
| [colored](https://crates.io/crates/colored) | 终端彩色输出 |
| [indicatif](https://crates.io/crates/indicatif) | 进度条 |
| [chrono](https://crates.io/crates/chrono) | 时间戳 |

## 开发

```sh
# 构建
cargo build
cargo build --release

# 运行测试
cargo test                    # 全部测试（单元 + 集成）
cargo test --lib              # 仅单元测试
cargo test --test integration # 仅集成测试
cargo test test_module_discovery  # 运行特定测试

# 安装本地版本
cargo install --path .
```

## License

MIT
