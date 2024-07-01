// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

// Test for a trait with a lifetime parameter.

#[dropshot::api_description]
trait MyApi<'a> {
    type Context;
}

enum MyImpl {}

// This should not produce errors about the trait or the context type being
// missing.
impl<'a> MyApi<'a> for MyImpl {
    type Context = ();
}

fn main() {
    // These items will NOT be present because of the invalid trait, and will
    // cause errors to be generated.
    my_api::api_description::<MyImpl>();
    my_api::stub_api_description();
}
