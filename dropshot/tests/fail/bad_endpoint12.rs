// Copyright 2020 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::endpoint;
use dropshot::HttpError;
use dropshot::RequestContext;
use std::sync::Arc;

#[endpoint {
    method = GET,
    path = "/test",
}]
async fn bad_response_type(
    _: Arc<RequestContext<()>>,
) -> Result<String, HttpError> {
    Ok("aok".to_string())
}

fn main() {}
