// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

// Test for a trait with a type parameter.

#[dropshot::server]
trait MyTrait<T> {
    type Context;
}

enum MyImpl {}

// This should not produce errors about the trait or the context type being
// missing.
impl<T> MyTrait<T> for MyImpl {
    type Context = ();
}

fn main() {}
