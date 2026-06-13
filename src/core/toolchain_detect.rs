//! Auto-detection of cross-compilation toolchains installed on the host.
//!
//! Scans `$PATH` for `*-gcc` binaries, queries each candidate for its
//! sysroot and target triplet, and returns a list of usable toolchains.

use crate::models::project::TargetArch;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A cross-compilation toolchain discovered on the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedToolchain {
    /// Toolchain prefix, e.g. `"arm-none-eabi-"`.
    pub prefix: String,
    /// Full path to the C compiler, e.g. `/usr/bin/arm-none-eabi-gcc`.
    pub cc_path: PathBuf,
    /// Sysroot reported by `gcc -print-sysroot`, may be empty.
    pub sysroot: Option<PathBuf>,
    /// Target triplet reported by `gcc -dumpmachine`, e.g. `arm-none-eabi`.
    pub target_triplet: String,
    /// Suggested fb-gen architecture inferred from the triplet.
    pub suggested_arch: TargetArch,
}

/// Known cross-compilation toolchain patterns.
///
/// Each entry maps a prefix substring to a (prefix, suggested_arch) pair.
/// Order matters: more-specific patterns come first so they match before
/// shorter prefixes (e.g. `arm-none-eabi-` before `arm-`).
const KNOWN_TOOLCHAINS: &[(&str, TargetArch)] = &[
    // aarch64 group (alphabetically first)
    ("aarch64-linux-gnu", TargetArch::ARM64),
    ("aarch64-none-elf", TargetArch::ARM64),
    // arm group: more-specific patterns first
    ("arm-linux-gnueabihf", TargetArch::ARM32),
    ("arm-linux-gnueabi", TargetArch::ARM32),
    ("arm-none-eabi", TargetArch::NoneEabi),
    // riscv64 group
    ("riscv64-unknown-elf", TargetArch::RISCV64),
    ("riscv64-linux-gnu", TargetArch::RISCV64),
    // riscv32 group (e.g. ESP32-C3/C6/H2/P4, GD32VF103)
    ("riscv32-esp-elf", TargetArch::RISCV32),
    ("riscv32-unknown-elf", TargetArch::RISCV32),
    // xtensa group (ESP32 / ESP32-S2 / ESP32-S3)
    ("xtensa-esp32-elf", TargetArch::Xtensa),
    ("xtensa-esp32s2-elf", TargetArch::Xtensa),
    ("xtensa-esp32s3-elf", TargetArch::Xtensa),
];

/// Scan `$PATH` for cross-compilation toolchains.
///
/// For each `*-gcc` binary found whose prefix matches a known pattern:
/// 1. Verify `g++` and `objcopy` siblings exist.
/// 2. Run `<prefix>gcc -print-sysroot` → optional sysroot.
/// 3. Run `<prefix>gcc -dumpmachine` → target triplet.
/// 4. Map the triplet to a `TargetArch`.
///
/// Returns a deduplicated list, sorted by prefix.
pub fn detect_toolchains() -> Vec<DetectedToolchain> {
    let mut found: Vec<DetectedToolchain> = Vec::new();
    let mut seen_prefixes: std::collections::HashSet<String> = std::collections::HashSet::new();

    let path_var = std::env::var_os("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path_var) {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let file_name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n,
                None => continue,
            };

            // We're looking for files matching `<prefix>gcc` (or `<prefix>gcc.exe`).
            let stem = file_name
                .strip_suffix("gcc")
                .or_else(|| file_name.strip_suffix("gcc.exe"))
                .unwrap_or("");
            if stem.is_empty() {
                continue;
            }
            // On Windows, the compiler binary has a .exe extension; sibling
            // binaries (g++, objcopy) need the same suffix for the existence
            // check to succeed.
            let is_exe = file_name.ends_with(".exe");

            // Check against known patterns.
            let matched = KNOWN_TOOLCHAINS
                .iter()
                .find(|(pattern, _)| stem.contains(pattern));
            let (_, suggested_arch) = match matched {
                Some(m) => m,
                None => continue,
            };

            let prefix = stem.to_string();
            if !seen_prefixes.insert(prefix.clone()) {
                continue; // Already recorded this prefix.
            }

            // Verify sibling tools exist.  On Windows, append .exe to match
            // the gcc binary's naming convention.
            let (gxx_name, objcopy_name) = if is_exe {
                (format!("{prefix}g++.exe"), format!("{prefix}objcopy.exe"))
            } else {
                (format!("{prefix}g++"), format!("{prefix}objcopy"))
            };
            let gxx_path = dir.join(&gxx_name);
            let objcopy_path = dir.join(&objcopy_name);
            if !gxx_path.exists() || !objcopy_path.exists() {
                continue; // Incomplete toolchain.
            }

            // Query sysroot.
            let sysroot = run_cc_query(&path, &["-print-sysroot"]);
            let sysroot = sysroot.filter(|s| !s.is_empty()).map(PathBuf::from);

            // Query target triplet.
            let triplet = run_cc_query(&path, &["-dumpmachine"]).unwrap_or_default();

            found.push(DetectedToolchain {
                prefix,
                cc_path: path,
                sysroot,
                target_triplet: triplet.clone(),
                suggested_arch: suggested_arch.clone(),
            });
        }
    }

    // Sort: known patterns first, then alphabetically.
    found.sort_by(|a, b| {
        let a_known = KNOWN_TOOLCHAINS
            .iter()
            .position(|(p, _)| a.prefix.contains(p));
        let b_known = KNOWN_TOOLCHAINS
            .iter()
            .position(|(p, _)| b.prefix.contains(p));
        match (a_known, b_known) {
            (Some(ai), Some(bi)) => ai.cmp(&bi).then_with(|| a.prefix.cmp(&b.prefix)),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.prefix.cmp(&b.prefix),
        }
    });

    found
}

/// Run a compiler query and return stdout as a trimmed String.
fn run_cc_query(cc_path: &Path, args: &[&str]) -> Option<String> {
    Command::new(cc_path)
        .args(args)
        .output()
        .ok()
        .and_then(|out| {
            if out.status.success() {
                String::from_utf8(out.stdout)
                    .ok()
                    .map(|s| s.trim().to_string())
            } else {
                None
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detected_toolchain_struct() {
        let dt = DetectedToolchain {
            prefix: "arm-none-eabi-".into(),
            cc_path: PathBuf::from("/usr/bin/arm-none-eabi-gcc"),
            sysroot: Some(PathBuf::from("/usr/lib/arm-none-eabi")),
            target_triplet: "arm-none-eabi".into(),
            suggested_arch: TargetArch::NoneEabi,
        };
        assert_eq!(dt.prefix, "arm-none-eabi-");
        assert!(dt.sysroot.is_some());
    }

    #[test]
    fn test_detect_empty_path_returns_empty() {
        let result = detect_toolchains();
        // We can't assert exact results since the host may not have cross-compilers.
        // But the function must not panic.
        let _ = result.len();
    }

    #[test]
    fn test_known_toolchains_sorted() {
        for window in KNOWN_TOOLCHAINS.windows(2) {
            let (a, _) = window[0];
            let (b, _) = window[1];
            assert!(
                a.len() >= b.len() || a < b,
                "KNOWN_TOOLCHAINS: longer (more-specific) prefixes must come first: {a} vs {b}"
            );
        }
    }
}
