// Copyright 2020 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::endpoint;
use dropshot::HttpError;
use dropshot::HttpResponseOk;
use dropshot::RequestContext;
use schemars::JsonSchema;


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
async fn bad_endpoint(_rqctx: &mut RequestContext<()>) -> Result<HttpResponseOk<Ret>, HttpError> {
    Ok(HttpResponseOk(Ret { "Oxide".to_string(), 0x1de }))
}

fn main() {}
