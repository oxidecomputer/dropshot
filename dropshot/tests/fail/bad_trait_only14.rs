// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::HttpError;
use dropshot::HttpResponseUpdatedNoContent;
use dropshot::RequestContext;
use schemars::JsonSchema;
use serde::Serialize;

#[dropshot::api_description]
trait MyApi {
    // Introduce a simple syntax error in the server definition -- no semicolon
    // after this.
    type Context

    #[endpoint {
        method = GET,
        path = "/test",
    }]
    pub async fn bad_endpoint(
        _rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError>;
}

enum MyImpl {}

// This currently DOES produce an error, but it's really hard to do better.
impl MyApi for MyImpl {
    type Context = ();

    async fn bad_endpoint(
        _rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError> {
        Ok(HttpResponseUpdatedNoContent())
    }
}

fn main() {
    // The parent `my_api` will NOT be present because of the syntax error,
    // and will cause errors to be generated.
    my_api::foo();
}
