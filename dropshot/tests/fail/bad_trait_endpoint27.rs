// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

// Check that a reasonable error is produced if `dropshot::api_description` is used on
// a function rather than a trait.

use dropshot::HttpError;
use dropshot::HttpResponseUpdatedNoContent;
use dropshot::RequestContext;

#[dropshot::api_description]
fn bad_server() {}

fn main() {}
