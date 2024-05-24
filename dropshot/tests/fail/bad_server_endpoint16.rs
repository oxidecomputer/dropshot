// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::HttpError;
use dropshot::HttpResponseOk;
use dropshot::RequestContext;

#[dropshot::server]
trait MyServer {
    type Context;

    #[endpoint {
        method = GET,
        path = "/test",
        content_type = "foo/bar",
    }]
    async fn bad_endpoint(
        _rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseOk<()>, HttpError>;
}

enum MyImpl {}

// This should not produce errors about items being missing.
impl MyServer for MyImpl {
    type Context = ();

    async fn bad_endpoint(
        _rqctx: RequestContext<()>,
    ) -> Result<HttpResponseOk<()>, HttpError> {
        Ok(HttpResponseOk(()))
    }
}

fn main() {}
