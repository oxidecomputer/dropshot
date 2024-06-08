// Copyright 2024 Oxide Computer Company

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

enum MyImpl {}

// This should not produce errors about items being missing.
impl MyServer for MyImpl {
    type Context = ();

    async fn bad_endpoint(
        _rqctx: RequestContext<Self::Context>,
    ) -> Result<HttpResponseOk<Ret>, HttpError> {
        // Validate that compiler errors show up with useful context and aren't
        // obscured by the macro.
        Ok(HttpResponseOk(Ret { "220".to_string(), 0x220 }))
    }
}

fn main() {
    // These items should be generated and accessible.
    my_server::api_description::<MyImpl>();
    my_server::stub_api_description();    
}
