//! Configuration inferrer — scans source files to detect C++ language features
//! and recommends the appropriate C++ standard and compile features.
//!
//! The inferrer looks for keywords / patterns that indicate a minimum C++
//! standard is required (e.g. `constexpr` → C++11, `std::optional` → C++17,
//! `concepts` → C++20).

use crate::models::module::CMakeModule;
use regex::Regex;
use std::collections::HashSet;

/// Holds the inferred configuration for a project.
#[derive(Debug, Clone)]
pub struct InferredConfig {
    /// Recommended C++ standard version (e.g. `"17"`, `"20"`).
    pub cpp_standard: String,
    /// Recommended C standard version.
    pub c_standard: String,
    /// CMake compile features (e.g. `cxx_std_17`, `cxx_auto_type`).
    pub compile_features: Vec<String>,
    /// Detected feature keywords and where they were found.
    pub evidence: Vec<String>,
}

/// Scans source files to infer the minimum required C++ standard and a list of
/// recommended CMake compile features.
pub struct ConfigInferrer;

impl ConfigInferrer {
    /// Infer configuration from discovered modules.
    ///
    /// Returns a tuple of `(cpp_standard, compile_features)`.
    /// The standard is returned as a string like `"11"`, `"14"`, `"17"`, `"20"`, `"23"`.
    pub fn infer(modules: &[CMakeModule]) -> (String, Vec<String>) {
        let inferred = Self::infer_detailed(modules);
        (inferred.cpp_standard, inferred.compile_features)
    }

