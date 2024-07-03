// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

// Check that a reasonable error is produced if `dropshot::channel` is used on
// a trait method rather than `dropshot::api_description`.

use dropshot::channel;
use dropshot::RequestContext;
use dropshot::WebsocketConnection;

trait MyTrait {
    type Context;

    #[channel {
        protocol = WEBSOCKETS,
        path = "/test",
    }]
    async fn bad_channel(
        _rqctx: RequestContext<Self::Context>,
        _upgraded: WebsocketConnection,
    ) -> dropshot::WebsocketChannelResult;
}

fn main() {}
