// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::endpoint;
use dropshot::HttpError;
use dropshot::HttpResponseOk;

// Check that bad_endpoint is present in the generated output, even though the
// #[endpoint] annotation is incorrect.
#[endpoint {
    method = UNKNOWN,
    path = "/test",
}]
async fn bad_endpoint() -> Result<HttpResponseOk<()>, HttpError> {
    Ok(HttpResponseOk(()))
}

fn main() {
    bad_endpoint(); // this line should not produce any errors
}
