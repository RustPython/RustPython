//! byte operation APIs

use num_traits::ToPrimitive;

use crate::{
    AsObject, PyObject, PyResult, VirtualMachine,
    protocol::{BufferFlags, PyBuffer},
};

// PyBytes_FromObject
pub fn bytes_from_object(vm: &VirtualMachine, obj: &PyObject) -> PyResult<Vec<u8>> {
    if obj.check_buffer() {
        let buffer = PyBuffer::from_object(vm, obj, BufferFlags::FULL_RO)?;
        return Ok(buffer.contiguous_or_collect(|bytes| bytes.to_vec()));
    }

    if !obj.fast_isinstance(vm.ctx.types.str_type)
        && let Ok(elements) = vm.map_iterable_object(obj, |x| value_from_object(vm, &x))
    {
        return elements;
    }

    Err(vm.new_type_error("can assign only bytes, buffers, or iterables of ints in range(0, 256)"))
}

pub fn value_from_object(vm: &VirtualMachine, obj: &PyObject) -> PyResult<u8> {
    obj.try_index(vm)?
        .as_bigint()
        .to_u8()
        .ok_or_else(|| vm.new_value_error("byte must be in range(0, 256)"))
}
