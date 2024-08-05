// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::HttpError;
use dropshot::HttpResponseUpdatedNoContent;
use dropshot::RequestContext;
use std::sync::Arc;

#[dropshot::api_description]
trait MyApi {
    type Context;

    // Test: final parameter is neither an ExclusiveExtractor nor a SharedExtractor.
    #[endpoint {
        method = GET,
        path = "/test",
    }]
    async fn bad_endpoint(
        _rqctx: RequestContext<Self::Context>,
        _param: String,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError>;
}

enum MyImpl {}

impl MyApi for MyImpl {
    type Context = Arc<()>;

    async fn bad_endpoint(
        _rqctx: RequestContext<Self::Context>,
        _param: String,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError> {
        Ok(HttpResponseUpdatedNoContent())
    }
}

fn main() {
    // These items should be generated and accessible.
    my_api_mod::api_description::<MyImpl>();
    my_api_mod::stub_api_description();
}
