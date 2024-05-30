// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::endpoint;
use dropshot::HttpError;
use dropshot::HttpResponseOk;
use dropshot::Query;
use dropshot::RequestContext;
use schemars::JsonSchema;
use serde::Deserialize;

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
struct QueryParams {
    x: String,
}

// Test: ABI in fn.
#[endpoint {
    method = GET,
    path = "/test",
}]
async extern "ABI" fn abi_endpoint(
    _rqctx: RequestContext<()>,
    _param1: Query<QueryParams>,
) -> Result<HttpResponseOk<()>, HttpError> {
    Ok(HttpResponseOk(()))
}

fn main() {}
