/*
 * Regular expressions.
 *
 * This module fits the python re interface onto the rust regular expression
 * system.
 */
use regex::{Match, Regex};

use crate::obj::objstr::PyStringRef;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{PyContext, PyObjectRef, PyRef, PyResult, PyValue};
use crate::vm::VirtualMachine;

impl PyValue for Regex {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("re", "Pattern")
    }
}

/// Inner data for a match object.
#[derive(Debug)]
struct PyMatch {
    start: usize,
    end: usize,
}

impl PyValue for PyMatch {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("re", "Match")
    }
}

type PyRegexRef = PyRef<Regex>;
type PyMatchRef = PyRef<PyMatch>;

fn re_match(pattern: PyStringRef, string: PyStringRef, vm: &VirtualMachine) -> PyResult {
    let regex = make_regex(vm, &pattern.value)?;
    do_match(vm, &regex, &string.value)
}

fn re_search(pattern: PyStringRef, string: PyStringRef, vm: &VirtualMachine) -> PyResult {
    let regex = make_regex(vm, &pattern.value)?;
    do_search(vm, &regex, &string.value)
}

fn do_match(vm: &VirtualMachine, regex: &Regex, search_text: &str) -> PyResult {
    // TODO: implement match!
    do_search(vm, regex, search_text)
}

fn do_search(vm: &VirtualMachine, regex: &Regex, search_text: &str) -> PyResult {
    match regex.find(search_text) {
        None => Ok(vm.get_none()),
        Some(result) => create_match(vm, &result),
    }
}

fn make_regex(vm: &VirtualMachine, pattern: &str) -> PyResult<Regex> {
    match Regex::new(pattern) {
        Ok(regex) => Ok(regex),
        Err(err) => Err(vm.new_value_error(format!("Error in regex: {:?}", err))),
    }
}

/// Take a found regular expression and convert it to proper match object.
fn create_match(vm: &VirtualMachine, match_value: &Match) -> PyResult {
    // let mo = vm.invoke(match_class, PyFuncArgs::default())?;
    // let txt = vm.ctx.new_str(result.as_str().to_string());
    // vm.ctx.set_attr(&mo, "str", txt);
    Ok(PyMatch {
        start: match_value.start(),
        end: match_value.end(),
    }
    .into_ref(vm)
    .into_object())
}

fn re_compile(pattern: PyStringRef, vm: &VirtualMachine) -> PyResult<Regex> {
    make_regex(vm, &pattern.value)
}

impl PyRegexRef {
    fn match_(self, text: PyStringRef, vm: &VirtualMachine) -> PyResult {
        do_match(vm, &self, &text.value)
    }
    fn search(self, text: PyStringRef, vm: &VirtualMachine) -> PyResult {
        do_search(vm, &self, &text.value)
    }
}

impl PyMatchRef {
    fn start(self, _vm: &VirtualMachine) -> usize {
        self.start
    }
    fn end(self, _vm: &VirtualMachine) -> usize {
        self.end
    }
}

/// Create the python `re` module with all its members.
pub fn make_module(ctx: &PyContext) -> PyObjectRef {
    let match_type = py_class!(ctx, "Match", ctx.object(), {
        "start" => ctx.new_rustfunc(PyMatchRef::start),
        "end" => ctx.new_rustfunc(PyMatchRef::end)
    });

    let pattern_type = py_class!(ctx, "Pattern", ctx.object(), {
        "match" => ctx.new_rustfunc(PyRegexRef::match_),
        "search" => ctx.new_rustfunc(PyRegexRef::search)
    });

    py_module!(ctx, "re", {
        "compile" => ctx.new_rustfunc(re_compile),
        "Match" => match_type,
        "match" => ctx.new_rustfunc(re_match),
        "Pattern" => pattern_type,
        "search" => ctx.new_rustfunc(re_search)
    })
}
