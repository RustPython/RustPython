/*
 * Regular expressions.
 *
 * This module fits the python re interface onto the rust regular expression
 * system.
 */
use regex::{Match, Regex, RegexBuilder};

use std::fmt;

use crate::function::{Args, OptionalArg};
use crate::obj::objint::PyIntRef;
use crate::obj::objstr::PyStringRef;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{PyClassImpl, PyObjectRef, PyResult, PyValue};
use crate::vm::VirtualMachine;
use num_traits::ToPrimitive;

// #[derive(Debug)]
#[pyclass(name = "Pattern")]
struct PyPattern {
    regex: Regex,
    pattern: String,
}

impl fmt::Debug for PyPattern {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Pattern()")
    }
}

const IGNORECASE: usize = 2;
const LOCALE: usize = 4;
const MULTILINE: usize = 8;
const DOTALL: usize = 16;
const UNICODE: usize = 32;
const VERBOSE: usize = 64;
const DEBUG: usize = 128;
const ASCII: usize = 256;

#[derive(Default)]
struct PyRegexFlags {
    ignorecase: bool,
    #[allow(unused)]
    locale: bool,
    multiline: bool,
    dotall: bool,
    unicode: bool,
    verbose: bool,
    #[allow(unused)]
    debug: bool,
    ascii: bool,
}

impl PyRegexFlags {
    fn from_int(bits: usize) -> Self {
        // TODO: detect unknown flag bits.
        PyRegexFlags {
            ignorecase: (bits & IGNORECASE) != 0,
            locale: (bits & LOCALE) != 0,
            multiline: (bits & MULTILINE) != 0,
            dotall: (bits & DOTALL) != 0,
            unicode: (bits & UNICODE) != 0,
            verbose: (bits & VERBOSE) != 0,
            debug: (bits & DEBUG) != 0,
            ascii: (bits & ASCII) != 0,
        }
    }
}

impl PyValue for PyPattern {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("re", "Pattern")
    }
}

/// Inner data for a match object.
#[pyclass(name = "Match")]
struct PyMatch {
    start: usize,
    end: usize,
    // m: Match<'t>,
}

impl fmt::Debug for PyMatch {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Match()")
    }
}

impl PyValue for PyMatch {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("re", "Match")
    }
}

// type PyPatternRef = PyRef<PyPattern>;
// type PyMatchRef = PyRef<PyMatch>;

fn re_match(
    pattern: PyStringRef,
    string: PyStringRef,
    flags: OptionalArg<PyIntRef>,
    vm: &VirtualMachine,
) -> PyResult {
    let flags = extract_flags(flags);
    let regex = make_regex(vm, &pattern.value, flags)?;
    do_match(vm, &regex, &string.value)
}

fn re_search(
    pattern: PyStringRef,
    string: PyStringRef,
    flags: OptionalArg<PyIntRef>,
    vm: &VirtualMachine,
) -> PyResult {
    let flags = extract_flags(flags);
    let regex = make_regex(vm, &pattern.value, flags)?;
    do_search(vm, &regex, &string.value)
}

fn do_match(vm: &VirtualMachine, regex: &PyPattern, search_text: &str) -> PyResult {
    // TODO: implement match!
    do_search(vm, regex, search_text)
}

fn do_search(vm: &VirtualMachine, regex: &PyPattern, search_text: &str) -> PyResult {
    match regex.regex.find(search_text) {
        None => Ok(vm.get_none()),
        Some(result) => create_match(vm, &result),
    }
}

