use fb_gen::core::analyzer::DependencyAnalyzer;
use fb_gen::core::discoverer::{ModuleDiscoverer, ScanOptions};
use fb_gen::core::generator::CMakeGenerator;
use fb_gen::models::dependency::DependencyGraph;
use fb_gen::models::module::SourceFile;
use fb_gen::models::project::ProjectConfig;
use fb_gen::scanner::fff_wrapper::FffScanner;

use std::path::PathBuf;
use tempfile::TempDir;

// ── Helper: create a multi-module test project ────────────────────

/// Creates a test project with the following structure:
/// ```text
/// tmp/
/// ├── CMakePresets.json
/// ├── link.ld
/// ├── core/
/// │   ├── core.c          → #include "utils/utils.h"
/// │   └── core.h
/// ├── utils/
/// │   ├── utils.c         → no includes
/// │   └── utils.h
/// ├── app/
/// │   ├── main.c          → #include "core/core.h"
/// │   └── app.h
/// └── asm/
///     └── startup.s
/// ```
fn create_test_project(dir: &TempDir) {
    let root = dir.path();

    // CMakePresets.json at root.
    std::fs::write(
        root.join("CMakePresets.json"),
        r#"{
  "version": 3,
  "configurePresets": [
    { "name": "debug", "generator": "Ninja", "binaryDir": "build/debug" },
    { "name": "release", "generator": "Ninja", "binaryDir": "build/release" }
  ],
  "buildPresets": [
    { "name": "debug", "configurePreset": "debug" },
    { "name": "release", "configurePreset": "release" }
  ]
}"#,
    )
    .unwrap();

    // Linker script at root.
    std::fs::write(
        root.join("link.ld"),
        r#"MEMORY
{
  FLASH (rx) : ORIGIN = 0x08000000, LENGTH = 512K
  RAM  (rwx) : ORIGIN = 0x20000000, LENGTH = 128K
}
ENTRY(Reset_Handler)
"#,
    )
    .unwrap();

    // core/
    let core_dir = root.join("core");
    std::fs::create_dir(&core_dir).unwrap();
    std::fs::write(
        core_dir.join("core.c"),
        "#include \"utils/utils.h\"\n\nint core_func() { return 0; }\n",
    )
    .unwrap();
    std::fs::write(core_dir.join("core.h"), "#pragma once\nint core_func();\n").unwrap();

    // utils/
    let utils_dir = root.join("utils");
    std::fs::create_dir(&utils_dir).unwrap();
    std::fs::write(
        utils_dir.join("utils.c"),
        "int util_func() { return 42; }\n",
    )
    .unwrap();
    std::fs::write(utils_dir.join("utils.h"), "#pragma once\nint util_func();\n").unwrap();

    // app/
    let app_dir = root.join("app");
    std::fs::create_dir(&app_dir).unwrap();
    std::fs::write(
        app_dir.join("main.c"),
        "#include \"core/core.h\"\n\nint main() { return core_func(); }\n",
    )
    .unwrap();
    std::fs::write(app_dir.join("app.h"), "#pragma once\nvoid app_init();\n").unwrap();

    // asm/
    let asm_dir = root.join("asm");
    std::fs::create_dir(&asm_dir).unwrap();
    std::fs::write(
        asm_dir.join("startup.s"),
        ".section .text\n.globl _start\n_start:\n  bl main\n  b .\n",
    )
    .unwrap();
}

/// Scan a directory and return the list of SourceFiles.
fn scan_project(root: &PathBuf) -> Vec<SourceFile> {
    let scanner = FffScanner::new(root);
    let options = fb_gen::scanner::fs_adapter::ScanOptions {
        root: root.clone(),
        ..Default::default()
    };
    scanner.scan_source_files(&options).unwrap()
}

// ── Tests ────────────────────────────────────────────────────────

