// Copyright 2020 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::HttpError;
use dropshot::HttpResponseOk;
use dropshot::RequestContext;
use schemars::JsonSchema;
use serde::Serialize;

#[dropshot::server]
trait MyServer {
    // Introduce a simple syntax error in the server definition -- no semicolon
    // after this.
    type Context

    #[endpoint {
        method = GET,
        path = "/test",
    }]
    pub async fn bad_endpoint(
        _rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseOk<()>, HttpError>;
}

enum MyImpl {}

// This currently DOES produce an error, but it's hard to to better with the
// current execution model.
impl MyServer for MyImpl {
    type Context = ();

    async fn bad_endpoint(
        _rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseOk<()>, HttpError> {
        Ok(HttpResponseOk(()))
    }
}

fn main() {}
