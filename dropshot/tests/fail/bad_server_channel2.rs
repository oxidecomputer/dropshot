// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::RequestContext;
use dropshot::WebsocketConnection;
use std::sync::Arc;

#[dropshot::server]
trait MyServer {
    type Context;

    #[channel {
        protocol = WEBSOCKETS,
        path = "/test",
    }]
    async fn ref_self_method(
        &self,
        rqctx: RequestContext<Self::Context>,
        upgraded: WebsocketConnection,
    ) -> dropshot::WebsocketChannelResult;

    #[channel {
        protocol = WEBSOCKETS,
        path = "/test",
    }]
    async fn mut_self_method(
        self,
        rqctx: RequestContext<Self::Context>,
        upgraded: WebsocketConnection,
    ) -> dropshot::WebsocketChannelResult;

    #[channel {
        protocol = WEBSOCKETS,
        path = "/test",
    }]
    async fn self_method(
        self,
        rqctx: RequestContext<Self::Context>,
        upgraded: WebsocketConnection,
    ) -> dropshot::WebsocketChannelResult;

    #[channel {
        protocol = WEBSOCKETS,
        path = "/test",
    }]
    async fn self_box_self_method(
        self: Box<Self>,
        rqctx: RequestContext<Self::Context>,
        upgraded: WebsocketConnection,
    ) -> dropshot::WebsocketChannelResult;

    #[channel {
        protocol = WEBSOCKETS,
        path = "/test",
    }]
    async fn self_arc_self_method(
        self: Arc<Self>,
        rqctx: RequestContext<Self::Context>,
        upgraded: WebsocketConnection,
    ) -> dropshot::WebsocketChannelResult;
}

enum MyImpl {}

// This should not produce errors about items being missing.
impl MyServer for MyImpl {
    type Context = ();

    async fn ref_self_method(
        &self,
        _rqctx: RequestContext<Self::Context>,
        _upgraded: WebsocketConnection,
    ) -> dropshot::WebsocketChannelResult {
        Ok(())
    }

    async fn mut_self_method(
        self,
        _rqctx: RequestContext<Self::Context>,
        _upgraded: WebsocketConnection,
    ) -> dropshot::WebsocketChannelResult {
        Ok(())
    }

    async fn self_method(
        self,
        _rqctx: RequestContext<Self::Context>,
        _upgraded: WebsocketConnection,
    ) -> dropshot::WebsocketChannelResult {
        Ok(())
    }

    async fn self_box_self_method(
        self: Box<Self>,
        _rqctx: RequestContext<Self::Context>,
        _upgraded: WebsocketConnection,
    ) -> dropshot::WebsocketChannelResult {
        Ok(())
    }

    async fn self_arc_self_method(
        self: Arc<Self>,
        _rqctx: RequestContext<Self::Context>,
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
