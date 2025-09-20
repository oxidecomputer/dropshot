// Copyright 2025 Oxide Computer Company

//! openapi-manager library facilities for implementing the openapi-manager
//! command-line tool

// helpers
pub mod dispatch;

// subcommands
pub(crate) mod check;
mod debug;
mod generate;
mod list;
