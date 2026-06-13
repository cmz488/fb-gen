# fb-gen — Fast Build Generate
  

<div align="center">
<pre style="display:inline-block;text-align:left;line-height:1.2;font-weight:bold;">
<span style="color:#00d4ff;"> ████   █    █   ██████</span>
<span style="color:#00bcd4;">█    █  ██  ██      █</span>
<span style="color:#5c9ce6;">█       █ ██ █     █</span>
<span style="color:#4a7fd4;">█       █    █    █</span>
<span style="color:#5cdb5c;">█    █  █    █   █</span>
<span style="color:#00a651;"> ████   █    █   ██████</span>
</pre>
</div>
<p align="center" style="color:gray;">
  <b>CMakeLists.txt自动生成工具</b>
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

自动扫描 C/C++ 项目目录结构，智能生成多模块 CMake 配置。

## 简介

`fb-gen` (Fast Build Generate) 是一个命令行工具，用于将 C/C++ 项目自动转化为结构清晰的 CMake 构建系统。
你是否厌烦了`CMake`琐碎的脚本语言(他居然是图灵完备的),想拥有cargo和rust的优雅体验，本工具模仿mod.rs的构建方法，把每一个含有*.c/*.h/*.cpp/*.hpp的独立文件夹视为一个项目模块，根据模块间依赖图，生成CMakeLists.txt配置文件。
给你丝滑的cpp/c项目构建体验。

**核心理念：**

- **发现即结构** — 每个包含 `.c`/`.cpp` 源文件的子目录自动成为一个独立的 CMake 子模块（`add_subdirectory`）
- **零配置假定** — 无需额外配置文件，通过扫描目录结构和文件内容智能推断模块边界与依赖关系
- **内容感知** — 解析 `#include` 指令，自动建立模块间依赖图
- **增量友好** — 文件结构变化时可快速重新生成配置，适合活跃开发的代码仓库

## 安装

```sh
# 从源码编译
git clone <repo-url>
cd fb-gen
cargo build --release

# 安装到系统路径
cargo install --path .
```

## 命令用法

### `fb-gen init`

初始化新项目，生成 `fb-gen.toml` 配置文件。通过交互式问答收集项目配置。

```sh
fb-gen init                    # 在当前目录初始化
fb-gen init --name MyProject   # 指定项目名称
```

### `fb-gen sync`

增量同步：扫描源文件变更并更新项目配置，但不生成 CMake 文件。

```sh
fb-gen sync
fb-gen sync --root /path/to/project
```

### `fb-gen check`

检查项目结构：对比当前目录结构与已有 CMakeLists.txt 的差异，输出 diff。

```sh
fb-gen check
fb-gen check --root /path/to/project
```

### `fb-gen validate`

验证当前 `fb-gen.toml` 配置的正确性。

```sh
fb-gen validate
```

### `fb-gen run`

运行完整的扫描 → 分析 → 生成流水线，输出 CMakeLists.txt。

```sh
fb-gen run
fb-gen run --lang C --no-deps
fb-gen run --watch            # 启用文件监控，自动增量更新
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
| `--lsp` |生成complie_command.json文件供lsp分析| - |

## 架构

```
CLI 交互层 (clap)
  └── 编排与协调层 (workflow / pipeline / reporter)
        └── 核心业务逻辑层
              ├── 模块发现 (discoverer)
              ├── 依赖分析 (analyzer)
              ├── CMake 生成 (generator)
              └── 配置推断 (inferrer)
                    └── 外部依赖层 (filesystem / fff_search / tera)
```

## 技术栈

| 组件 | 用途 |
|------|------|
| [clap](https://crates.io/crates/clap) | 命令行解析 |
| [tera](https://crates.io/crates/tera) | CMakeLists.txt 模板渲染 |
| [petgraph](https://crates.io/crates/petgraph) | 模块依赖图（拓扑排序、循环检测） |
| [walkdir](https://crates.io/crates/walkdir) | 递归目录遍历 |
| [regex](https://crates.io/crates/regex) | #include 指令解析、C++ 特性检测 |
| [serde](https://crates.io/crates/serde) / [serde_json](https://crates.io/crates/serde_json) / [serde_yaml](https://crates.io/crates/serde_yaml) | 序列化与配置存储 |
| [anyhow](https://crates.io/crates/anyhow) / [thiserror](https://crates.io/crates/thiserror) | 错误处理 |
| [colored](https://crates.io/crates/colored) | 终端彩色输出 |
| [indicatif](https://crates.io/crates/indicatif) | 进度条 |
| [chrono](https://crates.io/crates/chrono) | 时间戳 |

## License

MIT
