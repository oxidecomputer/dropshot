// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

// Test for a trait with a type parameter.

#[dropshot::server]
trait MyServer<T> {
    type Context;
}

enum MyImpl {}

// This should not produce errors about the trait or the context type being
// missing.
impl<T> MyServer<T> for MyImpl {
    type Context = ();
}

fn main() {
    // These items will NOT be present because of the invalid trait, and will
    // cause errors to be generated.
    my_server::api_description::<MyImpl>();
    my_server::stub_api_description();
}
