# 工具链配置重构 — 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将扁平的 `mcu_flags: String` 替换为结构化的 `ToolchainConfig`，正确地将 ARM32/ARM64 与 NoneEabi 分离，并为核心嵌入式目标添加 `USER_START`/`USER_END` 保留及 `CMAKE_FIND_ROOT_PATH_MODE`。

**Architecture:** `ProjectConfig` 获得 `toolchain: Option<ToolchainConfig>`（若架构不需要工具链文件则为 `None`）。仅 NoneEabi 和 RISCV64 生成 `toolchain.cmake`。`render_embedded_toolchain` 中的模板接收结构化字段并输出 `-mcpu=`、`-mfloat-abi=`、`-mfpu=`、用户标志、链接器脚本及 `CMAKE_FIND_ROOT_PATH_MODE`。

**Tech Stack:** Rust (edition 2021), serde, tera, tempfile

---

## File Structure

| 文件 | 角色 |
|---|---|
| `src/models/project.rs` | 新增 `ToolchainConfig` 结构体；`mcu_flags: String` → `toolchain: Option<ToolchainConfig>` |
| `src/core/generator.rs` | 重写 `render_toolchain()` 以接收 `&ToolchainConfig`；更新 `render_embedded_toolchain()`；新增 `CMAKE_FIND_ROOT_PATH_MODE` 和 `USER_START`/`USER_END` |
| `src/orchestration/query.rs` | MCU 提示仅限 NoneEabi；将响应构建成 `ToolchainConfig`；在 `confirm_config` 中显示工具链设置 |
| `src/orchestration/cache.rs` | 更新 `load_project_config` 和 `make_meta()` 测试辅助函数 |
| `src/orchestration/workflow.rs` | 更新 `make_config()` 测试辅助函数 |
| `tests/integration.rs` | 更新现有工具链测试；新增 ARM32/ARM64 无工具链测试、缺失 CPU 测试、用户块保留测试 |

---

### Task 1: 新增 `ToolchainConfig` 模型，替换 `mcu_flags`

**Files:**
- Modify: `src/models/project.rs`

- [ ] **Step 1: 在 `ProjectConfig` 之前新增 `ToolchainConfig` 结构体**

在 `src/models/project.rs` 的 `BuildBackend` 之后、`ProjectConfig` 之前插入：

```rust
/// Structured toolchain configuration for embedded cross-compilation targets.
/// `None` when the target architecture does not need a toolchain file.
#[derive(Debug, Clone, Serialize, Deserialize)]
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

    /// Linker script path relative to the project root.
    /// Generates `-T <path>` linker flag.
    pub linker_script: String,
}

impl Default for ToolchainConfig {
    fn default() -> Self {
        Self {
            cpu: String::new(),
            float_abi: String::new(),
            fpu: String::new(),
            extra_flags: String::new(),
            linker_script: String::new(),
        }
    }
}
```

- [ ] **Step 2: 在 `ProjectConfig` 中将 `mcu_flags` 替换为 `toolchain`**

在 `ProjectConfig` 结构体中，将：

```rust
    /// MCU/CPU flags for cross-compilation (e.g. "cortex-m3")
    pub mcu_flags: String,
```

替换为：

```rust
    /// Toolchain configuration for cross-compilation targets.
    /// `None` when the target architecture does not require a toolchain file.
    pub toolchain: Option<ToolchainConfig>,
```

- [ ] **Step 3: 更新 `impl Default for ProjectConfig`**

在 `Default` 实现中，将 `mcu_flags: String::new()` 替换为 `toolchain: None`。

- [ ] **Step 4: 编译以捕获所有受影响的站点**

```bash
cargo build 2>&1
```

预期：由于 `mcu_flags` → `toolchain` 字段类型变更，多个文件出现编译错误。后续任务将修复这些错误。

- [ ] **Step 5: Commit**

