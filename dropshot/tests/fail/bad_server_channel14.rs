// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

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

#[dropshot::server]
trait MyServer {
    type Context;

    #[channel {
        protocol = WEBSOCKETS,
        path = "/assets/{stuff:.*}",
        unpublished = true,
    }]
    async fn channel_with_wildcard(
        _rqctx: RequestContext<Self::Context>,
        _path: Path<PathParams>,
        _upgraded: WebsocketConnection,
    ) -> dropshot::WebsocketChannelResult;
}

enum MyImpl {}

// This should not produce errors about items being missing.
impl MyServer for MyImpl {
    type Context = ();

    async fn channel_with_wildcard(
        _rqctx: RequestContext<()>,
        _path: Path<PathParams>,
        _upgraded: WebsocketConnection,
    ) -> dropshot::WebsocketChannelResult {
        panic!()
    }
}

fn main() {
    // These items should be generated and accessible.
    my_server::api_description::<MyImpl>();
    my_server::stub_api_description();
}
