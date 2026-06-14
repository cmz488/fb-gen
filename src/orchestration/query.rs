//! Interactive user queries — collect project configuration via stdin.

use crate::models::{BuildBackend, Compiler, FbGenResult, ProjectConfig, TargetArch};
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

/// Reads a single line from stdin after printing a prompt.
fn prompt(prompt_text: &str) -> io::Result<String> {
    let mut stdout = io::stdout();
    print!("{prompt_text}");
    stdout.flush()?;

    let stdin = io::stdin();
    let mut line = String::new();
    stdin.lock().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

/// Reads a line with a default value, shown in the prompt.
fn prompt_with_default(prompt_text: &str, default: &str) -> io::Result<String> {
    let full_prompt = format!("{prompt_text} [{default}]: ");
    let answer = prompt(&full_prompt)?;
    if answer.is_empty() {
        Ok(default.to_string())
    } else {
        Ok(answer)
    }
}

/// Collects project configuration interactively from the user.
pub struct UserQuery;

impl UserQuery {
    /// Walk the user through all project-configuration questions and
    /// return a populated `ProjectConfig`.
    pub fn ask_project_config(root: &PathBuf, build_hint: Option<&str>) -> FbGenResult<ProjectConfig> {
        println!();
        println!("  Welcome to fb-gen — Fast Build Generate");
        println!("  Let's set up your project configuration.");
        println!();

        // ── project name ──────────────────────────────────────────
        let default_name = root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("untitled");
        let name = prompt_with_default("Project name", default_name).map_err(|e| {
            crate::models::FbGenError::Config(format!("failed to read project name: {e}"))
        })?;

        // ── language ──────────────────────────────────────────────
        println!();
        println!("  Language options:");
        println!("    1) C   — C only");
        println!("    2) CXX — C++ (default)");
        let lang_choice = prompt_with_default("Choose language [1-2]", "2").map_err(|e| {
            crate::models::FbGenError::Config(format!("failed to read language: {e}"))
        })?;
        let language = match lang_choice.as_str() {
            "1" => "C".to_string(),
            _ => "CXX".to_string(),
        };

        // ── C standard (only relevant for C projects) ────────────
        let c_standard = if language == "C" {
            prompt_with_default("C standard (e.g. 11, 17, 23)", "11").map_err(|e| {
                crate::models::FbGenError::Config(format!("failed to read C standard: {e}"))
            })?
        } else {
            "11".to_string()
        };

        // ── C++ standard (only relevant for C++ projects) ─────────
        let cpp_standard = if language == "CXX" {
            prompt_with_default("C++ standard (e.g. 14, 17, 20, 23)", "17").map_err(|e| {
                crate::models::FbGenError::Config(format!("failed to read C++ standard: {e}"))
            })?
        } else {
            "17".to_string()
        };

        // ── build system ───────────────────────────────────────────
        let build_system = match build_hint {
            Some("zig") => {
                println!();
                println!("  Build system: Zig (from --build flag)");
                crate::models::project::BuildSystem::Zig
            }
            Some("cmake") => {
                println!();
                println!("  Build system: CMake (from --build flag)");
                crate::models::project::BuildSystem::CMake
            }
            _ => {
                println!();
                println!("  Build system:");
                println!("    1) CMake  (default)");
                println!("    2) Zig    (single binary, built-in cross-compilation)");
                let bs_choice = prompt_with_default("Choose build system [1-2]", "1").map_err(|e| {
                    crate::models::FbGenError::Config(format!("failed to read build system: {e}"))
                })?;
                match bs_choice.as_str() {
                    "2" => crate::models::project::BuildSystem::Zig,
                    _ => crate::models::project::BuildSystem::CMake,
                }
            }
        };
        let is_zig = build_system == crate::models::project::BuildSystem::Zig;

        // ── target architecture ───────────────────────────────────
        println!();
        println!("  Target architecture:");
        println!("    1) x86_64    (default)");
        println!("    2) x86");
        println!("    3) ARM64");
        println!("    4) ARM32");
        println!("    5) RISC-V 64");
        println!("    6) RISC-V 32 (e.g. ESP32-C3/C6/H2/P4)");
        println!("    7) WASM");
        println!("    8) None-EABI (ARM Cortex-M)");
        println!("    9) Xtensa    (e.g. ESP32/ESP32-S2/ESP32-S3)");
        println!("    10) Custom");
        let arch_choice = prompt_with_default("Choose architecture [1-10]", "1").map_err(|e| {
            crate::models::FbGenError::Config(format!("failed to read architecture: {e}"))
        })?;
        let target_arch = match arch_choice.as_str() {
            "2" => TargetArch::X86,
            "3" => TargetArch::ARM64,
            "4" => TargetArch::ARM32,
            "5" => TargetArch::RISCV64,
            "6" => TargetArch::RISCV32,
            "7" => TargetArch::WASM,
            "8" => TargetArch::NoneEabi,
            "9" => TargetArch::Xtensa,
            "10" => {
                let custom = prompt("  Custom architecture name: ").map_err(|e| {
                    crate::models::FbGenError::Config(format!("failed to read custom arch: {e}"))
                })?;
                TargetArch::Custom(custom)
            }
            _ => TargetArch::X86_64,
        };

        // ── Toolchain config (cross-compile targets) ───────────────
        let mut toolchain: Option<crate::models::project::ToolchainConfig> = None;

        if matches!(target_arch, TargetArch::NoneEabi | TargetArch::ARM32 | TargetArch::ARM64 | TargetArch::RISCV64 | TargetArch::RISCV32 | TargetArch::Xtensa) {
            let detected = crate::core::detect_toolchains();

            // Filter to toolchains compatible with the chosen architecture.
            let compatible: Vec<_> = detected
                .iter()
                .filter(|dt| dt.suggested_arch == target_arch)
                .collect();

            let ask_mcu_cpu = || -> FbGenResult<(String, String, String, String, String, String)> {
                if matches!(target_arch, TargetArch::NoneEabi) {
                    println!();
                    println!("  ARM MCU/CPU selection:");
                    println!("    Specify the target chip model for -mcpu= flag.");
                    let cpu = prompt_with_default("  ARM MCU/CPU [cortex-m3]", "cortex-m3")
                        .map_err(|e| {
                            crate::models::FbGenError::Config(format!("failed to read MCU: {e}"))
                        })?;
                    let float_abi = prompt_with_default(
                        "  Float ABI (soft/softfp/hard, empty to skip) []", ""
                    ).map_err(|e| {
                        crate::models::FbGenError::Config(format!("failed to read float ABI: {e}"))
                    })?;
                    let fpu = prompt_with_default(
                        "  FPU (e.g. fpv4-sp-d16, empty to skip) []", ""
                    ).map_err(|e| {
                        crate::models::FbGenError::Config(format!("failed to read FPU: {e}"))
                    })?;
                    let extra_flags = prompt_with_default(
                        "  Extra flags (e.g. -mthumb, empty to skip) []", ""
                    ).map_err(|e| {
                        crate::models::FbGenError::Config(format!("failed to read extra flags: {e}"))
                    })?;
                    Ok((cpu, float_abi, fpu, extra_flags, String::new(), String::new()))
                } else if matches!(target_arch, TargetArch::RISCV32 | TargetArch::RISCV64) {
                    println!();
                    println!("  RISC-V architecture selection:");
                    println!("    Specify -march= value (e.g. rv32imac, rv64gc).");
                    let march = prompt_with_default(
                        if matches!(target_arch, TargetArch::RISCV32) {
                            "  -march= [rv32imac]"
                        } else {
                            "  -march= [rv64gc]"
                        },
                        if matches!(target_arch, TargetArch::RISCV32) { "rv32imac" } else { "rv64gc" },
                    ).map_err(|e| {
                        crate::models::FbGenError::Config(format!("failed to read march: {e}"))
                    })?;
                    let mabi = prompt_with_default(
                        if matches!(target_arch, TargetArch::RISCV32) {
                            "  -mabi= [ilp32]"
                        } else {
                            "  -mabi= [lp64d]"
                        },
                        if matches!(target_arch, TargetArch::RISCV32) { "ilp32" } else { "lp64d" },
                    ).map_err(|e| {
                        crate::models::FbGenError::Config(format!("failed to read mabi: {e}"))
                    })?;
                    let extra_flags = prompt_with_default(
                        "  Extra flags (empty to skip) []", ""
                    ).map_err(|e| {
                        crate::models::FbGenError::Config(format!("failed to read extra flags: {e}"))
                    })?;
                    Ok((String::new(), String::new(), String::new(), extra_flags, march, mabi))
                } else if matches!(target_arch, TargetArch::Xtensa) {
                    println!();
                    println!("  Xtensa (ESP32) toolchain:");
                    println!("    Built-in flags: -mlongcalls (per ESP-IDF standard)");
                    let extra_flags = prompt_with_default(
                        "  Extra flags (empty to skip) []", ""
                    ).map_err(|e| {
                        crate::models::FbGenError::Config(format!("failed to read extra flags: {e}"))
                    })?;
                    Ok((String::new(), String::new(), String::new(), extra_flags, String::new(), String::new()))
                } else {
                    let extra_flags = prompt_with_default(
                        "  Extra flags (empty to skip) []", ""
                    ).map_err(|e| {
                        crate::models::FbGenError::Config(format!("failed to read extra flags: {e}"))
                    })?;
                    Ok((String::new(), String::new(), String::new(), extra_flags, String::new(), String::new()))
                }
            };

            let ask_device_defines = || -> FbGenResult<Vec<String>> {
                /// Menu items: (display label, define value).
                fn menu() -> &'static [(&'static str, &'static str)] {
                    &[
                        // ── STM32 ─────────────────────────────────────
                        ("STM32F103xB", "STM32F103xB"),
                        ("STM32F103xE", "STM32F103xE"),
                        ("STM32F405xx", "STM32F405xx"),
                        ("STM32F407xx", "STM32F407xx"),
                        ("STM32F429xx", "STM32F429xx"),
                        ("STM32F446xx", "STM32F446xx"),
                        ("STM32F303xx", "STM32F303xx"),
                        ("STM32F767xx", "STM32F767xx"),
                        ("STM32G474xx", "STM32G474xx"),
                        ("STM32F030x6", "STM32F030x6"),
                        ("STM32F746xx", "STM32F746xx"),
                        ("STM32H743xx", "STM32H743xx"),
                        ("STM32H503xx", "STM32H503xx"),
                        ("STM32G031xx", "STM32G031xx"),
                        ("STM32L476xx", "STM32L476xx"),
                        ("STM32L031xx", "STM32L031xx"),
                        ("STM32L552xx", "STM32L552xx"),
                        ("STM32U575xx", "STM32U575xx"),
                        ("STM32WB55xx", "STM32WB55xx"),
                        ("STM32WL5xxx", "STM32WL5xxx"),
                        // ── Nordic nRF ────────────────────────────────
                        ("NRF52832", "NRF52832"),
                        ("NRF52840", "NRF52840"),
                        ("NRF5340", "NRF5340"),
                        ("NRF9160", "NRF9160"),
                        // ── Microchip/Atmel SAM ───────────────────────
                        ("SAMD21", "SAMD21"),
                        ("SAMD51", "SAMD51"),
                        ("SAME51", "SAME51"),
                        // ── NXP ───────────────────────────────────────
                        ("MK64F12", "MK64F12"),
                        ("LPC1768", "LPC1768"),
                        ("IMXRT1062", "IMXRT1062"),
                        // ── Espressif ─────────────────────────────────
                        ("ESP32", "ESP32"),
                        ("ESP32S3", "ESP32S3"),
                        // ── Raspberry Pi ──────────────────────────────
                        ("RP2040", "RP2040"),
                        // ── Other ─────────────────────────────────────
                        ("GD32F303", "GD32F303"),
                        ("AT32F407", "AT32F407"),
                        // ── Library config ────────────────────────────
                        ("USE_HAL_DRIVER", "USE_HAL_DRIVER"),
                        ("USE_FULL_LL_DRIVER", "USE_FULL_LL_DRIVER"),
                    ]
                }

                let menu = menu();
                println!();
                println!("  ── Device Define Selection ────────────────────────");
                println!("  Select space-separated numbers, or type C for custom, Enter to skip.");
                println!();

                // Print in three columns.
                for (i, (label, _define)) in menu.iter().enumerate() {
                    let num = i + 1;
                    if i == 20 {
                        println!(); // separator after STM32 group
                    } else if i == 24 {
                        println!(); // after nRF
                    } else if i == 27 {
                        println!(); // after SAM
                    } else if i == 30 {
                        println!(); // after NXP
                    } else if i == 32 {
                        println!(); // after Espressif
                    } else if i == 33 {
                        println!(); // after RP2040
                    } else if i == 35 {
                        println!(); // after Other
                    }
                    if (i % 4) == 3 {
                        println!("    {:>2}) {:<20}", num, label);
                    } else {
                        print!("    {:>2}) {:<20}", num, label);
                    }
                }
                // Flush last line if it didn't end with newline.
                if menu.len() % 4 != 0 {
                    println!();
                }

                let answer = prompt_with_default(
                    "  Selection (numbers, C=custom, Enter=skip) []", ""
                ).map_err(|e| {
                    crate::models::FbGenError::Config(format!(
                        "failed to read device defines: {e}"
                    ))
                })?;

                let trimmed = answer.trim();
                if trimmed.is_empty() {
                    return Ok(Vec::new());
                }

                // Custom: prompt for raw defines.
                if trimmed.eq_ignore_ascii_case("c") {
                    println!("  Examples: STM32F103xB USE_HAL_DRIVER");
                    let custom = prompt_with_default(
                        "  Enter device defines (space-separated) []", ""
                    ).map_err(|e| {
                        crate::models::FbGenError::Config(format!(
                            "failed to read device defines: {e}"
                        ))
                    })?;
                    if custom.is_empty() {
                        return Ok(Vec::new());
                    }
                    return Ok(custom.split_whitespace().map(String::from).collect());
                }

                // Parse numbers.
                let mut result: Vec<String> = Vec::new();
                for part in trimmed.split_whitespace() {
                    match part.parse::<usize>() {
                        Ok(n) if n >= 1 && n <= menu.len() => {
                            let define = menu[n - 1].1.to_string();
                            if !result.contains(&define) {
                                result.push(define);
                            }
                        }
                        _ => {
                            // Treat unrecognised input as a raw define.
                            let define = part.to_string();
                            if !result.contains(&define) {
                                result.push(define);
                            }
                        }
                    }
                }
                Ok(result)
            };

            if compatible.is_empty() {
                println!();
                println!("  No compatible toolchain auto-detected for {:?}.", target_arch);
                println!("  Falling back to manual configuration.");

                let default_prefix = match target_arch {
                    TargetArch::NoneEabi => "arm-none-eabi-",
                    TargetArch::ARM32 => "arm-linux-gnueabihf-",
                    TargetArch::ARM64 => "aarch64-none-elf-",
                    TargetArch::RISCV64 => "riscv64-unknown-elf-",
                    TargetArch::RISCV32 => "riscv32-esp-elf-",
                    TargetArch::Xtensa => "xtensa-esp32-elf-",
                    _ => "",
                };
                let prefix = prompt_with_default(
                    &format!("  Toolchain prefix [{}]", default_prefix),
                    default_prefix,
                ).map_err(|e| {
                    crate::models::FbGenError::Config(format!("failed to read prefix: {e}"))
                })?;

                let sysroot_str = prompt_with_default(
                    "  Sysroot path (empty to skip) []", ""
                ).map_err(|e| {
                    crate::models::FbGenError::Config(format!("failed to read sysroot: {e}"))
                })?;
                let sysroot = if sysroot_str.is_empty() { None } else { Some(sysroot_str) };

                let find_root_str = prompt_with_default(
                    "  Extra CMAKE_FIND_ROOT_PATH entries (space-separated, empty to skip) []", ""
                ).map_err(|e| {
                    crate::models::FbGenError::Config(format!("failed to read find root path: {e}"))
                })?;
                let find_root_path: Vec<String> = find_root_str
                    .split_whitespace()
                    .map(String::from)
                    .collect();

                let (cpu, float_abi, fpu, extra_flags, march, mabi) = ask_mcu_cpu()?;

                // ── Device defines ──
                let device_defines = ask_device_defines()?;

                toolchain = Some(crate::models::project::ToolchainConfig {
                    cpu,
                    float_abi,
                    fpu,
                    march,
                    mabi,
                    extra_flags,
                    prefix,
                    sysroot,
                    find_root_path,
                    device_defines,
                });
            } else {
                // Show detected toolchains for the user to pick from.
                println!();
                println!("  Detected {} compatible toolchain(s) for {:?}:", compatible.len(), target_arch);
                for (i, dt) in compatible.iter().enumerate() {
                    let sysroot_display = dt.sysroot.as_ref()
                        .map(|s| s.display().to_string())
                        .unwrap_or_else(|| "none".into());
                    println!("    {}) {} → {}  (sysroot: {})", i + 1, dt.prefix, dt.cc_path.display(), sysroot_display);
                }
                println!("    {}) Custom — enter prefix and sysroot manually", compatible.len() + 1);

                let choice = prompt_with_default(
                    &format!("  Choose toolchain [1-{}]", compatible.len() + 1),
                    "1",
                ).map_err(|e| {
                    crate::models::FbGenError::Config(format!("failed to read choice: {e}"))
                })?;

                let (prefix, sysroot, find_root_path) = if let Ok(idx) = choice.parse::<usize>() {
                    if idx >= 1 && idx <= compatible.len() {
                        let dt = &compatible[idx - 1];
                        let prefix = dt.prefix.clone();
                        let sysroot = dt.sysroot.clone().map(|p| p.to_string_lossy().to_string());
                        let find_root_str = prompt_with_default(
                            "  Extra CMAKE_FIND_ROOT_PATH entries (space-separated, empty to skip) []", ""
                        ).map_err(|e| {
                            crate::models::FbGenError::Config(format!("failed to read find root path: {e}"))
                        })?;
                        let find_root_path: Vec<String> = find_root_str
                            .split_whitespace()
                            .map(String::from)
                            .collect();
                        (prefix, sysroot, find_root_path)
                    } else {
                        // Custom entry.
                        let default_prefix = match target_arch {
                            TargetArch::NoneEabi => "arm-none-eabi-",
                            TargetArch::ARM32 => "arm-linux-gnueabihf-",
                            TargetArch::ARM64 => "aarch64-none-elf-",
                            TargetArch::RISCV64 => "riscv64-unknown-elf-",
                            TargetArch::RISCV32 => "riscv32-esp-elf-",
                            TargetArch::Xtensa => "xtensa-esp32-elf-",
                            _ => "arm-none-eabi-",
                        };
                        let prefix = prompt_with_default("  Toolchain prefix", default_prefix)
                            .map_err(|e| {
                                crate::models::FbGenError::Config(format!("failed to read prefix: {e}"))
                            })?;
                        let sysroot_str = prompt_with_default(
                            "  Sysroot path (empty to skip) []", ""
                        ).map_err(|e| {
                            crate::models::FbGenError::Config(format!("failed to read sysroot: {e}"))
                        })?;
                        let sysroot = if sysroot_str.is_empty() { None } else { Some(sysroot_str) };
                        let find_root_str = prompt_with_default(
                            "  Extra CMAKE_FIND_ROOT_PATH entries (space-separated) []", ""
                        ).map_err(|e| {
                            crate::models::FbGenError::Config(format!("failed to read find root: {e}"))
                        })?;
                        let find_root_path: Vec<String> = find_root_str
                            .split_whitespace()
                            .map(String::from)
                            .collect();
                        (prefix, sysroot, find_root_path)
                    }
                } else {
                    ("arm-none-eabi-".into(), None, Vec::new())
                };

                let (cpu, float_abi, fpu, extra_flags, march, mabi) = ask_mcu_cpu()?;

                // ── Device defines ──
                let device_defines = ask_device_defines()?;

                toolchain = Some(crate::models::project::ToolchainConfig {
                    cpu,
                    float_abi,
                    fpu,
                    march,
                    mabi,
                    extra_flags,
                    prefix,
                    sysroot,
                    find_root_path,
                    device_defines,
                });
            }
        }

        // ── compiler (CMake only; Zig is its own compiler) ─────────
        let compiler = if is_zig {
            Compiler::Zig
        } else {
            println!();
            println!("  Compiler:");
            println!("    1) GCC    (default)");
            println!("    2) Clang");
            println!("    3) MSVC");
            println!("    4) Custom");
            let cc_choice = prompt_with_default("Choose compiler [1-4]", "1").map_err(|e| {
                crate::models::FbGenError::Config(format!("failed to read compiler: {e}"))
            })?;
            match cc_choice.as_str() {
                "2" => Compiler::Clang,
                "3" => Compiler::MSVC,
                "4" => {
                    let custom = prompt("  Custom compiler name: ").map_err(|e| {
                        crate::models::FbGenError::Config(format!(
                            "failed to read custom compiler: {e}"
                        ))
                    })?;
                    Compiler::Custom(custom)
                }
                _ => Compiler::GCC,
            }
        };

        // ── build backend (CMake only; Zig is its own build system) ─
        let build_backend = if is_zig {
            BuildBackend::Ninja // unused by zig; set to default
        } else {
            println!();
            println!("  Build backend:");
            println!("    1) Ninja (default)");
            println!("    2) Make");
            println!("    3) MSBuild");
            println!("    4) Custom");
            let backend_choice = prompt_with_default("Choose backend [1-4]", "1").map_err(|e| {
                crate::models::FbGenError::Config(format!("failed to read build backend: {e}"))
            })?;
            match backend_choice.as_str() {
                "2" => BuildBackend::Make,
                "3" => BuildBackend::MSBuild,
                "4" => {
                    let custom = prompt("  Custom backend name: ").map_err(|e| {
                        crate::models::FbGenError::Config(format!("failed to read custom backend: {e}"))
                    })?;
                    BuildBackend::Custom(custom)
                }
                _ => BuildBackend::Ninja,
            }
        };

        // ── CMake minimum version (CMake only) ─────────────────────
        let cmake_min_version = if is_zig {
            "3.16".to_string()
        } else {
            prompt_with_default("CMake minimum version", "3.16").map_err(|e| {
                crate::models::FbGenError::Config(format!("failed to read CMake version: {e}"))
            })?
        };

        // ── output directory ──────────────────────────────────────
        let output_dir_str = prompt_with_default("Output directory", "build").map_err(|e| {
            crate::models::FbGenError::Config(format!("failed to read output dir: {e}"))
        })?;

        let config = ProjectConfig {
            name,
            version: "0.1.0".to_string(),
            root: root.clone(),
            language,
            c_standard,
            cpp_standard,
            target_arch,
            compiler,
            build_backend,
            cmake_min_version,
            exclude_dirs: vec!["build".into(), ".git".into()],
            output_dir: PathBuf::from(output_dir_str),
            enable_watch: false,
            modules: vec![],
            generated_at: String::new(),
            cmake_presets: None,
            toolchain_files: vec![],
            toolchain,
            build_system,
        };

        Ok(config)
    }

    /// Print a formatted summary of the configuration and ask the user
    /// to confirm before proceeding.
    pub fn confirm_config(config: &ProjectConfig) -> bool {
        println!();
        println!("  ── Configuration Summary ──────────────────────────");
        println!("  Project name:      {}", config.name);
        println!("  Language:          {}", config.language);
        if config.language == "C" {
            println!("  C standard:        C{}", config.c_standard);
        }
        if config.language == "CXX" {
            println!("  C++ standard:      C++{}", config.cpp_standard);
        }
        println!("  Architecture:      {:?}", config.target_arch);
        println!("  Build system:      {:?}", config.build_system);
        if config.build_system != crate::models::project::BuildSystem::Zig {
            println!("  Compiler:          {:?}", config.compiler);
            println!("  Build backend:     {:?}", config.build_backend);
            println!("  CMake min version: {}", config.cmake_min_version);
        }
        println!("  Root:              {}", config.root.display());
        println!("  Output dir:        {}", config.output_dir.display());
        if let Some(ref tc) = config.toolchain {
            println!("  ── Toolchain ────────────────────────────────────────");
            if !tc.prefix.is_empty() {
                println!("    Prefix:           {}", tc.prefix);
            }
            println!("    CPU:              {}", tc.cpu);
            if !tc.float_abi.is_empty() {
                println!("    Float ABI:        {}", tc.float_abi);
            }
            if !tc.fpu.is_empty() {
                println!("    FPU:              {}", tc.fpu);
            }
            if !tc.extra_flags.is_empty() {
                println!("    Extra flags:      {}", tc.extra_flags);
            }
            if let Some(ref sysroot) = tc.sysroot {
                println!("    Sysroot:          {}", sysroot);
            }
            if !tc.find_root_path.is_empty() {
                println!("    Find root paths:  {}", tc.find_root_path.join(" "));
            }
            if !tc.device_defines.is_empty() {
                println!("    Device defines:   {}", tc.device_defines.join(" "));
            }
        }
        println!("  ────────────────────────────────────────────────────");

        match prompt("  Proceed with this configuration? [Y/n]: ") {
            Ok(answer) => {
                let trimmed = answer.trim().to_lowercase();
                trimmed.is_empty() || trimmed == "y" || trimmed == "yes"
            }
            Err(_) => false,
        }
    }
}

// ── tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confirm_config_accepts_default() {
        let config = ProjectConfig::default();
        // Without a real tty we can only test that the summary prints
        // without panicking.
        let _ = UserQuery::confirm_config(&config);
    }
}
