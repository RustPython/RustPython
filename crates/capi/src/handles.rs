use crate::PyObject;
use crate::object::PyTypeObject;
use core::ptr;
use rustpython_vm::{AsObject, Py};
use rustpython_vm::builtins::PyType;
use rustpython_vm::vm::Context;
use std::alloc::{Layout, alloc_zeroed, dealloc};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

#[repr(C)]
struct CApiObjectHeader {
    ob_refcnt: isize,
    ob_type: *mut PyTypeObject,
}

#[repr(C)]
struct ExportedStaticObject {
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

static mut ACTUAL_PYBASEOBJECT_TYPE: *mut PyTypeObject = ptr::null_mut();
static mut ACTUAL_PYBOOL_TYPE: *mut PyTypeObject = ptr::null_mut();
static mut ACTUAL_PYBYTEARRAY_TYPE: *mut PyTypeObject = ptr::null_mut();
static mut ACTUAL_PYBYTES_TYPE: *mut PyTypeObject = ptr::null_mut();
static mut ACTUAL_PYDICT_TYPE: *mut PyTypeObject = ptr::null_mut();
static mut ACTUAL_PYLIST_TYPE: *mut PyTypeObject = ptr::null_mut();
static mut ACTUAL_PYLONG_TYPE: *mut PyTypeObject = ptr::null_mut();
static mut ACTUAL_PYMODULE_TYPE: *mut PyTypeObject = ptr::null_mut();
static mut ACTUAL_PYTUPLE_TYPE: *mut PyTypeObject = ptr::null_mut();
static mut ACTUAL_PYTYPE_TYPE: *mut PyTypeObject = ptr::null_mut();
static mut ACTUAL_PYUNICODE_TYPE: *mut PyTypeObject = ptr::null_mut();

static mut ACTUAL_PYNONESTRUCT: *mut PyObject = ptr::null_mut();
static mut ACTUAL_PYFALSESTRUCT: *mut PyObject = ptr::null_mut();
static mut ACTUAL_PYTRUESTRUCT: *mut PyObject = ptr::null_mut();

#[unsafe(export_name = "PyBaseObject_Type")]
static mut PYBASEOBJECT_TYPE_EXPORT: ExportedStaticObject = ExportedStaticObject {
    ob_refcnt: 1,
    ob_type: ptr::null_mut(),
};
#[unsafe(export_name = "PyBool_Type")]
static mut PYBOOL_TYPE_EXPORT: ExportedStaticObject = ExportedStaticObject {
    ob_refcnt: 1,
    ob_type: ptr::null_mut(),
};
#[unsafe(export_name = "PyByteArray_Type")]
static mut PYBYTEARRAY_TYPE_EXPORT: ExportedStaticObject = ExportedStaticObject {
    ob_refcnt: 1,
    ob_type: ptr::null_mut(),
};
#[unsafe(export_name = "PyBytes_Type")]
static mut PYBYTES_TYPE_EXPORT: ExportedStaticObject = ExportedStaticObject {
    ob_refcnt: 1,
    ob_type: ptr::null_mut(),
};
#[unsafe(export_name = "PyDict_Type")]
static mut PYDICT_TYPE_EXPORT: ExportedStaticObject = ExportedStaticObject {
    ob_refcnt: 1,
    ob_type: ptr::null_mut(),
};
#[unsafe(export_name = "PyList_Type")]
static mut PYLIST_TYPE_EXPORT: ExportedStaticObject = ExportedStaticObject {
    ob_refcnt: 1,
    ob_type: ptr::null_mut(),
};
#[unsafe(export_name = "PyLong_Type")]
static mut PYLONG_TYPE_EXPORT: ExportedStaticObject = ExportedStaticObject {
    ob_refcnt: 1,
    ob_type: ptr::null_mut(),
};
#[unsafe(export_name = "PyModule_Type")]
static mut PYMODULE_TYPE_EXPORT: ExportedStaticObject = ExportedStaticObject {
    ob_refcnt: 1,
    ob_type: ptr::null_mut(),
};
#[unsafe(export_name = "PyTuple_Type")]
static mut PYTUPLE_TYPE_EXPORT: ExportedStaticObject = ExportedStaticObject {
    ob_refcnt: 1,
    ob_type: ptr::null_mut(),
};
#[unsafe(export_name = "PyType_Type")]
static mut PYTYPE_TYPE_EXPORT: ExportedStaticObject = ExportedStaticObject {
    ob_refcnt: 1,
    ob_type: ptr::null_mut(),
};
#[unsafe(export_name = "PyUnicode_Type")]
static mut PYUNICODE_TYPE_EXPORT: ExportedStaticObject = ExportedStaticObject {
    ob_refcnt: 1,
    ob_type: ptr::null_mut(),
};

#[unsafe(export_name = "_Py_NoneStruct")]
static mut PYNONESTRUCT_EXPORT: ExportedStaticObject = ExportedStaticObject {
    ob_refcnt: 1,
    ob_type: ptr::null_mut(),
};
#[unsafe(export_name = "_Py_FalseStruct")]
static mut PYFALSESTRUCT_EXPORT: ExportedStaticObject = ExportedStaticObject {
    ob_refcnt: 1,
    ob_type: ptr::null_mut(),
};
#[unsafe(export_name = "_Py_TrueStruct")]
static mut PYTRUESTRUCT_EXPORT: ExportedStaticObject = ExportedStaticObject {
    ob_refcnt: 1,
    ob_type: ptr::null_mut(),
};

#[allow(static_mut_refs)]
pub(crate) unsafe fn init_exported_builtin_objects(ctx: &Context) {
    unsafe {
        ACTUAL_PYBASEOBJECT_TYPE =
            normalize_type_ptr(ctx.types.object_type as *const Py<PyType> as *mut PyTypeObject);
        ACTUAL_PYBOOL_TYPE =
            normalize_type_ptr(ctx.types.bool_type as *const Py<PyType> as *mut PyTypeObject);
        ACTUAL_PYBYTEARRAY_TYPE =
            normalize_type_ptr(ctx.types.bytearray_type as *const Py<PyType> as *mut PyTypeObject);
        ACTUAL_PYBYTES_TYPE =
            normalize_type_ptr(ctx.types.bytes_type as *const Py<PyType> as *mut PyTypeObject);
        ACTUAL_PYDICT_TYPE =
            normalize_type_ptr(ctx.types.dict_type as *const Py<PyType> as *mut PyTypeObject);
        ACTUAL_PYLIST_TYPE =
            normalize_type_ptr(ctx.types.list_type as *const Py<PyType> as *mut PyTypeObject);
        ACTUAL_PYLONG_TYPE =
            normalize_type_ptr(ctx.types.int_type as *const Py<PyType> as *mut PyTypeObject);
        ACTUAL_PYMODULE_TYPE =
            normalize_type_ptr(ctx.types.module_type as *const Py<PyType> as *mut PyTypeObject);
        ACTUAL_PYTUPLE_TYPE =
            normalize_type_ptr(ctx.types.tuple_type as *const Py<PyType> as *mut PyTypeObject);
        ACTUAL_PYTYPE_TYPE =
            normalize_type_ptr(ctx.types.type_type as *const Py<PyType> as *mut PyTypeObject);
        ACTUAL_PYUNICODE_TYPE =
            normalize_type_ptr(ctx.types.str_type as *const Py<PyType> as *mut PyTypeObject);

        ACTUAL_PYNONESTRUCT = ctx.none.as_object().as_raw().cast_mut();
        ACTUAL_PYFALSESTRUCT = ctx.false_value.as_object().as_raw().cast_mut();
        ACTUAL_PYTRUESTRUCT = ctx.true_value.as_object().as_raw().cast_mut();

        let pytype_export = ptr::addr_of_mut!(PYTYPE_TYPE_EXPORT).cast::<PyTypeObject>();
        PYTYPE_TYPE_EXPORT.ob_type = pytype_export;
        for exported in [
            ptr::addr_of_mut!(PYBASEOBJECT_TYPE_EXPORT),
            ptr::addr_of_mut!(PYBOOL_TYPE_EXPORT),
            ptr::addr_of_mut!(PYBYTEARRAY_TYPE_EXPORT),
            ptr::addr_of_mut!(PYBYTES_TYPE_EXPORT),
            ptr::addr_of_mut!(PYDICT_TYPE_EXPORT),
            ptr::addr_of_mut!(PYLIST_TYPE_EXPORT),
            ptr::addr_of_mut!(PYLONG_TYPE_EXPORT),
            ptr::addr_of_mut!(PYMODULE_TYPE_EXPORT),
            ptr::addr_of_mut!(PYTUPLE_TYPE_EXPORT),
            ptr::addr_of_mut!(PYUNICODE_TYPE_EXPORT),
        ] {
            (*exported).ob_type = pytype_export;
        }

        PYNONESTRUCT_EXPORT.ob_type = ctx.none.class() as *const Py<PyType> as *mut PyTypeObject;
        PYFALSESTRUCT_EXPORT.ob_type = ptr::addr_of_mut!(PYBOOL_TYPE_EXPORT).cast::<PyTypeObject>();
        PYTRUESTRUCT_EXPORT.ob_type = ptr::addr_of_mut!(PYBOOL_TYPE_EXPORT).cast::<PyTypeObject>();
    }
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
    let actual = normalize_type_ptr(actual);
    unsafe {
        if actual == ACTUAL_PYBASEOBJECT_TYPE {
            ptr::addr_of_mut!(PYBASEOBJECT_TYPE_EXPORT).cast()
        } else if actual == ACTUAL_PYBOOL_TYPE {
            ptr::addr_of_mut!(PYBOOL_TYPE_EXPORT).cast()
        } else if actual == ACTUAL_PYBYTEARRAY_TYPE {
            ptr::addr_of_mut!(PYBYTEARRAY_TYPE_EXPORT).cast()
        } else if actual == ACTUAL_PYBYTES_TYPE {
            ptr::addr_of_mut!(PYBYTES_TYPE_EXPORT).cast()
        } else if actual == ACTUAL_PYDICT_TYPE {
            ptr::addr_of_mut!(PYDICT_TYPE_EXPORT).cast()
        } else if actual == ACTUAL_PYLIST_TYPE {
            ptr::addr_of_mut!(PYLIST_TYPE_EXPORT).cast()
        } else if actual == ACTUAL_PYLONG_TYPE {
            ptr::addr_of_mut!(PYLONG_TYPE_EXPORT).cast()
        } else if actual == ACTUAL_PYMODULE_TYPE {
            ptr::addr_of_mut!(PYMODULE_TYPE_EXPORT).cast()
        } else if actual == ACTUAL_PYTUPLE_TYPE {
            ptr::addr_of_mut!(PYTUPLE_TYPE_EXPORT).cast()
        } else if actual == ACTUAL_PYTYPE_TYPE {
            ptr::addr_of_mut!(PYTYPE_TYPE_EXPORT).cast()
        } else if actual == ACTUAL_PYUNICODE_TYPE {
            ptr::addr_of_mut!(PYUNICODE_TYPE_EXPORT).cast()
        } else {
            actual
        }
    }
}

#[inline]
pub(crate) unsafe fn resolve_type_handle(exported: *mut PyTypeObject) -> *mut PyTypeObject {
    let exported = normalize_type_ptr(exported);
    unsafe {
        if exported == ptr::addr_of_mut!(PYBASEOBJECT_TYPE_EXPORT).cast() {
            ACTUAL_PYBASEOBJECT_TYPE
        } else if exported == ptr::addr_of_mut!(PYBOOL_TYPE_EXPORT).cast() {
            ACTUAL_PYBOOL_TYPE
        } else if exported == ptr::addr_of_mut!(PYBYTEARRAY_TYPE_EXPORT).cast() {
            ACTUAL_PYBYTEARRAY_TYPE
        } else if exported == ptr::addr_of_mut!(PYBYTES_TYPE_EXPORT).cast() {
            ACTUAL_PYBYTES_TYPE
        } else if exported == ptr::addr_of_mut!(PYDICT_TYPE_EXPORT).cast() {
            ACTUAL_PYDICT_TYPE
        } else if exported == ptr::addr_of_mut!(PYLIST_TYPE_EXPORT).cast() {
            ACTUAL_PYLIST_TYPE
        } else if exported == ptr::addr_of_mut!(PYLONG_TYPE_EXPORT).cast() {
            ACTUAL_PYLONG_TYPE
        } else if exported == ptr::addr_of_mut!(PYMODULE_TYPE_EXPORT).cast() {
            ACTUAL_PYMODULE_TYPE
        } else if exported == ptr::addr_of_mut!(PYTUPLE_TYPE_EXPORT).cast() {
            ACTUAL_PYTUPLE_TYPE
        } else if exported == ptr::addr_of_mut!(PYTYPE_TYPE_EXPORT).cast() {
            ACTUAL_PYTYPE_TYPE
        } else if exported == ptr::addr_of_mut!(PYUNICODE_TYPE_EXPORT).cast() {
            ACTUAL_PYUNICODE_TYPE
        } else {
            exported
        }
    }
}

#[inline]
pub(crate) unsafe fn exported_object_handle(actual: *mut PyObject) -> *mut PyObject {
    unsafe {
        if actual == ptr::addr_of_mut!(PYNONESTRUCT_EXPORT).cast()
            || actual == ptr::addr_of_mut!(PYFALSESTRUCT_EXPORT).cast()
            || actual == ptr::addr_of_mut!(PYTRUESTRUCT_EXPORT).cast()
            || actual == ptr::addr_of_mut!(PYBASEOBJECT_TYPE_EXPORT).cast()
            || actual == ptr::addr_of_mut!(PYBOOL_TYPE_EXPORT).cast()
            || actual == ptr::addr_of_mut!(PYBYTEARRAY_TYPE_EXPORT).cast()
            || actual == ptr::addr_of_mut!(PYBYTES_TYPE_EXPORT).cast()
            || actual == ptr::addr_of_mut!(PYDICT_TYPE_EXPORT).cast()
            || actual == ptr::addr_of_mut!(PYLIST_TYPE_EXPORT).cast()
            || actual == ptr::addr_of_mut!(PYLONG_TYPE_EXPORT).cast()
            || actual == ptr::addr_of_mut!(PYMODULE_TYPE_EXPORT).cast()
            || actual == ptr::addr_of_mut!(PYTUPLE_TYPE_EXPORT).cast()
            || actual == ptr::addr_of_mut!(PYTYPE_TYPE_EXPORT).cast()
            || actual == ptr::addr_of_mut!(PYUNICODE_TYPE_EXPORT).cast()
        {
            actual
        } else if wrapper_maps()
            .lock()
            .unwrap()
            .wrapper_to_inner
            .contains_key(&(actual as usize))
        {
            actual
        } else if actual == ACTUAL_PYNONESTRUCT {
            ptr::addr_of_mut!(PYNONESTRUCT_EXPORT).cast()
        } else if actual == ACTUAL_PYFALSESTRUCT {
            ptr::addr_of_mut!(PYFALSESTRUCT_EXPORT).cast()
        } else if actual == ACTUAL_PYTRUESTRUCT {
            ptr::addr_of_mut!(PYTRUESTRUCT_EXPORT).cast()
        } else {
            let actual_class =
                normalize_type_ptr((*actual).class() as *const Py<PyType> as *mut PyTypeObject);
            if actual_class == ACTUAL_PYTYPE_TYPE {
                exported_type_handle(actual.cast()).cast()
            } else {
                let maps = wrapper_maps().lock().unwrap();
                maps.inner_to_wrapper
                    .get(&(actual as usize))
                    .copied()
                    .map(|wrapper| wrapper as *mut PyObject)
                    .unwrap_or(actual)
            }
        }
    }
}

#[inline]
pub(crate) unsafe fn resolve_object_handle(exported: *mut PyObject) -> *mut PyObject {
    unsafe {
        if exported == ptr::addr_of_mut!(PYNONESTRUCT_EXPORT).cast() {
            ACTUAL_PYNONESTRUCT
        } else if exported == ptr::addr_of_mut!(PYFALSESTRUCT_EXPORT).cast() {
            ACTUAL_PYFALSESTRUCT
        } else if exported == ptr::addr_of_mut!(PYTRUESTRUCT_EXPORT).cast() {
            ACTUAL_PYTRUESTRUCT
        } else if exported == ptr::addr_of_mut!(PYBASEOBJECT_TYPE_EXPORT).cast() {
            ACTUAL_PYBASEOBJECT_TYPE.cast()
        } else if exported == ptr::addr_of_mut!(PYBOOL_TYPE_EXPORT).cast() {
            ACTUAL_PYBOOL_TYPE.cast()
        } else if exported == ptr::addr_of_mut!(PYBYTEARRAY_TYPE_EXPORT).cast() {
            ACTUAL_PYBYTEARRAY_TYPE.cast()
        } else if exported == ptr::addr_of_mut!(PYBYTES_TYPE_EXPORT).cast() {
            ACTUAL_PYBYTES_TYPE.cast()
        } else if exported == ptr::addr_of_mut!(PYDICT_TYPE_EXPORT).cast() {
            ACTUAL_PYDICT_TYPE.cast()
        } else if exported == ptr::addr_of_mut!(PYLIST_TYPE_EXPORT).cast() {
            ACTUAL_PYLIST_TYPE.cast()
        } else if exported == ptr::addr_of_mut!(PYLONG_TYPE_EXPORT).cast() {
            ACTUAL_PYLONG_TYPE.cast()
        } else if exported == ptr::addr_of_mut!(PYMODULE_TYPE_EXPORT).cast() {
            ACTUAL_PYMODULE_TYPE.cast()
        } else if exported == ptr::addr_of_mut!(PYTUPLE_TYPE_EXPORT).cast() {
            ACTUAL_PYTUPLE_TYPE.cast()
        } else if exported == ptr::addr_of_mut!(PYTYPE_TYPE_EXPORT).cast() {
            ACTUAL_PYTYPE_TYPE.cast()
        } else if exported == ptr::addr_of_mut!(PYUNICODE_TYPE_EXPORT).cast() {
            ACTUAL_PYUNICODE_TYPE.cast()
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
