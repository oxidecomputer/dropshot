// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::channel;
use dropshot::Query;
use dropshot::RequestContext;
use dropshot::WebsocketConnection;
use schemars::JsonSchema;
use serde::Deserialize;

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
struct QueryParams {
    x: String,
}

// Test: middle parameter is not a SharedExtractor.
#[channel {
    protocol = WEBSOCKETS,
    path = "/test",
}]
async fn non_extractor_as_last_argument(
    _: RequestContext<()>,
    _: String,
    _: WebsocketConnection,
) -> dropshot::WebsocketChannelResult {
    Ok(())
}

fn main() {}
