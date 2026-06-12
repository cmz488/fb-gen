use thiserror::Error;

#[derive(Error, Debug)]
pub enum FbGenError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("File search error: {0}")]
    FffSearch(String),

    #[error("Template rendering error: {0}")]
    Template(#[from] tera::Error),

    #[error("Circular dependency detected involving module: {0}")]
    CircularDependency(String),

    #[error("Module not found: {0}")]
    ModuleNotFound(String),

    #[error("No source files found in project: {0}")]
    NoSources(String),

    #[error("Generation failed: {0}")]
    GenerationFailed(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("YAML error: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Regex error: {0}")]
    Regex(#[from] regex::Error),
}

pub type FbGenResult<T> = std::result::Result<T, FbGenError>;
