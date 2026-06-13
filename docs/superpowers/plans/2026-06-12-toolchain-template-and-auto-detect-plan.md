# 工具链模板优化 & 自动检测 — 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 采用 CubeMX 风格的工具链模板，自动检测根目录 `.ld` 文件和用户 CMakeLists.txt，并确保 target_link_libraries 的拓扑依赖排序。

**Architecture:** 
- `ToolchainConfig` 移除 `linker_script`，`generate()` 新增 `detect_linker_scripts()` 以自动检测 `.ld`；
- `FffScanner` 新增 `scan_user_cmake_files()`，`scan_and_discover()` 返回用户模块列表；
- `render_root()` 接收 `user_modules` 并预先填充 `USER_START` 块；
- `render_module()` 按拓扑顺序排序 `target_link_libraries`。

**Tech Stack:** Rust (edition 2021), walkdir, tera, tempfile

---

## File Structure

| 文件 | 角色 |
|---|---|
| `src/models/project.rs` | 从 `ToolchainConfig` 中移除 `linker_script` |
| `src/core/generator.rs` | CubeMX 模板（ASM 标志修复、格式、注释），`detect_linker_scripts()`，`user_modules` 到 `render_root()`，`render_module()` 中的拓扑依赖排序 |
| `src/scanner/fff_wrapper.rs` | 新增 `scan_user_cmake_files()` |
| `src/cli/commands.rs` | 调用 `scan_user_cmake_files()`，将 `user_modules` 传递给 `generate()` |
| `src/orchestration/query.rs` | 移除链接器脚本提示 |
| `tests/integration.rs` | 更新 `test_cross_compile_template`，新增 3 个测试 |

---

### Task 1: 从 ToolchainConfig 中移除 `linker_script`

**Files:**
- Modify: `src/models/project.rs`
- Modify: `src/orchestration/query.rs`

- [ ] **Step 1: 从 ToolchainConfig 中移除 `linker_script`**

在 `src/models/project.rs` 中，从 `ToolchainConfig` 结构体中移除：

```rust
    /// Linker script path relative to the project root.
    /// Generates `-T <path>` linker flag.
    pub linker_script: String,
```

同时从 `Default` 实现中移除 `linker_script: String::new()`。

- [ ] **Step 2: 从 query.rs 中移除链接器脚本提示**

在 `src/orchestration/query.rs` 的 `ask_project_config()` 中，移除这几行（约在第 125-129 行）：

```rust
            let linker_script = prompt_with_default(
                "  Linker script path (empty to skip) []",
                "",
            )
            .map_err(|e| {
                crate::models::FbGenError::Config(format!("failed to read linker script: {e}"))
            })?;
```

同时从 `ToolchainConfig` 构建中移除 `linker_script,`。

同时从 `confirm_config()` 中移除 `if !tc.linker_script.is_empty()` 块。

- [ ] **Step 3: 编译并运行库测试**

```bash
cargo build 2>&1
cargo test --lib 2>&1
```

预期：生成器中有编译错误（`render_embedded_toolchain` 等仍引用 `tc.linker_script`），此问题将在任务 2 中修复。库测试应通过（44 个），因为 confirm_config test test 不使用链接器脚本字段。

- [ ] **Step 4: Commit**

