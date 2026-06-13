# Design: Bridge `install` and `sync` via Project Cache

**Date:** 2026-06-14
**Status:** Approved

## Problem

`fb-gen install` downloads SDK/middleware packages globally (`~/.fb-gen/`), but
`fb-gen sync` has no way to detect that packages were installed or removed.
The `inject_installed_packages()` bridge function exists and is called by
`cmd_init` and `cmd_run` (first-time path), but `do_incremental_sync` (shared
by `sync` and `run --incremental`) never calls it.

The result: after `fb-gen install <pkg>`, running `fb-gen sync` produces
CMakeLists.txt files that do NOT include the installed package's sources,
include dirs, or compile definitions.

## Root Cause

Two missing pieces:

1. **`install` never touches the project cache.** It writes only to
   `~/.fb-gen/installed/` (global records). The project's `.fb-gen/cache/`
   has no record of which packages the project depends on.

2. **`do_incremental_sync` never calls `inject_installed_packages()`.**
   Even after `init` has run and packages are globally installed, the
   incremental sync path skips package injection entirely.

## Design

### Marker file: `.fb-gen/cache/installed_packages.json`

A JSON file in the project's cache directory that records which packages are
currently installed globally. Written by `install`/`uninstall`/`upgrade` and
read by `sync`.

```json
{
  "packages": ["stm32f1-hal", "freertos"],
  "updated_at": "2026-06-14T10:30:00Z"
}
```

- Only written when `.fb-gen/cache/` already exists (project has been `init`-ed).
- If the cache directory doesn't exist, `install` skips this step — `init` will
  pick up installed packages via `inject_installed_packages()` which scans
  global records directly.

### `ProjectMeta` change

Add `installed_packages_hash` field to track the marker file's content hash
across syncs:

```rust
pub struct ProjectMeta {
    // ... existing fields ...
    /// SHA-256 of .fb-gen/cache/installed_packages.json.
    /// Used to detect install/uninstall changes between syncs.
    #[serde(default)]
    pub installed_packages_hash: String,
}
```

### `install` command changes

After a successful install, uninstall, or upgrade:

1. Resolve project root from `--root` flag (defaults to cwd).
2. If `<root>/.fb-gen/cache/` does not exist, skip (no project cache to update).
3. Scan `~/.fb-gen/installed/` for all currently-installed packages.
4. Write `<root>/.fb-gen/cache/installed_packages.json`.

Affected functions in `src/install/mod.rs`:
- `install_package()` — write marker after successful install
- `uninstall_package()` — write marker after successful uninstall
- `upgrade_package()` — write marker after successful upgrade

### `do_incremental_sync` changes

Before the early-return check (`changed_paths.is_empty()`):

1. Read `<root>/.fb-gen/cache/installed_packages.json`.
2. Compute its SHA-256 hash.
3. Compare against `prev_meta.installed_packages_hash`.
4. If different → treat as "packages changed", skip early return, proceed to
   regeneration.
5. Before `generator.generate()`, call `inject_installed_packages(&mut modules, reporter)`.

### `init` / first-time `run` path

No changes needed. These paths already call `inject_installed_packages()`.
After generation, `save_meta_cache()` will naturally persist the marker file's
hash in `installed_packages_hash`.

## Data Flow

```
fb-gen install stm32f1-hal
  ├─ Download → ~/.fb-gen/toolchains/stm32f1-hal/1.2.3/
  ├─ Record  → ~/.fb-gen/installed/stm32f1-hal.json
  └─ Scan ~/.fb-gen/installed/*.json
      └─ Write <root>/.fb-gen/cache/installed_packages.json  [NEW]

fb-gen sync
  ├─ Load prev_meta (includes installed_packages_hash)
  ├─ Read .fb-gen/cache/installed_packages.json → compute hash
  ├─ Hash differs? → treat as change, skip early return        [NEW]
  ├─ inject_installed_packages(&mut modules)                   [NEW]
  └─ generator.generate(&modules, ...)
```

## Files Changed

| File | Change |
|---|---|
| `src/models/project.rs` | Add `installed_packages_hash` to `ProjectMeta` |
| `src/install/mod.rs` | Call `write_installed_packages_marker()` after install/uninstall/upgrade |
| `src/install/bridge.rs` | Add `write_installed_packages_marker()` and `read_installed_packages_marker()` helpers |
| `src/cli/commands.rs` | `do_incremental_sync`: add hash comparison + `inject_installed_packages()` call; `save_sync_result`: persist `installed_packages_hash` |

## Error Handling

| Scenario | Behavior |
|---|---|
| `installed_packages.json` missing | Treat as empty package list (hash = ""). Not an error — project may have no packages installed. |
| `installed_packages.json` malformed | Log warning via reporter, treat as empty list, don't block sync. |
| No `.fb-gen/cache/` directory (before init) | `install` skips writing marker. `init` calls `inject_installed_packages()` directly. |
| `install` run outside a project directory | `--root` defaults to cwd. If no cache there, marker write is silently skipped. |

## Testing

- `install` writes correct `installed_packages.json` after installing a package
- `uninstall` removes package ID from `installed_packages.json`
- `sync` with no source changes but new install → regenerates CMakeLists.txt
- `sync` with no changes at all → early-returns correctly
- `init` on fresh project with globally-installed packages → injects them
- `installed_packages.json` missing → `do_incremental_sync` doesn't crash
