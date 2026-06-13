//! Module discoverer — groups source files by directory into CMake modules.
//!
//! Each directory containing >=1 `.c`/`.cpp` file becomes a `CMakeModule`.
//! Header files are assigned to the module of their containing directory.
//! Assembly (`.s`/`.S`) and linker script (`.ld`) files are also discovered.
//! Modules with `main()` → `Executable`; header-only dirs → `HeaderOnly`;
//! asm+header-only dirs → `StaticLibrary`; default → `StaticLibrary`.

use crate::models::error::FbGenResult;
use crate::models::module::{CMakeModule, SourceFile, TargetType};
use regex::Regex;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Options controlling module discovery behaviour.
#[derive(Debug, Clone)]
pub struct ScanOptions {
    /// Directories to exclude (e.g. `build`, `.git`, `third_party`).
    pub exclude_dirs: Vec<String>,
    /// Project root directory (used to compute relative paths).
    pub root: PathBuf,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            exclude_dirs: vec![
                "build".into(),
                ".git".into(),
                "third_party".into(),
                "cmake-build-debug".into(),
                "cmake-build-release".into(),
                "CMakeFiles".into(),
                ".idea".into(),
                ".vscode".into(),
            ],
            root: PathBuf::from("."),
        }
    }
}

/// Discovers CMake modules from a list of scanned source files.
pub struct ModuleDiscoverer {
    options: ScanOptions,
}

impl ModuleDiscoverer {
    /// Create a new discoverer with the given scan options.
    pub fn new(options: ScanOptions) -> Self {
        Self { options }
    }

    /// Discover CMake modules by grouping `sources` by their parent directory.
    ///
    /// * Every directory containing ≥1 `.c`/`.cpp` file becomes a module.
    /// * Header files in a directory are assigned to that directory's module.
    /// * Assembly (`.s`/`.S`) files are assigned to the directory's module.
    /// * If a module contains a file with a `main()` function it is marked `Executable`.
    /// * A directory with *only* header files becomes `HeaderOnly`.
    /// * A directory with *only* assembly + header files becomes `StaticLibrary`.
    /// * Linker scripts (`.ld`) are assigned to their directory's module if one
    ///   exists; otherwise they are placed in the root module.
    /// * All other modules default to `StaticLibrary`.
    pub fn discover(&self, sources: &[SourceFile]) -> FbGenResult<Vec<CMakeModule>> {
        // Group files by their parent directory (relative to project root).
        let mut dirs: BTreeMap<PathBuf, Vec<&SourceFile>> = BTreeMap::new();

        for sf in sources {
            // Determine the directory containing this file.
            let parent = sf.relative_path.parent().unwrap_or_else(|| Path::new("."));

            // Skip excluded directories.
            if self.is_excluded(parent) {
                continue;
            }

            dirs.entry(parent.to_path_buf()).or_default().push(sf);
        }

        let mut modules: Vec<CMakeModule> = Vec::new();
        // Linker scripts that don't belong to an existing module will be added
        // to the root module at the end.
        let mut orphan_linker_scripts: Vec<PathBuf> = Vec::new();

        for (dir, files) in &dirs {
            let sources: Vec<&SourceFile> = files
                .iter()
                .filter(|f| f.source_type.is_source())
                .copied()
                .collect();
            let headers: Vec<&SourceFile> = files
                .iter()
                .filter(|f| f.source_type.is_header())
                .copied()
                .collect();
            let asm: Vec<&SourceFile> = files
                .iter()
                .filter(|f| f.source_type.is_asm())
                .copied()
                .collect();
            let linkers: Vec<PathBuf> = files
                .iter()
                .filter(|f| f.source_type.is_linker())
                .map(|f| f.path.clone())
                .collect();

            // Only create a module if the directory has source files, asm files,
            // or headers.
            if sources.is_empty() && headers.is_empty() && asm.is_empty() {
                // Linker scripts in directories without a module become orphans.
                orphan_linker_scripts.extend(linkers);
                continue;
            }

            let module_name = CMakeModule::sanitize_name(dir);
            let has_main = sources.iter().any(|sf| has_main_function(sf));
            let target_type = if !sources.is_empty() && has_main {
                TargetType::Executable
            } else if sources.is_empty() && asm.is_empty() {
                // Header-only — no compilable sources in this directory.
                TargetType::HeaderOnly
            } else {
                TargetType::StaticLibrary
            };

            // Collect include directories: current dir + any subdir with headers.
            let include_dirs = collect_include_dirs(dir, files);

            let module = CMakeModule {
                name: module_name,
                path: self.options.root.join(dir),
                relative_path: dir.clone(),
                sources: sources.iter().map(|&s| s.clone()).collect(),
                headers: headers.iter().map(|&h| h.clone()).collect(),
                asm_sources: asm.iter().map(|&a| a.clone()).collect(),
                linker_scripts: linkers,
                dependencies: vec![],
                target_type,
                is_root: dir == Path::new(".") || dir.as_os_str().is_empty(),
                has_main,
                compile_features: vec![],
                compile_definitions: vec![],
                include_dirs,
                user_config: None,
            };

            modules.push(module);
        }

        // Assign orphan linker scripts to the root module.
        if !orphan_linker_scripts.is_empty() {
            if let Some(root_module) = modules.iter_mut().find(|m| m.is_root) {
                root_module.linker_scripts.extend(orphan_linker_scripts);
            } else {
                // Create a minimal root module if none exists.
                let root_module = CMakeModule {
                    name: String::new(),
                    path: self.options.root.clone(),
                    relative_path: PathBuf::from("."),
                    sources: vec![],
                    headers: vec![],
                    asm_sources: vec![],
                    linker_scripts: orphan_linker_scripts,
                    dependencies: vec![],
                    target_type: TargetType::StaticLibrary,
                    is_root: true,
                    has_main: false,
                    compile_features: vec![],
                    compile_definitions: vec![],
                    include_dirs: vec![PathBuf::from(".")],
                    user_config: None,
                };
                modules.push(root_module);
            }
        }

        // Merge root-level source/asm files into the executable module.
        // Root-level compilable files (e.g. startup_*.s for embedded projects)
        // would otherwise be silently dropped because the generator skips
        // `is_root` modules.  Root modules that only hold linker scripts or
        // headers are left alone — linker scripts are collected from all
        // modules and emitted in the root CMakeLists.txt by render_root().
        if let Some(root_idx) = modules.iter().position(|m| m.is_root) {
            let has_compilable = !modules[root_idx].sources.is_empty()
                || !modules[root_idx].asm_sources.is_empty();

            if has_compilable {
                if let Some(exe_idx) = modules
                    .iter()
                    .position(|m| m.target_type == TargetType::Executable && !m.is_root)
                {
                    // Merge root files into the executable module.
                    let mut root = modules.remove(root_idx);
                    let exe = &mut modules[if exe_idx > root_idx { exe_idx - 1 } else { exe_idx }];
                    exe.sources.append(&mut root.sources);
                    exe.asm_sources.append(&mut root.asm_sources);
                    exe.headers.append(&mut root.headers);
                    exe.linker_scripts.append(&mut root.linker_scripts);
                    // Merge include dirs (avoid duplicates).
                    for d in root.include_dirs {
                        if !exe.include_dirs.contains(&d) {
                            exe.include_dirs.push(d);
                        }
                    }
                } else {
                    // No executable module — keep the root module but let the
                    // generator produce a CMakeLists.txt for it.
                    modules[root_idx].is_root = false;
                    if modules[root_idx].name.is_empty() {
                        modules[root_idx].name = "root".into();
                    }
                }
            }
        }

        if modules.is_empty() {
            return Err(crate::models::error::FbGenError::NoSources(
                self.options.root.display().to_string(),
            ));
        }

        Ok(modules)
    }

