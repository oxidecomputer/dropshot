// Copyright 2020 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::endpoint;
use dropshot::HttpError;
use dropshot::HttpResponseOk;
use dropshot::RequestContext;
use schemars::JsonSchema;
use serde::Serialize;
use std::sync::Arc;

#[derive(JsonSchema, Serialize)]
struct Ret {}

#[endpoint {
    method = GET,
    path = "/test",
}]
fn bad_endpoint(
    _rqctx: Arc<RequestContext<()>>,
) -> Result<HttpResponseOk<Ret>, HttpError> {
    Ok(HttpResponseOk(Ret {}))
}

fn main() {}
