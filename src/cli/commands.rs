//! CLI command implementations — wires scanner → discoverer → analyzer → generator.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use crate::cli::Cli;
use crate::core::{CMakeGenerator, DependencyAnalyzer, ModuleDiscoverer};
use crate::models::dependency::DependencyGraph;
use crate::models::module::SourceFile;
use crate::models::{
    BuildBackend, CMakeModule, DependencySnapshot, FbGenError, FbGenResult, ProjectConfig,
    ProjectMeta,
};
use crate::orchestration::{FileWatcher, MetaCache, Reporter, UserQuery};
use crate::scanner::{self, FffScanner};

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
    if !user_modules.is_empty() {
        reporter.report_info(&format!(
            "Found {} user-defined CMake module(s)",
            user_modules.len()
        ));
    }

    Ok((modules, graph, user_modules))
}

// ── commands ───────────────────────────────────────────────────────────────

/// `fb-gen init` — interactive first-time project setup.
pub fn cmd_init(cli: &Cli, name: Option<&str>) -> FbGenResult<()> {
    let reporter = Reporter::new(cli.quiet);

    // ── Collect config ──
    let root = if cli.root == PathBuf::from(".") {
        std::env::current_dir().map_err(|e| FbGenError::Config(format!("cwd: {e}")))?
    } else {
        cli.root.clone()
    };

    let mut config = UserQuery::ask_project_config(&root)?;

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

    // ── Pipeline ──
    let start = Instant::now();
    let (modules, graph, user_modules) = scan_and_discover(cli, &config, &reporter)?;

    // ── Generate ──
    reporter.report_info("Generating CMakeLists.txt files ...");
    let generator = CMakeGenerator::new(&config)?;

    let empty_graph = DependencyGraph::new();
    let ref_graph = graph.as_ref().unwrap_or(&empty_graph);
    generator.generate(&modules, ref_graph, true, &user_modules)?;
    reporter.report_success("CMakeLists.txt files generated");

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

    Ok(())
}

/// `fb-gen sync` — incremental update using cached metadata.
///
/// Instead of a full re-scan, this uses `ProjectMeta` from `.fb-gen/cache/`
/// to detect exactly which files changed, then only re-processes affected modules.
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

    let config = prev_meta.config.clone();

    // ── 1. Detect changes via checksum comparison ──
    reporter.report_info("Checking for file changes ...");
    let watcher = FileWatcher::new(&root, config.exclude_dirs.clone());
    let changed_paths = watcher.get_changes(&prev_meta.file_checksums);

    if changed_paths.is_empty() {
        reporter.report_success("No changes detected — everything up to date.");
        return Ok(());
    }
    reporter.report_info(&format!("Detected {} changed file(s)", changed_paths.len()));

    let start = Instant::now();

    // ── 2. Classify changes: added / modified / deleted ──
    let scanner = FffScanner::new(&root);
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
            let hash = cache.compute_checksums(&[path.clone()]);
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
                    &root,
                    &config.exclude_dirs,
                    &mut affected_modules,
                    &mut module_list_changed,
                );
            }
        }
    }

    // Remove deleted paths from checksums
    for dp in &deleted_paths {
        prev_meta
            .file_checksums
            .remove(&dp.to_string_lossy().to_string());
    }

    if affected_modules.is_empty() && !module_list_changed {
        reporter.report_success("No modules affected by changes — skipping generation.");
        return Ok(());
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

    // ── 4. Regenerate CMakeLists.txt ──
    // Always pass all modules so the root CMakeLists.txt has the full subdirs list.
    // The generator's diff-check skips files whose content hasn't changed,
    // so only genuinely affected files are re-written.
    reporter.report_info("Regenerating affected CMakeLists.txt ...");
    let user_modules = scanner.scan_user_cmake_files(&root, &config.exclude_dirs);
    let generator = CMakeGenerator::new(&config)?;
    generator.generate(&modules, &graph, false, &user_modules)?;
    reporter.report_success(&format!("{} module(s) updated", n_affected));

    // ── 5. Merge checksums and save updated meta ──
    prev_meta.file_checksums.extend(new_checksums);

    let dep_snapshot = DependencySnapshot {
        nodes: modules.iter().map(|m| m.name.clone()).collect(),
        edges: modules
            .iter()
            .flat_map(|m| {
                graph
                    .get_dependencies(&m.name)
                    .into_iter()
                    .map(|(dep_name, _)| (m.name.clone(), dep_name))
            })
            .collect(),
    };

    let meta = ProjectMeta {
        config,
        modules,
        dependency_graph: dep_snapshot,
        file_checksums: prev_meta.file_checksums,
        last_sync: chrono::Utc::now().to_rfc3339(),
    };
    cache.save(&meta)?;

    let elapsed = start.elapsed();
    reporter.report_success(&format!(
        "Sync done in {:.1}s — {} module(s) updated",
        elapsed.as_secs_f64(),
        n_affected
    ));

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
        let found = if new_sf.source_type.is_source() {
            m.sources.iter().position(|sf| sf.path == new_sf.path)
        } else {
            m.headers.iter().position(|sf| sf.path == new_sf.path)
        };

        if let Some(pos) = found {
            if new_sf.source_type.is_source() {
                m.sources[pos] = new_sf;
            } else {
                m.headers[pos] = new_sf;
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
        } else {
            m.headers.push(sf);
        }
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
            user_config: None,
        };

        if sf.source_type.is_source() {
            new_module.sources.push(sf);
        } else {
            new_module.headers.push(sf);
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

        m.sources.retain(|sf| sf.path != path);
        m.headers.retain(|sf| sf.path != path);

        if m.sources.len() != old_src_len || m.headers.len() != old_hdr_len {
            affected.insert(m.name.clone());
        }

        // Remove module if it became empty.
        if m.sources.is_empty() && m.headers.is_empty() {
            modules.remove(i);
            *list_changed = true;
        }
    }
}

