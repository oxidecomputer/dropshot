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

// Test: last parameter is variadic.
#[channel {
    protocol = WEBSOCKETS,
    path = "/test",
}]
async fn variadic_argument(
    _rqctx: RequestContext<()>,
    _param1: Query<QueryParams>,
    ...
) -> dropshot::WebsocketChannelResult {
    Ok(())
}

fn main() {}
