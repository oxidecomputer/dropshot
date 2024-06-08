// Copyright 2024 Oxide Computer Company

#![allow(unused_imports)]

// Test for a missing context type.

#[dropshot::server]
trait MyTrait {}

enum MyImpl {}

// This should not produce errors about the trait being missing.
impl MyTrait for MyImpl {}

fn main() {}