/// Quick check if a SourceFile contains a main() function.
fn source_has_main(sf: &SourceFile) -> bool {
    if let Ok(content) = std::fs::read_to_string(&sf.path) {
        regex::Regex::new(r"(?:int|void)\s+main\s*\(")
            .map(|re| re.is_match(&content))
            .unwrap_or(false)
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
            reason: "cached".into(),
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
                .map(|sf| sf.path.clone())
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

    // Build a minimal config with standard exclusions.
    let config = ProjectConfig {
        name: root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("project")
            .to_string(),
        root: root.clone(),
        exclude_dirs: vec!["build".into(), ".git".into(), "third_party".into()],
        ..Default::default()
    };

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

    let generator = CMakeGenerator::new(&tmp_config)?;
    let empty_graph = DependencyGraph::new();
    let ref_graph = graph.as_ref().unwrap_or(&empty_graph);
    generator.generate(&modules, ref_graph, false, &user_modules)?;
    let mut diffs = 0usize;
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

    if diffs == 0 {
        reporter.report_success("All CMakeLists.txt files are in sync with project structure.");
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
    let gen_flags = cmake_generator_flag(&config);

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
    let toolchain_args = cmake_toolchain_args(&config);
    for f in &toolchain_args {
        cmd.arg(f);
    }
    let output = cmd
        .output()
        .map_err(|e| FbGenError::Config(format!("failed to run cmake: {e}")))?;

    if output.status.success() {
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

    let config = if cache.exists() {
        cache
            .load()
            .map(|m| m.config)
            .unwrap_or_else(fallback_config)
    } else {
        fallback_config()
    };

    let gen_flags = cmake_generator_flag(&config);

    // Generate if no CMakeLists.txt exists.
    let root_cmake = root.join("CMakeLists.txt");
    if !root_cmake.exists() {
        reporter.report_info("No CMakeLists.txt found — generating ...");
        let (modules, graph, user_modules) = scan_and_discover(cli, &config, &reporter)?;
        let generator = CMakeGenerator::new(&config)?;
        let empty_graph = DependencyGraph::new();
        let ref_graph = graph.as_ref().unwrap_or(&empty_graph);
        generator.generate(&modules, ref_graph, false, &user_modules)?;
        // Save cache for future incremental syncs.
        save_meta_cache(&root, &modules, graph.as_ref(), &config)?;
    }

    // Configure.
    let build_dir = root.join(&cli.output);
    std::fs::create_dir_all(&build_dir).map_err(FbGenError::Io)?;

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
    let toolchain_args = cmake_toolchain_args(&config);
    for f in &toolchain_args {
        cmd.arg(f);
    }
    let status = cmd
        .status()
        .map_err(|e| FbGenError::Config(format!("cmake: {e}")))?;

    if !status.success() {
        return Err(FbGenError::GenerationFailed(
            "cmake configure failed".into(),
        ));
    }

    // Build.
    reporter.report_info(&format!(
        "Building with cmake --build {} ...",
        build_dir.display()
    ));
    let start = Instant::now();

    let status = Command::new("cmake")
        .arg("--build")
        .arg(&build_dir)
        .status()
        .map_err(|e| FbGenError::Config(format!("cmake --build: {e}")))?;

    if status.success() {
        let elapsed = start.elapsed();
        reporter.report_success(&format!("Build succeeded in {:.1}s", elapsed.as_secs_f64()));
    } else {
        reporter.report_error("Build failed.");
        return Err(FbGenError::GenerationFailed("build failed".into()));
    }

    Ok(())
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
