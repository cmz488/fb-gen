//! Package installation for fb-gen.
//!
//! Downloads cross-compilation toolchains, MCU SDKs, and middleware,
//! installs them to `~/.fb-gen/`, and bridges SDK sources into the
//! CMake generation pipeline.

pub mod bridge;
pub mod catalogue;
pub mod downloader;
pub mod environment;
pub mod manifest;

use crate::models::{FbGenError, FbGenResult};
use catalogue::Package;
use std::collections::HashSet;
use std::path::PathBuf;

/// Install a package and all of its dependencies recursively.
///
/// Dependencies are resolved depth-first.  Already-installed packages
/// are skipped.  Circular dependencies are detected and reported.
pub fn install_package(pkg: &Package) -> FbGenResult<()> {
    let mut installed = HashSet::new();
    let mut visiting = HashSet::new();
    install_package_recursive(pkg, &mut installed, &mut visiting)
}

/// Recursive helper for install_package with cycle detection.
fn install_package_recursive(
    pkg: &Package,
    installed: &mut HashSet<String>,
    visiting: &mut HashSet<String>,
) -> FbGenResult<()> {
    // Cycle detection.
    if visiting.contains(pkg.id) {
        return Err(FbGenError::Config(format!(
            "Circular dependency detected involving package '{}'",
            pkg.id
        )));
    }

    // Already installed (in this session or on disk).
    let install_root = resolve_install_root();
    let dest_dir = install_root.join("toolchains").join(pkg.id).join(pkg.version);
    if dest_dir.exists() || installed.contains(pkg.id) {
        return Ok(());
    }

    // Mark as in-progress (for cycle detection).
    visiting.insert(pkg.id.to_string());

    // Install dependencies first.
    for dep_id in pkg.dependencies {
        let dep_pkg = catalogue::CATALOGUE
            .iter()
            .find(|p| p.id == *dep_id)
            .ok_or_else(|| {
                FbGenError::Config(format!(
                    "Dependency '{}' (required by '{}') not found in catalogue",
                    dep_id, pkg.id
                ))
            })?;
        install_package_recursive(dep_pkg, installed, visiting)?;
    }

    // Remove from visiting now that dependencies are done.
    visiting.remove(pkg.id);

    // Download + extract + configure.
    let archive_path = downloader::download_package(pkg)?;
    downloader::extract_package(&archive_path, &dest_dir)?;
    environment::link_current(&install_root.join("toolchains").join(pkg.id), &dest_dir)?;
    environment::write_env_file(&install_root, pkg, &dest_dir)?;
    environment::record_installed(&install_root, pkg, &dest_dir)?;
    if !pkg.verify.is_empty() {
        downloader::verify_install(pkg, &dest_dir)?;
    }

    installed.insert(pkg.id.to_string());

    Ok(())
}

/// Uninstall a previously installed package.
///
/// Removes the version directory, the `current` symlink (if it points
/// to this version), the installed record JSON, and the PATH line from
/// the env file.
pub fn uninstall_package(pkg_id: &str) -> FbGenResult<()> {
    let install_root = resolve_install_root();
    let record_path = install_root.join("installed").join(format!("{}.json", pkg_id));

    // Read the installed record to find paths.
    if !record_path.exists() {
        println!("Package '{}' is not installed.", pkg_id);
        return Ok(());
    }

    let json = std::fs::read_to_string(&record_path)
        .map_err(|e| FbGenError::Config(format!("Failed to read record: {e}")))?;
    let record: environment::InstalledRecord = serde_json::from_str(&json)
        .map_err(|e| FbGenError::Serialization(e.to_string()))?;

    // Remove the version directory.
    let version_dir = std::path::PathBuf::from(&record.prefix_path);
    if version_dir.exists() {
        std::fs::remove_dir_all(&version_dir)
            .map_err(|e| FbGenError::Config(format!("Failed to remove {}: {e}", version_dir.display())))?;
        println!("  Removed: {}", version_dir.display());
    }

    // Remove the `current` symlink if it points to this version.
    let current_link = version_dir.parent().map(|p| p.join("current"));
    if let Some(ref link) = current_link {
        if link.is_symlink() || link.exists() {
            let _ = std::fs::remove_file(link);
        }
    }

    // Remove PATH line from env file.
    environment::remove_from_env(&install_root, pkg_id)?;

    // Remove the installed record.
    std::fs::remove_file(&record_path)
        .map_err(|e| FbGenError::Config(format!("Failed to remove record: {e}")))?;

    println!("  Uninstalled: {} v{}", record.id, record.version);
    Ok(())
}

/// Resolve the install root directory.
pub fn resolve_install_root() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".fb-gen")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_install_root() {
        let root = resolve_install_root();
        assert!(root.ends_with(".fb-gen"));
    }

    #[test]
    fn test_dependency_cycle_detected() {
        // Create two packages that depend on each other.
        let pkg_a = Package {
            id: "cyclic-a",
            name: "Cyclic A",
            kind: catalogue::PackageKind::Toolchain,
            version: "1.0",
            arch: None,
            downloads: catalogue::PlatformDownloads::default(),
            verify: "",
            dependencies: &["cyclic-b"],
            scope: catalogue::InstallScope::Global,
            cmake_metadata: None,
        };
        // pkg_b depends on pkg_a → cycle.
        // Note: we can't actually test install_package here because it would
        // try to download. We just test that the recursive logic detects the
        // cycle by checking the visiting set is populated correctly.
        let mut installed = HashSet::new();
        let mut visiting = HashSet::new();
        visiting.insert("cyclic-a".to_string());
        let result = install_package_recursive(&pkg_a, &mut installed, &mut visiting);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Circular dependency"));
    }
}