```bash
git add src/models/project.rs src/orchestration/query.rs
git commit -m "refactor: remove linker_script from ToolchainConfig (auto-detect replaces it)

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 2: 重写生成器 — CubeMX 模板、.ld 检测、用户模块、拓扑排序

**Files:**
- Modify: `src/core/generator.rs`

- [ ] **Step 1: 新增 `detect_linker_scripts()` 辅助函数**

在 `render_aarch64_toolchain` 之后（`render_arm_eabi_toolchain` 之前或附近）添加一个独立的辅助函数：

```rust
/// Scan the project root (non-recursive) for `.ld` linker scripts.
///
/// Returns the detected linker script file names (basenames only).
/// When exactly one `.ld` file is found at the root, it is used in the
/// toolchain file.  Zero or multiple `.ld` files → empty vec.
fn detect_linker_scripts(root: &Path) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    let entries = match std::fs::read_dir(root) {
        Ok(e) => e,
        Err(_) => return found,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if ext.eq_ignore_ascii_case("ld") {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        found.push(name.to_string());
                    }
                }
            }
        }
    }
    // Only auto-detect when there is exactly ONE .ld file at the root.
    if found.len() == 1 {
        found
    } else {
        Vec::new()
    }
}
```

- [ ] **Step 2: 在 `generate()` 中调用 `detect_linker_scripts()` 并传递**

在 `generate()` 方法中（第 191 行之前），添加：

```rust
        // ── Detect root-level linker scripts ───────────────────────
        let root_ld_scripts = detect_linker_scripts(&self.config.root);
