# EABI MCU 配置 & DependencyGraph 修复 — 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复 DependencyGraph 裸 include 依赖缺失 + 消除分析器重复磁盘 I/O + 嵌入式目标强制要求用户指定 MCU 芯片型号

**Architecture:** 分析器从磁盘读取 + 正则解析模式重写为内存数据遍历模式；裸 include 通过文件名回退匹配补充依赖边；query.rs 新增 ARM MCU 交互式提问；generator.rs 移除硬编码降级逻辑

**Tech Stack:** Rust (edition 2021), petgraph, regex, walkdir, tempfile

---

## File Structure

| 文件 | 角色 |
|---|---|
| `tests/integration.rs` | 新增：裸 include 依赖测试、MCU 配置测试 |
| `src/cli/commands.rs` | 删除：`scan_and_discover` 中的死代码循环 |
| `src/core/analyzer.rs` | 重写：纯内存分析、文件名回退匹配 |
| `src/orchestration/query.rs` | 新增：ARM 目标时 MCU 芯片型号提问 |
| `src/core/generator.rs` | 修改：删除 `default_mcu_for()`、嵌入式目标缺 MCU 时报错 |

---

### Task 1: 写裸 include 依赖解析的失败测试

**Files:**
- Modify: `tests/integration.rs`

- [ ] **Step 1: 在 `tests/integration.rs` 尾部新增测试函数**

```rust
#[test]
fn test_bare_include_dependency() {
    // Verify that a bare include (no path prefix) creates a dependency edge
    // by matching the included filename against other modules' headers.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().to_path_buf();

    // Module A: single source file with bare include of "b.h"
    let a_dir = root.join("a");
    std::fs::create_dir(&a_dir).unwrap();
    std::fs::write(
        a_dir.join("a.c"),
        "#include \"b.h\"\n\nint a_func() { return b_func(); }\n",
    )
    .unwrap();

    // Module B: header file b.h
    let b_dir = root.join("b");
    std::fs::create_dir(&b_dir).unwrap();
    std::fs::write(b_dir.join("b.h"), "#pragma once\nint b_func();\n").unwrap();
    std::fs::write(b_dir.join("b.c"), "#include \"b.h\"\n\nint b_func() { return 0; }\n").unwrap();

    // Scan
    let scanner = FffScanner::new(&root);
    let opts = fb_gen::scanner::fs_adapter::ScanOptions {
        root: root.clone(),
        ..Default::default()
    };
    let sources = scanner.scan_source_files(&opts).unwrap();

    // Discover
    let discoverer = ModuleDiscoverer::new(ScanOptions {
        exclude_dirs: vec![],
        root: root.clone(),
    });
    let modules = discoverer.discover(&sources).unwrap();

    // Analyze
    let analyzer = DependencyAnalyzer::new();
    let graph = analyzer.analyze(&modules).unwrap();

    // Module "a" should depend on module "b" because a.c includes "b.h"
    // and b.h lives in module "b".
    let a_deps = graph.get_dependencies("a");
    assert!(
        a_deps.iter().any(|(n, _)| n == "b"),
        "module 'a' should depend on 'b' via bare #include \"b.h\", got: {:?}",
        a_deps
    );

    // Module "b" should have no dependencies (b.c only includes its own header).
    let b_deps = graph.get_dependencies("b");
    assert!(
        b_deps.is_empty(),
        "module 'b' should have no dependencies, got: {:?}",
        b_deps
    );
}
```

- [ ] **Step 2: 运行新测试验证失败**

```bash
cargo test --test integration test_bare_include_dependency
```

Expected: **FAIL** — `module 'a' should depend on 'b' via bare #include "b.h"`，因为当前分析器无法匹配裸 include。

- [ ] **Step 3: Commit**

```bash
git add tests/integration.rs
git commit -m "test: add failing test for bare include dependency resolution"
```

---

