// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::endpoint;
use dropshot::HttpError;
use dropshot::Query;
use dropshot::RequestContext;
use schemars::JsonSchema;
use serde::Deserialize;

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
struct QueryParams {
    x: String,
}

#[endpoint {
    method = GET,
    path = "/test",
}]
async fn weird_types<'a>(
    _rqctx: RequestContext<T, Self::U>,
    _param1: Query<&'a QueryParams>,
    _param2: for<'b> TypedBody<&'b ()>,
) -> Result<impl HttpResponse, HttpError> {
    Ok(HttpResponseOk(()))
}

fn main() {}
