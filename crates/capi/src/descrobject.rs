use crate::PyObject;
use crate::methodobject::{PyMethodDef, build_method_def};
use crate::object::PyTypeObject;
use crate::pystate::with_vm;
use core::ffi::{CStr, c_char, c_int, c_void};
use core::ptr::NonNull;
use rustpython_vm::builtins::{
    DescriptorMemberDef, MemberGetter, MemberKind, MemberSetter, PyDescriptorOwned, PyMappingProxy,
    PyMemberDescriptor,
};
use rustpython_vm::common::lock::PyRwLock;
use rustpython_vm::function::PySetterValue;
use rustpython_vm::{PyObjectRef, PyPayload, PyResult};

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

#[repr(C)]
pub struct PyMemberDef {
    pub name: *const c_char,
    pub type_code: c_int,
    pub offset: isize,
    pub flags: c_int,
    pub doc: *const c_char,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDictProxy_New(mapping: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let mapping = unsafe { &*mapping }.to_owned();
        Ok(PyMappingProxy::from_object(mapping, vm)?.into_ref(&vm.ctx))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDescr_NewMethod(
    typ: *mut PyTypeObject,
    method: *mut PyMethodDef,
) -> *mut PyObject {
    with_vm(|vm| -> PyResult<PyObjectRef> {
        let method = build_method_def(vm, unsafe { &*method }, true)?;
        Ok(method.build_method(unsafe { &*typ }, vm).into())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDescr_NewClassMethod(
    typ: *mut PyTypeObject,
    method: *mut PyMethodDef,
) -> *mut PyObject {
    with_vm(|vm| -> PyResult<PyObjectRef> {
        let method = build_method_def(vm, unsafe { &*method }, true)?;
        Ok(method.build_method(unsafe { &*typ }, vm).into())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDescr_NewGetSet(
    typ: *mut PyTypeObject,
    getset: *mut PyGetSetDef,
) -> *mut PyObject {
    with_vm(|vm| -> PyResult<PyObjectRef> {
        let typ = unsafe { &*typ };
        let getset = unsafe { &*getset };
        let name = unsafe { CStr::from_ptr(getset.name) }
            .to_str()
            .map_err(|_| vm.new_system_error("PyGetSetDef name was not valid UTF-8"))?;

        let descriptor = match (getset.get, getset.set) {
            (Some(get), Some(set)) => {
                let closure = getset.closure as usize;
                vm.ctx.new_static_getset(
                    name,
                    typ,
                    move |obj: PyObjectRef,
                          vm: &rustpython_vm::VirtualMachine|
                          -> PyResult<PyObjectRef> {
                        unsafe {
                            let closure = closure as *mut c_void;
                            let ret_ptr = get(obj.as_raw().cast_mut(), closure);
                            let ret_ptr = NonNull::new(ret_ptr).ok_or_else(|| {
                                vm.take_raised_exception().expect(
                                    "Native function returned NULL, but there was no exception set",
                                )
                            })?;
                            Ok(PyObjectRef::from_raw(ret_ptr))
                        }
                    },
                    move |obj: PyObjectRef,
                          value: PySetterValue,
                          vm: &rustpython_vm::VirtualMachine| unsafe {
                        let closure = closure as *mut c_void;
                        let value = value.unwrap_or_none(vm);
                        let result =
                            set(obj.as_raw().cast_mut(), value.as_raw().cast_mut(), closure);
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
                )
            }
            (Some(get), None) => {
                let closure = getset.closure as usize;
                vm.ctx.new_readonly_getset(
                    name,
                    typ,
                    move |obj: PyObjectRef,
                          vm: &rustpython_vm::VirtualMachine|
                          -> PyResult<PyObjectRef> {
                        unsafe {
                            let closure = closure as *mut c_void;
                            let ret_ptr = get(obj.as_raw().cast_mut(), closure);
                            let ret_ptr = NonNull::new(ret_ptr).ok_or_else(|| {
                                vm.take_raised_exception().expect(
                                    "Native function returned NULL, but there was no exception set",
                                )
                            })?;
                            Ok(PyObjectRef::from_raw(ret_ptr))
                        }
                    },
                )
            }
            (None, Some(set)) => {
                let closure = getset.closure as usize;
                vm.ctx.new_static_getset(
                    name,
                    typ,
                    move |_obj: PyObjectRef,
                          vm: &rustpython_vm::VirtualMachine|
                          -> PyResult<PyObjectRef> {
                        Err(vm.new_attribute_error("unreadable attribute"))
                    },
                    move |obj: PyObjectRef,
                          value: PySetterValue,
                          vm: &rustpython_vm::VirtualMachine| unsafe {
                        let closure = closure as *mut c_void;
                        let value = value.unwrap_or_none(vm);
                        let result =
                            set(obj.as_raw().cast_mut(), value.as_raw().cast_mut(), closure);
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
                )
            }
            (None, None) => vm.ctx.new_readonly_getset(
                name,
                typ,
                move |_obj: PyObjectRef,
                      vm: &rustpython_vm::VirtualMachine|
                      -> PyResult<PyObjectRef> {
                    Err(vm.new_attribute_error("unreadable attribute"))
                },
            ),
        };

        Ok(descriptor.into())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyDescr_NewMember(
    typ: *mut PyTypeObject,
    member: *mut PyMemberDef,
) -> *mut PyObject {
    const PY_READONLY: c_int = 1;
    const PY_RELATIVE_OFFSET: c_int = 8;

    with_vm(|vm| -> PyResult<PyObjectRef> {
        let typ = unsafe { &*typ };
        let member = unsafe { &*member };
        let name = unsafe { CStr::from_ptr(member.name) }
            .to_str()
            .map_err(|_| vm.new_system_error("PyMemberDef name was not valid UTF-8"))?;
        let kind = match member.type_code {
            6 => MemberKind::Object,
            16 => MemberKind::ObjectEx,
            14 => MemberKind::Bool,
            _ => {
                return Err(vm.new_system_error(format!(
                    "PyDescr_NewMember does not support member type code {}",
                    member.type_code
                )));
            }
        };
        if member.offset < 0 {
            return Err(vm.new_system_error("PyDescr_NewMember does not support negative offsets"));
        }
        if member.flags & PY_RELATIVE_OFFSET != 0 {
            return Err(
                vm.new_system_error("PyDescr_NewMember does not support Py_RELATIVE_OFFSET")
            );
        }

        let doc = NonNull::new(member.doc.cast_mut())
            .map(|doc| {
                unsafe { CStr::from_ptr(doc.as_ptr()) }
                    .to_str()
                    .map(|s| s.to_owned())
                    .map_err(|_| vm.new_system_error("PyMemberDef doc was not valid UTF-8"))
            })
            .transpose()?;

        let descriptor = PyMemberDescriptor {
            common: PyDescriptorOwned {
                typ: typ.to_owned(),
                name: vm.ctx.intern_str(name),
                qualname: PyRwLock::new(None),
            },
            member: DescriptorMemberDef {
                name: name.to_owned(),
                kind,
                getter: MemberGetter::Offset(member.offset as usize),
                setter: if member.flags & PY_READONLY != 0 {
                    MemberSetter::Setter(None)
                } else {
                    MemberSetter::Offset(member.offset as usize)
                },
                doc,
            },
        };

        Ok(descriptor.into_ref(&vm.ctx).into())
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyWrapper_New(descr: *mut PyObject, obj: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let descr = unsafe { &*descr };
        let obj = unsafe { &*obj };
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
