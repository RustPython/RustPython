use crate::PyObject;
use crate::object::PyTypeObject;
use rustpython_vm::Py;
use rustpython_vm::builtins::PyType;
use std::alloc::{Layout, alloc_zeroed, dealloc};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

#[repr(C)]
struct CApiObjectHeader {
    ob_refcnt: isize,
    ob_type: *mut PyTypeObject,
}

#[derive(Default)]
struct WrapperMaps {
    inner_to_wrapper: HashMap<usize, usize>,
    wrapper_to_inner: HashMap<usize, usize>,
}

fn wrapper_maps() -> &'static Mutex<WrapperMaps> {
    static WRAPPER_MAPS: OnceLock<Mutex<WrapperMaps>> = OnceLock::new();
    WRAPPER_MAPS.get_or_init(|| Mutex::new(WrapperMaps::default()))
}

#[inline]
fn normalize_type_ptr(ptr: *mut PyTypeObject) -> *mut PyTypeObject {
    ptr.map_addr(|addr| addr & !1)
}

unsafe fn create_wrapper(actual: *mut PyObject, min_size: usize) -> *mut PyObject {
    let header_size = core::mem::size_of::<CApiObjectHeader>();
    let size = min_size.max(header_size);
    let align = core::mem::align_of::<CApiObjectHeader>();
    let layout = Layout::from_size_align(size, align).expect("valid wrapper layout");
    let wrapper = unsafe { alloc_zeroed(layout) };
    if wrapper.is_null() {
        return core::ptr::null_mut();
    }

    let actual_type = unsafe { (*actual).class() as *const Py<PyType> as *mut PyTypeObject };
    let wrapper = wrapper.cast::<CApiObjectHeader>();
    unsafe {
        (*wrapper).ob_refcnt = 1;
        (*wrapper).ob_type = exported_type_handle(actual_type);
    }

    let wrapper_ptr = wrapper.cast::<PyObject>();
    unsafe {
        core::mem::forget((&*actual).to_owned());
    }
    let mut maps = wrapper_maps().lock().unwrap();
    maps.inner_to_wrapper
        .insert(actual as usize, wrapper_ptr as usize);
    maps.wrapper_to_inner
        .insert(wrapper_ptr as usize, actual as usize);
    wrapper_ptr
}

pub(crate) unsafe fn exported_object_wrapper(
    actual: *mut PyObject,
    min_size: usize,
) -> *mut PyObject {
    let maps = wrapper_maps().lock().unwrap();
    if let Some(wrapper) = maps.inner_to_wrapper.get(&(actual as usize)).copied() {
        wrapper as *mut PyObject
    } else {
        drop(maps);
        unsafe { create_wrapper(actual, min_size) }
    }
}

pub(crate) unsafe fn wrapper_refcnt(op: *mut PyObject) -> Option<isize> {
    let maps = wrapper_maps().lock().unwrap();
    maps.wrapper_to_inner
        .contains_key(&(op as usize))
        .then(|| unsafe { (*(op as *mut CApiObjectHeader)).ob_refcnt })
}

pub(crate) unsafe fn incref_wrapper(op: *mut PyObject) -> bool {
    let maps = wrapper_maps().lock().unwrap();
    if !maps.wrapper_to_inner.contains_key(&(op as usize)) {
        return false;
    }
    drop(maps);
    unsafe {
        let header = op as *mut CApiObjectHeader;
        (*header).ob_refcnt += 1;
    }
    true
}

pub(crate) unsafe fn decref_wrapper(op: *mut PyObject) -> bool {
    let inner = {
        let maps = wrapper_maps().lock().unwrap();
        let Some(inner) = maps.wrapper_to_inner.get(&(op as usize)).copied() else {
            return false;
        };
        inner as *mut PyObject
    };

    let should_free = unsafe {
        let header = op as *mut CApiObjectHeader;
        (*header).ob_refcnt -= 1;
        (*header).ob_refcnt == 0
    };

    if should_free {
        let mut maps = wrapper_maps().lock().unwrap();
        maps.wrapper_to_inner.remove(&(op as usize));
        maps.inner_to_wrapper.remove(&(inner as usize));
        drop(maps);

        unsafe {
            drop((&*inner).to_owned());
        }

        let header_size = core::mem::size_of::<CApiObjectHeader>();
        let align = core::mem::align_of::<CApiObjectHeader>();
        let layout = Layout::from_size_align(header_size, align).expect("valid wrapper layout");
        unsafe { dealloc(op.cast::<u8>(), layout) };
    }

    true
}

#[inline]
pub(crate) unsafe fn exported_type_handle(actual: *mut PyTypeObject) -> *mut PyTypeObject {
    normalize_type_ptr(actual)
}

#[inline]
pub(crate) unsafe fn resolve_type_handle(exported: *mut PyTypeObject) -> *mut PyTypeObject {
    normalize_type_ptr(exported)
}

#[inline]
pub(crate) unsafe fn exported_object_handle(actual: *mut PyObject) -> *mut PyObject {
    let maps = wrapper_maps().lock().unwrap();
    maps.inner_to_wrapper
        .get(&(actual as usize))
        .copied()
        .map(|wrapper| wrapper as *mut PyObject)
        .unwrap_or(actual)
}

#[inline]
pub(crate) unsafe fn resolve_object_handle(exported: *mut PyObject) -> *mut PyObject {
    let maps = wrapper_maps().lock().unwrap();
    maps.wrapper_to_inner
        .get(&(exported as usize))
        .copied()
        .map(|ptr| ptr as *mut PyObject)
        .unwrap_or(exported)
}
