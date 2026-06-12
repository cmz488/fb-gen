//! File watcher — detects changes by comparing file checksums between runs.

use crate::orchestration::cache::MetaCache;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Detects changed files by comparing current checksums against a
/// previously-stored snapshot.
pub struct FileWatcher {
    /// Project root directory for relative-path reporting.
    root: PathBuf,
    /// Directory names to exclude from scanning (e.g. `build`, `.git`).
    exclude_dirs: Vec<String>,
}

impl FileWatcher {
    /// Create a new watcher for the given project root.
    ///
    /// `exclude_dirs` contains directory *names* (not full paths) that
    /// should be skipped during file discovery.
    pub fn new(root: &Path, exclude_dirs: Vec<String>) -> Self {
        Self {
            root: root.to_path_buf(),
            exclude_dirs,
        }
    }

    /// Walk the project root and collect all source files.
    ///
    /// Only files whose extension matches a known C/C++ source or header
    /// extension are included.
    fn collect_source_files(&self) -> Vec<PathBuf> {
        let exts: &[&str] = &["c", "cc", "cpp", "cxx", "c++", "h", "hh", "hpp", "hxx", "h++"];
        let mut files = Vec::new();

        let walker = walkdir::WalkDir::new(&self.root)
            .into_iter()
            .filter_entry(|e| {
                // Skip excluded directories by name.
                if e.file_type().is_dir() {
                    if let Some(name) = e.file_name().to_str() {
                        return !self.exclude_dirs.iter().any(|d| d == name);
                    }
                }
                true
            });

        for entry in walker.filter_map(|e| e.ok()) {
            if entry.file_type().is_file() {
                if let Some(ext) = entry.path().extension().and_then(|e| e.to_str()) {
                    if exts.contains(&ext.to_lowercase().as_str()) {
                        files.push(entry.path().to_path_buf());
                    }
                }
            }
        }

        files
    }

    /// Compare current file checksums against a previously-saved snapshot.
    ///
    /// Returns a list of paths for files that are **new**, **modified**,
    /// or **deleted** relative to the snapshot.
    pub fn get_changes(&self, prev_checksums: &HashMap<String, String>) -> Vec<PathBuf> {
        let cache = MetaCache::new(&self.root);
        let current_files = self.collect_source_files();
        let current_checksums = cache.compute_checksums(&current_files);

        let mut changed = Vec::new();

        // Files that are new or modified.
        for (path_str, new_hash) in &current_checksums {
            match prev_checksums.get(path_str) {
                Some(old_hash) if old_hash == new_hash => {
                    // Unchanged — skip.
                }
                _ => {
                    changed.push(PathBuf::from(path_str));
                }
            }
        }

        // Files that existed before but are now missing (deleted).
        for path_str in prev_checksums.keys() {
            if !current_checksums.contains_key(path_str) {
                changed.push(PathBuf::from(path_str));
            }
        }

        changed
    }

    /// Check if any source files have changed since the last checksum
    /// snapshot.
    pub fn has_changes(&self, prev_checksums: &HashMap<String, String>) -> bool {
        !self.get_changes(prev_checksums).is_empty()
    }
}

// ── tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_file(dir: &Path, name: &str, content: &[u8]) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn no_changes_when_files_identical() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        write_file(root, "main.cpp", b"int main() { return 0; }");
        write_file(root, "utils.h", b"#pragma once");

        let watcher = FileWatcher::new(root, vec!["build".into(), ".git".into()]);

        // Compute "previous" checksums from the current state.
        let cache = MetaCache::new(root);
        let files = watcher.collect_source_files();
        let prev = cache.compute_checksums(&files);

        // Now compare — nothing should have changed.
        let changes = watcher.get_changes(&prev);
        assert!(changes.is_empty());
        assert!(!watcher.has_changes(&prev));
    }

    #[test]
    fn detects_new_file() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        write_file(root, "old.cpp", b"// old");

        let cache = MetaCache::new(root);
        let watcher = FileWatcher::new(root, vec![]);

        // Snapshot current state as "previous".
        let files_before = watcher.collect_source_files();
        let prev = cache.compute_checksums(&files_before);

        // Add a new file after the snapshot.
        let new_file = write_file(root, "new.cpp", b"// new");

        let changes = watcher.get_changes(&prev);
        assert!(changes.iter().any(|p| p == &new_file));
    }

    #[test]
    fn detects_modified_file() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        let file = write_file(root, "mod.cpp", b"original");

        let cache = MetaCache::new(root);
        let watcher = FileWatcher::new(root, vec![]);

        // Take snapshot of original content.
        let files_before = watcher.collect_source_files();
        let prev = cache.compute_checksums(&files_before);

        // Modify the file.
        std::fs::write(&file, b"modified content").unwrap();

        let changes = watcher.get_changes(&prev);
        assert!(changes.iter().any(|p| p == &file));
    }

    #[test]
    fn detects_deleted_file() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        let file = write_file(root, "del.cpp", b"to be deleted");

        let cache = MetaCache::new(root);
        let watcher = FileWatcher::new(root, vec![]);

        // Take snapshot.
        let files_before = watcher.collect_source_files();
        let prev = cache.compute_checksums(&files_before);

        // Delete the file.
        std::fs::remove_file(&file).unwrap();

        let changes = watcher.get_changes(&prev);
        assert!(changes.iter().any(|p| p == &file));
    }

    #[test]
    fn excludes_dirs_by_name() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        // Create a source file inside a build directory — should be excluded.
        let build_dir = root.join("build");
        std::fs::create_dir(&build_dir).unwrap();
        write_file(&build_dir, "ignored.cpp", b"// in build dir");

        // Create a source file at project root — should be included.
        let main_file = write_file(root, "main.cpp", b"int main(){}");

        let watcher = FileWatcher::new(root, vec!["build".into()]);
        let files = watcher.collect_source_files();

        assert!(files.iter().any(|p| p == &main_file));
        // The file inside "build" should NOT appear.
        for f in &files {
            assert!(!f.to_string_lossy().contains("build"));
        }
    }
}
