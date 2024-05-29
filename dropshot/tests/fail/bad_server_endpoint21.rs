// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

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

#[dropshot::server]
trait MyServer {
    type Context;

    // Test: last parameter is variadic.
    #[endpoint {
        method = GET,
        path = "/test",
    }]
    async fn variadic_argument(
        _rqctx: RequestContext<Self::Context>,
        _param1: Query<QueryParams>,
        ...
    ) -> Result<HttpResponseOk<()>, HttpError>;
}

enum MyImpl {}

// This should not produce errors about items being missing.
impl MyServer for MyImpl {
    type Context = ();

    async fn variadic_argument(
        _rqctx: RequestContext<()>,
        _param1: Query<QueryParams>,
        ...
    ) -> Result<HttpResponseOk<()>, HttpError> {
        Ok(HttpResponseOk(()))
    }
}

fn main() {}
