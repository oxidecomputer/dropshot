// Copyright 2020 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::channel;
use dropshot::RequestContext;
use dropshot::WebsocketUpgrade;

#[dropshot::server]
trait MyServer {
    type Context;

    // Test: missing, required `path` attribute.
    #[channel {
        protocol = WEBSOCKETS,
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
    my_server::api_description::<MyImpl>();
    my_server::stub_api_description();
}
