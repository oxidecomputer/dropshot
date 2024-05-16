// Copyright 2020 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::channel;
use dropshot::RequestContext;
use std::sync::Arc;

// Test: final parameter is not a WebsocketConnection
#[channel {
    protocol = WEBSOCKETS,
    path = "/test",
}]
async fn bad_channel(
    _rqctx: RequestContext<()>,
    _param: String,
) -> dropshot::WebsocketChannelResult {
    Ok(())
}

fn main() {}
