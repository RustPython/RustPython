/*
 * Regular expressions.
 *
 * This module fits the python re interface onto the rust regular expression
 * system.
 */

use std::path::PathBuf;

use regex::{Match, Regex};

use crate::function::PyFuncArgs;
use crate::import;
use crate::obj::objstr;
use crate::pyobject::{PyContext, PyObject, PyObjectRef, PyResult, PyValue, TypeProtocol};
use crate::vm::VirtualMachine;

impl PyValue for Regex {
    fn class(vm: &VirtualMachine) -> Vec<PyObjectRef> {
        vec![vm.class("re", "Pattern")]
    }
}

/// Create the python `re` module with all its members.
pub fn make_module(ctx: &PyContext) -> PyObjectRef {
    let match_type = py_class!(ctx, "Match", ctx.object(), {
        "start" => ctx.new_rustfunc(match_start),
        "end" => ctx.new_rustfunc(match_end)
    });

    let pattern_type = py_class!(ctx, "Pattern", ctx.object(), {
        "match" => ctx.new_rustfunc(pattern_match),
        "search" => ctx.new_rustfunc(pattern_search)
    });

    py_module!(ctx, "re", {
        "compile" => ctx.new_rustfunc(re_compile),
        "Match" => match_type,
        "match" => ctx.new_rustfunc(re_match),
        "Pattern" => pattern_type,
        "search" => ctx.new_rustfunc(re_search)
    })
}

/// Implement re.match
/// See also:
/// https://docs.python.org/3/library/re.html#re.match
fn re_match(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [
            (pattern, Some(vm.ctx.str_type())),
            (string, Some(vm.ctx.str_type()))
        ]
    );
    let regex = make_regex(vm, pattern)?;
    let search_text = objstr::get_value(string);

    do_match(vm, &regex, search_text)
}

/// Implement re.search
/// See also:
/// https://docs.python.org/3/library/re.html#re.search
fn re_search(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [
            (pattern, Some(vm.ctx.str_type())),
            (string, Some(vm.ctx.str_type()))
        ]
    );

    // let pattern_str = objstr::get_value(&pattern);
    let regex = make_regex(vm, pattern)?;
    let search_text = objstr::get_value(string);

    do_search(vm, &regex, search_text)
}

fn do_match(vm: &VirtualMachine, regex: &Regex, search_text: String) -> PyResult {
    // TODO: implement match!
    do_search(vm, regex, search_text)
}

fn do_search(vm: &VirtualMachine, regex: &Regex, search_text: String) -> PyResult {
    match regex.find(&search_text) {
        None => Ok(vm.get_none()),
        Some(result) => create_match(vm, &result),
    }
}

fn make_regex(vm: &VirtualMachine, pattern: &PyObjectRef) -> PyResult<Regex> {
    let pattern_str = objstr::get_value(pattern);

    match Regex::new(&pattern_str) {
        Ok(regex) => Ok(regex),
        Err(err) => Err(vm.new_value_error(format!("Error in regex: {:?}", err))),
    }
}

/// Inner data for a match object.
#[derive(Debug)]
struct PyMatch {
    start: usize,
    end: usize,
}

impl PyValue for PyMatch {
    fn class(vm: &VirtualMachine) -> Vec<PyObjectRef> {
        vec![vm.class("re", "Match")]
    }
}

/// Take a found regular expression and convert it to proper match object.
fn create_match(vm: &VirtualMachine, match_value: &Match) -> PyResult {
    // Return match object:
    // TODO: implement match object
    // TODO: how to refer to match object defined in this
    let module = import::import_module(vm, PathBuf::default(), "re").unwrap();
    let match_class = vm.ctx.get_attr(&module, "Match").unwrap();

    // let mo = vm.invoke(match_class, PyFuncArgs::default())?;
    // let txt = vm.ctx.new_str(result.as_str().to_string());
    // vm.ctx.set_attr(&mo, "str", txt);
    let match_value = PyMatch {
        start: match_value.start(),
        end: match_value.end(),
    };

    Ok(PyObject::new(match_value, match_class.clone()))
}

/// Compile a regular expression into a Pattern object.
/// See also:
/// https://docs.python.org/3/library/re.html#re.compile
fn re_compile(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(pattern, Some(vm.ctx.str_type()))] // TODO: flags=0
    );

    let regex = make_regex(vm, pattern)?;
    // TODO: retrieval of this module is akward:
    let module = import::import_module(vm, PathBuf::default(), "re").unwrap();
    let pattern_class = vm.ctx.get_attr(&module, "Pattern").unwrap();

    Ok(PyObject::new(regex, pattern_class.clone()))
}

fn pattern_match(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, None), (text, Some(vm.ctx.str_type()))]
    );

    let regex = get_regex(zelf);
    let search_text = objstr::get_value(text);
    do_match(vm, &regex, search_text)
}

fn pattern_search(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(zelf, None), (text, Some(vm.ctx.str_type()))]
    );

    let regex = get_regex(zelf);
    let search_text = objstr::get_value(text);
    do_search(vm, &regex, search_text)
}

/// Returns start of match
/// see: https://docs.python.org/3/library/re.html#re.Match.start
fn match_start(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, None)]);
    // TODO: implement groups
    let m = get_match(zelf);
    Ok(vm.new_int(m.start))
}

fn match_end(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(vm, args, required = [(zelf, None)]);
    // TODO: implement groups
    let m = get_match(zelf);
    Ok(vm.new_int(m.end))
}

/// Retrieve inner rust regex from python object:
fn get_regex(obj: &PyObjectRef) -> &Regex {
    // TODO: Regex shouldn't be stored in payload directly, create newtype wrapper
    if let Some(regex) = obj.payload::<Regex>() {
        return regex;
    }
    panic!("Inner error getting regex {:?}", obj);
}

/// Retrieve inner rust match from python object:
fn get_match(obj: &PyObjectRef) -> &PyMatch {
    if let Some(value) = obj.payload::<PyMatch>() {
        return value;
    }
    panic!("Inner error getting match {:?}", obj);
}
