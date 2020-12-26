mod constants;
mod interp;

pub(crate) use _sre::make_module;

#[pymodule]
mod _sre {
    use super::constants::SreFlag;
    use super::interp::{self, lower_ascii, lower_unicode, upper_unicode, State};
    use crate::builtins::{PyStrRef, PyTypeRef};
    use crate::pyobject::{Either, PyCallable, PyObjectRef, PyRef, PyResult, PyValue, StaticType};
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
        let code = vm.extract_elements::<u32>(&code)?;
        dbg!(&code);

        Ok(Pattern {
            pattern,
            flags: SreFlag::from_bits_truncate(flags),
            code,
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
        fn pymatch(&self, string_args: StringArgs, vm: &VirtualMachine) -> Option<PyRef<Match>> {
            interp::pymatch(
                string_args.string,
                string_args.pos,
                string_args.endpos,
                &self,
            )
            .map(|x| x.into_ref(vm))
        }
        #[pymethod]
        fn fullmatch(&self, string_args: StringArgs, vm: &VirtualMachine) -> Option<PyRef<Match>> {
            // TODO: need optimize
            let start = string_args.pos;
            let end = string_args.endpos;
            let m = self.pymatch(string_args, vm);
            if let Some(m) = m {
                if m.start == start && m.end == end {
                    return Some(m);
                }
            }
            None
        }
        #[pymethod]
        fn search(&self, string_args: StringArgs, vm: &VirtualMachine) -> Option<PyRef<Match>> {
            // TODO: optimize by op info and skip prefix
            let start = string_args.pos;
            for i in start..string_args.endpos {
                if let Some(m) =
                    interp::pymatch(string_args.string.clone(), i, string_args.endpos, &self)
                {
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
    }

    #[pyattr]
    #[pyclass(name = "Match")]
    #[derive(Debug)]
    pub(crate) struct Match {
        string: PyStrRef,
        pattern: PyObjectRef,
        start: usize,
        end: usize,
        lastindex: isize,
        // regs
        // lastgroup
    }
    impl PyValue for Match {
        fn class(vm: &VirtualMachine) -> &PyTypeRef {
            Self::static_type()
        }
    }

    #[pyimpl]
    impl Match {
        pub(crate) fn new(state: &State, pattern: PyObjectRef, string: PyStrRef) -> Self {
            Self {
                string,
                pattern,
                start: state.start,
                end: state.end,
                lastindex: state.lastindex,
            }
        }
        #[pyproperty]
        fn pos(&self) -> usize {
            self.start
        }
        #[pyproperty]
        fn endpos(&self) -> usize {
            self.end
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
            self.pattern.clone()
        }
        #[pyproperty]
        fn string(&self) -> PyStrRef {
            self.string.clone()
        }
    }
}
