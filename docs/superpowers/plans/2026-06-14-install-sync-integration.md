# Install-Sync Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bridge `fb-gen install` and `fb-gen sync` so that installed SDK/middleware packages are automatically linked into CMakeLists.txt generation.

**Architecture:** `install` writes a marker file (`.fb-gen/cache/installed_packages.json`) after install/uninstall/upgrade via a new bridge helper `write_installed_packages_marker`. `sync` detects changes by computing a hash of the marker file (via a local helper in `commands.rs`) and comparing against `ProjectMeta.installed_packages_hash`. When packages change, `do_incremental_sync` calls `inject_installed_packages()` before CMake generation.

**Tech Stack:** Rust, serde, std::collections::hash_map::DefaultHasher (for marker hash), existing fb-gen codebase patterns

**Layer note:** `write_installed_packages_marker` lives in `src/install/bridge.rs` and only calls sibling install modules. `compute_installed_packages_hash` lives in `src/cli/commands.rs` (CLI layer can import from all lower layers).

---

### Task 1: Add `installed_packages_hash` field to `ProjectMeta`

**Files:**
- Modify: `src/models/project.rs:172-178`

- [ ] **Step 1: Add the field**

```rust
/// Runtime metadata about the project, for caching and incremental updates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectMeta {
    pub config: ProjectConfig,
    pub modules: Vec<CMakeModule>,
    pub dependency_graph: DependencySnapshot,
    pub file_checksums: HashMap<String, String>,
    pub last_sync: String,
    /// Hash of .fb-gen/cache/installed_packages.json.
    /// Used to detect install/uninstall changes between syncs.
    /// Empty when no packages have ever been installed for this project.
    #[serde(default)]
    pub installed_packages_hash: String,
}
```

- [ ] **Step 2: Verify compilation**

```bash
cargo build 2>&1 | head -30
```

Expected: compiles with errors about missing field in `ProjectMeta` constructions (fixed in Task 3).

- [ ] **Step 3: Commit**

```bash
git add src/models/project.rs
git commit -m "feat: add installed_packages_hash to ProjectMeta

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 2: Add `write_installed_packages_marker` to bridge.rs

**Files:**
- Modify: `src/install/bridge.rs`

- [ ] **Step 1: Add `use std::path::Path` import**

Change line 4 from:
```rust
use std::path::PathBuf;
```
to:
```rust
use std::path::{Path, PathBuf};
```

- [ ] **Step 2: Add `write_installed_packages_marker` function**

Insert before the `#[cfg(test)]` block (before line 111):

```rust
/// Write `<root>/.fb-gen/cache/installed_packages.json` with the current
/// set of globally-installed package IDs.
///
/// Only writes when the cache directory already exists (project has been
/// init-ed).  No-op otherwise.
pub fn write_installed_packages_marker(root: &Path) {
    let cache_dir = root.join(".fb-gen").join("cache");
    if !cache_dir.exists() {
        return;
    }

    let install_root = crate::install::resolve_install_root();
    let records = crate::install::environment::read_installed_records(&install_root);

    let marker = serde_json::json!({
        "packages": records.iter().map(|r| &r.id).collect::<Vec<_>>(),
        "updated_at": chrono::Utc::now().to_rfc3339(),
    });

    let marker_path = cache_dir.join("installed_packages.json");
    if let Err(e) = std::fs::write(
        &marker_path,
        serde_json::to_string_pretty(&marker).unwrap_or_default(),
    ) {
        // Non-fatal: sync will still work, just without this project's
        // package list being up-to-date.
        eprintln!("fb-gen: warning: failed to write installed packages marker: {e}");
    }
}
```

- [ ] **Step 3: Verify compilation**

```bash
cargo build 2>&1 | head -20
```

Expected: compiles (function not yet called — may have dead_code warning, fine for now).

- [ ] **Step 4: Commit**

