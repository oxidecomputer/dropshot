// Copyright 2023 Oxide Computer Company

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

// Test: last parameter is variadic.
#[endpoint {
    method = GET,
    path = "/test",
}]
async fn variadic_argument(
    _rqctx: RequestContext<()>,
    _param1: Query<QueryParams>,
    ...
) -> Result<HttpResponseOk<()>, HttpError> {
    Ok(HttpResponseOk(()))
}

fn main() {}
