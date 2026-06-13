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
use std::path::PathBuf;

/// Entry point for `fb-gen install <args>`.
pub fn install_package(pkg: &Package) -> FbGenResult<()> {
    // Phase 1: download → verify → extract → link → record → env
    let install_root = resolve_install_root();
    let dest_dir = install_root.join("toolchains").join(&pkg.id).join(&pkg.version);

    if dest_dir.exists() {
        println!("Already installed: {} v{}", pkg.id, pkg.version);
        return Ok(());
    }

    let archive_path = downloader::download_package(pkg)?;
    downloader::extract_package(&archive_path, &dest_dir)?;
    environment::link_current(&install_root.join("toolchains").join(&pkg.id), &dest_dir)?;
    environment::write_env_file(&install_root, pkg, &dest_dir)?;
    environment::record_installed(&install_root, pkg, &dest_dir)?;
    downloader::verify_install(pkg, &dest_dir)?;

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
}
