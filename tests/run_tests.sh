#!/usr/bin/env bash

pushd tests

function finish {
   popd
}
trap finish EXIT

pipenv install
cargo build --verbose $@
pipenv run pytest -n auto
