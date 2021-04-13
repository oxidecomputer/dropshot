// Copyright 2020 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::endpoint;
use dropshot::HttpError;
use dropshot::HttpResponseOk;
use dropshot::RequestContext;
use schemars::JsonSchema;
use std::sync::Arc;

#[derive(JsonSchema)]
#[allow(dead_code)]
struct Ret {
    x: String,
    y: u32,
}

#[endpoint {
    method = GET,
    path = "/test",
}]
async fn bad_endpoint(_rqctx: Arc<RequestContext<()>>) -> Result<HttpResponseOk<Ret>, HttpError> {
    Ok(HttpResponseOk(Ret { x: "Oxide".to_string(), y: 0x1de }))
}

fn main() {}
