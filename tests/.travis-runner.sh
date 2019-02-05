#!/bin/sh -eux
# This script is intended to be run in Travis from the root of the repository

# Install Rust
curl -sSf https://build.travis-ci.org/files/rustup-init.sh | sh -s -- --default-toolchain=$TRAVIS_RUST_VERSION -y
export PATH=$HOME/.cargo/bin:$PATH

# Install pipenv
pip install pipenv

RUST_RELEASE=1 ./tests/run_tests.sh --release
