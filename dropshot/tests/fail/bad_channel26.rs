// Copyright 2020 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::channel;
use dropshot::RequestContext;
use dropshot::WebsocketUpgrade;

// Test: missing, required `path` attribute.

#[channel {
    protocol = WEBSOCKETS,
}]
async fn bad_channel(
    _rqctx: RequestContext<()>,
    _upgraded: WebsocketUpgrade,
) -> dropshot::WebsocketChannelResult {
    Ok(())
}

fn main() {}
