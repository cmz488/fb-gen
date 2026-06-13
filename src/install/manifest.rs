//! Remote manifest — fetches a JSON package catalogue from a URL, caches it
//! locally with a 48-hour TTL, and supplements the hard-coded CATALOGUE.
//!
//! The cache lives at `~/.fb-gen/manifest.json`.  Network fetch only happens
//! when the cache is missing or stale (older than 48 hours).  On any failure
//! (network error, invalid JSON, …) `fetch_manifest` returns `None` so the
//! caller falls back gracefully to the embedded catalogue.

use crate::install::catalogue::{Download, Package, PackageKind, PlatformDownloads, InstallScope};
use crate::models::project::TargetArch;
use serde::Deserialize;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

/// Default URL for the remote manifest.
const DEFAULT_MANIFEST_URL: &str =
    "https://raw.githubusercontent.com/cmz488/fb-gen/main/manifest.json";

/// Cache TTL: 48 hours.
const CACHE_TTL: Duration = Duration::from_secs(48 * 3600);

/// A remote manifest JSON structure.
#[derive(Debug, Deserialize)]
struct RemoteManifest {
    /// Schema version (consumer should check compatibility).
    #[allow(dead_code)]
    version: u32,
    /// Packages declared in this manifest.
    packages: Vec<RemotePackage>,
}

/// A package from the remote manifest.
///
/// Uses owned `String` fields (not `&'static str`) so deserialisation
/// is straightforward.  The fields mirror a subset of the hard-coded
/// `Package` struct plus the host-specific download URL triplets.
#[derive(Debug, Clone, Deserialize)]
pub struct RemotePackage {
    /// Stable identifier (e.g. `"arm-none-eabi-gcc"`).
    pub id: String,
    /// Human-readable display name.
    pub name: String,
    /// Upstream version string.
    pub version: String,
    /// Target architecture string (e.g. `"Xtensa"`, `"RISCV32"`, `"NoneEabi"`).
    /// Parsed into `TargetArch` via [`parse_arch_str`].
    pub arch: Option<String>,
    /// x86_64 Linux download URL.
    pub linux_x86_64_url: Option<String>,
    /// SHA-256 hex for the x86_64 Linux download.
    pub linux_x86_64_sha256: Option<String>,
    /// aarch64 Linux download URL.
    pub linux_aarch64_url: Option<String>,
    /// SHA-256 hex for the aarch64 Linux download.
    pub linux_aarch64_sha256: Option<String>,
    /// arm64 macOS download URL.
    pub macos_arm64_url: Option<String>,
    /// SHA-256 hex for the arm64 macOS download.
    pub macos_arm64_sha256: Option<String>,
    /// x86_64 macOS download URL.
    pub macos_x86_64_url: Option<String>,
    /// SHA-256 hex for the x86_64 macOS download.
    pub macos_x86_64_sha256: Option<String>,
    /// x86_64 Windows download URL.
    pub windows_x86_64_url: Option<String>,
    /// SHA-256 hex for the x86_64 Windows download.
    pub windows_x86_64_sha256: Option<String>,
    /// Substring expected in `{prefix}gcc --version` output after install.
    pub verify: String,
}

/// Fetch the remote manifest from `url`, respecting a local cache with 48h TTL.
///
/// Returns `None` when the cache is still fresh (no network call), on network
/// error, or when the JSON fails to validate.  The caller should fall back to
/// the hard-coded [`CATALOGUE`](crate::install::catalogue::CATALOGUE).
pub fn fetch_manifest(url: Option<&str>) -> Option<Vec<RemotePackage>> {
    let url = url.unwrap_or(DEFAULT_MANIFEST_URL);
    let cache_path = manifest_cache_path();

    // Check local cache freshness first.
    if let Some(packages) = read_cached_manifest(&cache_path) {
        return Some(packages);
    }

    // Cache is missing or stale — fetch from network.
    let data = match fetch_url(&url) {
        Some(d) => d,
        None => return None,
    };

    // Validate JSON schema.
    let manifest: RemoteManifest = serde_json::from_str(&data).ok()?;

    // Persist to cache.
    if let Some(parent) = cache_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(&cache_path, &data);

    Some(manifest.packages)
}

