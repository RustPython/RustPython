//! Serialization modes exposed only by the canonical cross-interpreter test hooks.
//! Keep fixture-specific modes out of the production SharedValue representation.

use super::*;

#[derive(Clone)]
pub(crate) enum CrossinterpData {
    Shared(SharedValue),
    Marshalled(Vec<u8>),
}

impl CrossinterpData {
    pub(crate) fn from_object(
        obj: &PyObject,
        fallback: Fallback,
        vm: &VirtualMachine,
    ) -> PyResult<Self> {
        SharedValue::from_object(obj, fallback, vm).map(Self::Shared)
    }

    pub(crate) fn from_code(code: &Py<PyCode>, vm: &VirtualMachine) -> PyResult<Self> {
        SharedValue::from_code(code, vm).map(Self::Shared)
    }

    pub(crate) fn from_function(obj: &PyObject, vm: &VirtualMachine) -> PyResult<Self> {
        SharedValue::from_function(obj, vm).map(Self::Shared)
    }

    pub(crate) fn from_pickle(obj: &PyObject, vm: &VirtualMachine) -> PyResult<Self> {
        pickle_dumps(obj, vm)
            .map(SharedValue::Pickled)
            .map(Self::Shared)
            .map_err(|cause| not_shareable_error_from(vm, "object could not be pickled", cause))
    }

    pub(crate) fn from_marshal(obj: &PyObject, vm: &VirtualMachine) -> PyResult<Self> {
        marshal_dumps(obj, vm).map(Self::Marshalled)
    }

    pub(crate) fn from_script(obj: &PyObject, pure: bool, vm: &VirtualMachine) -> PyResult<Self> {
        let code = script_code_with_purity(obj, pure, vm)
            .map_err(|cause| not_shareable_error_from(vm, "object not a valid script", cause))?;
        Self::from_code(&code, vm)
    }

    pub(crate) fn into_object(self, vm: &VirtualMachine) -> PyResult {
        match self {
            Self::Shared(data) => data.into_object(vm),
            Self::Marshalled(data) => marshal_loads(&data, vm),
        }
    }
}