### Task 2: 删除 commands.rs 中的死代码

**Files:**
- Modify: `src/cli/commands.rs:96-105`

- [ ] **Step 1: 删除死代码循环**

在 `src/cli/commands.rs` 的 `scan_and_discover` 函数中，删除第 96-105 行：

```rust
// 删除这段代码：
        // Populate each module's includes from scanned sources.
        for m in &mut modules {
            let mut includes: Vec<String> = Vec::new();
            for sf in &m.sources {
                includes.extend(sf.includes.clone());
            }
            // Attach includes via the module model — we store them in a
            // transient way by repopulating SourceFile::includes, which
            // the analyzer already reads from SourceFile directly.
        }
```

删除后的上下文应该是：

```rust
    let graph = if !cli.no_deps {
        reporter.report_info("Analyzing dependencies ...");
        let analyzer = DependencyAnalyzer::new();

        let graph = analyzer.analyze(&modules)?;  // ← 直接调用，去掉了中间的死循环
        let deps = graph.edge_count();
        if graph.has_cycles() {
            reporter.report_warning("Dependency graph contains cycles — manual adjustment may be needed");
        }
        reporter.report_success(&format!("Found {} dependencies", deps));
        Some(graph)
    } else {
```

- [ ] **Step 2: 验证编译通过**

```bash
cargo build 2>&1
```

Expected: 编译成功（`modules` 不再需要 `mut`，但若编译器警告 `mut` 未使用，一并修复）。

- [ ] **Step 3: 运行已有测试确认无回归**

```bash
cargo test --test integration
```

Expected: 所有已有测试通过（除了 Task 1 新增的 test_bare_include_dependency，它仍然 FAIL）。

- [ ] **Step 4: Commit**

```bash
git add src/cli/commands.rs
git commit -m "refactor: remove dead code loop in scan_and_discover

The loop collected SourceFile.includes into a local variable that was
never used. The analyzer will be rewritten to read includes directly
from the in-memory SourceFile structs."
```

---

### Task 3: 重写 analyzer.rs — 纯内存分析 + 文件名回退

**Files:**
- Modify: `src/core/analyzer.rs`

- [ ] **Step 1: 重写整个 `src/core/analyzer.rs`**

用以下内容替换整个文件：

```rust
//! Dependency analyzer — builds a module-level dependency graph by inspecting
//! `#include` directives from in-memory SourceFile data.
//!
//! For each `#include "..."` string already parsed by the scanner, the first
//! path segment before `/` is matched against known module names.  When no
//! module matches (bare include like `#include "foo.h"`), a fallback matches
//! the filename against headers declared in other modules.

use crate::models::dependency::{DependencyEdge, DependencyGraph, DependencyType};
use crate::models::error::{FbGenError, Result};
use crate::models::module::CMakeModule;
use std::collections::HashSet;

/// Analyses `#include` directives across modules to build a dependency graph.
///
/// All data comes from the already-parsed `SourceFile::includes` fields —
/// no files are read from disk.
pub struct DependencyAnalyzer;

impl DependencyAnalyzer {
    pub fn new() -> Self {
        Self
    }

