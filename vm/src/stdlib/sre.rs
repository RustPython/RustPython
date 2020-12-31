mod constants;
mod interp;

pub(crate) use _sre::make_module;

#[pymodule]
mod _sre {
    use itertools::Itertools;
    use rustpython_common::borrow::BorrowValue;
    use rustpython_common::lock::OnceCell;

    use super::constants::SreFlag;
    use super::interp::{self, lower_ascii, lower_unicode, upper_unicode, State};
    use crate::builtins::tuple::PyTupleRef;
    use crate::builtins::{PyStrRef, PyTypeRef};
    use crate::function::{Args, OptionalArg};
    use crate::pyobject::{
        Either, IntoPyObject, PyCallable, PyObjectRef, PyRef, PyResult, PyValue, StaticType,
        TypeProtocol,
    };
    use crate::VirtualMachine;
    use std::collections::HashMap;
    use std::convert::TryFrom;

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
        let ch = match char::try_from(ch) {
            Ok(ch) => ch,
            Err(_) => {
                return false;
            }
        };
        ch != lower_unicode(ch) || ch != upper_unicode(ch)
    }
    #[pyfunction]
    fn ascii_tolower(ch: i32) -> i32 {
        let ch = match char::try_from(ch as u32) {
            Ok(ch) => ch,
            Err(_) => {
                return ch;
            }
        };
        lower_ascii(ch) as i32
    }
    #[pyfunction]
    fn unicode_tolower(ch: i32) -> i32 {
        let ch = match char::try_from(ch as u32) {
            Ok(ch) => ch,
            Err(_) => {
                return ch;
            }
        };
        lower_unicode(ch) as i32
    }

    #[pyfunction]
    fn compile(
        pattern: PyObjectRef,
        flags: u16,
        code: PyObjectRef,
        groups: usize,
        groupindex: HashMap<String, usize>,
        indexgroup: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<Pattern> {
        Ok(Pattern {
            pattern,
            flags: SreFlag::from_bits_truncate(flags),
            code: vm.extract_elements::<u32>(&code)?,
            groups,
            groupindex,
            indexgroup: vm.extract_elements(&indexgroup)?,
        })
    }

    #[derive(FromArgs)]
    struct StringArgs {
        #[pyarg(any)]
        string: PyStrRef,
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
        string: PyStrRef,
        #[pyarg(any, default = "0")]
        count: usize,
    }

    #[pyattr]
    #[pyclass(name = "Pattern")]
    #[derive(Debug)]
    pub(crate) struct Pattern {
        pub pattern: PyObjectRef,
        pub flags: SreFlag,
        pub code: Vec<u32>,
        pub groups: usize,
        pub groupindex: HashMap<String, usize>,
        pub indexgroup: Vec<Option<String>>,
    }

    impl PyValue for Pattern {
        fn class(_vm: &VirtualMachine) -> &PyTypeRef {
            Self::static_type()
        }
    }

    #[pyimpl]
    impl Pattern {
        #[pymethod(name = "match")]
        fn pymatch(
            zelf: PyRef<Pattern>,
            string_args: StringArgs,
            vm: &VirtualMachine,
        ) -> Option<PyRef<Match>> {
            interp::pymatch(
                string_args.string,
                string_args.pos,
                string_args.endpos,
                zelf,
            )
            .map(|x| x.into_ref(vm))
        }
        #[pymethod]
        fn fullmatch(
            zelf: PyRef<Pattern>,
            string_args: StringArgs,
            vm: &VirtualMachine,
        ) -> Option<PyRef<Match>> {
            // TODO: need optimize
            let m = Self::pymatch(zelf, string_args, vm);
            if let Some(m) = m {
                if m.regs[0].0 == m.pos as isize && m.regs[0].1 == m.endpos as isize {
                    return Some(m);
                }
            }
            None
        }
        #[pymethod]
        fn search(
            zelf: PyRef<Pattern>,
            string_args: StringArgs,
            vm: &VirtualMachine,
        ) -> Option<PyRef<Match>> {
            // TODO: optimize by op info and skip prefix
            let start = string_args.pos;
            for i in start..string_args.endpos {
                if let Some(m) = interp::pymatch(
                    string_args.string.clone(),
                    i,
                    string_args.endpos,
                    zelf.clone(),
                ) {
                    return Some(m.into_ref(vm));
                }
            }
            None
        }
        #[pymethod]
        fn findall(&self, string_args: StringArgs) -> Option<PyObjectRef> {
            None
        }
        #[pymethod]
        fn finditer(&self, string_args: StringArgs) -> Option<PyObjectRef> {
            None
        }
        #[pymethod]
        fn scanner(&self, string_args: StringArgs) -> Option<PyObjectRef> {
            None
        }
        #[pymethod]
        fn sub(&self, sub_args: SubArgs, vm: &VirtualMachine) -> PyResult<PyStrRef> {
            Err(vm.new_not_implemented_error("".to_owned()))
        }
        #[pyproperty]
        fn flags(&self) -> u16 {
            self.flags.bits()
        }

        fn subx(&self, sub_args: SubArgs, vm: &VirtualMachine) -> PyResult<PyStrRef> {
            Err(vm.new_not_implemented_error("".to_owned()))
        }
    }

    #[pyattr]
    #[pyclass(name = "Match")]
    #[derive(Debug)]
    pub(crate) struct Match {
        string: PyStrRef,
        pattern: PyRef<Pattern>,
        pos: usize,
        endpos: usize,
        lastindex: isize,
        regs: Vec<(isize, isize)>,
        regs_pytuple: OnceCell<PyTupleRef>,
        // lastgroup
    }
    impl PyValue for Match {
        fn class(_vm: &VirtualMachine) -> &PyTypeRef {
            Self::static_type()
        }
    }

    #[pyimpl]
    impl Match {
        pub(crate) fn new(state: &State, pattern: PyRef<Pattern>, string: PyStrRef) -> Self {
            let mut regs = vec![(state.start as isize, state.string_position as isize)];
            for group in 0..pattern.groups {
                let mark_index = 2 * group;
                match (
                    mark_index + 1 < state.marks.len(),
                    state.marks[mark_index],
                    state.marks[mark_index + 1],
                ) {
                    (true, Some(start), Some(end)) => {
                        regs.push((start as isize, end as isize));
                    }
                    _ => {
                        regs.push((-1, -1));
                    }
                }
            }
            Self {
                string,
                pattern,
                pos: state.start,
                endpos: state.end,
                lastindex: state.lastindex,
                regs,
                regs_pytuple: OnceCell::new(),
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
        fn lastindex(&self) -> isize {
            self.lastindex
        }
        #[pyproperty]
        fn lastgroup(&self) -> Option<String> {
            None
        }
        #[pyproperty]
        fn re(&self) -> PyObjectRef {
            self.pattern.clone().into_object()
        }
        #[pyproperty]
        fn string(&self) -> PyStrRef {
            self.string.clone()
        }
        #[pyproperty]
        fn regs(&self, vm: &VirtualMachine) -> PyTupleRef {
            self.regs_pytuple
                .get_or_init(|| {
                    PyTupleRef::with_elements(
                        self.regs.iter().map(|&x| x.into_pyobject(vm)).collect(),
                        &vm.ctx,
                    )
                })
                .clone()
        }

        #[pymethod]
        fn start(&self, group: OptionalArg<isize>, vm: &VirtualMachine) -> PyResult<isize> {
            self.get_index(group.unwrap_or(0), vm)
                .map(|x| self.regs[x].0)
        }
        #[pymethod]
        fn end(&self, group: OptionalArg<isize>, vm: &VirtualMachine) -> PyResult<isize> {
            self.get_index(group.unwrap_or(0), vm)
                .map(|x| self.regs[x].1)
        }
        #[pymethod]
        fn span(&self, group: OptionalArg<isize>, vm: &VirtualMachine) -> PyResult<(isize, isize)> {
            self.get_index(group.unwrap_or(0), vm).map(|x| self.regs[x])
        }
        #[pymethod]
        fn group(&self, args: Args<isize>, vm: &VirtualMachine) -> PyResult {
            let mut args = args.into_vec();
            if args.is_empty() {
                args.push(0);
            }
            let mut v: Vec<PyObjectRef> = args
                .iter()
                .map(|&x| {
                    self.get_index(x, vm)
                        .map(|i| self.get_slice(i).unwrap().into_pyobject(vm))
                })
                .try_collect()?;
            if v.len() == 1 {
                Ok(v.pop().unwrap())
            } else {
                Ok(vm.ctx.new_tuple(v))
            }
        }
        #[pymethod(magic)]
        fn repr(zelf: PyRef<Match>) -> String {
            format!(
                "<re.Match object; span=({}, {}), match='{}'>",
                zelf.regs[0].0,
                zelf.regs[0].1,
                zelf.get_slice(0).unwrap()
            )
        }

        fn get_index(&self, group: isize, vm: &VirtualMachine) -> PyResult<usize> {
            // TODO: support key, value index
            if group >= 0 && group as usize <= self.pattern.groups {
                Ok(group as usize)
            } else {
                Err(vm.new_index_error("no such group".to_owned()))
            }
        }

        fn get_slice(&self, group: usize) -> Option<String> {
            let (start, end) = self.regs[group];
            if start < 0 || end < 0 {
                return None;
            }
            self.string
                .borrow_value()
                .get(start as usize..end as usize)
                .map(|x| x.to_string())
        }
    }
}
