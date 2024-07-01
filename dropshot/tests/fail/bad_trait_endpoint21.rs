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

// In this test, since we always output the API trait warts and all, we'll
// get errors produced by both the proc-macro and rustc.

#[dropshot::api_description]
trait MyApi {
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
    ) -> Result<HttpResponseUpdatedNoContent, HttpError>;
}

enum MyImpl {}

// This should not produce errors about items being missing.
impl MyApi for MyImpl {
    type Context = ();

    async fn variadic_argument(
        _rqctx: RequestContext<Self::Context>,
        _param1: Query<QueryParams>,
        ...
    ) -> Result<HttpResponseUpdatedNoContent, HttpError> {
        unreachable!()
    }
}

fn main() {
    // These items should be generated and accessible.
    my_api::api_description::<MyImpl>();
    my_api::stub_api_description();
}
