// Copyright 2023 Oxide Computer Company

// This does not currently work, but we may want to support it in the future.

#![allow(unused_imports)]

use dropshot::endpoint;
use dropshot::HttpError;
use dropshot::HttpResponseOk;
use dropshot::UntypedBody;

const MAX_REQUEST_BYTES: u64 = 400_000;

#[endpoint {
    method = GET,
    path = "/test",
    request_body_max_bytes = MAX_REQUEST_BYTES,
}]
async fn bad_request_body_max_bytes(
    _rqctx: RequestContext<()>,
    param: UntypedBody,
) -> Result<HttpResponseOk<()>, HttpError> {
    Ok(HttpResponseOk(()))
}

fn main() {}