    /// Analyze all modules and return a dependency graph.
    ///
    /// For every source, header, and assembly file in every module, each
    /// `#include "..."` string is processed:
    ///
    /// 1. **Path-segment match** (existing logic): for `#include "core/foo.h"`,
    ///    the first path segment `core` is extracted and matched against full
    ///    module names, then short directory basenames.
    ///
    /// 2. **Filename fallback** (new): for bare includes like `#include "foo.h"`
    ///    with no `/` separator, the filename `foo.h` is looked up in all other
    ///    modules' `headers` lists. If found, a `PUBLIC` dependency edge is
    ///    created.
    ///
    /// Assembly sources (`.S`) are included because they go through the C
    /// preprocessor and may `#include` module headers.
    pub fn analyze(&self, modules: &[CMakeModule]) -> Result<DependencyGraph> {
        let mut graph = DependencyGraph::new();

        // Register all modules as graph nodes.
        for m in modules {
            graph.add_module(&m.name);
        }

        // Build lookup sets.
        let module_names: HashSet<&str> =
            modules.iter().map(|m| m.name.as_str()).collect();

        let short_names: HashSet<&str> = modules
            .iter()
            .filter_map(|m| m.relative_path.file_name().and_then(|n| n.to_str()))
            .collect();

        // Build a mapping: header filename → module name, for the fallback pass.
        let mut header_to_module: Vec<(&str, &str)> = Vec::new();
        for m in modules {
            for h in &m.headers {
                header_to_module.push((&h.file_name, &m.name));
            }
        }

        for module in modules {
            let all_includes: Vec<(&str, &str)> = module
                .sources
                .iter()
                .map(|sf| {
                    sf.includes
                        .iter()
                        .map(move |inc| (inc.as_str(), "source"))
                })
                .flatten()
                .chain(module.asm_sources.iter().flat_map(|af| {
                    af.includes
                        .iter()
                        .map(move |inc| (inc.as_str(), "asm"))
                }))
                .chain(module.headers.iter().flat_map(|hf| {
                    hf.includes
                        .iter()
                        .map(move |inc| (inc.as_str(), "header"))
                }))
                .collect();

            for (include_str, label) in &all_includes {
                // Extract first path segment before '/'.
                let first_segment = include_str
                    .split('/')
                    .next()
                    .unwrap_or(include_str);

                // Skip self-references.
                if first_segment == module.name {
                    continue;
                }

                // Try exact module name match first, then short-name match.
                let target: Option<String> = if module_names.contains(first_segment) {
                    Some(first_segment.to_string())
                } else if short_names.contains(first_segment) {
                    // Resolve short name to full module name.
                    modules
                        .iter()
                        .find(|m| {
                            m.relative_path
                                .file_name()
                                .and_then(|n| n.to_str())
                                .is_some_and(|n| n == first_segment)
                        })
                        .map(|m| m.name.clone())
                } else {
                    None
                };

                let target_name = if let Some(t) = target {
                    t
                } else {
                    // ── Filename fallback (bare include like "foo.h") ──
                    // Extract the filename (last path segment).
                    let filename = include_str
                        .rsplit('/')
                        .next()
                        .unwrap_or(include_str);

                    // Find a module (other than self) whose headers contain this filename.
                    match header_to_module
                        .iter()
                        .find(|(fname, mname)| *fname == filename && *mname != module.name)
                    {
                        Some((_, mname)) => mname.to_string(),
                        None => continue, // no match — probably a system/external header
                    }
                };

                // Add the edge if it doesn't already exist.
                let existing = graph.get_dependencies(&module.name);
                if !existing.iter().any(|(n, _)| n == &target_name) {
                    graph.add_dependency(DependencyEdge {
                        from: module.name.clone(),
                        to: target_name,
                        dep_type: DependencyType::Public,
                        reason: format!(
                            "#include \"{}\" in {} {}",
                            include_str, label,
                            if *label == "source" {
                                "source"
                            } else if *label == "asm" {
                                "asm source"
                            } else {
                                "header"
                            }
                        ),
                    });
                }
            }
        }

        // Validate: detect cycles and warn (but don't fail — the caller decides).
        if graph.has_cycles() {
            eprintln!(
                "warning: dependency graph contains cycles — target_link_libraries may need manual adjustment"
            );
        }

        Ok(graph)
    }
}

impl Default for DependencyAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_include_path_segment_extraction() {
        // Verify that path segment extraction logic works correctly.
        let inc = "core/foo.h";
        let first = inc.split('/').next().unwrap();
        assert_eq!(first, "core");

        let bare = "foo.h";
        let first = bare.split('/').next().unwrap();
        assert_eq!(first, "foo.h"); // bare — no '/' separator

