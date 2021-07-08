// Copyright 2020 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::endpoint;
use dropshot::RequestContext;
use std::sync::Arc;

#[endpoint {
    method = GET,
    path = "/test",
}]
async fn bad_no_result(_: Arc<RequestContext<()>>) {}

fn main() {}
