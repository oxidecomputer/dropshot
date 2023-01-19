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

// Test: middle parameter is not a SharedExtractor.
#[endpoint {
    method = GET,
    path = "/test",
}]
async fn non_extractor_as_last_argument(
    _rqctx: RequestContext<()>,
    _param1: String,
    _param2: Query<QueryParams>,
) -> Result<HttpResponseOk<()>, HttpError> {
    Ok(HttpResponseOk(()))
}

fn main() {}