fn make_regex(vm: &VirtualMachine, pattern: &str, flags: PyRegexFlags) -> PyResult<PyPattern> {
    let unicode = if flags.unicode && flags.ascii {
        return Err(vm.new_value_error("ASCII and UNICODE flags are incompatible".to_string()));
    } else {
        !flags.ascii
    };
    let r = RegexBuilder::new(pattern)
        .case_insensitive(flags.ignorecase)
        .multi_line(flags.multiline)
        .dot_matches_new_line(flags.dotall)
        .ignore_whitespace(flags.verbose)
        .unicode(unicode)
        .build()
        .map_err(|err| vm.new_value_error(format!("Error in regex: {:?}", err)))?;
    Ok(PyPattern {
        regex: r,
        pattern: pattern.to_string(),
    })
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

fn extract_flags(flags: OptionalArg<PyIntRef>) -> PyRegexFlags {
    match flags {
        OptionalArg::Present(flags) => {
            PyRegexFlags::from_int(flags.as_bigint().to_usize().unwrap())
        }
        OptionalArg::Missing => Default::default(),
    }
}

fn re_compile(
    pattern: PyStringRef,
    flags: OptionalArg<PyIntRef>,
    vm: &VirtualMachine,
) -> PyResult<PyPattern> {
    let flags = extract_flags(flags);
    make_regex(vm, &pattern.value, flags)
}

fn re_escape(pattern: PyStringRef, _vm: &VirtualMachine) -> String {
    regex::escape(&pattern.value)
}

fn re_purge(_vm: &VirtualMachine) {}

#[pyimpl]
impl PyPattern {
    #[pymethod(name = "match")]
    fn match_(&self, text: PyStringRef, vm: &VirtualMachine) -> PyResult {
        do_match(vm, self, &text.value)
    }

    #[pymethod(name = "search")]
    fn search(&self, text: PyStringRef, vm: &VirtualMachine) -> PyResult {
        do_search(vm, self, &text.value)
    }

    #[pymethod(name = "sub")]
    fn sub(&self, repl: PyStringRef, text: PyStringRef, vm: &VirtualMachine) -> PyResult {
        // let replacer: &Replacer = ;

        let replaced_text: String = self
            .regex
            .replace_all(&text.value, { repl.value.as_str() })
            .into_owned();
        Ok(vm.ctx.new_str(replaced_text))
    }

    #[pymethod(name = "subn")]
    fn subn(&self, repl: PyStringRef, text: PyStringRef, vm: &VirtualMachine) -> PyResult {
        self.sub(repl, text, vm)
    }

    #[pyproperty(name = "pattern")]
    fn pattern(&self, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.new_str(self.pattern.clone()))
    }
}

#[pyimpl]
impl PyMatch {
    #[pymethod(name = "start")]
    fn start(&self, _group: OptionalArg<PyObjectRef>, _vm: &VirtualMachine) -> usize {
        self.start
    }

    #[pymethod(name = "end")]
    fn end(&self, _group: OptionalArg<PyObjectRef>, _vm: &VirtualMachine) -> usize {
        self.end
    }

    #[pymethod(name = "group")]
    fn group(&self, _groups: Args, _vm: &VirtualMachine) -> usize {
        /*
        let groups = groups.into_iter().collect();
        if groups.len() == 1 {
        } else {
        }
        */
        // println!("{:?}", groups);
        self.start
    }
}

/// Create the python `re` module with all its members.
pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    let match_type = PyMatch::make_class(ctx);
    let pattern_type = PyPattern::make_class(ctx);

    py_module!(vm, "re", {
        "compile" => ctx.new_rustfunc(re_compile),
        "escape" => ctx.new_rustfunc(re_escape),
        "purge" => ctx.new_rustfunc(re_purge),
        "Match" => match_type,
        "match" => ctx.new_rustfunc(re_match),
        "Pattern" => pattern_type,
        "search" => ctx.new_rustfunc(re_search),
        "IGNORECASE" => ctx.new_int(IGNORECASE),
        "I" => ctx.new_int(IGNORECASE),
        "LOCALE" => ctx.new_int(LOCALE),
        "MULTILINE" => ctx.new_int(MULTILINE),
        "M" => ctx.new_int(MULTILINE),
        "DOTALL" => ctx.new_int(DOTALL),
        "S" => ctx.new_int(DOTALL),
        "UNICODE" => ctx.new_int(UNICODE),
        "VERBOSE" => ctx.new_int(VERBOSE),
        "X" => ctx.new_int(VERBOSE),
        "DEBUG" => ctx.new_int(DEBUG),
        "ASCII" => ctx.new_int(ASCII),
    })
}
