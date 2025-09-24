// Copyright 2025 Oxide Computer Company

//! Shared types for the Dropshot API manager.
//!
//! This crate is part of the [Dropshot OpenAPI
//! manager](https://crates.io/crates/dropshot-api-manager).
//!
//! Code that defines [API
//! traits](https://docs.rs/dropshot/latest/dropshot/attr.api_description.html)
//! can depend on `dropshot-api-manager-types` to get access to interfaces
//! exposed by the Dropshot API manager. In particular, versioned APIs will want
//! to depend on this crate for access to the `api_versions!` macro.

mod apis;
mod validation;
mod versions;

pub use apis::*;
pub use validation::*;
pub use versions::*;

// Re-export these types for consumers of `api_versions!`.
pub use paste::paste;
pub use semver;
