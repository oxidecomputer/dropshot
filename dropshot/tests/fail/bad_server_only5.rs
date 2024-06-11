// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

// Test for a missing custom context type.

#[dropshot::server { context = OtherContext }]
trait MyServer {
    type Context;
}

enum MyImpl {}

// This should not produce errors about the trait or items being missing.
impl MyServer for MyImpl {
    type Context = ();
}

fn main() {
    // These items will NOT be present because of the lack of a context type,
    // and will cause errors to be generated.
    my_server::api_description::<MyImpl>();
    my_server::stub_api_description();
}
