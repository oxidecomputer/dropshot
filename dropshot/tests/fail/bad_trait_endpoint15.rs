// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::HttpError;
use dropshot::HttpResponseOk;
use dropshot::RequestContext;
use std::rc::Rc;
use std::time::Duration;

#[dropshot::api_description]
trait MyApi {
    type Context;

    #[endpoint {
        method = GET,
        path = "/test",
    }]
    async fn bad_endpoint(
        _: RequestContext<Self::Context>,
    ) -> Result<HttpResponseOk<i32>, HttpError> {
        // Note: we're check error messages from the default trait-provided impl
        // -- that's the code the proc macro actually transforms.
        let non_send_type = Rc::new(0);
        tokio::time::sleep(Duration::from_millis(1)).await;
        Ok(HttpResponseOk(*non_send_type))
    }
}

enum MyImpl {}

// This should not produce errors about items being missing.
impl MyApi for MyImpl {
    type Context = ();
}

fn main() {
    // These items should be generated and accessible.
    my_api_mod::api_description::<MyImpl>();
    my_api_mod::stub_api_description();
}
