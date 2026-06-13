//! Environment setup for installed packages.
//!
//! Creates "current" symlinks, env files, and records installations.
//! Implemented in Task 4.

use crate::install::catalogue::Package;
use crate::models::FbGenResult;
use std::path::PathBuf;

/// Create or update a "current" symlink for a toolchain family.
pub fn link_current(toolchain_dir: &PathBuf, dest: &PathBuf) -> FbGenResult<()> {
    let (_toolchain_dir, _dest) = (toolchain_dir, dest);
    todo!("implement in Task 4")
}

/// Write an environment file sourcing the installed package.
pub fn write_env_file(install_root: &PathBuf, pkg: &Package, dest: &PathBuf) -> FbGenResult<()> {
    let (_install_root, _pkg, _dest) = (install_root, pkg, dest);
    todo!("implement in Task 4")
}

/// Record the installation in the install root's registry.
pub fn record_installed(install_root: &PathBuf, pkg: &Package, dest: &PathBuf) -> FbGenResult<()> {
    let (_install_root, _pkg, _dest) = (install_root, pkg, dest);
    todo!("implement in Task 4")
}
