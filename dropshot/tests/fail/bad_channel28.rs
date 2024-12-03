// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::channel;
use dropshot::RequestContext;
use dropshot::WebsocketUpgrade;

// Test: request_body_max_bytes specified for channel (this parameter is only
// accepted for endpoints, not channels)

#[channel {
    protocol = WEBSOCKETS,
    path = "/test",
    request_body_max_bytes = 1024,
}]
async fn bad_channel(
    _rqctx: RequestContext<()>,
    _upgraded: WebsocketUpgrade,
) -> dropshot::WebsocketChannelResult {
    Ok(())
}

fn main() {}
