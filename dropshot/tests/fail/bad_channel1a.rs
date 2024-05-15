// Copyright 2020 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::channel;

#[channel {
    protocol = WEBSOCKETS,
    path = "/test",
}]
async fn bad_channel() -> dropshot::WebsocketChannelResult {
    Ok(())
}

fn main() {}
