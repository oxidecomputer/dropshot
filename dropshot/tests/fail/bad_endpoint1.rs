// Copyright 2020 Oxide Computer Company

// These "bad endpoint" tests are intended to throw compiler errors.  When a
// compiler error is thrown, imports which haven't yet successfully been used
// may throw "unused import" warnings. This is often a false positive, as
// the compiler error itself blocks intended usage.
//
// For this test (and other bad endpoint tests) disable the import warnings
// so output can be more precisely compared with the expected compiler error.
#![allow(unused_imports)]

use dropshot::endpoint;
use dropshot::HttpError;
use dropshot::HttpResponseOk;

#[endpoint {
    method = GET,
    path = "/test",
}]
async fn bad_endpoint() -> Result<HttpResponseOk<()>, HttpError> {
    Ok(HttpResponseOk(()))
}

fn main() {}
