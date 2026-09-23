use crate::util::CStrExt;
use crate::util::FfiPtrExt;
use crate::{PyObject, pystate::with_vm};
use alloc::slice;
use core::ffi::{c_char, c_int};
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
            // `to_string()` would replace lone surrogates with U+FFFD; keep the
            // raw WTF-8 so surrogate keys round-trip (issue #8228).
            let key = key
                .downcast_ref::<PyStr>()
                .map(|s| s.as_wtf8().to_owned())
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
        let callable = unsafe { callable.assume_borrowed() };
        let args = tuple_to_args(unsafe { args.assume_borrowed_and_cast::<PyTuple>(vm) }?);

        let kwargs: Option<KwArgs> = unsafe { kwargs.assume_borrowed_or_opt() }
            .map(|kwargs| dict_to_kwargs(vm, kwargs.try_downcast_ref::<PyDict>(vm)?))
            .transpose()?;

        callable.call_with_args(FuncArgs::new(args, kwargs.unwrap_or_default()), vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_CallNoArgs(callable: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| unsafe { callable.assume_borrowed() }.call((), vm))
}

fn call_method_obj_args(
    obj: *mut PyObject,
    name: *mut PyObject,
    args: &[*mut PyObject],
) -> *mut PyObject {
    with_vm(|vm| {
        if obj.is_null() || name.is_null() {
            return Err(vm.new_system_error("null argument"));
        }
        let obj = unsafe { &*obj };
        let name = unsafe { &*name }.try_downcast_ref::<PyStr>(vm)?;
        let method = obj.get_attr(name, vm)?;
        let mut posargs = Vec::with_capacity(args.len());
        for arg in args {
            if arg.is_null() {
                return Err(vm.new_system_error("null argument"));
            }
            posargs.push(unsafe { &**arg }.to_owned());
        }
        method.call(posargs, vm)
    })
}

/// Read `NULL`-terminated `*mut PyObject` slots starting at `args`.
///
/// # Safety
/// `args` must point at a C variadic tail of object pointers ended by NULL.
unsafe fn args_until_null(args: *const *mut PyObject) -> Vec<*mut PyObject> {
    let mut collected = Vec::new();
    let mut cursor = args;
    loop {
        let arg = unsafe { *cursor };
        if arg.is_null() {
            break;
        }
        collected.push(arg);
        cursor = unsafe { cursor.add(1) };
    }
    collected
}

/// Apple ARM64 passes the variadic tail on the stack. This arm reads the
/// caller's `sp`. Other AAPCS64 targets pass it in x2..x7 and then on the
/// stack; that arm forwards those registers and the entry `sp`.
#[cfg(all(target_arch = "aarch64", target_vendor = "apple"))]
#[unsafe(naked)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_CallMethodObjArgs() {
    core::arch::naked_asm!(
        "stp x29, x30, [sp, #-16]!",
        "mov x29, sp",
        "add x2, x29, #16",
        "bl {helper}",
        "ldp x29, x30, [sp], #16",
        "ret",
        helper = sym py_object_call_method_obj_args_stack,
    );
}

#[cfg(all(target_arch = "aarch64", target_vendor = "apple"))]
unsafe extern "C" fn py_object_call_method_obj_args_stack(
    obj: *mut PyObject,
    name: *mut PyObject,
    args: *const *mut PyObject,
) -> *mut PyObject {
    let collected = unsafe { args_until_null(args) };
    call_method_obj_args(obj, name, &collected)
}

/// Apple ARM64 passes the variadic tail on the stack. That arm reads the
/// caller's `sp`. This arm is every other AAPCS64 target: x2..x7, then the
/// stack, forwarded as the ninth parameter (the entry `sp`).
#[cfg(all(target_arch = "aarch64", not(target_vendor = "apple")))]
#[unsafe(naked)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_CallMethodObjArgs() {
    core::arch::naked_asm!(
        "stp x29, x30, [sp, #-16]!",
        "mov x29, sp",
        "sub sp, sp, #16",
        "add x9, x29, #16",
        "str x9, [sp]",
        "bl {helper}",
        "add sp, sp, #16",
        "ldp x29, x30, [sp], #16",
        "ret",
        helper = sym py_object_call_method_obj_args_aapcs,
    );
}

#[cfg(all(target_arch = "aarch64", not(target_vendor = "apple")))]
#[allow(clippy::too_many_arguments)]
unsafe extern "C" fn py_object_call_method_obj_args_aapcs(
    obj: *mut PyObject,
    name: *mut PyObject,
    a: *mut PyObject,
    b: *mut PyObject,
    c: *mut PyObject,
    d: *mut PyObject,
    e: *mut PyObject,
    f: *mut PyObject,
    rest: *const *mut PyObject,
) -> *mut PyObject {
    let mut collected = Vec::new();
    for arg in [a, b, c, d, e, f] {
        if arg.is_null() {
            return call_method_obj_args(obj, name, &collected);
        }
        collected.push(arg);
    }
    collected.extend(unsafe { args_until_null(rest) });
    call_method_obj_args(obj, name, &collected)
}

