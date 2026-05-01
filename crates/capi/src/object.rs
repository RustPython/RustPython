use crate::handles::{
    exported_object_handle, exported_object_wrapper, exported_type_handle, resolve_object_handle,
    resolve_type_handle,
};
use crate::methodobject::{PyMethodDef as CApiMethodDef, build_tp_method};
use crate::util::owned_from_exported_new_ref;
use crate::{PyObject, with_vm};
use core::ffi::{CStr, c_char, c_int, c_uint, c_ulong, c_void};
use core::ptr::NonNull;
use rustpython_vm::builtins::{PyStr, PyType};
use rustpython_vm::class::add_operators;
use rustpython_vm::convert::IntoObject;
use rustpython_vm::convert::ToPyObject;
use rustpython_vm::function::{Either, FsPath, PyComparisonValue};
use rustpython_vm::protocol::{PyIterReturn, PyMapping, PySequence};
use rustpython_vm::types::{PyTypeFlags, PyTypeSlots, SlotAccessor};
use rustpython_vm::{AsObject, Context, Py, PyObjectRef, PyResult, VirtualMachine};

const PY_TPFLAGS_LONG_SUBCLASS: c_ulong = 1 << 24;
const PY_TPFLAGS_LIST_SUBCLASS: c_ulong = 1 << 25;
const PY_TPFLAGS_TUPLE_SUBCLASS: c_ulong = 1 << 26;
const PY_TPFLAGS_BYTES_SUBCLASS: c_ulong = 1 << 27;
const PY_TPFLAGS_UNICODE_SUBCLASS: c_ulong = 1 << 28;
const PY_TPFLAGS_DICT_SUBCLASS: c_ulong = 1 << 29;
const PY_TPFLAGS_BASE_EXC_SUBCLASS: c_ulong = 1 << 30;
const PY_TPFLAGS_TYPE_SUBCLASS: c_ulong = 1 << 31;

pub type PyTypeObject = Py<PyType>;