```bash
git add src/models/project.rs
git commit -m "refactor: add ToolchainConfig model, replace mcu_flags with toolchain: Option<ToolchainConfig>

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 2: 重写 `generator.rs` 工具链渲染

**Files:**
- Modify: `src/core/generator.rs`

- [ ] **Step 1: 重写 `render_embedded_toolchain()` 以接收 `&ToolchainConfig`**

将整个 `render_embedded_toolchain` 函数（第 591-656 行）替换为：

```rust
/// Shared template for embedded cross-compilation toolchain files.
///
/// Produces a complete CMake toolchain file with compiler flags, linker options,
/// target-specific settings, and a user-customisation block.
fn render_embedded_toolchain(
    system_name: &str,
    processor: &str,
    prefix: &str,
    tc: &ToolchainConfig,
) -> String {
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

    // Linker-script flag.
    let ld_flag = if !tc.linker_script.is_empty() {
        format!(" -T ${{CMAKE_SOURCE_DIR}}/{}", tc.linker_script)
    } else {
        String::new()
    };

    format!(
        r#"# Auto-generated toolchain file by fb-gen

set(CMAKE_SYSTEM_NAME {system_name})
set(CMAKE_SYSTEM_PROCESSOR {processor})

set(CMAKE_C_COMPILER_ID GNU)
set(CMAKE_CXX_COMPILER_ID GNU)

set(TOOLCHAIN_PREFIX {prefix})

set(CMAKE_C_COMPILER ${{TOOLCHAIN_PREFIX}}gcc)
set(CMAKE_ASM_COMPILER ${{CMAKE_C_COMPILER}})
set(CMAKE_CXX_COMPILER ${{TOOLCHAIN_PREFIX}}g++)
set(CMAKE_LINKER ${{TOOLCHAIN_PREFIX}}g++)
set(CMAKE_OBJCOPY ${{TOOLCHAIN_PREFIX}}objcopy)
set(CMAKE_SIZE ${{TOOLCHAIN_PREFIX}}size)

set(CMAKE_EXECUTABLE_SUFFIX_ASM ".elf")
set(CMAKE_EXECUTABLE_SUFFIX_C ".elf")
set(CMAKE_EXECUTABLE_SUFFIX_CXX ".elf")

set(CMAKE_TRY_COMPILE_TARGET_TYPE STATIC_LIBRARY)

# ── Cross-compilation root paths ─────────────────────────────────
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_PACKAGE ONLY)

# MCU specific flags
set(TARGET_FLAGS "{target_flags} ")

set(CMAKE_C_FLAGS "${{CMAKE_C_FLAGS}} ${{TARGET_FLAGS}}")
set(CMAKE_ASM_FLAGS "${{CMAKE_ASM_FLAGS}} ${{TARGET_FLAGS}} -x assembler-with-cpp -MMD -MP")
set(CMAKE_C_FLAGS "${{CMAKE_C_FLAGS}} -Wall -fdata-sections -ffunction-sections -fstack-usage")

set(CMAKE_C_FLAGS_DEBUG "-O0 -g3")
set(CMAKE_C_FLAGS_RELEASE "-Os -g0")
set(CMAKE_CXX_FLAGS_DEBUG "-O0 -g3")
set(CMAKE_CXX_FLAGS_RELEASE "-Os -g0")

set(CMAKE_CXX_FLAGS "${{CMAKE_C_FLAGS}} -fno-rtti -fno-exceptions -fno-threadsafe-statics")

set(CMAKE_EXE_LINKER_FLAGS "${{TARGET_FLAGS}}")
set(CMAKE_EXE_LINKER_FLAGS "${{CMAKE_EXE_LINKER_FLAGS}} --specs=nano.specs")
set(CMAKE_EXE_LINKER_FLAGS "${{CMAKE_EXE_LINKER_FLAGS}} -Wl,-Map=${{CMAKE_PROJECT_NAME}}.map -Wl,--gc-sections")
set(CMAKE_EXE_LINKER_FLAGS "${{CMAKE_EXE_LINKER_FLAGS}} -Wl,--print-memory-usage")
set(CMAKE_EXE_LINKER_FLAGS "${{CMAKE_EXE_LINKER_FLAGS}}{ld_flag}")
set(TOOLCHAIN_LINK_LIBRARIES "m")

# ── User customisations ──────────────────────────────────────────
# USER_START
# USER_END
"#,
        system_name = system_name,
        processor = processor,
        prefix = prefix,
        target_flags = target_flags,
        ld_flag = ld_flag,
    )
}
```

- [ ] **Step 2: 更新调用点以传递 `&ToolchainConfig`**

将 `render_arm_eabi_toolchain`（第 576 行）从：

```rust
fn render_arm_eabi_toolchain(mcu_flags: &str) -> String {
    let prefix = "arm-none-eabi-";
    render_embedded_toolchain("Generic", "arm", prefix, mcu_flags)
}
```

改为：

```rust
fn render_arm_eabi_toolchain(tc: &ToolchainConfig) -> String {
    let prefix = "arm-none-eabi-";
    render_embedded_toolchain("Generic", "arm", prefix, tc)
}
```

类似地更新 `render_aarch64_toolchain` 和 `render_riscv64_toolchain`：

```rust
fn render_aarch64_toolchain(tc: &ToolchainConfig) -> String {
    let prefix = "aarch64-none-elf-";
    render_embedded_toolchain("Generic", "aarch64", prefix, tc)
}

