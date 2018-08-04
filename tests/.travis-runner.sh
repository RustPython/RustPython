#!/bin/sh -eux
# This script is intended to be run in Travis from the root of the repository

# Install Rust
curl -sSf https://build.travis-ci.org/files/rustup-init.sh | sh -s -- --default-toolchain=$TRAVIS_RUST_VERSION -y
export PATH=$HOME/.cargo/bin:$PATH

# Install pipenv
pip install pipenv
(cd tests; pipenv install)

# Build outside of the test runner
cargo build --verbose --release

# Run the tests
(cd tests; pipenv run pytest)
