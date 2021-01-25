#!/bin/bash
set -e

cd "$(dirname "$(dirname "$0")")"

python ast/asdl_rs.py -D ast/src/ast_gen.rs -M vm/src/stdlib/ast/gen.rs ast/Python.asdl

cargo fmt
