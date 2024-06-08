// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

// Test for a context type with a lifetime parameter.

#[dropshot::server]
trait MyTrait {
    type Context<'a>;
}

enum MyImpl {}

// This should not produce errors about the trait or the context type being
// missing.
impl MyTrait for MyImpl {
    type Context<'a> = ();
}

fn main() {}
