pub mod analyzer;
pub mod discoverer;
pub mod generator;
pub mod inferrer;
pub mod toolchain_detect;

pub use analyzer::DependencyAnalyzer;
pub use discoverer::{ModuleDiscoverer, ScanOptions};
pub use generator::CMakeGenerator;
pub use inferrer::ConfigInferrer;
pub use toolchain_detect::{detect_toolchains, DetectedToolchain};
