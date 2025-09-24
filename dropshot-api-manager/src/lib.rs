// Copyright 2025 Oxide Computer Company

//! OpenAPI manager for Dropshot.
//!
//! This tool manages OpenAPI documents corresponding to Dropshot API traits.
//! For more information, see the
//! [README](https://github.com/oxidecomputer/dropshot/tree/main/dropshot-api-manager).

mod apis;
mod cmd;
mod compatibility;
mod environment;
mod git;
mod iter_only;
mod output;
mod resolved;
mod spec_files_blessed;
mod spec_files_generated;
mod spec_files_generic;
mod spec_files_local;
pub mod test_util;
mod validation;

#[macro_use]
extern crate newtype_derive;

pub use apis::*;
pub use cmd::dispatch::*;
pub use environment::Environment;
pub use output::CheckResult;