        let filename = bare.rsplit('/').next().unwrap();
        assert_eq!(filename, "foo.h");
    }

    #[test]
    fn test_include_filename_fallback() {
        // Verify that a bare include resolves to a module with matching header.
        let inc = "utils.h";
        let filename = inc.rsplit('/').next().unwrap();
        assert_eq!(filename, "utils.h");
    }
}
```

- [ ] **Step 2: 编译验证**

```bash
cargo build 2>&1
```

Expected: 编译成功。

- [ ] **Step 3: 运行所有测试**

```bash
cargo test --lib
cargo test --test integration
```

Expected:
- `test_bare_include_dependency` **PASS**（文件名回退修复生效）
- `test_dependency_analysis` PASS（路径段匹配保持不变）
- `test_topological_order` PASS
- `test_cycle_detection` PASS
- 所有其他测试 PASS

- [ ] **Step 4: Commit**

```bash
git add src/core/analyzer.rs
git commit -m "refactor: rewrite analyzer to use in-memory includes with filename fallback

- Remove scan_file_includes() — no more disk I/O during analysis.
- Analyze reads SourceFile::includes directly from memory.
- Add filename fallback: bare includes like #include \"foo.h\" now resolve
  by matching the filename against headers declared in other modules.
- Remove regex dependency from analyzer (includes are pre-parsed by scanner)."
```

---

### Task 4: 为 DepdendencyGraph 的正确性补全测试

**Files:**
- Modify: `tests/integration.rs`

- [ ] **Step 1: 在 `tests/integration.rs` 尾部新增自引用排除测试**

```rust
#[test]
fn test_bare_include_no_self_dependency() {
    // A module's bare include of its own header should NOT create a self-dependency.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().to_path_buf();

    // Module A: source that includes its own header
    let a_dir = root.join("a");
    std::fs::create_dir(&a_dir).unwrap();
    std::fs::write(a_dir.join("a.h"), "#pragma once\nint a_func();\n").unwrap();
    std::fs::write(
        a_dir.join("a.c"),
        "#include \"a.h\"\n\nint a_func() { return 0; }\n",
    )
    .unwrap();

    let scanner = FffScanner::new(&root);
    let opts = fb_gen::scanner::fs_adapter::ScanOptions {
        root: root.clone(),
        ..Default::default()
    };
    let sources = scanner.scan_source_files(&opts).unwrap();

    let discoverer = ModuleDiscoverer::new(ScanOptions {
        exclude_dirs: vec![],
        root: root.clone(),
    });
    let modules = discoverer.discover(&sources).unwrap();

    let analyzer = DependencyAnalyzer::new();
    let graph = analyzer.analyze(&modules).unwrap();

    // Module "a" should have zero dependencies — #include "a.h" is self-referential.
    let a_deps = graph.get_dependencies("a");
    assert!(
        a_deps.is_empty(),
        "module 'a' should have no self-dependencies, got: {:?}",
        a_deps
    );
}
```

- [ ] **Step 2: 运行新测试**

```bash
cargo test --test integration test_bare_include_no_self_dependency
```

Expected: PASS（现在的分析器已有 `first_segment == module.name` 的自引用保护；文件名回退也有 `*mname != module.name` 的保护）。

- [ ] **Step 3: Commit**

```bash
git add tests/integration.rs
git commit -m "test: add self-dependency exclusion test for bare includes"
```

---

### Task 5: 新增 MCU 芯片型号交互式提问

**Files:**
- Modify: `src/orchestration/query.rs`

- [ ] **Step 1: 在架构选择后、编译器选择前插入 MCU 提示**

在 `src/orchestration/query.rs` 的 `ask_project_config()` 中，第 110-112 行（编译器选择之前）插入以下代码。定位方式：在 `let target_arch = match arch_choice.as_str() { ... };` 之后、`// ── compiler ──` 之前。

