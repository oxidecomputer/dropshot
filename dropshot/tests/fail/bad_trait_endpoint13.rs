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

#[dropshot::api_description]
trait MyApi {
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
impl MyApi for MyImpl {
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

fn main() {
    // These items should be generated and accessible.
    my_api_mod::api_description::<MyImpl>();
    my_api_mod::stub_api_description();
}
