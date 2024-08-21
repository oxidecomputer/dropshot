// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

// Check that a reasonable error is produced if `dropshot::endpoint` is used on
// a trait method rather than `dropshot::api_description`.

use dropshot::endpoint;
use dropshot::HttpError;
use dropshot::HttpResponseUpdatedNoContent;
use dropshot::RequestContext;

trait MyTrait {
    type Context;

    #[endpoint {
        method = GET,
        path = "/test",
    }]
    async fn bad_endpoint(
        _rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError>;
}

fn main() {}
