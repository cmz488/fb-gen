//! Package catalogue for fb-gen install.
//!
//! Defines the `Package` type and a registry of known toolchains/SDKs.
//! Populated in Task 2.

use serde::{Deserialize, Serialize};

/// Describes a downloadable package (toolchain, SDK, or middleware).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Package {
    /// Unique identifier, e.g. `"arm-gcc"`, `"stm32cube"`.
    pub id: String,
    /// Version string, e.g. `"13.2.0"`.
    pub version: String,
}
