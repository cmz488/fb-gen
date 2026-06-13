//! CMake bridge — collects source-file metadata from installed SDK packages
//! and feeds it into the CMakeLists.txt generation pipeline.

use std::path::{Path, PathBuf};

/// Sources collected from an installed SDK/middleware package,
/// ready to be injected into CMakeLists.txt generation.
#[derive(Debug, Clone)]
pub struct PackageSources {
    pub package_name: String,
    pub include_dirs: Vec<PathBuf>,
    pub source_files: Vec<PathBuf>,
    pub compile_defines: Vec<String>,
    pub link_libraries: Vec<String>,
}

/// Scan `~/.fb-gen/installed/` for packages with `CmakePackageMeta`,
/// expand source globs, and return injectable `PackageSources`.
pub fn scan_installed_packages() -> Vec<PackageSources> {
    use crate::install::catalogue::CATALOGUE;
    use crate::install::environment::read_installed_records;

    let install_root = crate::install::resolve_install_root();
    let records = read_installed_records(&install_root);

    let mut results: Vec<PackageSources> = Vec::new();

    for record in &records {
        // Find matching catalogue entry to get cmake_metadata.
        let catalogue_pkg = match CATALOGUE.iter().find(|p| p.id == record.id) {
            Some(p) => p,
            None => continue,
        };

        // Only SDKs and middleware have cmake_metadata.
        let meta = match &catalogue_pkg.cmake_metadata {
            Some(m) => m,
            None => continue,
        };

        let prefix = PathBuf::from(&record.prefix_path);

        // Expand source globs.
        let mut source_files: Vec<PathBuf> = Vec::new();
        for glob_pattern in meta.source_globs {
            let expanded = expand_simple_glob(&prefix, glob_pattern);
            source_files.extend(expanded);
        }

        // Resolve include dirs to absolute paths.
        let include_dirs: Vec<PathBuf> = meta
            .include_dirs
            .iter()
            .map(|d| prefix.join(d))
            .filter(|d| d.exists())
            .collect();

        results.push(PackageSources {
            package_name: catalogue_pkg.name.to_string(),
            include_dirs,
            source_files,
            compile_defines: meta.compile_defines.iter().map(|s| s.to_string()).collect(),
            link_libraries: meta.link_libraries.iter().map(|s| s.to_string()).collect(),
        });
    }

    results
}

/// Simple glob expansion: given a base directory and a pattern like
/// `"Drivers/STM32F1xx_HAL_Driver/Src/*.c"`, return all matching files.
fn expand_simple_glob(base_dir: &PathBuf, pattern: &str) -> Vec<PathBuf> {
    let mut results: Vec<PathBuf> = Vec::new();

    // Split pattern into directory part and file-suffix part.
    // Pattern: "Drivers/STM32F1xx_HAL_Driver/Src/*.c"
    //   -> dir: "Drivers/STM32F1xx_HAL_Driver/Src"
    //   -> suffix: ".c"
    let (dir_part, suffix) = match pattern.rsplit_once('/') {
        Some((d, "*")) => (d, ""),  // pattern ends with "/*" -> match all files
        Some((d, f)) if f.starts_with('*') => (d, &f[1..]),  // "*.c" -> suffix ".c"
        None if pattern.starts_with('*') => ("", &pattern[1..]),  // root-level "*.c"
        _ => return results,
    };

    let search_dir = base_dir.join(dir_part);
    if !search_dir.exists() {
        return results;
    }

    let entries = match std::fs::read_dir(&search_dir) {
        Ok(e) => e,
        Err(_) => return results,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            let file_name = path.file_name().unwrap_or_default().to_string_lossy();
            if suffix.is_empty() || file_name.ends_with(suffix) {
                results.push(path);
            }
        }
    }

    // Sort for deterministic output.
    results.sort();
    results
}

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
    });

    let marker_path = cache_dir.join("installed_packages.json");
    let content = match serde_json::to_string_pretty(&marker) {
        Ok(s) => s,
        Err(e) => {
            // serde_json::Value serialization should never fail in practice,
            // but handle it gracefully just in case.
            eprintln!("fb-gen: warning: failed to serialize installed packages marker: {e}");
            return;
        }
    };
    if let Err(e) = std::fs::write(&marker_path, &content) {
        // Non-fatal: sync will still work, just without this project's
        // package list being up-to-date.
        eprintln!("fb-gen: warning: failed to write installed packages marker: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_scan_installed_packages_does_not_panic() {
        let result = scan_installed_packages();
        // This reads from the real ~/.fb-gen/installed/ directory.
        // Don't assert on contents — the directory may have packages.
        // Just verify it doesn't panic and returns a valid Vec.
        let _ = result;
    }

    #[test]
    fn test_package_sources_default() {
        let ps = PackageSources {
            package_name: "test".into(),
            include_dirs: vec![],
            source_files: vec![],
            compile_defines: vec![],
            link_libraries: vec![],
        };
        assert_eq!(ps.package_name, "test");
        assert!(ps.include_dirs.is_empty());
    }

    #[test]
    fn test_expand_simple_glob() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();

        // Create Dir/Sub/file.c and Dir/Sub/file.h
        let sub = root.join("Dir").join("Sub");
        fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join("file.c"), b"int x;").unwrap();
        fs::write(sub.join("file.h"), b"// header").unwrap();

        // Pattern "Dir/Sub/*.c" should match only file.c
        let result = expand_simple_glob(&root, "Dir/Sub/*.c");
        assert_eq!(result.len(), 1);
        assert!(result[0].ends_with("file.c"));
    }

    #[test]
    fn test_expand_simple_glob_match_all_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();

        let sub = root.join("src");
        fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join("a.c"), b"int a;").unwrap();
        fs::write(sub.join("b.asm"), b".text").unwrap();

        // Pattern "src/*" should match all files
        let result = expand_simple_glob(&root, "src/*");
        assert_eq!(result.len(), 2);
    }
}
