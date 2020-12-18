/*
 * Regular expressions.
 *
 * This module fits the python re interface onto the rust regular expression
 * system.
 */
use num_traits::Signed;
use regex::bytes::{Captures, Regex, RegexBuilder};
use std::fmt;
use std::ops::Range;

use crate::builtins::int::{PyInt, PyIntRef};
use crate::builtins::pystr::{PyStr, PyStrRef};
use crate::builtins::pytype::PyTypeRef;
use crate::function::{Args, OptionalArg};
use crate::pyobject::{
    BorrowValue, IntoPyObject, PyClassImpl, PyObjectRef, PyResult, PyValue, StaticType,
    TryFromObject,
};
use crate::vm::VirtualMachine;

#[pyclass(module = "re", name = "Pattern")]
#[derive(Debug)]
struct PyPattern {
    regex: Regex,
    pattern: String,
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
    fn class(_vm: &VirtualMachine) -> &PyTypeRef {
        Self::static_type()
    }
}

/// Inner data for a match object.
#[pyclass(module = "re", name = "Match")]
struct PyMatch {
    haystack: PyStrRef,
    captures: Vec<Option<Range<usize>>>,
}

impl fmt::Debug for PyMatch {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Match()")
    }
}

impl PyValue for PyMatch {
    fn class(_vm: &VirtualMachine) -> &PyTypeRef {
        Self::static_type()
    }
}

// type PyPatternRef = PyRef<PyPattern>;
// type PyMatchRef = PyRef<PyMatch>;

fn re_match(
    pattern: PyStrRef,
    string: PyStrRef,
    flags: OptionalArg<usize>,
    vm: &VirtualMachine,
) -> PyResult<Option<PyMatch>> {
    let flags = extract_flags(flags);
    let regex = make_regex(vm, pattern.borrow_value(), flags)?;
    Ok(do_match(&regex, string))
}

fn re_search(
    pattern: PyStrRef,
    string: PyStrRef,
    flags: OptionalArg<usize>,
    vm: &VirtualMachine,
) -> PyResult<Option<PyMatch>> {
    let flags = extract_flags(flags);
    let regex = make_regex(vm, pattern.borrow_value(), flags)?;
    Ok(do_search(&regex, string))
}

fn re_sub(
    pattern: PyStrRef,
    repl: PyStrRef,
    string: PyStrRef,
    count: OptionalArg<usize>,
    flags: OptionalArg<usize>,
    vm: &VirtualMachine,
) -> PyResult<String> {
    let flags = extract_flags(flags);
    let regex = make_regex(vm, pattern.borrow_value(), flags)?;
    let limit = count.unwrap_or(0);
    Ok(do_sub(&regex, repl, string, limit))
}

fn re_findall(
    pattern: PyStrRef,
    string: PyStrRef,
    flags: OptionalArg<usize>,
    vm: &VirtualMachine,
) -> PyResult {
    let flags = extract_flags(flags);
    let regex = make_regex(vm, pattern.borrow_value(), flags)?;
    do_findall(vm, &regex, string)
}

fn re_split(
    pattern: PyStrRef,
    string: PyStrRef,
    maxsplit: OptionalArg<PyIntRef>,
    flags: OptionalArg<usize>,
    vm: &VirtualMachine,
) -> PyResult {
    let flags = extract_flags(flags);
    let regex = make_regex(vm, pattern.borrow_value(), flags)?;
    do_split(vm, &regex, string, maxsplit.into_option())
}

fn do_sub(pattern: &PyPattern, repl: PyStrRef, search_text: PyStrRef, limit: usize) -> String {
    let out = pattern.regex.replacen(
        search_text.borrow_value().as_bytes(),
        limit,
        repl.borrow_value().as_bytes(),
    );
    String::from_utf8_lossy(&out).into_owned()
}

fn do_match(pattern: &PyPattern, search_text: PyStrRef) -> Option<PyMatch> {
    // I really wish there was a better way to do this; I don't think there is
    let mut regex_text = r"\A".to_owned();
    regex_text.push_str(pattern.regex.as_str());
    let regex = Regex::new(&regex_text).unwrap();
    regex
        .captures(search_text.borrow_value().as_bytes())
        .map(|captures| create_match(search_text.clone(), captures))
}

fn do_search(regex: &PyPattern, search_text: PyStrRef) -> Option<PyMatch> {
    regex
        .regex
        .captures(search_text.borrow_value().as_bytes())
        .map(|captures| create_match(search_text.clone(), captures))
}

