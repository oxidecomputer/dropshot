// Copyright 2024 Oxide Computer Company

// XXX: There's probably no good reason to support wildcards in channel
// endpoints. We should just ban them.

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

#[channel {
    protocol = WEBSOCKETS,
    path = "/assets/{stuff:.*}",
}]
async fn must_be_unpublished(
    _rqctx: RequestContext<()>,
    _path: Path<PathParams>,
    _upgraded: WebsocketConnection,
) -> dropshot::WebsocketChannelResult {
    panic!()
}

fn main() {}
