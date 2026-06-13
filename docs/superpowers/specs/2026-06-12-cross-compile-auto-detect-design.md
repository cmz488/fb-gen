# Cross-Compilation Toolchain Auto-Detection & Sysroot Support

**Date**: 2026-06-12
**Status**: Draft

## Motivation

fb-gen currently hardcodes toolchain prefixes (e.g. `arm-none-eabi-`) in the
generated `cmake/toolchain.cmake` and omits `CMAKE_SYSROOT` /
`CMAKE_FIND_ROOT_PATH`. Users with cross-compilers installed at non-standard
locations must either rely on PATH or manually maintain a toolchain file,
defeating fb-gen's zero-config goal. Additionally, toolchain generation is
restricted to `NoneEabi` targets — `ARM32`, `ARM64`, and `RISCV64` get no
auto-generated toolchain file.

## Design

### 1. Model updates (`src/models/project.rs`)

Add three fields to `ToolchainConfig`:

| Field | Type | Purpose |
|---|---|---|
| `prefix` | `String` | Toolchain prefix, e.g. `"arm-none-eabi-"`. Required for all cross targets. |
| `sysroot` | `Option<String>` | Sysroot path returned by `gcc -print-sysroot`. `None` → omit `CMAKE_SYSROOT`. |
| `find_root_path` | `Vec<String>` | Extra `CMAKE_FIND_ROOT_PATH` entries beyond the sysroot itself. |

None of these fields are breaking — `prefix` defaults to the current hardcoded
value for `NoneEabi`, and `Default::default()` fills in sensible defaults.

### 2. Auto-detection module (`src/core/toolchain_detect.rs` — new)

A single public entry point:

```rust
pub fn detect_toolchains() -> Vec<DetectedToolchain>

pub struct DetectedToolchain {
    pub prefix: String,            // "arm-none-eabi-"
    pub cc_path: PathBuf,          // "/usr/bin/arm-none-eabi-gcc"
    pub sysroot: Option<PathBuf>,  // gcc -print-sysroot output, may be empty
    pub target_triplet: String,    // gcc -dumpmachine output
    pub suggested_arch: TargetArch, // inferred from triplet
}
```

**Scan algorithm**:

1. Walk each directory in `$PATH` (using `std::env::split_paths`).
2. For each `*-gcc` file, check whether the prefix matches a known cross
   pattern (`arm-none-eabi-`, `aarch64-none-elf-`, `riscv64-unknown-elf-`,
   `arm-linux-gnueabihf-`, etc.).
3. Verify the toolchain is complete: `gcc`, `g++`, `objcopy` must all exist.
4. Run `<prefix>gcc -print-sysroot` → sysroot (may be empty string).
5. Run `<prefix>gcc -dumpmachine` → target triplet.
6. Map triplet to `TargetArch` via a lookup table. Unrecognised triplets
   map to `TargetArch::Custom(triplet)`.
7. Return the list, deduplicated by prefix.

**Fallback**: When `detect_toolchains()` returns an empty vec, fb-gen falls
back to the current manual-config flow.

### 3. UserQuery changes (`src/orchestration/query.rs`)

In `ask_project_config()`, after the user selects a target architecture:

1. Call `detect_toolchains()`.
2. Filter to toolchains compatible with the chosen architecture.
3. Display a numbered list:
   ```
   Detected ARM bare-metal toolchains:
     1) arm-none-eabi-  → /usr/bin/arm-none-eabi-gcc  (sysroot: none)
     2) arm-none-eabi-  → /opt/gcc-arm/bin/arm-none-eabi-gcc  (sysroot: /opt/gcc-arm/arm-none-eabi)
     3) Custom — enter prefix and sysroot manually
   Choose toolchain [1-3]:
   ```
4. The user picks one. If they pick "Custom", prompt for prefix, sysroot,
   and find_root_path manually.
5. Store the selected prefix/sysroot/find_root_path in `ToolchainConfig`.
6. Continue to the existing MCU/FPU/abi prompts.

### 4. Generator changes (`src/core/generator.rs`)

**Template additions** — the generated `cmake/toolchain.cmake` gains two
conditional blocks:

```cmake
# ── Sysroot (auto-detected) ──────────────────────────────────
{% if sysroot -%}
set(CMAKE_SYSROOT {{ sysroot }})
set(CMAKE_FIND_ROOT_PATH ${CMAKE_SYSROOT} {% for p in find_root_path %}{{ p }} {% endfor %})
{% endif -%}

# ── Cross-compilation root paths ──────────────────────────────
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_PACKAGE ONLY)
```

When `sysroot` is `None`, the entire sysroot block is omitted. The
`CMAKE_FIND_ROOT_PATH_MODE_*` lines remain unconditional.

**Architecture coverage** — toolchain generation is extended from `NoneEabi`-only:

| TargetArch | Prefix pattern | Generates toolchain? |
|---|---|---|
| NoneEabi | `arm-none-eabi-` | Yes |
| ARM32 | `arm-none-eabi-` or `arm-linux-*` | Yes, when ToolchainConfig is Some |
| ARM64 | `aarch64-none-elf-` | Yes, when ToolchainConfig is Some |
| RISCV64 | `riscv64-unknown-elf-` | Yes, when ToolchainConfig is Some |

`X86_64`, `X86`, `WASM`, `Custom` remain no-toolchain.

**`render_embedded_toolchain()`** is refactored to accept the new fields and
switch from `format!()` to a Tera template for readability and consistency
with the rest of the codebase.

### 5. `.s` files in executable

Already addressed in a prior change — root-level `.s`/`.S` files are merged
into the executable module and listed via `target_sources()`. No further
changes needed.

## Non-goals

- Detecting libraries installed in the sysroot (pkg-config, etc.).
- Building a "SDK manager" that downloads toolchains.
- Windows host detection (MSVC cross-compilation, WSL paths).

## Testing

- **Unit**: `detect_toolchains()` with a mocked PATH containing fake `*-gcc`
  scripts that output known sysroot/triplet strings.
- **Unit**: `render_embedded_toolchain()` output includes sysroot block when
  `sysroot` is `Some`, omits it when `None`.
- **Integration**: `fb-gen init` with `--target NoneEabi` in a temp dir where
  a fake `arm-none-eabi-gcc` is on PATH.
- **Existing tests**: `test_cross_compile_template` and
  `test_toolchain_none_eabi_missing_cpu` must continue to pass.

## Migration

Existing `project.json` caches without the new `ToolchainConfig` fields
deserialise safely: `prefix` defaults to `""`, `sysroot` to `None`,
`find_root_path` to `vec![]`. Users can re-run `fb-gen init` to pick up the
auto-detected values. The `MetaCache` save/load round-trip already handles
`ToolchainConfig` via serde.
