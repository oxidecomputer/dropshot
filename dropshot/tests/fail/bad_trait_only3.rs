// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

// Check that if a custom context type is defined, it must be used everywhere,
// and must be reported as such in error messages.

use dropshot::HttpError;
use dropshot::HttpResponseUpdatedNoContent;
use dropshot::RequestContext;

#[dropshot::api_description { context = MyContext }]
trait MyApi {
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

    #[channel {
        protocol = WEBSOCKETS,
        path = "/test",
    }]
    async fn bad_channel(
        _rqctx: RequestContext<()>,
        _upgraded: dropshot::WebsocketConnection,
    ) -> dropshot::WebsocketChannelResult;

    #[channel {
        protocol = WEBSOCKETS,
        path = "/test2",
    }]
    async fn bad_channel2(
        _rqctx: RequestContext<Self::Context>,
        _upgraded: dropshot::WebsocketConnection,
    ) -> dropshot::WebsocketChannelResult;
}

enum MyImpl {}

// This should not produce errors about items being missing.
impl MyApi for MyImpl {
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

    async fn bad_channel(
        _rqctx: RequestContext<()>,
        _upgraded: dropshot::WebsocketConnection,
    ) -> dropshot::WebsocketChannelResult {
        Ok(())
    }

    async fn bad_channel2(
        _rqctx: RequestContext<Self::Context>,
        _upgraded: dropshot::WebsocketConnection,
    ) -> dropshot::WebsocketChannelResult {
        Ok(())
    }
}

fn main() {
    // These items should be generated and accessible.
    my_api::api_description::<MyImpl>();
    my_api::stub_api_description();
}
