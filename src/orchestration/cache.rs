//! Meta-cache layer — persists ProjectMeta and file checksums to disk
//! so incremental runs can skip unchanged files.

use crate::models::ProjectMeta;
use crate::models::{FbGenError, FbGenResult};
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Default sub-directory under the project root used for cache storage.
const CACHE_DIR: &str = ".fb-gen/cache";

/// Simple non-cryptographic hash function for file checksums.
///
/// This is a variant of djb2 that produces a u64 hash. It is fast and
/// sufficient for detecting content changes — we are not guarding against
/// malicious collisions.
fn djb2_hash(data: &[u8]) -> u64 {
    let mut hash: u64 = 5381;
    for &byte in data {
        hash = hash.wrapping_mul(33).wrapping_add(byte as u64);
    }
    hash
}

/// Persists `ProjectMeta` and file checksums under a `.fb-gen/cache/`
/// directory inside the project root.
pub struct MetaCache {
    /// Absolute path to the cache directory (e.g. `<root>/.fb-gen/cache`).
    cache_dir: PathBuf,
}

impl MetaCache {
    /// Create a new cache instance rooted at `project_root`.
    ///
    /// The actual cache files live in `project_root/.fb-gen/cache/`.
    pub fn new(project_root: &Path) -> Self {
        Self {
            cache_dir: project_root.join(CACHE_DIR),
        }
    }

    /// Return the path to the cache directory.
    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    /// Ensure the cache directory exists on disk.
    fn ensure_dir(&self) -> FbGenResult<()> {
        std::fs::create_dir_all(&self.cache_dir).map_err(FbGenError::Io)
    }

    // ── project.json ──────────────────────────────────────────────

    /// Path to the project-level metadata file.
    fn project_json(&self) -> PathBuf {
        self.cache_dir.join("project.json")
    }

    /// Path to the modules array file.
    fn modules_json(&self) -> PathBuf {
        self.cache_dir.join("modules.json")
    }

    /// Path to the checksums map file.
    fn checksums_json(&self) -> PathBuf {
        self.cache_dir.join("checksums.json")
    }

    // ── public API ────────────────────────────────────────────────

    /// Persist the full `ProjectMeta` to three JSON files under the cache
    /// directory: `project.json`, `modules.json`, `checksums.json`.
    pub fn save(&self, meta: &ProjectMeta) -> FbGenResult<()> {
        self.ensure_dir()?;

        // Write project-level metadata (excluding modules & checksums).
        let mut project = serde_json::json!({
            "name": meta.config.name,
            "version": meta.config.version,
            "root": meta.config.root,
            "language": meta.config.language,
            "c_standard": meta.config.c_standard,
            "cpp_standard": meta.config.cpp_standard,
            "target_arch": meta.config.target_arch,
            "compiler": meta.config.compiler,
            "build_backend": meta.config.build_backend,
            "cmake_min_version": meta.config.cmake_min_version,
            "exclude_dirs": meta.config.exclude_dirs,
            "output_dir": meta.config.output_dir,
            "enable_watch": meta.config.enable_watch,
            "generated_at": meta.config.generated_at,
            "dependency_graph": meta.dependency_graph,
            "last_sync": meta.last_sync,
        });

        // Include cmake_presets if present.
        if let Some(ref presets) = meta.config.cmake_presets {
            if let Ok(val) = serde_json::to_value(presets) {
                project["cmake_presets"] = val;
            }
        }

        // Include toolchain_files.
        if !meta.config.toolchain_files.is_empty() {
            if let Ok(val) = serde_json::to_value(&meta.config.toolchain_files) {
                project["toolchain_files"] = val;
            }
        }

        // Include toolchain config if present.
        if let Some(ref tc) = meta.config.toolchain {
            if let Ok(val) = serde_json::to_value(tc) {
                project["toolchain"] = val;
            }
        }

        let project_bytes = serde_json::to_vec_pretty(&project)
            .map_err(|e| FbGenError::Serialization(e.to_string()))?;
        std::fs::write(self.project_json(), &project_bytes).map_err(FbGenError::Io)?;

        // Write modules separately.
        let modules_bytes = serde_json::to_vec_pretty(&meta.modules)
            .map_err(|e| FbGenError::Serialization(e.to_string()))?;
        std::fs::write(self.modules_json(), &modules_bytes).map_err(FbGenError::Io)?;

        // Write checksums separately.
        let checksums_bytes = serde_json::to_vec_pretty(&meta.file_checksums)
            .map_err(|e| FbGenError::Serialization(e.to_string()))?;
        std::fs::write(self.checksums_json(), &checksums_bytes).map_err(FbGenError::Io)?;

        Ok(())
    }

