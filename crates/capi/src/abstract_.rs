use crate::{PyObject, pystate::with_vm};
use alloc::slice;
use core::ffi::c_int;
pub use iter::*;
pub use mapping::*;
pub use number::*;
use rustpython_vm::builtins::{PyDict, PyStr, PyTuple};
use rustpython_vm::function::{FuncArgs, KwArgs, PosArgs};
use rustpython_vm::{AsObject, Py, PyObjectRef, PyResult, VirtualMachine};
pub use sequence::*;

mod iter;
mod mapping;
mod number;
mod sequence;

const PY_VECTORCALL_ARGUMENTS_OFFSET: usize = 1usize << (usize::BITS as usize - 1);

fn tuple_to_args(tuple: &Py<PyTuple>) -> PosArgs {
    tuple.iter().cloned().collect::<Vec<_>>().into()
}

fn dict_to_kwargs(vm: &VirtualMachine, dict: &Py<PyDict>) -> PyResult<KwArgs> {
    dict.items_vec()
        .into_iter()
        .map(|(key, value)| {
            let key = key
                .downcast_ref::<PyStr>()
                .map(|s| s.to_string())
                .ok_or_else(|| vm.new_type_error("keywords must be strings"))?;
            Ok((key, value))
        })
        .collect::<PyResult<_>>()
        .map(KwArgs::new)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_Call(
    callable: *mut PyObject,
    args: *mut PyObject,
    kwargs: *mut PyObject,
) -> *mut PyObject {
    with_vm(|vm| {
        let callable = unsafe { &*callable };
        let args = tuple_to_args(unsafe { &*args }.try_downcast_ref::<PyTuple>(vm)?);

        let kwargs: Option<KwArgs> = unsafe { kwargs.as_ref() }
            .map(|kwargs| dict_to_kwargs(vm, kwargs.try_downcast_ref::<PyDict>(vm)?))
            .transpose()?;

        callable.call_with_args(FuncArgs::new(args, kwargs.unwrap_or_default()), vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_CallNoArgs(callable: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| unsafe { &*callable }.call((), vm))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_Vectorcall(
    callable: *mut PyObject,
    args: *const *mut PyObject,
    nargsf: usize,
    kwnames: *mut PyObject,
) -> *mut PyObject {
    with_vm(|vm| {
        let num_positional_args = nargsf & !PY_VECTORCALL_ARGUMENTS_OFFSET;

        let kwnames: Option<&[PyObjectRef]> = unsafe {
            kwnames
                .as_ref()
                .map(|tuple| Ok(&***tuple.try_downcast_ref::<PyTuple>(vm)?))
                .transpose()?
        };

        let args_len = num_positional_args + kwnames.map_or(0, <[PyObjectRef]>::len);
        let args = if args_len == 0 {
            Vec::new()
        } else {
            unsafe { slice::from_raw_parts(args, args_len) }
                .iter()
                .map(|arg| unsafe { &**arg }.to_owned())
                .collect::<Vec<_>>()
        };

        let callable = unsafe { &*callable };
        callable.vectorcall(args, num_positional_args, kwnames, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_VectorcallMethod(
    name: *mut PyObject,
    args: *const *mut PyObject,
    nargsf: usize,
    kwnames: *mut PyObject,
) -> *mut PyObject {
    with_vm(|vm| {
        let args_len = nargsf & !PY_VECTORCALL_ARGUMENTS_OFFSET;

        if args_len == 0 {
            return Err(vm.new_system_error("PyObject_VectorcallMethod called with no receiver"));
        }

        let (receiver, args) = unsafe { slice::from_raw_parts(args, args_len) }
            .split_first()
            .expect("args_len > 0 should guarantee a receiver");

        let method_name = unsafe { (&*name).try_downcast_ref::<PyStr>(vm)? };
        let callable = unsafe { (&**receiver).get_attr(method_name, vm)? };

        Ok(unsafe {
            PyObject_Vectorcall(
                callable.as_object().as_raw().cast_mut(),
                args.as_ptr(),
                nargsf - 1,
                kwnames,
            )
        })
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GetItem(obj: *mut PyObject, key: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let obj = unsafe { &*obj };
        let key = unsafe { &*key };
        obj.get_item(key, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_SetItem(
    obj: *mut PyObject,
    key: *mut PyObject,
    value: *mut PyObject,
) -> c_int {
    with_vm(|vm| {
        let obj = unsafe { &*obj };
        let key = unsafe { &*key };
        let value = unsafe { &*value }.to_owned();
        obj.set_item(key, value, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_DelItem(obj: *mut PyObject, key: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let obj = unsafe { &*obj };
        let key = unsafe { &*key };
        obj.del_item(key, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_IsSubclass(derived: *mut PyObject, cls: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let derived = unsafe { &*derived };
        let cls = unsafe { &*cls };
        derived.is_subclass(cls, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_IsInstance(inst: *mut PyObject, cls: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let inst = unsafe { &*inst };
        let cls = unsafe { &*cls };
        inst.is_instance(cls, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_Size(obj: *mut PyObject) -> isize {
    with_vm(|vm| {
        let obj = unsafe { &*obj };
        obj.length(vm)
    })
}
