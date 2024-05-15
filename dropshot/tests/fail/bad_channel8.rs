// Copyright 2020 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::channel;
use dropshot::RequestContext;
use dropshot::WebsocketConnection;

#[channel {
    protocol = WEBSOCKETS,
    path = "/test",
}]
fn bad_channel(
    _rqctx: RequestContext<()>,
    _upgraded: WebsocketConnection,
) -> dropshot::WebsocketChannelResult {
    Ok(())
}

fn main() {}
