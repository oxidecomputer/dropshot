// Copyright 2025 Oxide Computer Company

//! dropshot-api-manager library facilities for implementing the dropshot-api-manager
//! command-line tool

// helpers
pub mod dispatch;

// subcommands
pub(crate) mod check;
mod debug;
mod generate;
mod list;
