// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::channel;
use dropshot::Path;
use dropshot::RequestContext;
use dropshot::WebsocketConnection;
use schemars::JsonSchema;
use serde::Deserialize;

#[derive(JsonSchema, Deserialize)]
struct PathParams {
    stuff: Vec<String>,
}

// Wildcard endpoints require unpublished = true. Wildcard channels aren't
// supported at all, so this should fail even with unpublished = true.

#[channel {
    protocol = WEBSOCKETS,
    path = "/assets/{stuff:.*}",
    unpublished = true,
}]
async fn channel_with_wildcard(
    _rqctx: RequestContext<()>,
    _path: Path<PathParams>,
    _upgraded: WebsocketConnection,
) -> dropshot::WebsocketChannelResult {
    panic!()
}

fn main() {}
