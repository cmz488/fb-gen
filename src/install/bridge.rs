//! CMake bridge — collects source-file metadata from installed SDK packages
//! and feeds it into the CMakeLists.txt generation pipeline.
//!
//! Phase 1: toolchains carry no `CmakePackageMeta`, so this always returns
//! an empty `Vec`.  SDK support (Phase 3) will read `~/.fb-gen/installed/*.json`,
//! expand source globs, and return injectable `PackageSources`.

use std::path::PathBuf;

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
///
/// Phase 1 stub — always returns an empty `Vec`.
pub fn scan_installed_packages() -> Vec<PackageSources> {
    // Phase 3+ will:
    //   1. Read ~/.fb-gen/installed/*.json
    //   2. Filter packages with cmake_metadata
    //   3. Expand source_globs → absolute PathBufs
    //   4. Return Vec<PackageSources>
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scan_installed_packages_phase1_stub() {
        let result = scan_installed_packages();
        assert!(result.is_empty());
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
}
