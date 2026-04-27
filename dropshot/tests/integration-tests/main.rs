// Copyright 2024 Oxide Computer Company

//! Integration tests for Dropshot.
//!
//! These are all combined into the same file to ensure that a single binary is
//! generated, speeding up link times.

#[macro_use]
extern crate slog;
#[macro_use]
extern crate lazy_static;

mod api_trait;
mod common;
mod config;
mod custom_errors;
mod demo;
mod detached_shutdown;
mod gzip;
mod multipart;
mod openapi;
mod pagination;
mod pagination_schema;
mod path_names;
mod starter;
mod streaming;
mod tls;
mod versions;
