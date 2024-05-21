// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::channel;

#[channel {
    protocol = UNKNOWN,
    path = "/test",
}]
async fn bad_channel() -> dropshot::WebsocketChannelResult {
    Ok(())
}

fn main() {
    bad_channel(); // this line should not produce any errors
}
