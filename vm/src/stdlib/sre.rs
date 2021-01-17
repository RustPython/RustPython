mod constants;
mod interp;

pub(crate) use _sre::make_module;

#[pymodule]
mod _sre {
    use crossbeam_utils::atomic::AtomicCell;
    use itertools::Itertools;
    use num_traits::ToPrimitive;
    use rustpython_common::borrow::BorrowValue;

    use super::constants::SreFlag;
    use super::interp::{lower_ascii, lower_unicode, upper_unicode, State, StrDrive};
    use crate::builtins::list::PyListRef;
    use crate::builtins::memory::try_buffer_from_object;
    use crate::builtins::tuple::PyTupleRef;
    use crate::builtins::{
        PyCallableIterator, PyDictRef, PyInt, PyList, PyStr, PyStrRef, PyTypeRef,
    };
    use crate::function::{Args, OptionalArg};
    use crate::pyobject::{
        Either, IntoPyObject, ItemProtocol, PyCallable, PyObjectRef, PyRef, PyResult, PyValue,
        StaticType, TryFromObject,
    };
    use crate::VirtualMachine;
    use core::str;

    #[pyattr]
    pub const CODESIZE: usize = 4;
    #[pyattr]
    pub use super::constants::SRE_MAGIC as MAGIC;
    #[cfg(target_pointer_width = "32")]
    #[pyattr]
    pub const MAXREPEAT: usize = usize::MAX;
    #[cfg(target_pointer_width = "64")]
    #[pyattr]
    pub const MAXREPEAT: usize = u32::MAX as usize;
    #[cfg(target_pointer_width = "32")]
    #[pyattr]
    pub const MAXGROUPS: usize = MAXREPEAT / 4 / 2;
    #[cfg(target_pointer_width = "64")]
    #[pyattr]
    pub const MAXGROUPS: usize = MAXREPEAT / 2;

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
        let isbytes = !pattern.payload_is::<PyStr>();
        Ok(Pattern {
            pattern,
            flags: SreFlag::from_bits_truncate(flags),
            code: vm.extract_elements::<u32>(&code)?,
            groups,
            groupindex,
            indexgroup: vm.extract_elements(&indexgroup)?,
            isbytes,
        })
    }

    #[derive(FromArgs)]
    struct StringArgs {
        #[pyarg(any)]
        string: PyObjectRef,
        #[pyarg(any, default = "0")]
        pos: usize,
        #[pyarg(any, default = "std::isize::MAX as usize")]
        endpos: usize,
    }

    #[derive(FromArgs)]
    struct SubArgs {
        #[pyarg(any)]
        repl: Either<PyCallable, PyStrRef>,
        #[pyarg(any)]
        string: PyObjectRef,
        #[pyarg(any, default = "0")]
        count: usize,
    }

    #[derive(FromArgs)]
    struct SplitArgs {
        #[pyarg(any)]
        string: PyObjectRef,
        #[pyarg(any, default = "0")]
        maxsplit: isize,
    }

    #[pyattr]
    #[pyclass(name = "Pattern")]
    #[derive(Debug)]
    pub(crate) struct Pattern {
        pub pattern: PyObjectRef,
        pub flags: SreFlag,
        pub code: Vec<u32>,
        pub groups: usize,
        pub groupindex: PyDictRef,
        pub indexgroup: Vec<Option<String>>,
        pub isbytes: bool,
    }

    impl PyValue for Pattern {
        fn class(_vm: &VirtualMachine) -> &PyTypeRef {
            Self::static_type()
        }
    }

    #[pyimpl]
    impl Pattern {
        fn with_str_drive<R, F: FnOnce(StrDrive) -> PyResult<R>>(
            &self,
            string: PyObjectRef,
            vm: &VirtualMachine,
            f: F,
        ) -> PyResult<R> {
            let buffer;
            let guard;
            let vec;
            let s;
            let str_drive = if self.isbytes {
                buffer = try_buffer_from_object(vm, &string)?;
                let bytes = match buffer.as_contiguous() {
                    Some(bytes) => {
                        guard = bytes;
                        &*guard
                    }
                    None => {
                        vec = buffer.to_contiguous();
                        vec.as_slice()
                    }
                };
                StrDrive::Bytes(bytes)
            } else {
                s = string
                    .payload::<PyStr>()
                    .ok_or_else(|| vm.new_type_error("expected string".to_owned()))?;
                StrDrive::Str(s.borrow_value())
            };

            f(str_drive)
        }

        fn with_state<R, F: FnOnce(State) -> PyResult<R>>(
            &self,
            string: PyObjectRef,
            start: usize,
            end: usize,
            vm: &VirtualMachine,
            f: F,
        ) -> PyResult<R> {
            self.with_str_drive(string, vm, |str_drive| {
                let state = State::new(str_drive, start, end, self.flags, &self.code);
                f(state)
            })
        }

        #[pymethod(name = "match")]
        fn pymatch(
            zelf: PyRef<Pattern>,
            string_args: StringArgs,
            vm: &VirtualMachine,
        ) -> PyResult<Option<PyRef<Match>>> {
            zelf.with_state(
                string_args.string.clone(),
                string_args.pos,
                string_args.endpos,
                vm,
                |mut state| {
                    state = state.pymatch();
                    if state.has_matched != Some(true) {
                        Ok(None)
                    } else {
                        Ok(Some(
                            Match::new(&state, zelf.clone(), string_args.string).into_ref(vm),
                        ))
                    }
                },
            )
        }

        #[pymethod]
        fn fullmatch(
            zelf: PyRef<Pattern>,
            string_args: StringArgs,
            vm: &VirtualMachine,
        ) -> PyResult<Option<PyRef<Match>>> {
            // TODO: need optimize
            let m = Self::pymatch(zelf, string_args, vm)?;
            if let Some(m) = m {
                if m.regs[0].0 == m.pos as isize && m.regs[0].1 == m.endpos as isize {
                    return Ok(Some(m));
                }
            }
            Ok(None)
        }

        #[pymethod]
        fn search(
            zelf: PyRef<Pattern>,
            string_args: StringArgs,
            vm: &VirtualMachine,
        ) -> PyResult<Option<PyRef<Match>>> {
            zelf.with_state(
                string_args.string.clone(),
                string_args.pos,
                string_args.endpos,
                vm,
                |mut state| {
                    state = state.search();

                    if state.has_matched != Some(true) {
                        Ok(None)
                    } else {
                        Ok(Some(
                            Match::new(&state, zelf.clone(), string_args.string).into_ref(vm),
                        ))
                    }
                },
            )
        }

        #[pymethod]
        fn findall(
            zelf: PyRef<Pattern>,
            string_args: StringArgs,
            vm: &VirtualMachine,
        ) -> PyResult<PyListRef> {
            zelf.with_state(
                string_args.string.clone(),
                string_args.pos,
                string_args.endpos,
                vm,
                |mut state| {
                    let mut matchlist: Vec<PyObjectRef> = Vec::new();
                    while state.start <= state.end {
                        state.reset();

                        state = state.search();

                        if state.has_matched != Some(true) {
                            break;
                        }

                        let m = Match::new(&state, zelf.clone(), string_args.string.clone());

                        let item = if zelf.groups == 0 || zelf.groups == 1 {
                            m.get_slice(zelf.groups, state.string, vm)
                                .unwrap_or_else(|| vm.ctx.none())
                        } else {
                            m.groups(OptionalArg::Present(vm.ctx.new_str("")), vm)?
                                .into_object()
                        };

                        matchlist.push(item);

                        if state.string_position == state.start {
                            state.start += 1;
                        } else {
                            state.start = state.string_position;
                        }
                    }
                    Ok(PyList::from(matchlist).into_ref(vm))
                },
            )
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
                end: AtomicCell::new(string_args.endpos),
            }
            .into_ref(vm);
            let search = vm.get_method(scanner.into_object(), "search").unwrap()?;
            let search = PyCallable::try_from_object(vm, search)?;
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
                end: AtomicCell::new(string_args.endpos),
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
        ) -> PyResult<PyListRef> {
            zelf.with_state(
                split_args.string.clone(),
                0,
                std::usize::MAX,
                vm,
                |mut state| {
                    let mut splitlist: Vec<PyObjectRef> = Vec::new();

                    let mut n = 0;
                    let mut last_pos = 0;
                    while split_args.maxsplit == 0 || n < split_args.maxsplit {
                        state = state.search();

                        let start = state.start;
                        let end = state.string_position;
                        if start == end {
                            if last_pos == state.end {
                                break;
                            }
                            last_pos = end + 1;
                            continue;
                        }

                        splitlist.push(state.string.slice_to_pyobject(last_pos, start, vm));

                        let m = Match::new(&state, zelf.clone(), split_args.string.clone());

                        // add groups (if any)
                        for i in 1..zelf.groups + 1 {
                            splitlist.push(
                                m.get_slice(i, state.string, vm)
                                    .unwrap_or_else(|| vm.ctx.none()),
                            );
                        }
                        n += 1;
                        last_pos = end;
                    }

                    // get segment following last match (even if empty)
                    splitlist.push(state.string.slice_to_pyobject(
                        last_pos,
                        state.string.count(),
                        vm,
                    ));

                    Ok(PyList::from(splitlist).into_ref(vm))
                },
            )
        }

        #[pyproperty]
        fn flags(&self) -> u16 {
            self.flags.bits()
        }
        #[pyproperty]
        fn groupindex(&self) -> PyDictRef {
            self.groupindex.clone()
        }
        #[pyproperty]
        fn groups(&self) -> usize {
            self.groups
        }
        #[pyproperty]
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
            let filter: PyObjectRef = match repl {
                Either::A(callable) => callable.into_object(),
                Either::B(s) => {
                    if s.borrow_value().contains('\\') {
                        // handle non-literal strings ; hand it over to the template compiler
                        let re = vm.import("re", &[], 0)?;
                        let func = vm.get_attribute(re, "_subx")?;
                        vm.invoke(&func, (zelf.clone(), s))?
                    } else {
                        s.into_object()
                    }
                }
            };

            zelf.with_state(string.clone(), 0, std::usize::MAX, vm, |mut state| {
                let mut sublist: Vec<PyObjectRef> = Vec::new();
                let mut n = 0;
                let mut last_pos = 0;
                while count == 0 || n < count {
                    state = state.search();

                    if state.has_matched != Some(true) {
                        break;
                    }

                    if last_pos < state.start {
                        /* get segment before this match */
                        sublist.push(state.string.slice_to_pyobject(last_pos, state.start, vm));
                    }

                    if vm.is_callable(&filter) {
                        let m = Match::new(&state, zelf.clone(), string.clone());
                        let ret = vm.invoke(&filter, (m.into_ref(vm),))?;
                        sublist.push(ret);
                    } else {
                        sublist.push(filter.clone());
                    }

                    if state.string_position == state.start {
                        state.start += 1;
                    } else {
                        state.start = state.string_position;
                    }

                    last_pos = state.string_position;
                    n += 1;
                }

                /* get segment following last match */
                sublist.push(
                    state
                        .string
                        .slice_to_pyobject(last_pos, state.string.count(), vm),
                );

                let list = PyList::from(sublist).into_object(vm);
                let s = vm.ctx.new_str("");
                let ret = vm.call_method(&s, "join", (list,))?;

                Ok(if subn {
                    (ret, n).into_pyobject(vm)
                } else {
                    ret
                })
            })
        }
    }

    #[pyattr]
    #[pyclass(name = "Match")]
    #[derive(Debug)]
    pub(crate) struct Match {
        string: PyObjectRef,
        pattern: PyRef<Pattern>,
        pos: usize,
        endpos: usize,
        lastindex: isize,
        regs: Vec<(isize, isize)>,
    }
    impl PyValue for Match {
        fn class(_vm: &VirtualMachine) -> &PyTypeRef {
            Self::static_type()
        }
    }

    #[pyimpl]
    impl Match {
        pub(crate) fn new(state: &State, pattern: PyRef<Pattern>, string: PyObjectRef) -> Self {
            let mut regs = vec![(state.start as isize, state.string_position as isize)];
            for group in 0..pattern.groups {
                let mark_index = 2 * group;
                if mark_index + 1 < state.marks.len() {
                    if let (Some(start), Some(end)) =
                        (state.marks[mark_index], state.marks[mark_index + 1])
                    {
                        regs.push((start as isize, end as isize));
                        continue;
                    }
                }
                regs.push((-1, -1));
            }
            Self {
                string,
                pattern,
                pos: state.start,
                endpos: state.end,
                lastindex: state.lastindex,
                regs,
            }
        }

        #[pyproperty]
        fn pos(&self) -> usize {
            self.pos
        }
        #[pyproperty]
        fn endpos(&self) -> usize {
            self.endpos
        }
        #[pyproperty]
        fn lastindex(&self) -> Option<isize> {
            if self.lastindex >= 0 {
                Some(self.lastindex)
            } else {
                None
            }
        }
        #[pyproperty]
        fn lastgroup(&self) -> Option<String> {
            if self.lastindex >= 0 && (self.lastindex as usize) < self.pattern.indexgroup.len() {
                self.pattern.indexgroup[self.lastindex as usize].clone()
            } else {
                None
            }
        }
        #[pyproperty]
        fn re(&self) -> PyObjectRef {
            self.pattern.clone().into_object()
        }
        #[pyproperty]
        fn string(&self) -> PyObjectRef {
            self.string.clone()
        }
        #[pyproperty]
        fn regs(&self, vm: &VirtualMachine) -> PyTupleRef {
            PyTupleRef::with_elements(
                self.regs.iter().map(|&x| x.into_pyobject(vm)).collect(),
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
            let index = match group {
                OptionalArg::Present(group) => self
                    .get_index(group, vm)
                    .ok_or_else(|| vm.new_index_error("no such group".to_owned()))?,
                OptionalArg::Missing => 0,
            };
            Ok(self.regs[index])
        }

        #[pymethod]
        fn expand(zelf: PyRef<Match>, template: PyStrRef, vm: &VirtualMachine) -> PyResult {
            let re = vm.import("re", &[], 0)?;
            let func = vm.get_attribute(re, "_expand")?;
            vm.invoke(&func, (zelf.pattern.clone(), zelf, template))
        }

        #[pymethod]
        fn group(&self, args: Args<PyObjectRef>, vm: &VirtualMachine) -> PyResult {
            self.pattern
                .with_str_drive(self.string.clone(), vm, |str_drive| {
                    let args = args.into_vec();
                    if args.is_empty() {
                        return Ok(self.get_slice(0, str_drive, vm).unwrap().into_pyobject(vm));
                    }
                    let mut v: Vec<PyObjectRef> = args
                        .into_iter()
                        .map(|x| {
                            self.get_index(x, vm)
                                .ok_or_else(|| vm.new_index_error("no such group".to_owned()))
                                .map(|index| {
                                    self.get_slice(index, str_drive, vm)
                                        .map(|x| x.into_pyobject(vm))
                                        .unwrap_or_else(|| vm.ctx.none())
                                })
                        })
                        .try_collect()?;
                    if v.len() == 1 {
                        Ok(v.pop().unwrap())
                    } else {
                        Ok(vm.ctx.new_tuple(v))
                    }
                })
        }

        #[pymethod(magic)]
        fn getitem(
            &self,
            group: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult<Option<PyObjectRef>> {
            self.pattern
                .with_str_drive(self.string.clone(), vm, |str_drive| {
                    Ok(self
                        .get_index(group, vm)
                        .and_then(|i| self.get_slice(i, str_drive, vm)))
                })
        }

        #[pymethod]
        fn groups(
            &self,
            default: OptionalArg<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<PyTupleRef> {
            let default = default.unwrap_or_else(|| vm.ctx.none());

            self.pattern
                .with_str_drive(self.string.clone(), vm, |str_drive| {
                    let v: Vec<PyObjectRef> = (1..self.regs.len())
                        .map(|i| {
                            self.get_slice(i, str_drive, vm)
                                .map(|s| s.into_pyobject(vm))
                                .unwrap_or_else(|| default.clone())
                        })
                        .collect();
                    Ok(PyTupleRef::with_elements(v, &vm.ctx))
                })
        }

        #[pymethod]
        fn groupdict(
            &self,
            default: OptionalArg<PyObjectRef>,
            vm: &VirtualMachine,
        ) -> PyResult<PyDictRef> {
            let default = default.unwrap_or_else(|| vm.ctx.none());

            self.pattern
                .with_str_drive(self.string.clone(), vm, |str_drive| {
                    let dict = vm.ctx.new_dict();

                    for (key, index) in self.pattern.groupindex.clone() {
                        let value = self
                            .get_index(index, vm)
                            .and_then(|x| self.get_slice(x, str_drive, vm))
                            .map(|x| x.into_pyobject(vm))
                            .unwrap_or_else(|| default.clone());
                        dict.set_item(key, value, vm)?;
                    }
                    Ok(dict)
                })
        }

        #[pymethod(magic)]
        fn repr(&self, vm: &VirtualMachine) -> PyResult<String> {
            self.pattern
                .with_str_drive(self.string.clone(), vm, |str_drive| {
                    Ok(format!(
                        "<re.Match object; span=({}, {}), match={}>",
                        self.regs[0].0,
                        self.regs[0].1,
                        vm.to_repr(&self.get_slice(0, str_drive, vm).unwrap())?
                    ))
                })
        }

        fn get_index(&self, group: PyObjectRef, vm: &VirtualMachine) -> Option<usize> {
            let i = match group.downcast::<PyInt>() {
                Ok(i) => i,
                Err(group) => self
                    .pattern
                    .groupindex
                    .get_item_option(group, vm)
                    .ok()??
                    .downcast::<PyInt>()
                    .ok()?,
            };
            let i = i.borrow_value().to_isize()?;
            if i >= 0 && i as usize <= self.pattern.groups {
                Some(i as usize)
            } else {
                None
            }
        }

        fn get_slice(
            &self,
            index: usize,
            str_drive: StrDrive,
            vm: &VirtualMachine,
        ) -> Option<PyObjectRef> {
            let (start, end) = self.regs[index];
            if start < 0 || end < 0 {
                return None;
            }
            Some(str_drive.slice_to_pyobject(start as usize, end as usize, vm))
        }
    }

    #[pyattr]
    #[pyclass(name = "SRE_Scanner")]
    #[derive(Debug)]
    struct SreScanner {
        pattern: PyRef<Pattern>,
        string: PyObjectRef,
        start: AtomicCell<usize>,
        end: AtomicCell<usize>,
    }
    impl PyValue for SreScanner {
        fn class(_vm: &VirtualMachine) -> &PyTypeRef {
            Self::static_type()
        }
    }

    #[pyimpl]
    impl SreScanner {
        #[pyproperty]
        fn pattern(&self) -> PyRef<Pattern> {
            self.pattern.clone()
        }

        #[pymethod(name = "match")]
        fn pymatch(&self, vm: &VirtualMachine) -> PyResult<Option<PyRef<Match>>> {
            self.pattern.with_state(
                self.string.clone(),
                self.start.load(),
                self.end.load(),
                vm,
                |mut state| {
                    state = state.pymatch();

                    if state.has_matched != Some(true) {
                        Ok(None)
                    } else {
                        if state.start == state.string_position {
                            self.start.store(state.string_position + 1);
                        } else {
                            self.start.store(state.string_position);
                        }
                        Ok(Some(
                            Match::new(&state, self.pattern.clone(), self.string.clone())
                                .into_ref(vm),
                        ))
                    }
                },
            )
        }

        #[pymethod]
        fn search(&self, vm: &VirtualMachine) -> PyResult<Option<PyRef<Match>>> {
            self.pattern.with_state(
                self.string.clone(),
                self.start.load(),
                self.end.load(),
                vm,
                |mut state| {
                    state = state.search();

                    if state.has_matched == Some(true) {
                        if state.start == state.string_position {
                            self.start.store(state.string_position + 1);
                        } else {
                            self.start.store(state.string_position);
                        }
                        Ok(Some(
                            Match::new(&state, self.pattern.clone(), self.string.clone())
                                .into_ref(vm),
                        ))
                    } else {
                        Ok(None)
                    }
                },
            )
        }
    }
}
