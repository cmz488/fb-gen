use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Type of source file detected during scanning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceType {
    CSource,
    CppSource,
    CHeader,
    CppHeader,
    AsmSource,      // .s .S 汇编源文件
    LinkerScript,   // .ld 链接脚本
    Other,
}

impl SourceType {
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_lowercase().as_str() {
            "c" => SourceType::CSource,
            "cc" | "cpp" | "cxx" | "c++" => SourceType::CppSource,
            "h" => SourceType::CHeader,
            "hh" | "hpp" | "hxx" | "h++" => SourceType::CppHeader,
            "s" => SourceType::AsmSource,
            "ld" => SourceType::LinkerScript,
            _ => SourceType::Other,
        }
    }

    pub fn is_source(&self) -> bool {
        matches!(self, SourceType::CSource | SourceType::CppSource)
    }

    pub fn is_header(&self) -> bool {
        matches!(self, SourceType::CHeader | SourceType::CppHeader)
    }

    pub fn is_asm(&self) -> bool {
        matches!(self, SourceType::AsmSource)
    }

    pub fn is_linker(&self) -> bool {
        matches!(self, SourceType::LinkerScript)
    }
}

/// Represents a single source or header file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceFile {
    pub path: PathBuf,
    pub relative_path: PathBuf,
    pub file_name: String,
    pub source_type: SourceType,
    pub includes: Vec<String>,
    pub size_bytes: u64,
}

/// The type of build target for a module.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TargetType {
    Executable,
    StaticLibrary,
    SharedLibrary,
    HeaderOnly,
}

/// Represents a CMake module (a logical build unit).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CMakeModule {
    pub name: String,
    pub path: PathBuf,
    pub relative_path: PathBuf,
    pub sources: Vec<SourceFile>,
    pub headers: Vec<SourceFile>,
    pub asm_sources: Vec<SourceFile>,
    pub linker_scripts: Vec<PathBuf>,
    pub dependencies: Vec<String>,
    pub target_type: TargetType,
    pub is_root: bool,
    pub has_main: bool,
    pub compile_features: Vec<String>,
    pub compile_definitions: Vec<String>,
    pub include_dirs: Vec<PathBuf>,
    pub user_config: Option<String>,
}

impl CMakeModule {
    /// Convert a directory path into a CMake-safe module name.
    ///
    /// Root directory (`.` or empty path) returns an empty string.
    /// Nested directories use underscores as separators
    /// (e.g. `src/core` → `src_core`).
    pub fn sanitize_name(dir: &Path) -> String {
        if dir == Path::new(".") || dir.as_os_str().is_empty() {
            String::new()
        } else {
            dir.to_string_lossy()
                .replace('/', "_")
                .replace('\\', "_")
                .replace('-', "_")
        }
    }
}
