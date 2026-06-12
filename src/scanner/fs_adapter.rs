//! Filesystem adapter — walks the project tree and collects source files.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Abstract filesystem operations, enabling testability via mocking.
pub trait FileSystem {
    /// Read the contents of a directory, returning entry paths.
    fn read_dir(&self, path: &Path) -> io::Result<Vec<PathBuf>>;

    /// Read the entire contents of a file into a String.
    fn read_file(&self, path: &Path) -> io::Result<String>;

    /// Write `contents` to a file, creating parent directories if needed.
    fn write_file(&self, path: &Path, contents: &str) -> io::Result<()>;

    /// Return true if the path exists.
    fn exists(&self, path: &Path) -> bool;

    /// Return true if the path is a directory.
    fn is_dir(&self, path: &Path) -> bool;

    /// Canonicalize a path (resolve symlinks, `.`, `..`).
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf>;
}

/// Production filesystem — delegates to `std::fs`.
pub struct RealFileSystem;

impl FileSystem for RealFileSystem {
    fn read_dir(&self, path: &Path) -> io::Result<Vec<PathBuf>> {
        let entries: Vec<PathBuf> = fs::read_dir(path)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .collect();
        Ok(entries)
    }

    fn read_file(&self, path: &Path) -> io::Result<String> {
        fs::read_to_string(path)
    }

    fn write_file(&self, path: &Path, contents: &str) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, contents)
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        fs::canonicalize(path)
    }
}

/// Options controlling which source files are discovered during a scan.
#[derive(Debug, Clone)]
pub struct ScanOptions {
    /// Project root directory.
    pub root: PathBuf,

    /// Directory names to skip (e.g. `build`, `.git`, `third_party`).
    pub exclude_dirs: Vec<String>,

    /// File extensions to include.
    pub languages: Vec<String>,

    /// Whether to follow symlinks when walking the tree.
    pub follow_symlinks: bool,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            root: PathBuf::from("."),
            exclude_dirs: vec![
                "build".into(),
                ".git".into(),
                ".svn".into(),
                "third_party".into(),
                "node_modules".into(),
                ".cache".into(),
                "target".into(),
                "CMakeFiles".into(),
                "cmake-build-debug".into(),
                "cmake-build-release".into(),
                ".idea".into(),
                ".vscode".into(),
            ],
            languages: vec![
                "c".into(),
                "cpp".into(),
                "cc".into(),
                "cxx".into(),
                "h".into(),
                "hpp".into(),
                "s".into(),
                "ld".into(),
            ],
            follow_symlinks: false,
        }
    }
}
