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

#[dropshot::server]
trait MyServer {
    type Context;

    // Test: ABI in endpoint.
    #[channel {
        protocol = WEBSOCKETS,
        path = "/test",
    }]
    async extern "C" fn abi_endpoint(
        _rqctx: RequestContext<Self::Context>,
        _param1: Query<QueryParams>,
        _upgraded: WebsocketConnection,
    ) -> dropshot::WebsocketChannelResult {
        Ok(())
    }
}

enum MyImpl {}

// This should not produce errors about items being missing.
impl MyServer for MyImpl {
    type Context = ();

    async extern "C" fn abi_endpoint(
        _rqctx: RequestContext<Self::Context>,
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
