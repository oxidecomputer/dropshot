// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

// Test for a trait with where clauses.

#[dropshot::api_description]
trait MyApi
where
    usize: std::fmt::Debug,
{
    type Context;
}

enum MyImpl {}

// This should not produce errors about the trait or the context type being
// missing.
impl MyApi for MyImpl {
    type Context = ();
}

fn main() {
    // These items will NOT be present because of the invalid trait, and will
    // cause errors to be generated.
    my_api_mod::api_description::<MyImpl>();
    my_api_mod::stub_api_description();
}
