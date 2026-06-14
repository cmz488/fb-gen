//! Filesystem adapter — scan options for source file discovery.

use std::path::PathBuf;

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