    /// Full inference with evidence tracking.
    pub fn infer_detailed(modules: &[CMakeModule]) -> InferredConfig {
        let mut features = HashSet::new();
        let mut max_standard = 11u32; // minimum we consider
        let mut evidence: Vec<String> = Vec::new();

        for module in modules {
            for sf in &module.sources {
                let content = match std::fs::read_to_string(&sf.path) {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                let file_name = &sf.file_name;

                // ── C++11 features ──────────────────────────────────────
                if has_keyword(&content, "auto") && !content.contains("auto*") {
                    // `auto` is prevalent; only flag if it looks like C++11 usage
                    if contains_regex(&content, r"\bauto\s+\w+\s*=") {
                        max_standard = max_standard.max(11);
                        features.insert("cxx_auto_type".to_string());
                        evidence.push(format!("auto (C++11) in {}", file_name));
                    }
                }

                if has_keyword(&content, "nullptr") {
                    max_standard = max_standard.max(11);
                    features.insert("cxx_nullptr".to_string());
                    evidence.push(format!("nullptr (C++11) in {}", file_name));
                }

                if contains_regex(&content, r"\bforeach\s*\(") || has_keyword(&content, "range-based for") {
                    max_standard = max_standard.max(11);
                    features.insert("cxx_range_for".to_string());
                    evidence.push(format!("range-for (C++11) in {}", file_name));
                }

                // ── C++14 features ──────────────────────────────────────
                if contains_regex(&content, r"\bdecltype\s*\(\s*auto\s*\)") {
                    max_standard = max_standard.max(14);
                    evidence.push(format!("decltype(auto) (C++14) in {}", file_name));
                }

                if contains_regex(&content, r"\bstd::make_unique\b") {
                    max_standard = max_standard.max(14);
                    evidence.push(format!("std::make_unique (C++14) in {}", file_name));
                }

                // ── C++17 features ──────────────────────────────────────
                if has_keyword(&content, "std::optional") {
                    max_standard = max_standard.max(17);
                    features.insert("cxx_std_17".to_string());
                    evidence.push(format!("std::optional (C++17) in {}", file_name));
                }

                if has_keyword(&content, "std::variant") {
                    max_standard = max_standard.max(17);
                    features.insert("cxx_std_17".to_string());
                    evidence.push(format!("std::variant (C++17) in {}", file_name));
                }

                if has_keyword(&content, "std::string_view") {
                    max_standard = max_standard.max(17);
                    features.insert("cxx_std_17".to_string());
                    evidence.push(format!("std::string_view (C++17) in {}", file_name));
                }

                if has_keyword(&content, "constexpr") && max_standard < 17 {
                    // constexpr exists since C++11, but widespread use + constexpr if/lambda → C++17
                    if contains_regex(&content, r"\bif\s+constexpr\b") {
                        max_standard = max_standard.max(17);
                        features.insert("cxx_std_17".to_string());
                        evidence.push(format!("if constexpr (C++17) in {}", file_name));
                    } else {
                        max_standard = max_standard.max(11);
                        features.insert("cxx_constexpr".to_string());
                        evidence.push(format!("constexpr (C++11) in {}", file_name));
                    }
                }

                if has_keyword(&content, "std::filesystem") {
                    max_standard = max_standard.max(17);
                    features.insert("cxx_std_17".to_string());
                    evidence.push(format!("std::filesystem (C++17) in {}", file_name));
                }

                if has_keyword(&content, "structured binding") || contains_regex(&content, r"\bauto\s*\[") {
                    max_standard = max_standard.max(17);
                    features.insert("cxx_std_17".to_string());
                    evidence.push(format!("structured bindings (C++17) in {}", file_name));
                }

                // ── C++20 features ──────────────────────────────────────
                if has_keyword(&content, "concept") || has_keyword(&content, "requires") {
                    // Only flag if it looks like C++20 concepts (not just the English word).
                    if contains_regex(&content, r"\bconcept\s+\w+\s*=")
                        || contains_regex(&content, r"\brequires\s+std::")
                    {
                        max_standard = max_standard.max(20);
                        features.insert("cxx_std_20".to_string());
                        evidence.push(format!("concepts/requires (C++20) in {}", file_name));
                    }
                }

                if has_keyword(&content, "std::span") {
                    max_standard = max_standard.max(20);
                    features.insert("cxx_std_20".to_string());
                    evidence.push(format!("std::span (C++20) in {}", file_name));
                }

                if has_keyword(&content, "std::jthread") || has_keyword(&content, "std::stop_token") {
                    max_standard = max_standard.max(20);
                    features.insert("cxx_std_20".to_string());
                    evidence.push(format!("std::jthread (C++20) in {}", file_name));
                }

                if contains_regex(&content, r"\bco_await\b|\bco_return\b|\bco_yield\b") {
                    max_standard = max_standard.max(20);
                    features.insert("cxx_std_20".to_string());
                    evidence.push(format!("coroutines (C++20) in {}", file_name));
                }

                // ── C++23 features ──────────────────────────────────────
                if has_keyword(&content, "std::expected") {
                    max_standard = max_standard.max(23);
                    features.insert("cxx_std_23".to_string());
                    evidence.push(format!("std::expected (C++23) in {}", file_name));
                }

                if has_keyword(&content, "std::flat_map") || has_keyword(&content, "std::flat_set") {
                    max_standard = max_standard.max(23);
                    features.insert("cxx_std_23".to_string());
                    evidence.push(format!("std::flat_map/set (C++23) in {}", file_name));
                }

                if has_keyword(&content, "import std;") || contains_regex(&content, r"\bimport\s+<") {
                    max_standard = max_standard.max(23);
                    features.insert("cxx_std_23".to_string());
                    evidence.push(format!("modules (C++23) in {}", file_name));
                }
            }
        }

        // Build the compile features list. Always include the minimum standard feature.
        let mut compile_features: Vec<String> = Vec::new();
        // Push the standard feature first.
        compile_features.push(format!("cxx_std_{}", max_standard));
        // Add additional feature flags (deduplicated via HashSet).
        for f in &features {
            if !compile_features.contains(f) {
                compile_features.push(f.clone());
            }
        }

        let c_standard = infer_c_standard(modules);

        InferredConfig {
            cpp_standard: max_standard.to_string(),
            c_standard,
            compile_features,
            evidence,
        }
    }
}

/// Check if `keyword` appears as a whole-word match.
fn has_keyword(content: &str, keyword: &str) -> bool {
    let pattern = format!(r"\b{}\b", regex::escape(keyword));
    Regex::new(&pattern).map(|re| re.is_match(content)).unwrap_or(false)
}

/// Check if content matches a given regex.
fn contains_regex(content: &str, pattern: &str) -> bool {
    Regex::new(pattern).map(|re| re.is_match(content)).unwrap_or(false)
}

/// Infer minimum C standard from source files.
fn infer_c_standard(modules: &[CMakeModule]) -> String {
    let mut max_std = 11u32; // C11 as baseline

    for module in modules {
        for sf in &module.sources {
            let content = match std::fs::read_to_string(&sf.path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            if has_keyword(&content, "_Generic") {
                max_std = max_std.max(11);
            }
            if has_keyword(&content, "_Static_assert") || has_keyword(&content, "static_assert") {
                max_std = max_std.max(11);
            }
            // C17/C23 indicators are rare; stay conservative.
        }
    }

    max_std.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_has_keyword() {
        assert!(has_keyword("auto x = 5;", "auto"));
        assert!(!has_keyword("automatic", "auto")); // whole-word only
        assert!(has_keyword("std::optional<int>", "std::optional"));
    }

    #[test]
    fn test_contains_regex() {
        assert!(contains_regex("auto x = 42;", r"\bauto\s+\w+\s*="));
        assert!(!contains_regex("auto* p", r"\bauto\s+\w+\s*=")); // auto* is a pointer, not auto-typed variable
    }
}
