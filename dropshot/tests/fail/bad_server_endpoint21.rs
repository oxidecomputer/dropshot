// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::HttpError;
use dropshot::HttpResponseUpdatedNoContent;
use dropshot::Query;
use dropshot::RequestContext;
use schemars::JsonSchema;
use serde::Deserialize;

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
struct QueryParams {
    x: String,
}

// In this test, since we always output the server trait warts and all, we'll
// get errors produced by both the proc-macro and rustc.

#[dropshot::server]
trait MyServer {
    type Context;

    // Test: last parameter is variadic, extern "C", and unsafe.
    #[endpoint {
        method = GET,
        path = "/test",
    }]
    async fn variadic_argument(
        _rqctx: RequestContext<Self::Context>,
        _param1: Query<QueryParams>,
        ...
    ) -> Result<HttpResponseUpdatedNoContent, HttpError>;

    // Test: unsafe fn.
    #[endpoint {
        method = GET,
        path = "/test",
    }]
    async unsafe fn unsafe_endpoint(
        _rqctx: RequestContext<Self::Context>,
        _param1: Query<QueryParams>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError>;

    // Test: const fn.
    #[endpoint {
        method = GET,
        path = "/test",
    }]
    const fn const_endpoint(
        _rqctx: RequestContext<Self::Context>,
        _param1: Query<QueryParams>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError>;

    // Test: ABI in fn.
    #[endpoint {
        method = GET,
        path = "/test",
    }]
    async extern "C" fn abi_endpoint(
        _rqctx: RequestContext<Self::Context>,
        _param1: Query<QueryParams>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError>;
}

enum MyImpl {}

// This should not produce errors about items being missing.
impl MyServer for MyImpl {
    type Context = ();

    async fn variadic_argument(
        _rqctx: RequestContext<Self::Context>,
        _param1: Query<QueryParams>,
        ...
    ) -> Result<HttpResponseUpdatedNoContent, HttpError> {
        unreachable!()
    }

    async unsafe fn unsafe_endpoint(
        _rqctx: RequestContext<Self::Context>,
        _param1: Query<QueryParams>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError> {
        Ok(HttpResponseUpdatedNoContent())
    }

    const fn const_endpoint(
        _rqctx: RequestContext<Self::Context>,
        _param1: Query<QueryParams>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError> {
        Ok(HttpResponseUpdatedNoContent())
    }

    async extern "C" fn abi_endpoint(
        _rqctx: RequestContext<Self::Context>,
        _param1: Query<QueryParams>,
    ) -> Result<HttpResponseUpdatedNoContent, HttpError> {
        Ok(HttpResponseUpdatedNoContent())
    }
}

fn main() {
    // These items should be generated and accessible.
    my_server::api_description::<MyImpl>();
    my_server::stub_api_description();
}
