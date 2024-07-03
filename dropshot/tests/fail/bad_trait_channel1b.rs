// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::WebsocketConnection;

#[dropshot::api_description]
trait MyServer {
    type Context;

    #[channel {
        protocol = WEBSOCKETS,
        path = "/test",
    }]
    async fn bad_channel(
        _upgraded: WebsocketConnection,
    ) -> dropshot::WebsocketChannelResult {
        Ok(())
    }
}

enum MyImpl {}

// This should not produce errors about items being missing.
impl MyServer for MyImpl {
    type Context = ();

    async fn bad_channel(
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
