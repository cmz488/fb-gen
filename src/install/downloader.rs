//! Download, verify, and extract package archives.
//!
//! Uses `ureq` for HTTP(S) downloads and `sha2` for SHA-256 checksums.
//! Archive extraction delegates to the system `tar` binary (supports
//! `.tar.gz` and `.tar.xz`).

use crate::install::catalogue::{Download, Package};
use crate::models::{FbGenError, FbGenResult};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

/// Download a package archive to a temporary file.
///
/// The returned `PathBuf` points to the downloaded archive in a temporary
/// directory.  The caller is responsible for cleanup (use `tempfile` or
/// remove after extraction).
pub fn download_package(pkg: &Package) -> FbGenResult<PathBuf> {
    let dl = pkg
        .downloads
        .for_current_platform()
        .ok_or_else(|| {
            FbGenError::Config(format!(
                "No download available for current platform ({} {})",
                std::env::consts::OS,
                std::env::consts::ARCH
            ))
        })?;

    let tmp_dir = std::env::temp_dir().join("fb-gen-dl");
    fs::create_dir_all(&tmp_dir).map_err(FbGenError::Io)?;

    let archive_name = dl.url.rsplit('/').next().unwrap_or("archive.tar.xz");
    let dest = tmp_dir.join(archive_name);

    println!("  Downloading {} ...", pkg.name);
    let response = ureq::get(dl.url)
        .call()
        .map_err(|e| FbGenError::Config(format!("Download failed: {e}")))?;

    let mut reader = response.into_body().into_reader();
    let mut file = fs::File::create(&dest).map_err(FbGenError::Io)?;

    let mut buf = [0u8; 8192];
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| FbGenError::Config(format!("Read error: {e}")))?;
        if n == 0 {
            break;
        }
        io::Write::write_all(&mut file, &buf[..n]).map_err(FbGenError::Io)?;
    }

    // Verify SHA256 (skip check when the catalogue entry uses a placeholder).
    if dl.sha256 != "TODO_REAL_SHA256" {
        let actual_sha256 = sha256_file(&dest)?;
        if actual_sha256 != dl.sha256 {
            let _ = fs::remove_file(&dest);
            return Err(FbGenError::Config(format!(
                "SHA256 mismatch for {}:\n  expected: {}\n  got:      {}",
                pkg.id, dl.sha256, actual_sha256
            )));
        }
    }

    Ok(dest)
}

/// Compute the hex-encoded SHA-256 digest of a file.
pub fn sha256_file(path: &Path) -> FbGenResult<String> {
    let data = fs::read(path).map_err(FbGenError::Io)?;
    let mut hasher = Sha256::new();
    hasher.update(&data);
    Ok(format!("{:x}", hasher.finalize()))
}

/// Extract a `.tar.gz` or `.tar.xz` archive using the system `tar` command.
///
/// Strips the top-level directory component so that the package contents
/// land directly in `dest_dir`.
pub fn extract_package(archive_path: &Path, dest_dir: &Path) -> FbGenResult<()> {
    let parent = dest_dir.parent().unwrap_or(dest_dir);
    fs::create_dir_all(parent).map_err(FbGenError::Io)?;
    fs::create_dir_all(dest_dir).map_err(FbGenError::Io)?;

    let archive_str = archive_path.to_string_lossy();

    let status = std::process::Command::new("tar")
        .arg("xf")
        .arg(archive_str.as_ref())
        .arg("-C")
        .arg(dest_dir.to_string_lossy().as_ref())
        .arg("--strip-components=1")
        .status()
        .map_err(|e| FbGenError::Config(format!("tar extraction failed: {e}")))?;

    if !status.success() {
        return Err(FbGenError::Config(
            "tar extraction returned non-zero exit code".into(),
        ));
    }

    Ok(())
}

/// Verify that the installed toolchain works by running `{prefix}gcc --version`.
pub fn verify_install(pkg: &Package, dest_dir: &Path) -> FbGenResult<()> {
    let bin_dir = dest_dir.join("bin");

    // Search bin/ for a file ending with "gcc" whose name contains the verify
    // string (e.g. "arm-none-eabi-gcc" matching verify "arm-none-eabi").
    let entries: Vec<_> = fs::read_dir(&bin_dir)
        .map_err(FbGenError::Io)?
        .filter_map(|e| e.ok())
        .collect();

    let gcc_path = entries
        .iter()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .find(|n| n.ends_with("gcc") && n.contains(pkg.verify))
        .map(|n| bin_dir.join(&n));

    let gcc_path = match gcc_path {
        Some(p) => p,
        None => {
            let candidates: Vec<_> = entries
                .iter()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            return Err(FbGenError::Config(format!(
                "Could not find gcc binary in {}. Candidates: {:?}",
                bin_dir.display(),
                candidates
            )));
        }
    };

    let output = std::process::Command::new(&gcc_path)
        .arg("--version")
        .output()
        .map_err(|e| FbGenError::Config(format!("Failed to run {}: {e}", gcc_path.display())))?;

    if !output.status.success() {
        return Err(FbGenError::Config(format!(
            "{} --version failed with status {}",
            gcc_path.display(),
            output.status
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    if !stdout.contains(pkg.verify) {
        return Err(FbGenError::Config(format!(
            "{} --version output does not contain expected string '{}':\n{}",
            gcc_path.display(),
            pkg.verify,
            stdout
        )));
    }

    println!("  Verified: {} --version OK", gcc_path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sha256_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty");
        fs::write(&path, b"").unwrap();
        assert_eq!(
            sha256_file(&path).unwrap(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn test_sha256_hello() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hello");
        fs::write(&path, b"hello world\n").unwrap();
        assert_eq!(sha256_file(&path).unwrap().len(), 64); // 32 bytes hex = 64 chars
    }

    #[test]
    fn test_extract_tar_gz() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("test.tar.gz");
        let src = dir.path().join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("test.txt"), b"hello").unwrap();

        let status = std::process::Command::new("tar")
            .arg("czf")
            .arg(&archive)
            .arg("-C")
            .arg(dir.path())
            .arg("src")
            .status()
            .unwrap();
        assert!(status.success());

        let dest = dir.path().join("dest");
        extract_package(&archive, &dest).unwrap();
        assert!(dest.join("test.txt").exists());
    }
}
