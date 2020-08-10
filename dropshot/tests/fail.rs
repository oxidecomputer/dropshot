// Copyright 2020 Oxide Computer Company

#[test]
fn fail() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/fail/*.rs");
}
