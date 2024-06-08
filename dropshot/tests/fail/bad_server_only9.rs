// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

// Test for a trait with a lifetime parameter.

#[dropshot::server]
trait MyTrait<'a> {
    type Context;
}

enum MyImpl {}

// This should not produce errors about the trait or the context type being
// missing.
impl<'a> MyTrait<'a> for MyImpl {
    type Context = ();
}

fn main() {}
