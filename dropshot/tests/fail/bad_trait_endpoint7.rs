// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::HttpError;
use dropshot::HttpResponseOk;
use dropshot::Query;
use dropshot::RequestContext;
use schemars::JsonSchema;

#[derive(JsonSchema)]
#[allow(dead_code)]
struct Ret {
    x: String,
    y: u32,
}

#[dropshot::api_description]
trait MyApi {
    type Context;

    #[endpoint {
        method = GET,
        path = "/test",
    }]
    async fn bad_endpoint(
        _rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseOk<Ret>, HttpError>;
}

enum MyImpl {}

// This should not produce errors about items being missing. However, it does
// produce errors about the `QueryParams` type not having the right traits.
impl MyApi for MyImpl {
    type Context = ();

    async fn bad_endpoint(
        _rqctx: RequestContext<()>,
    ) -> Result<HttpResponseOk<Ret>, HttpError> {
        Ok(HttpResponseOk(Ret { x: "Oxide".to_string(), y: 0x1de }))
    }
}

fn main() {
    // These items should be generated and accessible.
    my_api_mod::api_description::<MyImpl>();
    my_api_mod::stub_api_description();
}
