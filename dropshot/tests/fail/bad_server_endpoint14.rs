// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::HttpError;
use dropshot::HttpResponseOk;
use dropshot::Path;
use dropshot::RequestContext;
use schemars::JsonSchema;
use serde::Deserialize;

#[derive(JsonSchema, Deserialize)]
struct PathParams {
    stuff: Vec<String>,
}

#[dropshot::server]
trait MyServer {
    type Context;

    #[endpoint {
        method = GET,
        path = "/assets/{stuff:.*}",
    }]
    async fn must_be_unpublished(
        _: RequestContext<Self::Context>,
        _: Path<PathParams>,
    ) -> Result<HttpResponseOk<String>, HttpError>;
}

enum MyImpl {}

// This should not produce errors about items being missing.
impl MyServer for MyImpl {
    type Context = ();

    async fn must_be_unpublished(
        _: RequestContext<()>,
        _: Path<PathParams>,
    ) -> Result<HttpResponseOk<String>, HttpError> {
        panic!()
    }
}

fn main() {}
