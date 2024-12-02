// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::HttpResponseUpdatedNoContent;
use dropshot::RequestContext;

#[dropshot::api_description]
trait MyApi {
    type Context;

    #[endpoint { method = GET, path = "/test" }]
    async fn bad_error_type(
        &self,
        rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseUpdatedNoContent, String>;
}

enum MyImpl {}

// This should not produce errors about items being missing.
impl MyApi for MyImpl {
    type Context = ();

    async fn bad_error_type(
        &self,
        _rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseUpdatedNoContent, String> {
        todo!()
    }
}

fn main() {
    // These items should be generated and accessible.
    my_api_mod::api_description::<MyImpl>();
    my_api_mod::stub_api_description();
}
