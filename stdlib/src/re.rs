pub(crate) use re::make_module;

#[pymodule]
mod re {
    /*
    * Regular expressions.
    *
    * This module fits the python re interface onto the rust regular expression
    * system.
    */
    use crate::{
        builtins::{PyInt, PyIntRef, PyStr, PyStrRef, PyTypeRef},
        function::{IntoPyObject, OptionalArg, PosArgs},
        PyClassImpl, PyObjectRef, PyResult, PyValue, StaticType, TryFromObject, VirtualMachine,
    };
    use num_traits::Signed;
    use regex::bytes::{Captures, Regex, RegexBuilder};
    use std::fmt;
    use std::ops::Range;

    #[pyattr]
    #[pyclass(module = "re", name = "Pattern")]
    #[derive(Debug, PyValue)]
    struct PyPattern {
        regex: Regex,
        pattern: String,
    }

    #[pyattr]
    const IGNORECASE: usize = 2;
    #[pyattr]
    const LOCALE: usize = 4;
    #[pyattr]
    const MULTILINE: usize = 8;
    #[pyattr]
    const DOTALL: usize = 16;
    #[pyattr]
    const UNICODE: usize = 32;
    #[pyattr]
    const VERBOSE: usize = 64;
    #[pyattr]
    const DEBUG: usize = 128;
    #[pyattr]
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

    /// Inner data for a match object.
    #[pyattr]
    #[pyclass(module = "re", name = "Match")]
    #[derive(PyValue)]
    struct PyMatch {
        haystack: PyStrRef,
        captures: Vec<Option<Range<usize>>>,
    }

    impl fmt::Debug for PyMatch {
        fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
            write!(f, "Match()")
        }
    }

    // type PyPatternRef = PyRef<PyPattern>;
    // type PyMatchRef = PyRef<PyMatch>;

    #[pyfunction]
    fn match(
        pattern: PyStrRef,
        string: PyStrRef,
        flags: OptionalArg<usize>,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyMatch>> {
        let flags = extract_flags(flags);
        let regex = make_regex(vm, pattern.as_str(), flags)?;
        Ok(do_match(&regex, string))
    }

    #[pyfunction]
    fn search(
        pattern: PyStrRef,
        string: PyStrRef,
        flags: OptionalArg<usize>,
        vm: &VirtualMachine,
    ) -> PyResult<Option<PyMatch>> {
        let flags = extract_flags(flags);
        let regex = make_regex(vm, pattern.as_str(), flags)?;
        Ok(do_search(&regex, string))
    }

    #[pyfunction]
    fn sub(
        pattern: PyStrRef,
        repl: PyStrRef,
        string: PyStrRef,
        count: OptionalArg<usize>,
        flags: OptionalArg<usize>,
        vm: &VirtualMachine,
    ) -> PyResult<String> {
        let flags = extract_flags(flags);
        let regex = make_regex(vm, pattern.as_str(), flags)?;
        let limit = count.unwrap_or(0);
        Ok(do_sub(&regex, repl, string, limit))
    }

    #[pyfunction]
    fn findall(
        pattern: PyStrRef,
        string: PyStrRef,
        flags: OptionalArg<usize>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let flags = extract_flags(flags);
        let regex = make_regex(vm, pattern.as_str(), flags)?;
        do_findall(vm, &regex, string)
    }

    #[pyfunction]
    fn split(
        pattern: PyStrRef,
        string: PyStrRef,
        maxsplit: OptionalArg<PyIntRef>,
        flags: OptionalArg<usize>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let flags = extract_flags(flags);
        let regex = make_regex(vm, pattern.as_str(), flags)?;
        do_split(vm, &regex, string, maxsplit.into_option())
    }

    fn do_sub(pattern: &PyPattern, repl: PyStrRef, search_text: PyStrRef, limit: usize) -> String {
        let out = pattern.regex.replacen(
            search_text.as_str().as_bytes(),
            limit,
            repl.as_str().as_bytes(),
        );
        String::from_utf8_lossy(&out).into_owned()
    }

    fn do_match(pattern: &PyPattern, search_text: PyStrRef) -> Option<PyMatch> {
        // I really wish there was a better way to do this; I don't think there is
        let mut regex_text = r"\A".to_owned();
        regex_text.push_str(pattern.regex.as_str());
        let regex = Regex::new(&regex_text).unwrap();
        regex
            .captures(search_text.as_str().as_bytes())
            .map(|captures| create_match(search_text.clone(), captures))
    }

    fn do_search(regex: &PyPattern, search_text: PyStrRef) -> Option<PyMatch> {
        regex
            .regex
            .captures(search_text.as_str().as_bytes())
            .map(|captures| create_match(search_text.clone(), captures))
    }

    fn do_findall(vm: &VirtualMachine, pattern: &PyPattern, search_text: PyStrRef) -> PyResult {
        let out = pattern
            .regex
            .captures_iter(search_text.as_str().as_bytes())
            .map(|captures| match captures.len() {
                1 => {
                    let full = captures.get(0).unwrap().as_bytes();
                    let full = String::from_utf8_lossy(full).into_owned();
                    vm.ctx.new_utf8_str(full)
                }
                2 => {
                    let capture = captures.get(1).unwrap().as_bytes();
                    let capture = String::from_utf8_lossy(capture).into_owned();
                    vm.ctx.new_utf8_str(capture)
                }
                _ => {
                    let out = captures
                        .iter()
                        .skip(1)
                        .map(|m| {
                            let s = m
                                .map(|m| String::from_utf8_lossy(m.as_bytes()).into_owned())
                                .unwrap_or_default();
                            vm.ctx.new_utf8_str(s)
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
            .map_or(false, |i| i.as_str().is_negative())
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
                vm.unwrap_or_none(
                    v.map(|v| vm.ctx.new_utf8_str(String::from_utf8_lossy(v).into_owned())),
                )
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

    #[pyfunction]
    fn compile(
        pattern: PyStrRef,
        flags: OptionalArg<usize>,
        vm: &VirtualMachine,
    ) -> PyResult<PyPattern> {
        let flags = extract_flags(flags);
        make_regex(vm, pattern.as_str(), flags)
    }

    #[pyfunction]
    fn escape(pattern: PyStrRef) -> String {
        regex::escape(pattern.as_str())
    }

    #[pyfunction]
    fn purge(_vm: &VirtualMachine) {}

    #[pyimpl]
    impl PyPattern {
        #[pymethod(name = "match")]
        fn match_(&self, text: PyStrRef) -> Option<PyMatch> {
            do_match(self, text)
        }

        #[pymethod]
        fn search(&self, text: PyStrRef) -> Option<PyMatch> {
            do_search(self, text)
        }

        #[pymethod]
        fn sub(&self, repl: PyStrRef, text: PyStrRef, vm: &VirtualMachine) -> PyResult {
            let replaced_text = self
                .regex
                .replace_all(text.as_str().as_bytes(), repl.as_str().as_bytes());
            let replaced_text = String::from_utf8_lossy(&replaced_text).into_owned();
            Ok(vm.ctx.new_utf8_str(replaced_text))
        }

        #[pymethod]
        fn subn(&self, repl: PyStrRef, text: PyStrRef, vm: &VirtualMachine) -> PyResult {
            self.sub(repl, text, vm)
        }

        #[pyproperty]
        fn pattern(&self, vm: &VirtualMachine) -> PyResult {
            Ok(vm.ctx.new_utf8_str(self.pattern.clone()))
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
            self.haystack.as_str()[bounds].to_owned()
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
        fn group(&self, groups: PosArgs, vm: &VirtualMachine) -> PyResult {
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
}