#[test]
fn test_include_parsing() {
    // Verify that `#include "..."` directives are correctly extracted.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    let source = root.join("main.c");
    std::fs::write(
        &source,
        "#include \"core/core.h\"\n#include <stdio.h>\n#include \"utils/util.h\"\n\nint main() {}\n",
    )
    .unwrap();

    let scanner = FffScanner::new(root);
    let includes = scanner.scan_includes(&source).unwrap();

    // Only quoted includes should be captured; angle-bracket includes are ignored.
    assert_eq!(includes, vec!["core/core.h", "utils/util.h"]);
}

#[test]
fn test_module_discovery() {
    // Verify that ModuleDiscoverer correctly groups source files by directory.
    let tmp = TempDir::new().unwrap();
    create_test_project(&tmp);

    let root = tmp.path().to_path_buf();
    let sources = scan_project(&root);

    // Sources should be from 3 directories: core, utils, app.
    let discoverer = ModuleDiscoverer::new(ScanOptions {
        exclude_dirs: vec![],
        root: root.clone(),
    });
    let modules = discoverer.discover(&sources).unwrap();

    // 4 non-root modules: core, utils, app, asm.
    // (Root module may also exist if linker scripts are orphaned.)
    let non_root_modules: Vec<_> = modules.iter().filter(|m| !m.is_root).collect();
    assert_eq!(
        non_root_modules.len(),
        4,
        "expected 4 modules (core, utils, app, asm), got: {:?}",
        modules.iter().map(|m| &m.name).collect::<Vec<_>>()
    );

    let module_names: Vec<&str> = modules.iter().map(|m| m.name.as_str()).collect();
    assert!(module_names.contains(&"core"), "missing core module");
    assert!(module_names.contains(&"utils"), "missing utils module");
    assert!(module_names.contains(&"app"), "missing app module");
    assert!(module_names.contains(&"asm"), "missing asm module");

    // The app module should be marked as Executable because main.c has int main().
    let app_module = modules.iter().find(|m| m.name == "app").unwrap();
    assert!(app_module.has_main, "app module should detect main()");
}

#[test]
fn test_dependency_analysis() {
    // Verify that dependency analysis produces a correct graph.
    let tmp = TempDir::new().unwrap();
    create_test_project(&tmp);

    let root = tmp.path().to_path_buf();
    let sources = scan_project(&root);

    let discoverer = ModuleDiscoverer::new(ScanOptions {
        exclude_dirs: vec![],
        root: root.clone(),
    });
    let modules = discoverer.discover(&sources).unwrap();

    let analyzer = DependencyAnalyzer::new();
    let graph = analyzer.analyze(&modules).unwrap();

    // core depends on utils (via #include "utils/utils.h")
    let core_deps = graph.get_dependencies("core");
    assert!(
        core_deps.iter().any(|(n, _)| n == "utils"),
        "core should depend on utils, got: {:?}",
        core_deps
    );

    // app depends on core (via #include "core/core.h")
    let app_deps = graph.get_dependencies("app");
    assert!(
        app_deps.iter().any(|(n, _)| n == "core"),
        "app should depend on core, got: {:?}",
        app_deps
    );

    // utils has no dependencies.
    let utils_deps = graph.get_dependencies("utils");
    assert!(utils_deps.is_empty(), "utils should have no dependencies");

    // Graph should have 5 nodes (root, core, utils, app, asm) and 2 edges.
    assert_eq!(graph.node_count(), 5);
    assert_eq!(graph.edge_count(), 2);
}

#[test]
fn test_topological_order() {
    // Verify that topological sort produces a valid build order.
    let tmp = TempDir::new().unwrap();
    create_test_project(&tmp);

    let root = tmp.path().to_path_buf();
    let sources = scan_project(&root);

    let discoverer = ModuleDiscoverer::new(ScanOptions {
        exclude_dirs: vec![],
        root: root.clone(),
    });
    let modules = discoverer.discover(&sources).unwrap();

    let analyzer = DependencyAnalyzer::new();
    let graph = analyzer.analyze(&modules).unwrap();

    let order = graph.topological_order().unwrap();

    // utils → core → app is the expected build order.
    // utils has no deps, core depends on utils, app depends on core.
    let utils_pos = order.iter().position(|n| n == "utils").unwrap();
    let core_pos = order.iter().position(|n| n == "core").unwrap();
    let app_pos = order.iter().position(|n| n == "app").unwrap();

    assert!(
        utils_pos < core_pos,
        "utils must come before core in topological order: {:?}",
        order
    );
    assert!(
        core_pos < app_pos,
        "core must come before app in topological order: {:?}",
        order
    );
}

