//! Environment management for installed packages.
//!
//! Writes shell-profile snippets (`~/.fb-gen/env`) to put toolchain `bin/`
//! directories on `PATH`, maintains `current` symlinks for version switching,
//! and persists `InstalledRecord` JSON files for the bridge and catalogue.

use crate::install::catalogue::Package;
use crate::models::{FbGenError, FbGenResult};
use serde::{Deserialize, Serialize};
use std::fs;
use std::os::unix::fs as unix_fs;
use std::path::Path;

/// Record of an installed package, persisted to `installed/{id}.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledRecord {
    pub id: String,
    pub version: String,
    pub installed_at: String,
    pub prefix_path: String,
    pub bin_path: String,
}

/// Append a `PATH` export line to the global `~/.fb-gen/env` file.
///
/// The file is sourced by the user's shell profile (`source ~/.fb-gen/env`).
/// Duplicate entries for the same package are skipped.
pub fn write_env_file(
    install_root: &Path,
    pkg: &Package,
    dest_dir: &Path,
) -> FbGenResult<()> {
    let bin_dir = dest_dir.join("bin");
    let env_file = install_root.join("env");

    let line = format!(
        "# fb-gen: {} v{}\nexport PATH=\"{}:$PATH\"\n",
        pkg.id,
        pkg.version,
        bin_dir.display()
    );

    let existing = fs::read_to_string(&env_file).unwrap_or_default();
    if !existing.contains(&format!("# fb-gen: {}", pkg.id)) {
        let mut new = existing;
        new.push_str(&line);
        fs::write(&env_file, &new).map_err(FbGenError::Io)?;
    }

    Ok(())
}

/// Remove the PATH line for a given package ID from the env file.
pub fn remove_from_env(install_root: &Path, pkg_id: &str) -> FbGenResult<()> {
    let env_file = install_root.join("env");
    if !env_file.exists() {
        return Ok(());
    }

    let content = std::fs::read_to_string(&env_file).unwrap_or_default();
    let marker = format!("# fb-gen: {}", pkg_id);

    // Filter out the 2-line block (comment + export) for this package.
    let lines: Vec<&str> = content.lines().collect();
    let mut out: Vec<&str> = Vec::new();
    let mut skip = false;

    for line in &lines {
        if line.starts_with(&marker) {
            skip = true;  // skip this line (the comment) and the next (export PATH)
            continue;
        }
        if skip {
            // This is the export PATH line — skip it.
            skip = false;
            continue;
        }
        out.push(line);
    }

    let new_content = out.join("\n");
    if !new_content.is_empty() {
        std::fs::write(&env_file, format!("{}\n", new_content)).map_err(FbGenError::Io)?;
    } else {
        std::fs::remove_file(&env_file).map_err(FbGenError::Io)?;
    }

    Ok(())
}

/// Create (or replace) a `current` symlink pointing to the given version
/// directory so that `…/toolchains/arm-none-eabi/current/bin/` always
/// resolves to the active version.
#[cfg(unix)]
pub fn link_current(toolchain_dir: &Path, version_dir: &Path) -> FbGenResult<()> {
    let current_link = toolchain_dir.join("current");

    if current_link.is_symlink() {
        fs::remove_file(&current_link).map_err(FbGenError::Io)?;
    } else if current_link.exists() {
        fs::remove_dir_all(&current_link).map_err(FbGenError::Io)?;
    }

    unix_fs::symlink(version_dir, &current_link).map_err(FbGenError::Io)?;
    Ok(())
}

/// On non-Unix platforms symlink is not supported — skip silently.
#[cfg(not(unix))]
pub fn link_current(_toolchain_dir: &Path, _version_dir: &Path) -> FbGenResult<()> {
    Ok(())
}

