// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

// Test for a missing custom context type.

#[dropshot::server { context = OtherContext }]
trait MyTrait {
    type Context;
}

enum MyImpl {}

// This should not produce errors about the trait or items being missing.
impl MyTrait for MyImpl {
    type Context = ();
}

fn main() {}
