// Copyright 2021 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::endpoint;
use dropshot::HttpError;
use dropshot::HttpResponseOk;
use dropshot::RequestContext;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

#[endpoint {
    method = GET,
    path = "/test",
}]
async fn bad_endpoint(
    _rqctx: Arc<RequestContext<()>>,
) -> Result<HttpResponseOk<i32>, HttpError> {
    let non_send_type = Rc::new(0);
    tokio::time::sleep(Duration::from_millis(1)).await;
    Ok(HttpResponseOk(*non_send_type))
}

fn main() {}