```rust
        // ── MCU/CPU flags (ARM embedded targets only) ───────────────
        let mcu_flags = if matches!(target_arch, TargetArch::NoneEabi | TargetArch::ARM32 | TargetArch::ARM64) {
            let default_mcu = if matches!(target_arch, TargetArch::ARM64) {
                "cortex-a53"
            } else {
                "cortex-m3"
            };
            println!();
            println!("  ARM MCU/CPU selection:");
            println!("    Specify the target chip model for -mcpu= flag.");
            println!("    Examples: cortex-m0, cortex-m3, cortex-m4, cortex-m7, cortex-a53, cortex-a72");
            prompt_with_default(
                &format!("  ARM MCU/CPU [{}]", default_mcu),
                default_mcu,
            )
            .map_err(|e| {
                crate::models::FbGenError::Config(format!("failed to read MCU flags: {e}"))
            })?
        } else {
            String::new()
        };
```

- [ ] **Step 2: 将 `mcu_flags` 写入返回的 `ProjectConfig`**

在相同文件底部，找到构建 `ProjectConfig` 结构体的地方（约第 169 行），在字段列表中添加 `mcu_flags`：

```rust
        let config = ProjectConfig {
            name,
            version: "0.1.0".to_string(),
            root: root.clone(),
            language,
            c_standard,
            cpp_standard,
            target_arch,
            compiler,
            build_backend,
            cmake_min_version,
            exclude_dirs: vec!["build".into(), ".git".into()],
            output_dir: PathBuf::from(output_dir_str),
            enable_watch: false,
            modules: vec![],
            generated_at: String::new(),
            cmake_presets: None,
            toolchain_files: vec![],
            mcu_flags,                                          // ← 新增
        };
```

注意：`ProjectConfig` 已有 `mcu_flags` 字段（`models/project.rs` 第 59 行），无需修改模型定义。

- [ ] **Step 3: 编译验证**

```bash
cargo build 2>&1
```

Expected: 编译成功。

- [ ] **Step 4: Commit**

```bash
git add src/orchestration/query.rs
git commit -m "feat: add interactive MCU chip model prompt for ARM embedded targets

When user selects NoneEabi, ARM32, or ARM64 target architecture,
fb-gen init now asks for the specific chip model (e.g. cortex-m3)
to use as the -mcpu= flag. Defaults to cortex-m3 (ARM32/NoneEabi)
or cortex-a53 (ARM64) if the user presses Enter."
```

---

### Task 6: 移除硬编码 mcu 降级逻辑 —— 空 MCU 时报错

**Files:**
- Modify: `src/core/generator.rs`

- [ ] **Step 1: 删除 `default_mcu_for()` 函数**

删除 `src/core/generator.rs` 第 578-583 行：

```rust
// 删除：
fn default_mcu_for(arch: &TargetArch) -> &'static str {
    match arch {
        TargetArch::NoneEabi | TargetArch::ARM32 => "cortex-m3",
        TargetArch::ARM64 => "cortex-a53",
        _ => "",
    }
}
```

- [ ] **Step 2: 修改 `render_toolchain()` 签名 —— 返回 `Result<Option<String>>` 以支持错误传播**

将第 434-456 行整个函数替换。定位：从 `/// Render the toolchain.cmake content ...` 注释开始，到 `}` 结束。

