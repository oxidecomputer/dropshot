// Copyright 2021 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::endpoint;
use dropshot::HttpError;
use dropshot::HttpResponseOk;
use dropshot::Path;
use dropshot::RequestContext;
use schemars::JsonSchema;
use serde::Deserialize;
use std::sync::Arc;

#[derive(JsonSchema, Deserialize)]
struct PathParams {
    stuff: Vec<String>,
}

#[endpoint {
    method = GET,
    path = "/assets/{stuff:.*}",
}]
async fn must_be_unpublished(
    _: RequestContext<()>,
    _: Path<PathParams>,
) -> Result<HttpResponseOk<String>, HttpError> {
    panic!()
}

fn main() {}