/// Persist an `InstalledRecord` to `~/.fb-gen/installed/{id}.json`.
pub fn record_installed(
    install_root: &Path,
    pkg: &Package,
    dest_dir: &Path,
) -> FbGenResult<()> {
    let installed_dir = install_root.join("installed");
    fs::create_dir_all(&installed_dir).map_err(FbGenError::Io)?;

    let record = InstalledRecord {
        id: pkg.id.to_string(),
        version: pkg.version.to_string(),
        installed_at: chrono::Utc::now().to_rfc3339(),
        prefix_path: dest_dir.to_string_lossy().into_owned(),
        bin_path: dest_dir.join("bin").to_string_lossy().into_owned(),
    };

    let json =
        serde_json::to_string_pretty(&record)
            .map_err(|e| FbGenError::Serialization(e.to_string()))?;

    let record_path = installed_dir.join(format!("{}.json", pkg.id));
    fs::write(&record_path, json).map_err(FbGenError::Io)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::install::catalogue::{Package, PackageKind, PlatformDownloads};

    fn make_test_pkg() -> Package {
        Package {
            id: "test-toolchain",
            name: "Test Toolchain",
            kind: PackageKind::Toolchain,
            version: "0.1.0",
            arch: None,
            downloads: PlatformDownloads::default(),
            verify: "test",
            dependencies: &[],
            scope: crate::install::catalogue::InstallScope::Global,
            cmake_metadata: None,
        }
    }

    #[test]
    fn test_write_env_file_creates_entry() {
        let dir = tempfile::tempdir().unwrap();
        let install_root = dir.path().join(".fb-gen");
        let dest_dir = install_root
            .join("toolchains")
            .join("test-toolchain")
            .join("0.1.0");
        fs::create_dir_all(dest_dir.join("bin")).unwrap();
        let pkg = make_test_pkg();

        write_env_file(&install_root, &pkg, &dest_dir).unwrap();
        let env_content = fs::read_to_string(install_root.join("env")).unwrap();
        assert!(env_content.contains("export PATH"));
        assert!(env_content.contains("test-toolchain"));
        assert!(env_content.contains("0.1.0"));
    }

    #[test]
    fn test_write_env_file_dedup() {
        let dir = tempfile::tempdir().unwrap();
        let install_root = dir.path().join(".fb-gen");
        let dest_dir = install_root
            .join("toolchains")
            .join("test-toolchain")
            .join("0.1.0");
        fs::create_dir_all(dest_dir.join("bin")).unwrap();
        let pkg = make_test_pkg();

        // Write twice — second call should not duplicate.
        write_env_file(&install_root, &pkg, &dest_dir).unwrap();
        write_env_file(&install_root, &pkg, &dest_dir).unwrap();

        let content = fs::read_to_string(install_root.join("env")).unwrap();
        let count = content.matches("fb-gen: test-toolchain").count();
        assert_eq!(count, 1, "env file should deduplicate");
    }

    #[test]
    fn test_record_installed_writes_json() {
        let dir = tempfile::tempdir().unwrap();
        let install_root = dir.path().join(".fb-gen");
        let dest_dir = install_root
            .join("toolchains")
            .join("test-toolchain")
            .join("0.1.0");
        fs::create_dir_all(dest_dir.join("bin")).unwrap();
        let pkg = make_test_pkg();

        record_installed(&install_root, &pkg, &dest_dir).unwrap();

        let record_path = install_root.join("installed").join("test-toolchain.json");
        assert!(record_path.exists());

        let json = fs::read_to_string(&record_path).unwrap();
        let record: InstalledRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(record.id, "test-toolchain");
        assert_eq!(record.version, "0.1.0");
    }

    #[test]
    fn test_remove_from_env() {
        let dir = tempfile::tempdir().unwrap();
        let install_root = dir.path().join(".fb-gen");
        let env_file = install_root.join("env");

        // Write two package entries.
        fs::create_dir_all(&install_root).unwrap();
        fs::write(
            &env_file,
            "# fb-gen: toolchain-a v1.0\nexport PATH=\"/path/to/a/bin:$PATH\"\n\
             # fb-gen: toolchain-b v2.0\nexport PATH=\"/path/to/b/bin:$PATH\"\n",
        ).unwrap();

        // Remove toolchain-a.
        remove_from_env(&install_root, "toolchain-a").unwrap();

        let content = fs::read_to_string(&env_file).unwrap();
        assert!(!content.contains("toolchain-a"), "toolchain-a should be removed");
        assert!(content.contains("toolchain-b"), "toolchain-b should remain");
    }
}
