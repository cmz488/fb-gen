# 工具链配置重构 — 设计规范

## 概述

重构嵌入式交叉编译工具链配置，使其包含结构化字段、正确的架构分离及用户块保留机制。

### 解决的问题

1. **ARM32 与 NoneEabi 混淆**：两者使用了相同的工具链前缀（`arm-none-eabi-`），但 ARM32 是 Linux 用户空间，不应生成工具链文件
2. **ARM64 角色不明确**：使用了 `aarch64-none-elf-`（裸金属），但 ARM64 应跳过工具链生成
3. **`mcu_flags` 过于简单**：单个字符串无法捕获浮点 ABI、FPU、额外标志和链接器脚本
4. **工具链模板不完整**：缺少 `CMAKE_FIND_ROOT_PATH_MODE`
5. **工具链文件无用户块**：用户对 `toolchain.cmake` 的编辑在重新生成时会被覆盖
6. **确认摘要不可见**：MCU 配置未在 `ConfirmConfig` 中显示

---

## 架构决策

### 目标架构与工具链生成

| 架构 | 生成工具链？ | 系统名称 | 编译器前缀 |
|---|---|---|---|
| NoneEabi | ✅ | Generic | arm-none-eabi- |
| ARM32 | ❌ | — | — |
| ARM64 | ❌ | — | — |
| RISCV64 | ✅ | Generic | riscv64-unknown-elf- |
| X86_64 / X86 | ❌ | — | — |
| WASM | ❌ | — | — |

---

## 模型变更

### `ProjectConfig` 字段替换

**删除**：`pub mcu_flags: String`

**新增**：`pub toolchain: Option<ToolchainConfig>`

### `ToolchainConfig` 结构体（`src/models/project.rs`）

```rust
/// 嵌入式交叉编译目标的结构化工具链配置。
/// 当 target_arch 不需要工具链时为 `None`。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolchainConfig {
    /// CPU/芯片型号（例如 "cortex-m3"、"cortex-m4"）
    /// 生成 -mcpu= 标志。NoneEabi 必填。
    pub cpu: String,

    /// 浮点 ABI：soft、softfp 或 hard。为空则跳过。
    /// 生成 -mfloat-abi= 标志。
    pub float_abi: String,

    /// FPU 单元（例如 "fpv4-sp-d16"、"fpv5-d16"）
    /// 生成 -mfpu= 标志。
    pub fpu: String,

    /// 原始编译器/链接器标志（例如 "-mthumb"、"-march=rv32imac"）
    /// 原样追加到 TARGET_FLAGS。
    pub extra_flags: String,

    /// 相对于项目根目录的链接器脚本路径
    /// 生成 -T <path> 链接器标志。
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

验证：当 `target_arch == NoneEabi` 且 `toolchain.cpu.is_empty()` 时必须返回错误。

### 受影响的代码位置

| 文件 | 变更 |
|---|---|
| `src/models/project.rs` | 新增 `ToolchainConfig`，将 `mcu_flags` 替换为 `toolchain: Option<ToolchainConfig>` |
| `src/orchestration/query.rs` | 仅 NoneEabi 显示 MCU 提示，移除 RISCV64 提示。在 `confirm_config` 中显示工具链设置 |
| `src/core/generator.rs` | `render_toolchain()` 接收 `&ToolchainConfig`；为工具链文件新增 `USER_START`/`USER_END` 标记；新增 `CMAKE_FIND_ROOT_PATH_MODE` |
| `src/orchestration/cache.rs` | 更新 `make_meta()` 测试辅助函数 |
| `src/orchestration/workflow.rs` | 更新 `make_config()` 测试辅助函数 |
| `tests/integration.rs` | 更新工具链测试以使用 `ToolchainConfig` |

---

## 交互式流程

### `fb-gen init` — 架构 NoneEabi 后的 MCU 提示

```
  ARM MCU/CPU selection:
    Specify the target chip model for -mcpu= flag.
  ARM MCU/CPU [cortex-m3]:

  Float ABI (soft/softfp/hard, empty to skip) []:

  FPU (e.g. fpv4-sp-d16, empty to skip) []:

  Extra flags (e.g. -mthumb, empty to skip) []:

  Linker script path (empty to skip) []:
```

- 仅当 `target_arch == NoneEabi` 时出现
- ARM32 / ARM64 / RISCV64 / X86* → 无 MCU 提示

### 确认摘要 — 显示工具链设置

当 `toolchain.is_some()` 时，在 `confirm_config` 中添加：

```
  Toolchain:
    CPU:              cortex-m3
    Float ABI:        hard
    FPU:              fpv4-sp-d16
    Extra flags:      -mthumb
    Linker script:    link.ld
```

---

## 工具链模板变更

### `render_embedded_toolchain()` — 接收 `&ToolchainConfig`

标志组装：
```rust
let mut flags = Vec::new();
if !tc.cpu.is_empty()       { flags.push(format!("-mcpu={}", tc.cpu)); }
if !tc.float_abi.is_empty() { flags.push(format!("-mfloat-abi={}", tc.float_abi)); }
if !tc.fpu.is_empty()       { flags.push(format!("-mfpu={}", tc.fpu)); }
if !tc.extra_flags.is_empty(){ flags.push(tc.extra_flags.clone()); }
let target_flags = flags.join(" ");
```

链接器脚本：
```rust
let ld_flag = if !tc.linker_script.is_empty() {
    format!("-T ${{CMAKE_SOURCE_DIR}}/{}", tc.linker_script)
} else {
    String::new()
};
```

### 模板新增内容

```cmake
# ── 交叉编译根路径 ─────────────────────────────────────────
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_PACKAGE ONLY)
```

### 用户块标记

在 `toolchain.cmake` 模板末尾添加：
```cmake
# ── 用户自定义 ──────────────────────────────────────────────
# USER_START
# USER_END
```

写入时：与现有 CMakeLists.txt 逻辑完全相同 —— 若目标存在则提取 `# USER_START` … `# USER_END` 块，并合并到新生成的内容中。

---

## 测试计划

### 更新的测试

| 测试 | 变更 |
|---|---|
| `test_cross_compile_template` | 使用 `ToolchainConfig { cpu: "cortex-m3".into(), ..Default::default() }` 替代 `mcu_flags: "cortex-m3"` |
| `test_toolchain_arm64` | **删除** — ARM64 不再生成工具链 |
| `test_toolchain_riscv64` | 使用 `toolchain: Some(ToolchainConfig::default())`（空 CPU，RISCV 无 -mcpu=） |
| `test_toolchain_not_generated_for_x86` | 不变 |
| `test_cmake_generation` | 将 `mcu_flags` 替换为 `toolchain: None` |

### 新增测试

1. **`test_toolchain_user_block_preservation`**：验证用户对 `toolchain.cmake` 的编辑在强制和非强制模式下都能保留
2. **`test_toolchain_none_eabi_missing_cpu`**：验证 NoneEabi 在没有 `cpu` 时返回错误
3. **`test_toolchain_arm32_no_toolchain`**：验证 ARM32 不生成 toolchain.cmake
4. **`test_toolchain_arm64_no_toolchain`**：验证 ARM64 不生成 toolchain.cmake

---

## 错误处理

- `target_arch == NoneEabi && toolchain.cpu.is_empty()` → `Err(FbGenError::Config("MCU/CPU is required for NoneEabi targets. Run `fb-gen init` to configure."))`
- ARM32、ARM64、X86_64、X86、WASM、Custom → `render_toolchain()` 返回 `Ok(None)`
- 若 `toolchain` 为 `None` → `render_toolchain()` 返回 `Ok(None)`
