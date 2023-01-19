// Copyright 2021 Oxide Computer Company

// For whatever reason, trybuild overrides the default disposition for the
// dead_code lint from "warn" to "allow" so we explicitly deny it here.
#![deny(dead_code)]

use dropshot::endpoint;
use dropshot::HttpError;
use dropshot::HttpResponseOk;
use dropshot::RequestContext;

// At some point we'd expect to see code like:
// ```
//     let mut api = ApiDescription::new();
//     api.register(unused_endpoint).unwrap();
// ```
// Defining but not using the endpoint should cause a warning.
#[endpoint {
    method = GET,
    path = "/test",
}]
async fn unused_endpoint(
    _rqctx: RequestContext<()>,
) -> Result<HttpResponseOk<()>, HttpError> {
    Ok(HttpResponseOk(()))
}

fn main() {}
