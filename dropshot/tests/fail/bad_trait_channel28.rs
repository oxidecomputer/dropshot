// Copyright 2020 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::channel;
use dropshot::RequestContext;
use dropshot::WebsocketUpgrade;

#[dropshot::api_description]
trait MyServer {
    type Context;

    // Test: request_body_max_bytes specified for channel (this parameter is only
    // accepted for endpoints, not channels)
    #[channel {
        protocol = WEBSOCKETS,
        path = "/test",
        request_body_max_bytes = 1024,
    }]
    async fn bad_channel(
        _rqctx: RequestContext<Self::Context>,
        _upgraded: WebsocketUpgrade,
    ) -> dropshot::WebsocketChannelResult;
}

enum MyImpl {}

// This should not produce errors about items being missing.
impl MyServer for MyImpl {
    type Context = ();

    async fn bad_channel(
        _rqctx: RequestContext<Self::Context>,
        _upgraded: WebsocketUpgrade,
    ) -> dropshot::WebsocketChannelResult {
        Ok(())
    }
}

fn main() {
    // These items should be generated and accessible.
    my_server_mod::api_description::<MyImpl>();
    my_server_mod::stub_api_description();
}
