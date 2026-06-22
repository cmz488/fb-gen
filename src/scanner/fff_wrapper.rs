//! Wrapper around `fff-search` for fast file discovery.
//!
//! Falls back to `walkdir` when the fff_search API is not suitable for
//! programmatic, structured directory scanning.

use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;
use walkdir::WalkDir;

use crate::models::error::FbGenResult;
use crate::models::module::{SourceFile, SourceType};
use crate::models::project::CMakePresets;
use crate::scanner::fs_adapter::ScanOptions;

/// Scanner that discovers C/C++ source files and parses their `#include`
/// directives.
///
/// Uses `walkdir` for reliable recursive directory traversal with
/// exclude-dir and extension filtering.  The `fff_search` crate is aimed at
/// interactive fuzzy finding and is not the right fit here, so `walkdir`
/// serves as the primary engine.
pub struct FffScanner {
    root: PathBuf,
}

impl FffScanner {
    /// Create a new scanner rooted at `root`.
    ///
    /// All discovered paths will be computed relative to this root.
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
        }
    }

    /// Recursively scan the project for C/C++ source and header files.
    ///
    /// Directories listed in `options.exclude_dirs` are skipped entirely.
    /// Only files whose extension appears in `options.languages` are
    /// collected.
    ///
    /// For every matching file `scan_includes` is called to extract the
    /// `#include "..."` directives.
    pub fn scan_source_files(&self, options: &ScanOptions) -> FbGenResult<Vec<SourceFile>> {
        let mut sources: Vec<SourceFile> = Vec::new();

        let walker = WalkDir::new(&options.root)
            .follow_links(options.follow_symlinks)
            .into_iter()
            .filter_entry(|e| {
                // Skip directories whose name is in the exclude list.
                if e.file_type().is_dir() {
                    let name = e.file_name().to_string_lossy();
                    !options.exclude_dirs.iter().any(|d| d == name.as_ref())
                } else {
                    true
                }
            });

        for entry in walker {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };

            // Only process regular files.
            if !entry.file_type().is_file() {
                continue;
            }

            let path = entry.path().to_path_buf();

            // Filter by extension.
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();

            if !options.languages.iter().any(|lang| lang.as_str() == ext) {
                continue;
            }

            let source_type = SourceType::from_extension(&ext);

            let includes = self.scan_includes(&path).unwrap_or_default();

            let relative_path = path.strip_prefix(&self.root).unwrap_or(&path).to_path_buf();

            let file_name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();


            sources.push(SourceFile {
                path,
                relative_path,
                file_name,
                source_type,
                includes,
            });
        }

        Ok(sources)
    }

    /// Parse a single source file and return the list of local header
    /// includes (i.e. `#include "..."`).
    ///
    /// System includes (`#include <...>`) are intentionally ignored because
    /// they do not represent intra-project dependencies.
    pub fn scan_includes(&self, file_path: &Path) -> FbGenResult<Vec<String>> {
        let content = fs::read_to_string(file_path)?;

        // Match `#include "foo.h"` — capture the text between the quotes.
        let re = Regex::new(r#"#include\s*"([^"]+)""#)?;
        let includes: Vec<String> = re
            .captures_iter(&content)
            .filter_map(|cap| cap.get(1))
            .map(|m| m.as_str().to_string())
            .collect();

        Ok(includes)
    }

    /// Scan a single file and return a fully-populated `SourceFile`.
    /// Used by incremental sync to avoid re-scanning the entire tree.
    pub fn scan_single(&self, file_path: &Path) -> FbGenResult<SourceFile> {
        let includes = self.scan_includes(file_path)?;

        let ext = file_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        let source_type = SourceType::from_extension(&ext);

        let relative_path = file_path
            .strip_prefix(&self.root)
            .unwrap_or(file_path)
            .to_path_buf();

        let file_name = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();

        Ok(SourceFile {
            path: file_path.to_path_buf(),
            relative_path,
            file_name,
            source_type,
            includes,
        })
    }

    /// Scan for a `CMakePresets.json` file at the project root.
    ///
    /// Returns `Ok(None)` if the file does not exist.
    pub fn scan_presets(&self, root: &Path) -> FbGenResult<Option<CMakePresets>> {
        let presets_path = root.join("CMakePresets.json");

        if !presets_path.exists() {
            return Ok(None);
        }

        let content = fs::read_to_string(&presets_path)?;
        let presets: CMakePresets = serde_json::from_str(&content)?;

        Ok(Some(presets))
    }

    /// Recursively scan the project for `.cmake` toolchain files.
    ///
    /// Directories listed in `exclude_dirs` are skipped entirely.
    /// Returns a list of absolute paths to `.cmake` files.
    pub fn scan_toolchain_files(
        &self,
        root: &Path,
        exclude_dirs: &[String],
    ) -> FbGenResult<Vec<PathBuf>> {
        let mut files: Vec<PathBuf> = Vec::new();

        let walker = WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                if e.file_type().is_dir() {
                    let name = e.file_name().to_string_lossy();
                    !exclude_dirs.iter().any(|d| d == name.as_ref())
                } else {
                    true
                }
            });

        for entry in walker {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };

            if !entry.file_type().is_file() {
                continue;
            }

            let path = entry.path().to_path_buf();

            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();

            if ext == "cmake" {
                files.push(path);
            }
        }

        Ok(files)
    }

    /// Scan subdirectories for existing `CMakeLists.txt` files that were NOT
    /// generated by fb-gen (no "Generated by fb-gen" header).
    ///
    /// Returns paths to the parent directories (relative to root) of user-owned
    /// CMakeLists.txt files.  These directories should be added as
    /// `add_subdirectory` in the root CMakeLists.txt USER_START block.
    pub fn scan_user_cmake_files(
        &self,
        root: &Path,
        exclude_dirs: &[String],
    ) -> Vec<PathBuf> {
        let mut user_dirs: Vec<PathBuf> = Vec::new();

        let walker = WalkDir::new(root)
            .max_depth(3)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                if e.file_type().is_dir() {
                    let name = e.file_name().to_string_lossy();
                    !exclude_dirs.iter().any(|d| d == name.as_ref())
                } else {
                    true
                }
            });

        for entry in walker {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };

            if !entry.file_type().is_file() {
                continue;
            }

            if entry.file_name() != "CMakeLists.txt" {
                continue;
            }

            // Skip root CMakeLists.txt — only interested in subdirectories.
            let path = entry.path();
            if path.parent().map_or(true, |p| p == root) {
                continue;
            }

            // Read and check if fb-gen generated.
            let content = match std::fs::read_to_string(path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            if !content.contains("Generated by fb-gen") {
                // User-owned CMakeLists.txt — record its parent directory.
                if let Some(parent) = path.parent() {
                    if let Ok(rel) = parent.strip_prefix(root) {
                        if !rel.as_os_str().is_empty() {
                            user_dirs.push(rel.to_path_buf());
                        }
                    }
                }
            }
        }

        user_dirs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scan_includes_extracts_local_includes() {
        let dir = tempfile::tempdir().unwrap();
        let header = dir.path().join("test.h");
        let source = dir.path().join("test.cpp");
        std::fs::write(
            &source,
            "#include \"test.h\"\n#include <vector>\n#include \"other.hpp\"\n",
        )
        .unwrap();
        std::fs::write(&header, "").unwrap();

        let scanner = FffScanner::new(dir.path());
        let includes = scanner.scan_includes(&source).unwrap();
        assert_eq!(includes, vec!["test.h", "other.hpp"]);
    }

    #[test]
    fn test_scan_source_files_filters_by_extension() {
        let dir = tempfile::tempdir().unwrap();

        let a_cpp = dir.path().join("a.cpp");
        let b_h = dir.path().join("b.h");
        let c_txt = dir.path().join("c.txt");
        std::fs::write(&a_cpp, "#include \"b.h\"\n").unwrap();
        std::fs::write(&b_h, "").unwrap();
        std::fs::write(&c_txt, "hello").unwrap();

        let scanner = FffScanner::new(dir.path());
        let options = ScanOptions {
            root: dir.path().to_path_buf(),
            ..Default::default()
        };

        let sources = scanner.scan_source_files(&options).unwrap();

        // Should only find .cpp and .h, not .txt
        assert_eq!(sources.len(), 2);
        let names: Vec<&str> = sources.iter().map(|s| s.file_name.as_str()).collect();
        assert!(names.contains(&"a.cpp"));
        assert!(names.contains(&"b.h"));
        assert!(!names.contains(&"c.txt"));
    }

    #[test]
    fn test_exclude_dirs_are_skipped() {
        let dir = tempfile::tempdir().unwrap();

        let build_dir = dir.path().join("build");
        std::fs::create_dir(&build_dir).unwrap();
        let ignored = build_dir.join("ignored.cpp");
        std::fs::write(&ignored, "").unwrap();

        let src = dir.path().join("main.cpp");
        std::fs::write(&src, "").unwrap();

        let scanner = FffScanner::new(dir.path());
        let options = ScanOptions {
            root: dir.path().to_path_buf(),
            ..Default::default()
        };

        let sources = scanner.scan_source_files(&options).unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].file_name, "main.cpp");
    }
}
