#!/bin/bash
set -e

cd "$(dirname "$(dirname "$0")")"

python ast/asdl_rs.py -D ast/bootstrap/ast_def.rs -M ast/bootstrap/ast_mod.rs ast/Python.asdl
rustfmt ast/bootstrap/ast_def.rs ast/bootstrap/ast_mod.rs
