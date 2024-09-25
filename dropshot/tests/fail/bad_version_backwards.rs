// Copyright 2024 Oxide Computer Company

//! Tests the case where the "versions" field is not parseable
//!
//! We do not bother testing most of the other "bad version" cases because we
//! have exhaustive unit tests for those.

#![allow(unused_imports)]

use dropshot::endpoint;
use dropshot::HttpError;
use dropshot::HttpResponseOk;

#[endpoint {
    method = GET,
    path = "/test",
    versions = "1.2.3".."1.2.2"
}]
async fn bad_version_backwards() -> Result<HttpResponseOk<()>, HttpError> {
    Ok(HttpResponseOk(()))
}

fn main() {}
