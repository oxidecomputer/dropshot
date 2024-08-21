// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

// Check a method being marked as both an endpoint and a channel, or an endpoint
// multiple times, produces an error.

use dropshot::HttpError;
use dropshot::HttpResponseUpdatedNoContent;
use dropshot::RequestContext;
use dropshot::WebsocketConnection;

#[dropshot::api_description]
trait MyApi {
    type Context;

    #[endpoint {
        method = GET,
        path = "/test",
    }]
    #[channel {
        protocol = WEBSOCKETS,
    }]
    async fn bad_endpoint(
        _rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError>;

    #[endpoint {
        method = GET,
        path = "/test",
    }]
    #[endpoint {
        method = GET,
        path = "/test",
    }]
    #[endpoint {
        method = GET,
        path = "/test",
    }]
    async fn bad_endpoint2(
        _rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError>;

    #[channel {
        protocol = WEBSOCKETS,
    }]
    #[endpoint {
        method = GET,
        path = "/test",
    }]
    async fn bad_channel(
        _rqctx: RequestContext<Self::Context>,
        _upgraded: WebsocketConnection,
    ) -> dropshot::WebsocketChannelResult;

    #[channel {
        protocol = WEBSOCKETS,
    }]
    #[channel {
        protocol = WEBSOCKETS,
    }]
    #[channel {
        protocol = WEBSOCKETS,
    }]
    async fn bad_channel2(
        _rqctx: RequestContext<Self::Context>,
        _upgraded: WebsocketConnection,
    ) -> dropshot::WebsocketChannelResult;
}

enum MyImpl {}

// This should not produce errors about items being missing.
impl MyApi for MyImpl {
    type Context = ();

    async fn bad_endpoint(
        _rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError> {
        Ok(HttpResponseUpdatedNoContent())
    }

    async fn bad_endpoint2(
        _rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError> {
        Ok(HttpResponseUpdatedNoContent())
    }

    async fn bad_channel(
        _rqctx: RequestContext<Self::Context>,
        _upgraded: WebsocketConnection,
    ) -> dropshot::WebsocketChannelResult {
        Ok(())
    }

    async fn bad_channel2(
        _rqctx: RequestContext<Self::Context>,
        _upgraded: WebsocketConnection,
    ) -> dropshot::WebsocketChannelResult {
        Ok(())
    }
}

fn main() {
    // These items should be generated and accessible.
    my_api_mod::api_description::<MyImpl>();
    my_api_mod::stub_api_description();
}
