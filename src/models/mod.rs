pub mod dependency;
pub mod error;
pub mod module;
pub mod project;

pub use dependency::{DependencyEdge, DependencyGraph, DependencyType};
pub use error::{FbGenError, FbGenResult};
pub use module::{CMakeModule, SourceFile, SourceType, TargetType};
pub use project::{
    BuildBackend, BuildPreset, CMakePresets, Compiler, ConfigurePreset, DependencySnapshot,
    ProjectConfig, ProjectMeta, TargetArch,
};