fn do_findall(vm: &VirtualMachine, pattern: &PyPattern, search_text: PyStrRef) -> PyResult {
    let out = pattern
        .regex
        .captures_iter(search_text.borrow_value().as_bytes())
        .map(|captures| match captures.len() {
            1 => {
                let full = captures.get(0).unwrap().as_bytes();
                let full = String::from_utf8_lossy(full).into_owned();
                vm.ctx.new_str(full)
            }
            2 => {
                let capture = captures.get(1).unwrap().as_bytes();
                let capture = String::from_utf8_lossy(capture).into_owned();
                vm.ctx.new_str(capture)
            }
            _ => {
                let out = captures
                    .iter()
                    .skip(1)
                    .map(|m| {
                        let s = m
                            .map(|m| String::from_utf8_lossy(m.as_bytes()).into_owned())
                            .unwrap_or_default();
                        vm.ctx.new_str(s)
                    })
                    .collect();
                vm.ctx.new_tuple(out)
            }
        })
        .collect();
    Ok(vm.ctx.new_list(out))
}

fn do_split(
    vm: &VirtualMachine,
    pattern: &PyPattern,
    search_text: PyStrRef,
    maxsplit: Option<PyIntRef>,
) -> PyResult {
    if maxsplit
        .as_ref()
        .map_or(false, |i| i.borrow_value().is_negative())
    {
        return Ok(vm.ctx.new_list(vec![search_text.into_object()]));
    }
    let maxsplit = maxsplit
        .map(|i| usize::try_from_object(vm, i.into_object()))
        .transpose()?
        .unwrap_or(0);
    let text = search_text.borrow_value().as_bytes();
    // essentially Regex::split, but it outputs captures as well
    let mut output = Vec::new();
    let mut last = 0;
    for (n, captures) in pattern.regex.captures_iter(text).enumerate() {
        let full = captures.get(0).unwrap();
        let matched = &text[last..full.start()];
        last = full.end();
        output.push(Some(matched));
        for m in captures.iter().skip(1) {
            output.push(m.map(|m| m.as_bytes()));
        }
        if maxsplit != 0 && n >= maxsplit {
            break;
        }
    }
    if last < text.len() {
        output.push(Some(&text[last..]));
    }
    let split = output
        .into_iter()
        .map(|v| {
            vm.unwrap_or_none(v.map(|v| vm.ctx.new_str(String::from_utf8_lossy(v).into_owned())))
        })
        .collect();
    Ok(vm.ctx.new_list(split))
}

fn make_regex(vm: &VirtualMachine, pattern: &str, flags: PyRegexFlags) -> PyResult<PyPattern> {
    let unicode = if flags.unicode && flags.ascii {
        return Err(vm.new_value_error("ASCII and UNICODE flags are incompatible".to_owned()));
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
        .map_err(|err| match err {
            regex::Error::Syntax(s) => vm.new_value_error(format!("Error in regex: {}", s)),
            err => vm.new_value_error(format!("Error in regex: {:?}", err)),
        })?;
    Ok(PyPattern {
        regex: r,
        pattern: pattern.to_owned(),
    })
}

/// Take a found regular expression and convert it to proper match object.
fn create_match(haystack: PyStrRef, captures: Captures) -> PyMatch {
    let captures = captures
        .iter()
        .map(|opt| opt.map(|m| m.start()..m.end()))
        .collect();
    PyMatch { haystack, captures }
}

fn extract_flags(flags: OptionalArg<usize>) -> PyRegexFlags {
    match flags {
        OptionalArg::Present(flags) => PyRegexFlags::from_int(flags),
        OptionalArg::Missing => Default::default(),
    }
}

fn re_compile(
    pattern: PyStrRef,
    flags: OptionalArg<usize>,
    vm: &VirtualMachine,
) -> PyResult<PyPattern> {
    let flags = extract_flags(flags);
    make_regex(vm, pattern.borrow_value(), flags)
}

fn re_escape(pattern: PyStrRef) -> String {
    regex::escape(pattern.borrow_value())
}

fn re_purge(_vm: &VirtualMachine) {}

#[pyimpl]
impl PyPattern {
    #[pymethod(name = "match")]
    fn match_(&self, text: PyStrRef) -> Option<PyMatch> {
        do_match(self, text)
    }

    #[pymethod(name = "search")]
    fn search(&self, text: PyStrRef) -> Option<PyMatch> {
        do_search(self, text)
    }

    #[pymethod(name = "sub")]
    fn sub(&self, repl: PyStrRef, text: PyStrRef, vm: &VirtualMachine) -> PyResult {
        let replaced_text = self.regex.replace_all(
            text.borrow_value().as_bytes(),
            repl.borrow_value().as_bytes(),
        );
        let replaced_text = String::from_utf8_lossy(&replaced_text).into_owned();
        Ok(vm.ctx.new_str(replaced_text))
    }

    #[pymethod(name = "subn")]
    fn subn(&self, repl: PyStrRef, text: PyStrRef, vm: &VirtualMachine) -> PyResult {
        self.sub(repl, text, vm)
    }

    #[pyproperty(name = "pattern")]
    fn pattern(&self, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.new_str(self.pattern.clone()))
    }

    #[pymethod]
    fn split(
        &self,
        search_text: PyStrRef,
        maxsplit: OptionalArg<PyIntRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        do_split(vm, self, search_text, maxsplit.into_option())
    }

    #[pymethod]
    fn findall(&self, search_text: PyStrRef, vm: &VirtualMachine) -> PyResult {
        do_findall(vm, self, search_text)
    }
}

