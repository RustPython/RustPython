mod constants;
mod interp;

pub(crate) use _sre::make_module;

#[pymodule]
mod _sre {
    use super::constants::SreFlag;
    use crate::builtins::PyTypeRef;
    use crate::pyobject::{PyObjectRef, PyResult, PyValue, StaticType};
    use crate::VirtualMachine;
    use std::collections::HashMap;

    #[pyattr]
    use super::constants::SRE_MAGIC as MAGIC;

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

        Ok(Pattern {
            pattern,
            flags: SreFlag::from_bits_truncate(flags),
            code,
            groups,
            groupindex,
            indexgroup: vm.extract_elements(&indexgroup)?,
        })
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
    impl Pattern {}
}
