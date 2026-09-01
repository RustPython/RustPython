//! byte operation APIs

use num_traits::ToPrimitive;

use crate::{
    AsObject, PyObject, PyObjectRef, PyResult, VirtualMachine,
    protocol::{BufferFlags, PyBuffer},
};

// PyBytes_FromObject
pub fn bytes_from_object(vm: &VirtualMachine, obj: &PyObject) -> PyResult<Vec<u8>> {
    collect_bytes(vm, obj, true, |name| {
        format!("cannot convert '{name}' object to bytes")
    })
}

/// [`bytes_from_object`] for the bytearray constructor and for assigning to a
/// slice of one, which run the iterator without asking the object they were
/// handed how long it is.
pub fn bytearray_from_object(vm: &VirtualMachine, obj: &PyObject) -> PyResult<Vec<u8>> {
    collect_bytes(vm, obj, false, |name| {
        format!("cannot convert '{name}' object to bytearray")
    })
}

/// [`bytes_from_object`] for `bytearray_extend()`, which names what it was
/// doing rather than what it was converting to.
pub fn bytearray_extend_from_object(vm: &VirtualMachine, obj: &PyObject) -> PyResult<Vec<u8>> {
    collect_bytes(vm, obj, true, |name| {
        format!("can't extend bytearray with {name}")
    })
}

/// `measured` is whether the object is asked how long it is; `unusable` names,
/// from the class name, what could not be done with one that is not iterable.
fn collect_bytes(
    vm: &VirtualMachine,
    obj: &PyObject,
    measured: bool,
    unusable: impl FnOnce(&str) -> String,
) -> PyResult<Vec<u8>> {
    if obj.check_buffer() {
        let buffer = PyBuffer::from_object(vm, obj, BufferFlags::FULL_RO)?;
        return Ok(buffer.contiguous_or_collect(|bytes| bytes.to_vec()));
    }

    if !obj.fast_isinstance(vm.ctx.types.str_type) {
        // What `PyObject_GetIter()` cannot take is answered for by the caller,
        // which knows what it was being asked to do, rather than by the
        // iteration protocol saying the object is not iterable.
        let cls = obj.class();
        if cls.slots.iter.load().is_none() && !cls.has_attr(identifier!(vm, __getitem__)) {
            return Err(vm.new_type_error(unusable(&cls.name())));
        }
        let value = |x: PyObjectRef| value_from_object(vm, &x);
        let elements = if measured {
            vm.map_iterable_object_sized(obj, value)
        } else {
            vm.map_iterable_object(obj, value)
        };
        if let Ok(elements) = elements {
            return elements;
        }
    }

    Err(vm.new_type_error("can assign only bytes, buffers, or iterables of ints in range(0, 256)"))
}

pub fn value_from_object(vm: &VirtualMachine, obj: &PyObject) -> PyResult<u8> {
    obj.try_index(vm)?
        .as_bigint()
        .to_u8()
        .ok_or_else(|| vm.new_value_error("byte must be in range(0, 256)"))
}
