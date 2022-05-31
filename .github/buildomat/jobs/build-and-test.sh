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

rustc --version

banner build
ptime -m cargo build --tests --verbose

banner test
ptime -m cargo test --verbose
