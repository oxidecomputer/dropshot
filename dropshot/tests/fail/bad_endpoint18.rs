// Copyright 2023 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::endpoint;
use dropshot::HttpError;
use dropshot::HttpResponseOk;
use dropshot::Query;
use dropshot::TypedBody;
use dropshot::RequestContext;
use schemars::JsonSchema;
use serde::Deserialize;

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
struct Stuff {
    x: String,
}

// Test: exclusive extractor not as the last argument
#[endpoint {
    method = GET,
    path = "/test",
}]
async fn exclusive_extractor_not_last(
    _rqctx: RequestContext<()>,
    _param1: TypedBody<Stuff>,
    _param2: Query<Stuff>,
) -> Result<HttpResponseOk<()>, HttpError> {
    Ok(HttpResponseOk(()))
}

fn main() {}
