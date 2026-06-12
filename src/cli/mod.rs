//! CLI argument definitions for fb-gen.

pub mod commands;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// Fast Build Generate — 自动扫描 C/C++ 项目并生成 CMake 配置
#[derive(Parser)]
#[command(name = "fb-gen", version, about, long_about = None)]
pub struct Cli {
    /// Project root directory
    #[arg(short, long, global = true, default_value = ".")]
    pub root: PathBuf,

    /// Directories to exclude (comma-separated)
    #[arg(long, global = true, value_delimiter = ',')]
    pub exclude: Vec<String>,

    /// Programming language: C or CXX
    #[arg(long, global = true, default_value = "CXX")]
    pub lang: String,

    /// Skip dependency scanning
    #[arg(long, global = true)]
    pub no_deps: bool,

    /// Output directory for generated files
    #[arg(short = 'o', long, global = true, default_value = "build")]
    pub output: PathBuf,

    /// Enable file watcher for continuous generation
    #[arg(short = 'w', long, global = true)]
    pub watch: bool,

    /// Verbosity level (-v for info, -vv for debug, -vvv for trace)
    #[arg(short = 'v', long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,

    /// Suppress all output except errors
    #[arg(short = 'q', long, global = true)]
    pub quiet: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Initialize a new fb-gen project (creates CMakeLists.txt)
    Init {
        /// Project name
        #[arg(short, long)]
        name: Option<String>,
    },

    /// Sync: scan sources and update CMakeLists.txt incrementally
    Sync,

    /// Check project structure without modifying files (diff mode)
    Check,

    /// Validate generated CMake configuration with cmake
    Validate,

    /// Run the full pipeline: generate + cmake configure + build
    Run,
}

/// Run the fb-gen CLI from parsed arguments.
pub fn run(cli: Cli) {
    use crate::cli::commands;

    let result = match &cli.command {
        Commands::Init { name } => commands::cmd_init(&cli, name.as_deref()),
        Commands::Sync => commands::cmd_sync(&cli),
        Commands::Check => commands::cmd_check(&cli),
        Commands::Validate => commands::cmd_validate(&cli),
        Commands::Run => commands::cmd_run(&cli),
    };

    if let Err(e) = result {
        eprintln!("fb-gen error: {e}");
        std::process::exit(1);
    }
}
