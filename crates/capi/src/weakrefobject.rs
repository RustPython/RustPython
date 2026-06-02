use crate::PyObject;
use crate::object::define_py_check;
use crate::pystate::with_vm;
use core::ffi::c_int;
use rustpython_vm::builtins::{PyWeak, PyWeakProxy};

define_py_check!(fn PyWeakref_CheckProxy, types.weakproxy_type);
define_py_check!(fn PyWeakref_CheckRef, types.weakref_type);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyWeakref_GetRef(
    reference: *mut PyObject,
    result: *mut *mut PyObject,
) -> c_int {
    with_vm(|vm| {
        unsafe {
            *result = core::ptr::null_mut();
        }

        let reference = unsafe { &*reference };
        let upgraded = if let Some(weak) = reference.downcast_ref::<PyWeak>() {
            weak.upgrade()
        } else if let Some(proxy) = reference.downcast_ref::<PyWeakProxy>() {
            proxy.get_weak().upgrade()
        } else {
            return Err(vm.new_type_error("expected a weakref"));
        };

        if let Some(obj) = upgraded {
            unsafe {
                *result = obj.into_raw().as_ptr();
            }
            Ok(true)
        } else {
            Ok(false)
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyWeakref_NewProxy(
    ob: *mut PyObject,
    callback: *mut PyObject,
) -> *mut PyObject {
    with_vm(|vm| {
        let ob = unsafe { &*ob };
        let callback = unsafe { callback.as_ref() }
            .filter(|callback| !vm.is_none(callback))
            .map(ToOwned::to_owned);
        PyWeakProxy::new_weakproxy(ob, callback, vm)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyWeakref_NewRef(
    ob: *mut PyObject,
    callback: *mut PyObject,
) -> *mut PyObject {
    with_vm(|vm| {
        let ob = unsafe { &*ob };
        let callback = unsafe { callback.as_ref() }
            .filter(|callback| !vm.is_none(callback))
            .map(ToOwned::to_owned);
        ob.downgrade(callback, vm)
    })
}

#[cfg(false)]
mod tests {
    use pyo3::prelude::*;
    use pyo3::types::PyAnyMethods;
    use pyo3::types::{PyInt, PyWeakrefMethods, PyWeakrefProxy, PyWeakrefReference};

    #[test]
    fn check_ref_and_proxy() {
        Python::attach(|py| {
            let object_ty = py.get_type::<PyInt>();

            let weak_ref = PyWeakrefReference::new(&object_ty).unwrap();
            let weak_proxy = PyWeakrefProxy::new(&object_ty).unwrap();

            assert!(weak_ref.is_instance_of::<PyWeakrefReference>());
            assert!(weak_proxy.is_instance_of::<PyWeakrefProxy>());
        });
    }

    #[test]
    fn new_ref_and_get_ref() {
        Python::attach(|py| {
            let object_ty = py.get_type::<PyInt>();
            let weak_ref = PyWeakrefReference::new(&object_ty).unwrap();

            assert!(weak_ref.upgrade().is_some_and(|obj| obj.is(&object_ty)));
        });
    }

    #[test]
    fn new_proxy() {
        Python::attach(|py| {
            let object_ty = py.get_type::<PyInt>();
            let weak_proxy = PyWeakrefProxy::new(&object_ty).unwrap();

            assert!(weak_proxy.upgrade().is_some_and(|obj| obj.is(&object_ty)));
        });
    }
}
