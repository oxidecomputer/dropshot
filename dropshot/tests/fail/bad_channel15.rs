// Copyright 2022 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::channel;
use dropshot::RequestContext;
use dropshot::WebsocketConnection;
use std::rc::Rc;
use std::time::Duration;

#[channel {
    protocol = WEBSOCKETS,
    path = "/test",
}]
async fn bad_channel(
    _rqctx: RequestContext<()>,
    _upgraded: WebsocketConnection,
) -> dropshot::WebsocketChannelResult {
    let _non_send_type = Rc::new(0);
    tokio::time::sleep(Duration::from_millis(1)).await;
    Ok(())
}

fn main() {}
