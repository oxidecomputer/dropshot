// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

// Test for a trait with a lifetime parameter.

#[dropshot::server]
trait MyServer<'a> {
    type Context;
}

enum MyImpl {}

// This should not produce errors about the trait or the context type being
// missing.
impl<'a> MyServer<'a> for MyImpl {
    type Context = ();
}

fn main() {
    // These items will NOT be present because of the invalid trait, and will
    // cause errors to be generated.
    my_server::api_description::<MyImpl>();
    my_server::stub_api_description();
}
