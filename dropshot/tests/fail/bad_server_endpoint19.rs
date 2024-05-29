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
#[dropshot::server]
trait MyServer {
    type Context;

    #[endpoint {
        method = GET,
        path = "/test",
    }]
    async fn non_extractor_as_last_argument(
        _rqctx: RequestContext<Self::Context>,
        _param1: String,
        _param2: Query<QueryParams>,
    ) -> Result<HttpResponseOk<()>, HttpError>;
}

enum MyImpl {}

// This should not produce errors about items being missing.
impl MyServer for MyImpl {
    type Context = ();

    async fn non_extractor_as_last_argument(
        _rqctx: RequestContext<()>,
        _param1: String,
        _param2: Query<QueryParams>,
    ) -> Result<HttpResponseOk<()>, HttpError> {
        Ok(HttpResponseOk(()))
    }
}

fn main() {}
