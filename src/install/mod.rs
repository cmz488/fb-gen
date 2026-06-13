//! Package installation for fb-gen.
//!
//! Downloads cross-compilation toolchains, MCU SDKs, and middleware,
//! installs them to `~/.fb-gen/`, and bridges SDK sources into the
//! CMake generation pipeline.

pub mod bridge;
pub mod catalogue;
pub mod downloader;
pub mod environment;

use crate::models::FbGenResult;
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
