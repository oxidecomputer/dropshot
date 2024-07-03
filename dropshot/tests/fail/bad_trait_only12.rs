// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::HttpError;
use dropshot::HttpResponseUpdatedNoContent;

// Check that MyApi is present in the generated output, even though the
// #[server] annotation is incorrect.
#[dropshot::api_description { context = 123 }]
trait MyApi {
    type Context;

    // Note here that the endpoint is incorrect (doesn't have an rqctx
    // parameter) -- we currently don't perform any validation if the
    // dropshot::api_description macro is invalid.
    #[endpoint {
        method = GET,
        path = "/test",
    }]
    async fn bad_endpoint() -> Result<HttpResponseUpdatedNoContent, HttpError>;
}

enum MyImpl {}

// This should not produce errors about items being missing.
impl MyApi for MyImpl {
    type Context = ();

    async fn bad_endpoint() -> Result<HttpResponseUpdatedNoContent, HttpError> {
        Ok(HttpResponseUpdatedNoContent())
    }
}

fn main() {
    // The parent `my_api` will NOT be present because of the invalid trait,
    // and will cause errors to be generated.
    my_api::foo();
}