#[test]
fn test_cycle_detection() {
    // Verify that cyclic dependencies are detected.
    let mut graph = DependencyGraph::new();

    graph.add_module("a");
    graph.add_module("b");

    // a → b → a creates a cycle.
    graph.add_dependency(fb_gen::models::dependency::DependencyEdge {
        from: "a".into(),
        to: "b".into(),
        dep_type: fb_gen::models::dependency::DependencyType::Private,
        reason: "test".into(),
    });
    graph.add_dependency(fb_gen::models::dependency::DependencyEdge {
        from: "b".into(),
        to: "a".into(),
        dep_type: fb_gen::models::dependency::DependencyType::Private,
        reason: "test".into(),
    });

    assert!(graph.has_cycles(), "graph with a→b→a should have a cycle");

    // topological_order should return an error for cyclic graphs.
    let result = graph.topological_order();
    assert!(
        result.is_err(),
        "topological_order should fail for cyclic graph"
    );
}

#[test]
fn test_cmake_generation() {
    // Verify that CMakeLists.txt is generated with expected content.
    let tmp = TempDir::new().unwrap();
    create_test_project(&tmp);

    let root = tmp.path().to_path_buf();
    let sources = scan_project(&root);

    let discoverer = ModuleDiscoverer::new(ScanOptions {
        exclude_dirs: vec![],
        root: root.clone(),
    });
    let modules = discoverer.discover(&sources).unwrap();

    let analyzer = DependencyAnalyzer::new();
    let graph = analyzer.analyze(&modules).unwrap();

    let config = ProjectConfig {
        name: "TestProject".into(),
        version: "0.1.0".into(),
        root: root.clone(),
        language: "C".into(),
        c_standard: "11".into(),
        cpp_standard: "17".into(),
        ..Default::default()
    };

    let generator = CMakeGenerator::new(&config).unwrap();
    generator.generate(&modules, &graph, true).unwrap();

    // Root CMakeLists.txt should exist.
    let root_cmake = root.join("CMakeLists.txt");
    assert!(root_cmake.exists(), "root CMakeLists.txt should exist");

    let root_content = std::fs::read_to_string(&root_cmake).unwrap();

    // Check for expected keywords in root CMakeLists.txt.
    assert!(
        root_content.contains("cmake_minimum_required"),
        "root CMakeLists.txt should set cmake_minimum_required"
    );
    assert!(
        root_content.contains("project(TestProject"),
        "root CMakeLists.txt should declare the project name"
    );
    assert!(
        root_content.contains("add_subdirectory"),
        "root CMakeLists.txt should have add_subdirectory calls"
    );

    // Per-module CMakeLists.txt should exist.
    let core_cmake = root.join("core").join("CMakeLists.txt");
    let utils_cmake = root.join("utils").join("CMakeLists.txt");
    let app_cmake = root.join("app").join("CMakeLists.txt");

    assert!(core_cmake.exists(), "core/CMakeLists.txt should exist");
    assert!(utils_cmake.exists(), "utils/CMakeLists.txt should exist");
    assert!(app_cmake.exists(), "app/CMakeLists.txt should exist");

    // app module should generate add_executable because it has main().
    let app_content = std::fs::read_to_string(&app_cmake).unwrap();
    assert!(
        app_content.contains("add_executable"),
        "app module (with main) should use add_executable, got:\n{}",
        app_content
    );

    // Check that dependency is linked in core's CMakeLists.txt.
    let core_content = std::fs::read_to_string(&core_cmake).unwrap();
    assert!(
        core_content.contains("target_link_libraries"),
        "core module should have target_link_libraries because it depends on utils"
    );
}

