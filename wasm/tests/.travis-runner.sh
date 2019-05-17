#!/bin/sh -eux
# This script is intended to be run in Travis from the root of the repository

# Install Rust
curl -sSf https://build.travis-ci.org/files/rustup-init.sh | sh -s -- --default-toolchain=$TRAVIS_RUST_VERSION -y
export PATH=$HOME/.cargo/bin:$PATH

# install wasm-pack
if [ ! -f $HOME/.cargo/bin/wasm-pack ]; then
    curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh
fi

# install geckodriver
wget https://github.com/mozilla/geckodriver/releases/download/v0.24.0/geckodriver-v0.24.0-linux32.tar.gz
mkdir geckodriver
tar -xzf geckodriver-v0.24.0-linux32.tar.gz -C geckodriver
export PATH=$PATH:$PWD/geckodriver

# Install pipenv
pip install pipenv
(cd wasm/tests; pipenv install)

(cd wasm/demo; npm install; npm run build; npm run ci)
