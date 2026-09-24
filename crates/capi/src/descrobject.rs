use crate::PyObject;
use crate::methodobject::{PyMethodDef, build_method_def};
use crate::object::PyTypeObject;
use crate::pystate::with_vm;
use crate::util::{CStrExt, FfiPtrExt};
use core::ffi::{c_char, c_int, c_void};
use rustpython_vm::builtins::{
    DescriptorMemberDef, MemberAccess, MemberKind, PY_RELATIVE_OFFSET, PyDescriptorOwned, PyGetSet,
    PyMappingProxy, PyMemberDescriptor, PyType,
};
use rustpython_vm::common::lock::PyRwLock;
use rustpython_vm::function::PySetterValue;
use rustpython_vm::{Py, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine};

#[repr(C)]
pub struct PyGetSetDef {
    pub name: *const c_char,
    pub get:
        Option<unsafe extern "C" fn(slf: *mut PyObject, closure: *mut c_void) -> *mut PyObject>,
    pub set: Option<
        unsafe extern "C" fn(
            slf: *mut PyObject,
            value: *mut PyObject,
            closure: *mut c_void,
        ) -> c_int,
    >,
    pub doc: *const c_char,
    pub closure: *mut c_void,
}

impl PyGetSetDef {
    pub(crate) fn build(
        &self,
        ty: &'static Py<PyType>,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<PyGetSet>> {
        let name = unsafe { self.name.try_as_str(vm) }?;
        let closure = self.closure as usize;

        let descriptor = match (self.get, self.set) {
            (Some(get), Some(set)) => vm.ctx.new_static_getset(
                name,
                ty,
                move |obj: PyObjectRef, vm: &VirtualMachine| -> PyResult<PyObjectRef> {
                    unsafe {
                        get(obj.as_raw().cast_mut(), closure as *mut c_void).assume_owned_or_err(vm)
                    }
                },
                move |obj: PyObjectRef, value: PySetterValue, vm: &VirtualMachine| unsafe {
                    let closure = closure as *mut c_void;
                    let value = value.unwrap_or_none(vm);
                    let result = set(obj.as_raw().cast_mut(), value.as_raw().cast_mut(), closure);
                    if result == 0 {
                        Ok(())
                    } else {
                        Err(vm.take_raised_exception().unwrap_or_else(|| {
                            vm.new_system_error(
                                "C setter returned error but did not set an exception",
                            )
                        }))
                    }
                },
            ),
            (Some(get), None) => vm.ctx.new_readonly_getset(
                name,
                ty,
                move |obj: PyObjectRef, vm: &VirtualMachine| -> PyResult<PyObjectRef> {
                    unsafe {
                        get(obj.as_raw().cast_mut(), closure as *mut c_void).assume_owned_or_err(vm)
                    }
                },
            ),
            (None, Some(set)) => vm.ctx.new_static_getset(
                name,
                ty,
                move |_obj: PyObjectRef, vm: &VirtualMachine| -> PyResult<PyObjectRef> {
                    Err(vm.new_attribute_error("unreadable attribute"))
                },
                move |obj: PyObjectRef, value: PySetterValue, vm: &VirtualMachine| unsafe {
                    let closure = closure as *mut c_void;
                    let value = value.unwrap_or_none(vm);
                    let result = set(obj.as_raw().cast_mut(), value.as_raw().cast_mut(), closure);
                    if result == 0 {
                        Ok(())
                    } else {
                        Err(vm.take_raised_exception().unwrap_or_else(|| {
                            vm.new_system_error(
                                "C setter returned error but did not set an exception",
                            )
                        }))
                    }
                },
            ),
            (None, None) => vm.ctx.new_readonly_getset(
                name,
                ty,
                move |_obj: PyObjectRef, vm: &VirtualMachine| -> PyResult<PyObjectRef> {
                    Err(vm.new_attribute_error("unreadable attribute"))
                },
            ),
        };

        Ok(descriptor)
    }
}

