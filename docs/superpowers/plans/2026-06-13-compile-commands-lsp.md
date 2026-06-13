# `--lsp` compile_commands.json Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a global `--lsp` flag that runs cmake configure with `-DCMAKE_EXPORT_COMPILE_COMMANDS=ON` and symlinks the result to the project root.

**Architecture:** One new global CLI flag + one shared helper function (`generate_compile_commands`) + hook it into 4 commands (init, sync, run, validate). The helper is a pure add-on — it never causes the primary command to fail.

**Tech Stack:** Rust, std::process::Command, std::os::unix::fs::symlink / std::os::windows::fs::symlink_file

---

## File Map

| File | Action | Purpose |
|------|--------|---------|
| `src/cli/mod.rs` | Modify | Add `--lsp` global flag |
| `src/cli/commands.rs` | Modify | Add `generate_compile_commands()` helper + wire into 4 commands |

---

### Task 1: Add `--lsp` global flag

**Files:**
- Modify: `src/cli/mod.rs:33-34`

- [ ] **Step 1: Add the field to `Cli` struct**

Insert after the `watch` field (line 34):

```rust
    /// Enable file watcher for continuous generation
    #[arg(short = 'w', long, global = true)]
    pub watch: bool,

    /// Generate compile_commands.json after command completes (for LSP / clangd)
    #[arg(long, global = true)]
    pub lsp: bool,
```

- [ ] **Step 2: Build and verify**

Run: `cargo build`
Expected: compiles cleanly. The new flag is parsed but unused (Rust allows this).

- [ ] **Step 3: Commit**

```bash
git add src/cli/mod.rs
git commit -m "feat: add --lsp global flag to CLI

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 2: Add `generate_compile_commands` helper function

**Files:**
- Modify: `src/cli/commands.rs` (new function after `save_sync_result`)

- [ ] **Step 1: Add the function**

Insert after the `save_sync_result` function (after its closing `}`):

```rust
/// Run cmake configure to produce `compile_commands.json`, then symlink it
/// into the project root so LSP tools (clangd, ccls) find it automatically.
///
/// Failures are reported as warnings — they never block the primary command.
fn generate_compile_commands(
    root: &Path,
    build_dir: &Path,
    config: &ProjectConfig,
    reporter: &Reporter,
) {
    use std::io::ErrorKind;

    // Ensure build directory exists.
    if let Err(e) = std::fs::create_dir_all(build_dir) {
        reporter.report_warning(&format!(
            "Cannot create build dir for compile_commands.json: {e}"
        ));
        return;
    }

    // Assemble cmake args: same flags as cmd_run uses for configure.
    let gen_flags = cmake_generator_flag(config);
    let toolchain_args = cmake_toolchain_args(config);

    let mut cmd = Command::new("cmake");
    cmd.arg("-S").arg(root).arg("-B").arg(build_dir);
    for f in &gen_flags {
        cmd.arg(f);
    }
    for f in &toolchain_args {
        cmd.arg(f);
    }
    cmd.arg("-DCMAKE_EXPORT_COMPILE_COMMANDS=ON");

    reporter.report_info(&format!(
        "Running cmake configure for compile_commands.json (--lsp) ..."
    ));

    match cmd.output() {
        Ok(output) if output.status.success() => {
            let cc_json = build_dir.join("compile_commands.json");
            if !cc_json.exists() {
                reporter.report_warning(
                    "cmake succeeded but compile_commands.json was not produced."
                );
                return;
            }
            // Create / refresh symlink in project root.
            let link_path = root.join("compile_commands.json");
            match symlink_or_copy(&cc_json, &link_path) {
                Ok(()) => reporter.report_success("compile_commands.json → project root"),
                Err(e) => reporter.report_warning(&format!(
                    "compile_commands.json generated but symlink failed: {e}"
                )),
            }
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            reporter.report_warning(&format!(
                "cmake configure for --lsp failed (command succeeded, check toolchain):\n{}",
                stderr
            ));
        }
        Err(e) => {
            reporter.report_warning(&format!(
                "Cannot run cmake for --lsp (is cmake installed?): {e}"
            ));
        }
    }
}

