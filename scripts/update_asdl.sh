#!/bin/bash
set -e

cd "$(dirname "$(dirname "$0")")"

python compiler/ast/asdl_rs.py -D compiler/ast/src/ast_gen.rs -M vm/src/stdlib/ast/gen.rs compiler/ast/Python.asdl
rustfmt compiler/ast/src/ast_gen.rs vm/src/stdlib/ast/gen.rs
