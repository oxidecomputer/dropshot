// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::Query;
use dropshot::RequestContext;
use dropshot::WebsocketConnection;
use schemars::JsonSchema;
use serde::Deserialize;

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
struct Stuff {
    x: String,
}

#[dropshot::server]
trait MyServer {
    type Context;

    // Test: websocket channel not as the last argument
    #[channel {
        protocol = WEBSOCKETS,
        path = "/test",
    }]
    async fn websocket_channel_not_last(
        _rqctx: RequestContext<Self::Context>,
        _upgraded: WebsocketConnection,
        _query: Query<Stuff>,
    ) -> dropshot::WebsocketChannelResult;
}

enum MyImpl {}

// This should not produce errors about items being missing.
impl MyServer for MyImpl {
    type Context = ();

    async fn websocket_channel_not_last(
        _rqctx: RequestContext<()>,
        _upgraded: WebsocketConnection,
        _query: Query<Stuff>,
    ) -> dropshot::WebsocketChannelResult {
        Ok(())
    }
}

fn main() {
    // These items should be generated and accessible.
    my_server::api_description::<MyImpl>();
    my_server::stub_api_description();
}