#[repr(C)]
pub struct PyMemberDef {
    pub name: *const c_char,
    pub type_code: c_int,
    pub offset: isize,
    pub flags: c_int,
    pub doc: *const c_char,
}

impl PyMemberDef {
    pub(crate) fn build(
        &self,
        ty: &Py<PyType>,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<PyMemberDescriptor>> {
        let name = unsafe { self.name.try_as_str(vm) }?;
        let Some(kind) = MemberKind::from_i32(self.type_code) else {
            return Err(vm.new_system_error(format!(
                "PyDescr_NewMember does not support member type code {}",
                self.type_code
            )));
        };
        let mut offset = self.offset;
        let mut flags = self.flags;
        if flags & PY_RELATIVE_OFFSET != 0 {
            // type creation adds tp_basicsize and clears the flag before GetOne.
            offset += (rustpython_vm::object::SIZEOF_PYOBJECT_HEAD + ty.slots.basicsize) as isize;
            flags &= !PY_RELATIVE_OFFSET;
        }

        let doc = unsafe { self.doc.try_as_str_opt(vm) }?.map(str::to_owned);

        let descriptor = PyMemberDescriptor {
            common: PyDescriptorOwned {
                typ: ty.to_owned(),
                name: vm.ctx.intern_str(name),
                qualname: PyRwLock::new(None),
            },
            member: DescriptorMemberDef {
                name: name.to_owned(),
                kind,
                offset,
                flags,
                doc,
            },
            // `offset` is a byte offset from the object to a pointer cell.
            access: MemberAccess::Slot,
        };

        Ok(descriptor.into_ref(&vm.ctx))
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDictProxy_New(mapping: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let mapping = unsafe { mapping.assume_borrowed() }.to_owned();
        Ok(PyMappingProxy::from_object(mapping, vm)?.into_ref(&vm.ctx))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDescr_NewMethod(
    typ: *mut PyTypeObject,
    method: *mut PyMethodDef,
) -> *mut PyObject {
    with_vm(|vm| {
        let method = build_method_def(vm, unsafe { &*method }, true)?;
        Ok(method.build_method(unsafe { typ.assume_borrowed() }, vm))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDescr_NewClassMethod(
    typ: *mut PyTypeObject,
    method: *mut PyMethodDef,
) -> *mut PyObject {
    with_vm(|vm| {
        let method = build_method_def(vm, unsafe { &*method }, true)?;
        Ok(method.build_method(unsafe { typ.assume_borrowed() }, vm))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDescr_NewGetSet(
    typ: *mut PyTypeObject,
    getset: *mut PyGetSetDef,
) -> *mut PyObject {
    with_vm(|vm| unsafe { &*getset }.build(unsafe { typ.assume_borrowed() }, vm))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDescr_NewMember(
    typ: *mut PyTypeObject,
    member: *mut PyMemberDef,
) -> *mut PyObject {
    with_vm(|vm| Ok(unsafe { &*member }.build(unsafe { typ.assume_borrowed() }, vm)))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyWrapper_New(descr: *mut PyObject, obj: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let descr = unsafe { descr.assume_borrowed() };
        let obj = unsafe { obj.assume_borrowed() };
        vm.call_special_method(
            descr,
            vm.ctx.names.__get__,
            (obj.to_owned(), obj.class().to_owned()),
        )
    })
}

#[cfg(test)]
mod tests {
    use pyo3::prelude::*;
    use pyo3::types::{PyDict, PyInt, PyMappingProxy};

    #[test]
    fn proxy_reads_items() {
        Python::attach(|py| {
            let dict = PyDict::new(py);
            dict.set_item("x", 7).unwrap();

            let mapping = dict.as_mapping();
            let proxy = PyMappingProxy::new(py, mapping);
            let value = proxy.get_item("x").unwrap().cast_into::<PyInt>().unwrap();
            assert_eq!(value, 7);
        })
    }
}
