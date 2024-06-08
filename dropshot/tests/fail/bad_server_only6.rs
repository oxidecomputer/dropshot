// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

// Test for a context type with a type parameter.

#[dropshot::server]
trait MyServer {
    type Context<T>;
}

enum MyImpl {}

// This should not produce errors about the trait or the context type being
// missing.
impl MyServer for MyImpl {
    type Context<T> = ();
}

fn main() {
    // These items will NOT be present because of the invalid context type, and
    // will cause errors to be generated.
    my_server::api_description::<MyImpl>();
    my_server::stub_api_description();
}
