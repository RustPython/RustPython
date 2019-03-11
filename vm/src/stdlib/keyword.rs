/*
 * Testing if a string is a keyword.
 */

extern crate rustpython_parser;
use self::rustpython_parser::lexer;
use crate::obj::objstr;
use crate::pyobject::{PyContext, PyFuncArgs, PyObjectRef, PyResult, TypeProtocol};
use crate::VirtualMachine;

fn keyword_iskeyword(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(s, Some(vm.ctx.str_type()))]);
    let s = objstr::get_value(s);
    let keywords = lexer::get_keywords();
    let value = keywords.contains_key(&s);
    let value = vm.ctx.new_bool(value);
    Ok(value)
}

pub fn make_module(ctx: &PyContext) -> PyObjectRef {
    let keyword_kwlist = ctx.new_list(
        lexer::get_keywords()
            .keys()
            .map(|k| ctx.new_str(k.to_string()))
            .collect(),
    );

    py_module!(ctx, "keyword", {
        "iskeyword" => ctx.new_rustfunc(keyword_iskeyword),
        "kwlist" => keyword_kwlist
    })
}