#[cfg(all(target_arch = "x86_64", not(target_os = "windows")))]
#[unsafe(naked)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_CallMethodObjArgs() {
    core::arch::naked_asm!(
        "push rbp",
        "mov rbp, rsp",
        "lea rax, [rbp + 16]",
        "sub rsp, 16",
        "mov [rsp], rax",
        "call {helper}",
        "add rsp, 16",
        "pop rbp",
        "ret",
        helper = sym py_object_call_method_obj_args_regs,
    );
}

#[cfg(all(target_arch = "x86_64", target_os = "windows"))]
#[unsafe(naked)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_CallMethodObjArgs() {
    core::arch::naked_asm!(
        "push rbp",
        "mov rbp, rsp",
        "lea rax, [rbp + 48]",
        "sub rsp, 48",
        "mov [rsp + 32], rax",
        "call {helper}",
        "add rsp, 48",
        "pop rbp",
        "ret",
        helper = sym py_object_call_method_obj_args_win,
    );
}

#[cfg(all(target_arch = "x86_64", target_os = "windows"))]
unsafe extern "C" fn py_object_call_method_obj_args_win(
    obj: *mut PyObject,
    name: *mut PyObject,
    a: *mut PyObject,
    b: *mut PyObject,
    rest: *const *mut PyObject,
) -> *mut PyObject {
    let mut collected = Vec::new();
    for arg in [a, b] {
        if arg.is_null() {
            return call_method_obj_args(obj, name, &collected);
        }
        collected.push(arg);
    }
    collected.extend(unsafe { args_until_null(rest) });
    call_method_obj_args(obj, name, &collected)
}

#[cfg(all(target_arch = "x86_64", not(target_os = "windows")))]
unsafe extern "C" fn py_object_call_method_obj_args_regs(
    obj: *mut PyObject,
    name: *mut PyObject,
    a: *mut PyObject,
    b: *mut PyObject,
    c: *mut PyObject,
    d: *mut PyObject,
    rest: *const *mut PyObject,
) -> *mut PyObject {
    let mut collected = Vec::new();
    for arg in [a, b, c, d] {
        if arg.is_null() {
            return call_method_obj_args(obj, name, &collected);
        }
        collected.push(arg);
    }
    collected.extend(unsafe { args_until_null(rest) });
    call_method_obj_args(obj, name, &collected)
}

