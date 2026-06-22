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
fb-gen sync --watch                # 启用文件监控，检测到变更自动同步
```

**增量同步流程：** 通过对每个文件计算 djb2 校验和，与缓存快照对比，精准识别新增、修改和删除的文件。仅当 `#include` 指令发生变化时才重新运行依赖分析，否则复用缓存的依赖图。已删除文件的孤立校验和会被自动清理。

### `fb-gen check`

检查项目结构：将当前扫描结果重新生成到临时目录，与现有构建文件逐行对比差异。

```sh
fb-gen check
fb-gen check --root /path/to/project
```

- **CMake 项目：** 对比根 `CMakeLists.txt` 和每个模块目录下的 `CMakeLists.txt`
- **Zig 项目：** 对比 `build.zig`

### `fb-gen validate`

验证生成的构建配置是否能被构建工具正确解析。

```sh
fb-gen validate
fb-gen validate --lsp               # 同时生成 compile_commands.json
```

- **CMake 项目：** 运行 `cmake -S . -B <build_dir>` 进行实际配置验证
- **Zig 项目：** 运行 `zig build` 解析 `build.zig`，检查语法和类型正确性

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

# 验证 build.zig 是否正确
fb-gen validate

# 运行（扫描 → 生成 build.zig → zig build）
fb-gen run
```

`fb-gen init --build zig` 会生成 `build.zig`（而非 `CMakeLists.txt`）。`fb-gen run` 会自动调用 `zig build` 完成构建。`fb-gen sync` 在检测到 `build.zig` 内容变化时会写入 `build.zig.new` 而非直接覆盖，保留手动修改的机会。

> **注意：** Zig 构建系统不支持 CMake 的 `add_subdirectory` 等效机制。项目中用户自定义的 CMake 模块（如 CubeMX 生成的 `Drivers/CMakeLists.txt`）无法自动集成到 `build.zig` 中，fb-gen 会打印警告提示手动处理。

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

### CMake 交叉编译工具链

对于 CMake 项目，当目标架构需要交叉编译时，fb-gen 会自动在 `cmake/toolchain.cmake` 中生成完整的工具链文件，包含：
- 编译器前缀（如 `arm-none-eabi-`）和交叉编译器路径
- MCU 特定标志（`-mcpu=`、`-mfloat-abi=`、`-mfpu=` for ARM；`-march=`、`-mabi=` for RISC-V）
- 链接器标志（`--specs=nano.specs` for ARM bare-metal，`-nostartfiles` for RISC-V）
- `CMAKE_SYSROOT` 和 `CMAKE_FIND_ROOT_PATH`（当编译器提供 sysroot 时）
- 内存使用报告（`--print-memory-usage`）

如果用户已存在自定义工具链文件（如 STM32CubeMX 生成），fb-gen 不会覆盖。工具链文件支持 `# USER_START` / `# USER_END` 标记以保留用户自定义内容。

`ARM32` 和 `ARM64` 目标在配置了工具链前缀时也会生成 Linux 用户空间交叉编译工具链（使用 `g++` 作为链接器）。

## 用户自定义保留

fb-gen 生成的构建文件中包含 `# USER_START` / `# USER_END` 标记块。用户在这些标记之间添加的自定义内容会在重新生成时自动保留：

```cmake
# ── User customisations ─────────────────────────────────────────────────────
# USER_START
add_subdirectory(Drivers)          # 用户添加的自定义模块
set(CMAKE_C_FLAGS "${CMAKE_C_FLAGS} -DMY_DEFINE")
# USER_END
```

在 `sync` 模式（`force=false`）下，如果生成内容与现有文件不同，fb-gen 会写入 `.new` 文件（如 `CMakeLists.txt.new`）并提示 diff 命令，而非直接覆盖。这适用于 CMake 和 Zig 两种构建系统。

此外，fb-gen 会自动检测项目中的用户自定义 `CMakeLists.txt`（非 fb-gen 生成），并将其作为子目录添加到根构建文件中，同时过滤与 fb-gen 自身模块源文件重叠的用户模块，避免重复编译。

## LSP 支持

`--lsp` 标志会在命令完成后自动生成 `compile_commands.json`，供 clangd 等语言服务器使用，在编辑器中提供准确的代码补全、跳转和诊断。适用于所有命令。

```sh
fb-gen run --lsp      # 构建后生成 compile_commands.json
fb-gen sync --lsp     # 增量同步后生成
fb-gen init --lsp     # 初始化后生成
fb-gen validate --lsp # 验证后生成
```

**CMake 项目：** 运行 `cmake configure` 并传递 `-DCMAKE_EXPORT_COMPILE_COMMANDS=ON`，然后将生成的 `compile_commands.json` 符号链接到项目根目录。

**Zig 项目：** 基于模块元数据直接生成 `compile_commands.json`，使用 `zig c++` / `zig cc` 作为驱动，自动注入：
- 正确的 `-target` 目标三元组
- `-std=` 语言标准标志
- `-mcpu=` CPU 型号（交叉编译时）
- `--sysroot` 系统根目录（交叉编译时）
- Zig 的 C 系统头文件路径
- 完整的传递依赖 include 路径（使用记忆化 DFS 高效计算）

## 模块发现规则

- 每个目录含 ≥1 个 `.c`/`.cpp` 文件 → 一个构建模块
- 检测到 `int main(` 或 `void main(` → 可执行文件目标
- 头文件（`.h`/`.hpp`）归属于其目录所在模块
- 汇编文件（`.s`/`.S`）和链接脚本（`.ld`）同样被识别
- 纯头文件目录 → `HeaderOnly` 目标
- 汇编 + 头文件 → `StaticLibrary` 目标
- 根目录的源文件/汇编文件会被合并到第一个可执行模块中
- 孤儿链接脚本（无对应源文件的目录中的 `.ld`）会归属于根模块
- 其他模块默认 → `StaticLibrary` 目标
- 默认排除目录：`build`, `.git`, `third_party`, `cmake-build-*`, `.idea`, `.vscode`

增量同步时，文件的 `main()` 函数变化（新增或删除）会触发模块目标类型自动调整（`StaticLibrary` ↔ `Executable`）。

## 依赖分析策略

对于每个 `#include "xxx/yyy.h"` 指令，采用三级匹配策略：

1. **路径段匹配** — 提取第一个路径段（`xxx`），匹配已知模块名
2. **目录短名匹配** — 回退到目录基名匹配（如 `core` 匹配路径以 `core` 结尾的模块）
3. **文件名回退（裸 include）** — 对于无路径分隔符的 `#include "foo.h"`，在所有模块的 headers 列表中查找匹配的头文件名

匹配成功 → 建立 `PUBLIC` 依赖边。尖括号引用（`#include <...>`）被忽略。汇编文件（`.S`）同样被扫描（经过 C 预处理器）。依赖类型（PUBLIC/PRIVATE/INTERFACE）在缓存序列化中完整保留。

## 架构

```
CLI 交互层 (clap)
  └── 编排与协调层 (reporter / cache / watcher / query)
        └── 核心业务逻辑层
              ├── ModuleDiscoverer — 模块发现与分组
              ├── DependencyAnalyzer — 依赖分析（petgraph DiGraph）
              ├── CMakeGenerator — CMakeLists.txt 生成（Tera 模板）
              ├── ZigGenerator — build.zig 生成（Tera 模板）
              ├── ConfigInferrer — C++ 标准特性推断
              └── ToolchainDetect — 交叉编译工具链自动检测
                    └── 扫描层 (FffScanner / walkdir)
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
