pub(crate) use _sre::make_module;

#[pymodule]
mod _sre {
    use crate::{
        atomic_func,
        builtins::{
            PyCallableIterator, PyDictRef, PyGenericAlias, PyInt, PyList, PyStr, PyStrRef, PyTuple,
            PyTupleRef, PyTypeRef,
        },
        common::{ascii, hash::PyHash},
        convert::ToPyObject,
        function::{ArgCallable, OptionalArg, PosArgs, PyComparisonValue},
        protocol::{PyBuffer, PyMappingMethods},
        stdlib::sys,
        types::{AsMapping, Comparable, Hashable},
        PyObject, PyObjectRef, PyPayload, PyRef, PyResult, TryFromBorrowedObject, TryFromObject,
        VirtualMachine,
    };
    use core::str;
    use crossbeam_utils::atomic::AtomicCell;
    use itertools::Itertools;
    use num_traits::ToPrimitive;
    use sre_engine::{
        constants::SreFlag,
        engine::{lower_ascii, lower_unicode, upper_unicode, Request, SearchIter, State, StrDrive},
    };

    #[pyattr]
    pub use sre_engine::{constants::SRE_MAGIC as MAGIC, CODESIZE, MAXGROUPS, MAXREPEAT};

    #[pyfunction]
    fn getcodesize() -> usize {
        CODESIZE
    }
    #[pyfunction]
    fn ascii_iscased(ch: i32) -> bool {
        (ch >= b'a' as i32 && ch <= b'z' as i32) || (ch >= b'A' as i32 && ch <= b'Z' as i32)
    }
    #[pyfunction]
    fn unicode_iscased(ch: i32) -> bool {
        let ch = ch as u32;
        ch != lower_unicode(ch) || ch != upper_unicode(ch)
    }
    #[pyfunction]
    fn ascii_tolower(ch: i32) -> i32 {
        lower_ascii(ch as u32) as i32
    }
    #[pyfunction]
    fn unicode_tolower(ch: i32) -> i32 {
        lower_unicode(ch as u32) as i32
    }

    trait SreStr: StrDrive {
        fn slice(&self, start: usize, end: usize, vm: &VirtualMachine) -> PyObjectRef;

        fn create_request(self, pattern: &Pattern, start: usize, end: usize) -> Request<Self> {
            Request::new(self, start, end, &pattern.code, false)
        }
    }

    impl SreStr for &[u8] {
        fn slice(&self, start: usize, end: usize, vm: &VirtualMachine) -> PyObjectRef {
            vm.ctx
                .new_bytes(self.iter().take(end).skip(start).cloned().collect())
                .into()
        }
    }

    impl SreStr for &str {
        fn slice(&self, start: usize, end: usize, vm: &VirtualMachine) -> PyObjectRef {
            vm.ctx
                .new_str(self.chars().take(end).skip(start).collect::<String>())
                .into()
        }
    }

