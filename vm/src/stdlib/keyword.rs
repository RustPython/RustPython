/*
 * Testing if a string is a keyword.
 */

use rustpython_parser::lexer;

use crate::obj::objstr::PyStringRef;
use crate::pyobject::{PyObjectRef, PyResult};
use crate::vm::VirtualMachine;

fn keyword_iskeyword(s: PyStringRef, vm: &VirtualMachine) -> PyResult {
    let keywords = lexer::get_keywords();
    let value = keywords.contains_key(s.as_str());
    let value = vm.ctx.new_bool(value);
    Ok(value)
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    let keyword_kwlist = ctx.new_list(
        lexer::get_keywords()
            .keys()
            .map(|k| ctx.new_str(k.to_owned()))
            .collect(),
    );

    py_module!(vm, "keyword", {
        "iskeyword" => ctx.new_function(keyword_iskeyword),
        "kwlist" => keyword_kwlist
    })
}
