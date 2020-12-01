mod constants;
mod interp;

pub(crate) use _sre::make_module;

#[pymodule]
mod _sre {
    use super::{constants::SreFlag, interp::{self, State}};
    use crate::common::borrow::BorrowValue;
    use crate::VirtualMachine;
    use crate::{
        builtins::PyStrRef,
        pyobject::{Either, PyCallable, PyObjectRef, PyResult, PyValue, StaticType},
    };
    use crate::{builtins::PyTypeRef, byteslike::PyBytesLike};
    use std::collections::HashMap;

    #[pyattr]
    use super::constants::SRE_CODESIZE as CODESIZE;
    #[pyattr]
    use super::constants::SRE_MAGIC as MAGIC;
    #[pyattr]
    use super::constants::SRE_MAXGROUPS as MAXGROUPS;
    #[pyattr]
    use super::constants::SRE_MAXREPEAT as MAXREPEAT;

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
            let mut state = State::new(
                // string_args.string,
                string,
                start,
                end,
                flags,
                pattern_codes
            );
            interp::pymatch(state);
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
    }
}
