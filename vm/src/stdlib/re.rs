/*
 * Regular expressions.
 *
 * This module fits the python re interface onto the rust regular expression
 * system.
 */
use regex::bytes::{Captures, Regex, RegexBuilder};

use std::fmt;

use crate::function::{Args, OptionalArg};
use crate::obj::objint::{PyInt, PyIntRef};
use crate::obj::objstr::{PyString, PyStringRef};
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{PyClassImpl, PyObjectRef, PyResult, PyValue, TryFromObject};
use crate::vm::VirtualMachine;
use num_traits::{Signed, ToPrimitive};

#[pyclass(name = "Pattern")]
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
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("re", "Pattern")
    }
}

/// Inner data for a match object.
#[pyclass(name = "Match")]
struct PyMatch {
    haystack: PyStringRef,
    captures: Vec<Option<(usize, usize)>>,
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
    do_match(vm, &regex, string)
}

fn re_search(
    pattern: PyStringRef,
    string: PyStringRef,
    flags: OptionalArg<PyIntRef>,
    vm: &VirtualMachine,
) -> PyResult {
    let flags = extract_flags(flags);
    let regex = make_regex(vm, &pattern.value, flags)?;
    do_search(vm, &regex, string)
}

fn re_sub(
    pattern: PyStringRef,
    repl: PyStringRef,
    string: PyStringRef,
    count: OptionalArg<usize>,
    flags: OptionalArg<PyIntRef>,
    vm: &VirtualMachine,
) -> PyResult {
    let flags = extract_flags(flags);
    let regex = make_regex(vm, pattern.as_str(), flags)?;
    let limit = count.unwrap_or(0);
    do_sub(vm, &regex, repl, string, limit)
}

fn re_findall(
    pattern: PyStringRef,
    string: PyStringRef,
    flags: OptionalArg<PyIntRef>,
    vm: &VirtualMachine,
) -> PyResult {
    let flags = extract_flags(flags);
    let regex = make_regex(vm, pattern.as_str(), flags)?;
    do_findall(vm, &regex, string)
}

fn re_split(
    pattern: PyStringRef,
    string: PyStringRef,
    maxsplit: OptionalArg<PyIntRef>,
    flags: OptionalArg<PyIntRef>,
    vm: &VirtualMachine,
) -> PyResult {
    let flags = extract_flags(flags);
    let regex = make_regex(vm, pattern.as_str(), flags)?;
    do_split(vm, &regex, string, maxsplit.into_option())
}

fn do_sub(
    vm: &VirtualMachine,
    pattern: &PyPattern,
    repl: PyStringRef,
    search_text: PyStringRef,
    limit: usize,
) -> PyResult {
    let out = pattern.regex.replacen(
        search_text.as_str().as_bytes(),
        limit,
        repl.as_str().as_bytes(),
    );
    let out = String::from_utf8_lossy(&out).into_owned();
    Ok(vm.new_str(out))
}

fn do_match(vm: &VirtualMachine, pattern: &PyPattern, search_text: PyStringRef) -> PyResult {
    // I really wish there was a better way to do this; I don't think there is
    let mut regex = r"\A".to_owned();
    regex.push_str(pattern.regex.as_str());
    let regex = Regex::new(&regex).unwrap();

    match regex.captures(search_text.as_str().as_bytes()) {
        None => Ok(vm.get_none()),
        Some(captures) => Ok(create_match(vm, search_text.clone(), captures)),
    }
}

fn do_search(vm: &VirtualMachine, regex: &PyPattern, search_text: PyStringRef) -> PyResult {
    match regex.regex.captures(search_text.as_str().as_bytes()) {
        None => Ok(vm.get_none()),
        Some(captures) => Ok(create_match(vm, search_text.clone(), captures)),
    }
}