fn render_riscv64_toolchain(tc: &ToolchainConfig) -> String {
    let prefix = "riscv64-unknown-elf-";
    render_embedded_toolchain("Generic", "riscv64", prefix, tc)
}
```

- [ ] **Step 3: 重写 `render_toolchain()` 以使用 `Option<ToolchainConfig>`**

将 `render_toolchain()` 方法（第 441-469 行）替换为：

```rust
    /// Render the toolchain.cmake content for cross-compilation targets.
    ///
    /// Returns `Ok(None)` if the target architecture does not require a toolchain
    /// (ARM32, ARM64, X86_64, X86, WASM, Custom) or no ToolchainConfig is set.
    ///
    /// Returns `Err` if NoneEabi has no ToolchainConfig or the CPU field is empty.
    fn render_toolchain(&self) -> Result<Option<String>> {
        let tc = match &self.config.toolchain {
            Some(tc) => tc,
            None => {
                // No toolchain config — only valid for non-embedded targets.
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

        match &self.config.target_arch {
            TargetArch::NoneEabi => {
                Ok(Some(render_arm_eabi_toolchain(tc)))
            }
            TargetArch::RISCV64 => {
                Ok(Some(render_riscv64_toolchain(tc)))
            }
            _ => Ok(None),
        }
    }
```

- [ ] **Step 4: 编译验证**

```bash
cargo build 2>&1
```

预期：编译成功（此前已修复所有引用处）。若 `cross_compile_context` 函数因 `mcu_flags` 删除产生未使用变量警告，根据该函数仍被 `#[allow(dead_code)]` 标记，警告应该被抑制。

- [ ] **Step 5: Commit**

```bash
git add src/core/generator.rs
git commit -m "refactor: rewrite toolchain renderer with ToolchainConfig, CMAKE_FIND_ROOT_PATH_MODE, USER blocks

- render_embedded_toolchain takes &ToolchainConfig with structured flags
- Add CMAKE_FIND_ROOT_PATH_MODE (PROGRAM/LIBRARY/INCLUDE/PACKAGE)
- Add # USER_START / # USER_END markers to toolchain template
- Only NoneEabi and RISCV64 generate toolchain files
- NoneEabi without CPU returns FbGenError::Config

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 3: 更新 `query.rs` — 仅 NoneEabi 的交互式工具链提示

**Files:**
- Modify: `src/orchestration/query.rs`

- [ ] **Step 1: 将 MCU 提示替换为针对 NoneEabi 的 ToolchainConfig 构建器**

将第 112-132 行（"MCU/CPU flags" 部分）替换为：

```rust
        // ── Toolchain config (NoneEabi bare-metal targets only) ────
        let toolchain = if matches!(target_arch, TargetArch::NoneEabi) {
            println!();
            println!("  ARM MCU/CPU selection:");
            println!("    Specify the target chip model for -mcpu= flag.");
            let cpu = prompt_with_default(
                "  ARM MCU/CPU [cortex-m3]",
                "cortex-m3",
            )
            .map_err(|e| {
                crate::models::FbGenError::Config(format!("failed to read MCU: {e}"))
            })?;

            let float_abi = prompt_with_default(
                "  Float ABI (soft/softfp/hard, empty to skip) []",
                "",
            )
            .map_err(|e| {
                crate::models::FbGenError::Config(format!("failed to read float ABI: {e}"))
            })?;

            let fpu = prompt_with_default(
                "  FPU (e.g. fpv4-sp-d16, empty to skip) []",
                "",
            )
            .map_err(|e| {
                crate::models::FbGenError::Config(format!("failed to read FPU: {e}"))
            })?;

            let extra_flags = prompt_with_default(
                "  Extra flags (e.g. -mthumb, empty to skip) []",
                "",
            )
            .map_err(|e| {
                crate::models::FbGenError::Config(format!("failed to read extra flags: {e}"))
            })?;

            let linker_script = prompt_with_default(
                "  Linker script path (empty to skip) []",
                "",
            )
            .map_err(|e| {
                crate::models::FbGenError::Config(format!("failed to read linker script: {e}"))
            })?;

            Some(crate::models::project::ToolchainConfig {
                cpu,
                float_abi,
                fpu,
                extra_flags,
                linker_script,
            })
        } else {
            None
        };
```

- [ ] **Step 2: 在 `ProjectConfig` 构建中使用 `toolchain`**

在 `ProjectConfig` 结构体初始化中（约第 209 行），将 `mcu_flags,` 替换为 `toolchain,`。

- [ ] **Step 3: 在 `confirm_config` 中添加工具链显示**

在 `confirm_config` 函数中，在 `println!("  Output dir:        {}", config.output_dir.display());` 之后添加：

```rust
        if let Some(ref tc) = config.toolchain {
            println!("  ── Toolchain ────────────────────────────────────────");
            println!("    CPU:              {}", tc.cpu);
            if !tc.float_abi.is_empty() {
                println!("    Float ABI:        {}", tc.float_abi);
            }
            if !tc.fpu.is_empty() {
                println!("    FPU:              {}", tc.fpu);
            }
            if !tc.extra_flags.is_empty() {
                println!("    Extra flags:      {}", tc.extra_flags);
            }
            if !tc.linker_script.is_empty() {
                println!("    Linker script:    {}", tc.linker_script);
            }
        }
```

- [ ] **Step 4: 编译验证**

```bash
cargo build 2>&1
```

预期：编译成功。

- [ ] **Step 5: Commit**

```bash
git add src/orchestration/query.rs
git commit -m "feat: interactive toolchain config prompt for NoneEabi only

- Replace single mcu_flags prompt with structured ToolchainConfig fields
- Only shown when target_arch == NoneEabi
- ARM32, ARM64, RISCV64, X86* get toolchain: None
- confirm_config displays toolchain settings when present

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 4: 修复 `cache.rs` 和 `workflow.rs` 测试辅助函数

**Files:**
- Modify: `src/orchestration/cache.rs`
- Modify: `src/orchestration/workflow.rs`

- [ ] **Step 1: 更新 `cache.rs` 中的 `load_project_config`**

在 `src/orchestration/cache.rs` 第 218 行，将 `mcu_flags: String::new()` 替换为 `toolchain: None`。

- [ ] **Step 2: 更新 `cache.rs` 中的 `make_meta()` 测试辅助函数**

在第 316 行，将 `mcu_flags: String::new()` 替换为 `toolchain: None`。

- [ ] **Step 3: 更新 `workflow.rs` 中的 `make_config()` 测试辅助函数**

在第 288 行，将 `mcu_flags: String::new()` 替换为 `toolchain: None`。

- [ ] **Step 4: 编译并运行库测试**

```bash
cargo test --lib 2>&1
```

预期：所有 44 个库测试通过。

- [ ] **Step 5: Commit**

```bash
git add src/orchestration/cache.rs src/orchestration/workflow.rs
git commit -m "fix: replace mcu_flags with toolchain: None in cache and workflow test helpers

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 5: 编写集成测试 — ARM32/ARM64 无工具链 + 缺失 CPU

**Files:**
- Modify: `tests/integration.rs`

- [ ] **Step 1: 更新 `test_cross_compile_template` 以使用 `ToolchainConfig`**

将测试中的 config 构建从 `mcu_flags: "cortex-m3".into()` 改为：

```rust
    let config = ProjectConfig {
        name: "CrossTest".into(),
        version: "0.1.0".into(),
        root: root.clone(),
        language: "C".into(),
        c_standard: "11".into(),
        cpp_standard: "17".into(),
        target_arch: fb_gen::models::project::TargetArch::NoneEabi,
        toolchain: Some(fb_gen::models::project::ToolchainConfig {
            cpu: "cortex-m3".into(),
            ..Default::default()
        }),
        ..Default::default()
    };
```

添加对 `CMAKE_FIND_ROOT_PATH_MODE` 和 `USER_START`/`USER_END` 标记的断言：

```rust
    // ── FIND_ROOT_PATH_MODE ───────────────────────────────────────────
    assert!(
        toolchain_content.contains("CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER"),
        "toolchain.cmake should set CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER"
    );

    // ── User block markers ────────────────────────────────────────────
    assert!(
        toolchain_content.contains("# USER_START"),
        "toolchain.cmake should contain USER_START marker"
    );
    assert!(
        toolchain_content.contains("# USER_END"),
        "toolchain.cmake should contain USER_END marker"
    );
```

- [ ] **Step 2: 删除 `test_toolchain_arm64` — ARM64 不再生成工具链**

删除整个 `test_toolchain_arm64` 函数（第 590-634 行）。

- [ ] **Step 3: 更新 `test_toolchain_riscv64` 以使用 `ToolchainConfig`**

将 config 构建改为：

```rust
    let config = ProjectConfig {
        name: "RISCVTest".into(),
        version: "0.1.0".into(),
        root: root.clone(),
        target_arch: fb_gen::models::project::TargetArch::RISCV64,
        toolchain: Some(fb_gen::models::project::ToolchainConfig::default()),
        ..Default::default()
    };
```

- [ ] **Step 4: 在文件末尾新增 `test_toolchain_arm32_no_toolchain`**

```rust
#[test]
fn test_toolchain_arm32_no_toolchain() {
    // ARM32 should NOT generate a toolchain file (Linux userspace target).
    let tmp = TempDir::new().unwrap();
    create_test_project(&tmp);

    let root = tmp.path().to_path_buf();
    let sources = scan_project(&root);

    let discoverer = ModuleDiscoverer::new(ScanOptions {
        exclude_dirs: vec![],
        root: root.clone(),
    });
    let modules = discoverer.discover(&sources).unwrap();

    let config = ProjectConfig {
        name: "ARM32Test".into(),
        version: "0.1.0".into(),
        root: root.clone(),
        target_arch: fb_gen::models::project::TargetArch::ARM32,
        ..Default::default()
    };

    let generator = CMakeGenerator::new(&config).unwrap();
    let empty_graph = DependencyGraph::new();
    generator.generate(&modules, &empty_graph, true).unwrap();

    let toolchain_path = root.join("cmake").join("toolchain.cmake");
    assert!(
        !toolchain_path.exists(),
        "toolchain.cmake should NOT be generated for ARM32 target"
    );
}
```

- [ ] **Step 5: 新增 `test_toolchain_arm64_no_toolchain`**

```rust
#[test]
fn test_toolchain_arm64_no_toolchain() {
    // ARM64 should NOT generate a toolchain file.
    let tmp = TempDir::new().unwrap();
    create_test_project(&tmp);

    let root = tmp.path().to_path_buf();
    let sources = scan_project(&root);

    let discoverer = ModuleDiscoverer::new(ScanOptions {
        exclude_dirs: vec![],
        root: root.clone(),
    });
    let modules = discoverer.discover(&sources).unwrap();

    let config = ProjectConfig {
        name: "ARM64Test".into(),
        version: "0.1.0".into(),
        root: root.clone(),
        target_arch: fb_gen::models::project::TargetArch::ARM64,
        ..Default::default()
    };

    let generator = CMakeGenerator::new(&config).unwrap();
    let empty_graph = DependencyGraph::new();
    generator.generate(&modules, &empty_graph, true).unwrap();

    let toolchain_path = root.join("cmake").join("toolchain.cmake");
    assert!(
        !toolchain_path.exists(),
        "toolchain.cmake should NOT be generated for ARM64 target"
    );
}
```

- [ ] **Step 6: 新增 `test_toolchain_none_eabi_missing_cpu`**

```rust
#[test]
fn test_toolchain_none_eabi_missing_cpu() {
    // NoneEabi with empty CPU should return an error.
    let tmp = TempDir::new().unwrap();
    create_test_project(&tmp);

    let root = tmp.path().to_path_buf();
    let sources = scan_project(&root);

    let discoverer = ModuleDiscoverer::new(ScanOptions {
        exclude_dirs: vec![],
        root: root.clone(),
    });
    let modules = discoverer.discover(&sources).unwrap();

    let config = ProjectConfig {
        name: "NoCpuTest".into(),
        version: "0.1.0".into(),
        root: root.clone(),
        target_arch: fb_gen::models::project::TargetArch::NoneEabi,
        toolchain: Some(fb_gen::models::project::ToolchainConfig {
            cpu: String::new(), // missing!
            ..Default::default()
        }),
        ..Default::default()
    };

    let generator = CMakeGenerator::new(&config).unwrap();
    let empty_graph = DependencyGraph::new();
    let result = generator.generate(&modules, &empty_graph, true);

    assert!(
        result.is_err(),
        "NoneEabi with empty CPU should return an error, got: {:?}",
        result
    );
}
```

- [ ] **Step 7: 运行新增和更新的测试**

```bash
cargo test --test integration test_cross_compile_template
cargo test --test integration test_toolchain_riscv64
cargo test --test integration test_toolchain_arm32_no_toolchain
cargo test --test integration test_toolchain_arm64_no_toolchain
cargo test --test integration test_toolchain_none_eabi_missing_cpu
cargo test --test integration test_toolchain_not_generated_for_x86
```

预期：全部 7 个测试通过。

- [ ] **Step 8: Commit**

```bash
git add tests/integration.rs
git commit -m "test: update toolchain tests for ToolchainConfig, add ARM32/ARM64/missing-CPU tests

- test_cross_compile_template uses ToolchainConfig with cpu: cortex-m3
- test_toolchain_riscv64 uses ToolchainConfig::default()
- Remove test_toolchain_arm64 (ARM64 no longer generates toolchain)
- Add test_toolchain_arm32_no_toolchain
- Add test_toolchain_arm64_no_toolchain
- Add test_toolchain_none_eabi_missing_cpu
- Add assertions for CMAKE_FIND_ROOT_PATH_MODE and USER_START/USER_END

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 6: 编写工具链用户块保留测试

**Files:**
- Modify: `tests/integration.rs`

- [ ] **Step 1: 在文件末尾新增 `test_toolchain_user_block_preservation`**

```rust
#[test]
fn test_toolchain_user_block_preservation() {
    // Verify that user edits in # USER_START / # USER_END are preserved
    // when the toolchain file is regenerated.
    let tmp = TempDir::new().unwrap();
    create_test_project(&tmp);

    let root = tmp.path().to_path_buf();
    let sources = scan_project(&root);

    let discoverer = ModuleDiscoverer::new(ScanOptions {
        exclude_dirs: vec![],
        root: root.clone(),
    });
    let modules = discoverer.discover(&sources).unwrap();

    let config = ProjectConfig {
        name: "UserBlockTest".into(),
        version: "0.1.0".into(),
        root: root.clone(),
        target_arch: fb_gen::models::project::TargetArch::NoneEabi,
        toolchain: Some(fb_gen::models::project::ToolchainConfig {
            cpu: "cortex-m4".into(),
            ..Default::default()
        }),
        ..Default::default()
    };

    // ── First generation (force mode) ──────────────────────────────
    let generator = CMakeGenerator::new(&config).unwrap();
    let empty_graph = DependencyGraph::new();
    generator.generate(&modules, &empty_graph, true).unwrap();

    let toolchain_path = root.join("cmake").join("toolchain.cmake");
    assert!(toolchain_path.exists());

    // Manually edit the toolchain file to add user content.
    let original = std::fs::read_to_string(&toolchain_path).unwrap();
    let edited = original.replace(
        "# USER_START\n# USER_END",
        "# USER_START\nset(CMAKE_C_FLAGS \"${CMAKE_C_FLAGS} -DMY_DEFINE\")\n# USER_END",
    );
    std::fs::write(&toolchain_path, &edited).unwrap();

    // ── Regenerate (non-force / sync mode) ────────────────────────
    generator.generate(&modules, &empty_graph, false).unwrap();

    let regenerated = std::fs::read_to_string(&toolchain_path).unwrap();
    assert!(
        regenerated.contains("set(CMAKE_C_FLAGS \"${CMAKE_C_FLAGS} -DMY_DEFINE\")"),
        "user block should be preserved in toolchain.cmake, got:\n{}",
        regenerated
    );
}
```

- [ ] **Step 2: 运行测试**

```bash
cargo test --test integration test_toolchain_user_block_preservation
```

预期：PASS。

- [ ] **Step 3: Commit**

```bash
git add tests/integration.rs
git commit -m "test: add toolchain user block preservation test

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 7: 全量测试验证

**Files:** (无改动，仅验证)

- [ ] **Step 1: 运行全部库测试**

```bash
cargo test --lib
```

预期：所有库测试通过（无警告）。

- [ ] **Step 2: 运行全部集成测试**

```bash
cargo test --test integration
```

预期：所有测试通过。测试总数：旧方案 15 个 - 1 个已删除的 ARM64 测试 + 4 个新增 = **18 个测试**。

具体清单：
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
- `test_toolchain_riscv64` PASS
- `test_toolchain_not_generated_for_x86` PASS
- `test_bare_include_dependency` PASS
- `test_bare_include_no_self_dependency` PASS
- `test_toolchain_arm32_no_toolchain` PASS（新增）
- `test_toolchain_arm64_no_toolchain` PASS（新增）
- `test_toolchain_none_eabi_missing_cpu` PASS（新增）
- `test_toolchain_user_block_preservation` PASS（新增）

- [ ] **Step 3: 发布版本构建**

```bash
cargo build --release
```

预期：零警告编译。

- [ ] **Step 4: Commit（如有微调）**

```bash
git add -A
git commit -m "chore: final adjustments after full test suite run

Co-Authored-By: Claude <noreply@anthropic.com>"
```

仅当有微调时才需此步骤。
