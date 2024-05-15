// Copyright 2021 Oxide Computer Company

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
