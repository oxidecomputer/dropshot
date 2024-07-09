// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::HttpError;
use dropshot::HttpResponseUpdatedNoContent;
use dropshot::RequestContext;

// Test _dropshot_crate provided in annotations rather than at the
// `#[dropshot::api_description]` level.

#[dropshot::api_description]
trait MyApi {
    type Context;

    #[endpoint { method = GET, path = "/test", _dropshot_crate = "dropshot" }]
    async fn bad_endpoint(
        _rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError>;

    #[channel { protocol = WEBSOCKETS, path = "/test", _dropshot_crate = "dropshot" }]
    async fn bad_channel(
        _rqctx: RequestContext<Self::Context>,
        _upgraded: dropshot::WebsocketConnection,
    ) -> dropshot::WebsocketChannelResult;
}

enum MyImpl {}

// This should not produce errors about the trait or any of the items within
// being missing.
impl MyApi for MyImpl {
    type Context = ();

    async fn bad_endpoint(
        _rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError> {
        Ok(HttpResponseUpdatedNoContent())
    }

    async fn bad_channel(
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
