// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::HttpError;
use dropshot::HttpResponse;
use dropshot::HttpResponseOk;
use dropshot::Query;
use dropshot::RequestContext;
use dropshot::TypedBody;
use dropshot::WebsocketConnection;
use schemars::JsonSchema;
use serde::Deserialize;

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
struct QueryParams {
    x: String,
}

type MyRequestContext<T, U> = RequestContext<U>;

#[dropshot::api_description]
trait MyServer {
    type Context;

    #[channel {
        protocol = WEBSOCKETS,
        path = "/test",
    }]
    async fn weird_types<'a, T>(
        _rqctx: MyRequestContext<T, Self::Context>,
        _param1: Query<&'a QueryParams>,
        _param2: dyn for<'b> TypedBody<&'b ()>,
    ) -> Result<impl HttpResponse, HttpError> {
        Ok(HttpResponseOk(()))
    }
}

enum MyImpl {}

// This should not produce errors about items being missing.
impl MyServer for MyImpl {
    type Context = ();

    async fn weird_types<'a, T>(
        _rqctx: RequestContext<T, Self::Context>,
        _param1: Query<&'a QueryParams>,
        _param2: dyn for<'b> TypedBody<&'b ()>,
    ) -> Result<impl HttpResponse, HttpError> {
        Ok(HttpResponseOk(()))
    }
}

fn main() {
    // These items should be generated and accessible.
    my_server::api_description::<MyImpl>();
    my_server::stub_api_description();
}
