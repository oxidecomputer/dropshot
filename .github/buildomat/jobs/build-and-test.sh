#!/bin/bash
#:
#: name = "build-and-test / illumos"
#: variety = "basic"
#: target = "helios"
#: rust_toolchain = "stable"
#:

set -o errexit
set -o pipefail
set -o xtrace

cargo --version
rustc --version

banner build
ptime -m cargo build --all-features --locked --all-targets --verbose

banner test
RUST_BACKTRACE=1 ptime -m cargo test --all-features --locked --verbose
