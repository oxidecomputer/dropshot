// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

// Test for a trait with an associated type that has an endpoint annotation.
// (Endpoint annotations can only live on functions.)

#[dropshot::server]
trait MyTrait {
    type Context;

    #[endpoint { method = GET }]
    type MyType;
}

enum MyImpl {}

// This should not produce errors about the trait or any of the items within
// being missing.
impl MyTrait for MyImpl {
    type Context = ();
    type MyType = ();
}

fn main() {}
