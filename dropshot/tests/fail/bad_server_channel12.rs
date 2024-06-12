// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::HttpError;
use dropshot::RequestContext;
use dropshot::WebsocketConnection;

#[dropshot::server]
trait MyServer {
    type Context;

    #[channel {
        protocol = WEBSOCKETS,
        path = "/test",
    }]
    async fn bad_response_type(
        _rqctx: RequestContext<Self::Context>,
        _upgraded: WebsocketConnection,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync + 'static>>;
}

enum MyImpl {}

// This should not produce errors about items being missing.
impl MyServer for MyImpl {
    type Context = ();

    async fn bad_response_type(
        _rqctx: RequestContext<()>,
        _upgraded: WebsocketConnection,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync + 'static>>
    {
        Ok("aok".to_string())
    }
}

fn main() {
    // These items should be generated and accessible.
    my_server::api_description::<MyImpl>();
    my_server::stub_api_description();
}
