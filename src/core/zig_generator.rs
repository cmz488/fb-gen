//! Zig build system generator — produces `build.zig` from module and
//! dependency data.
//!
//! Activated when `ProjectConfig::build_system == BuildSystem::Zig`.
//! Completely independent of the CMake code path.
//!
//! # Leaf-model dependency graph
//!
//! The root module (containing `main()`) is the executable.  Every other
//! module is a static library leaf.  Dependencies flow from the root
//! executable down to the leaf libraries — matching `#include "..."` edges
//! discovered by the analyser.  Assembly (`.s` / `.S`) and linker-script
//! (`.ld`) files in the root directory are assigned to the root module.
//! Orphan linker scripts (in directories with no source files) are also
//! attached to the root module.

use crate::models::dependency::DependencyGraph;
use crate::models::error::{FbGenError, FbGenResult};
use crate::models::module::{CMakeModule, SourceFile, SourceType, TargetType};
use crate::models::project::{ProjectConfig, TargetArch};
use std::fs;
use std::path::PathBuf;
use std::sync::LazyLock;
use regex::Regex;

/// Regex matching `int main(` or `void main(` — used to identify entry-point
/// source files during orphan filtering.
static MAIN_FUNCTION_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:int|void)\s+main\s*\(").expect("MAIN_FUNCTION_RE regex")
});

// ── Zig target descriptor ───────────────────────────────────────────────────

/// A resolved Zig target: separates the CPU architecture enum variant from
/// the module path where CPU models are defined.
///
/// Zig organises CPU models under `<module>.cpu.<name>`.  The module does
/// *not* always match the arch enum variant — e.g. `x86_64` models live
/// under `std.Target.x86`, `riscv32` models under `std.Target.riscv`.
pub(crate) struct ZigTarget {
    /// Zig module name for CPU model paths (e.g. `"x86"`, `"arm"`, `"riscv"`).
    pub target_module: &'static str,
    /// Zig `cpu_arch` enum variant (e.g. `.x86_64`, `.thumb`, `.riscv32`).
    pub cpu_arch: &'static str,
    /// Default CPU model name under `std.Target.<module>.cpu.<name>`.
    pub default_cpu: &'static str,
    /// Zig `os_tag` enum variant (`.linux`, `.freestanding`).
    pub os_tag: &'static str,
    /// Zig `abi` enum variant (`.gnu`, `.none`, …).
    pub abi: &'static str,
}

impl ZigTarget {
    /// Build a Zig target triple string (e.g. `"thumb-freestanding-none"`).
    pub fn triple(&self) -> String {
        format!("{}-{}-{}", self.cpu_arch, self.os_tag, self.abi)
    }

    /// Human-readable label for the target (e.g. "ARM Cortex-M (thumb)").
    pub fn label(&self) -> String {
        format!(
            "{} ({}.{}.{})",
            self.cpu_arch, self.target_module, self.default_cpu, self.abi
        )
    }
}

pub(crate) fn zig_target(arch: &TargetArch) -> ZigTarget {
    match arch {
        // ── Native / Linux targets ──────────────────────────────────
        TargetArch::X86_64 => ZigTarget {
            target_module: "x86", cpu_arch: "x86_64", default_cpu: "x86_64",
            os_tag: "linux", abi: "gnu",
        },
        TargetArch::X86 => ZigTarget {
            target_module: "x86", cpu_arch: "x86", default_cpu: "generic",
            os_tag: "linux", abi: "gnu",
        },
        // ── ARM ────────────────────────────────────────────────────
        TargetArch::NoneEabi => ZigTarget {
            target_module: "arm", cpu_arch: "thumb", default_cpu: "cortex_m3",
            // .eabi selects soft-float ABI — Cortex-M3/M4 without FPU need this.
            // .none would default to hard-float, which faults on M3.
            os_tag: "freestanding", abi: "eabi",
        },
        TargetArch::ARM32 => ZigTarget {
            target_module: "arm", cpu_arch: "arm", default_cpu: "generic",
            os_tag: "freestanding", abi: "none",
        },
        // ── AArch64 ────────────────────────────────────────────────
        TargetArch::ARM64 => ZigTarget {
            target_module: "aarch64", cpu_arch: "aarch64", default_cpu: "generic",
            os_tag: "freestanding", abi: "none",
        },
        // ── RISC-V — all models under `std.Target.riscv.cpu.*` ─────
        TargetArch::RISCV64 => ZigTarget {
            target_module: "riscv", cpu_arch: "riscv64", default_cpu: "generic_rv64",
            os_tag: "freestanding", abi: "none",
        },
        TargetArch::RISCV32 => ZigTarget {
            target_module: "riscv", cpu_arch: "riscv32", default_cpu: "generic_rv32",
            os_tag: "freestanding", abi: "none",
        },
        // ── Xtensa (ESP32) — only `generic` model exists ───────────
        TargetArch::Xtensa => ZigTarget {
            target_module: "xtensa", cpu_arch: "xtensa", default_cpu: "generic",
            os_tag: "freestanding", abi: "none",
        },
        // ── WASM ──────────────────────────────────────────────────
        TargetArch::WASM => ZigTarget {
            target_module: "wasm", cpu_arch: "wasm32", default_cpu: "generic",
            os_tag: "freestanding", abi: "none",
        },
        // ── Custom — assume native ─────────────────────────────────
        TargetArch::Custom(_) => ZigTarget {
            target_module: "x86", cpu_arch: "x86_64", default_cpu: "x86_64",
            os_tag: "linux", abi: "gnu",
        },
    }
}

// ── Template ─────────────────────────────────────────────────────────────────
//
// Follows the Zig 0.16 build system API:
//   - `b.standardTargetOptions(.{})` for native (user-overridable).
//   - `b.resolveTargetQuery(.{...})` for cross / embedded (locked target).
//   - `b.createModule(.{ .target, .optimize, .link_libc?, .link_libcpp? })`.
//   - `b.addExecutable(.{ .name, .root_module })`.
//   - `b.addLibrary(.{ .name, .linkage = .static, .root_module })`.
//   - `mod.addCSourceFiles(.{ .files, .flags })`.
//   - `mod.addCSourceFile(.{ .file, .flags })` for individual asm files.
//   - `mod.addIncludePath(b.path(...))`.
//   - `mod.linkLibrary(...)` on the *Module* for inter-module deps.
//   - `exe.setLinkerScript(b.path(...))` — MUST appear after addExecutable.
//
// # Leaf model
// Dependencies flow root → leaf libraries.  The dependency section calls
// `linkLibrary` on the *Module* to link the dependency's compiled output.

