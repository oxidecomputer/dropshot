// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::HttpError;
use dropshot::HttpResponseOk;
use dropshot::RequestContext;
use schemars::JsonSchema;
use serde::Serialize;

trait Stuff {
    fn do_stuff();
}

#[dropshot::server]
trait MyServer {
    type Context;

    #[endpoint {
        method = GET,
        path = "/test",
    }]
    async fn generics_and_where_clause<S: Stuff + Sync + Send + 'static>(
        _: RequestContext<Self::Context>,
    ) -> Result<HttpResponseOk<String>, HttpError>
    where
        usize: 'static;
}

enum MyImpl {}

// This should not produce errors about items being missing.
impl MyServer for MyImpl {
    type Context = ();

    async fn generics_and_where_clause<S: Stuff + Sync + Send + 'static>(
        _: RequestContext<()>,
    ) -> Result<HttpResponseOk<String>, HttpError>
    where
        usize: 'static,
    {
        S::do_stuff();
        panic!()
    }
}

fn main() {}