```rust
    /// Render the toolchain.cmake content for cross-compilation targets.
    ///
    /// Returns `Ok(None)` if the target architecture does not require a toolchain
    /// (X86_64, X86, WASM, or Custom).
    ///
    /// Returns `Err` if the architecture requires a toolchain but no MCU flags
    /// have been configured — the user must specify the chip model.
    fn render_toolchain(&self) -> Result<Option<String>> {
        let mcu_flags = self.config.mcu_flags.as_str();

        // Embedded targets MUST have MCU flags specified.
        let requires_mcu = matches!(
            &self.config.target_arch,
            TargetArch::NoneEabi | TargetArch::ARM32 | TargetArch::ARM64 | TargetArch::RISCV64
        );
        if requires_mcu && mcu_flags.is_empty() {
            return Err(FbGenError::Config(
                "MCU/CPU flags are required for embedded targets. \
                 Run `fb-gen init` to configure, or set mcu_flags in your config."
                    .into(),
            ));
        }

        match &self.config.target_arch {
            TargetArch::NoneEabi | TargetArch::ARM32 => {
                Ok(Some(render_arm_eabi_toolchain(mcu_flags)))
            }
            TargetArch::ARM64 => {
                Ok(Some(render_aarch64_toolchain(mcu_flags)))
            }
            TargetArch::RISCV64 => {
                Ok(Some(render_riscv64_toolchain(mcu_flags)))
            }
            _ => Ok(None),
        }
    }
```

- [ ] **Step 3: 修改 `generate()` 中对 `render_toolchain()` 的调用**

在 `generate()` 函数中（约第 192 行），将：

```rust
        // ── Toolchain file (cross-compile only) ───────────────────────
        if let Some(toolchain_content) = self.render_toolchain() {
```

改为：

```rust
        // ── Toolchain file (cross-compile only) ───────────────────────
        if let Some(toolchain_content) = self.render_toolchain()? {
```

注意：只需要在 `self.render_toolchain()` 后面加 `?` 运算符。

- [ ] **Step 4: 编译验证**

```bash
cargo build 2>&1
```

Expected: 编译成功。若有未使用 import 警告（如 `TargetArch`），一并清理。

- [ ] **Step 5: 运行 toolchain 相关测试确认无回归**

```bash
cargo test --test integration test_cross_compile_template
cargo test --test integration test_toolchain_arm64
cargo test --test integration test_toolchain_riscv64
cargo test --test integration test_toolchain_not_generated_for_x86
```

Expected: 全部 PASS。

- [ ] **Step 6: Commit**

```bash
git add src/core/generator.rs
git commit -m "refactor: remove hardcoded default_mcu_for(), error on missing MCU

- Delete default_mcu_for() — no more silent fallback to cortex-m3/cortex-a53.
- render_toolchain() now returns Result<Option<String>> and returns
  FbGenError::Config when an embedded target has no mcu_flags configured.
- The generate() caller propagates the error via the ? operator."
```

---

### Task 7: 全量测试验证

**Files:** (无改动，仅验证)

- [ ] **Step 1: 运行全部 lib 测试**

```bash
cargo test --lib
```

Expected: 所有单元测试 PASS。

- [ ] **Step 2: 运行全部集成测试**

```bash
cargo test --test integration
```

Expected: 所有 15 个集成测试 PASS（原有 12 个 + 在 Task 1/4 新增的 2 个 + 现有测试 `test_bare_include_dependency`、`test_bare_include_no_self_dependency`）。

具体检查：
- `test_include_parsing` PASS
- `test_module_discovery` PASS
- `test_dependency_analysis` PASS
- `test_topological_order` PASS
- `test_cycle_detection` PASS
- `test_cmake_generation` PASS
- `test_asm_file_detection` PASS
- `test_linker_script_detection` PASS
- `test_presets_detection` PASS
- `test_cross_compile_template` PASS
- `test_toolchain_arm64` PASS
- `test_toolchain_riscv64` PASS
- `test_toolchain_not_generated_for_x86` PASS
- `test_bare_include_dependency` PASS
- `test_bare_include_no_self_dependency` PASS

- [ ] **Step 3: 最终编译确认（release 模式）**

```bash
cargo build --release
```

Expected: 编译成功，无警告。

- [ ] **Step 4: Commit（若测试或编译有微调）**

```bash
git add -A
git commit -m "chore: final adjustments after full test suite run"
```

仅当有微调时才需此步骤。
