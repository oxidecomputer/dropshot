// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::HttpError;
use dropshot::HttpResponseUpdatedNoContent;

// Check that MyServer is present in the generated output, even though the
// #[server] annotation is incorrect.
#[dropshot::server { context = 123 }]
trait MyServer {
    type Context;

    // Note here that the endpoint is incorrect (doesn't have an rqctx
    // parameter) -- we currently don't perform any validation if the
    // dropshot::server macro is invalid.
    #[endpoint {
        method = GET,
        path = "/test",
    }]
    async fn bad_endpoint() -> Result<HttpResponseUpdatedNoContent, HttpError>;
}

enum MyImpl {}

// This should not produce errors about items being missing.
impl MyServer for MyImpl {
    type Context = ();

    async fn bad_endpoint() -> Result<HttpResponseUpdatedNoContent, HttpError> {
        Ok(HttpResponseUpdatedNoContent())
    }
}

fn main() {
    // The parent `my_server` will NOT be present because of the invalid trait,
    // and will cause errors to be generated.
    my_server::foo();
}
