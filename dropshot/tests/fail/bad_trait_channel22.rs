// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

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

#[dropshot::api_description]
trait MyServer {
    type Context;

    // Test: unsafe fn.
    #[channel {
        protocol = WEBSOCKETS,
        path = "/test",
    }]
    async unsafe fn unsafe_endpoint(
        _rqctx: RequestContext<Self::Context>,
        _param1: Query<QueryParams>,
        _upgraded: WebsocketConnection,
    ) -> dropshot::WebsocketChannelResult;
}

enum MyImpl {}

// This should not produce errors about items being missing.
impl MyServer for MyImpl {
    type Context = ();

    async unsafe fn unsafe_endpoint(
        _rqctx: RequestContext<()>,
        _param1: Query<QueryParams>,
        _upgraded: WebsocketConnection,
    ) -> dropshot::WebsocketChannelResult {
        Ok(())
    }
}

fn main() {
    // These items should be generated and accessible.
    my_server::api_description::<MyImpl>();
    my_server::stub_api_description();
}
