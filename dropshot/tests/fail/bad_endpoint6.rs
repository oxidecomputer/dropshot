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
#[allow(dead_code)]
struct Ret {
    x: String,
    y: u32,
}

#[endpoint {
    method = GET,
    path = "/test",
}]
async fn bad_endpoint(
    _rqctx: Arc<RequestContext<()>>,
) -> Result<HttpResponseOk<Ret>, HttpError> {
    // Validate that compiler errors show up with useful context and aren't
    // obscured by the macro.
    Ok(HttpResponseOk(Ret { "Oxide".to_string(), 0x1de }))
}

fn main() {}
