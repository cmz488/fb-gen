//! CLI command implementations — wires scanner → discoverer → analyzer → generator.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::LazyLock;
use std::thread;
use std::time::Instant;

use colored::{Color, Colorize};
use regex::Regex;

use crate::cli::Cli;
use crate::core::{CMakeGenerator, DependencyAnalyzer, ModuleDiscoverer, ZigGenerator};
use crate::models::project::BuildSystem;
use crate::models::dependency::DependencyGraph;
use crate::models::module::SourceFile;
use crate::models::{
    BuildBackend, CMakeModule, DependencySnapshot, FbGenError, FbGenResult, ProjectConfig,
    ProjectMeta,
};
use serde_json;
use crate::orchestration::{FileWatcher, MetaCache, Reporter, UserQuery};
use crate::scanner::{self, FffScanner};

// ── static regexes ──────────────────────────────────────────────────────────

/// Regex for extracting source-file paths from CMakeLists.txt content.
/// Matches relative paths like `../../Core/Src/main.c` (the CubeMX convention)
/// and also bare relative paths like `Core/Src/main.c` or paths prefixed with
/// CMake variables like `${CMAKE_CURRENT_SOURCE_DIR}/../../Core/Src/main.c`.
static CMAKE_SOURCE_PATH_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?:(?:\.\./)+|(?:[a-zA-Z0-9_/.+-]+/)*[a-zA-Z0-9_/.+-]+)\.(?:c|cpp|cc|cxx|s|S)\b"#,
    )
    .expect("CMAKE_SOURCE_PATH_RE regex")
});

/// Regex for detecting `main()` function signatures in source files.
static MAIN_FUNCTION_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:int|void)\s+main\s*\(").expect("MAIN_FUNCTION_RE regex")
});

// ── helpers ────────────────────────────────────────────────────────────────

/// Build the scanner's `ScanOptions` from CLI flags and project config.
fn scanner_opts(cli: &Cli, config: &ProjectConfig) -> scanner::ScanOptions {
    let root = if cli.root == PathBuf::from(".") {
        config.root.clone()
    } else {
        cli.root.clone()
    };

    let languages = vec![
        "c".into(),
        "cpp".into(),
        "cc".into(),
        "cxx".into(),
        "c++".into(),
        "h".into(),
        "hpp".into(),
        "hh".into(),
        "hxx".into(),
        "h++".into(),
        "s".into(),
        "S".into(),
        "ld".into(),
    ];

    // Start with built-in defaults, then union in user-specified excludes.
    let mut exclude_dirs: Vec<String> = scanner::ScanOptions::default().exclude_dirs;
    let user_excludes = if cli.exclude.is_empty() {
        &config.exclude_dirs
    } else {
        &cli.exclude
    };
    for d in user_excludes {
        if !exclude_dirs.contains(d) {
            exclude_dirs.push(d.clone());
        }
    }

    scanner::ScanOptions {
        root,
        exclude_dirs,
        languages,
        follow_symlinks: false,
    }
}

/// Build the discoverer's `ScanOptions`.
fn discoverer_opts(cli: &Cli, config: &ProjectConfig) -> crate::core::ScanOptions {
    let root = if cli.root == PathBuf::from(".") {
        config.root.clone()
    } else {
        cli.root.clone()
    };

    // Start with built-in defaults, then union in user-specified excludes.
    let mut exclude_dirs: Vec<String> = crate::core::ScanOptions::default().exclude_dirs;
    let user_excludes = if cli.exclude.is_empty() {
        &config.exclude_dirs
    } else {
        &cli.exclude
    };
    for d in user_excludes {
        if !exclude_dirs.contains(d) {
            exclude_dirs.push(d.clone());
        }
    }

    crate::core::ScanOptions { root, exclude_dirs }
}






/// Run the full scan → discover → analyze pipeline. Returns modules + optional dep graph.
fn scan_and_discover(
    cli: &Cli,
    config: &ProjectConfig,
    reporter: &Reporter,
) -> FbGenResult<(Vec<CMakeModule>, Option<DependencyGraph>, Vec<PathBuf>)> {
    // ── Scan ──
    reporter.report_info(&format!(
        "Scanning sources in {} ...",
        config.root.display()
    ));
    let s_opts = scanner_opts(cli, config);
    let scanner = FffScanner::new(&s_opts.root);
    let sources = scanner.scan_source_files(&s_opts)?;
    reporter.report_success(&format!("Found {} source/header files", sources.len()));

    if sources.is_empty() {
        return Err(FbGenError::NoSources(config.root.display().to_string()));
    }

    // ── Discover ──
    reporter.report_info("Discovering modules ...");
    let d_opts = discoverer_opts(cli, config);
    let discoverer = ModuleDiscoverer::new(d_opts);
    let modules = discoverer.discover(&sources)?;
    reporter.report_success(&format!("Discovered {} modules", modules.len()));

    // ── Analyze (optional) ──
    let graph = if !cli.no_deps {
        reporter.report_info("Analyzing dependencies ...");
        let analyzer = DependencyAnalyzer::new();

        let graph = analyzer.analyze(&modules)?;
        let deps = graph.edge_count();
        if graph.has_cycles() {
            reporter.report_warning(
                "Dependency graph contains cycles — manual adjustment may be needed",
            );
        }
        reporter.report_success(&format!("Found {} dependencies", deps));
        Some(graph)
    } else {
        reporter.report_info("Dependency analysis skipped (--no-deps)");
        None
    };

    // ── Detect user-defined CMakeLists ──
    let user_modules = scanner.scan_user_cmake_files(&config.root, &config.exclude_dirs);

    // Filter out user modules whose source files overlap with fb-gen's
    // own modules (e.g. CubeMX cmake files that compile the same .c files).
    let user_modules = filter_overlapping_user_modules(user_modules, &modules, &config.root, reporter);
    if !user_modules.is_empty() {
        reporter.report_info(&format!(
            "Found {} user-defined CMake module(s)",
            user_modules.len()
        ));
    }

    Ok((modules, graph, user_modules))
}

/// Check that required external tools (cmake, compiler) are available on PATH.
/// Reports warnings for missing tools; never blocks the user.
fn check_required_tools(config: &ProjectConfig, reporter: &Reporter) {
    // ── build tool ───────────────────────────────────────────────────
    match config.build_system {
        BuildSystem::CMake => {
            match find_on_path("cmake") {
                Some(path) => reporter.report_info(&format!("cmake found: {}", path.display())),
                None => reporter.report_warning(
                    "cmake not found on PATH. Install it before running `fb-gen run`."
                ),
            }
        }
        BuildSystem::Zig => {
            match find_on_path("zig") {
                Some(path) => reporter.report_info(&format!("zig found: {}", path.display())),
                None => reporter.report_warning(
                    "zig not found on PATH. Install it before running `fb-gen run`."
                ),
            }
        }
    }

    // ── Compiler ──────────────────────────────────────────────────────
    use crate::models::project::{Compiler, TargetArch};
    let compiler_name = match &config.target_arch {
        TargetArch::NoneEabi | TargetArch::ARM32 | TargetArch::ARM64
        | TargetArch::RISCV64 | TargetArch::RISCV32 | TargetArch::Xtensa => {
            // Cross-compilation: check the toolchain prefix if configured.
            match &config.toolchain {
                Some(tc) if !tc.prefix.is_empty() => {
                    format!("{}gcc", tc.prefix)
                }
                _ => {
                    reporter.report_warning(
                        "Cross-compilation target selected but no toolchain prefix configured. \
                         Run `fb-gen init` again to configure the toolchain."
                    );
                    return;
                }
            }
        }
        _ => match &config.compiler {
            Compiler::GCC => "gcc".to_string(),
            Compiler::Clang => "clang".to_string(),
            Compiler::Zig => "zig".to_string(),
            Compiler::MSVC => "cl".to_string(),
            Compiler::Custom(ref name) => name.clone(),
        },
    };

    match find_on_path(&compiler_name) {
        Some(path) => reporter.report_info(&format!("compiler found: {} ({})", compiler_name, path.display())),
        None => reporter.report_warning(&format!(
            "Compiler '{}' not found on PATH.  Install it or add it to PATH before building.",
            compiler_name
        )),
    }
}