#[test]
fn test_asm_file_detection() {
    // Verify that .s assembly files are detected and assigned to module asm_sources.
    let tmp = TempDir::new().unwrap();
    create_test_project(&tmp);

    let root = tmp.path().to_path_buf();
    let sources = scan_project(&root);

    let discoverer = ModuleDiscoverer::new(ScanOptions {
        exclude_dirs: vec![],
        root: root.clone(),
    });
    let modules = discoverer.discover(&sources).unwrap();

    // Find the asm module.
    let asm_module = modules
        .iter()
        .find(|m| m.name == "asm")
        .expect("asm module should exist");

    assert!(
        !asm_module.asm_sources.is_empty(),
        "asm module should have asm_sources, got 0"
    );
    assert!(
        asm_module
            .asm_sources
            .iter()
            .any(|a| a.file_name == "startup.s"),
        "asm module should contain startup.s"
    );
}

#[test]
fn test_linker_script_detection() {
    // Verify that .ld linker scripts are detected and assigned to a module.
    let tmp = TempDir::new().unwrap();
    create_test_project(&tmp);

    let root = tmp.path().to_path_buf();
    let sources = scan_project(&root);

    let discoverer = ModuleDiscoverer::new(ScanOptions {
        exclude_dirs: vec![],
        root: root.clone(),
    });
    let modules = discoverer.discover(&sources).unwrap();

    // The root module should have the orphaned linker script.
    let root_module = modules
        .iter()
        .find(|m| m.is_root)
        .expect("root module should exist (for orphan linker script)");

    assert!(
        !root_module.linker_scripts.is_empty(),
        "root module should have linker_scripts"
    );
    assert!(
        root_module
            .linker_scripts
            .iter()
            .any(|p| p.to_string_lossy().contains("link.ld")),
        "root module should contain link.ld"
    );
}

#[test]
fn test_presets_detection() {
    // Verify that CMakePresets.json is parsed correctly.
    let tmp = TempDir::new().unwrap();
    create_test_project(&tmp);

    let root = tmp.path().to_path_buf();
    let scanner = FffScanner::new(&root);
    let presets = scanner
        .scan_presets(&root)
        .unwrap()
        .expect("CMakePresets.json should be found and parsed");

    assert_eq!(presets.version, 3);
    assert_eq!(presets.configure_presets.len(), 2);
    assert_eq!(presets.configure_presets[0].name, "debug");
    assert_eq!(presets.configure_presets[1].name, "release");
    assert_eq!(presets.build_presets.len(), 2);
}

