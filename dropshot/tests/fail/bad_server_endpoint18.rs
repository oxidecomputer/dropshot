// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::HttpError;
use dropshot::HttpResponseUpdatedNoContent;
use dropshot::Query;
use dropshot::RequestContext;
use dropshot::TypedBody;
use schemars::JsonSchema;
use serde::Deserialize;

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
struct Stuff {
    x: String,
}

// Test: exclusive extractor not as the last argument
#[dropshot::server]
trait MyServer {
    type Context;

    #[endpoint {
        method = GET,
        path = "/test",
    }]
    async fn exclusive_extractor_not_last(
        _rqctx: RequestContext<Self::Context>,
        _param1: TypedBody<Stuff>,
        _param2: Query<Stuff>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError>;
}

enum MyImpl {}

// This should not produce errors about items being missing.

impl MyServer for MyImpl {
    type Context = ();

    async fn exclusive_extractor_not_last(
        _rqctx: RequestContext<()>,
        _param1: TypedBody<Stuff>,
        _param2: Query<Stuff>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError> {
        Ok(HttpResponseUpdatedNoContent())
    }
}

fn main() {}