```

将 `self.render_toolchain()?` 改为 `self.render_toolchain(&root_ld_scripts)?`。

同时在调用处（第 213 行）将 `self.render_module(module, &deps)` 改为 `self.render_module(module, &deps, graph)`。

- [ ] **Step 3: 更新 `render_toolchain()` 以接收 `root_ld_scripts` 并传递**

将 `render_toolchain` 方法签名从：
```rust
    fn render_toolchain(&self) -> Result<Option<String>> {
```
改为：
```rust
    fn render_toolchain(&self, root_ld_scripts: &[String]) -> Result<Option<String>> {
```

将调用 `render_arm_eabi_toolchain(tc)` 改为 `render_arm_eabi_toolchain(tc, root_ld_scripts)`，`render_riscv64_toolchain` 同理。

- [ ] **Step 4: 更新 toolchain 渲染函数以接收并传递 `root_ld_scripts`**

更新 `render_arm_eabi_toolchain`：
```rust
fn render_arm_eabi_toolchain(tc: &ToolchainConfig, root_ld_scripts: &[String]) -> String {
    let prefix = "arm-none-eabi-";
    render_embedded_toolchain("Generic", "arm", prefix, tc, root_ld_scripts)
}
```

更新 `render_riscv64_toolchain`：
```rust
fn render_riscv64_toolchain(tc: &ToolchainConfig, root_ld_scripts: &[String]) -> String {
    let prefix = "riscv64-unknown-elf-";
    render_embedded_toolchain("Generic", "riscv64", prefix, tc, root_ld_scripts)
}
```

更新 `render_embedded_toolchain` 签名以接收 `root_ld_scripts: &[String]`。

将链接器脚本标志汇编替换为：
```rust
    // Linker-script: auto-detect from project root (exactly one .ld file).
    let ld_flag = if root_ld_scripts.len() == 1 {
        format!(" -T \"${{CMAKE_SOURCE_DIR}}/{}\"", root_ld_scripts[0])
    } else {
        String::new()
    };
```

- [ ] **Step 5: 重写模板格式（CubeMX 风格）**

将整个 `render_embedded_toolchain` 中的 `format!(...)` 块替换为：

```rust
    format!(
        r#"# Auto-generated toolchain file by fb-gen

set(CMAKE_SYSTEM_NAME               {system_name})
set(CMAKE_SYSTEM_PROCESSOR          {processor})

set(CMAKE_C_COMPILER_ID GNU)
set(CMAKE_CXX_COMPILER_ID GNU)

# Some default GCC settings
# {prefix} must be part of path environment
set(TOOLCHAIN_PREFIX                {prefix})

set(CMAKE_C_COMPILER                ${{TOOLCHAIN_PREFIX}}gcc)
set(CMAKE_ASM_COMPILER              ${{CMAKE_C_COMPILER}})
set(CMAKE_CXX_COMPILER              ${{TOOLCHAIN_PREFIX}}g++)
set(CMAKE_LINKER                    ${{TOOLCHAIN_PREFIX}}g++)
set(CMAKE_OBJCOPY                   ${{TOOLCHAIN_PREFIX}}objcopy)
set(CMAKE_SIZE                      ${{TOOLCHAIN_PREFIX}}size)

set(CMAKE_EXECUTABLE_SUFFIX_ASM     ".elf")
set(CMAKE_EXECUTABLE_SUFFIX_C       ".elf")
set(CMAKE_EXECUTABLE_SUFFIX_CXX     ".elf")

set(CMAKE_TRY_COMPILE_TARGET_TYPE STATIC_LIBRARY)

# ── Cross-compilation root paths ─────────────────────────────────
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_PACKAGE ONLY)

# MCU specific flags
set(TARGET_FLAGS "{target_flags} ")

set(CMAKE_C_FLAGS "${{CMAKE_C_FLAGS}} ${{TARGET_FLAGS}}")
set(CMAKE_C_FLAGS "${{CMAKE_C_FLAGS}} -Wall -fdata-sections -ffunction-sections -fstack-usage")
set(CMAKE_ASM_FLAGS "${{CMAKE_C_FLAGS}} -x assembler-with-cpp -MMD -MP")

set(CMAKE_C_FLAGS_DEBUG "-O0 -g3")
set(CMAKE_C_FLAGS_RELEASE "-Os -g0")
set(CMAKE_CXX_FLAGS_DEBUG "-O0 -g3")
set(CMAKE_CXX_FLAGS_RELEASE "-Os -g0")

set(CMAKE_CXX_FLAGS "${{CMAKE_C_FLAGS}} -fno-rtti -fno-exceptions -fno-threadsafe-statics")

set(CMAKE_EXE_LINKER_FLAGS "${{TARGET_FLAGS}}")
set(CMAKE_EXE_LINKER_FLAGS "${{CMAKE_EXE_LINKER_FLAGS}}{ld_flag}")
set(CMAKE_EXE_LINKER_FLAGS "${{CMAKE_EXE_LINKER_FLAGS}} --specs=nano.specs")
set(CMAKE_EXE_LINKER_FLAGS "${{CMAKE_EXE_LINKER_FLAGS}} -Wl,-Map=${{CMAKE_PROJECT_NAME}}.map -Wl,--gc-sections")
set(CMAKE_EXE_LINKER_FLAGS "${{CMAKE_EXE_LINKER_FLAGS}} -Wl,--print-memory-usage")
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
```

关键变更：
- `CMAKE_ASM_FLAGS` 现在基于 `CMAKE_C_FLAGS`（不仅仅是 `CMAKE_ASM_FLAGS`）
- `CMAKE_EXE_LINKER_FLAGS` 中 `-T` 标志在 `--specs=nano.specs` **之前**（参考 CubeMX）
- 对齐格式
- 通过注释说明环境路径要求

- [ ] **Step 6: 更新 `render_root()` 以接收 `user_modules`**

将函数签名改为：
```rust
    fn render_root(&self, modules: &[CMakeModule], graph: &DependencyGraph, user_modules: &[PathBuf]) -> Result<String> {
```

将 `ctx.insert("subdirs", &subdirs);` 之前的 USER_START 块修改为在上下文中包含 `user_modules`：

在 `ctx.insert("subdirs", &subdirs);` 之前，添加：
```rust
        // User-defined CMake modules (pre-existing CMakeLists.txt not from fb-gen).
        let user_module_dirs: Vec<String> = user_modules
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();
        ctx.insert("user_modules", &user_module_dirs);
```

在 `ROOT_TEMPLATE` 中（`const ROOT_TEMPLATE` 字符串），将：
```cmake
# ── User customisations ─────────────────────────────────────────────────────
# USER_START
# USER_END
```

替换为：
```cmake
# ── User customisations ─────────────────────────────────────────────────────
# USER_START
{% for um in user_modules -%}
add_subdirectory({{ um }})
{% endfor %}
# USER_END
```

- [ ] **Step 7: 更新 `render_module()` 以进行拓扑依赖排序**

将 `render_module` 签名改为：
```rust
    fn render_module(&self, module: &CMakeModule, deps: &[(String, DependencyType)], graph: &DependencyGraph) -> Result<String> {
```

在合并依赖项列表之前，在方法中添加排序逻辑：
```rust
        // Sort dependencies in topological order: dependencies first, dependents later.
        let ordered_modules = graph.topological_order().unwrap_or_default();
        let mut sorted_deps = deps.to_vec();
        sorted_deps.sort_by_key(|(name, _)| {
            ordered_modules.iter().position(|n| n == name).unwrap_or(usize::MAX)
        });

        // Dependencies with their type.
        let dep_list: Vec<tera::Value> = sorted_deps
            .iter()
            .map(|(name, dep_type)| {
```

将 `deps.iter()` 替换为 `sorted_deps.iter()`。

- [ ] **Step 8: 编译**

```bash
cargo build 2>&1
```

预期：由于 `generate()`、`render_root()`、`render_module()` 的签名变更，`commands.rs` 中出现编译错误。这些错误将在任务 4 中修复。

- [ ] **Step 9: Commit**

```bash
git add src/core/generator.rs
git commit -m "feat: CubeMX-style template, auto-detect .ld, user_modules, topological deps

- Fix CMAKE_ASM_FLAGS to base on CMAKE_C_FLAGS (like CubeMX)
- CubeMX-style alignment formatting and environment comments
- detect_linker_scripts() auto-detects single .ld at project root
- render_root accepts user_modules for USER_START block
- render_module sorts target_link_libraries in topological order
- Remove linker_script from ToolchainConfig usage

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 3: 在 scanner 中新增 `scan_user_cmake_files()`

**Files:**
- Modify: `src/scanner/fff_wrapper.rs`

- [ ] **Step 1: 新增 `scan_user_cmake_files()`**

在 `FffScanner` 的 `impl` 块中，于 `scan_toolchain_files` 之后添加方法：

```rust
    /// Scan subdirectories for existing `CMakeLists.txt` files that were NOT
    /// generated by fb-gen (no "Generated by fb-gen" header).
    ///
    /// Returns paths to the parent directories (relative to root) of user-owned
    /// CMakeLists.txt files.  These directories should be added as
    /// `add_subdirectory` in the root CMakeLists.txt USER_START block.
    pub fn scan_user_cmake_files(
        &self,
        root: &Path,
        exclude_dirs: &[String],
    ) -> Vec<PathBuf> {
        let mut user_dirs: Vec<PathBuf> = Vec::new();

        let walker = WalkDir::new(root)
            .max_depth(3)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                if e.file_type().is_dir() {
                    let name = e.file_name().to_string_lossy();
                    !exclude_dirs.iter().any(|d| d == name.as_ref())
                } else {
                    true
                }
            });

        for entry in walker {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };

            if !entry.file_type().is_file() {
                continue;
            }

            if entry.file_name() != "CMakeLists.txt" {
                continue;
            }

            // Skip root CMakeLists.txt — only interested in subdirectories.
            let path = entry.path();
            if path.parent().map_or(true, |p| p == root) {
                continue;
            }

            // Read and check if fb-gen generated.
            let content = match std::fs::read_to_string(path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            if !content.contains("Generated by fb-gen") {
                // User-owned CMakeLists.txt — record its parent directory.
                if let Some(parent) = path.parent() {
                    if let Ok(rel) = parent.strip_prefix(root) {
                        if !rel.as_os_str().is_empty() {
                            user_dirs.push(rel.to_path_buf());
                        }
                    }
                }
            }
        }

        user_dirs
    }
```

- [ ] **Step 2: 编译**

```bash
cargo build 2>&1
```

预期：仅在 `commands.rs` 中有编译错误（`generate` 签名不匹配）。无新的警告或错误。

- [ ] **Step 3: Commit**

```bash
git add src/scanner/fff_wrapper.rs
git commit -m "feat: add scan_user_cmake_files() for detecting non-fb-gen CMakeLists.txt

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 4: 更新 `commands.rs` 以连接用户模块和新的生成器签名

**Files:**
- Modify: `src/cli/commands.rs`

- [ ] **Step 1: 在 `scan_and_discover()` 中新增 `user_modules` 返回值**

将返回类型从：
```rust
) -> Result<(Vec<CMakeModule>, Option<DependencyGraph>)> {
```
改为：
```rust
) -> Result<(Vec<CMakeModule>, Option<DependencyGraph>, Vec<PathBuf>)> {
```

在 `scan_and_discover` 末尾，return 之前添加：
```rust
    // ── Detect user-defined CMakeLists ──
    let user_modules = scanner.scan_user_cmake_files(&config.root, &config.exclude_dirs);
    if !user_modules.is_empty() {
        reporter.report_info(&format!(
            "Found {} user-defined CMake module(s)",
            user_modules.len()
        ));
    }
```

并将 return 更新为 `Ok((modules, graph, user_modules))`。

- [ ] **Step 2: 更新所有 `scan_and_discover` 调用**

在 `cmd_init()` 中（第 156 行），更新解构：
```rust
    let (modules, graph, user_modules) = scan_and_discover(cli, &config, &reporter)?;
```

并更新 `generate` 调用以传递 `user_modules`：
```rust
    generator.generate(&modules, ref_graph, true, &user_modules)?;
```

在 `cmd_check()` 中（第 576 行），更新解构和调用：
```rust
    let (modules, graph, user_modules) = scan_and_discover(cli, &config, &reporter)?;
    // ...
    generator.generate(&modules, ref_graph, false, &user_modules)?;
```

在 `cmd_run()` 中（约第 706 行），做相同操作。

- [ ] **Step 3: 将 `save_meta_cache` 调用中的 `user_modules` 纳入元数据或忽略**

为简单起见，不在缓存中持久化 `user_modules`（sync 会在每次运行时重新检测它们）。将 `user_modules` 传递到 `save_meta_cache` 调用，但暂时保持函数签名不变（用户模块不会保存到缓存中，sync 会重新扫描）。

- [ ] **Step 4: 更新 `cmd_sync()` 以重新发现用户模块**

在 `cmd_sync()` 的重生成部分（约第 301 行），在调用 `generator.generate(...)` 之前添加：
```rust
    let scanner = FffScanner::new(&root);
    let user_modules = scanner.scan_user_cmake_files(&root, &config.exclude_dirs);
```

并更新 generate 调用。

- [ ] **Step 5: 编译**

```bash
cargo build 2>&1
```

预期：编译成功，零警告。

- [ ] **Step 6: Commit**

```bash
git add src/cli/commands.rs
git commit -m "feat: wire user_modules and new generator signatures into CLI commands

- scan_and_discover returns user_modules from scan_user_cmake_files()
- cmd_init, cmd_check, cmd_run pass user_modules to generate()
- cmd_sync re-discovers user modules before regeneration

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 5: 更新集成测试

**Files:**
- Modify: `tests/integration.rs`

- [ ] **Step 1: 更新所有 `generate()` 调用以包含空 `user_modules`**

在所有集成测试中，将 `generator.generate(&modules, &empty_graph, true)` 替换为 `generator.generate(&modules, &empty_graph, true, &[])`。

测试列表：
- `test_cmake_generation`
- `test_cross_compile_template`
- `test_toolchain_not_generated_for_x86`
- `test_toolchain_riscv64`
- `test_toolchain_arm32_no_toolchain`
- `test_toolchain_arm64_no_toolchain`
- `test_toolchain_none_eabi_missing_cpu`
- `test_toolchain_user_block_preservation`

- [ ] **Step 2: 更新 `test_cross_compile_template` — 新增根目录 `.ld` 文件**

在创建测试项目之后、扫描文件之前，添加：

```rust
    // Create a linker script at root for auto-detection test.
    std::fs::write(
        root.join("STM32F103XX_FLASH.ld"),
        "MEMORY { FLASH (rx) : ORIGIN = 0x08000000, LENGTH = 512K }\n",
    )
    .unwrap();
```

更新 `ToolchainConfig` 构建以**不**包含链接器脚本（已移除字段）：
```rust
        toolchain: Some(fb_gen::models::project::ToolchainConfig {
            cpu: "cortex-m3".into(),
            ..Default::default()
        }),
```

添加新的断言：
```rust
    // ── Auto-detected linker script ─────────────────────────────────
    assert!(
        toolchain_content.contains("-T \"${CMAKE_SOURCE_DIR}/STM32F103XX_FLASH.ld\""),
        "toolchain.cmake should auto-detect STM32F103XX_FLASH.ld at root"
    );
```

- [ ] **Step 3: 更新 `test_toolchain_user_block_preservation` 模板断言**

更新测试中的 `ToolchainConfig` 构建以移除 `linker_script` 字段。更新任何引用旧模板格式的字符串断言。

- [ ] **Step 4: 新增 `test_linker_script_auto_detect_single`**

```rust
#[test]
fn test_linker_script_auto_detect_single() {
    // Verify that a single .ld file at project root is auto-detected
    // and included in the toolchain file.
    let tmp = TempDir::new().unwrap();
    create_test_project(&tmp);
    let root = tmp.path().to_path_buf();

    // Create exactly one linker script at root.
    std::fs::write(
        root.join("flash.ld"),
        "MEMORY { FLASH (rx) : ORIGIN = 0x08000000, LENGTH = 256K }\n",
    )
    .unwrap();

    let sources = scan_project(&root);
    let discoverer = ModuleDiscoverer::new(ScanOptions {
        exclude_dirs: vec![],
        root: root.clone(),
    });
    let modules = discoverer.discover(&sources).unwrap();

    let config = ProjectConfig {
        name: "LdDetectTest".into(),
        version: "0.1.0".into(),
        root: root.clone(),
        target_arch: fb_gen::models::project::TargetArch::NoneEabi,
        toolchain: Some(fb_gen::models::project::ToolchainConfig {
            cpu: "cortex-m3".into(),
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
        content.contains("-T \"${CMAKE_SOURCE_DIR}/flash.ld\""),
        "toolchain.cmake should auto-detect flash.ld at root, got:\n{}",
        content
    );
}
```

- [ ] **Step 5: 新增 `test_linker_script_auto_detect_multiple`**

```rust
#[test]
fn test_linker_script_auto_detect_multiple() {
    // Verify that multiple .ld files at root → no -T flag (ambiguous).
    let tmp = TempDir::new().unwrap();
    create_test_project(&tmp);
    let root = tmp.path().to_path_buf();

    // Create TWO linker scripts at root (ambiguous).
    std::fs::write(
        root.join("flash_256k.ld"),
        "MEMORY { FLASH (rx) : ORIGIN = 0x08000000, LENGTH = 256K }\n",
    )
    .unwrap();
    std::fs::write(
        root.join("flash_512k.ld"),
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
        name: "LdAmbiguousTest".into(),
        version: "0.1.0".into(),
        root: root.clone(),
        target_arch: fb_gen::models::project::TargetArch::NoneEabi,
        toolchain: Some(fb_gen::models::project::ToolchainConfig {
            cpu: "cortex-m3".into(),
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
        !content.contains("-T \"${CMAKE_SOURCE_DIR}"),
        "toolchain.cmake should NOT auto-detect linker script when multiple .ld files at root"
    );
}
```

- [ ] **Step 6: 新增 `test_user_cmake_detection`**

```rust
#[test]
fn test_user_cmake_detection() {
    // Verify that pre-existing, non-fb-gen CMakeLists.txt files are
    // detected and listed in the root CMakeLists.txt USER_START block.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().to_path_buf();

    // Create a normal project structure.
    let core_dir = root.join("core");
    std::fs::create_dir(&core_dir).unwrap();
    std::fs::write(core_dir.join("core.c"), "int core_func() { return 0; }\n").unwrap();

    // Create a user-defined CMakeLists.txt in a subdirectory (no fb-gen header).
    let drivers_dir = root.join("Drivers");
    std::fs::create_dir(&drivers_dir).unwrap();
    std::fs::write(
        drivers_dir.join("CMakeLists.txt"),
        "project(Drivers)\nadd_library(drivers STATIC driver.c)\n",
    )
    .unwrap();

    // Scan for user CMakeLists.
    let scanner = FffScanner::new(&root);
    let user_modules = scanner.scan_user_cmake_files(&root, &["build".into(), ".git".into()]);

    assert_eq!(
        user_modules.len(),
        1,
        "should detect 1 user CMake module, got: {:?}",
        user_modules
    );
    assert!(
        user_modules[0].to_string_lossy().contains("Drivers"),
        "user module should be Drivers directory"
    );

    // Generate root CMakeLists.txt.
    let sources: Vec<SourceFile> = fb_gen::scanner::fs_adapter::ScanOptions {
        root: root.clone(),
        ..Default::default()
    };
    let scanner2 = FffScanner::new(&root);
    let sources = scanner2.scan_source_files(&fb_gen::scanner::fs_adapter::ScanOptions {
        root: root.clone(),
        ..Default::default()
    }).unwrap();

    let discoverer = ModuleDiscoverer::new(ScanOptions {
        exclude_dirs: vec![],
        root: root.clone(),
    });
    let modules = discoverer.discover(&sources).unwrap();

    let analyzer = DependencyAnalyzer::new();
    let graph = analyzer.analyze(&modules).unwrap();

    let config = ProjectConfig {
        name: "UserModulesTest".into(),
        version: "0.1.0".into(),
        root: root.clone(),
        ..Default::default()
    };

    let generator = CMakeGenerator::new(&config).unwrap();
    generator.generate(&modules, &graph, true, &user_modules).unwrap();

    let root_cmake = root.join("CMakeLists.txt");
    let content = std::fs::read_to_string(&root_cmake).unwrap();

    assert!(
        content.contains("add_subdirectory(Drivers)"),
        "root CMakeLists.txt should include user module Drivers in USER_START block"
    );
    // It should appear between USER_START and USER_END.
    let start = content.find("# USER_START").unwrap();
    let end = content.find("# USER_END").unwrap();
    let block = &content[start..end];
    assert!(
        block.contains("add_subdirectory(Drivers)"),
        "Drivers should be listed in the USER_START block"
    );
}
```

- [ ] **Step 7: 运行新增的测试**

```bash
cargo test --test integration test_linker_script_auto_detect_single
cargo test --test integration test_linker_script_auto_detect_multiple
cargo test --test integration test_user_cmake_detection
```

预期：全部通过。

- [ ] **Step 8: Commit**

```bash
git add tests/integration.rs
git commit -m "test: add auto-detect .ld, user CMakeLists, update existing tests

- Update all generate() calls with &[] user_modules parameter
- test_cross_compile_template: add root .ld file, verify auto-detection
- Add test_linker_script_auto_detect_single
- Add test_linker_script_auto_detect_multiple
- Add test_user_cmake_detection

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 6: 全量测试验证

**Files:** (无改动，仅验证)

- [ ] **Step 1: 运行全部库测试**

```bash
cargo test --lib
```

预期：所有 44 个单元测试通过。

- [ ] **Step 2: 运行全部集成测试**

```bash
cargo test --test integration
```

预期：所有测试通过。测试总数：18 个旧测试 + 3 个新增 = **21 个测试**。

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
