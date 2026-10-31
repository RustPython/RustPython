//! Serialization modes exposed only by the canonical cross-interpreter test hooks.
//! Keep fixture-specific modes out of the production SharedValue representation.

use super::*;
use crate::builtins::PyUtf8Str;

#[derive(Clone)]
pub(crate) enum CrossinterpData {
    Shared(SharedValue),
    Marshalled(Vec<u8>),
}

impl CrossinterpData {
    pub(crate) fn from_object(
        obj: &PyObject,
        mode: Option<&PyObject>,
        vm: &VirtualMachine,
    ) -> PyResult<Self> {
        let mode_obj = mode.filter(|mode| !vm.is_none(mode));
        let mode = match mode_obj {
            None => "xidata",
            Some(mode) => {
                if !mode.downcastable::<PyStr>() {
                    return Err(
                        vm.new_type_error(format!("expected mode str, got {}", mode.repr(vm)?))
                    );
                }
                // CPython compares a NUL-terminated UTF-8 string.
                mode.try_downcast_ref::<PyUtf8Str>(vm)?
                    .as_str()
                    .split('\0')
                    .next()
                    .unwrap_or_default()
            }
        };
        let data = match mode {
            "" | "xidata" => SharedValue::from_object(obj, Fallback::XidataOnly, vm),
            "fallback" => SharedValue::from_object(obj, Fallback::Full, vm),
            "pickle" => pickle_dumps(obj, vm)
                .map(SharedValue::Pickled)
                .map_err(|cause| {
                    not_shareable_error_from(vm, "object could not be pickled", cause)
                }),
            "marshal" => return marshal_dumps(obj, vm).map(Self::Marshalled),
            "code" => {
                let code = obj.downcast_ref::<PyCode>().ok_or_else(|| {
                    not_shareable_error(vm, format!("expected code, got {}", render_repr(obj, vm)))
                })?;
                SharedValue::from_code(code, vm)
            }
            "func" => SharedValue::from_function(obj, vm),
            "script" | "script-pure" => {
                let code =
                    script_code_with_purity(obj, mode == "script-pure", vm).map_err(|cause| {
                        not_shareable_error_from(vm, "object not a valid script", cause)
                    })?;
                SharedValue::from_code(&code, vm)
            }
            _ => {
                return Err(
                    vm.new_value_error(format!("unsupported mode {}", mode_obj.unwrap().repr(vm)?))
                );
            }
        }?;
        Ok(Self::Shared(data))
    }

    pub(crate) fn into_object(self, vm: &VirtualMachine) -> PyResult {
        match self {
            Self::Shared(data) => data.into_object(vm),
            Self::Marshalled(data) => marshal_loads(&data, vm),
        }
    }
}