const ZIG_BUILD_TEMPLATE: &str = r#"// Auto-generated by fb-gen — do not edit manually.
// Target: {{ target_label }}
{% if device_cpu -%}
// Device / CPU: {{ device_cpu }}
{% endif -%}
const std = @import("std");

pub fn build(b: *std.Build) void {
{% if is_cross -%}
    // Cross-compilation: locked target (not overridable via -Dtarget).
    const target = b.resolveTargetQuery(.{
        .cpu_arch = .{{ cpu_arch }},
        .cpu_model = .{ .explicit = &std.Target.{{ target_module }}.cpu.{{ cpu_model }} },
        .os_tag = .{{ os_tag }},
        .abi = .{{ abi }},
    });
{% else -%}
    // Native: user-overridable via -Dtarget= / -Dcpu=.
    const target = b.standardTargetOptions(.{});
{% endif -%}
    const optimize = b.standardOptimizeOption(.{
        .preferred_optimize_mode = .{{ optimize }},
    });

    // ── Modules (leaf model: source-bearing only) ────────────
{% for mod in modules -%}
{% if mod.is_executable -%}
    // Executable: {{ mod.name }}
    const {{ mod.name_safe }}_mod = b.createModule(.{
        .target = target,
        .optimize = optimize,
{% if mod.link_libc -%}
        .link_libc = true,
{% endif -%}
{% if mod.link_libcpp -%}
        .link_libcpp = true,
{% endif -%}
    });
{% if mod.c_sources | length > 0 -%}
    {{ mod.name_safe }}_mod.addCSourceFiles(.{
        .files = &.{ {% for src in mod.c_sources -%}"{{ src }}", {% endfor -%} },
        .flags = &.{ {% for flag in mod.c_flags -%}"{{ flag }}", {% endfor -%} },
    });
{% endif -%}
{% if mod.cxx_sources | length > 0 -%}
    {{ mod.name_safe }}_mod.addCSourceFiles(.{
        .files = &.{ {% for src in mod.cxx_sources -%}"{{ src }}", {% endfor -%} },
        .flags = &.{ {% for flag in mod.cxx_flags -%}"{{ flag }}", {% endfor -%} },
    });
{% endif -%}
{% for asm in mod.asm_cpp_sources -%}
    // .S (with preprocessor)
    {{ mod.name_safe }}_mod.addCSourceFile(.{ .file = b.path("{{ asm }}"), .flags = &.{ {% for flag in mod.asm_flags -%}"{{ flag }}", {% endfor -%}"-x", "assembler-with-cpp" } });
{% endfor -%}
{% for asm in mod.asm_raw_sources -%}
    // .s (no preprocessor)
    {{ mod.name_safe }}_mod.addCSourceFile(.{ .file = b.path("{{ asm }}"), .flags = &.{ {% for flag in mod.asm_flags -%}"{{ flag }}", {% endfor -%}"-x", "assembler" } });
{% endfor -%}
{% for inc in mod.include_dirs -%}
{% if inc | length > 0 -%}
    {{ mod.name_safe }}_mod.addIncludePath(b.path("{{ inc }}"));
{% endif -%}
{% endfor -%}
{% for def in mod.compile_definitions -%}
    {{ mod.name_safe }}_mod.addCMacro("{{ def.name }}", "{{ def.value }}");
{% endfor -%}
    const {{ mod.name_safe }} = b.addExecutable(.{
        .name = "{{ mod.exe_name }}",
        .root_module = {{ mod.name_safe }}_mod,
    });
{% if mod.linker_script -%}
    {{ mod.name_safe }}.setLinkerScript(b.path("{{ mod.linker_script }}"));
{% endif -%}
{% if needs_objcopy -%}
    // Raw binary for flashing (e.g. via OpenOCD / STM32CubeProgrammer).
    const {{ mod.name_safe }}_bin = b.addObjCopy({{ mod.name_safe }}.getEmittedBin(), .{
        .format = .bin,
    });
    {{ mod.name_safe }}_bin.step.dependOn(&{{ mod.name_safe }}.step);
    b.getInstallStep().dependOn(&b.addInstallBinFile({{ mod.name_safe }}_bin.getOutput(), "{{ project_name }}.bin").step);
{% endif -%}

{% else -%}
    // Library: {{ mod.name }}
    const {{ mod.name_safe }}_mod = b.createModule(.{
        .target = target,
        .optimize = optimize,
{% if mod.link_libc -%}
        .link_libc = true,
{% endif -%}
{% if mod.link_libcpp -%}
        .link_libcpp = true,
{% endif -%}
    });
{% if mod.c_sources | length > 0 -%}
    {{ mod.name_safe }}_mod.addCSourceFiles(.{
        .files = &.{ {% for src in mod.c_sources -%}"{{ src }}", {% endfor -%} },
        .flags = &.{ {% for flag in mod.c_flags -%}"{{ flag }}", {% endfor -%} },
    });
{% endif -%}
{% if mod.cxx_sources | length > 0 -%}
    {{ mod.name_safe }}_mod.addCSourceFiles(.{
        .files = &.{ {% for src in mod.cxx_sources -%}"{{ src }}", {% endfor -%} },
        .flags = &.{ {% for flag in mod.cxx_flags -%}"{{ flag }}", {% endfor -%} },
    });
{% endif -%}
{% for asm in mod.asm_cpp_sources -%}
    // .S (with preprocessor)
    {{ mod.name_safe }}_mod.addCSourceFile(.{ .file = b.path("{{ asm }}"), .flags = &.{ {% for flag in mod.asm_flags -%}"{{ flag }}", {% endfor -%}"-x", "assembler-with-cpp" } });
{% endfor -%}
{% for asm in mod.asm_raw_sources -%}
    // .s (no preprocessor)
    {{ mod.name_safe }}_mod.addCSourceFile(.{ .file = b.path("{{ asm }}"), .flags = &.{ {% for flag in mod.asm_flags -%}"{{ flag }}", {% endfor -%}"-x", "assembler" } });
{% endfor -%}
{% for inc in mod.include_dirs -%}
{% if inc | length > 0 -%}
    {{ mod.name_safe }}_mod.addIncludePath(b.path("{{ inc }}"));
{% endif -%}
{% endfor -%}
{% for def in mod.compile_definitions -%}
    {{ mod.name_safe }}_mod.addCMacro("{{ def.name }}", "{{ def.value }}");
{% endfor -%}
    const {{ mod.name_safe }} = b.addLibrary(.{
        .name = "{{ mod.name }}",
        .linkage = .static,
        .root_module = {{ mod.name_safe }}_mod,
    });
{% if mod.linker_script -%}
    {{ mod.name_safe }}.setLinkerScript(b.path("{{ mod.linker_script }}"));
{% endif -%}

{% endif -%}
{% endfor -%}
    // ── Module dependencies (leaf model: root → libs) ──────────
{% for edge in dependencies -%}
{% if edge.is_real_dep -%}
    {{ edge.from_safe }}_mod.addIncludePath(b.path("{{ edge.to_path }}"));
{% endif -%}
{% if edge.to_has_sources -%}
    {{ edge.from_safe }}_mod.linkLibrary({{ edge.to_safe }});
{% endif -%}
{% endfor -%}

    // ── Install ──────────────────────────────────────────────
{% for mod in modules -%}
{% if mod.is_executable -%}
    b.installArtifact({{ mod.name_safe }});
{% endif -%}
{% endfor -%}
}
"#;

