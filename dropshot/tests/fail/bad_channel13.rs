// Copyright 2021 Oxide Computer Company

// XXX: The error message produced by this test isn't great -- we should fix it.

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
