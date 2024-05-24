// Copyright 2024 Oxide Computer Company

// The error messages produced by this test aren't great, with messages like
// "can't use generic parameters from outer item" which leak implementation
// details. We should improve this by generating channels directly rather than
// converting them to endpoints.

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
