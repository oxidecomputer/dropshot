// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

// Check a method being marked as both a channel and an endpoint, or a channel
// multiple times, produces an error.

use dropshot::RequestContext;
use dropshot::WebsocketConnection;

#[dropshot::server]
trait MyTrait {
    type Context;

    #[channel {
        protocol = WEBSOCKETS,
    }]
    #[endpoint {
        method = GET,
        path = "/test",
    }]
    async fn bad_channel(
        _rqctx: RequestContext<Self::Context>,
        _upgraded: WebsocketConnection,
    ) -> dropshot::WebsocketChannelResult;

    #[channel {
        protocol = WEBSOCKETS,
    }]
    #[channel {
        protocol = WEBSOCKETS,
    }]
    #[channel {
        protocol = WEBSOCKETS,
    }]
    async fn bad_channel2(
        _rqctx: RequestContext<Self::Context>,
        _upgraded: WebsocketConnection,
    ) -> dropshot::WebsocketChannelResult;
}

enum MyImpl {}

// This should not produce errors about items being missing.
impl MyTrait for MyImpl {
    type Context = ();

    async fn bad_channel(
        _rqctx: RequestContext<Self::Context>,
        _upgraded: WebsocketConnection,
    ) -> dropshot::WebsocketChannelResult {
        Ok(())
    }

    async fn bad_channel2(
        _rqctx: RequestContext<Self::Context>,
        _upgraded: WebsocketConnection,
    ) -> dropshot::WebsocketChannelResult {
        Ok(())
    }
}

fn main() {}
