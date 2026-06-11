# EABI MCU 配置修复 & DependencyGraph 正确性修复

## 概述

修复两个问题：
1. 用户必须显式指定 ARM 芯片型号（`-mcpu=cortex-m3`），不允许静默降级
2. DependencyGraph 完全正确：消除重复磁盘 I/O、修复裸 `#include` 依赖缺失

---

## 改动范围

| 文件 | 改动类型 | 内容 |
|---|---|---|
| `src/orchestration/query.rs` | 新增 | ARM 嵌入式目标时交互式采集 `mcu_flags` |
| `src/core/analyzer.rs` | 重写 | 不再读磁盘，改用内存 `SourceFile.includes`；新增文件名回退匹配 |
| `src/cli/commands.rs` | 删除 | 移除 `scan_and_discover` 中的死代码循环（行 96-105） |
| `src/core/generator.rs` | 修改 | 删除 `default_mcu_for()`；`mcu_flags` 为空时对嵌入式目标报错 |
| `tests/integration.rs` | 新增 | 裸 include 依赖解析、MCU 配置测试 |

---

## 设计详情

### 1. MCU 芯片型号采集 (`query.rs`)

`ask_project_config()` 在架构选择完成后，若目标为 `NoneEabi | ARM32 | ARM64`，弹出：

```
ARM MCU/CPU (e.g. cortex-m3, cortex-m4, cortex-m7, cortex-a53) [cortex-m3]:
```

- ARM32/NoneEabi 默认值：`cortex-m3`
- ARM64 默认值：`cortex-a53`
- 用户输入存入 `config.mcu_flags`
- 允许空输入时使用默认值

### 2. 删除死代码 (`commands.rs`)

移除 `scan_and_discover()` 中第 96-105 行的循环：它收集 `sf.includes` 到局部变量 `includes` 但从未使用。注释称 analyzer 直接从 SourceFile 读取 — 现在 analyzer 确实将从内存读取。

### 3. 重写 Analyzer (`analyzer.rs`)

**删除**：`scan_file_includes()` 函数（读磁盘 + 正则解析）及 `use regex::Regex`。

**重写 `analyze()`**：
- 直接遍历 `module.sources[].includes`、`module.headers[].includes`、`module.asm_sources[].includes`
- 对每个 include 字符串做三段解析：
  1. 提取第一个路径段（`/` 之前的部分）
  2. 提取文件名（最后一个路径段，`/` 之后或无 `/` 时为整体）
  3. 去除扩展名得到基础名（如 `foo.h` → `foo`）

**匹配策略（两级回退）**：
1. **路径段匹配**（现有逻辑）：`core/foo.h` → `core` 匹配模块名或短目录名
2. **文件名回退**（新增）：若步骤 1 未匹配（裸 include），在所有其他模块的 `headers` 列表中按 `file_name` 查找。找到则创建 `PUBLIC` 依赖边。

所有操作零磁盘 I/O，纯内存数据变换。

### 4. 移除硬编码降级 (`generator.rs`)

- 删除 `default_mcu_for()` 函数
- `render_toolchain()` 中，当 `mcu_flags` 为空且架构为 `NoneEabi | ARM32 | ARM64 | RISCV64` 时，返回 `Err(FbGenError::Config(...))` 而非静默使用 `cortex-m3`
- 实际上 `mcu_flags` 现在由 `UserQuery` 保证非空，但防御性检查仍然保留

---

## 测试计划

### 新增测试

1. **裸 include 依赖解析**：模块 A 的 `a.c` 写 `#include "b.h"`（无路径前缀），模块 B 有 `b.h`。验证 `graph.get_dependencies("A")` 包含 `B`。

2. **文件名匹配不跨模块自引用**：模块自身的 `#include` 不应创建自依赖边。

### 已有测试

- 所有 12 个现有集成测试必须通过，特别是 `test_dependency_analysis`、`test_topological_order`、`test_cross_compile_template`、`test_toolchain_arm64`。
