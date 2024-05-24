// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::channel;
use dropshot::Query;
use dropshot::RequestContext;
use dropshot::WebsocketConnection;
use schemars::JsonSchema;

#[derive(JsonSchema)]
#[allow(dead_code)]
struct QueryParams {
    x: String,
    y: u32,
}

#[channel {
    protocol = WEBSOCKETS,
    path = "/test",
}]
async fn bad_channel(
    _rqctx: RequestContext<()>,
    _params: Query<QueryParams>,
    _upgraded: WebsocketConnection,
) -> dropshot::WebsocketChannelResult {
    Ok(())
}

fn main() {}
