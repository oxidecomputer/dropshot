// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::HttpError;
use dropshot::HttpResponseUpdatedNoContent;
use dropshot::RequestContext;
use dropshot::TypedBody;
use dropshot::UntypedBody;
use schemars::JsonSchema;
use serde::Deserialize;

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
struct Stuff {
    x: String,
}

// Test: two exclusive extractors.
// This winds up being tested implicitly by the fact that we test that middle
// parameters impl `SharedExtractor`.  So this winds up being the same as a
// previous test case.  However, it seems worth testing explicitly.
#[dropshot::api_description]
trait MyApi {
    type Context;

    #[endpoint {
        method = GET,
        path = "/test",
    }]
    async fn two_exclusive_extractors(
        _rqctx: RequestContext<Self::Context>,
        _param1: TypedBody<Stuff>,
        _param2: UntypedBody,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError>;
}

enum MyImpl {}

// This should not produce errors about items being missing.
impl MyApi for MyImpl {
    type Context = ();

    async fn two_exclusive_extractors(
        _rqctx: RequestContext<()>,
        _param1: TypedBody<Stuff>,
        _param2: UntypedBody,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError> {
        Ok(HttpResponseUpdatedNoContent())
    }
}

fn main() {
    // These items should be generated and accessible.
    my_api_mod::api_description::<MyImpl>();
    my_api_mod::stub_api_description();
}