#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_CallMethodObjArgs(
    obj: *mut PyObject,
    name: *mut PyObject,
) -> *mut PyObject {
    call_method_obj_args(obj, name, &[])
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_CallObject(
    callable: *mut PyObject,
    args: *mut PyObject,
) -> *mut PyObject {
    with_vm(|vm| {
        let callable = unsafe { callable.assume_borrowed() };
        if let Some(args) = unsafe { args.assume_borrowed_or_opt() } {
            callable.call(tuple_to_args(args.try_downcast_ref::<PyTuple>(vm)?), vm)
        } else {
            callable.call((), vm)
        }
    })
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
                .assume_borrowed_or_opt()
                .map(|tuple| Ok(&***tuple.try_downcast_ref::<PyTuple>(vm)?))
                .transpose()?
        };

        let args_len = num_positional_args + kwnames.map_or(0, <[PyObjectRef]>::len);
        let args = if args_len == 0 {
            Vec::new()
        } else {
            unsafe { slice::from_raw_parts(args, args_len) }
                .iter()
                .map(|arg| unsafe { arg.assume_borrowed() }.to_owned())
                .collect::<Vec<_>>()
        };

        let callable = unsafe { callable.assume_borrowed() };
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

        let method_name = unsafe { name.assume_borrowed_and_cast::<PyStr>(vm)? };
        let callable = unsafe { receiver.assume_borrowed().get_attr(method_name, vm)? };

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
pub unsafe extern "C" fn PyVectorcall_Call(
    callable: *mut PyObject,
    tuple: *mut PyObject,
    kwargs: *mut PyObject,
) -> *mut PyObject {
    with_vm(|vm| {
        let callable = unsafe { callable.assume_borrowed() };
        let tuple = unsafe { tuple.assume_borrowed_and_cast::<PyTuple>(vm) }?;

        let mut args = tuple.iter().cloned().collect::<Vec<_>>();
        let num_positional_args = args.len();

        let mut kwnames = Vec::new();
        if let Some(kwargs) = unsafe { kwargs.assume_borrowed_or_opt() } {
            let kwargs = kwargs.try_downcast_ref::<PyDict>(vm)?;
            for (key, value) in kwargs.items_vec() {
                let key = key
                    .downcast_ref::<PyStr>()
                    .map(ToOwned::to_owned)
                    .ok_or_else(|| vm.new_type_error("keywords must be strings"))?;
                kwnames.push(key.into());
                args.push(value);
            }
        }

        let kwnames = if kwnames.is_empty() {
            None
        } else {
            Some(kwnames.as_slice())
        };

        callable.vectorcall(args, num_positional_args, kwnames, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GetItem(obj: *mut PyObject, key: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let obj = unsafe { obj.assume_borrowed() };
        let key = unsafe { key.assume_borrowed() };
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
        let obj = unsafe { obj.assume_borrowed() };
        let key = unsafe { key.assume_borrowed() };
        let value = unsafe { value.assume_borrowed() }.to_owned();
        obj.set_item(key, value, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_DelItem(obj: *mut PyObject, key: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let obj = unsafe { obj.assume_borrowed() };
        let key = unsafe { key.assume_borrowed() };
        obj.del_item(key, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_DelItemString(obj: *mut PyObject, key: *const c_char) -> c_int {
    with_vm(|vm| {
        let obj = unsafe { obj.assume_borrowed() };
        let key = unsafe { key.try_as_str(vm) }?;
        obj.del_item(key, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_Format(
    obj: *mut PyObject,
    format_spec: *mut PyObject,
) -> *mut PyObject {
    with_vm(|vm| {
        let obj = unsafe { obj.assume_borrowed() };
        let spec = unsafe { format_spec.assume_borrowed_or_opt() }
            .map(|spec| spec.try_downcast_ref::<PyStr>(vm))
            .transpose()?
            .unwrap_or_else(|| vm.ctx.empty_str);
        vm.format(obj, spec.to_owned())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_IsSubclass(derived: *mut PyObject, cls: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let derived = unsafe { derived.assume_borrowed() };
        let cls = unsafe { cls.assume_borrowed() };
        derived.is_subclass(cls, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_IsInstance(inst: *mut PyObject, cls: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let inst = unsafe { inst.assume_borrowed() };
        let cls = unsafe { cls.assume_borrowed() };
        inst.is_instance(cls, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_Size(obj: *mut PyObject) -> isize {
    with_vm(|vm| {
        let obj = unsafe { obj.assume_borrowed() };
        obj.length(vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_Length(obj: *mut PyObject) -> isize {
    unsafe { PyObject_Size(obj) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_Type(obj: *mut PyObject) -> *mut PyObject {
    with_vm(|_vm| unsafe { obj.assume_borrowed() }.obj_type())
}

#[cfg(test)]
mod tests {
    use pyo3::prelude::*;
    use pyo3::types::{PyDict, PyDictMethods, PyString};

    #[test]
    fn call_method_obj_args_variadic() {
        Python::attach(|py| {
            let locals = PyDict::new(py);
            py.run(
                c"class Box:
    def take(self, *args):
        return args
box = Box()
",
                None,
                Some(&locals),
            )
            .unwrap();
            let receiver = locals.get_item("box").unwrap().unwrap();
            let name = PyString::new(py, "take");
            let obj = receiver.as_ptr().cast::<crate::PyObject>();
            let name_ptr = name.as_ptr().cast::<crate::PyObject>();
            let call: unsafe extern "C" fn(
                *mut crate::PyObject,
                *mut crate::PyObject,
                ...
            ) -> *mut crate::PyObject =
                unsafe { core::mem::transmute(super::PyObject_CallMethodObjArgs as *const ()) };
            let end = core::ptr::null_mut::<crate::PyObject>();

            let got = |ret: *mut crate::PyObject| -> Vec<i64> {
                assert!(!ret.is_null());
                unsafe { Bound::from_owned_ptr(py, ret.cast()) }
                    .extract()
                    .unwrap()
            };

            let ret = unsafe { call(obj, name_ptr, end) };
            assert_eq!(got(ret), Vec::<i64>::new());

            let one = 1i64.into_pyobject(py).unwrap();
            let ret = unsafe { call(obj, name_ptr, one.as_ptr().cast::<crate::PyObject>(), end) };
            assert_eq!(got(ret), vec![1]);

            let values: Vec<_> = (1..=7i64).map(|i| i.into_pyobject(py).unwrap()).collect();
            let p = |i: usize| values[i].as_ptr().cast::<crate::PyObject>();
            let ret = unsafe { call(obj, name_ptr, p(0), p(1), p(2), p(3), p(4), p(5), p(6), end) };
            assert_eq!(got(ret), vec![1, 2, 3, 4, 5, 6, 7]);
        });
    }

    #[test]
    fn call_method1() {
        Python::attach(|py| {
            let string = PyString::new(py, "Hello, World!");
            assert!(
                string
                    .call_method1("endswith", ("!",))
                    .unwrap()
                    .is_truthy()
                    .unwrap()
            );
        })
    }

    #[test]
    fn object_set_get_del_item() {
        Python::attach(|py| {
            let obj = PyDict::new(py).into_any();
            obj.set_item("key", "value").unwrap();
            assert_eq!(
                obj.get_item("key")
                    .unwrap()
                    .cast_into::<PyString>()
                    .unwrap(),
                "value"
            );
            obj.del_item("key").unwrap();
            assert!(obj.get_item("key").is_err());
        })
    }
}
