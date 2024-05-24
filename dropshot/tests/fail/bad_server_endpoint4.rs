// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::HttpError;
use dropshot::HttpResponseOk;
use dropshot::Query;
use dropshot::RequestContext;
use std::sync::Arc;

#[allow(dead_code)]
struct QueryParams {
    x: String,
    y: u32,
}

#[dropshot::server]
trait MyServer {
    type Context;

    #[endpoint {
        method = GET,
        path = "/test",
    }]
    async fn bad_endpoint(
        _rqctx: RequestContext<Self::Context>,
        _params: Query<QueryParams>,
    ) -> Result<HttpResponseOk<()>, HttpError>;
}

enum MyImpl {}

// This should not produce errors about items being missing. However, it does
// produce errors about the `QueryParams` type not having the right traits.
impl MyServer for MyImpl {
    type Context = ();

    async fn bad_endpoint(
        _rqctx: RequestContext<()>,
        _params: Query<QueryParams>,
    ) -> Result<HttpResponseOk<()>, HttpError> {
        Ok(HttpResponseOk(()))
    }
}

fn main() {}
