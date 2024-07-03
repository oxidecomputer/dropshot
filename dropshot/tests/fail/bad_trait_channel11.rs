// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::RequestContext;
use dropshot::WebsocketConnection;

#[dropshot::api_description]
trait MyServer {
    type Context;

    #[channel {
        protocol = WEBSOCKETS,
        path = "/test",
    }]
    async fn bad_no_result(
        _rqctx: RequestContext<Self::Context>,
        _upgraded: WebsocketConnection,
    );
}

enum MyImpl {}

// This should not produce errors about items being missing.
impl MyServer for MyImpl {
    type Context = ();

    async fn bad_no_result(
        _rqctx: RequestContext<()>,
        _upgraded: WebsocketConnection,
    ) {
    }
}

fn main() {
    // These items should be generated and accessible.
    my_server::api_description::<MyImpl>();
    my_server::stub_api_description();
}
