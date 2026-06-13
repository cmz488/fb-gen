# `--lsp` Flag: Auto-generate `compile_commands.json`

## Context

LSP servers (clangd, ccls) need `compile_commands.json` to provide code intelligence
(go-to-definition, find-references, diagnostics). CMake generates this file when
`-DCMAKE_EXPORT_COMPILE_COMMANDS=ON` is passed during configure.

Currently only `fb-gen run` and `fb-gen validate` invoke cmake, and neither sets
this flag. `fb-gen init` and `fb-gen sync` only write CMakeLists.txt — they never
call cmake at all. Users must manually run cmake configure to get LSP support.

## Design

Add a global `--lsp` flag that, after the command's normal work, runs cmake
configure with `-DCMAKE_EXPORT_COMPILE_COMMANDS=ON` and symlinks the result to
the project root.

### CLI — `src/cli/mod.rs`

New global argument on `Cli`:

```rust
/// Generate compile_commands.json for LSP (clangd / ccls)
#[arg(long, global = true)]
pub lsp: bool,
```

All five subcommands automatically inherit the flag.

### Shared helper — `src/cli/commands.rs`

```rust
/// Run cmake configure to produce compile_commands.json, then symlink
/// it from the project root so LSP tools find it without extra config.
fn generate_compile_commands(
    root: &Path,
    build_dir: &Path,
    config: &ProjectConfig,
    reporter: &Reporter,
) -> FbGenResult<()>
```

Steps:

1. Ensure `build_dir` exists (`create_dir_all`).
2. Build the same cmake argument list as `cmd_run`:
   - `-S <root>` `-B <build_dir>`
   - Generator flags from `cmake_generator_flag(config)`
   - Toolchain args from `cmake_toolchain_args(config)`
   - `-DCMAKE_EXPORT_COMPILE_COMMANDS=ON`
3. Run `cmake` via `Command::new("cmake")`. On failure, report a warning (not
   a hard error — the primary command already succeeded).
4. If configure succeeded and `build_dir/compile_commands.json` exists, create
   a symlink at `root/compile_commands.json` pointing to it.
   - If the symlink already exists and points to the correct target, skip.
   - If a regular file or stale symlink exists at that path, replace it.
   - On Windows, copy the file instead of symlinking.

### Call sites

| Command | Insertion point |
|---------|----------------|
| `cmd_init` | After `save_meta_cache`, before summary |
| `cmd_sync` | After `cache.save(&meta)`, before elapsed report |
| `cmd_run` | Append `-DCMAKE_EXPORT_COMPILE_COMMANDS=ON` to the existing cmake configure call; symlink file after |
| `cmd_validate` | Append `-DCMAKE_EXPORT_COMPILE_COMMANDS=ON` to the existing cmake configure call; symlink file after |
| `cmd_check` | Not applicable (read-only, no cmake invocation) |

### Platform compatibility

- Unix: `std::os::unix::fs::symlink`
- Windows: `std::os::windows::fs::symlink_file` (fallback: copy)

### Error handling

`--lsp` failure is always a **warning**, never an error. The primary command
(init/sync/run/validate) must not be rolled back or reported as failed because
of a compile_commands.json generation issue.

## Verification

1. `cargo build` compiles cleanly.
2. `cargo test` — all 71 existing tests pass.
3. Manual: `fb-gen init --lsp` on a test project → verify `compile_commands.json`
   symlink appears in the project root.
4. Manual: `fb-gen sync --lsp` → symlink still correct after incremental sync.
5. Manual: `fb-gen run --lsp` → configure step includes the `-D` flag.
6. Manual: run without `--lsp` → zero behavioural change.
