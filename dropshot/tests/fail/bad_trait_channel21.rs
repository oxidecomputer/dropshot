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

// In this test, since we always output the server trait warts and all, we'll
// get errors produced by both the proc-macro and rustc.

#[dropshot::api_description]
trait MyServer {
    type Context;

    // Test: last parameter is variadic.
    #[channel {
        protocol = WEBSOCKETS,
        path = "/test",
    }]
    async fn variadic_argument(
        _rqctx: RequestContext<Self::Context>,
        _param1: Query<QueryParams>,
        ...
    ) -> dropshot::WebsocketChannelResult;
}

enum MyImpl {}

// This should not produce errors about items being missing.
impl MyServer for MyImpl {
    type Context = ();

    async fn variadic_argument(
        _rqctx: RequestContext<()>,
        _param1: Query<QueryParams>,
        ...
    ) -> dropshot::WebsocketChannelResult {
        Ok(())
    }
}

fn main() {
    // These items should be generated and accessible.
    my_server_mod::api_description::<MyImpl>();
    my_server_mod::stub_api_description();
}
