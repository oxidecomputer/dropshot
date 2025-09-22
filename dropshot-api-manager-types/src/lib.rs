// Copyright 2025 Oxide Computer Company

//! Shared types for the OpenAPI manager.
//!
//! API trait crates can depend on this crate to get access to interfaces
//! exposed by the OpenAPI manager.

mod apis;
mod validation;
mod versions;

pub use apis::*;
pub use validation::*;
pub use versions::*;

// Re-export these types for consumers of `api_versions!`.
pub use paste::paste;
pub use semver;
