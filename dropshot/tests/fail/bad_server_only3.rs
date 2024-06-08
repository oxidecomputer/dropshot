// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

// Check that if a custom context type is defined, it must be used everywhere,
// and must be reported as such in error messages.

use dropshot::HttpError;
use dropshot::HttpResponseUpdatedNoContent;
use dropshot::RequestContext;

#[dropshot::server { context = MyContext }]
trait MyTrait {
    type MyContext;
    type Context: dropshot::ServerContext;

    #[endpoint {
        method = GET,
        path = "/test",
    }]
    async fn good_endpoint(
        _rqctx: RequestContext<Self::MyContext>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError>;

    #[endpoint {
        method = GET,
        path = "/test",
    }]
    async fn bad_endpoint(
        _rqctx: RequestContext<()>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError>;

    #[endpoint {
        method = GET,
        path = "/test2",
    }]
    async fn bad_endpoint2(
        _rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError>;
}

enum MyImpl {}

// This should not produce errors about items being missing.
impl MyTrait for MyImpl {
    type MyContext = ();
    type Context = ();

    async fn good_endpoint(
        _rqctx: RequestContext<Self::MyContext>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError> {
        Ok(HttpResponseUpdatedNoContent())
    }

    async fn bad_endpoint(
        _rqctx: RequestContext<()>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError> {
        Ok(HttpResponseUpdatedNoContent())
    }

    async fn bad_endpoint2(
        _rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError> {
        Ok(HttpResponseUpdatedNoContent())
    }
}

fn main() {}