#[test]
fn test_cross_compile_template() {
    // Verify that NoneEabi target arch produces a complete toolchain.cmake file
    // with TOOLCHAIN_PREFIX, MCU flags, compiler/linker settings, NOT in root CMakeLists.txt.
    let tmp = TempDir::new().unwrap();
    create_test_project(&tmp);

    let root = tmp.path().to_path_buf();
    let sources = scan_project(&root);

    let discoverer = ModuleDiscoverer::new(ScanOptions {
        exclude_dirs: vec![],
        root: root.clone(),
    });
    let modules = discoverer.discover(&sources).unwrap();

    let config = ProjectConfig {
        name: "CrossTest".into(),
        version: "0.1.0".into(),
        root: root.clone(),
        language: "C".into(),
        c_standard: "11".into(),
        cpp_standard: "17".into(),
        target_arch: fb_gen::models::project::TargetArch::NoneEabi,
        mcu_flags: "cortex-m3".into(),
        ..Default::default()
    };

    let generator = CMakeGenerator::new(&config).unwrap();
    let empty_graph = DependencyGraph::new();
    generator.generate(&modules, &empty_graph, true).unwrap();

    // Toolchain file should be created at cmake/toolchain.cmake.
    let toolchain_path = root.join("cmake").join("toolchain.cmake");
    assert!(
        toolchain_path.exists(),
        "cross-compile should generate cmake/toolchain.cmake, but it was not found"
    );

    let toolchain_content = std::fs::read_to_string(&toolchain_path).unwrap();

    // ── Compiler identification ──────────────────────────────────────
    assert!(
        toolchain_content.contains("CMAKE_SYSTEM_NAME Generic"),
        "toolchain.cmake should set CMAKE_SYSTEM_NAME Generic"
    );
    assert!(
        toolchain_content.contains("CMAKE_SYSTEM_PROCESSOR arm"),
        "toolchain.cmake should set CMAKE_SYSTEM_PROCESSOR arm"
    );
    assert!(
        toolchain_content.contains("CMAKE_C_COMPILER_ID GNU"),
        "toolchain.cmake should set CMAKE_C_COMPILER_ID GNU"
    );

    // ── TOOLCHAIN_PREFIX ─────────────────────────────────────────────
    assert!(
        toolchain_content.contains("TOOLCHAIN_PREFIX arm-none-eabi-"),
        "toolchain.cmake should set TOOLCHAIN_PREFIX arm-none-eabi-,\ngot:\n{}",
        toolchain_content
    );
    assert!(
        toolchain_content.contains("${TOOLCHAIN_PREFIX}gcc"),
        "toolchain.cmake should use TOOLCHAIN_PREFIX for C compiler"
    );
    assert!(
        toolchain_content.contains("${TOOLCHAIN_PREFIX}g++"),
        "toolchain.cmake should use TOOLCHAIN_PREFIX for C++ compiler"
    );

    // ── Executable suffix (bare-metal ELF) ───────────────────────────
    assert!(
        toolchain_content.contains("CMAKE_EXECUTABLE_SUFFIX"),
        "toolchain.cmake should set CMAKE_EXECUTABLE_SUFFIX"
    );

    // ── Try-compile guard ────────────────────────────────────────────
    assert!(
        toolchain_content.contains("CMAKE_TRY_COMPILE_TARGET_TYPE STATIC_LIBRARY"),
        "toolchain.cmake should set CMAKE_TRY_COMPILE_TARGET_TYPE STATIC_LIBRARY"
    );

    // ── MCU flags ────────────────────────────────────────────────────
    assert!(
        toolchain_content.contains("-mcpu=cortex-m3"),
        "toolchain.cmake should contain -mcpu=cortex-m3 for ARM bare-metal,\ngot:\n{}",
        toolchain_content
    );

    // ── Compiler flags ───────────────────────────────────────────────
    assert!(
        toolchain_content.contains("CMAKE_C_FLAGS_DEBUG"),
        "toolchain.cmake should set CMAKE_C_FLAGS_DEBUG"
    );
    assert!(
        toolchain_content.contains("CMAKE_C_FLAGS_RELEASE"),
        "toolchain.cmake should set CMAKE_C_FLAGS_RELEASE"
    );
    assert!(
        toolchain_content.contains("-fdata-sections"),
        "toolchain.cmake should include -fdata-sections"
    );

    // ── Linker flags ─────────────────────────────────────────────────
    assert!(
        toolchain_content.contains("CMAKE_EXE_LINKER_FLAGS"),
        "toolchain.cmake should set CMAKE_EXE_LINKER_FLAGS"
    );
    assert!(
        toolchain_content.contains("--specs=nano.specs"),
        "toolchain.cmake should include --specs=nano.specs"
    );
    assert!(
        toolchain_content.contains("--gc-sections"),
        "toolchain.cmake should include --gc-sections"
    );
    assert!(
        toolchain_content.contains("--print-memory-usage"),
        "toolchain.cmake should include --print-memory-usage"
    );

    // ── Link libraries ───────────────────────────────────────────────
    assert!(
        toolchain_content.contains("TOOLCHAIN_LINK_LIBRARIES \"m\""),
        "toolchain.cmake should set TOOLCHAIN_LINK_LIBRARIES"
    );

    // ── Sanity: root CMakeLists.txt must NOT contain toolchain directives ──
    let root_cmake = root.join("CMakeLists.txt");
    let root_content = std::fs::read_to_string(&root_cmake).unwrap();
    assert!(
        !root_content.contains("arm-none-eabi-"),
        "root CMakeLists.txt should NOT contain compiler directives (moved to toolchain.cmake)"
    );
    assert!(
        !root_content.contains("CMAKE_SYSTEM_NAME"),
        "root CMakeLists.txt should NOT contain CMAKE_SYSTEM_NAME (moved to toolchain.cmake)"
    );
}

