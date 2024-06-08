// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

// Test for a trait with where clauses.

#[dropshot::server]
trait MyTrait
where
    usize: std::fmt::Debug,
{
    type Context;
}

enum MyImpl {}

// This should not produce errors about the trait or the context type being
// missing.
impl MyTrait for MyImpl {
    type Context = ();
}

fn main() {}