/// Bare-metal runtime stubs emitted for freestanding (cross-compilation)
/// targets.  GCC startup assembly (e.g. STM32CubeMX `startup_*.s`) calls
/// `__libc_init_array` to run `.init_array` constructors, but Zig's
/// freestanding environment doesn't link libc and thus doesn't provide
/// these symbols.  This tiny `.c` file provides empty stubs so the linker
/// can resolve the references without pulling in a full C runtime.
const BARE_METAL_STUB_SRC: &str = r#"/* Auto-generated by fb-gen — bare-metal init stubs. */
void __libc_init_array(void) {}
void __libc_fini_array(void) {}
"#;

// ── Generator ────────────────────────────────────────────────────────────────

pub struct ZigGenerator {
    config: ProjectConfig,
    tera: tera::Tera,
}

impl ZigGenerator {
    pub fn new(config: &ProjectConfig) -> FbGenResult<Self> {
        let mut tera = tera::Tera::default();
        tera.add_raw_template("zig", ZIG_BUILD_TEMPLATE)
            .map_err(FbGenError::Template)?;
        Ok(Self {
            config: config.clone(),
            tera,
        })
    }

    fn needs_libc(arch: &TargetArch) -> bool {
        // Custom arches are mapped to x86_64-linux-gnu by zig_target(),
        // so they need libc too — otherwise C standard library functions
        // (printf, malloc, etc.) will fail to link.
        matches!(arch, TargetArch::X86_64 | TargetArch::X86 | TargetArch::Custom(_))
    }

    fn resolve_cpu_model(&self, target: &ZigTarget) -> String {
        self.config
            .toolchain
            .as_ref()
            .and_then(|tc| {
                if tc.cpu.is_empty() {
                    None
                } else {
                    // GCC convention uses hyphens (cortex-m3), Zig uses
                    // underscores (cortex_m3).  Normalise here so we never
                    // generate an invalid Zig identifier.
                    Some(tc.cpu.replace('-', "_"))
                }
            })
            .unwrap_or_else(|| target.default_cpu.to_string())
    }

    fn module_has_cxx(m: &CMakeModule) -> bool {
        m.sources
            .iter()
            .any(|s| matches!(s.source_type, SourceType::CppSource | SourceType::CppHeader))
    }

    fn split_define(def: &str) -> (&str, &str) {
        match def.split_once('=') {
            Some((name, value)) => (name, value),
            None => (def, ""),
        }
    }

    /// Build C compiler flags (language standard + toolchain extras).
    fn c_compile_flags(&self) -> Vec<String> {
        let mut flags: Vec<String> = Vec::new();
        flags.push(format!("-std=c{}", self.config.c_standard));
        if let Some(ref tc) = self.config.toolchain {
            if !tc.extra_flags.is_empty() {
                for f in tc.extra_flags.split_whitespace() {
                    flags.push(f.to_string());
                }
            }
        }
        flags
    }

    /// Build C++ compiler flags (language standard + toolchain extras).
    fn cxx_compile_flags(&self) -> Vec<String> {
        let mut flags: Vec<String> = Vec::new();
        flags.push(format!("-std=c++{}", self.config.cpp_standard));
        if let Some(ref tc) = self.config.toolchain {
            if !tc.extra_flags.is_empty() {
                for f in tc.extra_flags.split_whitespace() {
                    flags.push(f.to_string());
                }
            }
        }
        flags
    }

    /// Build arch-only flags for assembly files (no `-std=` — meaningless for asm).
    fn build_asm_flags(&self) -> Vec<String> {
        let mut flags: Vec<String> = Vec::new();

        if let Some(ref tc) = self.config.toolchain {
            if !tc.extra_flags.is_empty() {
                for f in tc.extra_flags.split_whitespace() {
                    flags.push(f.to_string());
                }
            }
        }

        flags
    }

    /// Check whether a source-file name indicates it needs the C preprocessor
    /// (`.S` — uppercase extension).
    fn asm_needs_cpp(file_name: &str) -> bool {
        file_name.ends_with(".S")
    }

