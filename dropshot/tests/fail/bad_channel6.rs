// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::channel;
use dropshot::RequestContext;
use dropshot::WebsocketConnection;
use schemars::JsonSchema;
use serde::Serialize;

#[derive(JsonSchema, Serialize)]
#[allow(dead_code)]
struct Ret {
    x: String,
    y: u32,
}

#[channel {
    protocol = WEBSOCKETS,
    path = "/test",
}]
async fn bad_channel(
    _rqctx: RequestContext<()>,
    _upgraded: WebsocketConnection,
) -> dropshot::WebsocketChannelResult {
    // Validate that compiler errors show up with useful context and aren't
    // obscured by the macro.
    let _ = Ret { "Oxide".to_string(), 0x1de };
    Ok(())
}

fn main() {}