    /// Attempt to load the cached `ProjectMeta` from disk.
    ///
    /// Returns `None` when any of the three cache files is missing or
    /// cannot be deserialised.
    pub fn load(&self) -> Option<ProjectMeta> {
        let project_json: serde_json::Value = {
            let bytes = std::fs::read(self.project_json()).ok()?;
            serde_json::from_slice(&bytes).ok()?
        };

        let modules: Vec<crate::models::CMakeModule> = {
            let bytes = std::fs::read(self.modules_json()).ok()?;
            serde_json::from_slice(&bytes).ok()?
        };

        let file_checksums: HashMap<String, String> = {
            let bytes = std::fs::read(self.checksums_json()).ok()?;
            serde_json::from_slice(&bytes).ok()?
        };

        let config = crate::models::ProjectConfig {
            name: project_json.get("name")?.as_str()?.to_string(),
            version: project_json
                .get("version")
                .and_then(|v| v.as_str())
                .unwrap_or("0.1.0")
                .to_string(),
            root: PathBuf::from(project_json.get("root")?.as_str()?),
            language: project_json
                .get("language")
                .and_then(|v| v.as_str())
                .unwrap_or("CXX")
                .to_string(),
            c_standard: project_json
                .get("c_standard")
                .and_then(|v| v.as_str())
                .unwrap_or("11")
                .to_string(),
            cpp_standard: project_json
                .get("cpp_standard")
                .and_then(|v| v.as_str())
                .unwrap_or("17")
                .to_string(),
            target_arch: serde_json::from_value(project_json.get("target_arch")?.clone())
                .unwrap_or(crate::models::TargetArch::X86_64),
            compiler: serde_json::from_value(project_json.get("compiler")?.clone())
                .unwrap_or(crate::models::Compiler::GCC),
            build_backend: serde_json::from_value(project_json.get("build_backend")?.clone())
                .unwrap_or_default(),
            cmake_min_version: project_json
                .get("cmake_min_version")
                .and_then(|v| v.as_str())
                .unwrap_or("3.16")
                .to_string(),
            exclude_dirs: project_json
                .get("exclude_dirs")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
            output_dir: PathBuf::from(
                project_json
                    .get("output_dir")
                    .and_then(|v| v.as_str())
                    .unwrap_or("build"),
            ),
            enable_watch: project_json
                .get("enable_watch")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            modules: modules.clone(),
            generated_at: project_json
                .get("generated_at")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            cmake_presets: project_json
                .get("cmake_presets")
                .and_then(|v| serde_json::from_value(v.clone()).ok()),
            toolchain_files: project_json
                .get("toolchain_files")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default(),
            toolchain: project_json
                .get("toolchain")
                .and_then(|v| serde_json::from_value(v.clone()).ok()),
        };

        let dependency_graph: crate::models::DependencySnapshot =
            serde_json::from_value(project_json.get("dependency_graph")?.clone()).ok()?;

        let last_sync = project_json
            .get("last_sync")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        Some(ProjectMeta {
            config,
            modules,
            dependency_graph,
            file_checksums,
            last_sync,
        })
    }

    /// Returns `true` when all three cache files exist on disk.
    pub fn exists(&self) -> bool {
        self.project_json().exists()
            && self.modules_json().exists()
            && self.checksums_json().exists()
    }

    /// Remove all cached files without removing the cache directory itself.
    pub fn clear(&self) -> FbGenResult<()> {
        if self.project_json().exists() {
            std::fs::remove_file(self.project_json()).map_err(FbGenError::Io)?;
        }
        if self.modules_json().exists() {
            std::fs::remove_file(self.modules_json()).map_err(FbGenError::Io)?;
        }
        if self.checksums_json().exists() {
            std::fs::remove_file(self.checksums_json()).map_err(FbGenError::Io)?;
        }
        Ok(())
    }

    /// Compute djb2 checksums for a set of files.
    ///
    /// Returns a map from the string representation of the file path to a
    /// hex-encoded u64 checksum. Files that cannot be read are silently
    /// skipped.
    pub fn compute_checksums(&self, files: &[PathBuf]) -> HashMap<String, String> {
        let mut checksums = HashMap::with_capacity(files.len());
        for path in files {
            if let Ok(data) = std::fs::read(path) {
                let hash = djb2_hash(&data);
                checksums.insert(path.to_string_lossy().to_string(), format!("{hash:016x}"));
            }
        }
        checksums
    }

