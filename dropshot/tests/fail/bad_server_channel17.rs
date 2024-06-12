// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::RequestContext;
use dropshot::WebsocketConnection;
use schemars::JsonSchema;
use serde::Deserialize;

#[dropshot::server]
trait MyServer {
    type Context;

    // Test: two websocket channels.
    #[channel {
        protocol = WEBSOCKETS,
        path = "/test",
    }]
    async fn two_websocket_channels(
        _rqctx: RequestContext<Self::Context>,
        _upgraded1: WebsocketConnection,
        _upgraded2: WebsocketConnection,
    ) -> dropshot::WebsocketChannelResult;
}

enum MyImpl {}

// This should not produce errors about items being missing.
impl MyServer for MyImpl {
    type Context = ();

    async fn two_websocket_channels(
        _rqctx: RequestContext<()>,
        _upgraded1: WebsocketConnection,
        _upgraded2: WebsocketConnection,
    ) -> dropshot::WebsocketChannelResult {
        Ok(())
    }
}

fn main() {
    // These items should be generated and accessible.
    my_server::api_description::<MyImpl>();
    my_server::stub_api_description();
}
