//! Dependency analyzer — builds a module-level dependency graph by inspecting
//! `#include` directives from in-memory SourceFile data.
//!
//! For each `#include "..."` string already parsed by the scanner, the first
//! path segment before `/` is matched against known module names.  When no
//! module matches (bare include like `#include "foo.h"`), a fallback matches
//! the filename against headers declared in other modules.

use crate::models::dependency::{DependencyEdge, DependencyGraph, DependencyType};
use crate::models::error::FbGenResult;
use crate::models::module::CMakeModule;
use std::collections::HashSet;

/// Analyses `#include` directives across modules to build a dependency graph.
///
/// All data comes from the already-parsed `SourceFile::includes` fields —
/// no files are read from disk.
pub struct DependencyAnalyzer;

impl DependencyAnalyzer {
    pub fn new() -> Self {
        Self
    }

    /// Analyze all modules and return a dependency graph.
    ///
    /// For every source, header, and assembly file in every module, each
    /// `#include "..."` string is processed:
    ///
    /// 1. **Path-segment match** (existing logic): for `#include "core/foo.h"`,
    ///    the first path segment `core` is extracted and matched against full
    ///    module names, then short directory basenames.
    ///
    /// 2. **Filename fallback** (new): for bare includes like `#include "foo.h"`
    ///    with no `/` separator, the filename `foo.h` is looked up in all other
    ///    modules' `headers` lists. If found, a `PUBLIC` dependency edge is
    ///    created.
    ///
    /// Assembly sources (`.S`) are included because they go through the C
    /// preprocessor and may `#include` module headers.
    pub fn analyze(&self, modules: &[CMakeModule]) -> FbGenResult<DependencyGraph> {
        let mut graph = DependencyGraph::new();

        // Register all modules as graph nodes.
        for m in modules {
            graph.add_module(&m.name);
        }

        // Build lookup sets.
        let module_names: HashSet<&str> = modules.iter().map(|m| m.name.as_str()).collect();

        let short_names: HashSet<&str> = modules
            .iter()
            .filter_map(|m| m.relative_path.file_name().and_then(|n| n.to_str()))
            .collect();

        // Build a mapping: header filename → module name, for the fallback pass.
        let mut header_to_module: Vec<(&str, &str)> = Vec::new();
        for m in modules {
            for h in &m.headers {
                header_to_module.push((&h.file_name, &m.name));
            }
        }

        for module in modules {
            let all_includes: Vec<(&str, &str)> =
                module
                    .sources
                    .iter()
                    .flat_map(|sf| sf.includes.iter().map(move |inc| (inc.as_str(), "source")))
                    .chain(
                        module.asm_sources.iter().flat_map(|af| {
                            af.includes.iter().map(move |inc| (inc.as_str(), "asm"))
                        }),
                    )
                    .chain(
                        module.headers.iter().flat_map(|hf| {
                            hf.includes.iter().map(move |inc| (inc.as_str(), "header"))
                        }),
                    )
                    .collect();

            for (include_str, _label) in &all_includes {
                // Extract first path segment before '/'.
                let first_segment = include_str.split('/').next().unwrap_or(include_str);

                // Skip self-references.
                if first_segment == module.name {
                    continue;
                }

                // Try exact module name match first, then short-name match.
                let target: Option<String> = if module_names.contains(first_segment) {
                    Some(first_segment.to_string())
                } else if short_names.contains(first_segment) {
                    // Resolve short name to full module name.
                    modules
                        .iter()
                        .find(|m| {
                            m.relative_path
                                .file_name()
                                .and_then(|n| n.to_str())
                                .is_some_and(|n| n == first_segment)
                        })
                        .map(|m| m.name.clone())
                } else {
                    None
                };

                let target_name = if let Some(t) = target {
                    t
                } else {
                    // ── Filename fallback (bare include like "foo.h") ──
                    // Extract the filename (last path segment).
                    let filename = include_str.rsplit('/').next().unwrap_or(include_str);

                    // Find a module (other than self) whose headers contain this filename.
                    match header_to_module
                        .iter()
                        .find(|(fname, mname)| *fname == filename && *mname != module.name)
                    {
                        Some((_, mname)) => mname.to_string(),
                        None => continue, // no match — probably a system/external header
                    }
                };

                // Add the edge if it doesn't already exist.
                let existing = graph.get_dependencies(&module.name);
                if !existing.iter().any(|(n, _)| n == &target_name) {
                    graph.add_dependency(DependencyEdge {
                        from: module.name.clone(),
                        to: target_name,
                        dep_type: DependencyType::Public,
                    });
                }
            }
        }

        // Validate: detect cycles and warn (but don't fail — the caller decides).
        if graph.has_cycles() {
            eprintln!(
                "warning: dependency graph contains cycles — target_link_libraries may need manual adjustment"
            );
        }

        Ok(graph)
    }
}

