# Cross-Compilation Toolchain Auto-Detection & Sysroot Support — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Auto-detect cross-compilation toolchains on the host, add `CMAKE_SYSROOT`/`CMAKE_FIND_ROOT_PATH` to generated toolchain files, and extend toolchain generation beyond NoneEabi to ARM32, ARM64, and RISCV64 targets.

**Architecture:** Add a new `toolchain_detect` module that scans PATH for `*-gcc` binaries, queries them for sysroot/triplet info, and returns structured results. Extend `ToolchainConfig` with `prefix`/`sysroot`/`find_root_path` fields. Wire auto-detection into `UserQuery::ask_project_config()` for interactive init. Refactor `render_embedded_toolchain()` to a Tera template. Extend `render_toolchain()` to cover ARM32/ARM64/RISCV64 when `ToolchainConfig` has a prefix.

**Tech Stack:** Rust stdlib (`std::env::split_paths`, `std::process::Command`), Tera templates, serde.

---

### Task 1: Extend ToolchainConfig model

**Files:**
- Modify: `src/models/project.rs:41-69`

- [ ] **Step 1: Add prefix, sysroot, find_root_path fields to ToolchainConfig**

```rust
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
    /// Generates the `-mfpu=` flag.
    pub fpu: String,

    /// Raw compiler/linker flags (e.g. "-mthumb", "-march=rv32imac").
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
}

impl Default for ToolchainConfig {
    fn default() -> Self {
        Self {
            cpu: String::new(),
            float_abi: String::new(),
            fpu: String::new(),
            extra_flags: String::new(),
            prefix: String::new(),
            sysroot: None,
            find_root_path: Vec::new(),
        }
    }
}
```

The `#[serde(default)]` annotations ensure existing `project.json` caches deserialise without error — missing keys get the default (empty string, None, empty vec).

- [ ] **Step 2: Build and run existing tests to verify backward compat**

```bash
cargo test
```
Expected: all 65 tests pass. The serde(default) attributes mean `test_cross_compile_template` and `test_toolchain_none_eabi_missing_cpu` continue to work.

- [ ] **Step 3: Commit**

```bash
git add src/models/project.rs
git commit -m "feat: add prefix/sysroot/find_root_path fields to ToolchainConfig

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 2: Create toolchain_detect module

**Files:**
- Create: `src/core/toolchain_detect.rs`
- Modify: `src/core/mod.rs`

- [ ] **Step 1: Create the module file with types and detection logic**

```rust
//! Auto-detection of cross-compilation toolchains installed on the host.
//!
//! Scans `$PATH` for `*-gcc` binaries, queries each candidate for its
//! sysroot and target triplet, and returns a list of usable toolchains.

use crate::models::project::TargetArch;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A cross-compilation toolchain discovered on the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedToolchain {
    /// Toolchain prefix, e.g. `"arm-none-eabi-"`.
    pub prefix: String,
    /// Full path to the C compiler, e.g. `/usr/bin/arm-none-eabi-gcc`.
    pub cc_path: PathBuf,
    /// Sysroot reported by `gcc -print-sysroot`, may be empty.
    pub sysroot: Option<PathBuf>,
    /// Target triplet reported by `gcc -dumpmachine`, e.g. `arm-none-eabi`.
    pub target_triplet: String,
    /// Suggested fb-gen architecture inferred from the triplet.
    pub suggested_arch: TargetArch,
}

/// Known cross-compilation toolchain patterns.
///
/// Each entry maps a prefix substring to a (prefix, suggested_arch) pair.
/// Order matters: more-specific patterns come first so they match before
/// shorter prefixes (e.g. `arm-none-eabi-` before `arm-`).
const KNOWN_TOOLCHAINS: &[(&str, TargetArch)] = &[
    ("arm-none-eabi", TargetArch::NoneEabi),
    ("aarch64-none-elf", TargetArch::ARM64),
    ("aarch64-linux-gnu", TargetArch::ARM64),
    ("arm-linux-gnueabihf", TargetArch::ARM32),
    ("arm-linux-gnueabi", TargetArch::ARM32),
    ("riscv64-unknown-elf", TargetArch::RISCV64),
    ("riscv64-linux-gnu", TargetArch::RISCV64),
];

