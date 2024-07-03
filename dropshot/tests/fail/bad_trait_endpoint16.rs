// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::HttpError;
use dropshot::HttpResponseUpdatedNoContent;
use dropshot::RequestContext;

#[dropshot::api_description]
trait MyApi {
    type Context;

    #[endpoint {
        method = GET,
        path = "/test",
        content_type = "foo/bar",
    }]
    async fn bad_endpoint(
        _rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError>;
}

enum MyImpl {}

// This should not produce errors about items being missing.
impl MyApi for MyImpl {
    type Context = ();

    async fn bad_endpoint(
        _rqctx: RequestContext<()>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError> {
        Ok(HttpResponseUpdatedNoContent())
    }
}

fn main() {
    // These items should be generated and accessible.
    my_api::api_description::<MyImpl>();
    my_api::stub_api_description();
}
