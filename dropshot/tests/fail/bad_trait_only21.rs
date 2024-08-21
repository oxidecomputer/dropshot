// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::HttpError;
use dropshot::HttpResponseUpdatedNoContent;
use dropshot::RequestContext;

// Test invalid tag configuration: incorrect endpoint tag policy.
#[dropshot::api_description {
    tag_config = {
        policy = 2 + 2,
        tags = {},
    }
}]
trait MyApi {
    type Context;
}

enum MyImpl {}

// This should not produce errors about the trait or any of the items within
// being missing.
impl MyApi for MyImpl {
    type Context = ();
}

fn main() {
    // These items are generated because the type mismatch is a semantic/code
    // gen error, not a parsing error.
    my_api_mod::api_description::<MyImpl>();
    my_api_mod::stub_api_description();
}