    /// Persist only the checksums map (used during incremental updates).
    pub fn save_checksums(&self, checksums: &HashMap<String, String>) -> FbGenResult<()> {
        self.ensure_dir()?;
        let mut file = std::fs::File::create(self.checksums_json()).map_err(FbGenError::Io)?;
        let json = serde_json::to_string_pretty(checksums)
            .map_err(|e| FbGenError::Serialization(e.to_string()))?;
        file.write_all(json.as_bytes()).map_err(FbGenError::Io)?;
        Ok(())
    }
}

// ── tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{BuildBackend, Compiler, DependencySnapshot, ProjectConfig, TargetArch};
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn make_meta() -> ProjectMeta {
        ProjectMeta {
            config: ProjectConfig {
                name: "test-cache".into(),
                version: "0.1.0".into(),
                root: PathBuf::from("/tmp/test"),
                language: "CXX".into(),
                c_standard: "11".into(),
                cpp_standard: "17".into(),
                target_arch: TargetArch::X86_64,
                compiler: Compiler::GCC,
                build_backend: BuildBackend::Ninja,
                cmake_min_version: "3.16".into(),
                exclude_dirs: vec![],
                output_dir: PathBuf::from("build"),
                enable_watch: false,
                modules: vec![],
                generated_at: String::new(),
                cmake_presets: None,
                toolchain_files: vec![],
                toolchain: None,
            },
            modules: vec![],
            dependency_graph: DependencySnapshot {
                nodes: vec![],
                edges: vec![],
            },
            file_checksums: HashMap::new(),
            last_sync: "2025-01-01T00:00:00Z".into(),
        }
    }

    #[test]
    fn save_and_load_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let cache = MetaCache::new(tmp.path());
        assert!(!cache.exists());

        let meta = make_meta();
        cache.save(&meta).unwrap();
        assert!(cache.exists());

        let loaded = cache.load().unwrap();
        assert_eq!(loaded.config.name, "test-cache");
        assert_eq!(loaded.last_sync, "2025-01-01T00:00:00Z");
    }

    #[test]
    fn load_missing_returns_none() {
        let tmp = TempDir::new().unwrap();
        let cache = MetaCache::new(tmp.path());
        assert!(cache.load().is_none());
    }

    #[test]
    fn clear_removes_files() {
        let tmp = TempDir::new().unwrap();
        let cache = MetaCache::new(tmp.path());
        cache.save(&make_meta()).unwrap();
        assert!(cache.exists());

        cache.clear().unwrap();
        assert!(!cache.exists());
    }

    #[test]
    fn compute_checksums_for_files() {
        let tmp = TempDir::new().unwrap();
        let file_a = tmp.path().join("a.txt");
        let file_b = tmp.path().join("b.txt");
        std::fs::write(&file_a, b"hello").unwrap();
        std::fs::write(&file_b, b"world").unwrap();

        let cache = MetaCache::new(tmp.path());
        let checksums = cache.compute_checksums(&[file_a.clone(), file_b.clone()]);

        assert_eq!(checksums.len(), 2);
        let hash_a = checksums
            .get(&file_a.to_string_lossy().to_string())
            .unwrap();
        let hash_b = checksums
            .get(&file_b.to_string_lossy().to_string())
            .unwrap();
        assert_ne!(hash_a, hash_b); // different content → different hash
    }

    #[test]
    fn compute_checksums_skip_unreadable() {
        let tmp = TempDir::new().unwrap();
        let missing = tmp.path().join("does_not_exist.txt");
        let exists = tmp.path().join("exists.txt");
        std::fs::write(&exists, b"data").unwrap();

        let cache = MetaCache::new(tmp.path());
        let checksums = cache.compute_checksums(&[missing, exists.clone()]);

        assert_eq!(checksums.len(), 1);
        assert!(checksums.contains_key(&exists.to_string_lossy().to_string()));
    }

    #[test]
    fn hash_stability() {
        // djb2 should produce the same hash for the same input.
        let h1 = djb2_hash(b"hello world");
        let h2 = djb2_hash(b"hello world");
        assert_eq!(h1, h2);
    }
}