#[pyimpl]
impl PyMatch {
    #[pymethod]
    fn start(&self, group: OptionalArg, vm: &VirtualMachine) -> PyResult {
        let group = group.unwrap_or_else(|| vm.ctx.new_int(0));
        let start = self
            .get_bounds(group, vm)?
            .map_or_else(|| vm.ctx.new_int(-1), |r| vm.ctx.new_int(r.start));
        Ok(start)
    }

    #[pymethod]
    fn end(&self, group: OptionalArg, vm: &VirtualMachine) -> PyResult {
        let group = group.unwrap_or_else(|| vm.ctx.new_int(0));
        let end = self
            .get_bounds(group, vm)?
            .map_or_else(|| vm.ctx.new_int(-1), |r| vm.ctx.new_int(r.end));
        Ok(end)
    }

    fn subgroup(&self, bounds: Range<usize>) -> String {
        self.haystack.borrow_value()[bounds].to_owned()
    }

    fn get_bounds(&self, id: PyObjectRef, vm: &VirtualMachine) -> PyResult<Option<Range<usize>>> {
        match_class!(match id {
            i @ PyInt => {
                let i = usize::try_from_object(vm, i.into_object())?;
                let capture = self
                    .captures
                    .get(i)
                    .ok_or_else(|| vm.new_index_error("No such group".to_owned()))?;
                Ok(capture.clone())
            }
            _s @ PyStr => unimplemented!(),
            _ => Err(vm.new_index_error("No such group".to_owned())),
        })
    }

    fn get_group(&self, id: PyObjectRef, vm: &VirtualMachine) -> PyResult<Option<String>> {
        let bounds = self.get_bounds(id, vm)?;
        Ok(bounds.map(|b| self.subgroup(b)))
    }

    #[pymethod]
    fn group(&self, groups: Args, vm: &VirtualMachine) -> PyResult {
        let mut groups = groups.into_vec();
        match groups.len() {
            0 => Ok(self
                .subgroup(self.captures[0].clone().unwrap())
                .into_pyobject(vm)),
            1 => self
                .get_group(groups.pop().unwrap(), vm)
                .map(|g| g.into_pyobject(vm)),
            _ => {
                let output: Result<Vec<_>, _> = groups
                    .into_iter()
                    .map(|id| self.get_group(id, vm).map(|g| g.into_pyobject(vm)))
                    .collect();
                Ok(vm.ctx.new_tuple(output?))
            }
        }
    }

    #[pymethod]
    fn groups(&self, default: OptionalArg, vm: &VirtualMachine) -> PyObjectRef {
        let default = default.into_option();
        let groups = self
            .captures
            .iter()
            .map(|capture| {
                vm.unwrap_or_none(
                    capture
                        .as_ref()
                        .map(|bounds| self.subgroup(bounds.clone()).into_pyobject(vm))
                        .or_else(|| default.clone()),
                )
            })
            .collect();
        vm.ctx.new_tuple(groups)
    }
}

/// Create the python `re` module with all its members.
pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;

    let match_type = PyMatch::make_class(ctx);
    let pattern_type = PyPattern::make_class(ctx);

    py_module!(vm, "regex_crate", {
        "compile" => ctx.new_function("compile", re_compile),
        "escape" => ctx.new_function("escape", re_escape),
        "purge" => ctx.new_function("purge", re_purge),
        "Match" => match_type,
        "match" => ctx.new_function("match", re_match),
        "Pattern" => pattern_type,
        "search" => ctx.new_function("search", re_search),
        "sub" => ctx.new_function("sub", re_sub),
        "findall" => ctx.new_function("findall", re_findall),
        "split" => ctx.new_function("split", re_split),
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
