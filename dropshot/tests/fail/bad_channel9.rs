// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::channel;
use dropshot::Query;
use dropshot::RequestContext;
use dropshot::WebsocketConnection;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Serialize, Deserialize, JsonSchema)]
struct QueryParams {
    x: String,
    y: u32,
}

#[channel {
    protocol = WEBSOCKETS,
    path = "/test",
}]
async fn bad_channel(
    _: Query<QueryParams>,
    _: WebsocketConnection,
) -> dropshot::WebsocketChannelResult {
    Ok(())
}

fn main() {}