    #[pyfunction]
    fn compile(
        pattern: PyObjectRef,
        flags: u16,
        code: PyObjectRef,
        groups: usize,
        groupindex: PyDictRef,
        indexgroup: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<Pattern> {
        // FIXME:
        // pattern could only be None if called by re.Scanner
        // re.Scanner has no official API and in CPython's implement
        // isbytes will be hanging (-1)
        // here is just a hack to let re.Scanner works only with str not bytes
        let isbytes = !vm.is_none(&pattern) && !pattern.payload_is::<PyStr>();
        let code = code.try_to_value(vm)?;
        Ok(Pattern {
            pattern,
            flags: SreFlag::from_bits_truncate(flags),
            code,
            groups,
            groupindex,
            indexgroup: indexgroup.try_to_value(vm)?,
            isbytes,
        })
    }

    #[derive(FromArgs)]
    struct StringArgs {
        string: PyObjectRef,
        #[pyarg(any, default = "0")]
        pos: usize,
        #[pyarg(any, default = "sys::MAXSIZE as usize")]
        endpos: usize,
    }

    #[derive(FromArgs)]
    struct SubArgs {
        // repl: Either<ArgCallable, PyStrRef>,
        repl: PyObjectRef,
        string: PyObjectRef,
        #[pyarg(any, default = "0")]
        count: usize,
    }

    #[derive(FromArgs)]
    struct SplitArgs {
        string: PyObjectRef,
        #[pyarg(any, default = "0")]
        maxsplit: isize,
    }

    #[pyattr]
    #[pyclass(name = "Pattern")]
    #[derive(Debug, PyPayload)]
    pub(crate) struct Pattern {
        pub pattern: PyObjectRef,
        pub flags: SreFlag,
        pub code: Vec<u32>,
        pub groups: usize,
        pub groupindex: PyDictRef,
        pub indexgroup: Vec<Option<PyStrRef>>,
        pub isbytes: bool,
    }

    macro_rules! with_sre_str {
        ($pattern:expr, $string:expr, $vm:expr, $f:expr) => {
            if $pattern.isbytes {
                Pattern::with_bytes($string, $vm, $f)
            } else {
                Pattern::with_str($string, $vm, $f)
            }
        };
    }

    #[pyclass(with(Hashable, Comparable))]
    impl Pattern {
        fn with_str<F, R>(string: &PyObject, vm: &VirtualMachine, f: F) -> PyResult<R>
        where
            F: FnOnce(&str) -> PyResult<R>,
        {
            string
                .payload::<PyStr>()
                .ok_or_else(|| vm.new_type_error("expected string".to_owned()))
                .and_then(|x| f(x.as_str()))
        }

        fn with_bytes<F, R>(string: &PyObject, vm: &VirtualMachine, f: F) -> PyResult<R>
        where
            F: FnOnce(&[u8]) -> PyResult<R>,
        {
            PyBuffer::try_from_borrowed_object(vm, string).and_then(|x| x.contiguous_or_collect(f))
        }

        #[pymethod(name = "match")]
        fn pymatch(
            zelf: PyRef<Pattern>,
            string_args: StringArgs,
            vm: &VirtualMachine,
        ) -> PyResult<Option<PyRef<Match>>> {
            let StringArgs {
                string,
                pos,
                endpos,
            } = string_args;
            with_sre_str!(zelf, &string.clone(), vm, |x| {
                let req = x.create_request(&zelf, pos, endpos);
                let mut state = State::default();
                state.pymatch(req);
                Ok(state
                    .has_matched
                    .then(|| Match::new(&state, zelf.clone(), string).into_ref(vm)))
            })
        }

        #[pymethod]
        fn fullmatch(
            zelf: PyRef<Pattern>,
            string_args: StringArgs,
            vm: &VirtualMachine,
        ) -> PyResult<Option<PyRef<Match>>> {
            with_sre_str!(zelf, &string_args.string.clone(), vm, |x| {
                let mut req = x.create_request(&zelf, string_args.pos, string_args.endpos);
                req.match_all = true;
                let mut state = State::default();
                state.pymatch(req);
                Ok(state
                    .has_matched
                    .then(|| Match::new(&state, zelf.clone(), string_args.string).into_ref(vm)))
            })
        }

        #[pymethod]
        fn search(
            zelf: PyRef<Pattern>,
            string_args: StringArgs,
            vm: &VirtualMachine,
        ) -> PyResult<Option<PyRef<Match>>> {
            with_sre_str!(zelf, &string_args.string.clone(), vm, |x| {
                let req = x.create_request(&zelf, string_args.pos, string_args.endpos);
                let mut state = State::default();
                state.search(req);
                Ok(state
                    .has_matched
                    .then(|| Match::new(&state, zelf.clone(), string_args.string).into_ref(vm)))
            })
        }

        #[pymethod]
        fn findall(
            zelf: PyRef<Pattern>,
            string_args: StringArgs,
            vm: &VirtualMachine,
        ) -> PyResult<Vec<PyObjectRef>> {
            with_sre_str!(zelf, &string_args.string, vm, |s| {
                let req = s.create_request(&zelf, string_args.pos, string_args.endpos);
                let state = State::default();
                let mut matchlist: Vec<PyObjectRef> = Vec::new();
                let mut iter = SearchIter { req, state };

                while iter.next().is_some() {
                    let m = Match::new(&iter.state, zelf.clone(), string_args.string.clone());

                    let item = if zelf.groups == 0 || zelf.groups == 1 {
                        m.get_slice(zelf.groups, s, vm)
                            .unwrap_or_else(|| vm.ctx.none())
                    } else {
                        m.groups(OptionalArg::Present(vm.ctx.new_str(ascii!("")).into()), vm)?
                            .into()
                    };

                    matchlist.push(item);
                }

                Ok(matchlist)
            })
        }

        #[pymethod]
        fn finditer(
            zelf: PyRef<Pattern>,
            string_args: StringArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyCallableIterator> {
            let scanner = SreScanner {
                pattern: zelf,
                string: string_args.string,
                start: AtomicCell::new(string_args.pos),
                end: string_args.endpos,
                must_advance: AtomicCell::new(false),
            }
            .into_ref(vm);
            let search = vm.get_str_method(scanner.into(), "search").unwrap()?;
            let search = ArgCallable::try_from_object(vm, search)?;
            let iterator = PyCallableIterator::new(search, vm.ctx.none());
            Ok(iterator)
        }

        #[pymethod]
        fn scanner(
            zelf: PyRef<Pattern>,
            string_args: StringArgs,
            vm: &VirtualMachine,
        ) -> PyRef<SreScanner> {
            SreScanner {
                pattern: zelf,
                string: string_args.string,
                start: AtomicCell::new(string_args.pos),
                end: string_args.endpos,
                must_advance: AtomicCell::new(false),
            }
            .into_ref(vm)
        }

        #[pymethod]
        fn sub(zelf: PyRef<Pattern>, sub_args: SubArgs, vm: &VirtualMachine) -> PyResult {
            Self::subx(zelf, sub_args, false, vm)
        }
        #[pymethod]
        fn subn(zelf: PyRef<Pattern>, sub_args: SubArgs, vm: &VirtualMachine) -> PyResult {
            Self::subx(zelf, sub_args, true, vm)
        }

        #[pymethod]
        fn split(
            zelf: PyRef<Pattern>,
            split_args: SplitArgs,
            vm: &VirtualMachine,
        ) -> PyResult<Vec<PyObjectRef>> {
            with_sre_str!(zelf, &split_args.string, vm, |s| {
                let req = s.create_request(&zelf, 0, usize::MAX);
                let state = State::default();
                let mut splitlist: Vec<PyObjectRef> = Vec::new();
                let mut iter = SearchIter { req, state };
                let mut n = 0;
                let mut last = 0;

                while (split_args.maxsplit == 0 || n < split_args.maxsplit) && iter.next().is_some()
                {
                    /* get segment before this match */
                    splitlist.push(s.slice(last, iter.state.start, vm));

                    let m = Match::new(&iter.state, zelf.clone(), split_args.string.clone());

                    // add groups (if any)
                    for i in 1..=zelf.groups {
                        splitlist.push(m.get_slice(i, s, vm).unwrap_or_else(|| vm.ctx.none()));
                    }

                    n += 1;
                    last = iter.state.string_position;
                }

                // get segment following last match (even if empty)
                splitlist.push(req.string.slice(last, s.count(), vm));

                Ok(splitlist)
            })
        }

        #[pymethod(magic)]
        fn repr(&self, vm: &VirtualMachine) -> PyResult<String> {
            let flag_names = [
                ("re.TEMPLATE", SreFlag::TEMPLATE),
                ("re.IGNORECASE", SreFlag::IGNORECASE),
                ("re.LOCALE", SreFlag::LOCALE),
                ("re.MULTILINE", SreFlag::MULTILINE),
                ("re.DOTALL", SreFlag::DOTALL),
                ("re.UNICODE", SreFlag::UNICODE),
                ("re.VERBOSE", SreFlag::VERBOSE),
                ("re.DEBUG", SreFlag::DEBUG),
                ("re.ASCII", SreFlag::ASCII),
            ];

            /* Omit re.UNICODE for valid string patterns. */
            let mut flags = self.flags;
            if !self.isbytes
                && (flags & (SreFlag::LOCALE | SreFlag::UNICODE | SreFlag::ASCII))
                    == SreFlag::UNICODE
            {
                flags &= !SreFlag::UNICODE;
            }

            let flags = flag_names
                .iter()
                .filter(|(_, flag)| flags.contains(*flag))
                .map(|(name, _)| name)
                .join("|");

            let pattern = self.pattern.repr(vm)?;
            let truncated: String;
            let s = if pattern.char_len() > 200 {
                truncated = pattern.as_str().chars().take(200).collect();
                &truncated
            } else {
                pattern.as_str()
            };

            if flags.is_empty() {
                Ok(format!("re.compile({})", s))
            } else {
                Ok(format!("re.compile({}, {})", s, flags))
            }
        }

        #[pygetset]
        fn flags(&self) -> u16 {
            self.flags.bits()
        }
        #[pygetset]
        fn groupindex(&self) -> PyDictRef {
            self.groupindex.clone()
        }
        #[pygetset]
        fn groups(&self) -> usize {
            self.groups
        }
        #[pygetset]
        fn pattern(&self) -> PyObjectRef {
            self.pattern.clone()
        }

        fn subx(
            zelf: PyRef<Pattern>,
            sub_args: SubArgs,
            subn: bool,
            vm: &VirtualMachine,
        ) -> PyResult {
            let SubArgs {
                repl,
                string,
                count,
            } = sub_args;

            let (is_callable, filter) = if vm.is_callable(&repl) {
                (true, repl)
            } else {
                let is_template = if zelf.isbytes {
                    Self::with_bytes(&repl, vm, |x| Ok(x.contains(&b'\\')))
                } else {
                    Self::with_str(&repl, vm, |x| Ok(x.contains('\\')))
                }?;
                if is_template {
                    let re = vm.import("re", None, 0)?;
                    let func = re.get_attr("_subx", vm)?;
                    let filter = vm.invoke(&func, (zelf.clone(), repl))?;
                    (vm.is_callable(&filter), filter)
                } else {
                    (false, repl)
                }
            };

            with_sre_str!(zelf, &string, vm, |s| {
                let req = s.create_request(&zelf, 0, usize::MAX);
                let state = State::default();
                let mut sublist: Vec<PyObjectRef> = Vec::new();
                let mut iter = SearchIter { req, state };
                let mut n = 0;
                let mut last_pos = 0;

                while (count == 0 || n < count) && iter.next().is_some() {
                    if last_pos < iter.state.start {
                        /* get segment before this match */
                        sublist.push(s.slice(last_pos, iter.state.start, vm));
                    }

                    if is_callable {
                        let m = Match::new(&iter.state, zelf.clone(), string.clone());
                        let ret = vm.invoke(&filter, (m.into_ref(vm),))?;
                        sublist.push(ret);
                    } else {
                        sublist.push(filter.clone());
                    }

                    last_pos = iter.state.string_position;
                    n += 1;
                }

                /* get segment following last match */
                sublist.push(s.slice(last_pos, iter.req.end, vm));

                let list = PyList::from(sublist).into_pyobject(vm);

                let join_type: PyObjectRef = if zelf.isbytes {
                    vm.ctx.new_bytes(vec![]).into()
                } else {
                    vm.ctx.new_str(ascii!("")).into()
                };
                let ret = vm.call_method(&join_type, "join", (list,))?;

                Ok(if subn { (ret, n).to_pyobject(vm) } else { ret })
            })
        }

        #[pyclassmethod(magic)]
        fn class_getitem(cls: PyTypeRef, args: PyObjectRef, vm: &VirtualMachine) -> PyGenericAlias {
            PyGenericAlias::new(cls, args, vm)
        }
    }

    impl Hashable for Pattern {
        fn hash(zelf: &crate::Py<Self>, vm: &VirtualMachine) -> PyResult<PyHash> {
            let hash = zelf.pattern.hash(vm)?;
            let (_, code, _) = unsafe { zelf.code.align_to::<u8>() };
            let hash = hash ^ vm.state.hash_secret.hash_bytes(code);
            let hash = hash ^ (zelf.flags.bits() as PyHash);
            let hash = hash ^ (zelf.isbytes as i64);
            Ok(hash)
        }
    }

    impl Comparable for Pattern {
        fn cmp(
            zelf: &crate::Py<Self>,
            other: &PyObject,
            op: crate::types::PyComparisonOp,
            vm: &VirtualMachine,
        ) -> PyResult<PyComparisonValue> {
            if let Some(res) = op.identical_optimization(zelf, other) {
                return Ok(res.into());
            }
            op.eq_only(|| {
                if let Some(other) = other.downcast_ref::<Pattern>() {
                    Ok(PyComparisonValue::Implemented(
                        zelf.flags == other.flags
                            && zelf.isbytes == other.isbytes
                            && zelf.code == other.code
                            && vm.bool_eq(&zelf.pattern, &other.pattern)?,
                    ))
                } else {
                    Ok(PyComparisonValue::NotImplemented)
                }
            })
        }
    }

    #[pyattr]
    #[pyclass(name = "Match")]
    #[derive(Debug, PyPayload)]
    pub(crate) struct Match {
        string: PyObjectRef,
        pattern: PyRef<Pattern>,
        pos: usize,
        endpos: usize,
        lastindex: isize,
        regs: Vec<(isize, isize)>,
    }

    #[pyclass(with(AsMapping))]
    impl Match {
        pub(crate) fn new<S: StrDrive>(
            state: &State<S>,
            pattern: PyRef<Pattern>,
            string: PyObjectRef,
        ) -> Self {
            let mut regs = vec![(state.start as isize, state.string_position as isize)];
            for group in 0..pattern.groups {
                let mark_index = 2 * group;
                if mark_index + 1 < state.marks.len() {
                    let start = state.marks[mark_index];
                    let end = state.marks[mark_index + 1];
                    if start.is_some() && end.is_some() {
                        regs.push((start.unpack() as isize, end.unpack() as isize));
                        continue;
                    }
                }
                regs.push((-1, -1));
            }
            Self {
                string,
                pattern,
                pos: state.start,
                endpos: state.string_position,
                lastindex: state.marks.last_index(),
                regs,
            }
        }

        #[pygetset]
        fn pos(&self) -> usize {
            self.pos
        }
        #[pygetset]
        fn endpos(&self) -> usize {
            self.endpos
        }
        #[pygetset]
        fn lastindex(&self) -> Option<isize> {
            if self.lastindex >= 0 {
                Some(self.lastindex)
            } else {
                None
            }
        }
        #[pygetset]
        fn lastgroup(&self) -> Option<PyStrRef> {
            self.lastindex
                .to_usize()
                .and_then(|i| self.pattern.indexgroup.get(i).cloned().flatten())
        }
        #[pygetset]
        fn re(&self) -> PyRef<Pattern> {
            self.pattern.clone()
        }
        #[pygetset]
        fn string(&self) -> PyObjectRef {
            self.string.clone()
        }
        #[pygetset]
        fn regs(&self, vm: &VirtualMachine) -> PyTupleRef {
            PyTuple::new_ref(
                self.regs.iter().map(|&x| x.to_pyobject(vm)).collect(),
                &vm.ctx,
            )
        }

        #[pymethod]
        fn start(&self, group: OptionalArg<PyObjectRef>, vm: &VirtualMachine) -> PyResult<isize> {
            self.span(group, vm).map(|x| x.0)
        }
        #[pymethod]
        fn end(&self, group: OptionalArg<PyObjectRef>, vm: &VirtualMachine) -> PyResult<isize> {
            self.span(group, vm).map(|x| x.1)
        }
        #[pymethod]
        fn span(
            &self,
            group: OptionalArg<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<(isize, isize)> {
            let index = group.map_or(Ok(0), |group| {
                self.get_index(group, vm)
                    .ok_or_else(|| vm.new_index_error("no such group".to_owned()))
            })?;
            Ok(self.regs[index])
        }

        #[pymethod]
        fn expand(zelf: PyRef<Match>, template: PyStrRef, vm: &VirtualMachine) -> PyResult {
            let re = vm.import("re", None, 0)?;
            let func = re.get_attr("_expand", vm)?;
            vm.invoke(&func, (zelf.pattern.clone(), zelf, template))
        }

        #[pymethod]
        fn group(&self, args: PosArgs<PyObjectRef>, vm: &VirtualMachine) -> PyResult {
            with_sre_str!(self.pattern, &self.string, vm, |str_drive| {
                let args = args.into_vec();
                if args.is_empty() {
                    return Ok(self.get_slice(0, str_drive, vm).unwrap().to_pyobject(vm));
                }
                let mut v: Vec<PyObjectRef> = args
                    .into_iter()
                    .map(|x| {
                        self.get_index(x, vm)
                            .ok_or_else(|| vm.new_index_error("no such group".to_owned()))
                            .map(|index| {
                                self.get_slice(index, str_drive, vm)
                                    .map(|x| x.to_pyobject(vm))
                                    .unwrap_or_else(|| vm.ctx.none())
                            })
                    })
                    .try_collect()?;
                if v.len() == 1 {
                    Ok(v.pop().unwrap())
                } else {
                    Ok(vm.ctx.new_tuple(v).into())
                }
            })
        }

        #[pymethod(magic)]
        fn getitem(
            &self,
            group: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<Option<PyObjectRef>> {
            with_sre_str!(self.pattern, &self.string, vm, |str_drive| {
                let i = self
                    .get_index(group, vm)
                    .ok_or_else(|| vm.new_index_error("no such group".to_owned()))?;
                Ok(self.get_slice(i, str_drive, vm))
            })
        }

        #[pymethod]
        fn groups(
            &self,
            default: OptionalArg<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<PyTupleRef> {
            let default = default.unwrap_or_else(|| vm.ctx.none());

            with_sre_str!(self.pattern, &self.string, vm, |str_drive| {
                let v: Vec<PyObjectRef> = (1..self.regs.len())
                    .map(|i| {
                        self.get_slice(i, str_drive, vm)
                            .map(|s| s.to_pyobject(vm))
                            .unwrap_or_else(|| default.clone())
                    })
                    .collect();
                Ok(PyTuple::new_ref(v, &vm.ctx))
            })
        }

        #[pymethod]
        fn groupdict(
            &self,
            default: OptionalArg<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<PyDictRef> {
            let default = default.unwrap_or_else(|| vm.ctx.none());

            with_sre_str!(self.pattern, &self.string, vm, |str_drive| {
                let dict = vm.ctx.new_dict();

                for (key, index) in self.pattern.groupindex.clone() {
                    let value = self
                        .get_index(index, vm)
                        .and_then(|x| self.get_slice(x, str_drive, vm))
                        .map(|x| x.to_pyobject(vm))
                        .unwrap_or_else(|| default.clone());
                    dict.set_item(&*key, value, vm)?;
                }
                Ok(dict)
            })
        }

        #[pymethod(magic)]
        fn repr(&self, vm: &VirtualMachine) -> PyResult<String> {
            with_sre_str!(self.pattern, &self.string, vm, |str_drive| {
                Ok(format!(
                    "<re.Match object; span=({}, {}), match={}>",
                    self.regs[0].0,
                    self.regs[0].1,
                    self.get_slice(0, str_drive, vm).unwrap().repr(vm)?
                ))
            })
        }

        fn get_index(&self, group: PyObjectRef, vm: &VirtualMachine) -> Option<usize> {
            let i = if let Ok(i) = group.try_index(vm) {
                i
            } else {
                self.pattern
                    .groupindex
                    .get_item_opt(&*group, vm)
                    .ok()??
                    .downcast::<PyInt>()
                    .ok()?
            };
            let i = i.as_bigint().to_isize()?;
            if i >= 0 && i as usize <= self.pattern.groups {
                Some(i as usize)
            } else {
                None
            }
        }

        fn get_slice<S: SreStr>(
            &self,
            index: usize,
            str_drive: S,
            vm: &VirtualMachine,
        ) -> Option<PyObjectRef> {
            let (start, end) = self.regs[index];
            if start < 0 || end < 0 {
                return None;
            }
            Some(str_drive.slice(start as usize, end as usize, vm))
        }

        #[pyclassmethod(magic)]
        fn class_getitem(cls: PyTypeRef, args: PyObjectRef, vm: &VirtualMachine) -> PyGenericAlias {
            PyGenericAlias::new(cls, args, vm)
        }
    }

    impl AsMapping for Match {
        fn as_mapping() -> &'static PyMappingMethods {
            static AS_MAPPING: PyMappingMethods = PyMappingMethods {
                subscript: atomic_func!(|mapping, needle, vm| {
                    Match::mapping_downcast(mapping)
                        .getitem(needle.to_owned(), vm)
                        .map(|x| x.to_pyobject(vm))
                }),
                ..PyMappingMethods::NOT_IMPLEMENTED
            };
            &AS_MAPPING
        }
    }

    #[pyattr]
    #[pyclass(name = "SRE_Scanner")]
    #[derive(Debug, PyPayload)]
    struct SreScanner {
        pattern: PyRef<Pattern>,
        string: PyObjectRef,
        start: AtomicCell<usize>,
        end: usize,
        must_advance: AtomicCell<bool>,
    }

    #[pyclass]
    impl SreScanner {
        #[pygetset]
        fn pattern(&self) -> PyRef<Pattern> {
            self.pattern.clone()
        }

        #[pymethod(name = "match")]
        fn pymatch(&self, vm: &VirtualMachine) -> PyResult<Option<PyRef<Match>>> {
            with_sre_str!(self.pattern, &self.string.clone(), vm, |s| {
                let mut req = s.create_request(&self.pattern, self.start.load(), self.end);
                let mut state = State::default();
                req.must_advance = self.must_advance.load();
                state.pymatch(req);

                self.must_advance
                    .store(state.string_position == state.start);
                self.start.store(state.string_position);

                Ok(state.has_matched.then(|| {
                    Match::new(&state, self.pattern.clone(), self.string.clone()).into_ref(vm)
                }))
            })
        }

        #[pymethod]
        fn search(&self, vm: &VirtualMachine) -> PyResult<Option<PyRef<Match>>> {
            if self.start.load() > self.end {
                return Ok(None);
            }
            with_sre_str!(self.pattern, &self.string.clone(), vm, |s| {
                let mut req = s.create_request(&self.pattern, self.start.load(), self.end);
                let mut state = State::default();
                req.must_advance = self.must_advance.load();

                state.search(req);

                self.must_advance
                    .store(state.string_position == state.start);
                self.start.store(state.string_position);

                Ok(state.has_matched.then(|| {
                    Match::new(&state, self.pattern.clone(), self.string.clone()).into_ref(vm)
                }))
            })
        }
    }
}
