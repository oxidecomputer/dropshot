// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::channel;
use dropshot::RequestContext;
use dropshot::WebsocketConnection;

trait Stuff {
    fn do_stuff();
}

#[dropshot::server]
trait MyServer {
    type Context;

    #[channel {
        protocol = WEBSOCKETS,
        path = "/test",
    }]
    async fn generics_and_where_clause<S: Stuff + Sync + Send + 'static>(
        _rqctx: RequestContext<S>,
        _upgraded: WebsocketConnection,
    ) -> dropshot::WebsocketChannelResult
    where
        usize: 'static;
}

enum MyImpl {}

// This should not produce errors about items being missing.
impl MyServer for MyImpl {
    type Context = ();

    async fn generics_and_where_clause<S: Stuff + Sync + Send + 'static>(
        _rqctx: RequestContext<S>,
        _upgraded: WebsocketConnection,
    ) -> dropshot::WebsocketChannelResult
    where
        usize: 'static,
    {
        S::do_stuff();
        panic!()
    }
}

fn main() {
    // These items should be generated and accessible.
    my_server::api_description::<MyImpl>();
    my_server::stub_api_description();
}
