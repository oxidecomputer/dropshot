// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

// Test for a missing custom context type.

#[dropshot::api_description { context = OtherContext }]
trait MyApi {
    type Context;
}

enum MyImpl {}

// This should not produce errors about the trait or items being missing.
impl MyApi for MyImpl {
    type Context = ();
}

fn main() {
    // These items will NOT be present because of the lack of a context type,
    // and will cause errors to be generated.
    my_api_mod::api_description::<MyImpl>();
    my_api_mod::stub_api_description();
}
