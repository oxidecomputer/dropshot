// Copyright 2023 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::endpoint;
use dropshot::HttpError;
use dropshot::HttpResponseOk;
use dropshot::RequestContext;
use dropshot::TypedBody;
use dropshot::UntypedBody;
use schemars::JsonSchema;
use serde::Deserialize;
use std::sync::Arc;

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
struct Stuff {
    x: String,
}

// Test: two exclusive extractors.
// This winds up being tested implicitly by the fact that we test that middle
// parameters impl `SharedExtractor`.  So this winds up being the same as a
// previous test case.  However, it seems worth testing explicitly.
#[endpoint {
    method = GET,
    path = "/test",
}]
async fn two_exclusive_extractors(
    _rqctx: Arc<RequestContext<()>>,
    _param1: TypedBody<Stuff>,
    _param2: UntypedBody,
) -> Result<HttpResponseOk<()>, HttpError> {
    Ok(HttpResponseOk(()))
}

fn main() {}
