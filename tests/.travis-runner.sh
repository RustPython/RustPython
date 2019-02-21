#!/bin/sh -eux
# This script is intended to be run in Travis from the root of the repository

# Install Rust
curl -sSf https://build.travis-ci.org/files/rustup-init.sh | sh -s -- --default-toolchain=$TRAVIS_RUST_VERSION -y
export PATH=$HOME/.cargo/bin:$PATH

# Install pipenv
pip install pipenv
(cd tests; pipenv install)

# Build outside of the test runner
if [ $CODE_COVERAGE = "true" ]
then
    find . -name '*.gcda' -delete

    export CARGO_INCREMENTAL=0
    export RUSTFLAGS="-Zprofile -Ccodegen-units=1 -Cinline-threshold=0 -Clink-dead-code -Coverflow-checks=off -Zno-landing-pads"

    cargo build --verbose
else
    cargo build --verbose --release
fi

# Run the tests
(cd tests; pipenv run pytest)

if [ $CODE_COVERAGE = "true" ]
then
    cargo test --verbose --all
    zip -0 ccov.zip `find . \( -name "rustpython*.gc*" \) -print`

    # Install grcov
    curl -L https://github.com/mozilla/grcov/releases/download/v0.4.1/grcov-linux-x86_64.tar.bz2 | tar jxf -

    ./grcov ccov.zip -s . -t lcov --llvm --branch --ignore-not-existing --ignore-dir "/*" -p "x" > lcov.info

    # Install codecov.io reporter
    curl -s https://codecov.io/bash -o codecov.sh
    bash codecov.sh -f lcov.info
fi
