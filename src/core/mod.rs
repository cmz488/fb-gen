pub mod analyzer;
pub mod discoverer;
pub mod generator;
pub mod inferrer;

pub use analyzer::DependencyAnalyzer;
pub use discoverer::{ModuleDiscoverer, ScanOptions};
pub use generator::CMakeGenerator;
pub use inferrer::ConfigInferrer;
