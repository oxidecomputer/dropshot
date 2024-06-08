// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

// Test for an unsafe trait.

#[dropshot::server]
unsafe trait MyTrait {
    type Context;
}

enum MyImpl {}

// This should not produce errors about the trait or the context type being
// missing.
unsafe impl MyTrait for MyImpl {
    type Context = ();
}

fn main() {}