/// Create a symlink at `link` pointing to `target`.  On Windows where
/// symlinks require elevated privileges, copy the file instead.
fn symlink_or_copy(target: &Path, link: &Path) -> std::io::Result<()> {
    // Remove existing link / file if present.
    if link.exists() {
        // If it's already a symlink pointing to the right target, we're done.
        if link.is_symlink() {
            if let Ok(existing) = std::fs::read_link(link) {
                if existing == target {
                    return Ok(());
                }
            }
        }
        std::fs::remove_file(link)?;
    }

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(windows)]
    {
        match std::os::windows::fs::symlink_file(target, link) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                // Fall back to copy on permission-denied (common without
                // developer mode on Windows).
                std::fs::copy(target, link)?;
                Ok(())
            }
            Err(e) => Err(e),
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        // Generic fallback: copy.
        std::fs::copy(target, link)?;
        Ok(())
    }
}
```

- [ ] **Step 2: Build and verify**

Run: `cargo build`
Expected: compiles cleanly. The new functions are unused for now (Rust allows this if they're private).

- [ ] **Step 3: Commit**

```bash
git add src/cli/commands.rs
git commit -m "feat: add generate_compile_commands helper with symlink logic

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 3: Wire `--lsp` into `cmd_init` and `cmd_sync`

**Files:**
- Modify: `src/cli/commands.rs` — `cmd_init` (~line 250) and `cmd_sync` (~line 290)

- [ ] **Step 1: Wire into `cmd_init`**

In `cmd_init`, after the summary report and before the final `Ok(())` (currently after line 250):

```rust
    // ── Summary ──
    let elapsed = start.elapsed();
    reporter.report_success(&format!(
        "Done in {:.1}s — {} modules, {} files",
        elapsed.as_secs_f64(),
        modules.len(),
        modules
            .iter()
            .map(|m| m.sources.len() + m.headers.len())
            .sum::<usize>()
    ));

    // ── LSP ──────────────────────────────────────────────────────────
    if cli.lsp {
        generate_compile_commands(&root, &config.output_dir, &config, &reporter);
    }

    Ok(())
```

- [ ] **Step 2: Wire into `cmd_sync`**

In `cmd_sync`, after the success report and before the final `Ok(())` (currently after line 292):

```rust
    } else {
        reporter.report_success("No changes detected — everything up to date.");
    }

    // ── LSP ──────────────────────────────────────────────────────────
    if cli.lsp {
        generate_compile_commands(&root, &config.output_dir, &config, &reporter);
    }

    // ── Watch loop ──────────────────────────────────────────────────
    if !cli.watch {
        return Ok(());
    }
```

- [ ] **Step 3: Build and verify**

Run: `cargo build`
Expected: compiles cleanly.

- [ ] **Step 4: Commit**

```bash
git add src/cli/commands.rs
git commit -m "feat: wire --lsp into cmd_init and cmd_sync

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 4: Wire `--lsp` into `cmd_run`

**Files:**
- Modify: `src/cli/commands.rs` — `cmd_run` (~line 1127-1143)

- [ ] **Step 1: Append -D flag to cmake configure call**

In `cmd_run`, locate the cmake configure command (around line 1127):

```rust
    let mut cmd = Command::new("cmake");
    cmd.arg("-S").arg(&root).arg("-B").arg(&build_dir);
    for f in &gen_flags {
        cmd.arg(f);
    }
    for f in &toolchain_args {
        cmd.arg(f);
    }
    if cli.lsp {
        cmd.arg("-DCMAKE_EXPORT_COMPILE_COMMANDS=ON");
    }
    let status = cmd
        .status()
        .map_err(|e| FbGenError::Config(format!("cmake: {e}")))?;

    if !status.success() {
        return Err(FbGenError::GenerationFailed(
            "cmake configure failed".into(),
        ));
    }

    // ── LSP symlink ──────────────────────────────────────────────────
    if cli.lsp {
        let cc_json = build_dir.join("compile_commands.json");
        if cc_json.exists() {
            match symlink_or_copy(&cc_json, &root.join("compile_commands.json")) {
                Ok(()) => reporter.report_success("compile_commands.json → project root"),
                Err(e) => reporter.report_warning(&format!(
                    "compile_commands.json symlink failed: {e}"
                )),
            }
        }
    }