/// Convert a [`RemotePackage`] into a static [`Package`] reference for use
/// with [`install_package`](crate::install::install_package).
///
/// # Leaks
///
/// This function uses `Box::leak` to produce `&'static str` fields.  Memory
/// is reclaimed by the OS when the process exits.  This is acceptable for
/// a one-shot CLI tool — the leaked amount is a few hundred bytes per
/// remote package installed.
pub fn remote_to_package(rp: &RemotePackage) -> Package {
    let arch = rp.arch.as_deref().and_then(parse_arch_str);

    Package {
        id: Box::leak(rp.id.clone().into_boxed_str()),
        name: Box::leak(rp.name.clone().into_boxed_str()),
        kind: PackageKind::Toolchain,
        version: Box::leak(rp.version.clone().into_boxed_str()),
        arch,
        downloads: PlatformDownloads {
            linux_x86_64: rp.linux_x86_64_url.as_ref().map(|url| Download {
                url: Box::leak(url.clone().into_boxed_str()),
                sha256: Box::leak(
                    rp.linux_x86_64_sha256
                        .clone()
                        .unwrap_or_else(|| "TODO_REAL_SHA256".into())
                        .into_boxed_str(),
                ),
            }),
            linux_aarch64: rp.linux_aarch64_url.as_ref().map(|url| Download {
                url: Box::leak(url.clone().into_boxed_str()),
                sha256: Box::leak(
                    rp.linux_aarch64_sha256
                        .clone()
                        .unwrap_or_else(|| "TODO_REAL_SHA256".into())
                        .into_boxed_str(),
                ),
            }),
            macos_arm64: rp.macos_arm64_url.as_ref().map(|url| Download {
                url: Box::leak(url.clone().into_boxed_str()),
                sha256: Box::leak(
                    rp.macos_arm64_sha256
                        .clone()
                        .unwrap_or_else(|| "TODO_REAL_SHA256".into())
                        .into_boxed_str(),
                ),
            }),
            macos_x86_64: rp.macos_x86_64_url.as_ref().map(|url| Download {
                url: Box::leak(url.clone().into_boxed_str()),
                sha256: Box::leak(
                    rp.macos_x86_64_sha256
                        .clone()
                        .unwrap_or_else(|| "TODO_REAL_SHA256".into())
                        .into_boxed_str(),
                ),
            }),
            windows_x86_64: rp.windows_x86_64_url.as_ref().map(|url| Download {
                url: Box::leak(url.clone().into_boxed_str()),
                sha256: Box::leak(
                    rp.windows_x86_64_sha256
                        .clone()
                        .unwrap_or_else(|| "TODO_REAL_SHA256".into())
                        .into_boxed_str(),
                ),
            }),
        },
        verify: Box::leak(rp.verify.clone().into_boxed_str()),
        dependencies: &[],
        scope: InstallScope::Global,
        cmake_metadata: None,
    }
}

// ── internal helpers ──────────────────────────────────────────────────────────

/// Perform an HTTP GET and return the response body as a `String`.
fn fetch_url(url: &str) -> Option<String> {
    let response = ureq::get(url).call().ok()?;
    let mut data = String::new();
    let mut reader = response.into_body().into_reader();
    std::io::Read::read_to_string(&mut reader, &mut data).ok()?;
    Some(data)
}

/// Read the local cache if it exists and is still within the TTL window.
fn read_cached_manifest(cache_path: &PathBuf) -> Option<Vec<RemotePackage>> {
    let meta = fs::metadata(cache_path).ok()?;
    let modified = meta.modified().ok()?;
    let elapsed = SystemTime::now().duration_since(modified).ok()?;
    if elapsed >= CACHE_TTL {
        return None;
    }
    let data = fs::read_to_string(cache_path).ok()?;
    let manifest: RemoteManifest = serde_json::from_str(&data).ok()?;
    Some(manifest.packages)
}

/// Path to the cached manifest: `~/.fb-gen/manifest.json`.
fn manifest_cache_path() -> PathBuf {
    super::resolve_install_root().join("manifest.json")
}

/// Parse an architecture string into [`TargetArch`].
///
/// Accepts common forms: `"Xtensa"`, `"RISCV32"`, `"NoneEabi"`, `"arm64"`,
/// `"aarch64"`, etc.  Comparison is case-insensitive.
pub fn parse_arch_str(s: &str) -> Option<TargetArch> {
    match s.to_lowercase().as_str() {
        "x86_64" => Some(TargetArch::X86_64),
        "x86" => Some(TargetArch::X86),
        "arm64" | "aarch64" => Some(TargetArch::ARM64),
        "arm32" | "arm" => Some(TargetArch::ARM32),
        "riscv64" => Some(TargetArch::RISCV64),
        "riscv32" => Some(TargetArch::RISCV32),
        "wasm" => Some(TargetArch::WASM),
        "noneeabi" => Some(TargetArch::NoneEabi),
        "xtensa" => Some(TargetArch::Xtensa),
        _ => None,
    }
}

// ── tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_arch_str_x86_64() {
        assert_eq!(parse_arch_str("x86_64"), Some(TargetArch::X86_64));
    }

    #[test]
    fn test_parse_arch_str_x86() {
        assert_eq!(parse_arch_str("x86"), Some(TargetArch::X86));
    }

    #[test]
    fn test_parse_arch_str_arm64() {
        assert_eq!(parse_arch_str("arm64"), Some(TargetArch::ARM64));
        assert_eq!(parse_arch_str("aarch64"), Some(TargetArch::ARM64));
    }

    #[test]
    fn test_parse_arch_str_arm32() {
        assert_eq!(parse_arch_str("arm32"), Some(TargetArch::ARM32));
        assert_eq!(parse_arch_str("arm"), Some(TargetArch::ARM32));
    }

    #[test]
    fn test_parse_arch_str_riscv() {
        assert_eq!(parse_arch_str("riscv64"), Some(TargetArch::RISCV64));
        assert_eq!(parse_arch_str("riscv32"), Some(TargetArch::RISCV32));
    }

    #[test]
    fn test_parse_arch_str_noneeabi() {
        assert_eq!(parse_arch_str("NoneEabi"), Some(TargetArch::NoneEabi));
        assert_eq!(parse_arch_str("NONEEABI"), Some(TargetArch::NoneEabi));
    }

    #[test]
    fn test_parse_arch_str_xtensa() {
        assert_eq!(parse_arch_str("Xtensa"), Some(TargetArch::Xtensa));
        assert_eq!(parse_arch_str("XTENSA"), Some(TargetArch::Xtensa));
    }

    #[test]
    fn test_parse_arch_str_wasm() {
        assert_eq!(parse_arch_str("wasm"), Some(TargetArch::WASM));
    }

    #[test]
    fn test_parse_arch_str_unknown() {
        assert_eq!(parse_arch_str("msp430"), None);
        assert_eq!(parse_arch_str(""), None);
    }

    #[test]
    fn test_remote_to_package_minimal() {
        let rp = RemotePackage {
            id: "test-pkg".into(),
            name: "Test Package".into(),
            version: "1.0.0".into(),
            arch: Some("Xtensa".into()),
            linux_x86_64_url: Some("https://example.com/pkg.tar.gz".into()),
            linux_x86_64_sha256: Some("abcdef1234567890".into()),
            linux_aarch64_url: None,
            linux_aarch64_sha256: None,
            macos_arm64_url: None,
            macos_arm64_sha256: None,
            macos_x86_64_url: None,
            macos_x86_64_sha256: None,
            windows_x86_64_url: None,
            windows_x86_64_sha256: None,
            verify: "test-pkg".into(),
        };

        let pkg = remote_to_package(&rp);
        assert_eq!(pkg.id, "test-pkg");
        assert_eq!(pkg.name, "Test Package");
        assert_eq!(pkg.version, "1.0.0");
        assert_eq!(pkg.kind, PackageKind::Toolchain);
        assert_eq!(pkg.arch, Some(TargetArch::Xtensa));
        assert!(pkg.downloads.linux_x86_64.is_some());
        assert!(pkg.downloads.linux_aarch64.is_none());
        assert_eq!(pkg.verify, "test-pkg");
        assert!(pkg.dependencies.is_empty());
        assert!(pkg.cmake_metadata.is_none());
    }

    #[test]
    fn test_remote_to_package_sets_default_sha256_when_missing() {
        let rp = RemotePackage {
            id: "test-pkg".into(),
            name: "Test".into(),
            version: "1.0".into(),
            arch: None,
            linux_x86_64_url: Some("https://example.com/pkg.tar.gz".into()),
            linux_x86_64_sha256: None,
            linux_aarch64_url: None,
            linux_aarch64_sha256: None,
            macos_arm64_url: None,
            macos_arm64_sha256: None,
            macos_x86_64_url: None,
            macos_x86_64_sha256: None,
            windows_x86_64_url: None,
            windows_x86_64_sha256: None,
            verify: "test".into(),
        };

        let pkg = remote_to_package(&rp);
        let dl = pkg.downloads.linux_x86_64.unwrap();
        assert_eq!(dl.sha256, "TODO_REAL_SHA256");
    }

    #[test]
    fn test_manifest_cache_path_ends_correctly() {
        // We can't easily test the full path (depends on $HOME),
        // but we can check the structural invariants.
        let path = manifest_cache_path();
        let s = path.to_string_lossy();
        assert!(s.ends_with("manifest.json"), "should end with manifest.json, got: {s}");
        assert!(s.contains(".fb-gen"), "should contain .fb-gen, got: {s}");
    }

    #[test]
    fn test_fetch_url_fails_on_bogus_url() {
        // Should gracefully return None for a nonsense URL.
        let result = fetch_url("http://0.0.0.0/nonexistent");
        assert!(result.is_none());
    }

    #[test]
    fn test_read_cached_manifest_on_missing_file() {
        let path = PathBuf::from("/tmp/__nonexistent_fb_gen_test_cache__.json");
        let result = read_cached_manifest(&path);
        assert!(result.is_none());
    }

    #[test]
    fn test_fetch_manifest_fails_on_bogus_url() {
        // Should gracefully return None.
        let result = fetch_manifest(Some("http://0.0.0.0/bogus-manifest.json"));
        assert!(result.is_none());
    }
}
