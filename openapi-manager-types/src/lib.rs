// Copyright 2025 Oxide Computer Company

//! Shared types for the OpenAPI manager.
//!
//! API trait crates can depend on this crate to get access to interfaces
//! exposed by the OpenAPI manager.

mod validation;
mod versions;

pub use validation::*;
pub use versions::*;
// Re-export `paste` for consumers of `api_versions!`.
pub use paste::paste;