#[test]
fn test_toolchain_arm64() {
    // Verify ARM64 generates aarch64-none-elf- toolchain with cortex-a53.
    let tmp = TempDir::new().unwrap();
    create_test_project(&tmp);

    let root = tmp.path().to_path_buf();
    let sources = scan_project(&root);

    let discoverer = ModuleDiscoverer::new(ScanOptions {
        exclude_dirs: vec![],
        root: root.clone(),
    });
    let modules = discoverer.discover(&sources).unwrap();

    let config = ProjectConfig {
        name: "ARM64Test".into(),
        version: "0.1.0".into(),
        root: root.clone(),
        target_arch: fb_gen::models::project::TargetArch::ARM64,
        mcu_flags: "cortex-a53".into(),
        ..Default::default()
    };

    let generator = CMakeGenerator::new(&config).unwrap();
    let empty_graph = DependencyGraph::new();
    generator.generate(&modules, &empty_graph, true).unwrap();

    let toolchain_path = root.join("cmake").join("toolchain.cmake");
    assert!(toolchain_path.exists());

    let content = std::fs::read_to_string(&toolchain_path).unwrap();
    assert!(
        content.contains("TOOLCHAIN_PREFIX aarch64-none-elf-"),
        "ARM64 toolchain should use aarch64-none-elf- prefix"
    );
    assert!(
        content.contains("-mcpu=cortex-a53"),
        "ARM64 toolchain should use cortex-a53 MCU flags"
    );
    assert!(
        content.contains("CMAKE_SYSTEM_PROCESSOR aarch64"),
        "ARM64 toolchain should set processor to aarch64"
    );
}

#[test]
fn test_toolchain_riscv64() {
    // Verify RISCV64 generates riscv64-unknown-elf- toolchain without MCU flags.
    let tmp = TempDir::new().unwrap();
    create_test_project(&tmp);

    let root = tmp.path().to_path_buf();
    let sources = scan_project(&root);

    let discoverer = ModuleDiscoverer::new(ScanOptions {
        exclude_dirs: vec![],
        root: root.clone(),
    });
    let modules = discoverer.discover(&sources).unwrap();

    let config = ProjectConfig {
        name: "RISCVTest".into(),
        version: "0.1.0".into(),
        root: root.clone(),
        target_arch: fb_gen::models::project::TargetArch::RISCV64,
        ..Default::default()
    };

    let generator = CMakeGenerator::new(&config).unwrap();
    let empty_graph = DependencyGraph::new();
    generator.generate(&modules, &empty_graph, true).unwrap();

    let toolchain_path = root.join("cmake").join("toolchain.cmake");
    assert!(toolchain_path.exists());

    let content = std::fs::read_to_string(&toolchain_path).unwrap();
    assert!(
        content.contains("TOOLCHAIN_PREFIX riscv64-unknown-elf-"),
        "RISCV64 toolchain should use riscv64-unknown-elf- prefix"
    );
    assert!(
        content.contains("CMAKE_SYSTEM_PROCESSOR riscv64"),
        "RISCV64 toolchain should set processor to riscv64"
    );
    // RISCV64 should NOT have an -mcpu flag (empty MCU).
    assert!(
        !content.contains("-mcpu="),
        "RISCV64 toolchain should NOT contain -mcpu= flag (empty MCU)"
    );
    // The TARGET_FLAGS line should still exist, just with an empty value.
    assert!(
        content.contains("set(TARGET_FLAGS "),
        "RISCV64 toolchain should still have the TARGET_FLAGS variable"
    );
}

