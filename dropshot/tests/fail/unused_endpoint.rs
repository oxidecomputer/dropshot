// Copyright 2021 Oxide Computer Company

#![allow(unused_imports)]
#![deny(warnings)]

use dropshot::endpoint;
use dropshot::ApiDescription;
use dropshot::HttpError;
use dropshot::HttpResponseOk;
use dropshot::RequestContext;
use std::sync::Arc;

#[endpoint {
    method = GET,
    path = "/test",
}]
async fn unused_endpoint(
    _rqctx: Arc<RequestContext<()>>,
) -> Result<HttpResponseOk<()>, HttpError> {
    Ok(HttpResponseOk(()))
}

fn main() {}
