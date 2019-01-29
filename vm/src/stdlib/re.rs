/*
 * Regular expressions.
 *
 * This module fits the python re interface onto the rust regular expression
 * system.
 */

extern crate regex;
use self::regex::Regex;

use super::super::obj::{objstr, objtype};
use super::super::pyobject::{PyContext, PyFuncArgs, PyObjectRef, PyResult, TypeProtocol};
use super::super::VirtualMachine;

fn re_match(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    // TODO:
    error!("TODO: implement match");
    re_search(vm, args)
}

fn re_search(vm: &mut VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [
            (pattern, Some(vm.ctx.str_type())),
            (string, Some(vm.ctx.str_type()))
        ]
    );

    let pattern_str = objstr::get_value(&pattern);
    let search_text = objstr::get_value(&string);

    match Regex::new(&pattern_str) {
        Ok(regex) => {
            // Now use regex to search:
            match regex.find(&search_text) {
                None => Ok(vm.get_none()),
                Some(result) => {
                    // Return match object:
                    // TODO: implement match object
                    // TODO: how to refer to match object defined in this
                    // module?
                    Ok(vm.ctx.new_str(result.as_str().to_string()))
                }
            }
        }
        Err(err) => Err(vm.new_value_error(format!("Error in regex: {:?}", err))),
    }
}

pub fn mk_module(ctx: &PyContext) -> PyObjectRef {
    let py_mod = ctx.new_module("re", ctx.new_scope(None));

    let match_type = ctx.new_class("Match", ctx.object());
    ctx.set_attr(&py_mod, "Match", match_type);

    ctx.set_attr(&py_mod, "match", ctx.new_rustfunc(re_match));
    ctx.set_attr(&py_mod, "search", ctx.new_rustfunc(re_search));

    py_mod
}