#[unsafe(no_mangle)]
pub extern "C" fn Py_TYPE(op: *mut PyObject) -> *const PyTypeObject {
    // SAFETY: The caller must guarantee that `op` is a valid pointer to a `PyObject`.
    unsafe {
        let actual = (*resolve_object_handle(op)).class() as *const Py<PyType> as *mut PyTypeObject;
        exported_type_handle(actual).cast_const()
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn Py_IS_TYPE(op: *mut PyObject, ty: *mut PyTypeObject) -> c_int {
    with_vm(|_vm| {
        let obj = unsafe { &*resolve_object_handle(op) };
        let ty = unsafe { &*resolve_type_handle(ty) };
        obj.class().is(ty)
    })
}

#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn PyType_GetFlags(ptr: *const PyTypeObject) -> c_ulong {
    let ctx = Context::genesis();
    let zoo = &ctx.types;
    let exp_zoo = &ctx.exceptions;

    // SAFETY: The caller must guarantee that `ptr` is a valid pointer to a `PyType` object.
    let ty = unsafe { &*resolve_type_handle(ptr.cast_mut()) };
    let mut flags = ty.slots.flags.bits();

    if ty.is_subtype(zoo.int_type) {
        flags |= PY_TPFLAGS_LONG_SUBCLASS;
    }
    if ty.is_subtype(zoo.list_type) {
        flags |= PY_TPFLAGS_LIST_SUBCLASS
    }
    if ty.is_subtype(zoo.tuple_type) {
        flags |= PY_TPFLAGS_TUPLE_SUBCLASS;
    }
    if ty.is_subtype(zoo.bytes_type) {
        flags |= PY_TPFLAGS_BYTES_SUBCLASS;
    }
    if ty.is_subtype(zoo.str_type) {
        flags |= PY_TPFLAGS_UNICODE_SUBCLASS;
    }
    if ty.is_subtype(zoo.dict_type) {
        flags |= PY_TPFLAGS_DICT_SUBCLASS;
    }
    if ty.is_subtype(exp_zoo.base_exception_type) {
        flags |= PY_TPFLAGS_BASE_EXC_SUBCLASS;
    }
    if ty.is_subtype(zoo.type_type) {
        flags |= PY_TPFLAGS_TYPE_SUBCLASS;
    }

    flags
}

#[unsafe(no_mangle)]
pub extern "C" fn PyType_GetName(ptr: *const PyTypeObject) -> *mut PyObject {
    let ty = unsafe { &*resolve_type_handle(ptr.cast_mut()) };
    with_vm(move |vm| ty.__name__(vm))
}

#[unsafe(no_mangle)]
pub extern "C" fn PyType_GetQualName(ptr: *const PyTypeObject) -> *mut PyObject {
    let ty = unsafe { &*resolve_type_handle(ptr.cast_mut()) };
    with_vm(move |vm| ty.__qualname__(vm))
}

#[unsafe(no_mangle)]
pub extern "C" fn PyType_GetModuleName(ptr: *const PyTypeObject) -> *mut PyObject {
    let ty = unsafe { &*resolve_type_handle(ptr.cast_mut()) };
    with_vm(move |vm| ty.__module__(vm))
}

#[unsafe(no_mangle)]
pub extern "C" fn PyType_GetFullyQualifiedName(ptr: *const PyTypeObject) -> *mut PyObject {
    let ty = unsafe { &*resolve_type_handle(ptr.cast_mut()) };
    with_vm(move |vm| {
        let module = ty.__module__(vm).downcast::<PyStr>().unwrap();
        let qualname = ty.__qualname__(vm).downcast::<PyStr>().unwrap();
        let fully_qualified_name = format!(
            "{}.{}",
            module.to_string_lossy(),
            qualname.to_string_lossy()
        );
        vm.ctx.new_str(fully_qualified_name)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyType_IsSubtype(a: *const PyTypeObject, b: *const PyTypeObject) -> c_int {
    with_vm(move |_vm| {
        let a = unsafe { &*resolve_type_handle(a.cast_mut()) };
        let b = unsafe { &*resolve_type_handle(b.cast_mut()) };
        Ok(a.is_subtype(b))
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyType_GetSlot(ty: *const PyTypeObject, slot: c_int) -> *mut c_void {
    with_vm(|_vm| -> Option<*mut c_void> {
        let ty = unsafe { &*resolve_type_handle(ty.cast_mut()) };
        let slot: u8 = slot
            .try_into()
            .expect("slot number out of range for SlotAccessor");
        let slot_accessor: SlotAccessor = slot
            .try_into()
            .expect("invalid slot number for SlotAccessor");

        match slot_accessor {
            SlotAccessor::TpNew => {
                if let Some(vtable) = ty.get_type_data::<TypeVTable>() {
                    vtable.new_func.map(|newfunc| newfunc as *mut c_void)
                } else if ty.is(_vm.ctx.types.object_type) {
                    Some(PyType_GenericNew as *mut c_void)
                } else {
                    None
                }
            }
            _ => {
                todo!("Slot {slot_accessor:?} for {ty:?} is not yet implemented in PyType_GetSlot")
            }
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyType_GenericAlloc(
    subtype: *mut PyTypeObject,
    nitems: isize,
) -> *mut PyObject {
    with_vm(|vm| {
        let subtype = unsafe { &*resolve_type_handle(subtype) };
        let alloc = subtype
            .slots
            .alloc
            .load()
            .ok_or_else(|| vm.new_type_error(format!("type {} has no tp_alloc", subtype.name())))?;
        let inner = alloc(subtype.to_owned(), nitems.try_into().unwrap_or(0usize), vm)?;
        let size = subtype
            .slots
            .basicsize
            .saturating_add(subtype.slots.itemsize.saturating_mul(nitems.max(0) as usize));
        Ok(unsafe { exported_object_wrapper(inner.as_object().as_raw().cast_mut(), size) })
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyType_GenericNew(
    subtype: *mut PyTypeObject,
    _args: *mut PyObject,
    _kwargs: *mut PyObject,
) -> *mut PyObject {
    with_vm(|vm| {
        let subtype = unsafe { &*resolve_type_handle(subtype) };
        let alloc = subtype
            .slots
            .alloc
            .load()
            .ok_or_else(|| vm.new_type_error(format!("type {} has no tp_alloc", subtype.name())))?;
        let inner = alloc(subtype.to_owned(), 0, vm)?;
        let size = subtype.slots.basicsize;
        Ok(unsafe { exported_object_wrapper(inner.as_object().as_raw().cast_mut(), size) })
    })
}

#[repr(C)]
pub struct PyType_Slot {
    slot: c_int,
    pfunc: *mut c_void,
}

#[repr(C)]
pub struct PyType_Spec {
    name: *const c_char,
    basicsize: c_int,
    itemsize: c_int,
    flags: c_uint,
    slots: *mut PyType_Slot,
}

#[repr(C)]
pub struct PyGetSetDef {
    name: *const c_char,
    get: extern "C" fn(*mut PyObject, usize) -> *mut PyObject,
    set: Option<extern "C" fn(*mut PyObject, *mut PyObject, usize) -> c_int>,
    doc: *const c_char,
    closure: usize,
}

#[derive(Default)]
struct TypeVTable {
    new_func: Option<newfunc>,
    init_func: Option<initproc>,
    float_func: Option<unaryfunc>,
    str_func: Option<unaryfunc>,
    repr_func: Option<unaryfunc>,
    sq_length_func: Option<lenfunc>,
    contains_func: Option<objobjproc>,
    iter_func: Option<unaryfunc>,
    iternext_func: Option<unaryfunc>,
    mp_subscript_func: Option<binaryfunc>,
    mp_length_func: Option<lenfunc>,
    richcompare_func: Option<richcmpfunc>,
    hash_func: Option<hashfunc>,
    subtract_func: Option<binaryfunc>,
    and_func: Option<binaryfunc>,
    or_func: Option<binaryfunc>,
    xor_func: Option<binaryfunc>,
}

type newfunc = unsafe extern "C" fn(
    ty: *mut PyTypeObject,
    args: *mut PyObject,
    kwargs: *mut PyObject,
) -> *mut PyObject;

type initproc = unsafe extern "C" fn(
    slf: *mut PyObject,
    args: *mut PyObject,
    kwargs: *mut PyObject,
) -> c_int;

type unaryfunc = unsafe extern "C" fn(slf: *mut PyObject) -> *mut PyObject;
type objobjproc = unsafe extern "C" fn(slf: *mut PyObject, obj: *mut PyObject) -> c_int;
type binaryfunc = unsafe extern "C" fn(slf: *mut PyObject, obj: *mut PyObject) -> *mut PyObject;
type lenfunc = unsafe extern "C" fn(slf: *mut PyObject) -> isize;
type richcmpfunc =
    unsafe extern "C" fn(slf: *mut PyObject, obj: *mut PyObject, op: c_int) -> *mut PyObject;
type hashfunc = unsafe extern "C" fn(slf: *mut PyObject) -> isize;

fn native_tp_new(ty: rustpython_vm::builtins::PyTypeRef, args: rustpython_vm::function::FuncArgs, vm: &VirtualMachine) -> PyResult {
    let new_func = ty.get_type_data::<TypeVTable>().unwrap().new_func.unwrap();
    let kwargs = vm.ctx.new_dict();
    for (name, value) in &args.kwargs {
        kwargs.set_item(&*vm.ctx.new_str(name.clone()), value.clone(), vm)?;
    }
    let args = vm.ctx.new_tuple(args.args);
    let result = unsafe {
        new_func(
            (&*ty) as *const _ as *mut _,
            args.as_object().as_raw().cast_mut(),
            kwargs.as_object().as_raw().cast_mut(),
        )
    };
    let result = NonNull::new(result).ok_or_else(|| {
        vm.take_raised_exception()
            .expect("native tp_new returned NULL, but there was no exception set")
    })?;
    unsafe { Ok(owned_from_exported_new_ref(result.as_ptr())) }
}

fn native_tp_init(obj: PyObjectRef, args: rustpython_vm::function::FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
    let init_func = obj
        .class()
        .get_type_data::<TypeVTable>()
        .unwrap()
        .init_func
        .unwrap();
    let kwargs = vm.ctx.new_dict();
    for (name, value) in &args.kwargs {
        kwargs.set_item(&*vm.ctx.new_str(name.clone()), value.clone(), vm)?;
    }
    let args = vm.ctx.new_tuple(args.args);
    let rc = unsafe {
        let exported_obj = exported_object_handle(obj.as_object().as_raw().cast_mut());
        init_func(
            exported_obj,
            args.as_object().as_raw().cast_mut(),
            kwargs.as_object().as_raw().cast_mut(),
        )
    };
    if rc == 0 {
        Ok(())
    } else {
        let class_name = obj.class().name().to_string();
        Err(vm.take_raised_exception().unwrap_or_else(|| {
            vm.new_type_error(format!(
                "native tp_init for {class_name} failed without exception"
            ))
        }))
    }
}

fn native_nb_float(
    num: rustpython_vm::protocol::PyNumber<'_>,
    vm: &VirtualMachine,
) -> PyResult {
    let float_func = num
        .class()
        .get_type_data::<TypeVTable>()
        .and_then(|vtable| vtable.float_func)
        .expect("native_nb_float called without a registered float slot");
    let slf_ptr = unsafe { exported_object_handle(num.obj.as_raw().cast_mut()) };
    let result = unsafe { float_func(slf_ptr) };
    let result = NonNull::new(result).ok_or_else(|| {
        vm.take_raised_exception()
            .expect("native nb_float returned NULL, but there was no exception set")
    })?;
    unsafe { Ok(owned_from_exported_new_ref(result.as_ptr())) }
}

fn native_tp_str(obj: &PyObject, vm: &VirtualMachine) -> PyResult<rustpython_vm::PyRef<PyStr>> {
    let str_func = obj
        .class()
        .get_type_data::<TypeVTable>()
        .and_then(|vtable| vtable.str_func)
        .expect("native_tp_str called without a registered str slot");
    let slf_ptr = unsafe { exported_object_handle(obj.as_raw().cast_mut()) };
    let result = unsafe { str_func(slf_ptr) };
    let result = NonNull::new(result).ok_or_else(|| {
        vm.take_raised_exception()
            .expect("native tp_str returned NULL, but there was no exception set")
    })?;
    let resolved = unsafe { owned_from_exported_new_ref(result.as_ptr()) };
    let class_name = obj.class().name().to_string();
    resolved.downcast::<PyStr>().map_err(|obj| {
        vm.new_type_error(format!(
            "native tp_str for {class_name} returned non-str {}",
            obj.class().name()
        ))
    })
}

fn native_tp_repr(obj: &PyObject, vm: &VirtualMachine) -> PyResult<rustpython_vm::PyRef<PyStr>> {
    let repr_func = obj
        .class()
        .get_type_data::<TypeVTable>()
        .and_then(|vtable| vtable.repr_func)
        .expect("native_tp_repr called without a registered repr slot");
    let slf_ptr = unsafe { exported_object_handle(obj.as_raw().cast_mut()) };
    let result = unsafe { repr_func(slf_ptr) };
    let result = NonNull::new(result).ok_or_else(|| {
        vm.take_raised_exception()
            .expect("native tp_repr returned NULL, but there was no exception set")
    })?;
    let resolved = unsafe { owned_from_exported_new_ref(result.as_ptr()) };
    let class_name = obj.class().name().to_string();
    resolved.downcast::<PyStr>().map_err(|obj| {
        vm.new_type_error(format!(
            "native tp_repr for {class_name} returned non-str {}",
            obj.class().name()
        ))
    })
}

fn native_sq_contains(seq: PySequence<'_>, needle: &PyObject, vm: &VirtualMachine) -> PyResult<bool> {
    let contains_func = seq
        .obj
        .class()
        .get_type_data::<TypeVTable>()
        .and_then(|vtable| vtable.contains_func)
        .expect("native_sq_contains called without a registered contains slot");
    let slf_ptr = unsafe { exported_object_handle(seq.obj.as_raw().cast_mut()) };
    let needle_ptr = unsafe { exported_object_handle(needle.as_raw().cast_mut()) };
    let rc = unsafe { contains_func(slf_ptr, needle_ptr) };
    match rc {
        1 => Ok(true),
        0 => Ok(false),
        _ => Err(vm
            .take_raised_exception()
            .expect("native sq_contains returned error, but there was no exception set")),
    }
}

fn native_sq_length(seq: PySequence<'_>, vm: &VirtualMachine) -> PyResult<usize> {
    let length_func = seq
        .obj
        .class()
        .get_type_data::<TypeVTable>()
        .and_then(|vtable| vtable.sq_length_func)
        .expect("native_sq_length called without a registered sq_length slot");
    let slf_ptr = unsafe { exported_object_handle(seq.obj.as_raw().cast_mut()) };
    let result = unsafe { length_func(slf_ptr) };
    if result >= 0 {
        Ok(result as usize)
    } else {
        Err(vm
            .take_raised_exception()
            .expect("native sq_length returned error, but there was no exception set"))
    }
}

fn native_tp_iter(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
    let iter_func = obj
        .class()
        .get_type_data::<TypeVTable>()
        .and_then(|vtable| vtable.iter_func)
        .expect("native_tp_iter called without a registered iter slot");
    let slf_ptr = unsafe { exported_object_handle(obj.as_object().as_raw().cast_mut()) };
    let result = unsafe { iter_func(slf_ptr) };
    let result = NonNull::new(result).ok_or_else(|| {
        vm.take_raised_exception()
            .expect("native tp_iter returned NULL, but there was no exception set")
    })?;
    unsafe { Ok(owned_from_exported_new_ref(result.as_ptr())) }
}

fn native_tp_iternext(obj: &PyObject, vm: &VirtualMachine) -> PyResult<PyIterReturn> {
    let iternext_func = obj
        .class()
        .get_type_data::<TypeVTable>()
        .and_then(|vtable| vtable.iternext_func)
        .expect("native_tp_iternext called without a registered iternext slot");
    let slf_ptr = unsafe { exported_object_handle(obj.as_raw().cast_mut()) };
    let result = unsafe { iternext_func(slf_ptr) };
    match NonNull::new(result) {
        Some(result) => {
            let resolved = unsafe { owned_from_exported_new_ref(result.as_ptr()) };
            Ok(PyIterReturn::Return(resolved))
        }
        None => match vm.take_raised_exception() {
            Some(err) if err.fast_isinstance(vm.ctx.exceptions.stop_iteration) => {
                Ok(PyIterReturn::StopIteration(err.get_arg(0)))
            }
            Some(err) => Err(err),
            None => Ok(PyIterReturn::StopIteration(None)),
        },
    }
}

fn native_mp_subscript(mapping: PyMapping<'_>, needle: &PyObject, vm: &VirtualMachine) -> PyResult {
    let subscript_func = mapping
        .obj
        .class()
        .get_type_data::<TypeVTable>()
        .and_then(|vtable| vtable.mp_subscript_func)
        .expect("native_mp_subscript called without a registered subscript slot");
    let slf_ptr = unsafe { exported_object_handle(mapping.obj.as_raw().cast_mut()) };
    let needle_ptr = unsafe { exported_object_handle(needle.as_raw().cast_mut()) };
    let result = unsafe { subscript_func(slf_ptr, needle_ptr) };
    let result = NonNull::new(result).ok_or_else(|| {
        vm.take_raised_exception()
            .expect("native mp_subscript returned NULL, but there was no exception set")
    })?;
    unsafe { Ok(owned_from_exported_new_ref(result.as_ptr())) }
}

fn native_mp_length(mapping: PyMapping<'_>, vm: &VirtualMachine) -> PyResult<usize> {
    let length_func = mapping
        .obj
        .class()
        .get_type_data::<TypeVTable>()
        .and_then(|vtable| vtable.mp_length_func)
        .expect("native_mp_length called without a registered length slot");
    let slf_ptr = unsafe { exported_object_handle(mapping.obj.as_raw().cast_mut()) };
    let result = unsafe { length_func(slf_ptr) };
    if result >= 0 {
        Ok(result as usize)
    } else {
        Err(vm
            .take_raised_exception()
            .expect("native mp_length returned error, but there was no exception set"))
    }
}

fn native_tp_hash(obj: &PyObject, vm: &VirtualMachine) -> PyResult<rustpython_vm::common::hash::PyHash> {
    let hash_func = obj
        .class()
        .get_type_data::<TypeVTable>()
        .and_then(|vtable| vtable.hash_func)
        .expect("native_tp_hash called without a registered hash slot");
    let slf_ptr = unsafe { exported_object_handle(obj.as_raw().cast_mut()) };
    let result = unsafe { hash_func(slf_ptr) };
    if result == -1 {
        if let Some(err) = vm.take_raised_exception() {
            return Err(err);
        }
    }
    Ok(result as rustpython_vm::common::hash::PyHash)
}

fn native_nb_subtract(left: &PyObject, right: &PyObject, vm: &VirtualMachine) -> PyResult {
    let subtract_func = left
        .class()
        .get_type_data::<TypeVTable>()
        .and_then(|vtable| vtable.subtract_func)
        .expect("native_nb_subtract called without a registered subtract slot");
    let left_ptr = unsafe { exported_object_handle(left.as_raw().cast_mut()) };
    let right_ptr = unsafe { exported_object_handle(right.as_raw().cast_mut()) };
    let result = unsafe { subtract_func(left_ptr, right_ptr) };
    let result = NonNull::new(result).ok_or_else(|| {
        vm.take_raised_exception()
            .expect("native nb_subtract returned NULL, but there was no exception set")
    })?;
    unsafe { Ok(owned_from_exported_new_ref(result.as_ptr())) }
}

fn native_nb_and(left: &PyObject, right: &PyObject, vm: &VirtualMachine) -> PyResult {
    let and_func = left
        .class()
        .get_type_data::<TypeVTable>()
        .and_then(|vtable| vtable.and_func)
        .expect("native_nb_and called without a registered and slot");
    let left_ptr = unsafe { exported_object_handle(left.as_raw().cast_mut()) };
    let right_ptr = unsafe { exported_object_handle(right.as_raw().cast_mut()) };
    let result = unsafe { and_func(left_ptr, right_ptr) };
    let result = NonNull::new(result).ok_or_else(|| {
        vm.take_raised_exception()
            .expect("native nb_and returned NULL, but there was no exception set")
    })?;
    unsafe { Ok(owned_from_exported_new_ref(result.as_ptr())) }
}

fn native_nb_or(left: &PyObject, right: &PyObject, vm: &VirtualMachine) -> PyResult {
    let or_func = left
        .class()
        .get_type_data::<TypeVTable>()
        .and_then(|vtable| vtable.or_func)
        .expect("native_nb_or called without a registered or slot");
    let left_ptr = unsafe { exported_object_handle(left.as_raw().cast_mut()) };
    let right_ptr = unsafe { exported_object_handle(right.as_raw().cast_mut()) };
    let result = unsafe { or_func(left_ptr, right_ptr) };
    let result = NonNull::new(result).ok_or_else(|| {
        vm.take_raised_exception()
            .expect("native nb_or returned NULL, but there was no exception set")
    })?;
    unsafe { Ok(owned_from_exported_new_ref(result.as_ptr())) }
}

fn native_nb_xor(left: &PyObject, right: &PyObject, vm: &VirtualMachine) -> PyResult {
    let xor_func = left
        .class()
        .get_type_data::<TypeVTable>()
        .and_then(|vtable| vtable.xor_func)
        .expect("native_nb_xor called without a registered xor slot");
    let left_ptr = unsafe { exported_object_handle(left.as_raw().cast_mut()) };
    let right_ptr = unsafe { exported_object_handle(right.as_raw().cast_mut()) };
    let result = unsafe { xor_func(left_ptr, right_ptr) };
    let result = NonNull::new(result).ok_or_else(|| {
        vm.take_raised_exception()
            .expect("native nb_xor returned NULL, but there was no exception set")
    })?;
    unsafe { Ok(owned_from_exported_new_ref(result.as_ptr())) }
}

fn native_tp_richcompare(
    obj: &PyObject,
    other: &PyObject,
    op: rustpython_vm::types::PyComparisonOp,
    vm: &VirtualMachine,
) -> PyResult<Either<PyObjectRef, PyComparisonValue>> {
    let richcompare_func = obj
        .class()
        .get_type_data::<TypeVTable>()
        .and_then(|vtable| vtable.richcompare_func)
        .expect("native_tp_richcompare called without a registered richcompare slot");
    let slf_ptr = unsafe { exported_object_handle(obj.as_raw().cast_mut()) };
    let other_ptr = unsafe { exported_object_handle(other.as_raw().cast_mut()) };
    let opid = match op {
        rustpython_vm::types::PyComparisonOp::Lt => 0,
        rustpython_vm::types::PyComparisonOp::Le => 1,
        rustpython_vm::types::PyComparisonOp::Eq => 2,
        rustpython_vm::types::PyComparisonOp::Ne => 3,
        rustpython_vm::types::PyComparisonOp::Gt => 4,
        rustpython_vm::types::PyComparisonOp::Ge => 5,
    };
    let result = unsafe { richcompare_func(slf_ptr, other_ptr, opid) };
    let result = NonNull::new(result).ok_or_else(|| {
        vm.take_raised_exception()
            .expect("native tp_richcompare returned NULL, but there was no exception set")
    })?;
    let resolved = unsafe { owned_from_exported_new_ref(result.as_ptr()) };
    Ok(Either::A(resolved))
}

#[unsafe(no_mangle)]
pub extern "C" fn PyType_FromSpec(spec: *mut PyType_Spec) -> *mut PyObject {
    with_vm(|vm| {
        let spec = unsafe { &*spec };
        let full_class_name = unsafe {
            CStr::from_ptr(spec.name)
                .to_str()
                .expect("type name must be valid UTF-8")
        };
        let (module_name, class_name) = full_class_name
            .rsplit_once('.')
            .map_or((None, full_class_name), |(module, name)| (Some(module), name));
        let mut base = vm.ctx.types.object_type;
        let mut slots = PyTypeSlots::heap_default();

        slots.basicsize = spec.basicsize as _;
        slots.itemsize = spec.itemsize as _;
        slots.flags = PyTypeFlags::from_bits(spec.flags as u64).expect("invalid flags value");

        let mut attributes: &[PyGetSetDef] = &[];
        let mut methods: &[CApiMethodDef] = &[];
        let mut vtable = TypeVTable::default();
        let mut has_explicit_getattro = false;
        let mut has_explicit_setattro = false;
        let mut slot_ptr = spec.slots;
        while let slot = unsafe { &*slot_ptr }
            && slot.slot != 0
        {
            let accessor = SlotAccessor::try_from(slot.slot as u8)
                .expect("invalid slot number in PyType_Spec");

            match accessor {
                SlotAccessor::TpDealloc => {
                    // RustPython already owns object allocation and payload drops.
                    // For PyType_FromSpec heap types, accept a native tp_dealloc
                    // slot without trying to drive CPython-style raw memory teardown
                    // from the facade.
                }
                SlotAccessor::TpBase => {
                    base = unsafe { &*resolve_type_handle(slot.pfunc.cast::<PyTypeObject>()) }
                }
                SlotAccessor::TpGetset => {
                    let start = slot.pfunc.cast::<PyGetSetDef>();
                    let mut end = start;
                    while unsafe { !(*end).name.is_null() } {
                        end = unsafe { end.add(1) }
                    }
                    attributes = unsafe {
                        core::slice::from_raw_parts(start, end.offset_from(start) as usize)
                    };
                }
                SlotAccessor::TpMethods => {
                    let start = slot.pfunc.cast::<CApiMethodDef>();
                    let mut end = start;
                    while unsafe { !(*end).ml_name.is_null() } {
                        end = unsafe { end.add(1) }
                    }
                    methods = unsafe {
                        core::slice::from_raw_parts(start, end.offset_from(start) as usize)
                    };
                }
                SlotAccessor::TpNew => {
                    vtable.new_func = Some(unsafe { core::mem::transmute(slot.pfunc) });
                    slots.new.store(Some(native_tp_new));
                }
                SlotAccessor::TpInit => {
                    vtable.init_func = Some(unsafe { core::mem::transmute(slot.pfunc) });
                    slots.init.store(Some(native_tp_init));
                }
                SlotAccessor::NbFloat => {
                    vtable.float_func = Some(unsafe { core::mem::transmute(slot.pfunc) });
                    slots.as_number.float.store(Some(native_nb_float));
                }
                SlotAccessor::TpGetattro => {
                    has_explicit_getattro = true;
                    slots.getattro
                        .store(Some(unsafe { core::mem::transmute(slot.pfunc) }));
                }
                SlotAccessor::TpStr => {
                    vtable.str_func = Some(unsafe { core::mem::transmute(slot.pfunc) });
                    slots.str.store(Some(native_tp_str));
                }
                SlotAccessor::TpRepr => {
                    vtable.repr_func = Some(unsafe { core::mem::transmute(slot.pfunc) });
                    slots.repr.store(Some(native_tp_repr));
                }
                SlotAccessor::TpSetattro => {
                    has_explicit_setattro = true;
                    slots.setattro
                        .store(Some(unsafe { core::mem::transmute(slot.pfunc) }));
                }
                SlotAccessor::SqContains => {
                    vtable.contains_func = Some(unsafe { core::mem::transmute(slot.pfunc) });
                    slots.as_sequence.contains.store(Some(native_sq_contains));
                }
                SlotAccessor::SqLength => {
                    vtable.sq_length_func = Some(unsafe { core::mem::transmute(slot.pfunc) });
                    slots.as_sequence.length.store(Some(native_sq_length));
                }
                SlotAccessor::TpIter => {
                    vtable.iter_func = Some(unsafe { core::mem::transmute(slot.pfunc) });
                    slots.iter.store(Some(native_tp_iter));
                }
                SlotAccessor::TpIternext => {
                    vtable.iternext_func = Some(unsafe { core::mem::transmute(slot.pfunc) });
                    slots.iternext.store(Some(native_tp_iternext));
                }
                SlotAccessor::MpSubscript => {
                    vtable.mp_subscript_func = Some(unsafe { core::mem::transmute(slot.pfunc) });
                    slots.as_mapping.subscript.store(Some(native_mp_subscript));
                }
                SlotAccessor::MpLength => {
                    vtable.mp_length_func = Some(unsafe { core::mem::transmute(slot.pfunc) });
                    slots.as_mapping.length.store(Some(native_mp_length));
                }
                SlotAccessor::TpRichcompare => {
                    vtable.richcompare_func = Some(unsafe { core::mem::transmute(slot.pfunc) });
                    slots.richcompare.store(Some(native_tp_richcompare));
                }
                SlotAccessor::TpHash => {
                    vtable.hash_func = Some(unsafe { core::mem::transmute(slot.pfunc) });
                    slots.hash.store(Some(native_tp_hash));
                }
                SlotAccessor::NbSubtract => {
                    vtable.subtract_func = Some(unsafe { core::mem::transmute(slot.pfunc) });
                    slots.as_number.subtract.store(Some(native_nb_subtract));
                }
                SlotAccessor::NbAnd => {
                    vtable.and_func = Some(unsafe { core::mem::transmute(slot.pfunc) });
                    slots.as_number.and.store(Some(native_nb_and));
                }
                SlotAccessor::NbOr => {
                    vtable.or_func = Some(unsafe { core::mem::transmute(slot.pfunc) });
                    slots.as_number.or.store(Some(native_nb_or));
                }
                SlotAccessor::NbXor => {
                    vtable.xor_func = Some(unsafe { core::mem::transmute(slot.pfunc) });
                    slots.as_number.xor.store(Some(native_nb_xor));
                }
                SlotAccessor::TpDoc => {
                    let doc = unsafe {
                        CStr::from_ptr(slot.pfunc.cast::<c_char>())
                            .to_str()
                            .expect("tp_doc must be a valid UTF-8 string")
                    };
                    slots.doc = Some(doc);
                }
                _ => todo!("Slot {accessor:?} is not yet supported in PyType_FromSpec"),
            }

            slot_ptr = unsafe { slot_ptr.add(1) };
        }

        let has_native_new = vtable.new_func.is_some();
        let has_native_init = vtable.init_func.is_some();
        let class = vm.ctx.new_class(None, class_name, base.to_owned(), slots);
        if let Some(module_name) = module_name {
            class.set_attr(vm.ctx.intern_str("__module__"), vm.ctx.new_str(module_name).into());
        }
        class.init_type_data(vtable).unwrap();
        let class_static: &'static Py<PyType> = Box::leak(Box::new(class.to_owned()));
        for attribute in attributes {
            let name = unsafe {
                CStr::from_ptr(attribute.name)
                    .to_str()
                    .expect("attribute name must be valid UTF-8")
            };
            let closure = attribute.closure;
            let getter = attribute.get;
            let getset = if let Some(setter) = attribute.set {
                todo!();
                unsafe {
                    vm.ctx.new_getset(
                        name,
                        &class,
                        |obj: PyObjectRef, vm: &VirtualMachine| {},
                        |obj: PyObjectRef, value: PyObjectRef, vm: &VirtualMachine| {},
                    )
                }
            } else {
                vm.ctx.new_readonly_getset(
                    name,
                    class_static,
                    move |obj: PyObjectRef, vm: &VirtualMachine| {
                        let exported_obj =
                            unsafe { exported_object_handle(obj.as_raw().cast_mut()) };
                        let result = getter(exported_obj, closure);
                        let result = NonNull::new(result).ok_or_else(|| {
                            vm.take_raised_exception().unwrap_or_else(|| {
                                vm.new_system_error(
                                    "native getset returned NULL without raising".to_owned(),
                                )
                            })
                        })?;
                        let resolved: PyResult<PyObjectRef> =
                            Ok(unsafe { owned_from_exported_new_ref(result.as_ptr()) });
                        resolved
                    },
                )
            };
            class
                .attributes
                .write()
                .insert(vm.ctx.intern_str(name), getset.into_object());
        }
        if class
            .get_type_data::<TypeVTable>()
            .and_then(|vtable| vtable.float_func)
            .is_some()
        {
            class.slots.as_number.float.store(Some(native_nb_float));
        }
        if class
            .get_type_data::<TypeVTable>()
            .and_then(|vtable| vtable.str_func)
            .is_some()
        {
            class.slots.str.store(Some(native_tp_str));
        }
        if class
            .get_type_data::<TypeVTable>()
            .and_then(|vtable| vtable.repr_func)
            .is_some()
        {
            class.slots.repr.store(Some(native_tp_repr));
        }
        if class
            .get_type_data::<TypeVTable>()
            .and_then(|vtable| vtable.contains_func)
            .is_some()
        {
            class
                .slots
                .as_sequence
                .contains
                .store(Some(native_sq_contains));
        }
        if class
            .get_type_data::<TypeVTable>()
            .and_then(|vtable| vtable.sq_length_func)
            .is_some()
        {
            class
                .slots
                .as_sequence
                .length
                .store(Some(native_sq_length));
        }
        if class
            .get_type_data::<TypeVTable>()
            .and_then(|vtable| vtable.iter_func)
            .is_some()
        {
            class.slots.iter.store(Some(native_tp_iter));
        }
        if class
            .get_type_data::<TypeVTable>()
            .and_then(|vtable| vtable.iternext_func)
            .is_some()
        {
            class.slots.iternext.store(Some(native_tp_iternext));
        }
        if class
            .get_type_data::<TypeVTable>()
            .and_then(|vtable| vtable.mp_subscript_func)
            .is_some()
        {
            class
                .slots
                .as_mapping
                .subscript
                .store(Some(native_mp_subscript));
        }
        if class
            .get_type_data::<TypeVTable>()
            .and_then(|vtable| vtable.mp_length_func)
            .is_some()
        {
            class
                .slots
                .as_mapping
                .length
                .store(Some(native_mp_length));
        }
        if class
            .get_type_data::<TypeVTable>()
            .and_then(|vtable| vtable.richcompare_func)
            .is_some()
        {
            class.slots.richcompare.store(Some(native_tp_richcompare));
        }
        if class
            .get_type_data::<TypeVTable>()
            .and_then(|vtable| vtable.hash_func)
            .is_some()
        {
            class.slots.hash.store(Some(native_tp_hash));
        }
        if class
            .get_type_data::<TypeVTable>()
            .and_then(|vtable| vtable.subtract_func)
            .is_some()
        {
            class
                .slots
                .as_number
                .subtract
                .store(Some(native_nb_subtract));
        }
        if class
            .get_type_data::<TypeVTable>()
            .and_then(|vtable| vtable.and_func)
            .is_some()
        {
            class.slots.as_number.and.store(Some(native_nb_and));
        }
        if class
            .get_type_data::<TypeVTable>()
            .and_then(|vtable| vtable.or_func)
            .is_some()
        {
            class.slots.as_number.or.store(Some(native_nb_or));
        }
        if class
            .get_type_data::<TypeVTable>()
            .and_then(|vtable| vtable.xor_func)
            .is_some()
        {
            class.slots.as_number.xor.store(Some(native_nb_xor));
        }
        let ctx: &'static Context = unsafe { &*std::sync::Arc::as_ptr(&vm.ctx) };
        add_operators(class_static, ctx);
        for method in methods {
            let (name, descriptor) = build_tp_method(method, class_static, vm);
            class
                .attributes
                .write()
                .insert(ctx.intern_str(name), descriptor);
        }
        if has_native_new {
            class.slots.new.store(Some(native_tp_new));
        }
        if has_native_init {
            class.slots.init.store(Some(native_tp_init));
        }
        if !has_explicit_getattro {
            class.slots.getattro.store(base.slots.getattro.load());
        }
        if !has_explicit_setattro {
            class.slots.setattro.store(base.slots.setattro.load());
        }
        class
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyType_Freeze(_ty: *mut PyTypeObject) -> c_int {
    // TODO: Implement immutable type freezing semantics.
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn PyObject_GetAttr(obj: *mut PyObject, name: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let obj = unsafe { &*resolve_object_handle(obj) };
        let name = unsafe { &*resolve_object_handle(name) }.try_downcast_ref::<PyStr>(vm)?;
        obj.get_attr(name, vm)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyObject_GetAttrString(
    obj: *mut PyObject,
    attr_name: *const c_char,
) -> *mut PyObject {
    with_vm(|vm| {
        let obj = unsafe { &*resolve_object_handle(obj) };
        let name = unsafe {
            CStr::from_ptr(attr_name)
                .to_str()
                .expect("attribute name must be valid UTF-8")
        };
        obj.get_attr(name, vm)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyObject_SetAttrString(
    obj: *mut PyObject,
    attr_name: *const c_char,
    value: *mut PyObject,
) -> c_int {
    with_vm(|vm| {
        let name = unsafe { CStr::from_ptr(attr_name) }
            .to_str()
            .expect("attribute name must be valid UTF-8");
        let obj = unsafe { &*resolve_object_handle(obj) };
        let value = unsafe { &*resolve_object_handle(value) }.to_owned();
        obj.set_attr(name, value, vm)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyObject_SetAttr(
    obj: *mut PyObject,
    name: *mut PyObject,
    value: *mut PyObject,
) -> c_int {
    with_vm(|vm| {
        let obj = unsafe { &*resolve_object_handle(obj) };
        let name = unsafe { &*resolve_object_handle(name) }.try_downcast_ref::<PyStr>(vm)?;
        let value = unsafe { &*resolve_object_handle(value) }.to_owned();
        obj.set_attr(name, value, vm)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyObject_Repr(obj: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let Some(obj) = NonNull::new(unsafe { resolve_object_handle(obj) }) else {
            return Ok(vm.ctx.new_str("<NULL>"));
        };

        unsafe { obj.as_ref() }.repr(vm)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyObject_Str(obj: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let Some(obj) = NonNull::new(unsafe { resolve_object_handle(obj) }) else {
            return Ok(vm.ctx.new_str("<NULL>"));
        };

        unsafe { obj.as_ref() }.str(vm)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyOS_FSPath(path: *mut PyObject) -> *mut PyObject {
    with_vm(|vm| {
        let path = unsafe { &*resolve_object_handle(path) }.to_owned();
        Ok(FsPath::try_from_path_like(path, true, vm)?.to_pyobject(vm))
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn Py_GetConstantBorrowed(constant_id: c_uint) -> *mut PyObject {
    unsafe {
        let ptr: *mut PyObject = with_vm(|vm| {
            let ctx = &vm.ctx;
            match constant_id {
                0 => ctx.none.as_object(),
                1 => ctx.false_value.as_object(),
                2 => ctx.true_value.as_object(),
                3 => ctx.ellipsis.as_object(),
                4 => ctx.not_implemented.as_object(),
                _ => panic!("Invalid constant_id passed to Py_GetConstantBorrowed"),
            }
            .as_raw()
            .cast_mut()
        });
        exported_object_handle(ptr)
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn PyObject_IsTrue(obj: *mut PyObject) -> c_int {
    with_vm(|vm| {
        let obj = unsafe { &*resolve_object_handle(obj) };
        obj.to_owned().is_true(vm)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyObject_GenericGetDict(
    obj: *mut PyObject,
    _context: *mut c_void,
) -> *mut PyObject {
    with_vm(|vm| {
        let obj = unsafe { &*resolve_object_handle(obj) };
        obj.get_attr("__dict__", vm)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn PyObject_GenericSetDict(
    obj: *mut PyObject,
    value: *mut PyObject,
    _context: *mut c_void,
) -> c_int {
    with_vm(|vm| {
        let obj = unsafe { &*resolve_object_handle(obj) };
        let value = unsafe { &*resolve_object_handle(value) }.to_owned();
        obj.set_attr("__dict__", value, vm)
    })
}

#[cfg(test)]
mod tests {
    use pyo3::prelude::*;
    use pyo3::types::{PyBool, PyDict, PyInt, PyNone, PyString, PyStringMethods};

    #[test]
    fn test_is_truthy() {
        Python::attach(|py| {
            assert!(!py.None().is_truthy(py).unwrap());
        })
    }

    #[test]
    fn test_is_none() {
        Python::attach(|py| {
            assert!(py.None().is_none(py));
        })
    }

    #[test]
    fn test_bool() {
        Python::attach(|py| {
            assert!(PyBool::new(py, true).is_truthy().unwrap());
            assert!(!PyBool::new(py, false).is_truthy().unwrap());
        })
    }

    #[test]
    fn test_type_name() {
        Python::attach(|py| {
            let string = PyString::new(py, "Hello, World!");
            assert_eq!(string.get_type().name().unwrap().to_cow().unwrap(), "str");
        })
    }

    #[test]
    fn test_static_type_pointers() {
        Python::attach(|py| {
            assert!(py.None().bind(py).is_instance_of::<PyNone>());
            assert!(PyBool::new(py, true).is_instance_of::<PyBool>());
        })
    }

    #[test]
    fn test_repr() {
        Python::attach(|py| {
            let module = py.import("sys").unwrap();
            assert_eq!(module.repr().unwrap(), "<module 'sys' (built-in)>");
        })
    }

    #[test]
    fn test_obj_to_str() {
        Python::attach(|py| {
            let number = PyInt::new(py, 42);
            assert_eq!(number.str().unwrap(), "42");
        })
    }

    #[test]
    fn test_get_attr() {
        Python::attach(|py| {
            let sys = py.import("sys").unwrap();
            let implementation = sys
                .getattr("implementation")
                .unwrap()
                .getattr("name")
                .unwrap()
                .str()
                .unwrap();

            assert_eq!(implementation, "rustpython");
        })
    }

    #[test]
    fn test_generic_get_dict() {
        Python::attach(|py| {
            let globals = PyDict::new(py);
            py.run(c"class MyClass: ...", None, Some(&globals)).unwrap();
            let my_class = globals.get_item("MyClass").unwrap().unwrap();
            let instance = my_class.call0().unwrap();
            instance.setattr("foo", 42).unwrap();
            let dict = instance.getattr("__dict__").unwrap();
            assert!(dict.get_item("foo").is_ok());
        })
    }

    #[test]
    fn test_rust_class() {
        #[pyclass]
        struct MyClass {
            #[pyo3(get)]
            num: i32,
        }

        #[pymethods]
        impl MyClass {
            #[new]
            fn new(value: i32) -> Self {
                MyClass { num: value }
            }

            fn method1(&self) -> PyResult<i32> {
                Ok(10)
            }
        }

        Python::attach(|py| {
            let obj = Bound::new(py, MyClass { num: 3 }).unwrap();

            let globals = PyDict::new(py);
            globals.set_item("instance", obj).unwrap();
            py.run(c"assert instance.num == 3", Some(&globals), None)
                .unwrap();
        });
    }
}
