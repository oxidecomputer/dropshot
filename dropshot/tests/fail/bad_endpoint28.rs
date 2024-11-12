// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::endpoint;
use dropshot::HttpError;
use dropshot::HttpResponseUpdatedNoContent;
use dropshot::RequestContext;

// Test: incorrect type for request_body_max_bytes.

#[endpoint {
    method = GET,
    path = "/test",
    request_body_max_bytes = "not_a_number"
}]
async fn bad_endpoint(
    _rqctx: RequestContext<()>,
) -> Result<HttpResponseUpdatedNoContent, HttpError> {
    Ok(HttpResponseUpdatedNoContent())
}

fn main() {}
