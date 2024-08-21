// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

// Test for a trait with an associated constant that has an endpoint annotation.
// (Endpoint annotations can only live on functions.)

#[dropshot::api_description]
trait MyApi {
    type Context;
    #[endpoint { method = GET }]
    const MY_CONSTANT: u32;

    #[channel { protocol = WEBSOCKETS }]
    const MY_CONSTANT2: u32;
}

enum MyImpl {}

// This should not produce errors about the trait or any of the items within
// being missing.
impl MyApi for MyImpl {
    type Context = ();
    const MY_CONSTANT: u32 = 42;
    const MY_CONSTANT2: u32 = 84;
}

fn main() {
    // These items should be generated and accessible.
    my_api_mod::api_description::<MyImpl>();
    my_api_mod::stub_api_description();
}