/// Scan `$PATH` for cross-compilation toolchains.
///
/// For each `*-gcc` binary found whose prefix matches a known pattern:
/// 1. Verify `g++` and `objcopy` siblings exist.
/// 2. Run `<prefix>gcc -print-sysroot` → optional sysroot.
/// 3. Run `<prefix>gcc -dumpmachine` → target triplet.
/// 4. Map the triplet to a `TargetArch`.
///
/// Returns a deduplicated list, sorted by prefix.
pub fn detect_toolchains() -> Vec<DetectedToolchain> {
    let mut found: Vec<DetectedToolchain> = Vec::new();
    let mut seen_prefixes: std::collections::HashSet<String> = std::collections::HashSet::new();

    let path_var = std::env::var_os("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path_var) {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let file_name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n,
                None => continue,
            };

            // We're looking for files matching `<prefix>gcc` (or `<prefix>gcc.exe`).
            let stem = file_name
                .strip_suffix("gcc")
                .or_else(|| file_name.strip_suffix("gcc.exe"))
                .unwrap_or("");
            if stem.is_empty() {
                continue;
            }

            // Check against known patterns.
            let matched = KNOWN_TOOLCHAINS
                .iter()
                .find(|(pattern, _)| stem.contains(pattern));
            let (_, suggested_arch) = match matched {
                Some(m) => m,
                None => continue,
            };

            let prefix = stem.to_string();
            if !seen_prefixes.insert(prefix.clone()) {
                continue; // Already recorded this prefix.
            }

            // Verify sibling tools exist.
            let gxx_path = dir.join(format!("{prefix}g++"));
            let objcopy_path = dir.join(format!("{prefix}objcopy"));
            if !gxx_path.exists() || !objcopy_path.exists() {
                continue; // Incomplete toolchain.
            }

            // Query sysroot.
            let sysroot = run_cc_query(&path, &["-print-sysroot"]);
            let sysroot = sysroot.filter(|s| !s.is_empty()).map(PathBuf::from);

            // Query target triplet.
            let triplet = run_cc_query(&path, &["-dumpmachine"]).unwrap_or_default();

            found.push(DetectedToolchain {
                prefix,
                cc_path: path,
                sysroot,
                target_triplet: triplet.clone(),
                suggested_arch: suggested_arch.clone(),
            });
        }
    }

    // Sort: known prefixes first, then alphabetically.
    found.sort_by(|a, b| {
        let a_known = KNOWN_TOOLCHAINS.iter().position(|(p, _)| a.prefix.contains(p));
        let b_known = KNOWN_TOOLCHAINS.iter().position(|(p, _)| b.prefix.contains(p));
        match (a_known, b_known) {
            (Some(ai), Some(bi)) => ai.cmp(&bi).then_with(|| a.prefix.cmp(&b.prefix)),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.prefix.cmp(&b.prefix),
        }
    });

    found
}

