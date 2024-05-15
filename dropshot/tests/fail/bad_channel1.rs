// Copyright 2020 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::channel;
use dropshot::HttpError;
use dropshot::HttpResponseOk;
use dropshot::WebsocketConnection;

#[channel {
    protocol = WEBSOCKETS,
    path = "/test",
}]
async fn bad_channel(
    _upgraded: WebsocketConnection,
) -> dropshot::WebsocketChannelResult {
    Ok(())
}

fn main() {}