```bash
git add src/install/bridge.rs
git commit -m "feat: add write_installed_packages_marker helper to bridge

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 3: Add `compute_installed_packages_hash` helper + update all `ProjectMeta` sites

**Files:**
- Modify: `src/cli/commands.rs`

- [ ] **Step 1: Add `compute_installed_packages_hash` function**

Insert after the existing helper functions (before `cmd_init` around line 323, or after `inject_installed_packages` around line 189):

```rust
/// Compute a hash of `<root>/.fb-gen/cache/installed_packages.json`.
///
/// Returns an empty string when the marker file does not exist (no packages
/// have ever been installed for this project, or project hasn't been init-ed).
fn compute_installed_packages_hash(root: &Path) -> String {
    use std::hash::{Hash, Hasher};
    let marker_path = root
        .join(".fb-gen")
        .join("cache")
        .join("installed_packages.json");
    match std::fs::read(&marker_path) {
        Ok(data) => {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            data.hash(&mut hasher);
            format!("{:016x}", hasher.finish())
        }
        Err(_) => String::new(),
    }
}
```

Requires adding `use std::path::Path;` to the imports at the top of `commands.rs` if not already present.

- [ ] **Step 2: Update `save_sync_result` (line ~531)**

Add hash computation before `ProjectMeta` construction:

```rust
fn save_sync_result(
    cache: &MetaCache,
    config: &ProjectConfig,
    prev_meta: &ProjectMeta,
) -> FbGenResult<()> {
    let dep_snapshot = DependencySnapshot {
        nodes: prev_meta.modules.iter().map(|m| m.name.clone()).collect(),
        edges: prev_meta
            .modules
            .iter()
            .flat_map(|m| {
                rebuild_graph_from_snapshot(&prev_meta.dependency_graph)
                    .get_dependencies(&m.name)
                    .into_iter()
                    .map(|(dep_name, _)| (m.name.clone(), dep_name))
            })
            .collect(),
    };

    let installed_packages_hash = compute_installed_packages_hash(&config.root);

    let meta = ProjectMeta {
        config: config.clone(),
        modules: prev_meta.modules.clone(),
        dependency_graph: dep_snapshot,
        file_checksums: prev_meta.file_checksums.clone(),
        last_sync: chrono::Utc::now().to_rfc3339(),
        installed_packages_hash,
    };
    cache.save(&meta)
}
```

- [ ] **Step 3: Update `save_meta_cache` (line ~1126)**

Add hash computation before `ProjectMeta` construction:

```rust
fn save_meta_cache(
    root: &Path,
    modules: &[CMakeModule],
    graph: Option<&DependencyGraph>,
    config: &ProjectConfig,
) -> FbGenResult<()> {
    let cache = MetaCache::new(root);
    let all_paths: Vec<PathBuf> = modules
        .iter()
        .flat_map(|m| {
            m.sources
                .iter()
                .chain(m.headers.iter())
                .chain(m.asm_sources.iter())
                .map(|sf| sf.path.clone())
                .chain(m.linker_scripts.iter().cloned())
        })
        .collect();
    let checksums = cache.compute_checksums(&all_paths);

    let dep_snapshot = if let Some(g) = graph {
        DependencySnapshot {
            nodes: modules.iter().map(|m| m.name.clone()).collect(),
            edges: modules
                .iter()
                .flat_map(|m| {
                    g.get_dependencies(&m.name)
                        .into_iter()
                        .map(|(dep_name, _)| (m.name.clone(), dep_name))
                })
                .collect(),
        }
    } else {
        DependencySnapshot {
            nodes: vec![],
            edges: vec![],
        }
    };

    let installed_packages_hash = compute_installed_packages_hash(root);

    let meta = ProjectMeta {
        config: config.clone(),
        modules: modules.to_vec(),
        dependency_graph: dep_snapshot,
        file_checksums: checksums,
        last_sync: chrono::Utc::now().to_rfc3339(),
        installed_packages_hash,
    };
    cache.save(&meta)
}
```

- [ ] **Step 4: Update `cmd_run` inline `ProjectMeta` (line ~1396)**

```rust
                match do_incremental_sync(&root, &mut config, &mut prev_meta, &reporter) {
                    Ok(n) if n > 0 => {
                        // Persist updated metadata.
                        let installed_packages_hash =
                            compute_installed_packages_hash(&root);
                        let meta = ProjectMeta {
                            config: config.clone(),
                            modules: prev_meta.modules,
                            dependency_graph: prev_meta.dependency_graph,
                            file_checksums: prev_meta.file_checksums,
                            last_sync: chrono::Utc::now().to_rfc3339(),
                            installed_packages_hash,
                        };
                        if let Err(e) = cache.save(&meta) {
                            reporter.report_warning(&format!(
                                "Failed to save synced metadata: {}. Build will proceed.",
                                e
                            ));
                        } else {
                            reporter.report_success("CMakeLists.txt synced before build");
                        }
                    }
```

- [ ] **Step 5: Verify compilation**

```bash
cargo build 2>&1
```

Expected: compiles cleanly.

- [ ] **Step 6: Commit**

```bash
git add src/cli/commands.rs
git commit -m "feat: add compute_installed_packages_hash, persist in all ProjectMeta sites

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 4: Update `cmd_install` to write marker after mutations

**Files:**
- Modify: `src/cli/commands.rs:1932-2165`

- [ ] **Step 1: Add marker write after `upgrade_package`**

Change the early-return block at ~line 1945:

```rust
    // ── --upgrade: upgrade a package ──
    if let Some(pkg_id) = upgrade {
        install::upgrade_package(pkg_id)?;
        crate::install::bridge::write_installed_packages_marker(&cli.root);
        return Ok(());
    }
```

- [ ] **Step 2: Add marker write after `uninstall_package`**

Change the early-return block at ~line 1951:

```rust
    // ── --uninstall: remove a package ──
    if let Some(pkg_id) = uninstall {
        install::uninstall_package(pkg_id)?;
        crate::install::bridge::write_installed_packages_marker(&cli.root);
        return Ok(());
    }
```

- [ ] **Step 3: Add marker write after `install_package`**

After line 2157 (`install::install_package(pkg)?;`):

```rust
    // ── Execute install ──
    install::install_package(pkg)?;

    crate::install::bridge::write_installed_packages_marker(&cli.root);

    reporter.report_success(&format!("Installed {} v{}", pkg.id, pkg.version));
```

- [ ] **Step 4: Verify compilation**

```bash
cargo build 2>&1
```

Expected: compiles cleanly.

- [ ] **Step 5: Commit**

```bash
git add src/cli/commands.rs
git commit -m "feat: write installed packages marker after install/uninstall/upgrade

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 5: Update `do_incremental_sync` to detect package changes and inject

**Files:**
- Modify: `src/cli/commands.rs:659-813` (`do_incremental_sync`)

- [ ] **Step 1: Add installed-packages change detection BEFORE the first early-return**

Between computing `changed_paths` (line ~667) and the first early-return `if changed_paths.is_empty()` (line ~669), add:

```rust
    // ── Detect installed-package changes ──
    let current_packages_hash = compute_installed_packages_hash(root);
    let packages_changed = current_packages_hash != prev_meta.installed_packages_hash;
    if packages_changed {
        reporter.report_info("Installed packages changed — will regenerate.");
    }
```

- [ ] **Step 2: Modify the FIRST early-return guard**

Change:
```rust
    if changed_paths.is_empty() {
        return Ok(0);
    }
```
to:
```rust
    if changed_paths.is_empty() && !packages_changed {
        return Ok(0);
    }
```

This ensures that when packages change (even with no source file changes), we proceed through the pipeline to regenerate.

- [ ] **Step 3: Modify the SECOND early-return guard**

Change:
```rust
    if affected_modules.is_empty() && !module_list_changed {
```
to:
```rust
    if affected_modules.is_empty() && !module_list_changed && !packages_changed {
```

- [ ] **Step 4: Add `inject_installed_packages` call before generation**

Before the generation block starting around line 780 (`reporter.report_info("Regenerating affected CMakeLists.txt ...");`), add:

```rust
    // ── Inject installed SDK packages ──
    let sdk_count = inject_installed_packages(&mut modules, reporter);
    if sdk_count > 0 {
        reporter.report_success(&format!("Injected {} installed SDK package(s)", sdk_count));
    }
```

- [ ] **Step 5: Update `n_affected` to account for package changes**

After computing `n_affected` (currently `let n_affected = affected_modules.len();` at ~line 765), ensure the return value is ≥ 1 when packages changed so the caller (`cmd_sync`) saves the cache:

```rust
    let n_affected = if packages_changed {
        affected_modules.len().max(1)
    } else {
        affected_modules.len()
    };
```

Without this, `cmd_sync`'s `if n_affected > 0 { save_sync_result(...) }` would skip saving when only packages changed, causing the hash mismatch to re-trigger on every subsequent sync.

- [ ] **Step 6: Update `prev_meta.installed_packages_hash` after generation**

At the end of `do_incremental_sync`, just before `Ok(n_affected)`:

```rust
    prev_meta.installed_packages_hash = current_packages_hash;

    Ok(n_affected)
```

- [ ] **Step 5: Verify compilation**

```bash
cargo build 2>&1
```

Expected: compiles cleanly.

- [ ] **Step 6: Commit**

```bash
git add src/cli/commands.rs
git commit -m "feat: detect installed package changes and inject in incremental sync

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 6: Run existing tests and fix any breakage

- [ ] **Step 1: Run full test suite**

```bash
cargo test 2>&1
```

- [ ] **Step 2: Fix any tests that construct `ProjectMeta` inline**

Search for test code that constructs `ProjectMeta` without the new field:

```bash
grep -rn "ProjectMeta {" src/
```

For each test construction found, add `installed_packages_hash: String::new()`.

- [ ] **Step 3: Run tests again to confirm**

```bash
cargo test 2>&1
```

Expected: all tests pass.

- [ ] **Step 4: Commit any fixes**

```bash
git add -u
git commit -m "test: fix ProjectMeta constructions for new installed_packages_hash field

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 7: Verify end-to-end manually

- [ ] **Step 1: Build release binary**

```bash
cargo build --release
```

- [ ] **Step 2: Test install → marker file flow**

```bash
# Init a test project
./target/release/fb-gen init --root /tmp/test-fb-gen --name test-proj

# Install a package
./target/release/fb-gen install --arch NoneEabi --kind toolchain --root /tmp/test-fb-gen

# Verify marker file was written
cat /tmp/test-fb-gen/.fb-gen/cache/installed_packages.json
```

Expected: JSON file exists with `"packages": [...]` and `"updated_at"` fields.

- [ ] **Step 3: Test sync detects install changes**

```bash
# Run sync — should report "Installed packages changed"
./target/release/fb-gen sync --root /tmp/test-fb-gen 2>&1
```

Expected: output mentions "Installed packages changed" and "regenerat" (regenerating or similar).

- [ ] **Step 4: Test sync idempotency**

```bash
# Run sync again — should report no changes
./target/release/fb-gen sync --root /tmp/test-fb-gen 2>&1
```

Expected: output mentions "No changes detected" or similar.

- [ ] **Step 5: Test uninstall → sync detects change**

```bash
# Find the installed package ID from the marker file
PKG_ID=$(cat /tmp/test-fb-gen/.fb-gen/cache/installed_packages.json | grep -o '"packages":\["[^"]*"\]' | sed 's/.*"\([^"]*\)".*/\1/')

# Uninstall
./target/release/fb-gen install --uninstall "$PKG_ID" --root /tmp/test-fb-gen

# Sync — should detect change
./target/release/fb-gen sync --root /tmp/test-fb-gen 2>&1
```

Expected: sync detects the package removal and regenerates.

- [ ] **Step 6: Clean up**

```bash
rm -rf /tmp/test-fb-gen
```
