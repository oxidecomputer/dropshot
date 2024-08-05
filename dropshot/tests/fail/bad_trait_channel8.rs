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
    fn bad_channel(
        _rqctx: RequestContext<Self::Context>,
        _upgraded: WebsocketConnection,
    ) -> dropshot::WebsocketChannelResult {
        Ok(())
    }
}

enum MyImpl {}

// This should not produce errors about items being missing.
impl MyServer for MyImpl {
    type Context = ();

    fn bad_channel(
        _rqctx: RequestContext<Self::Context>,
        _upgraded: WebsocketConnection,
    ) -> dropshot::WebsocketChannelResult {
        Ok(())
    }
}

fn main() {
    // These items should be generated and accessible.
    my_server_mod::api_description::<MyImpl>();
    my_server_mod::stub_api_description();
}
