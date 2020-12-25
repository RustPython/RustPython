mod constants;
mod interp;

pub(crate) use _sre::make_module;

#[pymodule]
mod _sre {
    use super::interp::{lower_ascii, lower_unicode, upper_unicode};
    use super::{
        constants::SreFlag,
        interp::{self, State},
    };
    use crate::builtins::{PyStrRef, PyTypeRef};
    use crate::byteslike::PyBytesLike;
    use crate::common::borrow::BorrowValue;
    use crate::pyobject::{Either, PyCallable, PyObjectRef, PyResult, PyValue, StaticType};
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
        repl: Either<PyCallable, PyBytesLike>,
        #[pyarg(any)]
        string: PyBytesLike,
        #[pyarg(any, default = "0")]
        count: usize,
    }

    #[pyattr]
    #[pyclass(name = "Pattern")]
    #[derive(Debug)]
    struct Pattern {
        pattern: PyObjectRef,
        flags: SreFlag,
        code: Vec<u32>,
        groups: usize,
        groupindex: HashMap<String, usize>,
        indexgroup: Vec<Option<String>>,
    }

    impl PyValue for Pattern {
        fn class(_vm: &VirtualMachine) -> &PyTypeRef {
            Self::static_type()
        }
    }

    #[pyimpl]
    impl Pattern {
        #[pymethod(name = "match")]
        fn pymatch(&self, string_args: StringArgs) -> Option<PyObjectRef> {
            let start = string_args.pos;
            let end = string_args.endpos;
            let flags = self.flags;
            let pattern_codes = self.code.clone();
            let string = string_args.string.borrow_value();
            let mut state = State::new(string, start, end, flags, pattern_codes);
            dbg!(&state);
            dbg!(interp::pymatch(state));
            None
        }
        #[pymethod]
        fn fullmatch(&self, string_args: StringArgs) -> Option<PyObjectRef> {
            None
        }
        #[pymethod]
        fn search(&self, string_args: StringArgs) -> Option<PyObjectRef> {
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
}
