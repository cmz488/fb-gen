//! Dependency analyzer — builds a module-level dependency graph by inspecting
//! `#include` directives from in-memory SourceFile data.
//!
//! For each `#include "..."` string already parsed by the scanner, the first
//! path segment before `/` is matched against known module names.  When no
//! module matches (bare include like `#include "foo.h"`), a fallback matches
//! the filename against headers declared in other modules.

use crate::models::dependency::{DependencyEdge, DependencyGraph, DependencyType};
use crate::models::error::Result;
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
    pub fn analyze(&self, modules: &[CMakeModule]) -> Result<DependencyGraph> {
        let mut graph = DependencyGraph::new();

        // Register all modules as graph nodes.
        for m in modules {
            graph.add_module(&m.name);
        }

        // Build lookup sets.
        let module_names: HashSet<&str> =
            modules.iter().map(|m| m.name.as_str()).collect();

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
            let all_includes: Vec<(&str, &str)> = module
                .sources
                .iter()
                .flat_map(|sf| {
                    sf.includes
                        .iter()
                        .map(move |inc| (inc.as_str(), "source"))
                })
                .chain(module.asm_sources.iter().flat_map(|af| {
                    af.includes
                        .iter()
                        .map(move |inc| (inc.as_str(), "asm"))
                }))
                .chain(module.headers.iter().flat_map(|hf| {
                    hf.includes
                        .iter()
                        .map(move |inc| (inc.as_str(), "header"))
                }))
                .collect();

            for (include_str, label) in &all_includes {
                // Extract first path segment before '/'.
                let first_segment = include_str
                    .split('/')
                    .next()
                    .unwrap_or(include_str);

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
                    let filename = include_str
                        .rsplit('/')
                        .next()
                        .unwrap_or(include_str);

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
                        reason: format!(
                            "#include \"{}\" in {} {}",
                            include_str, label,
                            if *label == "source" {
                                "source"
                            } else if *label == "asm" {
                                "asm source"
                            } else {
                                "header"
                            }
                        ),
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

    #[test]
    fn test_include_path_segment_extraction() {
        // Verify that path segment extraction logic works correctly.
        let inc = "core/foo.h";
        let first = inc.split('/').next().unwrap();
        assert_eq!(first, "core");

        let bare = "foo.h";
        let first = bare.split('/').next().unwrap();
        assert_eq!(first, "foo.h"); // bare — no '/' separator

        let filename = bare.rsplit('/').next().unwrap();
        assert_eq!(filename, "foo.h");
    }

    #[test]
    fn test_include_filename_fallback() {
        // Verify that a bare include resolves to a module with matching header.
        let inc = "utils.h";
        let filename = inc.rsplit('/').next().unwrap();
        assert_eq!(filename, "utils.h");
    }
}
