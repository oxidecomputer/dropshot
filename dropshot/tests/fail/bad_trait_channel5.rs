// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::Query;
use dropshot::RequestContext;
use dropshot::WebsocketConnection;
use schemars::JsonSchema;

#[derive(JsonSchema)]
#[allow(dead_code)]
struct QueryParams {
    x: String,
    y: u32,
}

#[dropshot::api_description]
trait MyServer {
    type Context;

    #[channel {
    protocol = WEBSOCKETS,
    path = "/test",
}]
    async fn bad_channel(
        _rqctx: RequestContext<Self::Context>,
        _params: Query<QueryParams>,
        _upgraded: WebsocketConnection,
    ) -> dropshot::WebsocketChannelResult {
        Ok(())
    }
}

enum MyImpl {}

impl MyServer for MyImpl {
    type Context = ();

    async fn bad_channel(
        _rqctx: RequestContext<Self::Context>,
        _params: Query<QueryParams>,
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