    /// Decide whether a `.c`/`.cpp` source file is referenced by the project
    /// and should be compiled.  Orphans (no `#include` links and no matching
    /// `.h` in any module, and not containing `main()`) are excluded.
    ///
    /// Uses `contains` on the already-read content rather than re-reading;
    /// the caller should batch-read once if needed.
    /// Check whether a source file is referenced by the project (has local
    /// `#include` directives, a matching `.h` in some module, or contains
    /// `main()`).  Orphans are excluded from both `build.zig` and
    /// `compile_commands.json`.
    pub(crate) fn is_file_referenced(
        sf: &crate::models::module::SourceFile,
        modules: &[CMakeModule],
    ) -> bool {
        // Entry-point: file contains main() declaration.
        if sf.source_type.is_source() {
            if let Ok(content) = std::fs::read_to_string(&sf.path) {
                if MAIN_FUNCTION_RE.is_match(&content) {
                    return true;
                }
            }
        }
        // Connected via local `#include "..."` directives.
        if !sf.includes.is_empty() {
            return true;
        }
        // Has a matching header in some module (gpio.c ↔ gpio.h).
        let stem = sf
            .file_name
            .rsplit_once('.')
            .map(|(s, _)| s)
            .unwrap_or(&sf.file_name);
        for m in modules {
            for h in &m.headers {
                let h_stem = h
                    .file_name
                    .rsplit_once('.')
                    .map(|(s, _)| s)
                    .unwrap_or(&h.file_name);
                if h_stem == stem {
                    return true;
                }
            }
        }
        false
    }