fn do_findall(vm: &VirtualMachine, pattern: &PyPattern, search_text: PyStringRef) -> PyResult {
    let out = pattern
        .regex
        .captures_iter(search_text.as_str().as_bytes())
        .map(|captures| match captures.len() {
            1 => {
                let full = captures.get(0).unwrap().as_bytes();
                let full = String::from_utf8_lossy(full).into_owned();
                vm.new_str(full)
            }
            2 => {
                let capture = captures.get(1).unwrap().as_bytes();
                let capture = String::from_utf8_lossy(capture).into_owned();
                vm.new_str(capture)
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
    search_text: PyStringRef,
    maxsplit: Option<PyIntRef>,
) -> PyResult {
    if maxsplit
        .as_ref()
        .map_or(false, |i| i.as_bigint().is_negative())
    {
        return Ok(vm.ctx.new_list(vec![search_text.into_object()]));
    }
    let maxsplit = maxsplit
        .map(|i| usize::try_from_object(vm, i.into_object()))
        .transpose()?
        .unwrap_or(0);
    let text = search_text.as_str().as_bytes();
    // essentially Regex::split, but it outputs captures as well
    let mut output = Vec::new();
    let mut last = 0;
    let mut n = 0;
    for captures in pattern.regex.captures_iter(text) {
        let full = captures.get(0).unwrap();
        let matched = &text[last..full.start()];
        last = full.end();
        output.push(Some(matched));
        for m in captures.iter().skip(1) {
            output.push(m.map(|m| m.as_bytes()));
        }
        n += 1;
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
            v.map(|v| vm.new_str(String::from_utf8_lossy(v).into_owned()))
                .unwrap_or_else(|| vm.get_none())
        })
        .collect();
    Ok(vm.ctx.new_list(split))
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
        .map_err(|err| match err {
            regex::Error::Syntax(s) => vm.new_value_error(format!("Error in regex: {}", s)),
            err => vm.new_value_error(format!("Error in regex: {:?}", err)),
        })?;
    Ok(PyPattern {
        regex: r,
        pattern: pattern.to_string(),
    })
}

/// Take a found regular expression and convert it to proper match object.
fn create_match(vm: &VirtualMachine, haystack: PyStringRef, captures: Captures) -> PyObjectRef {
    let captures = captures
        .iter()
        .map(|opt| opt.map(|m| (m.start(), m.end())))
        .collect();
    PyMatch { haystack, captures }.into_ref(vm).into_object()
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
        do_match(vm, self, text)
    }

    #[pymethod(name = "search")]
    fn search(&self, text: PyStringRef, vm: &VirtualMachine) -> PyResult {
        do_search(vm, self, text)
    }

    #[pymethod(name = "sub")]
    fn sub(&self, repl: PyStringRef, text: PyStringRef, vm: &VirtualMachine) -> PyResult {
        let replaced_text = self
            .regex
            .replace_all(text.value.as_bytes(), repl.as_str().as_bytes());
        let replaced_text = String::from_utf8_lossy(&replaced_text).into_owned();
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

    #[pymethod]
    fn split(
        &self,
        search_text: PyStringRef,
        maxsplit: OptionalArg<PyIntRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        do_split(vm, self, search_text, maxsplit.into_option())
    }

    #[pymethod]
    fn findall(&self, search_text: PyStringRef, vm: &VirtualMachine) -> PyResult {
        do_findall(vm, self, search_text)
    }
}

#[pyimpl]
impl PyMatch {
    #[pymethod]
    fn start(&self, group: OptionalArg, vm: &VirtualMachine) -> PyResult {
        let group = group.unwrap_or_else(|| vm.new_int(0));
        let start = self
            .get_bounds(group, vm)?
            .map_or_else(|| vm.new_int(-1), |(start, _)| vm.new_int(start));
        Ok(start)
    }

    #[pymethod]
    fn end(&self, group: OptionalArg, vm: &VirtualMachine) -> PyResult {
        let group = group.unwrap_or_else(|| vm.new_int(0));
        let end = self
            .get_bounds(group, vm)?
            .map_or_else(|| vm.new_int(-1), |(_, end)| vm.new_int(end));
        Ok(end)
    }

    fn subgroup(&self, bounds: (usize, usize), vm: &VirtualMachine) -> PyObjectRef {
        vm.new_str(self.haystack.as_str()[bounds.0..bounds.1].to_owned())
    }

    fn get_bounds(&self, id: PyObjectRef, vm: &VirtualMachine) -> PyResult<Option<(usize, usize)>> {
        match_class!(id,
            i @ PyInt => {
                let i = usize::try_from_object(vm,i.into_object())?;
                match self.captures.get(i) {
                    None => Err(vm.new_index_error("No such group".to_owned())),
                    Some(None) => Ok(None),
                    Some(Some(bounds)) => Ok(Some(*bounds)),
                }
            },
            _s @ PyString => unimplemented!(),
            _ => Err(vm.new_index_error("No such group".to_owned())),
        )
    }

    fn get_group(&self, id: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let bounds = self.get_bounds(id, vm)?;
        let group = match bounds {
            Some(bounds) => self.subgroup(bounds, vm),
            None => vm.get_none(),
        };
        Ok(group)
    }

    #[pymethod]
    fn group(&self, groups: Args, vm: &VirtualMachine) -> PyResult {
        let mut groups = groups.into_vec();
        match groups.len() {
            0 => Ok(self.subgroup(self.captures[0].unwrap(), vm)),
            1 => self.get_group(groups.pop().unwrap(), vm),
            len => {
                let mut output = Vec::with_capacity(len);
                for id in groups {
                    output.push(self.get_group(id, vm)?);
                }
                Ok(vm.ctx.new_tuple(output))
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
                capture
                    .map(|bounds| self.subgroup(bounds, vm))
                    .or_else(|| default.clone())
                    .unwrap_or_else(|| vm.get_none())
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

    py_module!(vm, "re", {
        "compile" => ctx.new_rustfunc(re_compile),
        "escape" => ctx.new_rustfunc(re_escape),
        "purge" => ctx.new_rustfunc(re_purge),
        "Match" => match_type,
        "match" => ctx.new_rustfunc(re_match),
        "Pattern" => pattern_type,
        "search" => ctx.new_rustfunc(re_search),
        "sub" => ctx.new_rustfunc(re_sub),
        "findall" => ctx.new_rustfunc(re_findall),
        "split" => ctx.new_rustfunc(re_split),
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