impl Default for DependencyAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::module::{SourceFile, SourceType};
    use std::path::PathBuf;

    fn make_source(name: &str, source_type: SourceType, includes: Vec<&str>) -> SourceFile {
        SourceFile {
            path: PathBuf::from(name),
            relative_path: PathBuf::from(name),
            file_name: name.rsplit('/').next().unwrap_or(name).to_string(),
            source_type,
            includes: includes.into_iter().map(String::from).collect(),
        }
    }

    fn make_module(name: &str, rel_path: &str, sources: Vec<SourceFile>, headers: Vec<SourceFile>) -> CMakeModule {
        CMakeModule {
            name: name.into(),
            path: PathBuf::from(rel_path),
            relative_path: PathBuf::from(rel_path),
            sources,
            headers,
            asm_sources: vec![],
            linker_scripts: vec![],
            dependencies: vec![],
            target_type: crate::models::module::TargetType::StaticLibrary,
            is_root: false,
            has_main: false,
            compile_features: vec![],
            compile_definitions: vec![],
            include_dirs: vec![PathBuf::from(rel_path)],
        }
    }

    #[test]
    fn test_analyze_empty_modules() {
        let analyzer = DependencyAnalyzer::new();
        let graph = analyzer.analyze(&[]).unwrap();
        assert_eq!(graph.node_count(), 0);
        assert_eq!(graph.edge_count(), 0);
    }

    #[test]
    fn test_analyze_no_includes() {
        let a = make_module("a", "a", vec![make_source("a/a.c", SourceType::CSource, vec![])], vec![]);
        let b = make_module("b", "b", vec![make_source("b/b.c", SourceType::CSource, vec![])], vec![]);
        let analyzer = DependencyAnalyzer::new();
        let graph = analyzer.analyze(&[a, b]).unwrap();
        assert_eq!(graph.node_count(), 2);
        assert_eq!(graph.edge_count(), 0);
    }

    #[test]
    fn test_analyze_path_segment_dependency() {
        let a = make_module("a", "a", vec![make_source("a/a.c", SourceType::CSource, vec!["b/b.h"])], vec![]);
        let b = make_module("b", "b", vec![make_source("b/b.c", SourceType::CSource, vec![])], vec![make_source("b/b.h", SourceType::CHeader, vec![])]);
        let analyzer = DependencyAnalyzer::new();
        let graph = analyzer.analyze(&[a, b]).unwrap();
        let a_deps = graph.get_dependencies("a");
        assert!(a_deps.iter().any(|(n, _)| n == "b"), "a should depend on b via #include \"b/b.h\"");
    }

    #[test]
    fn test_analyze_self_reference_excluded() {
        let a = make_module("a", "a", vec![make_source("a/a.c", SourceType::CSource, vec!["a.h"])], vec![make_source("a/a.h", SourceType::CHeader, vec![])]);
        let analyzer = DependencyAnalyzer::new();
        let graph = analyzer.analyze(&[a]).unwrap();
        assert!(graph.get_dependencies("a").is_empty(), "self-reference via own header should be excluded");
    }

    #[test]
    fn test_analyze_bare_include_fallback() {
        let a = make_module("a", "a", vec![make_source("a/a.c", SourceType::CSource, vec!["utils.h"])], vec![]);
        let b = make_module("b", "b", vec![make_source("b/b.c", SourceType::CSource, vec![])], vec![make_source("b/utils.h", SourceType::CHeader, vec![])]);
        let analyzer = DependencyAnalyzer::new();
        let graph = analyzer.analyze(&[a, b]).unwrap();
        let a_deps = graph.get_dependencies("a");
        assert!(a_deps.iter().any(|(n, _)| n == "b"), "a should depend on b via bare include \"utils.h\"");
    }
}
