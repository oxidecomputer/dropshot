// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::HttpError;
use dropshot::HttpResponseUpdatedNoContent;
use dropshot::RequestContext;
use std::sync::Arc;

// Test: final parameter is not a WebsocketConnection

#[dropshot::api_description]
trait MyServer {
    type Context;

    // Test: final parameter is neither an ExclusiveExtractor nor a SharedExtractor.
    #[channel {
        protocol = WEBSOCKETS,
        path = "/test",
    }]
    async fn bad_channel(
        _rqctx: RequestContext<Self::Context>,
        _param: String,
    ) -> dropshot::WebsocketChannelResult;
}

enum MyImpl {}

impl MyServer for MyImpl {
    type Context = Arc<()>;

    async fn bad_channel(
        _rqctx: RequestContext<Self::Context>,
        _param: String,
    ) -> dropshot::WebsocketChannelResult {
        Ok(())
    }
}

fn main() {
    // These items should be generated and accessible.
    my_server_mod::api_description::<MyImpl>();
    my_server_mod::stub_api_description();
}
