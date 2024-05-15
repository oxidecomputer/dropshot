// Copyright 2023 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::channel;
use dropshot::RequestContext;
use dropshot::WebsocketConnection;
use schemars::JsonSchema;
use serde::Deserialize;

// Test: two websocket channels.
#[channel {
    protocol = WEBSOCKETS,
    path = "/test",
}]
async fn two_websocket_channels(
    _rqctx: RequestContext<()>,
    _upgraded1: WebsocketConnection,
    _upgraded2: WebsocketConnection,
) -> dropshot::WebsocketChannelResult {
    Ok(())
}

fn main() {}
