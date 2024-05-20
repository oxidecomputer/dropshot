// Copyright 2020 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::channel;
use dropshot::HttpError;
use dropshot::RequestContext;
use dropshot::WebsocketConnection;

#[channel {
    protocol = WEBSOCKETS,
    path = "/test",
}]
async fn bad_response_type(
    _rqctx: RequestContext<()>,
    _upgraded: WebsocketConnection,
) -> Result<String, Box<dyn std::error::Error + Send + Sync + 'static>> {
    Ok("aok".to_string())
}

fn main() {}
