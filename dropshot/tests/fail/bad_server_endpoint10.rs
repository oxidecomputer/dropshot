// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::HttpError;
use dropshot::HttpResponseUpdatedNoContent;
use dropshot::RequestContext;

#[dropshot::server]
trait MyServer {
    type Context;

    #[endpoint {
        method = GET,
        path = "/test",
    }]
    async fn bad_error_type(
        _rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseUpdatedNoContent, String>;
}

enum MyImpl {}

// This should not produce errors about items being missing. However, it does
// produce errors about `String` not being HttpError.
impl MyServer for MyImpl {
    type Context = ();

    async fn bad_error_type(
        _rqctx: RequestContext<()>,
    ) -> Result<HttpResponseUpdatedNoContent, String> {
        Ok(HttpResponseUpdatedNoContent())
    }
}

fn main() {
    // These items should be generated and accessible.
    my_server::api_description::<MyImpl>();
    my_server::stub_api_description();
}
