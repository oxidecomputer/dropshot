// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

use dropshot::EndpointTagPolicy;
use dropshot::HttpError;
use dropshot::HttpResponseUpdatedNoContent;
use dropshot::RequestContext;

// Test invalid tag configuration: external_docs without url field.

#[dropshot::api_description {
    tag_config = {
        policy = EndpointTagPolicy::AtLeastOne,
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
    // These items are not generated because the api_description macro's fields
    // are invalid.
    my_api_mod::api_description::<MyImpl>();
    my_api_mod::stub_api_description();
}
