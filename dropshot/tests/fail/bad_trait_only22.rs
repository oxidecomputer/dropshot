// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::HttpError;
use dropshot::HttpResponseUpdatedNoContent;
use dropshot::RequestContext;

// Test invalid tag configuration: external_docs without url field.

#[dropshot::api_description {
    tag_config = {
        policy = at_least_one,
        tags = {
            foo = {
                external_docs = {}
            }
        },
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
    // These items are not generated.
    my_api::api_description::<MyImpl>();
    my_api::stub_api_description();
}