#[test]
fn test_toolchain_not_generated_for_x86() {
    // X86_64 should NOT generate a toolchain file.
    let tmp = TempDir::new().unwrap();
    create_test_project(&tmp);

    let root = tmp.path().to_path_buf();
    let sources = scan_project(&root);

    let discoverer = ModuleDiscoverer::new(ScanOptions {
        exclude_dirs: vec![],
        root: root.clone(),
    });
    let modules = discoverer.discover(&sources).unwrap();

    let config = ProjectConfig {
        name: "X86Test".into(),
        version: "0.1.0".into(),
        root: root.clone(),
        target_arch: fb_gen::models::project::TargetArch::X86_64,
        ..Default::default()
    };

    let generator = CMakeGenerator::new(&config).unwrap();
    let empty_graph = DependencyGraph::new();
    generator.generate(&modules, &empty_graph, true).unwrap();

    // Toolchain file should NOT exist for x86_64.
    let toolchain_path = root.join("cmake").join("toolchain.cmake");
    assert!(
        !toolchain_path.exists(),
        "toolchain.cmake should NOT be generated for X86_64 target"
    );
}

#[test]
fn test_bare_include_dependency() {
    // Verify that a bare include (no path prefix) creates a dependency edge
    // by matching the included filename against other modules' headers.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().to_path_buf();

    // Module A: single source file with bare include of "b.h"
    let a_dir = root.join("a");
    std::fs::create_dir(&a_dir).unwrap();
    std::fs::write(
        a_dir.join("a.c"),
        "#include \"b.h\"\n\nint a_func() { return b_func(); }\n",
    )
    .unwrap();

    // Module B: header file b.h
    let b_dir = root.join("b");
    std::fs::create_dir(&b_dir).unwrap();
    std::fs::write(b_dir.join("b.h"), "#pragma once\nint b_func();\n").unwrap();
    std::fs::write(b_dir.join("b.c"), "#include \"b.h\"\n\nint b_func() { return 0; }\n").unwrap();

    // Scan
    let scanner = FffScanner::new(&root);
    let opts = fb_gen::scanner::fs_adapter::ScanOptions {
        root: root.clone(),
        ..Default::default()
    };
    let sources = scanner.scan_source_files(&opts).unwrap();

    // Discover
    let discoverer = ModuleDiscoverer::new(ScanOptions {
        exclude_dirs: vec![],
        root: root.clone(),
    });
    let modules = discoverer.discover(&sources).unwrap();

    // Analyze
    let analyzer = DependencyAnalyzer::new();
    let graph = analyzer.analyze(&modules).unwrap();

    // Module "a" should depend on module "b" because a.c includes "b.h"
    // and b.h lives in module "b".
    let a_deps = graph.get_dependencies("a");
    assert!(
        a_deps.iter().any(|(n, _)| n == "b"),
        "module 'a' should depend on 'b' via bare #include \"b.h\", got: {:?}",
        a_deps
    );

    // Module "b" should have no dependencies (b.c only includes its own header).
    let b_deps = graph.get_dependencies("b");
    assert!(
        b_deps.is_empty(),
        "module 'b' should have no dependencies, got: {:?}",
        b_deps
    );
}

#[test]
fn test_bare_include_no_self_dependency() {
    // A module's bare include of its own header should NOT create a self-dependency.
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().to_path_buf();

    // Module A: source that includes its own header
    let a_dir = root.join("a");
    std::fs::create_dir(&a_dir).unwrap();
    std::fs::write(a_dir.join("a.h"), "#pragma once\nint a_func();\n").unwrap();
    std::fs::write(
        a_dir.join("a.c"),
        "#include \"a.h\"\n\nint a_func() { return 0; }\n",
    )
    .unwrap();

    let scanner = FffScanner::new(&root);
    let opts = fb_gen::scanner::fs_adapter::ScanOptions {
        root: root.clone(),
        ..Default::default()
    };
    let sources = scanner.scan_source_files(&opts).unwrap();

    let discoverer = ModuleDiscoverer::new(ScanOptions {
        exclude_dirs: vec![],
        root: root.clone(),
    });
    let modules = discoverer.discover(&sources).unwrap();

    let analyzer = DependencyAnalyzer::new();
    let graph = analyzer.analyze(&modules).unwrap();

    // Module "a" should have zero dependencies — #include "a.h" is self-referential.
    let a_deps = graph.get_dependencies("a");
    assert!(
        a_deps.is_empty(),
        "module 'a' should have no self-dependencies, got: {:?}",
        a_deps
    );
}
