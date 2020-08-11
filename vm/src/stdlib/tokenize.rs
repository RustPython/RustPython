/*
 * python tokenize module.
 */

use std::iter::FromIterator;

use crate::obj::objstr::PyStringRef;
use crate::pyobject::{BorrowValue, PyObjectRef, PyResult};
use crate::vm::VirtualMachine;
use rustpython_parser::lexer;

fn tokenize_tokenize(s: PyStringRef, vm: &VirtualMachine) -> PyResult {
    let source = s.borrow_value();

    // TODO: implement generator when the time has come.
    let lexer1 = lexer::make_tokenizer(source);

    let tokens = lexer1.map(|st| vm.ctx.new_str(format!("{:?}", st.unwrap().1)));
    let tokens = Vec::from_iter(tokens);
    Ok(vm.ctx.new_list(tokens))
}

// TODO: create main function when called with -m

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    py_module!(vm, "tokenize", {
        "tokenize" => ctx.new_function(tokenize_tokenize)
    })
}
