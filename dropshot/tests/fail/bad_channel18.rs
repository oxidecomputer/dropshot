// Copyright 2023 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::channel;
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

// Test: websocket channel not as the last argument
#[channel {
    protocol = WEBSOCKETS,
    path = "/test",
}]
async fn websocket_channel_not_last(
    _rqctx: RequestContext<()>,
    _upgraded: WebsocketConnection,
    _query: Query<Stuff>,
) -> dropshot::WebsocketChannelResult {
    Ok(())
}

fn main() {}
