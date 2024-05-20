// Copyright 2020 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::HttpError;
use dropshot::HttpResponseOk;
use dropshot::RequestContext;
use schemars::JsonSchema;
use serde::Serialize;

#[derive(JsonSchema, Serialize)]
#[allow(dead_code)]
struct Ret {
    x: String,
    y: u32,
}

#[dropshot::server]
trait MyServer {
    type Context;

    #[endpoint {
        method = GET,
        path = "/test",
    }]
    async fn bad_endpoint(
        _rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseOk<Ret>, HttpError> {
        // Validate that compiler errors show up with useful context and aren't
        // obscured by the macro.
        Ok(HttpResponseOk(Ret { "Oxide".to_string(), 0x1de }))
    }
}

fn main() {}