    /// Generate `build.zig` at the project root.
    pub fn generate(
        &self,
        modules: &[CMakeModule],
        graph: &DependencyGraph,
        _force: bool,
        _user_modules: &[PathBuf],
    ) -> FbGenResult<()> {
        let target = zig_target(&self.config.target_arch);
        let is_cross = target.os_tag == "freestanding";
        let cpu_model = self.resolve_cpu_model(&target);
        let has_libc = Self::needs_libc(&self.config.target_arch);
        let has_libcpp = self.config.language == "CXX"
            || modules.iter().any(Self::module_has_cxx);
        let c_flags = self.c_compile_flags();
        let cxx_flags = self.cxx_compile_flags();
        let asm_flags = self.build_asm_flags();

        // Bare-metal (freestanding) targets need stubs for libc init
        // functions.  Written inside `.fb-gen/stubs/` so the scanner
        // never picks them up (`.fb-gen` is an excluded directory).
        // WASM is freestanding but uses its own runtime — no stubs.
        let is_wasm = matches!(self.config.target_arch, TargetArch::WASM);
        let mut needs_bare_metal_stub = is_cross && !is_wasm;
        let device_cpu = if is_cross || self.config.toolchain.is_some() {
            Some(format!(
                "{} ({})",
                cpu_model,
                target.label()
            ))
        } else {
            None
        };

        let optimize = if is_cross { "ReleaseSmall" } else { "Debug" };

        // Executable output name: use the project name (sanitised) rather
        // than the module name so the installed binary reflects the project.
        // Freestanding targets (except WASM) get a .elf extension.
        let project_name = sanitize_name(&self.config.name);
        let exe_ext = if is_cross && !is_wasm { ".elf" } else { "" };
        // Raw binary objcopy only makes sense for flashable MCU targets.
        let needs_objcopy = is_cross && !is_wasm;

        let mut ctx = tera::Context::new();
        ctx.insert("is_cross", &is_cross);
        ctx.insert("target_module", target.target_module);
        ctx.insert("cpu_arch", target.cpu_arch);
        ctx.insert("cpu_model", &cpu_model);
        ctx.insert("os_tag", target.os_tag);
        ctx.insert("abi", target.abi);
        ctx.insert("optimize", optimize);
        ctx.insert("target_label", &target.label());
        ctx.insert("device_cpu", &device_cpu);
        ctx.insert("project_name", &project_name);
        ctx.insert("exe_ext", exe_ext);
        ctx.insert("needs_objcopy", &needs_objcopy);

        // ═══════════════════════════════════════════════════════════════
        // Leaf-model: only modules with source / asm files become Zig
        // modules.  Header-only directories are inlined — their include
        // dirs are merged into the depending source-bearing modules.
        // ═══════════════════════════════════════════════════════════════

        // ── Classify modules ──────────────────────────────────────────
        let is_header_only = |m: &CMakeModule| -> bool {
            m.sources.is_empty() && m.asm_sources.is_empty()
        };
        let is_source_bearing = |m: &CMakeModule| -> bool {
            !m.sources.is_empty() || !m.asm_sources.is_empty()
        };

        // ── Collect orphan root asm / ld (root dir with only .s/.ld) ─
        let mut orphan_asm_cpp: Vec<String> = Vec::new();
        let mut orphan_asm_raw: Vec<String> = Vec::new();
        let mut orphan_ld: Vec<PathBuf> = Vec::new();

        for m in modules {
            if m.is_root && m.sources.is_empty() && m.headers.is_empty() {
                for s in &m.asm_sources {
                    if Self::asm_needs_cpp(&s.file_name) {
                        orphan_asm_cpp.push(s.relative_path.to_string_lossy().to_string());
                    } else {
                        orphan_asm_raw.push(s.relative_path.to_string_lossy().to_string());
                    }
                }
                for ld in &m.linker_scripts {
                    let rel = if ld.is_absolute() {
                        ld.strip_prefix(&self.config.root).unwrap_or(ld).to_path_buf()
                    } else {
                        ld.clone()
                    };
                    if !orphan_ld.contains(&rel) {
                        orphan_ld.push(rel);
                    }
                }
            }
        }

        // ── Inline header-only include dirs into source-bearing deps ─
        // For each source-bearing module, compute the transitive closure
        // of its dependencies and collect include dirs from header-only
        // modules along the way.
        let mut extra_includes: std::collections::HashMap<&str, Vec<String>> =
            std::collections::HashMap::new();

        for m in modules {
            if !is_source_bearing(m) {
                continue;
            }
            let mut collected: Vec<String> = Vec::new();
            let mut visited: std::collections::HashSet<String> =
                std::collections::HashSet::new();
            let mut stack: Vec<String> = graph
                .get_dependencies(&m.name)
                .into_iter()
                .map(|(n, _)| n)
                .collect();
            // Push the module's own name so we include direct deps too.
            // Actually we need the deps' deps.  Start from direct deps.
            while let Some(dep_name) = stack.pop() {
                if dep_name.is_empty() || !visited.insert(dep_name.clone()) {
                    continue;
                }
                if let Some(dep) = modules.iter().find(|modu| modu.name == dep_name) {
                    if is_header_only(dep) {
                        // Inline this header-only module's include dirs.
                        for inc in &dep.include_dirs {
                            let s = inc.to_string_lossy().to_string();
                            if !s.is_empty() && !collected.contains(&s) {
                                collected.push(s);
                            }
                        }
                        // Also add the module's own relative path.
                        let rp = dep.relative_path.to_string_lossy().to_string();
                        if !rp.is_empty() && !collected.contains(&rp) {
                            collected.push(rp);
                        }
                    }
                    // Always traverse transitive deps — a header-only module
                    // can sit behind a source-bearing module in the dependency
                    // chain (e.g. A(source)→B(source)→C(header-only)).
                    for (sub, _) in graph.get_dependencies(&dep_name) {
                        if !visited.contains(&sub) {
                            stack.push(sub);
                        }
                    }
                }
            }
            if !collected.is_empty() {
                extra_includes.insert(&m.name, collected);
            }
        }

        // ── Build module contexts (source-bearing only) ──────────────
        let mut mod_ctxs: Vec<serde_json::Value> = Vec::new();
        let safe_names: Vec<(String, String)> = modules
            .iter()
            .map(|m| (m.name.clone(), sanitize_name(&m.name)))
            .collect();
        // Track all executable modules (after filtering).  Orphan asm/ld
        // files are merged into the FIRST executable only; auto-link orphan
        // libraries are linked to ALL executables.
        let mut actual_exes: Vec<String> = Vec::new();
        let mut orphans_merged: bool = false;

        for m in modules {
            // Skip empty root-only modules (merged into exe above).
            if m.is_root && m.sources.is_empty() && m.headers.is_empty() {
                continue;
            }
            // Skip header-only modules (inlined into dependents).
            if is_header_only(m) {
                continue;
            }

            let name_safe = sanitize_name(&m.name);

            // Filter to only referenced source files — exclude orphans
            // (e.g. STM32CubeMX syscalls.c / sysmem.c) that have no
            // #include links and no matching header in any module.
            // Split into C vs C++ so each language gets its own
            // `addCSourceFiles` call with the correct `-std=` flag.
            let referenced_sources: Vec<&SourceFile> = m
                .sources.iter()
                .filter(|s| Self::is_file_referenced(s, modules))
                .collect();
            let c_source_paths: Vec<String> = referenced_sources
                .iter()
                .filter(|s| matches!(s.source_type, SourceType::CSource))
                .map(|s| s.relative_path.to_string_lossy().to_string())
                .collect();
            let cxx_source_paths: Vec<String> = referenced_sources
                .iter()
                .filter(|s| matches!(s.source_type, SourceType::CppSource))
                .map(|s| s.relative_path.to_string_lossy().to_string())
                .collect();

            let mut asm_cpp_paths: Vec<String> = m
                .asm_sources.iter()
                .filter(|s| Self::asm_needs_cpp(&s.file_name))
                .map(|s| s.relative_path.to_string_lossy().to_string())
                .collect();
            let mut asm_raw_paths: Vec<String> = m
                .asm_sources.iter()
                .filter(|s| !Self::asm_needs_cpp(&s.file_name))
                .map(|s| s.relative_path.to_string_lossy().to_string())
                .collect();

            let is_executable = (m.has_main || m.is_root)
                && (!c_source_paths.is_empty() || !cxx_source_paths.is_empty())
                && m.target_type != TargetType::HeaderOnly;

            if is_executable {
                actual_exes.push(m.name.clone());
            }

            // Merge orphan root asm / ld into the FIRST executable only,
            // to avoid duplicate symbols when multiple executables exist.
            if is_executable && !orphans_merged {
                orphans_merged = true;
                asm_cpp_paths.extend(orphan_asm_cpp.iter().cloned());
                asm_raw_paths.extend(orphan_asm_raw.iter().cloned());
            }

            // Attach bare-metal init stubs to the FIRST executable module.
            // Written inside `.fb-gen/stubs/` so the scanner never picks
            // them up (`.fb-gen` is excluded from scanning) and
            // `is_file_referenced()` doesn't filter them out on re-scan.
            let mut c_source_paths = c_source_paths;
            if is_executable && needs_bare_metal_stub {
                let stub_dir = self.config.root.join(".fb-gen").join("stubs");
                let stub_path = stub_dir.join("_fb_gen_init.c");
                if !stub_path.exists() {
                    if let Err(e) = std::fs::create_dir_all(&stub_dir) {
                        // Can't create the stubs directory — skip the stub.
                        // The linker will report __libc_init_array as
                        // undefined, which is at least a clear error.
                        eprintln!(
                            "fb-gen warning: failed to create {}: {}",
                            stub_dir.display(),
                            e
                        );
                    } else if let Err(e) = std::fs::write(&stub_path, BARE_METAL_STUB_SRC) {
                        eprintln!(
                            "fb-gen warning: failed to write {}: {}",
                            stub_path.display(),
                            e
                        );
                    }
                }
                // Only add the stub to compilation if it actually exists
                // on disk (either just written, or left from a previous run).
                if stub_path.exists() {
                    let rel = stub_path
                        .strip_prefix(&self.config.root)
                        .unwrap_or(&stub_path);
                    c_source_paths.push(rel.to_string_lossy().to_string());
                }
                needs_bare_metal_stub = false; // only once
            }

            // Build final include dirs: own + inlined header-only transitive.
            // Skip empty paths (root directory "").
            let mut include_dirs: Vec<String> = m
                .include_dirs.iter()
                .map(|d| d.to_string_lossy().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if let Some(extra) = extra_includes.get(m.name.as_str()) {
                for e in extra {
                    if !include_dirs.contains(e) {
                        include_dirs.push(e.clone());
                    }
                }
            }

            // Device defines from toolchain config.
            let mut defines: Vec<serde_json::Value> = m
                .compile_definitions.iter()
                .map(|d| {
                    let (name, value) = Self::split_define(d);
                    serde_json::json!({ "name": name, "value": value })
                })
                .collect();
            if let Some(ref tc) = self.config.toolchain {
                for dd in &tc.device_defines {
                    let (name, value) = Self::split_define(dd);
                    defines.push(serde_json::json!({ "name": name, "value": value }));
                }
            }

            // Linker script: module's own, then orphan from root.
            // If absolute and outside the project root, keep as-is —
            // Zig's b.path() accepts absolute paths, though relative
            // paths are preferred for portability.
            let linker_script: Option<String> = m.linker_scripts.first()
                .or_else(|| if is_executable { orphan_ld.first() } else { None })
                .map(|p| {
                    if p.is_absolute() {
                        match p.strip_prefix(&self.config.root) {
                            Ok(rel) => rel.to_string_lossy().to_string(),
                            Err(_) => {
                                // Path is outside project root — pass through
                                // as absolute.  Zig's b.path() handles this,
                                // but the build may not be portable.
                                p.to_string_lossy().to_string()
                            }
                        }
                    } else {
                        p.to_string_lossy().to_string()
                    }
                });

            mod_ctxs.push(serde_json::json!({
                "name": m.name,
                "name_safe": name_safe,
                "is_executable": is_executable,
                "has_compile_step": true,
                // Placeholder — resolved after actual_exes is known.
                "exe_name": String::new(),
                "c_sources": c_source_paths,
                "c_flags": c_flags,
                "cxx_sources": cxx_source_paths,
                "cxx_flags": cxx_flags,
                "asm_flags": asm_flags,
                "asm_cpp_sources": asm_cpp_paths,
                "asm_raw_sources": asm_raw_paths,
                "include_dirs": include_dirs,
                "compile_definitions": defines,
                "link_libc": has_libc,
                "link_libcpp": has_libcpp && Self::module_has_cxx(m),
                "linker_script": linker_script,
            }));
        }
        // ── Resolve executable names ───────────────────────────────────
        // When there's only one executable, use the project name directly.
        // With multiple (unusual, but possible), append the module suffix
        // so every artifact gets a unique name.
        let multi_exe = actual_exes.len() > 1;
        for mc in &mut mod_ctxs {
            if mc["is_executable"].as_bool() == Some(true) {
                let name_safe = mc["name_safe"].as_str().unwrap_or("root");
                let exe_name = if multi_exe {
                    format!("{}_{}{}", project_name, name_safe, exe_ext)
                } else {
                    format!("{}{}", project_name, exe_ext)
                };
                mc["exe_name"] = serde_json::json!(exe_name);
            }
        }
        ctx.insert("modules", &mod_ctxs);

        // ── Dependency edges (leaf model: only source↔source) ────────
        // Include-path edges: every source module gets include paths from
        // its transitive header-only deps (already inlined above).
        // linkLibrary edges: only between source-bearing modules.
        let mut dep_ctxs: Vec<serde_json::Value> = Vec::new();
        let mut has_incoming: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        for m in modules {
            // Same skip conditions as the module-context loop above.
            if m.is_root && m.sources.is_empty() && m.headers.is_empty() {
                continue;
            }
            if !is_source_bearing(m) {
                continue;
            }
            let from_safe = safe_names
                .iter()
                .find(|(n, _)| n == &m.name)
                .map(|(_, s)| s.clone())
                .unwrap_or_else(|| sanitize_name(&m.name));

            // Traverse transitive dependencies through header-only modules
            // to find the effective source-bearing dependencies.  Example:
            // A (source) → B (header-only) → C (source)  ⇒  emit A→C.
            let mut visited: std::collections::HashSet<String> =
                std::collections::HashSet::new();
            // Start with direct deps of m.
            let mut stack: Vec<String> = graph
                .get_dependencies(&m.name)
                .into_iter()
                .map(|(n, _)| n)
                .collect();
            while let Some(dep_name) = stack.pop() {
                if dep_name.is_empty() || !visited.insert(dep_name.clone()) {
                    continue;
                }
                if let Some(dep) = modules.iter().find(|modu| modu.name == dep_name) {
                    if is_source_bearing(dep) {
                        // Found a source-bearing transitive dep — emit edge.
                        has_incoming.insert(dep_name.clone());
                        let to_path = dep.relative_path.to_string_lossy().to_string();
                        let to_safe = safe_names
                            .iter()
                            .find(|(n, _)| n == &dep_name)
                            .map(|(_, s)| s.clone())
                            .unwrap_or_else(|| sanitize_name(&dep_name));
                        dep_ctxs.push(serde_json::json!({
                            "from_safe": from_safe,
                            "to_safe": to_safe,
                            "to_path": to_path,
                            "to_has_sources": true,
                            "is_real_dep": true,
                        }));
                    } else {
                        // Header-only — traverse through to find source-bearing
                        // modules deeper in the dependency chain.
                        for (sub, _) in graph.get_dependencies(&dep_name) {
                            if !visited.contains(&sub) {
                                stack.push(sub);
                            }
                        }
                    }
                }
            }
        }

        // ── Auto-link orphan source libraries to ALL executables ────
        // Orphan libraries (no incoming edges in the dependency graph) are
        // linked to every executable so that all entry points can resolve
        // their symbols.
        for exe in &actual_exes {
            let exe_safe = sanitize_name(exe);
            for m in modules {
                if !is_source_bearing(m) || m.has_main || m.is_root {
                    continue;
                }
                if !has_incoming.contains(m.name.as_str()) {
                    let to_safe = sanitize_name(&m.name);
                    dep_ctxs.push(serde_json::json!({
                        "from_safe": exe_safe,
                        "to_safe": to_safe,
                        "to_path": m.relative_path.to_string_lossy().to_string(),
                        "to_has_sources": true,
                        "is_real_dep": false,
                    }));
                }
            }
        }

        ctx.insert("dependencies", &dep_ctxs);

        let rendered = self.tera.render("zig", &ctx).map_err(FbGenError::Template)?;

        let dest = self.config.root.join("build.zig");
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).map_err(FbGenError::Io)?;
        }
        fs::write(&dest, rendered).map_err(FbGenError::Io)?;

