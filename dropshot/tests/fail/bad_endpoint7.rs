// Copyright 2020 Oxide Computer Company

use dropshot::endpoint;
use dropshot::ExtractedParameter;
use dropshot::HttpError;
use dropshot::HttpResponseOk;
use dropshot::RequestContext;
use std::sync::Arc;

#[derive(ExtractedParameter)]
#[allow(dead_code)]
struct Ret {
    x: String,
    y: u32,
}

#[endpoint {
    method = GET,
    path = "/test",
}]
async fn bad_endpoint(
    _rqctx: Arc<RequestContext>,
) -> Result<HttpResponseOk<Ret>, HttpError> {
    Ok(HttpResponseOk(Ret {
        x: "Oxide".to_string(),
        y: 0x1de,
    }))
}

fn main() {}
