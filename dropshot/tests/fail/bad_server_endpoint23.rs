// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::HttpError;
use dropshot::HttpResponseUpdatedNoContent;
use dropshot::Query;
use dropshot::RequestContext;
use schemars::JsonSchema;
use serde::Deserialize;

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
struct QueryParams {
    x: String,
}

// In this test, since we always output the server trait warts and all, we'll
// get errors produced by both the proc-macro and rustc.

#[dropshot::server]
trait MyServer {
    type Context;

    // Test: const fn.
    #[endpoint {
        method = GET,
        path = "/test",
    }]
    const async fn const_endpoint(
        _rqctx: RequestContext<Self::Context>,
        _param1: Query<QueryParams>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError>;
}

enum MyImpl {}

// This should not produce errors about items being missing.
impl MyServer for MyImpl {
    type Context = ();

    const async fn const_endpoint(
        _rqctx: RequestContext<Self::Context>,
        _param1: Query<QueryParams>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError> {
        Ok(HttpResponseUpdatedNoContent())
    }
}

fn main() {
    // These items should be generated and accessible.
    my_server::api_description::<MyImpl>();
    my_server::stub_api_description();
}
