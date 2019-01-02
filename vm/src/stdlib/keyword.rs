/*
 * Testing if a string is a keyword.
 */

extern crate rustpython_parser;
use self::rustpython_parser::lexer;
use super::super::obj::{objstr, objtype};
use super::super::pyobject::{PyContext, PyFuncArgs, PyObjectRef, PyResult, TypeProtocol};
use super::super::VirtualMachine;

fn keyword_iskeyword(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(s, Some(vm.ctx.str_type()))]);
    let s = objstr::get_value(s);
    let keywords = lexer::get_keywords();
    let value = keywords.contains_key(&s);
    let value = vm.ctx.new_bool(value);
    Ok(value)
}

pub fn mk_module(ctx: &PyContext) -> PyObjectRef {
    let keyword_kwlist = ctx.new_list(
        lexer::get_keywords()
            .keys()
            .map(|k| ctx.new_str(k.to_string()))
            .collect(),
    );
    py_item!(ctx, mod keyword {
        fn iskeyword = keyword_iskeyword;
        let kwlist = keyword_kwlist;
    })
}
