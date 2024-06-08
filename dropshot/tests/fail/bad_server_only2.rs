// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

// Check that the RequestContext type parameter must be Self::Context and
// nothing else -- not some other type like (), and not some other associated
// type.

use dropshot::HttpError;
use dropshot::HttpResponseUpdatedNoContent;
use dropshot::RequestContext;

#[dropshot::server]
trait MyTrait {
    type Context;
    type OtherContext: dropshot::ServerContext;

    #[endpoint {
        method = GET,
        path = "/test",
    }]
    async fn good_endpoint(
        _rqctx: RequestContext<Self::Context>,
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
        _rqctx: RequestContext<Self::OtherContext>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError>;
}

enum MyImpl {}

// This should not produce errors about items being missing.
impl MyTrait for MyImpl {
    type Context = ();
    type OtherContext = ();

    async fn good_endpoint(
        _rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError> {
        Ok(HttpResponseUpdatedNoContent())
    }

    async fn bad_endpoint(
        _rqctx: RequestContext<()>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError> {
        Ok(HttpResponseUpdatedNoContent())
    }

    async fn bad_endpoint2(
        _rqctx: RequestContext<Self::OtherContext>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError> {
        Ok(HttpResponseUpdatedNoContent())
    }
}

fn main() {}