        Ok(())
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn sanitize_name(name: &str) -> String {
    if name.is_empty() {
        "root".to_string()
    } else {
        name.replace('-', "_")
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::project::BuildSystem;

    // ── zig_target ────────────────────────────────────────────────────

    #[test]
    fn test_zig_target_x86_64() {
        let t = zig_target(&TargetArch::X86_64);
        assert_eq!(t.target_module, "x86");
        assert_eq!(t.cpu_arch, "x86_64");
        assert_eq!(t.os_tag, "linux");
        assert_eq!(t.abi, "gnu");
        assert_eq!(t.triple(), "x86_64-linux-gnu");
    }

    #[test]
    fn test_zig_target_thumb() {
        let t = zig_target(&TargetArch::NoneEabi);
        assert_eq!(t.target_module, "arm");
        assert_eq!(t.cpu_arch, "thumb");
        assert_eq!(t.default_cpu, "cortex_m3");
        assert_eq!(t.os_tag, "freestanding");
        assert_eq!(t.abi, "eabi");
        assert_eq!(t.triple(), "thumb-freestanding-eabi");
    }

    #[test]
    fn test_zig_target_riscv32() {
        let t = zig_target(&TargetArch::RISCV32);
        assert_eq!(t.target_module, "riscv");
        assert_eq!(t.cpu_arch, "riscv32");
        assert_eq!(t.default_cpu, "generic_rv32");
        assert_eq!(t.triple(), "riscv32-freestanding-none");
    }

    #[test]
    fn test_zig_target_riscv64() {
        let t = zig_target(&TargetArch::RISCV64);
        assert_eq!(t.target_module, "riscv");
        assert_eq!(t.cpu_arch, "riscv64");
        assert_eq!(t.default_cpu, "generic_rv64");
        assert_eq!(t.triple(), "riscv64-freestanding-none");
    }

    #[test]
    fn test_zig_target_xtensa_generic_only() {
        let t = zig_target(&TargetArch::Xtensa);
        assert_eq!(t.target_module, "xtensa");
        assert_eq!(t.default_cpu, "generic");
        assert_eq!(t.triple(), "xtensa-freestanding-none");
    }

    #[test]
    fn test_zig_target_wasm() {
        let t = zig_target(&TargetArch::WASM);
        assert_eq!(t.target_module, "wasm");
        assert_eq!(t.cpu_arch, "wasm32");
        assert_eq!(t.triple(), "wasm32-freestanding-none");
    }

    #[test]
    fn test_cross_detection() {
        assert!(!zig_target(&TargetArch::X86_64).os_tag.contains("freestanding"));
        assert!(zig_target(&TargetArch::Xtensa).os_tag.contains("freestanding"));
    }

    // ── helpers ───────────────────────────────────────────────────────

    #[test]
    fn test_split_define() {
        assert_eq!(ZigGenerator::split_define("FOO=bar"), ("FOO", "bar"));
        assert_eq!(ZigGenerator::split_define("DEBUG"), ("DEBUG", ""));
        assert_eq!(ZigGenerator::split_define("A=B=C"), ("A", "B=C"));
    }

    #[test]
    fn test_needs_libc_only_for_linux() {
        assert!(ZigGenerator::needs_libc(&TargetArch::X86_64));
        assert!(!ZigGenerator::needs_libc(&TargetArch::NoneEabi));
    }

    #[test]
    fn test_resolve_cpu_normalises_hyphens() {
        use crate::models::project::ToolchainConfig;
        let config = ProjectConfig {
            toolchain: Some(ToolchainConfig {
                cpu: "cortex-m4".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        let gen = ZigGenerator::new(&config).unwrap();
        let target = zig_target(&TargetArch::NoneEabi);
        assert_eq!(gen.resolve_cpu_model(&target), "cortex_m4");
    }

    #[test]
    fn test_resolve_cpu_falls_back_to_default() {
        let config = ProjectConfig::default();
        let gen = ZigGenerator::new(&config).unwrap();
        let target = zig_target(&TargetArch::NoneEabi);
        assert_eq!(gen.resolve_cpu_model(&target), "cortex_m3");
    }

    #[test]
    fn test_sanitize_name_empty_is_root() {
        assert_eq!(sanitize_name(""), "root");
        assert_eq!(sanitize_name("src"), "src");
        assert_eq!(sanitize_name("core-src"), "core_src");
    }

    #[test]
    fn test_asm_needs_cpp() {
        assert!(ZigGenerator::asm_needs_cpp("startup.S"));
        assert!(ZigGenerator::asm_needs_cpp("path/to/vector.S"));
        assert!(!ZigGenerator::asm_needs_cpp("startup.s"));
        assert!(!ZigGenerator::asm_needs_cpp("bootstrap.s"));
    }

    #[test]
    fn test_build_c_compile_flags() {
        let config = ProjectConfig {
            language: "C".into(),
            c_standard: "11".into(),
            ..Default::default()
        };
        let gen = ZigGenerator::new(&config).unwrap();
        let flags = gen.c_compile_flags();
        assert!(flags.contains(&"-std=c11".to_string()));
    }

    #[test]
    fn test_build_cxx_compile_flags() {
        let config = ProjectConfig {
            language: "CXX".into(),
            cpp_standard: "17".into(),
            ..Default::default()
        };
        let gen = ZigGenerator::new(&config).unwrap();
        let flags = gen.cxx_compile_flags();
        assert!(flags.contains(&"-std=c++17".to_string()));
    }

    #[test]
    fn test_asm_flags_no_std() {
        let config = ProjectConfig {
            language: "CXX".into(),
            cpp_standard: "17".into(),
            ..Default::default()
        };
        let gen = ZigGenerator::new(&config).unwrap();
        let flags = gen.build_asm_flags();
        // asm_flags must NOT contain language-standard flags.
        assert!(!flags.iter().any(|f| f.starts_with("-std=")),
            "asm_flags should not contain -std= flags, got: {:?}", flags);
    }

    // ── integration ──────────────────────────────────────────────────

    #[test]
    fn test_generate_does_not_panic() {
        let tmp = tempfile::tempdir().unwrap();
        let config = ProjectConfig {
            name: "test_zig".into(),
            root: tmp.path().to_path_buf(),
            ..Default::default()
        };
        let gen = ZigGenerator::new(&config).unwrap();
        let result = gen.generate(&[], &DependencyGraph::new(), true, &[]);
        assert!(result.is_ok());
        assert!(tmp.path().join("build.zig").exists());
    }

    #[test]
    fn test_generate_with_linker_script() {
        let tmp = tempfile::tempdir().unwrap();
        let config = ProjectConfig {
            name: "test_zig".into(),
            root: tmp.path().to_path_buf(),
            language: "C".into(),
            ..Default::default()
        };
        std::fs::create_dir_all(tmp.path().join("src")).unwrap();
        std::fs::write(tmp.path().join("src").join("main.c"), "int main(void) { return 0; }\n").unwrap();
        std::fs::write(tmp.path().join("src").join("app.ld"), "SECTIONS { }").unwrap();

        let gen = ZigGenerator::new(&config).unwrap();
        let module = CMakeModule {
            name: "src".into(),
            path: tmp.path().join("src"),
            relative_path: PathBuf::from("src"),
            sources: vec![crate::models::module::SourceFile {
                path: tmp.path().join("src").join("main.c"),
                relative_path: PathBuf::from("src/main.c"),
                file_name: "main.c".into(),
                source_type: SourceType::CSource,
                includes: vec![],
            }],
            has_main: true,
            is_root: true,
            linker_scripts: vec![PathBuf::from("src/app.ld")],
            headers: vec![],
            asm_sources: vec![],
            dependencies: vec![],
            target_type: TargetType::Executable,
            compile_features: vec![],
            compile_definitions: vec![],
            include_dirs: vec![],
        };
        let modules = vec![module];
        let mut graph = DependencyGraph::new();
        graph.add_module("src");
        let result = gen.generate(&modules, &graph, true, &[]);
        assert!(result.is_ok());

        let content = std::fs::read_to_string(tmp.path().join("build.zig")).unwrap();
        assert!(content.contains("setLinkerScript"), "linker script not emitted: {}", content);
        let exe_pos = content.find("addExecutable").expect("addExecutable not found");
        let ls_pos = content.find("setLinkerScript").expect("setLinkerScript not found");
        assert!(ls_pos > exe_pos, "setLinkerScript must come after addExecutable, but exe_pos={exe_pos} ls_pos={ls_pos}");
    }

    #[test]
    fn test_generate_with_asm_splits_lower_s_vs_upper_s() {
        let tmp = tempfile::tempdir().unwrap();
        let config = ProjectConfig {
            name: "test_zig".into(),
            root: tmp.path().to_path_buf(),
            language: "C".into(),
            ..Default::default()
        };
        std::fs::create_dir_all(tmp.path().join("src")).unwrap();
        std::fs::write(tmp.path().join("src").join("main.c"), "int main(void) { return 0; }\n").unwrap();

        let gen = ZigGenerator::new(&config).unwrap();
        let module = CMakeModule {
            name: "src".into(),
            path: tmp.path().join("src"),
            relative_path: PathBuf::from("src"),
            sources: vec![crate::models::module::SourceFile {
                path: tmp.path().join("src").join("main.c"),
                relative_path: PathBuf::from("src/main.c"),
                file_name: "main.c".into(),
                source_type: SourceType::CSource,
                includes: vec![],
            }],
            asm_sources: vec![
                crate::models::module::SourceFile {
                path: tmp.path().join("src").join("startup.S"),
                relative_path: PathBuf::from("src/startup.S"),
                file_name: "startup.S".into(),
                source_type: SourceType::AsmSource,
                includes: vec![],
            },
                crate::models::module::SourceFile {
                path: tmp.path().join("src").join("bootstrap.s"),
                relative_path: PathBuf::from("src/bootstrap.s"),
                file_name: "bootstrap.s".into(),
                source_type: SourceType::AsmSource,
                includes: vec![],
            },
            ],
            has_main: true,
            is_root: true,
            headers: vec![],
            linker_scripts: vec![],
            dependencies: vec![],
            target_type: TargetType::Executable,
            compile_features: vec![],
            compile_definitions: vec![],
            include_dirs: vec![],
        };
        let modules = vec![module];
        let mut graph = DependencyGraph::new();
        graph.add_module("src");
        let result = gen.generate(&modules, &graph, true, &[]);
        assert!(result.is_ok());

        let content = std::fs::read_to_string(tmp.path().join("build.zig")).unwrap();
        // .S → assembler-with-cpp
        assert!(content.contains("startup.S") && content.contains("assembler-with-cpp"),
            ".S file should get assembler-with-cpp:\n{}", content);
        // .s → assembler (no -cpp)
        assert!(content.contains("bootstrap.s") && content.contains("\"assembler\""),
            ".s file should get assembler (no -cpp):\n{}", content);
        // asm must NOT get -std= flags.
        let asm_s_line = content.find("bootstrap.s").unwrap();
        let after_asm = &content[asm_s_line..];
        assert!(!after_asm[..after_asm.find('\n').unwrap_or(after_asm.len())].contains("-std="),
            "asm should not have -std= flags");
    }

    #[test]
    fn test_no_duplicate_include_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let config = ProjectConfig {
            name: "test_zig".into(),
            root: tmp.path().to_path_buf(),
            language: "C".into(),
            ..Default::default()
        };
        std::fs::create_dir_all(tmp.path().join("src")).unwrap();
        std::fs::write(tmp.path().join("src").join("main.c"), "int main(void) { return 0; }\n").unwrap();

        let gen = ZigGenerator::new(&config).unwrap();
        let module = CMakeModule {
            name: "src".into(),
            path: tmp.path().join("src"),
            relative_path: PathBuf::from("src"),
            sources: vec![crate::models::module::SourceFile {
                path: tmp.path().join("src").join("main.c"),
                relative_path: PathBuf::from("src/main.c"),
                file_name: "main.c".into(),
                source_type: SourceType::CSource,
                includes: vec![],
            }],
            has_main: true,
            is_root: false,
            headers: vec![],
            asm_sources: vec![],
            linker_scripts: vec![],
            dependencies: vec![],
            target_type: TargetType::Executable,
            compile_features: vec![],
            compile_definitions: vec![],
            include_dirs: vec![PathBuf::from("src")],
        };
        let modules = vec![module];
        let mut graph = DependencyGraph::new();
        graph.add_module("src");
        let result = gen.generate(&modules, &graph, true, &[]);
        assert!(result.is_ok());

        let content = std::fs::read_to_string(tmp.path().join("build.zig")).unwrap();
        // "src" should appear exactly once as an addIncludePath argument.
        let n = content.matches("addIncludePath(b.path(\"src\"))").count();
        assert_eq!(n, 1, "include path 'src' should appear exactly once, got {}:\n{}", n, content);
    }

    #[test]
    fn test_build_system_default_is_cmake() {
        assert_eq!(BuildSystem::default(), BuildSystem::CMake);
    }
}
