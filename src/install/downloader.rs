//! Download, verify, and extract toolchain packages.
//!
//! Implemented in Task 3.

use crate::install::catalogue::Package;
use crate::models::FbGenResult;
use std::path::PathBuf;

/// Download a package archive and return its local path.
pub fn download_package(pkg: &Package) -> FbGenResult<PathBuf> {
    let _ = pkg;
    todo!("implement in Task 3")
}

/// Extract an archive into `dest_dir`.
pub fn extract_package(archive: &PathBuf, dest: &PathBuf) -> FbGenResult<()> {
    let (_archive, _dest) = (archive, dest);
    todo!("implement in Task 3")
}

/// Verify that the installation looks correct (e.g. expected binaries exist).
pub fn verify_install(pkg: &Package, dest: &PathBuf) -> FbGenResult<()> {
    let (_pkg, _dest) = (pkg, dest);
    todo!("implement in Task 3")
}