/// Search for an executable on `$PATH`.  On Windows also checks `.exe` suffix.
fn find_on_path(name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(name);
        if candidate.exists() {
            return Some(candidate);
        }
        // On Windows, try appending .exe.
        #[cfg(windows)]
        {
            let exe_candidate = dir.join(format!("{name}.exe"));
            if exe_candidate.exists() {
                return Some(exe_candidate);
            }
        }
    }
    None
}

// ── commands ───────────────────────────────────────────────────────────────

/// `fb-gen init` — interactive first-time project setup.
pub fn cmd_init(cli: &Cli, name: Option<&str>, build: Option<&str>) -> FbGenResult<()> {
    let reporter = Reporter::new(cli.quiet);

    // ── Collect config ──
    let root = if cli.root == PathBuf::from(".") {
        std::env::current_dir().map_err(|e| FbGenError::Config(format!("cwd: {e}")))?
    } else {
        cli.root.clone()
    };

    let mut config = UserQuery::ask_project_config(&root, build)?;

    // Override project name if given on the command line.
    if let Some(n) = name {
        config.name = n.to_string();
    }

    // Override from CLI flags.
    if !cli.exclude.is_empty() {
        config.exclude_dirs = cli.exclude.clone();
    }
    config.output_dir = cli.output.clone();
    config.enable_watch = cli.watch;

    if !UserQuery::confirm_config(&config) {
        reporter.report_warning("Aborted by user.");
        return Ok(());
    }

    // ── Environment checks ────────────────────────────────────────────
    check_required_tools(&config, &reporter);

    // ── Pipeline ──
    let start = Instant::now();
    let (modules, graph, user_modules) = scan_and_discover(cli, &config, &reporter)?;

    // ── Generate ──
    reporter.report_info(&format!(
        "Generating {} build files ...",
        match config.build_system { BuildSystem::CMake => "CMakeLists.txt", BuildSystem::Zig => "build.zig" }
    ));
    let empty_graph = DependencyGraph::new();
    let ref_graph = graph.as_ref().unwrap_or(&empty_graph);
    generate_build_files(&config, &modules, ref_graph, true, &user_modules)?;
    reporter.report_success("Build files generated");

    // ── Scan presets & toolchain files ──
    let scanner = FffScanner::new(&root);
    config.cmake_presets = scanner.scan_presets(&root)?;
    config.toolchain_files = scanner.scan_toolchain_files(&root, &config.exclude_dirs)?;
    if config.cmake_presets.is_some() {
        reporter.report_info("CMakePresets.json detected and parsed");
    }
    if !config.toolchain_files.is_empty() {
        reporter.report_info(&format!(
            "Found {} toolchain file(s)",
            config.toolchain_files.len()
        ));
    }

    // ── Device defines: switch presets to fb-gen's toolchain ────────
    if config.build_system == BuildSystem::CMake {
        let cmake_gen = CMakeGenerator::new(&config)?;
        ensure_device_defines_preset(&mut config, &cmake_gen, &root, &reporter)?;
    }

    // ── Cache ──
    save_meta_cache(&root, &modules, graph.as_ref(), &config)?;
    reporter.report_info("Metadata cached to .fb-gen/cache/");

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
        generate_compile_commands(&root, &config.output_dir, &config, &modules, &reporter);
    }

    Ok(())
}

/// `fb-gen sync` — incremental update using cached metadata.
///
/// Instead of a full re-scan, this uses `ProjectMeta` from `.fb-gen/cache/`
/// to detect exactly which files changed, then only re-processes affected modules.
///
/// With `--watch`, enters a polling loop that automatically re-syncs whenever
/// source files change (ctrl+c to stop).
pub fn cmd_sync(cli: &Cli) -> FbGenResult<()> {
    let reporter = Reporter::new(cli.quiet);
    let root = resolve_root(cli)?;

    let cache = MetaCache::new(&root);
    if !cache.exists() {
        reporter.report_warning("No cached metadata found. Run `fb-gen init` first.");
        return Ok(());
    }

    let mut prev_meta = cache
        .load()
        .ok_or_else(|| FbGenError::Config("Failed to load cached metadata".into()))?;

    let mut config = prev_meta.config.clone();

    // ── Initial sync ────────────────────────────────────────────────────
    let start = Instant::now();
    let n_affected = do_incremental_sync(&root, &mut config, &mut prev_meta, &reporter)?;

    if n_affected > 0 {
        save_sync_result(&cache, &config, &prev_meta)?;
        let elapsed = start.elapsed();
        reporter.report_success(&format!(
            "Sync done in {:.1}s — {} module(s) updated",
            elapsed.as_secs_f64(),
            n_affected
        ));
    } else {
        reporter.report_success("No changes detected — everything up to date.");
    }

    // ── LSP ──────────────────────────────────────────────────────────
    if cli.lsp {
        generate_compile_commands(&root, &config.output_dir, &config, &prev_meta.modules, &reporter);
    }

    // ── Watch loop ──────────────────────────────────────────────────────
    if !cli.watch {
        return Ok(());
    }

    reporter.report_info("Watching for file changes (ctrl+c to stop) ...");

    loop {
        std::thread::sleep(std::time::Duration::from_secs(2));

        // Re-load cached metadata (the previous iteration may have updated it).
        let mut prev_meta = match cache.load() {
            Some(m) => m,
            None => {
                reporter.report_warning("Failed to reload cached metadata — retrying ...");
                continue;
            }
        };
        let mut config = prev_meta.config.clone();

        // Quick check: are there any file changes?
        let watcher = FileWatcher::new(&root, config.exclude_dirs.clone());
        let changed = watcher.get_changes(&prev_meta.file_checksums);

        if changed.is_empty() {
            continue;
        }

        reporter.report_info(&format!(
            "{} file(s) changed — syncing ...",
            changed.len()
        ));

        match do_incremental_sync(&root, &mut config, &mut prev_meta, &reporter) {
            Ok(n) if n > 0 => {
                if let Err(e) = save_sync_result(&cache, &config, &prev_meta) {
                    reporter.report_warning(&format!("Failed to save metadata: {e}"));
                } else {
                    reporter.report_success(&format!("{} module(s) updated", n));
                }
            }
            Ok(_) => {
                // No modules affected (checksum-only changes like orphan cleanups).
                let _ = save_sync_result(&cache, &config, &prev_meta);
            }
            Err(e) => {
                reporter.report_warning(&format!("Sync error (will retry): {e}"));
            }
        }
    }
}

/// Persist sync results to the metadata cache.
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

    let meta = ProjectMeta {
        config: config.clone(),
        modules: prev_meta.modules.clone(),
        dependency_graph: dep_snapshot,
        file_checksums: prev_meta.file_checksums.clone(),
        last_sync: chrono::Utc::now().to_rfc3339(),
    };
    cache.save(&meta)
}

// ── compile_commands.json helpers (LSP support) ────────────────────────────

