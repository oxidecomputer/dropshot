// Copyright 2023 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::endpoint;
use dropshot::HttpError;
use dropshot::HttpResponseOk;
use dropshot::UntypedBody;

#[endpoint {
    method = GET,
    path = "/test",
    request_body_max_bytes = true,
}]
async fn bad_request_body_max_bytes(
    _rqctx: RequestContext<()>,
    param: UntypedBody,
) -> Result<HttpResponseOk<()>, HttpError> {
    Ok(HttpResponseOk(()))
}

fn main() {}
