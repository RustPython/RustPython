//! byte operation APIs
use crate::object::AsObject;
use crate::{PyObject, PyResult, VirtualMachine};
use num_traits::ToPrimitive;

pub fn bytes_from_object(vm: &VirtualMachine, obj: &PyObject) -> PyResult<Vec<u8>> {
    if let Ok(elements) = obj.try_bytes_like(vm, |bytes| bytes.to_vec()) {
        return Ok(elements);
    }

    if !obj.fast_isinstance(vm.ctx.types.str_type) {
        if let Ok(elements) = vm.map_iterable_object(obj, |x| value_from_object(vm, &x)) {
            return elements;
        }
    }

    Err(vm.new_type_error(
        "can assign only bytes, buffers, or iterables of ints in range(0, 256)".to_owned(),
    ))
}

pub fn value_from_object(vm: &VirtualMachine, obj: &PyObject) -> PyResult<u8> {
    obj.try_index(vm)?
        .as_bigint()
        .to_u8()
        .ok_or_else(|| vm.new_value_error("byte must be in range(0, 256)".to_owned()))
}