```

Note: do NOT move the `// Build.` section — it stays right after.

- [ ] **Step 2: Build and verify**

Run: `cargo build`
Expected: compiles cleanly.

- [ ] **Step 3: Commit**

```bash
git add src/cli/commands.rs
git commit -m "feat: wire --lsp into cmd_run cmake configure

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 5: Wire `--lsp` into `cmd_validate`

**Files:**
- Modify: `src/cli/commands.rs` — `cmd_validate` (~line 880)

- [ ] **Step 1: Append -D flag and symlink after configure**

In `cmd_validate`, locate the cmake Command builder (same pattern as cmd_run). Add the flag and symlink in the same way as Task 4:

```rust
    let mut cmd = Command::new("cmake");
    cmd.arg("-S").arg(&root).arg("-B").arg(&build_dir);
    for f in &gen_flags {
        cmd.arg(f);
    }
    for f in &toolchain_args {
        cmd.arg(f);
    }
    if cli.lsp {
        cmd.arg("-DCMAKE_EXPORT_COMPILE_COMMANDS=ON");
    }
    let output = cmd
        .output()
        .map_err(|e| FbGenError::Config(format!("failed to run cmake: {e}")))?;

    if output.status.success() {
        // ── LSP symlink ──────────────────────────────────────────────
        if cli.lsp {
            let cc_json = build_dir.join("compile_commands.json");
            if cc_json.exists() {
                match symlink_or_copy(&cc_json, &root.join("compile_commands.json")) {
                    Ok(()) => reporter.report_success("compile_commands.json → project root"),
                    Err(e) => reporter.report_warning(&format!(
                        "compile_commands.json symlink failed: {e}"
                    )),
                }
            }
        }
        reporter.report_success("CMake configuration is valid.");
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        reporter.report_error("CMake configuration failed:");
        eprintln!("{}", stderr);
        return Err(FbGenError::GenerationFailed(
            "cmake validation failed".into(),
        ));
    }
```

- [ ] **Step 2: Build and verify**

Run: `cargo build`
Expected: compiles cleanly.

- [ ] **Step 3: Commit**

```bash
git add src/cli/commands.rs
git commit -m "feat: wire --lsp into cmd_validate cmake configure

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 6: Final verification

- [ ] **Step 1: Run full test suite**

Run: `cargo test`
Expected: all 71 tests pass.

- [ ] **Step 2: Run clippy**

Run: `cargo clippy`
Expected: no new warnings in `src/cli/`.

- [ ] **Step 3: Manual smoke test**

```bash
# Build the binary
cargo build --release

# On a test project with existing fb-gen cache:
cd /path/to/test1_fb_gen

# Test 1: --lsp flag on sync
fb-gen sync --lsp
# Expected: runs sync, then cmake configure, symlink appears at ./compile_commands.json
ls -la compile_commands.json  # should be a symlink → build/compile_commands.json

# Test 2: Without --lsp, no change
fb-gen sync
# Expected: sync only, no cmake invocation, no symlink created/updated

# Test 3: With run
fb-gen run --lsp
# Expected: configure step includes -DCMAKE_EXPORT_COMPILE_COMMANDS=ON banner in cmake output
```

- [ ] **Step 4: Commit any fixes**

If smoke test reveals issues, fix and commit.
