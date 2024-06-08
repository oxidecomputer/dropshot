// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::HttpError;
use dropshot::HttpResponseUpdatedNoContent;
use dropshot::RequestContext;
use schemars::JsonSchema;
use serde::Serialize;

#[dropshot::server]
trait MyServer {
    type Context;

    // Introduce a syntax error outside of a function definition ("pub" doesn't
    // work here).
    //
    // Currently, this produces a message about "endpoint" not being valid. This
    // isn't ideal, but at least the rest of the error output is good.
    #[endpoint {
        method = GET,
        path = "/test",
    }]
    pub async fn bad_endpoint(
        _rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError>;
}

enum MyImpl {}

// This should not produce errors about items being missing.
impl MyServer for MyImpl {
    type Context = ();

    async fn bad_endpoint(
        _rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError> {
        Ok(HttpResponseUpdatedNoContent())
    }
}

fn main() {}
