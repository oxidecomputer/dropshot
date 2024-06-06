// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::channel;
use dropshot::RequestContext;
use dropshot::WebsocketConnection;

trait Stuff {
    fn do_stuff();
}

#[channel {
    protocol = WEBSOCKETS,
    path = "/test",
}]
async fn bad_channel<S: Stuff + Sync + Send + 'static>(
    _rqctx: RequestContext<S>,
    _upgraded: WebsocketConnection,
) -> dropshot::WebsocketChannelResult {
    S::do_stuff();
    panic!()
}

fn main() {}
