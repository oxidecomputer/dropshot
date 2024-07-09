// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::RequestContext;
use dropshot::WebsocketConnection;

// Check that MyServer is present in the generated output, even though one
// of the endpoint annotations is incorrect.
#[dropshot::api_description]
trait MyServer {
    type Context;

    #[channel {
        protocol = UNKNOWN,
        path = "/test",
    }]
    async fn bad_channel(
        _: RequestContext<Self::Context>,
        _: WebsocketConnection,
    ) -> dropshot::WebsocketChannelResult;
}

enum MyImpl {}

impl MyServer for MyImpl {
    type Context = ();

    async fn bad_channel(
        _: RequestContext<Self::Context>,
        _: WebsocketConnection,
    ) -> dropshot::WebsocketChannelResult {
        Ok(())
    }
}

fn main() {
    // These items should be generated and accessible.
    my_server::api_description::<MyImpl>();
    my_server::stub_api_description();
}
