use crate::PyObject;
use crate::object::PyTypeObject;
use rustpython_vm::AsObject;
use rustpython_vm::Py;
use rustpython_vm::builtins::PyType;
use rustpython_vm::vm::Context;
use std::alloc::{Layout, alloc_zeroed, dealloc};
use std::collections::HashMap;
use std::ptr;
use std::sync::{Mutex, OnceLock};

#[repr(C)]
struct CApiObjectHeader {
    ob_refcnt: isize,
    ob_type: *mut PyTypeObject,
    alloc_size: usize,
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

unsafe fn create_wrapper(actual: *mut PyObject, min_size: usize) -> *mut PyObject {
    let header_size = core::mem::size_of::<CApiObjectHeader>();
    let size = min_size.max(header_size);
    let layout = Layout::from_size_align(size, core::mem::align_of::<CApiObjectHeader>()).unwrap();
    let wrapper = unsafe { alloc_zeroed(layout) }.cast::<CApiObjectHeader>();
    assert!(!wrapper.is_null(), "wrapper allocation failed");

    let actual_type = unsafe { (*actual).class() } as *const Py<PyType> as *mut PyTypeObject;
    unsafe {
        (*wrapper).ob_refcnt = 1;
        (*wrapper).ob_type = exported_type_handle(actual_type);
        (*wrapper).alloc_size = size;
    }

    let wrapper_ptr = wrapper.cast::<PyObject>();
    // The wrapper owns one strong reference to the underlying RustPython object.
    // We intentionally leak this reference for now; wrappers themselves are also leaked
    // in this prototype, so this keeps the mapping valid for the wrapper's lifetime.
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
    let mut maps = wrapper_maps().lock().unwrap();
    if let Some(wrapper) = maps.inner_to_wrapper.get(&(actual as usize)).copied() {
        wrapper as *mut PyObject
    } else {
        drop(maps);
        unsafe { create_wrapper(actual, min_size) }
    }
}

#[inline]
pub(crate) unsafe fn wrapper_refcnt(op: *mut PyObject) -> Option<isize> {
    let maps = wrapper_maps().lock().unwrap();
    maps.wrapper_to_inner
        .contains_key(&(op as usize))
        .then(|| unsafe { (*(op as *mut CApiObjectHeader)).ob_refcnt })
}

#[inline]
pub(crate) unsafe fn incref_wrapper(op: *mut PyObject) -> bool {
    let maps = wrapper_maps().lock().unwrap();
    if maps.wrapper_to_inner.contains_key(&(op as usize)) {
        unsafe {
            (*(op as *mut CApiObjectHeader)).ob_refcnt += 1;
        }
        true
    } else {
        false
    }
}

#[inline]
pub(crate) unsafe fn decref_wrapper(op: *mut PyObject) -> bool {
    let maybe_inner = {
        let mut maps = wrapper_maps().lock().unwrap();
        let Some(inner) = maps.wrapper_to_inner.get(&(op as usize)).copied() else {
            return false;
        };

        let header = unsafe { &mut *(op as *mut CApiObjectHeader) };
        header.ob_refcnt -= 1;
        if header.ob_refcnt > 0 {
            return true;
        }

        maps.wrapper_to_inner.remove(&(op as usize));
        maps.inner_to_wrapper.remove(&inner);
        Some((inner as *mut PyObject, header.alloc_size))
    };

    if let Some((inner, alloc_size)) = maybe_inner {
        drop(unsafe { (&*inner).to_owned() });
        let layout =
            Layout::from_size_align(alloc_size, core::mem::align_of::<CApiObjectHeader>()).unwrap();
        unsafe {
            dealloc(op.cast::<u8>(), layout);
        }
    }
    true
}

#[inline]
fn normalize_type_ptr(ptr: *mut PyTypeObject) -> *mut PyTypeObject {
    ptr.map_addr(|addr| addr & !1)
}

#[unsafe(export_name = "PyBaseObject_Type")]
pub static mut PYBASEOBJECT_TYPE_HANDLE: *mut PyTypeObject = ptr::null_mut();
#[unsafe(export_name = "PyBool_Type")]
pub static mut PYBOOL_TYPE_HANDLE: *mut PyTypeObject = ptr::null_mut();
#[unsafe(export_name = "PyByteArray_Type")]
pub static mut PYBYTEARRAY_TYPE_HANDLE: *mut PyTypeObject = ptr::null_mut();
#[unsafe(export_name = "PyBytes_Type")]
pub static mut PYBYTES_TYPE_HANDLE: *mut PyTypeObject = ptr::null_mut();
#[unsafe(export_name = "PyDict_Type")]
pub static mut PYDICT_TYPE_HANDLE: *mut PyTypeObject = ptr::null_mut();
#[unsafe(export_name = "PyList_Type")]
pub static mut PYLIST_TYPE_HANDLE: *mut PyTypeObject = ptr::null_mut();
#[unsafe(export_name = "PyLong_Type")]
pub static mut PYLONG_TYPE_HANDLE: *mut PyTypeObject = ptr::null_mut();
#[unsafe(export_name = "PyModule_Type")]
pub static mut PYMODULE_TYPE_HANDLE: *mut PyTypeObject = ptr::null_mut();
#[unsafe(export_name = "PyTuple_Type")]
pub static mut PYTUPLE_TYPE_HANDLE: *mut PyTypeObject = ptr::null_mut();
#[unsafe(export_name = "PyType_Type")]
pub static mut PYTYPE_TYPE_HANDLE: *mut PyTypeObject = ptr::null_mut();
#[unsafe(export_name = "PyUnicode_Type")]
pub static mut PYUNICODE_TYPE_HANDLE: *mut PyTypeObject = ptr::null_mut();

#[unsafe(export_name = "_Py_NoneStruct")]
pub static mut PYNONESTRUCT_HANDLE: *mut PyObject = ptr::null_mut();
#[unsafe(export_name = "_Py_FalseStruct")]
pub static mut PYFALSESTRUCT_HANDLE: *mut PyObject = ptr::null_mut();
#[unsafe(export_name = "_Py_TrueStruct")]
pub static mut PYTRUESTRUCT_HANDLE: *mut PyObject = ptr::null_mut();

#[allow(static_mut_refs)]
pub(crate) unsafe fn init_symbol_handles(ctx: &Context) {
    unsafe {
        PYBASEOBJECT_TYPE_HANDLE =
            normalize_type_ptr(ctx.types.object_type as *const Py<PyType> as *mut PyTypeObject);
        PYBOOL_TYPE_HANDLE =
            normalize_type_ptr(ctx.types.bool_type as *const Py<PyType> as *mut PyTypeObject);
        PYBYTEARRAY_TYPE_HANDLE =
            normalize_type_ptr(ctx.types.bytearray_type as *const Py<PyType> as *mut PyTypeObject);
        PYBYTES_TYPE_HANDLE =
            normalize_type_ptr(ctx.types.bytes_type as *const Py<PyType> as *mut PyTypeObject);
        PYDICT_TYPE_HANDLE =
            normalize_type_ptr(ctx.types.dict_type as *const Py<PyType> as *mut PyTypeObject);
        PYLIST_TYPE_HANDLE =
            normalize_type_ptr(ctx.types.list_type as *const Py<PyType> as *mut PyTypeObject);
        PYLONG_TYPE_HANDLE =
            normalize_type_ptr(ctx.types.int_type as *const Py<PyType> as *mut PyTypeObject);
        PYMODULE_TYPE_HANDLE =
            normalize_type_ptr(ctx.types.module_type as *const Py<PyType> as *mut PyTypeObject);
        PYTUPLE_TYPE_HANDLE =
            normalize_type_ptr(ctx.types.tuple_type as *const Py<PyType> as *mut PyTypeObject);
        PYTYPE_TYPE_HANDLE =
            normalize_type_ptr(ctx.types.type_type as *const Py<PyType> as *mut PyTypeObject);
        PYUNICODE_TYPE_HANDLE =
            normalize_type_ptr(ctx.types.str_type as *const Py<PyType> as *mut PyTypeObject);

        PYNONESTRUCT_HANDLE = ctx.none.as_object().as_raw().cast_mut();
        PYFALSESTRUCT_HANDLE = ctx.false_value.as_object().as_raw().cast_mut();
        PYTRUESTRUCT_HANDLE = ctx.true_value.as_object().as_raw().cast_mut();
    }
}

#[inline]
pub(crate) unsafe fn exported_type_handle(actual: *mut PyTypeObject) -> *mut PyTypeObject {
    let actual = normalize_type_ptr(actual);
    unsafe {
        if actual == PYBASEOBJECT_TYPE_HANDLE {
            ptr::addr_of_mut!(PYBASEOBJECT_TYPE_HANDLE).cast()
        } else if actual == PYBOOL_TYPE_HANDLE {
            ptr::addr_of_mut!(PYBOOL_TYPE_HANDLE).cast()
        } else if actual == PYBYTEARRAY_TYPE_HANDLE {
            ptr::addr_of_mut!(PYBYTEARRAY_TYPE_HANDLE).cast()
        } else if actual == PYBYTES_TYPE_HANDLE {
            ptr::addr_of_mut!(PYBYTES_TYPE_HANDLE).cast()
        } else if actual == PYDICT_TYPE_HANDLE {
            ptr::addr_of_mut!(PYDICT_TYPE_HANDLE).cast()
        } else if actual == PYLIST_TYPE_HANDLE {
            ptr::addr_of_mut!(PYLIST_TYPE_HANDLE).cast()
        } else if actual == PYLONG_TYPE_HANDLE {
            ptr::addr_of_mut!(PYLONG_TYPE_HANDLE).cast()
        } else if actual == PYMODULE_TYPE_HANDLE {
            ptr::addr_of_mut!(PYMODULE_TYPE_HANDLE).cast()
        } else if actual == PYTUPLE_TYPE_HANDLE {
            ptr::addr_of_mut!(PYTUPLE_TYPE_HANDLE).cast()
        } else if actual == PYTYPE_TYPE_HANDLE {
            ptr::addr_of_mut!(PYTYPE_TYPE_HANDLE).cast()
        } else if actual == PYUNICODE_TYPE_HANDLE {
            ptr::addr_of_mut!(PYUNICODE_TYPE_HANDLE).cast()
        } else {
            actual
        }
    }
}

#[inline]
pub(crate) unsafe fn resolve_type_handle(exported: *mut PyTypeObject) -> *mut PyTypeObject {
    let exported = normalize_type_ptr(exported);
    unsafe {
        if exported == ptr::addr_of_mut!(PYBASEOBJECT_TYPE_HANDLE).cast() {
            normalize_type_ptr(PYBASEOBJECT_TYPE_HANDLE)
        } else if exported == ptr::addr_of_mut!(PYBOOL_TYPE_HANDLE).cast() {
            normalize_type_ptr(PYBOOL_TYPE_HANDLE)
        } else if exported == ptr::addr_of_mut!(PYBYTEARRAY_TYPE_HANDLE).cast() {
            normalize_type_ptr(PYBYTEARRAY_TYPE_HANDLE)
        } else if exported == ptr::addr_of_mut!(PYBYTES_TYPE_HANDLE).cast() {
            normalize_type_ptr(PYBYTES_TYPE_HANDLE)
        } else if exported == ptr::addr_of_mut!(PYDICT_TYPE_HANDLE).cast() {
            normalize_type_ptr(PYDICT_TYPE_HANDLE)
        } else if exported == ptr::addr_of_mut!(PYLIST_TYPE_HANDLE).cast() {
            normalize_type_ptr(PYLIST_TYPE_HANDLE)
        } else if exported == ptr::addr_of_mut!(PYLONG_TYPE_HANDLE).cast() {
            normalize_type_ptr(PYLONG_TYPE_HANDLE)
        } else if exported == ptr::addr_of_mut!(PYMODULE_TYPE_HANDLE).cast() {
            normalize_type_ptr(PYMODULE_TYPE_HANDLE)
        } else if exported == ptr::addr_of_mut!(PYTUPLE_TYPE_HANDLE).cast() {
            normalize_type_ptr(PYTUPLE_TYPE_HANDLE)
        } else if exported == ptr::addr_of_mut!(PYTYPE_TYPE_HANDLE).cast() {
            normalize_type_ptr(PYTYPE_TYPE_HANDLE)
        } else if exported == ptr::addr_of_mut!(PYUNICODE_TYPE_HANDLE).cast() {
            normalize_type_ptr(PYUNICODE_TYPE_HANDLE)
        } else {
            normalize_type_ptr(exported)
        }
    }
}

#[inline]
pub(crate) unsafe fn exported_object_handle(actual: *mut PyObject) -> *mut PyObject {
    let actual_type = normalize_type_ptr(actual.cast());
    let is_type_object = actual == actual_type.cast();
    unsafe {
        if actual == PYNONESTRUCT_HANDLE {
            ptr::addr_of_mut!(PYNONESTRUCT_HANDLE).cast()
        } else if actual == PYFALSESTRUCT_HANDLE {
            ptr::addr_of_mut!(PYFALSESTRUCT_HANDLE).cast()
        } else if actual == PYTRUESTRUCT_HANDLE {
            ptr::addr_of_mut!(PYTRUESTRUCT_HANDLE).cast()
        } else if is_type_object && actual_type == PYBASEOBJECT_TYPE_HANDLE {
            ptr::addr_of_mut!(PYBASEOBJECT_TYPE_HANDLE).cast()
        } else if is_type_object && actual_type == PYBOOL_TYPE_HANDLE {
            ptr::addr_of_mut!(PYBOOL_TYPE_HANDLE).cast()
        } else if is_type_object && actual_type == PYBYTEARRAY_TYPE_HANDLE {
            ptr::addr_of_mut!(PYBYTEARRAY_TYPE_HANDLE).cast()
        } else if is_type_object && actual_type == PYBYTES_TYPE_HANDLE {
            ptr::addr_of_mut!(PYBYTES_TYPE_HANDLE).cast()
        } else if is_type_object && actual_type == PYDICT_TYPE_HANDLE {
            ptr::addr_of_mut!(PYDICT_TYPE_HANDLE).cast()
        } else if is_type_object && actual_type == PYLIST_TYPE_HANDLE {
            ptr::addr_of_mut!(PYLIST_TYPE_HANDLE).cast()
        } else if is_type_object && actual_type == PYLONG_TYPE_HANDLE {
            ptr::addr_of_mut!(PYLONG_TYPE_HANDLE).cast()
        } else if is_type_object && actual_type == PYMODULE_TYPE_HANDLE {
            ptr::addr_of_mut!(PYMODULE_TYPE_HANDLE).cast()
        } else if is_type_object && actual_type == PYTUPLE_TYPE_HANDLE {
            ptr::addr_of_mut!(PYTUPLE_TYPE_HANDLE).cast()
        } else if is_type_object && actual_type == PYTYPE_TYPE_HANDLE {
            ptr::addr_of_mut!(PYTYPE_TYPE_HANDLE).cast()
        } else if is_type_object && actual_type == PYUNICODE_TYPE_HANDLE {
            ptr::addr_of_mut!(PYUNICODE_TYPE_HANDLE).cast()
        } else {
            let maps = wrapper_maps().lock().unwrap();
            if let Some(wrapper) = maps.inner_to_wrapper.get(&(actual as usize)).copied() {
                wrapper as *mut PyObject
            } else {
                actual
            }
        }
    }
}

#[inline]
pub(crate) unsafe fn resolve_object_handle(exported: *mut PyObject) -> *mut PyObject {
    unsafe {
        if exported == ptr::addr_of_mut!(PYNONESTRUCT_HANDLE).cast() {
            PYNONESTRUCT_HANDLE
        } else if exported == ptr::addr_of_mut!(PYFALSESTRUCT_HANDLE).cast() {
            PYFALSESTRUCT_HANDLE
        } else if exported == ptr::addr_of_mut!(PYTRUESTRUCT_HANDLE).cast() {
            PYTRUESTRUCT_HANDLE
        } else if exported == ptr::addr_of_mut!(PYBASEOBJECT_TYPE_HANDLE).cast() {
            PYBASEOBJECT_TYPE_HANDLE.cast()
        } else if exported == ptr::addr_of_mut!(PYBOOL_TYPE_HANDLE).cast() {
            PYBOOL_TYPE_HANDLE.cast()
        } else if exported == ptr::addr_of_mut!(PYDICT_TYPE_HANDLE).cast() {
            PYDICT_TYPE_HANDLE.cast()
        } else if exported == ptr::addr_of_mut!(PYLIST_TYPE_HANDLE).cast() {
            PYLIST_TYPE_HANDLE.cast()
        } else if exported == ptr::addr_of_mut!(PYLONG_TYPE_HANDLE).cast() {
            PYLONG_TYPE_HANDLE.cast()
        } else if exported == ptr::addr_of_mut!(PYMODULE_TYPE_HANDLE).cast() {
            PYMODULE_TYPE_HANDLE.cast()
        } else if exported == ptr::addr_of_mut!(PYTUPLE_TYPE_HANDLE).cast() {
            PYTUPLE_TYPE_HANDLE.cast()
        } else if exported == ptr::addr_of_mut!(PYTYPE_TYPE_HANDLE).cast() {
            PYTYPE_TYPE_HANDLE.cast()
        } else if exported == ptr::addr_of_mut!(PYUNICODE_TYPE_HANDLE).cast() {
            PYUNICODE_TYPE_HANDLE.cast()
        } else {
            let maps = wrapper_maps().lock().unwrap();
            maps.wrapper_to_inner
                .get(&(exported as usize))
                .copied()
                .map(|ptr| ptr as *mut PyObject)
                .unwrap_or(exported)
        }
    }
}
