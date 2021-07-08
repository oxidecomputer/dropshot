// Copyright 2020 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::endpoint;
use dropshot::HttpResponseOk;
use dropshot::RequestContext;
use std::sync::Arc;

#[endpoint {
    method = GET,
    path = "/test",
}]
async fn bad_error_type(
    _: Arc<RequestContext<()>>,
) -> Result<HttpResponseOk<()>, String> {
    Ok(HttpResponseOk(()))
}

fn main() {}
