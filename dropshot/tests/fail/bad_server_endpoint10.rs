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
    }]
    async fn bad_error_type(
        _rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseOk<()>, String>;
}

enum MyImpl {}

// This should not produce errors about items being missing. However, it does
// produce errors about `String` not being HttpError.
impl MyServer for MyImpl {
    type Context = ();

    async fn bad_error_type(
        _rqctx: RequestContext<()>,
    ) -> Result<HttpResponseOk<()>, String> {
        Ok(HttpResponseOk(()))
    }
}

fn main() {}
