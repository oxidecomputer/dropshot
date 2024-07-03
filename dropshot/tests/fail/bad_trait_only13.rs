// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::HttpError;
use dropshot::HttpResponseUpdatedNoContent;
use dropshot::RequestContext;
use schemars::JsonSchema;
use serde::Serialize;

#[dropshot::api_description]
trait MyApi {
    type Context;

    // Introduce a syntax error outside of a function definition (in standard
    // Rust, "pub" doesn't work here).
    //
    // Currently, this produces a message about "endpoint" and "channel" not
    // being valid. This isn't ideal, but at least the rest of the error output
    // is good.
    #[endpoint {
        method = GET,
        path = "/test",
    }]
    pub async fn bad_endpoint(
        _rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError>;

    #[channel {
        protocol = WEBSOCKETS,
        path = "/test",
    }]
    pub async fn bad_channel(
        _rqctx: RequestContext<Self::Context>,
        _upgraded: dropshot::WebsocketConnection,
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
