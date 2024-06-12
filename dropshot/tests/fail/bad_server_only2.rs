// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

// Check that the RequestContext type parameter must be Self::Context and
// nothing else -- not some other type like (), and not some other associated
// type.

use dropshot::HttpError;
use dropshot::HttpResponseUpdatedNoContent;
use dropshot::RequestContext;

#[dropshot::server]
trait MyServer {
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
        _rqctx: RequestContext<Self::OtherContext>,
        _upgraded: dropshot::WebsocketConnection,
    ) -> dropshot::WebsocketChannelResult;
}

enum MyImpl {}

// This should not produce errors about items being missing.
impl MyServer for MyImpl {
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

    async fn bad_channel(
        _rqctx: RequestContext<()>,
        _upgraded: dropshot::WebsocketConnection,
    ) -> dropshot::WebsocketChannelResult {
        Ok(())
    }

    async fn bad_channel2(
        _rqctx: RequestContext<Self::OtherContext>,
        _upgraded: dropshot::WebsocketConnection,
    ) -> dropshot::WebsocketChannelResult {
        Ok(())
    }
}

fn main() {
    // These items should be generated and accessible.
    my_server::api_description::<MyImpl>();
    my_server::stub_api_description();
}
