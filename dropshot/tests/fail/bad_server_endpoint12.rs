// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::HttpError;
use dropshot::HttpResponseOk;
use dropshot::RequestContext;
use schemars::JsonSchema;
use serde::Serialize;

#[derive(JsonSchema, Serialize)]
struct Ret {}

#[dropshot::server]
trait MyServer {
    type Context;

    #[endpoint {
        method = GET,
        path = "/test",
    }]
    async fn bad_response_type(
        _: RequestContext<Self::Context>,
    ) -> Result<String, HttpError>;
}

enum MyImpl {}

// This should not produce errors about items being missing.
impl MyServer for MyImpl {
    type Context = ();

    async fn bad_response_type(
        _: RequestContext<()>,
    ) -> Result<String, HttpError> {
        Ok("aok".to_string())
    }
}

fn main() {}