    /// Check whether a directory path should be excluded.
    fn is_excluded(&self, dir: &Path) -> bool {
        self.options.exclude_dirs.iter().any(|ex| {
            // Match if any path component equals the excluded name.
            dir.components()
                .any(|c| c.as_os_str().to_str().is_some_and(|s| s == ex.as_str()))
        })
    }
}

/// Detect the presence of a `main()` function by reading file content.
///
/// Searches for `int main(` or `void main(` patterns using a regex.
fn has_main_function(sf: &SourceFile) -> bool {
    let content = match std::fs::read_to_string(&sf.path) {
        Ok(c) => c,
        Err(_) => return false,
    };

    // Match `int main(` or `void main(` — handles whitespace between return type and name.
    let re = Regex::new(r"(?:int|void)\s+main\s*\(").unwrap();
    re.is_match(&content)
}

/// Collect include directories for a module.
///
/// The current directory is always included. Additionally, any subdirectory
/// that contains header files is added as an include path.
fn collect_include_dirs(dir: &Path, files: &[&SourceFile]) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = vec![dir.to_path_buf()];

    for sf in files.iter().filter(|f| f.source_type.is_header()) {
        if let Some(hdr_dir) = sf.relative_path.parent() {
            if hdr_dir != dir && !dirs.contains(&hdr_dir.to_path_buf()) {
                dirs.push(hdr_dir.to_path_buf());
            }
        }
    }

    dirs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::module::{SourceFile, SourceType};

    #[allow(dead_code)]
    fn make_source(path: &str, st: SourceType, includes: Vec<&str>) -> SourceFile {
        SourceFile {
            path: PathBuf::from(path),
            relative_path: PathBuf::from(path),
            file_name: Path::new(path)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_string(),
            source_type: st,
            includes: includes.iter().map(|s| s.to_string()).collect(),
            size_bytes: 0,
        }
    }

    #[test]
    fn test_has_main_detection() {
        // We can't easily test with real files on disk, but the regex logic is simple.
        // Test the regex directly.
        let re = Regex::new(r"(?:int|void)\s+main\s*\(").unwrap();
        assert!(re.is_match("int main(int argc, char** argv) {"));
        assert!(re.is_match("void main() {"));
        assert!(re.is_match("int   main  (  ) {"));
        assert!(!re.is_match("int main_foo() {"));
    }

    #[test]
    fn test_sanitize_name() {
        assert_eq!(CMakeModule::sanitize_name(Path::new(".")), "");
        assert_eq!(CMakeModule::sanitize_name(Path::new("")), "");
        assert_eq!(CMakeModule::sanitize_name(Path::new("src")), "src");
        assert_eq!(
            CMakeModule::sanitize_name(Path::new("src/core")),
            "src_core"
        );
        assert_eq!(CMakeModule::sanitize_name(Path::new("my-lib")), "my_lib");
    }

    #[test]
    fn test_is_excluded() {
        let opts = ScanOptions::default();
        let d = ModuleDiscoverer::new(opts);
        assert!(d.is_excluded(Path::new("build")));
        assert!(d.is_excluded(Path::new("src/build/subdir")));
        assert!(!d.is_excluded(Path::new("src")));
        assert!(!d.is_excluded(Path::new(".")));
    }
}