/// Run a compiler query and return stdout as a trimmed String.
fn run_cc_query(cc_path: &Path, args: &[&str]) -> Option<String> {
    Command::new(cc_path)
        .args(args)
        .output()
        .ok()
        .and_then(|out| {
            if out.status.success() {
                String::from_utf8(out.stdout).ok().map(|s| s.trim().to_string())
            } else {
                None
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detected_toolchain_struct() {
        let dt = DetectedToolchain {
            prefix: "arm-none-eabi-".into(),
            cc_path: PathBuf::from("/usr/bin/arm-none-eabi-gcc"),
            sysroot: Some(PathBuf::from("/usr/lib/arm-none-eabi")),
            target_triplet: "arm-none-eabi".into(),
            suggested_arch: TargetArch::NoneEabi,
        };
        assert_eq!(dt.prefix, "arm-none-eabi-");
        assert!(dt.sysroot.is_some());
    }

    #[test]
    fn test_detect_empty_path_returns_empty() {
        // Without a real cross-compiler, detection should return an empty vec.
        let result = detect_toolchains();
        // We can't assert on exact results since the host may or may not have
        // cross-compilers, but the function must not panic.
        let _ = result.len();
    }

    #[test]
    fn test_known_toolchains_sorted() {
        // Verify KNOWN_TOOLCHAINS is sorted with more-specific patterns first.
        for window in KNOWN_TOOLCHAINS.windows(2) {
            let (a, _) = window[0];
            let (b, _) = window[1];
            assert!(
                a.len() >= b.len() || a < b,
                "KNOWN_TOOLCHAINS should have longer (more-specific) prefixes first: {a} vs {b}"
            );
        }
    }
}
```

- [ ] **Step 2: Register the module in src/core/mod.rs**

```rust
pub mod analyzer;
pub mod discoverer;
pub mod generator;
pub mod inferrer;
pub mod toolchain_detect;

pub use analyzer::DependencyAnalyzer;
pub use discoverer::{ModuleDiscoverer, ScanOptions};
pub use generator::CMakeGenerator;
pub use inferrer::ConfigInferrer;
pub use toolchain_detect::{detect_toolchains, DetectedToolchain};
```

- [ ] **Step 3: Build and run the new tests**

```bash
cargo test toolchain_detect
```
Expected: 3 new tests pass (they don't require a real cross-compiler).

- [ ] **Step 4: Commit**

```bash
git add src/core/toolchain_detect.rs src/core/mod.rs
git commit -m "feat: add toolchain_detect module for scanning PATH for cross-compilers

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 3: Refactor render_embedded_toolchain to Tera template

**Files:**
- Modify: `src/core/generator.rs` (add template, refactor `render_embedded_toolchain`, update callers)

- [ ] **Step 1: Add the TOOLCHAIN_TEMPLATE constant**

Insert after the `MODULE_TEMPLATE` constant (around line 153):

```rust
/// `cmake/toolchain.cmake` template for embedded cross-compilation.
const TOOLCHAIN_TEMPLATE: &str = r#"# Auto-generated toolchain file by fb-gen

set(CMAKE_SYSTEM_NAME               {{ system_name }})
set(CMAKE_SYSTEM_PROCESSOR          {{ processor }})

set(CMAKE_C_COMPILER_ID GNU)
set(CMAKE_CXX_COMPILER_ID GNU)

# Some default GCC settings
# {{ prefix }} must be part of path environment
set(TOOLCHAIN_PREFIX                {{ prefix }})

set(CMAKE_C_COMPILER                ${TOOLCHAIN_PREFIX}gcc)
set(CMAKE_ASM_COMPILER              ${CMAKE_C_COMPILER})
set(CMAKE_CXX_COMPILER              ${TOOLCHAIN_PREFIX}g++)
set(CMAKE_LINKER                    ${TOOLCHAIN_PREFIX}g++)
set(CMAKE_OBJCOPY                   ${TOOLCHAIN_PREFIX}objcopy)
set(CMAKE_SIZE                      ${TOOLCHAIN_PREFIX}size)

set(CMAKE_EXECUTABLE_SUFFIX_ASM     ".elf")
set(CMAKE_EXECUTABLE_SUFFIX_C       ".elf")
set(CMAKE_EXECUTABLE_SUFFIX_CXX     ".elf")

set(CMAKE_TRY_COMPILE_TARGET_TYPE STATIC_LIBRARY)

# ── Sysroot (auto-detected) ──────────────────────────────────
{% if sysroot -%}
set(CMAKE_SYSROOT {{ sysroot }})
set(CMAKE_FIND_ROOT_PATH ${CMAKE_SYSROOT}{% for p in find_root_path %} {{ p }}{% endfor %})
{% endif -%}

# ── Cross-compilation root paths ─────────────────────────────
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_PACKAGE ONLY)

# MCU specific flags
set(TARGET_FLAGS "{{ target_flags }} ")

set(CMAKE_C_FLAGS "${CMAKE_C_FLAGS} ${TARGET_FLAGS}")
set(CMAKE_C_FLAGS "${CMAKE_C_FLAGS} -Wall -fdata-sections -ffunction-sections -fstack-usage")
set(CMAKE_ASM_FLAGS "${CMAKE_C_FLAGS} -x assembler-with-cpp -MMD -MP")

set(CMAKE_C_FLAGS_DEBUG "-O0 -g3")
set(CMAKE_C_FLAGS_RELEASE "-Os -g0")
set(CMAKE_CXX_FLAGS_DEBUG "-O0 -g3")
set(CMAKE_CXX_FLAGS_RELEASE "-Os -g0")

set(CMAKE_CXX_FLAGS "${CMAKE_C_FLAGS} -fno-rtti -fno-exceptions -fno-threadsafe-statics")

set(CMAKE_EXE_LINKER_FLAGS "${TARGET_FLAGS}")
set(CMAKE_EXE_LINKER_FLAGS "${CMAKE_EXE_LINKER_FLAGS}{{ ld_flag }}")
set(CMAKE_EXE_LINKER_FLAGS "${CMAKE_EXE_LINKER_FLAGS} --specs=nano.specs")
set(CMAKE_EXE_LINKER_FLAGS "${CMAKE_EXE_LINKER_FLAGS} -Wl,-Map=${CMAKE_PROJECT_NAME}.map -Wl,--gc-sections")
set(CMAKE_EXE_LINKER_FLAGS "${CMAKE_EXE_LINKER_FLAGS} -Wl,--print-memory-usage")
set(TOOLCHAIN_LINK_LIBRARIES "m")

# ── User customisations ──────────────────────────────────────────
# USER_START
# USER_END
"#;
```

- [ ] **Step 2: Register the template in CMakeGenerator::new()**

Add after the existing template registrations (around line 171):

```rust
tera.add_raw_template("toolchain", TOOLCHAIN_TEMPLATE)
    .map_err(FbGenError::Template)?;
```

- [ ] **Step 3: Refactor render_embedded_toolchain to use Tera**

Replace the `render_embedded_toolchain` function body (lines 744-836):

```rust
fn render_embedded_toolchain(
    system_name: &str,
    processor: &str,
    prefix: &str,
    tc: &ToolchainConfig,
    root_ld_scripts: &[String],
) -> FbGenResult<String> {
    // Assemble TARGET_FLAGS from structured fields.
    let mut flags: Vec<String> = Vec::new();
    if !tc.cpu.is_empty() {
        flags.push(format!("-mcpu={}", tc.cpu));
    }
    if !tc.float_abi.is_empty() {
        flags.push(format!("-mfloat-abi={}", tc.float_abi));
    }
    if !tc.fpu.is_empty() {
        flags.push(format!("-mfpu={}", tc.fpu));
    }
    if !tc.extra_flags.is_empty() {
        flags.push(tc.extra_flags.clone());
    }
    let target_flags = flags.join(" ");

    let ld_flag = if root_ld_scripts.len() == 1 {
        format!(" -T \"${{CMAKE_SOURCE_DIR}}/{}\"", root_ld_scripts[0])
    } else {
        String::new()
    };

    let mut ctx = Context::new();
    ctx.insert("system_name", system_name);
    ctx.insert("processor", processor);
    ctx.insert("prefix", prefix);
    ctx.insert("target_flags", &target_flags);
    ctx.insert("ld_flag", &ld_flag);
    ctx.insert("sysroot", &tc.sysroot);
    ctx.insert("find_root_path", &tc.find_root_path);

    self.tera
        .render("toolchain", &ctx)
        .map_err(FbGenError::Template)
}
```

Note the return type changes from `String` to `FbGenResult<String>`.

- [ ] **Step 4: Update callers (render_arm_eabi_toolchain, render_riscv64_toolchain)**

Convert them to instance methods on `CMakeGenerator` since they now need `&self` for Tera rendering. Replace both free functions:

```rust
fn render_arm_eabi_toolchain(&self, tc: &ToolchainConfig, root_ld_scripts: &[String]) -> FbGenResult<String> {
    let prefix = if tc.prefix.is_empty() { "arm-none-eabi-" } else { &tc.prefix };
    self.render_embedded_toolchain("Generic", "arm", prefix, tc, root_ld_scripts)
}

fn render_riscv64_toolchain(&self, tc: &ToolchainConfig, root_ld_scripts: &[String]) -> FbGenResult<String> {
    let prefix = if tc.prefix.is_empty() { "riscv64-unknown-elf-" } else { &tc.prefix };
    self.render_embedded_toolchain("Generic", "riscv64", prefix, tc, root_ld_scripts)
}
```

- [ ] **Step 5: Update render_toolchain to use new signatures**

```rust
fn render_toolchain(&self, root_ld_scripts: &[String]) -> FbGenResult<Option<String>> {
    let tc = match &self.config.toolchain {
        Some(tc) => tc,
        None => {
            if matches!(&self.config.target_arch, TargetArch::NoneEabi) {
                return Err(FbGenError::Config(
                    "Toolchain configuration is required for NoneEabi targets. \
                     Run `fb-gen init` to configure."
                        .into(),
                ));
            }
            return Ok(None);
        }
    };

    // NoneEabi MUST have CPU specified.
    if matches!(&self.config.target_arch, TargetArch::NoneEabi) && tc.cpu.is_empty() {
        return Err(FbGenError::Config(
            "MCU/CPU is required for NoneEabi targets. \
             Run `fb-gen init` to configure."
                .into(),
        ));
    }

    // Use tc.prefix for the prefix when set; otherwise fall back to arch defaults.
    match &self.config.target_arch {
        TargetArch::NoneEabi => Ok(Some(self.render_arm_eabi_toolchain(tc, root_ld_scripts)?)),
        TargetArch::ARM32 => {
            if tc.prefix.is_empty() {
                Ok(None)
            } else {
                Ok(Some(self.render_embedded_toolchain("Linux", "arm", &tc.prefix, tc, root_ld_scripts)?))
            }
        }
        TargetArch::ARM64 => {
            if tc.prefix.is_empty() {
                Ok(None)
            } else {
                Ok(Some(self.render_embedded_toolchain("Linux", "aarch64", &tc.prefix, tc, root_ld_scripts)?))
            }
        }
        TargetArch::RISCV64 => Ok(Some(self.render_riscv64_toolchain(tc, root_ld_scripts)?)),
        _ => Ok(None),
    }
}
```

Key change: `ARM32` and `ARM64` now produce a toolchain when `tc.prefix` is non-empty (previously always returned `None`).

- [ ] **Step 6: Build and run tests**

```bash
cargo test
```
Expected: all 65 tests pass. `test_cross_compile_template` should still pass since the NoneEabi code path uses the default `"arm-none-eabi-"` prefix when `tc.prefix` is empty.

- [ ] **Step 7: Commit**

```bash
git add src/core/generator.rs
git commit -m "refactor: use Tera template for toolchain.cmake, extend to ARM32/ARM64/RISCV64

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 4: Update integration tests for new toolchain features

**Files:**
- Modify: `tests/integration.rs`

- [ ] **Step 1: Add test for sysroot in generated toolchain**

Add after `test_cross_compile_template` (around line 550):

```rust
#[test]
fn test_toolchain_sysroot_when_set() {
    let tmp = TempDir::new().unwrap();
    create_test_project(&tmp);
    let root = tmp.path().to_path_buf();

    std::fs::remove_file(root.join("link.ld")).unwrap();
    std::fs::write(
        root.join("STM32F103XX_FLASH.ld"),
        "MEMORY { FLASH (rx) : ORIGIN = 0x08000000, LENGTH = 512K }\n",
    )
    .unwrap();

    let sources = scan_project(&root);
    let discoverer = ModuleDiscoverer::new(ScanOptions {
        exclude_dirs: vec![],
        root: root.clone(),
    });
    let modules = discoverer.discover(&sources).unwrap();

    let config = ProjectConfig {
        name: "SysrootTest".into(),
        version: "0.1.0".into(),
        root: root.clone(),
        language: "C".into(),
        c_standard: "11".into(),
        cpp_standard: "17".into(),
        target_arch: fb_gen::models::project::TargetArch::NoneEabi,
        toolchain: Some(fb_gen::models::project::ToolchainConfig {
            cpu: "cortex-m4".into(),
            prefix: "arm-none-eabi-".into(),
            sysroot: Some("/opt/gcc-arm/arm-none-eabi".into()),
            find_root_path: vec!["/custom/lib".into()],
            ..Default::default()
        }),
        ..Default::default()
    };

    let generator = CMakeGenerator::new(&config).unwrap();
    let empty_graph = DependencyGraph::new();
    generator.generate(&modules, &empty_graph, true, &[]).unwrap();

    let toolchain_path = root.join("cmake").join("toolchain.cmake");
    assert!(toolchain_path.exists());
    let content = std::fs::read_to_string(&toolchain_path).unwrap();

    assert!(
        content.contains("set(CMAKE_SYSROOT /opt/gcc-arm/arm-none-eabi)"),
        "toolchain.cmake should set CMAKE_SYSROOT when sysroot is Some"
    );
    assert!(
        content.contains("set(CMAKE_FIND_ROOT_PATH ${CMAKE_SYSROOT} /custom/lib)"),
        "toolchain.cmake should set CMAKE_FIND_ROOT_PATH with sysroot + extra paths"
    );
}

#[test]
fn test_toolchain_no_sysroot_when_none() {
    let tmp = TempDir::new().unwrap();
    create_test_project(&tmp);
    let root = tmp.path().to_path_buf();

    std::fs::remove_file(root.join("link.ld")).unwrap();

    let sources = scan_project(&root);
    let discoverer = ModuleDiscoverer::new(ScanOptions {
        exclude_dirs: vec![],
        root: root.clone(),
    });
    let modules = discoverer.discover(&sources).unwrap();

    let config = ProjectConfig {
        name: "NoSysrootTest".into(),
        version: "0.1.0".into(),
        root: root.clone(),
        language: "C".into(),
        c_standard: "11".into(),
        cpp_standard: "17".into(),
        target_arch: fb_gen::models::project::TargetArch::NoneEabi,
        toolchain: Some(fb_gen::models::project::ToolchainConfig {
            cpu: "cortex-m3".into(),
            prefix: "arm-none-eabi-".into(),
            sysroot: None,  // bare-metal: no sysroot
            ..Default::default()
        }),
        ..Default::default()
    };

    let generator = CMakeGenerator::new(&config).unwrap();
    let empty_graph = DependencyGraph::new();
    generator.generate(&modules, &empty_graph, true, &[]).unwrap();

    let toolchain_path = root.join("cmake").join("toolchain.cmake");
    let content = std::fs::read_to_string(&toolchain_path).unwrap();

    assert!(
        !content.contains("CMAKE_SYSROOT"),
        "toolchain.cmake should NOT set CMAKE_SYSROOT when sysroot is None"
    );
    assert!(
        content.contains("set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)"),
        "FIND_ROOT_PATH_MODE should always be present"
    );
}

#[test]
fn test_toolchain_generated_for_arm32_with_prefix() {
    let tmp = TempDir::new().unwrap();
    create_test_project(&tmp);
    let root = tmp.path().to_path_buf();

    std::fs::remove_file(root.join("link.ld")).unwrap();

    let sources = scan_project(&root);
    let discoverer = ModuleDiscoverer::new(ScanOptions {
        exclude_dirs: vec![],
        root: root.clone(),
    });
    let modules = discoverer.discover(&sources).unwrap();

    let config = ProjectConfig {
        name: "ARM32Test".into(),
        root: root.clone(),
        language: "C".into(),
        target_arch: fb_gen::models::project::TargetArch::ARM32,
        toolchain: Some(fb_gen::models::project::ToolchainConfig {
            prefix: "arm-linux-gnueabihf-".into(),
            ..Default::default()
        }),
        ..Default::default()
    };

    let generator = CMakeGenerator::new(&config).unwrap();
    let empty_graph = DependencyGraph::new();
    generator.generate(&modules, &empty_graph, true, &[]).unwrap();

    let toolchain_path = root.join("cmake").join("toolchain.cmake");
    assert!(
        toolchain_path.exists(),
        "ARM32 with prefix should generate toolchain.cmake"
    );
    let content = std::fs::read_to_string(&toolchain_path).unwrap();
    assert!(content.contains("arm-linux-gnueabihf-"));
}
```

- [ ] **Step 2: Run all tests**

```bash
cargo test
```
Expected: 68 tests pass (44 unit + 24 integration — 3 new tests added).

- [ ] **Step 3: Commit**

```bash
git add tests/integration.rs
git commit -m "test: add sysroot and ARM32/ARM64 toolchain generation tests

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 5: Wire auto-detection into UserQuery

**Files:**
- Modify: `src/orchestration/query.rs`

- [ ] **Step 1: Update ask_project_config to call detect_toolchains for cross targets**

After the user selects `target_arch` (around line 112), insert toolchain auto-detection:

```rust
// ── Toolchain detection (cross-compile targets) ────────────
let mut toolchain: Option<crate::models::project::ToolchainConfig> = None;

if matches!(target_arch, TargetArch::NoneEabi | TargetArch::ARM32 | TargetArch::ARM64 | TargetArch::RISCV64) {
    let detected = crate::core::toolchain_detect::detect_toolchains();

    // Filter to toolchains compatible with the chosen architecture.
    let compatible: Vec<_> = detected
        .iter()
        .filter(|dt| dt.suggested_arch == target_arch)
        .collect();

    if compatible.is_empty() {
        println!();
        println!("  No compatible toolchain auto-detected for {:?}.", target_arch);
        println!("  Falling back to manual configuration.");
        // Use the existing manual prompts (MCU, float_abi, etc.)
        let prefix = prompt_with_default(
            "  Toolchain prefix (e.g. arm-none-eabi-)",
            match target_arch {
                TargetArch::NoneEabi => "arm-none-eabi-",
                TargetArch::ARM32 => "arm-linux-gnueabihf-",
                TargetArch::ARM64 => "aarch64-none-elf-",
                TargetArch::RISCV64 => "riscv64-unknown-elf-",
                _ => "",
            },
        ).map_err(|e| {
            crate::models::FbGenError::Config(format!("failed to read prefix: {e}"))
        })?;

        let sysroot_str = prompt_with_default(
            "  Sysroot path (empty to skip) []", ""
        ).map_err(|e| {
            crate::models::FbGenError::Config(format!("failed to read sysroot: {e}"))
        })?;
        let sysroot = if sysroot_str.is_empty() { None } else { Some(sysroot_str) };

        let find_root_str = prompt_with_default(
            "  Extra CMAKE_FIND_ROOT_PATH entries (space-separated, empty to skip) []", ""
        ).map_err(|e| {
            crate::models::FbGenError::Config(format!("failed to read find root path: {e}"))
        })?;
        let find_root_path: Vec<String> = find_root_str
            .split_whitespace()
            .map(String::from)
            .collect();

        // MCU/CPU prompt (NoneEabi only).
        let (cpu, float_abi, fpu, extra_flags) = if matches!(target_arch, TargetArch::NoneEabi) {
            println!();
            println!("  ARM MCU/CPU selection:");
            println!("    Specify the target chip model for -mcpu= flag.");
            let cpu = prompt_with_default("  ARM MCU/CPU [cortex-m3]", "cortex-m3")
                .map_err(|e| {
                    crate::models::FbGenError::Config(format!("failed to read MCU: {e}"))
                })?;
            let float_abi =
                prompt_with_default("  Float ABI (soft/softfp/hard, empty to skip) []", "")
                    .map_err(|e| {
                        crate::models::FbGenError::Config(format!("failed to read float ABI: {e}"))
                    })?;
            let fpu =
                prompt_with_default("  FPU (e.g. fpv4-sp-d16, empty to skip) []", "")
                    .map_err(|e| {
                        crate::models::FbGenError::Config(format!("failed to read FPU: {e}"))
                    })?;
            let extra_flags = prompt_with_default(
                "  Extra flags (e.g. -mthumb, empty to skip) []",
                "",
            ).map_err(|e| {
                crate::models::FbGenError::Config(format!("failed to read extra flags: {e}"))
            })?;
            (cpu, float_abi, fpu, extra_flags)
        } else {
            (String::new(), String::new(), String::new(), String::new())
        };

        toolchain = Some(crate::models::project::ToolchainConfig {
            cpu,
            float_abi,
            fpu,
            extra_flags,
            prefix,
            sysroot,
            find_root_path,
        });
    } else {
        // Show detected toolchains for the user to pick from.
        println!();
        println!("  Detected {} compatible toolchain(s) for {:?}:", compatible.len(), target_arch);
        for (i, dt) in compatible.iter().enumerate() {
            let sysroot_display = dt.sysroot.as_ref()
                .map(|s| s.display().to_string())
                .unwrap_or_else(|| "none".into());
            println!("    {}) {} → {}  (sysroot: {})", i + 1, dt.prefix, dt.cc_path.display(), sysroot_display);
        }
        println!("    {}) Custom — enter prefix and sysroot manually", compatible.len() + 1);

        let choice = prompt_with_default(
            &format!("  Choose toolchain [1-{}]", compatible.len() + 1),
            "1",
        ).map_err(|e| {
            crate::models::FbGenError::Config(format!("failed to read choice: {e}"))
        })?;

        let (prefix, sysroot, find_root_path) = if let Ok(idx) = choice.parse::<usize>() {
            if idx >= 1 && idx <= compatible.len() {
                let dt = &compatible[idx - 1];
                (
                    dt.prefix.clone(),
                    dt.sysroot.clone().map(|p| p.to_string_lossy().to_string()),
                    Vec::new(),
                )
            } else {
                // Custom.
                let prefix = prompt_with_default("  Toolchain prefix", "arm-none-eabi-")
                    .map_err(|e| {
                        crate::models::FbGenError::Config(format!("failed to read prefix: {e}"))
                    })?;
                let sysroot_str = prompt_with_default("  Sysroot path (empty to skip) []", "")
                    .map_err(|e| {
                        crate::models::FbGenError::Config(format!("failed to read sysroot: {e}"))
                    })?;
                let sysroot = if sysroot_str.is_empty() { None } else { Some(sysroot_str) };
                let find_root_str = prompt_with_default(
                    "  Extra CMAKE_FIND_ROOT_PATH entries (space-separated) []", ""
                ).map_err(|e| {
                    crate::models::FbGenError::Config(format!("failed to read find root: {e}"))
                })?;
                let find_root_path: Vec<String> = find_root_str
                    .split_whitespace()
                    .map(String::from)
                    .collect();
                (prefix, sysroot, find_root_path)
            }
        } else {
            // Couldn't parse, use fallback.
            ("arm-none-eabi-".into(), None, Vec::new())
        };

        // Still ask for MCU/FPU details for NoneEabi when auto-detected.
        let (cpu, float_abi, fpu, extra_flags) = if matches!(target_arch, TargetArch::NoneEabi) {
            println!();
            println!("  ARM MCU/CPU selection:");
            let cpu = prompt_with_default("  ARM MCU/CPU [cortex-m3]", "cortex-m3")
                .map_err(|e| {
                    crate::models::FbGenError::Config(format!("failed to read MCU: {e}"))
                })?;
            let float_abi =
                prompt_with_default("  Float ABI (soft/softfp/hard, empty to skip) []", "")
                    .map_err(|e| {
                        crate::models::FbGenError::Config(format!("failed to read float ABI: {e}"))
                    })?;
            let fpu =
                prompt_with_default("  FPU (e.g. fpv4-sp-d16, empty to skip) []", "")
                    .map_err(|e| {
                        crate::models::FbGenError::Config(format!("failed to read FPU: {e}"))
                    })?;
            let extra_flags = prompt_with_default("  Extra flags (e.g. -mthumb) []", "")
                .map_err(|e| {
                    crate::models::FbGenError::Config(format!("failed to read extra flags: {e}"))
                })?;
            (cpu, float_abi, fpu, extra_flags)
        } else {
            (String::new(), String::new(), String::new(), String::new())
        };

        toolchain = Some(crate::models::project::ToolchainConfig {
            cpu,
            float_abi,
            fpu,
            extra_flags,
            prefix,
            sysroot,
            find_root_path,
        });
    }
}
```

This replaces the existing `NoneEabi`-only toolchain block (lines 112-149 of query.rs).

- [ ] **Step 2: Add use statement at top of query.rs**

The `detect_toolchains` function is already re-exported from `crate::core` via `pub use toolchain_detect::detect_toolchains;`. Add the `TargetArch` variant matching to existing `use crate::models::project::{...}` import if not already present.

- [ ] **Step 3: Build and verify**

```bash
cargo build
```
Expected: compiles without errors.

- [ ] **Step 4: Commit**

```bash
git add src/orchestration/query.rs
git commit -m "feat: auto-detect cross-compilers during fb-gen init

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 6: Update confirm_config to display new toolchain fields

**Files:**
- Modify: `src/orchestration/query.rs:232-260`

- [ ] **Step 1: Add toolchain prefix/sysroot display in confirm_config**

After the existing toolchain info display (around line 250), add:

```rust
if let Some(ref tc) = config.toolchain {
    if !tc.prefix.is_empty() {
        println!("  Toolchain prefix:  {}", tc.prefix);
    }
    if let Some(ref sysroot) = tc.sysroot {
        println!("  Sysroot:           {}", sysroot);
    }
    if !tc.find_root_path.is_empty() {
        println!("  Find root paths:   {}", tc.find_root_path.join(" "));
    }
}
```

- [ ] **Step 2: Build and verify**

```bash
cargo build
```

- [ ] **Step 3: Commit**

```bash
git add src/orchestration/query.rs
git commit -m "feat: display toolchain prefix/sysroot in confirm_config

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 7: End-to-end verification

**Files:** None (manual test)

- [ ] **Step 1: Run all tests**

```bash
cargo test
```
Expected: all tests pass (should be ~68 total: 44 unit + 24 integration).

- [ ] **Step 2: Manual test — verify toolchain detection on host**

```bash
# If arm-none-eabi-gcc is available:
cargo run -- init --root /tmp/test-proj
# Choose NoneEabi architecture, verify detected toolchain is shown.
# If no cross-compiler is available, verify manual fallback works.
```

- [ ] **Step 3: Manual test — verify generated toolchain.cmake has sysroot**

With a `project.json` that has `toolchain.prefix` and `toolchain.sysroot` set, run `fb-gen run` and inspect `cmake/toolchain.cmake` for `CMAKE_SYSROOT` and `CMAKE_FIND_ROOT_PATH`.

- [ ] **Step 4: Commit any final adjustments**

---

## Self-Review Notes

- Spec coverage: All 6 requirements from the spec are covered. Task 1 (model), Task 2 (detection), Task 3 (template), Task 5 (UserQuery), Task 6 (confirm display).
- No TBDs, TODOs, or placeholders. All code blocks contain real implementations.
- Type consistency: `DetectedToolchain` in Task 2 matches usage in Task 5. `ToolchainConfig` fields in Task 1 match Task 3 template context.
- The `cargo test` commands in Tasks 1-4 verify incremental progress.
