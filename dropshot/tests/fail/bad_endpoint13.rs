// Copyright 2021 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::endpoint;
use dropshot::HttpError;
use dropshot::HttpResponseOk;
use dropshot::RequestContext;
use std::sync::Arc;

trait Stuff {
    fn do_stuff();
}

#[endpoint {
    method = GET,
    path = "/test",
}]
async fn bad_response_type<S: Stuff + Sync + Send + 'static>(
    _: Arc<RequestContext<S>>,
) -> Result<HttpResponseOk<String>, HttpError> {
    S::do_stuff();
    panic!()
}

fn main() {}