/// Run cmake configure to produce `compile_commands.json`, then symlink it
/// into the project root so LSP tools (clangd, ccls) find it automatically.
///
/// Failures are reported as warnings — they never block the primary command.
fn generate_compile_commands(
    root: &Path,
    build_dir: &Path,
    config: &ProjectConfig,
    modules: &[CMakeModule],
    reporter: &Reporter,
) {
    if config.build_system == BuildSystem::Zig {
        generate_compile_commands_zig(root, modules, reporter);
        return;
    }

    // Resolve the absolute build directory (config.output_dir may be relative).
    let abs_build_dir = if build_dir.is_absolute() {
        build_dir.to_path_buf()
    } else {
        root.join(build_dir)
    };

    // Ensure build directory exists.
    if let Err(e) = std::fs::create_dir_all(&abs_build_dir) {
        reporter.report_warning(&format!(
            "Cannot create build dir for compile_commands.json: {e}"
        ));
        return;
    }

    // Assemble cmake args: same flags as cmd_run uses for configure.
    let gen_flags = cmake_generator_flag(config);
    let toolchain_args = cmake_toolchain_args(config);

    let mut cmd = Command::new("cmake");
    cmd.arg("-S").arg(root).arg("-B").arg(&abs_build_dir);
    for f in &gen_flags {
        cmd.arg(f);
    }
    for f in &toolchain_args {
        cmd.arg(f);
    }
    cmd.arg("-DCMAKE_EXPORT_COMPILE_COMMANDS=ON");
    with_toolchain_path(&mut cmd, config);

    reporter.report_info(
        "Running cmake configure for compile_commands.json (--lsp) ..."
    );

    match cmd.output() {
        Ok(output) if output.status.success() => {
            let cc_json = abs_build_dir.join("compile_commands.json");
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
    // NOTE: is_symlink() is checked first because Path::exists() returns
    // false for dangling symlinks, so we'd skip the removal and hit EEXIST.
    if link.is_symlink() {
        if let Ok(existing) = std::fs::read_link(link) {
            if existing == target {
                return Ok(()); // already points to the right place
            }
        }
        std::fs::remove_file(link)?;
    } else if link.exists() {
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

// ── shared incremental sync core ──────────────────────────────────────────

/// Run the incremental sync pipeline against `prev_meta` and update it in
/// place.  Returns the number of affected modules (0 means nothing changed).
///
/// This is the shared implementation used by both `cmd_sync` and `cmd_run`.
fn do_incremental_sync(
    root: &Path,
    config: &mut ProjectConfig,
    prev_meta: &mut ProjectMeta,
    reporter: &Reporter,
) -> FbGenResult<usize> {
    // ── 1. Detect changes via checksum comparison ──
    let watcher = FileWatcher::new(root, config.exclude_dirs.clone());
    let changed_paths = watcher.get_changes(&prev_meta.file_checksums);


    if changed_paths.is_empty() {
        return Ok(0);
    }
    reporter.report_info(&format!("Detected {} changed file(s)", changed_paths.len()));

    // ── 2. Classify changes: added / modified / deleted ──
    let scanner = FffScanner::new(root);
    let cache = MetaCache::new(root);
    let mut modules = prev_meta.modules.clone();
    let mut includes_changed = false;
    let mut affected_modules: HashSet<String> = HashSet::new();
    let mut module_list_changed = false;
    let mut new_checksums: HashMap<String, String> = HashMap::new();
    let mut deleted_paths: Vec<PathBuf> = Vec::new();

    for path in &changed_paths {
        let key = path.to_string_lossy().to_string();
        let existed_before = prev_meta.file_checksums.contains_key(&key);

        if !path.exists() {
            // ── Deleted ──
            deleted_paths.push(path.clone());
            remove_file_from_modules(
                &mut modules,
                path,
                &mut affected_modules,
                &mut module_list_changed,
            );
        } else if is_c_cpp_file(path) {
            // ── Added or Modified ──
            let sf = match scanner.scan_single(path) {
                Ok(sf) => sf,
                Err(_) => continue,
            };

            // Track checksum
            let hash = cache.compute_checksums(std::slice::from_ref(path));
            new_checksums.extend(hash);

            if existed_before {
                // Modified: update existing SourceFile and check if includes changed
                let old_includes = find_old_includes(&modules, path);
                if old_includes
                    .as_ref()
                    .map(|oi| *oi != sf.includes)
                    .unwrap_or(true)
                {
                    includes_changed = true;
                }
                update_file_in_modules(&mut modules, sf, &mut affected_modules);
            } else {
                // Added: insert into appropriate module
                add_file_to_modules(
                    &mut modules,
                    sf,
                    root,
                    &config.exclude_dirs,
                    &mut affected_modules,
                    &mut module_list_changed,
                );
            }
        }
    }

    // Remove deleted paths from checksums (only when we're going to save).
    // Do this BEFORE the early-return check so orphaned checksums don't
    // cause infinite redetection — if we remove them here and return early,
    // they come back from the on-disk cache next time.  Instead, defer the
    // removal to *after* the early-return guard.
    if affected_modules.is_empty() && !module_list_changed {
        // Still remove orphaned checksums for deleted files that are no
        // longer tracked by any module.  Without this, the same deletion
        // is reported on every subsequent sync.
        let mut orphaned_removed = false;
        for dp in &deleted_paths {
            let key = dp.to_string_lossy().to_string();
            if prev_meta.file_checksums.remove(&key).is_some() {
                orphaned_removed = true;
            }
        }
        if orphaned_removed {
            // We mutated prev_meta — caller must persist.
            reporter.report_info("Cleaned up orphaned checksums for deleted files.");
            return Ok(0);
        }
        reporter.report_info("No modules affected by changes — skipping generation.");
        return Ok(0);
    }

    // Now it's safe to remove deleted checksums.
    for dp in &deleted_paths {
        prev_meta
            .file_checksums
            .remove(&dp.to_string_lossy().to_string());
    }

    let n_affected = affected_modules.len();
    reporter.report_info(&format!("{} module(s) affected", n_affected));

    // ── 3. Re-analyze dependencies only if includes changed ──
    let mut graph = rebuild_graph_from_snapshot(&prev_meta.dependency_graph);

    if includes_changed {
        reporter.report_info("Include dependencies changed — re-analyzing ...");
        let analyzer = DependencyAnalyzer::new();
        graph = analyzer.analyze(&modules)?;
        reporter.report_success(&format!("Found {} dependencies", graph.edge_count()));
    } else {
        reporter.report_info("Dependencies unchanged — skipping re-analysis.");
    }

    // ── 5. Regenerate CMakeLists.txt ──
    reporter.report_info("Regenerating affected build files ...");
    let user_modules = scanner.scan_user_cmake_files(root, &config.exclude_dirs);
    let user_modules = filter_overlapping_user_modules(user_modules, &modules, root, reporter);
    generate_build_files(config, &modules, &graph, true, &user_modules)?;
    reporter.report_success(&format!("{} module(s) updated", n_affected));

    // ── 6. Refresh presets & toolchain file list ──
    config.cmake_presets = scanner.scan_presets(root)?;
    config.toolchain_files = scanner.scan_toolchain_files(root, &config.exclude_dirs)?;

    // ── 7. Device defines: ensure fb-gen's toolchain is active ──
    if config.build_system == BuildSystem::CMake {
        let cmake_gen = CMakeGenerator::new(config)?;
        ensure_device_defines_preset(config, &cmake_gen, root, reporter)?;
    }

    // ── 8. Merge checksums and update prev_meta in place ──
    prev_meta.file_checksums.extend(new_checksums);
    prev_meta.modules = modules;
    prev_meta.dependency_graph = DependencySnapshot {
        nodes: prev_meta.modules.iter().map(|m| m.name.clone()).collect(),
        edges: prev_meta
            .modules
            .iter()
            .flat_map(|m| {
                graph
                    .get_dependencies(&m.name)
                    .into_iter()
                    .map(|(dep_name, _)| (m.name.clone(), dep_name))
            })
            .collect(),
    };

    Ok(n_affected)
}

/// When device defines are configured in the toolchain config, switch every
/// configure preset's `toolchainFile` to fb-gen's own `cmake/toolchain.cmake`
/// and force-generate that file (so the defines are baked into TARGET_FLAGS).
/// Persists the updated `CMakePresets.json` to disk.
fn ensure_device_defines_preset(
    config: &mut ProjectConfig,
    generator: &CMakeGenerator,
    root: &Path,
    reporter: &Reporter,
) -> FbGenResult<()> {
    let has_device_defines = config
        .toolchain
        .as_ref()
        .is_some_and(|tc| !tc.device_defines.is_empty());
    if !has_device_defines {
        return Ok(());
    }

    // If a user-owned toolchain file already exists at the target path,
    // warn before overwriting it.
    let toolchain_path = root.join("cmake").join("toolchain.cmake");
    if toolchain_path.exists() && generator.user_has_toolchain_file() {
        reporter.report_warning(
            "Device defines configured — fb-gen will replace your existing \
             cmake/toolchain.cmake. Consider backing it up first.",
        );
    }

    // Force-generate the toolchain file.  For non-embedded targets
    // (X86_64, X86, WASM, Custom) render_toolchain returns None and
    // force_generate_toolchain is a no-op.
    generator.force_generate_toolchain()?;

    if !toolchain_path.exists() {
        // The target architecture doesn't use a toolchain file.
        reporter.report_warning(
            "Device defines are configured but the target architecture \
             does not use a generated toolchain file. \
             Add the defines to your build flags manually.",
        );
    }

    // Switch configure presets to point at fb-gen's toolchain file.
    if let Some(ref mut presets) = config.cmake_presets {
        let mut switched = false;
        for cp in &mut presets.configure_presets {
            if cp.toolchain_file.is_some() {
                cp.toolchain_file = Some("${sourceDir}/cmake/toolchain.cmake".into());
                switched = true;
            }
        }
        if switched {
            reporter.report_info(
                "Toolchain preset switched to fb-gen toolchain.cmake (device defines configured)",
            );

            // Persist CMakePresets.json for IDE / cmake --preset consumers.
            let presets_path = root.join("CMakePresets.json");
            if let Ok(json) = serde_json::to_string_pretty(presets) {
                if let Err(e) = std::fs::write(&presets_path, json) {
                    reporter.report_warning(&format!(
                        "Failed to write updated CMakePresets.json: {}. \
                         Run `fb-gen init` again or update the file manually.",
                        e
                    ));
                }
            }
        }
    } else if toolchain_path.exists() {
        // No CMakePresets.json but the toolchain file was generated.
        // The device defines will still take effect because cmake
        // picks up CMAKE_TOOLCHAIN_FILE from CMakeLists.txt or the
        // command line.
        reporter.report_info(
            "Device defines written to cmake/toolchain.cmake. \
             Pass -DCMAKE_TOOLCHAIN_FILE=cmake/toolchain.cmake to cmake \
             if not already configured.",
        );
    }

    Ok(())
}

// ── incremental helpers ────────────────────────────────────────────────────

/// Check if a path is a C/C++ source or header file.
fn is_c_cpp_file(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    matches!(
        ext.as_str(),
        "c" | "cpp" | "cc" | "cxx" | "c++" | "h" | "hpp" | "hh" | "hxx" | "h++" | "s" | "ld"
    )
}

/// Find the old includes list for a file in the cached modules.
fn find_old_includes(modules: &[CMakeModule], path: &Path) -> Option<Vec<String>> {
    for m in modules {
        for sf in m
            .sources
            .iter()
            .chain(m.headers.iter())
            .chain(m.asm_sources.iter())
        {
            if sf.path == path {
                return Some(sf.includes.clone());
            }
        }
    }
    None
}

/// Update a modified SourceFile in-place within the module list.
fn update_file_in_modules(
    modules: &mut [CMakeModule],
    new_sf: SourceFile,
    affected: &mut HashSet<String>,
) {
    for m in modules.iter_mut() {
        // Search all containers; linker scripts are PathBuf (not SourceFile)
        // so a content change to a .ld file has no in-memory SourceFile to
        // update — the path itself didn't change.  Mark the module affected
        // anyway so the CMakeLists.txt is regenerated.
        let found = if new_sf.source_type.is_source() {
            m.sources.iter().position(|sf| sf.path == new_sf.path)
        } else if new_sf.source_type.is_header() {
            m.headers.iter().position(|sf| sf.path == new_sf.path)
        } else if new_sf.source_type.is_asm() {
            m.asm_sources.iter().position(|sf| sf.path == new_sf.path)
        } else if new_sf.source_type.is_linker() {
            // Linker script: just mark affected and return.
            if m.linker_scripts.iter().any(|p| p == &new_sf.path) {
                affected.insert(m.name.clone());
                return;
            }
            None
        } else {
            None
        };

        if let Some(pos) = found {
            if new_sf.source_type.is_source() {
                m.sources[pos] = new_sf;
            } else if new_sf.source_type.is_header() {
                m.headers[pos] = new_sf;
            } else if new_sf.source_type.is_asm() {
                m.asm_sources[pos] = new_sf;
            }
            affected.insert(m.name.clone());
            return;
        }
    }
}

/// Add a new SourceFile to the appropriate module, creating one if needed.
fn add_file_to_modules(
    modules: &mut Vec<CMakeModule>,
    sf: SourceFile,
    root: &Path,
    exclude_dirs: &[String],
    affected: &mut HashSet<String>,
    list_changed: &mut bool,
) {
    let parent = sf.relative_path.parent().unwrap_or_else(|| Path::new("."));

    // Check if parent dir is excluded.
    let excluded = parent.components().any(|c| {
        c.as_os_str()
            .to_str()
            .is_some_and(|s| exclude_dirs.iter().any(|d| d == s))
    });
    if excluded {
        return;
    }

    // Find existing module for this directory.
    let parent_buf = parent.to_path_buf();
    if let Some(m) = modules.iter_mut().find(|m| m.relative_path == parent_buf) {
        if sf.source_type.is_source() {
            m.sources.push(sf);
            // If the newly-added source contains main(), promote the module
            // to Executable (e.g. user added main.c to an existing library).
            if !m.has_main && source_has_main(m.sources.last().unwrap()) {
                m.has_main = true;
                m.target_type = crate::models::module::TargetType::Executable;
            }
        } else if sf.source_type.is_header() {
            m.headers.push(sf);
        } else if sf.source_type.is_asm() {
            m.asm_sources.push(sf);
        } else if sf.source_type.is_linker() {
            // Linker scripts are tracked as PathBuf, not SourceFile.
            m.linker_scripts.push(sf.path.clone());
        }
        // Note: SourceType::Other files are intentionally skipped — they
        // represent unrecognised extensions that the scanner included in
        // the file list but that have no meaningful role in a CMake module.
        affected.insert(m.name.clone());
    } else {
        // Create a new module for this directory.
        let module_name = CMakeModule::sanitize_name(parent);
        let has_main = sf.source_type.is_source() && source_has_main(&sf);
        let target_type = if has_main {
            crate::models::module::TargetType::Executable
        } else {
            crate::models::module::TargetType::StaticLibrary
        };

        let mut new_module = CMakeModule {
            name: module_name.clone(),
            path: root.join(parent),
            relative_path: parent_buf,
            sources: vec![],
            headers: vec![],
            asm_sources: vec![],
            linker_scripts: vec![],
            dependencies: vec![],
            target_type,
            is_root: parent == Path::new(".") || parent.as_os_str().is_empty(),
            has_main,
            compile_features: vec![],
            compile_definitions: vec![],
            include_dirs: vec![parent.to_path_buf()],
        };

        if sf.source_type.is_source() {
            new_module.sources.push(sf);
        } else if sf.source_type.is_header() {
            new_module.headers.push(sf);
        } else if sf.source_type.is_asm() {
            new_module.asm_sources.push(sf);
        } else if sf.source_type.is_linker() {
            new_module.linker_scripts.push(sf.path.clone());
        }

        modules.push(new_module);
        affected.insert(module_name);
        *list_changed = true;
    }
}

/// Remove a deleted file from its module. If the module becomes empty, remove it entirely.
fn remove_file_from_modules(
    modules: &mut Vec<CMakeModule>,
    path: &Path,
    affected: &mut HashSet<String>,
    list_changed: &mut bool,
) {
    for i in (0..modules.len()).rev() {
        let m = &mut modules[i];
        let old_src_len = m.sources.len();
        let old_hdr_len = m.headers.len();
        let old_asm_len = m.asm_sources.len();
        let old_ld_len = m.linker_scripts.len();

        m.sources.retain(|sf| sf.path != path);
        m.headers.retain(|sf| sf.path != path);
        m.asm_sources.retain(|sf| sf.path != path);
        m.linker_scripts.retain(|p| p != path);

        if m.sources.len() != old_src_len
            || m.headers.len() != old_hdr_len
            || m.asm_sources.len() != old_asm_len
            || m.linker_scripts.len() != old_ld_len
        {
            affected.insert(m.name.clone());
        }

        // Remove module if it became empty (check all containers).
        if m.sources.is_empty()
            && m.headers.is_empty()
            && m.asm_sources.is_empty()
            && m.linker_scripts.is_empty()
        {
            modules.remove(i);
            *list_changed = true;
        }
    }
}

/// Quick check if a SourceFile contains a main() function.
fn source_has_main(sf: &SourceFile) -> bool {
    if let Ok(content) = std::fs::read_to_string(&sf.path) {
        MAIN_FUNCTION_RE.is_match(&content)
    } else {
        false
    }
}

/// Rebuild a DependencyGraph from a cached DependencySnapshot.
fn rebuild_graph_from_snapshot(snap: &DependencySnapshot) -> DependencyGraph {
    let mut graph = DependencyGraph::new();
    for node in &snap.nodes {
        graph.add_module(node);
    }
    for (from, to) in &snap.edges {
        graph.add_dependency(crate::models::dependency::DependencyEdge {
            from: from.clone(),
            to: to.clone(),
            dep_type: crate::models::dependency::DependencyType::Private,
        });
    }
    graph
}

/// Save project metadata to cache (shared by init, sync, and run).
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

    let meta = ProjectMeta {
        config: config.clone(),
        modules: modules.to_vec(),
        dependency_graph: dep_snapshot,
        file_checksums: checksums,
        last_sync: chrono::Utc::now().to_rfc3339(),
    };
    cache.save(&meta)
}

/// `fb-gen check` — compare generated CMake against existing files.
pub fn cmd_check(cli: &Cli) -> FbGenResult<()> {
    let reporter = Reporter::new(cli.quiet);
    let root = resolve_root(cli)?;

    // Load config from cache when available so the generated output matches
    // what `init` / `sync` produce (language, standards, architecture, …).
    // Fall back to a minimal config if no cache exists.
    let config = MetaCache::new(&root)
        .load()
        .map(|m| m.config)
        .unwrap_or_else(|| ProjectConfig {
            name: root
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("project")
                .to_string(),
            root: root.clone(),
            exclude_dirs: vec!["build".into(), ".git".into(), "third_party".into()],
            ..Default::default()
        });

    let (modules, graph, user_modules) = scan_and_discover(cli, &config, &reporter)?;

    // Generate into memory (write to temp location, then compare).
    let tmp_dir =
        tempfile::TempDir::new().map_err(|e| FbGenError::Config(format!("tempdir: {e}")))?;
    let tmp_root = tmp_dir.path();

    // Generate root CMakeLists.txt to temp.
    let mut tmp_config = config.clone();
    tmp_config.root = tmp_root.to_path_buf();

    // Mirror module directory structure under temp so the generator writes correctly.
    for m in &modules {
        if !m.is_root {
            let dest = tmp_root.join(&m.relative_path);
            std::fs::create_dir_all(&dest).map_err(FbGenError::Io)?;
        }
    }

    let empty_graph = DependencyGraph::new();
    let ref_graph = graph.as_ref().unwrap_or(&empty_graph);
    generate_build_files(&tmp_config, &modules, ref_graph, false, &user_modules)?;
    let mut diffs = 0usize;

    if config.build_system == BuildSystem::Zig {
        let gen = tmp_root.join("build.zig");
        let real = root.join("build.zig");
        diffs += diff_file(&gen, &real, "build.zig", &reporter);
    } else {
        // Root CMakeLists.txt
        let gen_root = tmp_root.join("CMakeLists.txt");
        let real_root = root.join("CMakeLists.txt");
        diffs += diff_file(&gen_root, &real_root, "CMakeLists.txt (root)", &reporter);

        // Per-module CMakeLists.txt
        for m in &modules {
            if m.is_root {
                continue;
            }
            let gen = tmp_root.join(&m.relative_path).join("CMakeLists.txt");
            let real = root.join(&m.relative_path).join("CMakeLists.txt");
            let label = format!("CMakeLists.txt ({})", m.name);
            diffs += diff_file(&gen, &real, &label, &reporter);
        }
    }

    if diffs == 0 {
        reporter.report_success("All build files are in sync with project structure.");
    } else {
        reporter.report_warning(&format!("{} file(s) differ from generated output.", diffs));
    }

    Ok(())
}

/// `fb-gen validate` — run cmake to verify generated configuration.
pub fn cmd_validate(cli: &Cli) -> FbGenResult<()> {
    let reporter = Reporter::new(cli.quiet);
    let root = resolve_root(cli)?;

    let build_dir = root.join(&cli.output);
    std::fs::create_dir_all(&build_dir).map_err(FbGenError::Io)?;

    let config = load_or_default_config(&root, &cli.output);

    if config.build_system == BuildSystem::Zig {
        reporter.report_info("Zig projects are validated by `zig build` — skipping cmake.");
        return Ok(());
    }

    let gen_flags = cmake_generator_flag(&config);
    let toolchain_args = cmake_toolchain_args(&config);

    // If cross-compiling, nuke stale cache so the toolchain file is honoured.
    if !toolchain_args.is_empty() {
        let cache_file = build_dir.join("CMakeCache.txt");
        if cache_file.exists() {
            std::fs::remove_file(&cache_file).ok();
        }
    }

    let gen_str = if gen_flags.is_empty() {
        String::new()
    } else {
        format!(" -G {}", gen_flags[1])
    };
    reporter.report_info(&format!(
        "Running cmake -S {} -B {}{} ...",
        root.display(),
        build_dir.display(),
        gen_str
    ));

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

    Ok(())
}

/// `fb-gen run` — full build pipeline (generate + cmake --build).
pub fn cmd_run(cli: &Cli) -> FbGenResult<()> {
    let reporter = Reporter::new(cli.quiet);

    // ── CMAKE banner ──────────────────────────────────────────────────
    // Gradient: bright-cyan → cyan → blue → bright-green → green
    if !cli.quiet {
        let banner: [(&str, Color); 6] = [
            (" ████   █    █   ██████", Color::BrightCyan),
            ("█    █  ██  ██      █",    Color::Cyan),
            ("█       █ ██ █     █",    Color::BrightBlue),
            ("█       █    █    █",     Color::Blue),
            ("█    █  █    █   █",      Color::BrightGreen),
            (" ████   █    █   ██████", Color::Green),
        ];
        for (line, color) in &banner {
            println!("{}", line.color(*color).bold());
        }
        println!();
    }

    // Ensure CMakeLists.txt are up to date first.
    let root = resolve_root(cli)?;
    let cache = MetaCache::new(&root);

    let fallback_config = || ProjectConfig {
        name: root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("project")
            .to_string(),
        root: root.clone(),
        output_dir: cli.output.clone(),
        exclude_dirs: vec!["build".into(), ".git".into(), "third_party".into()],
        ..Default::default()
    };

    let mut config = if cache.exists() {
        cache
            .load()
            .map(|m| m.config)
            .unwrap_or_else(fallback_config)
    } else {
        fallback_config()
    };

    let gen_flags = cmake_generator_flag(&config);

    // Ensure build files are up to date.
    let build_file = match config.build_system {
        BuildSystem::CMake => root.join("CMakeLists.txt"),
        BuildSystem::Zig => root.join("build.zig"),
    };
    if !build_file.exists() {
        // First time: full generation.
        reporter.report_info(&format!("No {} found — generating ...", build_file.file_name().unwrap_or_default().to_string_lossy()));
        let (modules, graph, user_modules) = scan_and_discover(cli, &config, &reporter)?;

        let empty_graph = DependencyGraph::new();
        let ref_graph = graph.as_ref().unwrap_or(&empty_graph);
        generate_build_files(&config, &modules, ref_graph, true, &user_modules)?;
        // Save cache for future incremental syncs.
        save_meta_cache(&root, &modules, graph.as_ref(), &config)?;
    } else if cache.exists() {
        // CMakeLists.txt exists and we have cached metadata — run an
        // incremental sync so that any source-file changes since the last
        // init/sync are reflected before configure + build.
        reporter.report_info("Checking for source changes ...");
        match cache.load() {
            Some(mut prev_meta) => {
                match do_incremental_sync(&root, &mut config, &mut prev_meta, &reporter) {
                    Ok(n) if n > 0 => {
                        // Persist updated metadata (hash was already
                        // computed and stored by do_incremental_sync).
                        let meta = ProjectMeta {
                            config: config.clone(),
                            modules: prev_meta.modules,
                            dependency_graph: prev_meta.dependency_graph,
                            file_checksums: prev_meta.file_checksums,
                            last_sync: chrono::Utc::now().to_rfc3339(),
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
                    Ok(_) => {
                        reporter.report_info("No source changes — skipping sync");
                    }
                    Err(e) => {
                        // Sync failure shouldn't block the build — the
                        // existing CMakeLists.txt may still be valid.
                        reporter.report_warning(&format!(
                            "Incremental sync failed: {}. Proceeding with existing CMakeLists.txt.",
                            e
                        ));
                    }
                }
            }
            None => {
                reporter.report_warning(
                    "Cache exists but failed to load — skipping sync, proceeding with build.",
                );
            }
        }
    }

    // Configure.
    let build_dir = root.join(&cli.output);
    std::fs::create_dir_all(&build_dir).map_err(FbGenError::Io)?;

    let toolchain_args = cmake_toolchain_args(&config);

    // If cross-compiling with a toolchain file, remove stale cache leftover
    // from a previous configure that ran without one.  Otherwise cmake
    // ignores the new -DCMAKE_TOOLCHAIN_FILE= and keeps the host compiler.
    if !toolchain_args.is_empty() {
        let cache_file = build_dir.join("CMakeCache.txt");
        if cache_file.exists() {
            let cache_stale = std::fs::read_to_string(&cache_file)
                .map(|contents| {
                    !toolchain_args.iter().any(|a| {
                        if let Some(path) = a.strip_prefix("-DCMAKE_TOOLCHAIN_FILE=") {
                            contents.contains(path)
                        } else {
                            contents.contains(a.as_str())
                        }
                    })
                })
                .unwrap_or(true);
            if cache_stale {
                reporter.report_info(
                    "Stale CMake cache detected — removing for clean toolchain configure.",
                );
                let _ = std::fs::remove_file(&cache_file);
            }
        }
    }

    let start = Instant::now();
    let (status, build_lines) = match config.build_system {
        BuildSystem::CMake => {
            let gen_str = if gen_flags.is_empty() {
                String::new()
            } else {
                format!(" -G {}", gen_flags[1])
            };
            reporter.report_info(&format!(
                "Configuring with cmake -S {} -B {}{} ...",
                root.display(),
                build_dir.display(),
                gen_str
            ));

            let mut cmd = Command::new("cmake");
            cmd.arg("-S").arg(&root).arg("-B").arg(&build_dir);
            for f in &gen_flags {
                cmd.arg(f);
            }
            for f in &toolchain_args {
                cmd.arg(f);
            }
            with_toolchain_path(&mut cmd, &config);
            let (s, _cfg_lines) = run_cmake_formatted(&mut cmd, cli.quiet)
                .map_err(|e| FbGenError::Config(format!("cmake: {e}")))?;
            if !s.success() {
                return Err(FbGenError::GenerationFailed("cmake configure failed".into()));
            }

            // ── LSP symlink ──────────────────────────────────────────

            // Build.
            reporter.report_info(&format!(
                "Building with cmake --build {} ...",
                build_dir.display()
            ));
            let mut build_cmd = Command::new("cmake");
            build_cmd.arg("--build").arg(&build_dir);
            with_toolchain_path(&mut build_cmd, &config);
            run_cmake_formatted(&mut build_cmd, cli.quiet)
                .map_err(|e| FbGenError::Config(format!("cmake --build: {e}")))?
        }
        BuildSystem::Zig => {
            reporter.report_info("Building with zig build ...");
            let mut build_cmd = Command::new("zig");
            build_cmd.arg("build").current_dir(&root);
            let (s, lines) = run_cmake_formatted(&mut build_cmd, cli.quiet)
                .map_err(|e| FbGenError::Config(format!("zig build: {e}")))?;
            (s, lines)
        }
    };

    if status.success() {
        let elapsed = start.elapsed();
        reporter.report_success(&format!("Build succeeded in {:.1}s", elapsed.as_secs_f64()));

        // Highlight memory/flash usage summary, if present.
        if let Some(summary) = extract_memory_summary(&build_lines) {
            if !cli.quiet {
                println!(
                    "\n{}",
                    "──── Memory Usage ────".cyan().bold()
                );
                for line in summary.lines() {
                    println!("  {}", format_cmake_line(line));
                }
            }
        }
    } else {
        reporter.report_error("Build failed.");
        return Err(FbGenError::GenerationFailed("build failed".into()));
    }

    Ok(())
}

/// Filter out user CMake modules whose source files overlap with fb-gen's
/// own modules.  Prevents duplicate compilation when a user-provided cmake
/// file (e.g. CubeMX) references the same `.c` / `.cpp` / `.s` files that
/// fb-gen already discovered.
fn filter_overlapping_user_modules(
    user_modules: Vec<PathBuf>,
    modules: &[CMakeModule],
    root: &Path,
    reporter: &Reporter,
) -> Vec<PathBuf> {
    // Build a set of all file paths covered by fb-gen modules.
    // Canonicalise both sides so symlinks don't break the comparison.
    let canonicalize = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());

    let fb_files: std::collections::HashSet<PathBuf> = modules
        .iter()
        .flat_map(|m| {
            m.sources
                .iter()
                .map(|s| canonicalize(&s.path))
                .chain(m.asm_sources.iter().map(|s| canonicalize(&s.path)))
        })
        .collect();

    if fb_files.is_empty() {
        return user_modules;
    }

    user_modules
        .into_iter()
        .filter(|um| {
            let cmake_path = root.join(um).join("CMakeLists.txt");
            let content = match std::fs::read_to_string(&cmake_path) {
                Ok(c) => c,
                Err(e) => {
                    reporter.report_warning(&format!(
                        "Cannot read user CMakeLists.txt '{}': {}. Keeping module.",
                        cmake_path.display(),
                        e
                    ));
                    return true; // can't read → keep it (preserve existing behaviour)
                }
            };

            // Collect source paths referenced in this cmake file.
            for cap in CMAKE_SOURCE_PATH_RE.captures_iter(&content) {
                if let Some(m) = cap.get(1) {
                    let rel_path = m.as_str();
                    // Resolve relative to the cmake file's directory.
                    let resolved = canonicalize(&root.join(um).join(rel_path));

                    if fb_files.contains(&resolved) {
                        reporter.report_info(&format!(
                            "Skipping user CMake module '{}' — sources overlap with fb-gen modules",
                            um.display()
                        ));
                        return false; // overlap → exclude this user module
                    }
                }
            }

            true
        })
        .collect()
}

// ── CMake output formatting ─────────────────────────────────────────────────

/// Format a single line of CMake build output with color based on its content.
fn format_cmake_line(line: &str) -> String {
    // Memory table header (arm-none-eabi-size output):
    //   text    data     bss     dec     hex filename
    if line.contains("text")
        && line.contains("data")
        && line.contains("bss")
        && line.contains("dec")
        && line.contains("hex")
    {
        return line.cyan().bold().to_string();
    }

    // Memory table data: whitespace-prefixed lines with 5+ numeric columns.
    let tokens: Vec<&str> = line.split_whitespace().collect();
    if tokens.len() >= 5
        && tokens[0].parse::<u64>().is_ok()
        && tokens[1].parse::<u64>().is_ok()
    {
        return line.cyan().to_string();
    }

    // Custom memory region table (STM32CubeMX / GNU ld --print-memory-usage):
    // Memory region         Used Size  Region Size  %age Used
    if line.contains("Memory region") && line.contains("Used Size") {
        return line.cyan().bold().to_string();
    }

    // Errors — highest visibility
    if line.contains("error:")
        || line.contains("Error:")
        || line.contains("FAILED:")
        || line.contains("ninja: build stopped:")
    {
        return line.red().bold().to_string();
    }

    // Warnings
    if line.contains("warning:") || line.contains("Warning:") {
        return line.yellow().to_string();
    }

    // Build progress: lines like "[42/128] Building C object ..." (Ninja)
    let trimmed = line.trim_start();
    if trimmed.starts_with('[') && trimmed.contains(']') && trimmed.contains('/') {
        return line.dimmed().to_string();
    }

    // Linking / success
    if line.contains("Linking")
        || line.contains("Built target")
        || line == "Build succeeded."
    {
        return line.green().to_string();
    }

    // Normal lines: pass through unchanged
    line.to_string()
}

/// Run a CMake command, piping stdout/stderr through the color formatter.
///
/// Streams lines in real-time while also collecting all lines into a
/// `Vec<String>` for post-processing (e.g. memory-usage extraction).
fn run_cmake_formatted(
    cmd: &mut Command,
    quiet: bool,
) -> std::io::Result<(ExitStatus, Vec<String>)> {
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let stdout = child.stdout.take().expect("stdout pipe configured");
    let stderr = child.stderr.take().expect("stderr pipe configured");

    let all_lines: std::sync::Arc<std::sync::Mutex<Vec<String>>> =
        std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let stderr_lines = std::sync::Arc::clone(&all_lines);

    // Spawn a thread to read stderr; prevents deadlock when the child
    // fills the stderr pipe buffer while the parent reads stdout.
    let stderr_handle = thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for result in reader.lines() {
            if let Ok(line) = result {
                if !quiet {
                    let formatted = format_cmake_line(&line);
                    eprintln!("{formatted}");
                }
                stderr_lines
                    .lock()
                    .expect("lock stderr")
                    .push(line);
            }
        }
    });

    // Read stdout in the main thread.
    {
        let reader = BufReader::new(stdout);
        for result in reader.lines() {
            if let Ok(line) = result {
                if !quiet {
                    let formatted = format_cmake_line(&line);
                    println!("{formatted}");
                }
                all_lines
                    .lock()
                    .expect("lock stdout")
                    .push(line);
            }
        }
    }

    // Wait for the child to exit, which guarantees both write ends of
    // the pipes are closed and the stderr thread will finish.
    let status = child.wait()?;

    // Join the stderr reader thread (should return promptly now).
    stderr_handle.join().expect("stderr thread panicked");

    let lines = std::sync::Arc::into_inner(all_lines)
        .expect("Arc refcount is 1")
        .into_inner()
        .expect("mutex not poisoned");

    Ok((status, lines))
}

/// Extract memory/flash usage summary lines from captured build output.
fn extract_memory_summary(lines: &[String]) -> Option<String> {
    let mut result: Vec<&str> = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        let line = &lines[i];

        // arm-none-eabi-size format:
        //   text    data     bss     dec     hex filename
        //   12344     56    7890   20290   4f42 firmware.elf
        if line.contains("text")
            && line.contains("data")
            && line.contains("bss")
            && line.contains("dec")
            && line.contains("hex")
        {
            result.push(line.as_str());
            // Grab the data line immediately below, if it exists.
            if i + 1 < lines.len() && !lines[i + 1].trim().is_empty() {
                result.push(lines[i + 1].as_str());
            }
            i += 2;
            continue;
        }

        // Custom memory region table (STM32CubeMX style).
        if line.starts_with("Memory region") && line.contains("Used Size") {
            result.push(line.as_str());
            // Collect all following lines that look like data rows.
            for j in (i + 1)..lines.len() {
                let row = &lines[j];
                let trimmed = row.trim_start();
                if trimmed.starts_with("FLASH:")
                    || trimmed.starts_with("RAM:")
                    || trimmed.starts_with("SRAM:")
                    || trimmed.starts_with("CCMRAM:")
                    || trimmed.starts_with("BKPSRAM:")
                {
                    result.push(row.as_str());
                } else if row.trim().is_empty() {
                    break;
                } else {
                    // Stop at first non-data, non-empty line.
                    break;
                }
            }
            break;
        }

        i += 1;
    }

    if result.is_empty() {
        None
    } else {
        Some(result.join("\n"))
    }
}

// ── internal helpers ───────────────────────────────────────────────────────

fn resolve_root(cli: &Cli) -> FbGenResult<PathBuf> {
    if cli.root == PathBuf::from(".") {
        std::env::current_dir().map_err(|e| FbGenError::Config(format!("cwd: {e}")))
    } else {
        Ok(cli.root.clone())
    }
}

/// Compare a generated file against the real one; report diffs.
fn diff_file(generated: &Path, real: &Path, label: &str, reporter: &Reporter) -> usize {
    let gen_content = match std::fs::read_to_string(generated) {
        Ok(c) => c,
        Err(_) => {
            reporter.report_warning(&format!("Cannot read generated {label}"));
            return 0;
        }
    };

    if !real.exists() {
        reporter.report_warning(&format!("{label} does not exist (would be created)"));
        return 1;
    }

    let real_content = match std::fs::read_to_string(real) {
        Ok(c) => c,
        Err(_) => {
            reporter.report_warning(&format!("Cannot read {label}"));
            return 0;
        }
    };

    if gen_content.trim() != real_content.trim() {
        reporter.report_warning(&format!("{label} differs from generated output."));

        // Print a simple unified diff.
        if !reporter.is_quiet() {
            print_diff(label, &real_content, &gen_content);
        }
        1
    } else {
        0
    }
}

/// Print a simple line-by-line diff.
fn print_diff(label: &str, old: &str, new: &str) {
    println!("--- {} (existing)", label);
    println!("+++ {} (generated)", label);

    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    let max_len = old_lines.len().max(new_lines.len());

    for i in 0..max_len {
        let o = old_lines.get(i);
        let n = new_lines.get(i);
        if o != n {
            if let Some(line) = o {
                println!("-{}", line);
            }
            if let Some(line) = n {
                println!("+{}", line);
            }
        }
    }
}

/// Extend a `Command`'s `PATH` so the cross-compilation toolchain is
/// discoverable.  No-op for native (x86) targets or when the toolchain
/// directory can't be resolved.
fn with_toolchain_path(cmd: &mut Command, config: &ProjectConfig) {
    if config.toolchain.is_none() {
        return;
    }
    // Try to locate the compiler binary from the sysroot.
    let dirs = toolchain_bin_dirs(config);
    if dirs.is_empty() {
        return;
    }
    let current_path = std::env::var_os("PATH").unwrap_or_default();
    let mut new_path = std::ffi::OsString::new();
    for d in &dirs {
        new_path.push(d);
        new_path.push(":");
    }
    new_path.push(&current_path);
    cmd.env("PATH", new_path);
}

/// Return candidate directories that contain the cross-compiler for the
/// project's configured toolchain.
fn toolchain_bin_dirs(config: &ProjectConfig) -> Vec<std::path::PathBuf> {
    let tc = match config.toolchain.as_ref() {
        Some(t) => t,
        None => return vec![],
    };
    let mut dirs: Vec<std::path::PathBuf> = Vec::new();

    // Derive bin/ from sysroot (sysroot is typically at .../<arch>/bin/../<arch>
    // or similar, so sysroot.parent() often points to the directory containing bin/).
    if let Some(ref sysroot) = tc.sysroot {
        let sysroot_path = std::path::Path::new(sysroot);
        // Normalise: resolve ".." components.
        if let Ok(canon) = sysroot_path.canonicalize() {
            if let Some(parent) = canon.parent() {
                let bin = parent.join("bin");
                if bin.exists() {
                    dirs.push(bin);
                }
            }
        }
        // Also try the raw path's parent.
        if let Some(parent) = sysroot_path.parent() {
            let bin = parent.join("bin");
            if bin.exists() && !dirs.contains(&bin) {
                dirs.push(bin);
            }
        }
    }

    dirs
}

/// Map BuildBackend to cmake -G argument(s).
fn cmake_generator_flag(config: &ProjectConfig) -> Vec<String> {
    match config.build_backend {
        BuildBackend::Ninja => vec!["-G".into(), "Ninja".into()],
        BuildBackend::Make => vec![], // use system default (Unix Makefiles)
        BuildBackend::MSBuild => vec!["-G".into(), "Visual Studio 17 2022".into()],
        BuildBackend::Custom(ref name) => vec!["-G".into(), name.clone()],
    }
}

/// Returns `-DCMAKE_TOOLCHAIN_FILE=...` args for cross-compilation targets.
fn cmake_toolchain_args(config: &ProjectConfig) -> Vec<String> {
    use crate::models::project::TargetArch;
    let is_cross = !matches!(
        config.target_arch,
        TargetArch::X86_64 | TargetArch::X86 | TargetArch::Custom(_)
    );
    if !is_cross {
        return vec![];
    }

    // Prefer path from CMakePresets.
    let path = config
        .cmake_presets
        .as_ref()
        .and_then(|p| {
            p.configure_presets
                .iter()
                .find_map(|cp| cp.toolchain_file.as_ref())
        })
        .map(|tf| CMakeGenerator::resolve_preset_path(&config.root, tf))
        .unwrap_or_else(|| config.root.join("cmake").join("toolchain.cmake"));

    if path.exists() {
        vec![format!("-DCMAKE_TOOLCHAIN_FILE={}", path.display())]
    } else {
        vec![]
    }
}

/// Dispatch generate to CMake or Zig based on `config.build_system`.
fn generate_build_files(
    config: &ProjectConfig,
    modules: &[CMakeModule],
    graph: &DependencyGraph,
    force: bool,
    user_modules: &[PathBuf],
) -> FbGenResult<()> {
    match config.build_system {
        BuildSystem::CMake => {
            let generator = CMakeGenerator::new(config)?;
            generator.generate(modules, graph, force, user_modules)
        }
        BuildSystem::Zig => {
            let generator = ZigGenerator::new(config)?;
            generator.generate(modules, graph, force, user_modules)
        }
    }
}

/// Generate `compile_commands.json` for a Zig project using the project's
/// own module metadata (source files, include dirs, compile definitions).
/// Produces clang-compatible `zig cc` commands that clangd can parse.
fn generate_compile_commands_zig(
    root: &Path,
    modules: &[CMakeModule],
    reporter: &Reporter,
) {
    reporter.report_info("Generating compile_commands.json from module metadata ...");

    let mut entries: Vec<serde_json::Value> = Vec::new();
    let mut seen_dirs = std::collections::HashSet::new();

    for m in modules {
        // Collect -I flags as a single string (deduped, skipping empty / zig-cache).
        seen_dirs.clear();
        let mut inc_flags = String::new();
        let own_dir = m.relative_path.to_string_lossy().to_string();
        if !own_dir.is_empty() && !own_dir.contains(".zig-cache") && seen_dirs.insert(own_dir.clone()) {
            inc_flags.push_str(&format!(" -I {}", own_dir));
        }
        for inc in &m.include_dirs {
            let d = inc.to_string_lossy().to_string();
            if !d.is_empty() && !d.contains(".zig-cache") && seen_dirs.insert(d.clone()) {
                inc_flags.push_str(&format!(" -I {}", d));
            }
        }

        // Collect -D flags
        let mut def_flags = String::new();
        for def in &m.compile_definitions {
            def_flags.push_str(&format!(" -D {}", def));
        }

        for src in &m.sources {
            if !src.source_type.is_source() {
                continue;
            }
            let file_path = src.path.to_string_lossy().to_string();
            let file_rel = src.relative_path.to_string_lossy().to_string();

            let command = format!(
                "zig cc{} {} -c {} -o {}",
                inc_flags,
                def_flags,
                file_rel,
                file_rel.replace('/', "_").replace(".cpp", ".o").replace(".c", ".o"),
            );

            entries.push(serde_json::json!({
                "directory": root.to_string_lossy(),
                "file": file_path,
                "command": command,
            }));
        }
    }

    let cc_json = serde_json::json!(entries);
    match serde_json::to_string_pretty(&cc_json) {
        Ok(json_str) => {
            let dest = root.join("compile_commands.json");
            if let Err(e) = std::fs::write(&dest, &json_str) {
                reporter.report_warning(&format!("Failed to write compile_commands.json: {e}"));
            } else {
                reporter.report_success(&format!(
                    "compile_commands.json written ({} entries)",
                    entries.len()
                ));
            }
        }
        Err(e) => {
            reporter.report_warning(&format!("Failed to serialize compile_commands.json: {e}"));
        }
    }
}
/// Load config from cache, or return a default (for validate).
fn load_or_default_config(root: &Path, output_dir: &Path) -> ProjectConfig {
    MetaCache::new(root)
        .load()
        .map(|m| m.config)
        .unwrap_or_else(|| ProjectConfig {
            name: root
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("project")
                .into(),
            root: root.to_path_buf(),
            output_dir: output_dir.to_path_buf(),
            exclude_dirs: vec!["build".into(), ".git".into(), "third_party".into()],
            ..Default::default()
        })
}

// ── tests ──────────────────────────────────────────────────────────────────
// ── tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod cmake_format_tests {
    use super::*;

    /// Wrapper that forces `colored` to emit ANSI codes even when stdout is
    /// not a terminal (e.g. inside `cargo test`).
    ///
    /// Uses a mutex to prevent race conditions on `colored`'s global state
    /// when tests run in parallel.
    fn format_colored(line: &str) -> String {
        static MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = MUTEX.lock().expect("format_colored lock");
        colored::control::set_override(true);
        let result = format_cmake_line(line);
        colored::control::set_override(false);
        result
    }

    // ── Colored output: result differs from input ────────────────────

    #[test]
    fn error_lines_are_colored() {
        let line = "error: 'foo' was not declared";
        let result = format_colored(line);
        assert_ne!(result, line, "error lines should be colored");
        assert!(result.contains("error:"), "should still contain original text");
    }

    #[test]
    fn warning_lines_are_colored() {
        let line = "warning: unused variable 'x'";
        let result = format_colored(line);
        assert_ne!(result, line, "warnings should be colored");
    }

    #[test]
    fn progress_lines_are_colored() {
        let line = "[42/128] Building C object foo.c.obj";
        let result = format_colored(line);
        assert_ne!(result, line, "progress lines should be colored");
    }

    #[test]
    fn memory_header_is_colored() {
        let line = "   text    data     bss     dec     hex filename";
        let result = format_colored(line);
        assert_ne!(result, line, "memory headers should be colored");
    }

    #[test]
    fn memory_data_line_is_colored() {
        let line = "  12344     56    7890   20290   4f42 firmware.elf";
        let result = format_colored(line);
        assert_ne!(result, line, "memory data should be colored");
    }

    #[test]
    fn linking_line_is_colored() {
        let line = "Linking C executable firmware.elf";
        let result = format_colored(line);
        assert_ne!(result, line, "linking lines should be colored");
    }

    #[test]
    fn built_target_is_colored() {
        let line = "Built target firmware";
        let result = format_colored(line);
        assert_ne!(result, line, "built target should be colored");
    }

    #[test]
    fn ninja_failed_is_colored() {
        let line = "ninja: build stopped: subcommand failed.";
        let result = format_colored(line);
        assert_ne!(result, line, "ninja failed should be colored");
    }

    #[test]
    fn failed_line_is_colored() {
        let line = "FAILED: [code=1] output.elf";
        let result = format_colored(line);
        assert_ne!(result, line, "FAILED lines should be colored");
    }

    // ── Passthrough: result equals input ────────────────────────────

    #[test]
    fn normal_line_passes_through() {
        let line = "/usr/bin/gcc -c source.c -o source.o";
        let result = format_colored(line);
        assert_eq!(result, line);
    }

    #[test]
    fn empty_line_unchanged() {
        assert_eq!(format_colored(""), "");
    }
}
