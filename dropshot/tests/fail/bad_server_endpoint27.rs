// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

// Check that a reasonable error is produced if `dropshot::server` is used on
// a function rather than a trait.

use dropshot::HttpError;
use dropshot::HttpResponseUpdatedNoContent;
use dropshot::RequestContext;

#[dropshot::server]
fn bad_server() {}

fn main() {}
