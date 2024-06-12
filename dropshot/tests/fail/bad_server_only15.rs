// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

// Test for a trait with an associated type that has an endpoint annotation.
// (Endpoint annotations can only live on functions.)

#[dropshot::server]
trait MyServer {
    type Context;

    #[endpoint { method = GET }]
    type MyType;

    #[channel { protocol = WEBSOCKETS }]
    type MyType2;
}

enum MyImpl {}

// This should not produce errors about the trait or any of the items within
// being missing.
impl MyServer for MyImpl {
    type Context = ();
    type MyType = ();
    type MyType2 = ();
}

fn main() {
    // These items should be generated and accessible.
    my_server::api_description::<MyImpl>();
    my_server::stub_api_description();
}
