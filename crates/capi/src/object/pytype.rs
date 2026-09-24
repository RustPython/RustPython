use crate::methodobject::{PyMethodDef, build_method_def};
use crate::object::define_py_check;
use crate::pystate::with_vm;
use crate::util::{CStrExt, FfiPtrExt};
use core::ffi::{c_char, c_int, c_ulong, c_void};
use core::ptr;
use rustpython_vm::builtins::{PyStr, PyType};
use rustpython_vm::function::{HeapMethodDef, PyMethodFlags};
use rustpython_vm::object::PyCBody;
use rustpython_vm::object::rustpython_free;
use rustpython_vm::types::{
    CAllocFunc, CDestructor, CFreeFunc, CNewFunc, CSlotId, CSlots, PyTypeFlags, PyTypeSlots,
};
use rustpython_vm::{AsObject, Py, PyObject, PyRef, PyResult, VirtualMachine, identifier};

pub type PyTypeObject = Py<PyType>;

define_py_check!(fn PyType_Check, types.type_type);
define_py_check!(exact fn PyType_CheckExact, types.type_type);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_TYPE(op: *mut PyObject) -> *const PyTypeObject {
    unsafe { op.assume_borrowed() }.class()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Py_IS_TYPE(op: *mut PyObject, ty: *mut PyTypeObject) -> c_int {
    with_vm(|_vm| {
        let obj = unsafe { op.assume_borrowed() };
        let ty = unsafe { ty.assume_borrowed() };
        obj.class().is(ty)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_GetFlags(ptr: *mut PyTypeObject) -> c_ulong {
    let ty = unsafe { ptr.assume_borrowed() };
    ty.slots.flags.bits() as u32 as c_ulong
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_IsSubtype(a: *mut PyTypeObject, b: *mut PyTypeObject) -> c_int {
    with_vm(move |_vm| {
        let a = unsafe { a.assume_borrowed() };
        let b = unsafe { b.assume_borrowed() };
        Ok(a.is_subtype(b))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_GetName(ptr: *mut PyTypeObject) -> *mut PyObject {
    with_vm(|vm| unsafe { ptr.assume_borrowed() }.__name__(vm))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_GetQualName(ptr: *mut PyTypeObject) -> *mut PyObject {
    with_vm(|vm| unsafe { ptr.assume_borrowed() }.__qualname__(vm))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_GetModuleName(ptr: *mut PyTypeObject) -> *mut PyObject {
    with_vm(|vm| unsafe { ptr.assume_borrowed() }.__module__(vm))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_GetFullyQualifiedName(ptr: *mut PyTypeObject) -> *mut PyObject {
    with_vm(|vm| {
        let ty = unsafe { ptr.assume_borrowed() };
        let qualname = ty.__qualname__(vm).try_downcast::<PyStr>(vm)?;
        let module = ty.__module__(vm);

        if let Some(module) = module.downcast_ref::<PyStr>()
            && module.as_wtf8() != "builtins"
        {
            Ok(vm.ctx.new_str(format!("{module}.{qualname}")))
        } else {
            Ok(qualname)
        }
    })
}

// C level type slots.
//
// Slots cross the boundary in both directions. A `newfunc` from an extension
// goes into the type's `CSlots` table and `new` gets the trampoline that
// reads it; the table pointer is inherited alongside the trampoline. A slot
// implemented in Rust is paired with a static C function of that same
// implementation, and `PyType_GetSlot` returns it — for a subclass, the
// base's function — so the caller can run that body with its own subtype.
//
// `__new__` is the same wrapper a native type gets, so a call through it is
// checked by `PyType::__new__` before it reaches the slot.

#[allow(non_camel_case_types)]
pub type newfunc = CNewFunc;

#[allow(non_upper_case_globals)]
pub const Py_tp_new: c_int = CSlotId::TpNew as c_int;

/// Install a C `newfunc` as the type's tp_new.
///
/// `ty` must be a heap type that does not already define `__new__` itself.
pub fn set_tp_new(vm: &VirtualMachine, ty: &Py<PyType>, tp_new: newfunc) -> PyResult<()> {
    let c_slots = CSlots::new();
    c_slots.new.store(Some(tp_new));
    install_c_slots(vm, ty, c_slots)
}

/// Store `c_slots` on `ty` and, when it holds tp_new, publish the `__new__`
/// wrapper that reaches the trampoline.
fn install_c_slots(vm: &VirtualMachine, ty: &Py<PyType>, c_slots: CSlots) -> PyResult<()> {
    let has_new = c_slots.new.load().is_some();
    ty.set_c_slots(c_slots, vm)?;
    if has_new {
        install_new_wrapper(vm, ty);
    }
    Ok(())
}

/// The wrapper that reaches the slot the checked way, as a native type gets
/// from `extend_class`. Stored without the attribute protocol so that
/// `update_slot` does not read it back as a definition of `__new__`.
fn install_new_wrapper(vm: &VirtualMachine, ty: &Py<PyType>) {
    let def = vm
        .ctx
        .new_method_def("__new__", PyType::__new__, PyMethodFlags::METHOD, None);
    let wrapper = def.build_function(vm, Some(ty.to_owned().into()));
    ty.set_attr(identifier!(vm, __new__), wrapper.into());
}

/// The C function installed in `slot`.
///
/// `Py_tp_new` is reported for a function installed from C and for a Rust
/// `tp_new`, including one a subclass inherited. `Py_tp_dealloc` is reported
/// when it was installed from C. `Py_tp_free` is the VM freer, or the
/// function an extension stored. `Py_tp_alloc` is reported only when an
/// extension stored one. Any other id reads as empty.
#[unsafe(no_mangle)]
#[allow(non_upper_case_globals)]
pub unsafe extern "C" fn PyType_GetSlot(ty: *const PyTypeObject, slot: c_int) -> *mut c_void {
    let ty = unsafe { &*ty };
    let c_slots = ty.slots.c_slots();
    match CSlotId::from_raw(slot) {
        Some(CSlotId::TpNew) => ty.c_tp_new().map_or(ptr::null_mut(), |f| f as *mut c_void),
        Some(CSlotId::TpDealloc) => c_slots
            .and_then(|c| c.dealloc.load())
            .map_or(ptr::null_mut(), |f| f as *mut c_void),
        Some(CSlotId::TpFree) => {
            let free = c_slots
                .and_then(|c| c.free.load())
                .unwrap_or(rustpython_free);
            free as *mut c_void
        }
        Some(CSlotId::TpAlloc) => c_slots
            .and_then(|c| c.alloc.load())
            .map_or(ptr::null_mut(), |f| f as *mut c_void),
        None => ptr::null_mut(),
    }
}

/// Outer slot list entry. Layout matches the 3.15 `PySlot`: id, flags, a
/// reserved word, then a 64-bit value.
#[repr(C)]
#[derive(Copy, Clone)]
pub struct PySlot {
    sl_id: u16,
    sl_flags: u16,
    reserved: u32,
    value: PySlotValue,
}

#[repr(C)]
#[derive(Copy, Clone)]
union PySlotValue {
    ptr: *mut c_void,
    uint64: u64,
    size: isize,
}

/// Inner slot list entry, terminated by `slot == 0`.
#[repr(C)]
struct PyType_Slot {
    slot: c_int,
    pfunc: *mut c_void,
}

const PY_TP_ALLOC: c_int = 47;
const PY_TP_BASE: c_int = 48;
const PY_TP_DEALLOC: c_int = 52;
const PY_TP_DOC: c_int = 56;
const PY_TP_METHODS: c_int = 64;
const PY_TP_NEW: c_int = 65;
const PY_TP_FREE: c_int = 74;

const PY_TP_SLOTS: u16 = 93;
const PY_TP_NAME: u16 = 95;
const PY_TP_BASICSIZE: u16 = 96;
const PY_TP_EXTRA_BASICSIZE: u16 = 97;
const PY_TP_FLAGS: u16 = 99;

const TPFLAGS_BASETYPE: u64 = 1 << 10;
const BODY_ALIGN: usize = 16;

fn base_basicsize(cls: &Py<PyType>) -> usize {
    cls.base.deref().map_or(0, |base| base.slots.basicsize)
}

fn align_up_16(size: usize) -> Option<usize> {
    size.checked_add(BODY_ALIGN - 1)
        .map(|size| size & !(BODY_ALIGN - 1))
}

fn slot_size(value: isize, vm: &VirtualMachine) -> PyResult<usize> {
    usize::try_from(value).map_err(|_| vm.new_system_error("negative type size"))
}

struct TypeSpec {
    name: Option<String>,
    flags: u64,
    explicit_basicsize: Option<usize>,
    extra_basicsize: usize,
    base: Option<PyRef<PyType>>,
    tp_new: Option<newfunc>,
    dealloc: Option<CDestructor>,
    free: Option<CFreeFunc>,
    alloc: Option<CAllocFunc>,
    methods: Vec<(String, PyRef<HeapMethodDef>)>,
    doc: Option<String>,
}

fn read_type_spec(vm: &VirtualMachine, slots: *const PySlot) -> PyResult<TypeSpec> {
    if slots.is_null() {
        return Err(vm.new_system_error("type slots are null"));
    }
    let mut spec = TypeSpec {
        name: None,
        flags: 0,
        explicit_basicsize: None,
        extra_basicsize: 0,
        base: None,
        tp_new: None,
        dealloc: None,
        free: None,
        alloc: None,
        methods: Vec::new(),
        doc: None,
    };
    let mut cursor = slots;
    loop {
        let slot = unsafe { &*cursor };
        if slot.sl_id == 0 {
            break;
        }
        match slot.sl_id {
            PY_TP_NAME => {
                let name = unsafe { slot.value.ptr.cast::<c_char>().try_as_str(vm)? };
                spec.name = Some(name.to_owned());
            }
            PY_TP_FLAGS => {
                spec.flags = unsafe { slot.value.uint64 };
            }
            PY_TP_BASICSIZE => {
                spec.explicit_basicsize = Some(slot_size(unsafe { slot.value.size }, vm)?);
            }
            PY_TP_EXTRA_BASICSIZE => {
                spec.extra_basicsize = slot_size(unsafe { slot.value.size }, vm)?;
            }
            PY_TP_SLOTS => read_inner_slots(vm, &mut spec, unsafe { slot.value.ptr })?,
            other => {
                return Err(vm.new_system_error(format!("unsupported type slot {other}")));
            }
        }
        cursor = unsafe { cursor.add(1) };
    }
    Ok(spec)
}

fn read_inner_slots(vm: &VirtualMachine, spec: &mut TypeSpec, slots: *mut c_void) -> PyResult<()> {
    if slots.is_null() {
        return Err(vm.new_system_error("inner type slots are null"));
    }
    let mut cursor = slots.cast::<PyType_Slot>();
    loop {
        let slot = unsafe { &*cursor };
        if slot.slot == 0 {
            break;
        }
        match slot.slot {
            PY_TP_BASE => {
                if slot.pfunc.is_null() {
                    return Err(vm.new_system_error("type base is null"));
                }
                spec.base = Some(unsafe { &*slot.pfunc.cast::<Py<PyType>>() }.to_owned());
            }
            PY_TP_NEW => {
                if !slot.pfunc.is_null() {
                    spec.tp_new =
                        Some(unsafe { core::mem::transmute::<*mut c_void, newfunc>(slot.pfunc) });
                }
            }
            PY_TP_DEALLOC => {
                if !slot.pfunc.is_null() {
                    spec.dealloc = Some(unsafe {
                        core::mem::transmute::<*mut c_void, CDestructor>(slot.pfunc)
                    });
                }
            }
            PY_TP_FREE => {
                if !slot.pfunc.is_null() {
                    spec.free =
                        Some(unsafe { core::mem::transmute::<*mut c_void, CFreeFunc>(slot.pfunc) });
                }
            }
            PY_TP_ALLOC => {
                if !slot.pfunc.is_null() {
                    spec.alloc = Some(unsafe {
                        core::mem::transmute::<*mut c_void, CAllocFunc>(slot.pfunc)
                    });
                }
            }
            PY_TP_METHODS => read_methods(vm, spec, slot.pfunc)?,
            PY_TP_DOC => {
                spec.doc =
                    unsafe { slot.pfunc.cast::<c_char>().try_as_str_opt(vm)? }.map(str::to_owned);
            }
            other => {
                return Err(vm.new_system_error(format!("unsupported type slot {other}")));
            }
        }
        cursor = unsafe { cursor.add(1) };
    }
    Ok(())
}

fn read_methods(vm: &VirtualMachine, spec: &mut TypeSpec, methods: *mut c_void) -> PyResult<()> {
    if methods.is_null() {
        return Ok(());
    }
    let mut cursor = methods.cast::<PyMethodDef>();
    loop {
        let def = unsafe { &*cursor };
        if def.ml_name.is_null() {
            break;
        }
        let name = unsafe { def.ml_name.try_as_str(vm)? };
        let built = build_method_def(vm, def, true)?;
        spec.methods.push((name.to_owned(), built));
        cursor = unsafe { cursor.add(1) };
    }
    Ok(())
}

fn type_flags(spec_flags: u64) -> PyTypeFlags {
    let mut flags = PyTypeFlags::heap_type_flags();
    if spec_flags & TPFLAGS_BASETYPE == 0 {
        flags.remove(PyTypeFlags::BASETYPE);
    }
    flags
}

/// Create a heap type from an outer `PySlot` list.
///
/// The returned pointer is a new reference. `tp_dealloc` is stored and not called.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_FromSlots(slots: *const PySlot) -> *mut crate::PyObject {
    with_vm(|vm| {
        let spec = read_type_spec(vm, slots)?;
        let qualified = spec
            .name
            .as_deref()
            .ok_or_else(|| vm.new_system_error("type name is missing"))?;
        let (module, name) = match qualified.rsplit_once('.') {
            Some((module, name)) => (Some(module), name),
            None => (None, qualified),
        };
        if name.is_empty() {
            return Err(vm.new_system_error("type name is missing"));
        }
        let base = spec
            .base
            .unwrap_or_else(|| vm.ctx.types.object_type.to_owned());
        let basicsize = match spec.explicit_basicsize {
            Some(size) => size,
            None => {
                let aligned = align_up_16(base.slots.basicsize)
                    .ok_or_else(|| vm.new_system_error("type size overflow"))?;
                aligned
                    .checked_add(spec.extra_basicsize)
                    .ok_or_else(|| vm.new_system_error("type size overflow"))?
            }
        };
        let mut slots = PyTypeSlots::default();
        slots.flags = type_flags(spec.flags);
        slots.basicsize = basicsize;
        let ty = PyType::new_heap(
            name,
            vec![base],
            Default::default(),
            slots,
            vm.ctx.types.type_type.to_owned(),
            &vm.ctx,
        )
        .map_err(|msg| vm.new_system_error(format!("failed to create type from slots: {msg}")))?;
        if let Some(module) = module {
            ty.set_attr(identifier!(vm, __module__), vm.ctx.new_str(module).into());
        }
        if let Some(doc) = spec.doc.as_deref() {
            ty.set_attr(identifier!(vm, __doc__), vm.ctx.new_str(doc).into());
        }
        // The descriptor keeps a borrowed class pointer. The type owns the
        // descriptor, and the reference returned below keeps the type alive.
        let class: &'static Py<PyType> = unsafe { &*(&*ty as *const Py<PyType>) };
        for (name, method) in &spec.methods {
            let descriptor = method.build_method(class, vm);
            ty.set_attr(vm.ctx.intern_str(name.as_str()), descriptor.into());
        }
        if spec.tp_new.is_some()
            || spec.dealloc.is_some()
            || spec.free.is_some()
            || spec.alloc.is_some()
        {
            let c_slots = CSlots::new();
            if let Some(tp_new) = spec.tp_new {
                c_slots.new.store(Some(tp_new));
            }
            if let Some(dealloc) = spec.dealloc {
                c_slots.dealloc.store(Some(dealloc));
            }
            if let Some(free) = spec.free {
                c_slots.free.store(Some(free));
            }
            if let Some(alloc) = spec.alloc {
                c_slots.alloc.store(Some(alloc));
            }
            install_c_slots(vm, &ty, c_slots)?;
        }
        Ok::<PyRef<PyType>, _>(ty)
    })
}

/// Pointer to `cls`'s extra bytes inside `obj`'s C body.
///
/// The payload must be a [`PyCBody`], and `obj`'s class must be `cls` or a
/// subclass. The address is `body + align_up(base.basicsize, 16)`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyObject_GetTypeData(
    obj: *mut crate::PyObject,
    cls: *mut PyTypeObject,
) -> *mut c_void {
    with_vm(|vm| -> PyResult<*mut c_void> {
        if obj.is_null() || cls.is_null() {
            return Err(vm.new_type_error("null argument"));
        }
        let obj = unsafe { &*obj };
        let cls = unsafe { &*cls };
        if !obj.class().is_subtype(cls) {
            return Err(vm.new_type_error("object is not an instance of the given type"));
        }
        let body = obj
            .downcast_ref::<PyCBody>()
            .ok_or_else(|| vm.new_type_error("instance has no C body"))?;
        let offset = align_up_16(base_basicsize(cls))
            .ok_or_else(|| vm.new_system_error("type size overflow"))?;
        Ok(unsafe { body.as_mut_ptr().add(offset).cast::<c_void>() })
    })
}

/// Mark a type immutable. Nothing is frozen yet; success lets type creation finish.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_Freeze(_ty: *mut PyTypeObject) -> c_int {
    0
}

/// Bytes of type-specific data: `basicsize - align_up(base.basicsize, 16)`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn PyType_GetTypeDataSize(cls: *mut PyTypeObject) -> isize {
    if cls.is_null() {
        return 0;
    }
    let cls = unsafe { &*cls };
    let Some(aligned) = align_up_16(base_basicsize(cls)) else {
        return 0;
    };
    cls.slots.basicsize.saturating_sub(aligned) as isize
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PyObject;
    use core::ptr::NonNull;
    use pyo3::Python;
    use pyo3::types::{PyAnyMethods, PyInt as PyIntType, PyString, PyStringMethods, PyTypeMethods};
    use rustpython_vm::builtins::{PyInt, PyStrRef, PyTuple, PyTypeRef};
    use rustpython_vm::function::{FuncArgs, KwArgs};
    use rustpython_vm::vm::thread::{current_vm_is_set, with_current_vm};
    use rustpython_vm::{AsObject, PyObjectRef, PyRef};

    #[test]
    fn type_name() {
        Python::attach(|py| {
            let string = PyString::new(py, "Hello, World!");
            assert_eq!(string.get_type().name().unwrap().to_str().unwrap(), "str");
        })
    }

    #[test]
    fn type_get_module_name() {
        Python::attach(|py| {
            assert_eq!(
                py.get_type::<PyIntType>()
                    .module()
                    .unwrap()
                    .to_str()
                    .unwrap(),
                "builtins"
            );
        })
    }

    fn load_tp_new(ty: &PyTypeObject) -> newfunc {
        let slot = unsafe { PyType_GetSlot(ty, Py_tp_new) };
        assert!(!slot.is_null());
        unsafe { core::mem::transmute::<*mut c_void, newfunc>(slot) }
    }

    fn call_tp_new(tp_new: newfunc, subtype: &PyTypeObject, args: PyRef<PyTuple>) -> PyObjectRef {
        let ptr = unsafe {
            tp_new(
                core::ptr::from_ref(subtype).cast_mut(),
                args.as_object().as_raw().cast_mut(),
                core::ptr::null_mut(),
            )
        };
        let Some(ptr) = NonNull::new(ptr) else {
            let msg = with_current_vm(|vm| {
                vm.take_raised_exception()
                    .and_then(|exc| exc.as_object().str(vm).ok())
                    .map_or_else(|| "no exception".to_owned(), |msg| msg.to_string())
            });
            panic!("tp_new returned NULL: {msg}");
        };
        unsafe { PyObjectRef::from_raw(ptr) }
    }

    /// `PyType_GetSlot(base, Py_tp_new)(subtype, args, NULL)`.
    ///
    /// A missing slot sets TypeError ("base type without tp_new") and returns NULL.
    /// The pointer from the base, including NULL, is returned unchanged.
    fn call_base_tp_new(
        vm: &VirtualMachine,
        base: &PyTypeObject,
        subtype: *mut PyTypeObject,
        args: &PyRef<PyTuple>,
    ) -> *mut PyObject {
        let slot = unsafe { PyType_GetSlot(base, Py_tp_new) };
        if slot.is_null() {
            vm.set_exception(Some(vm.new_type_error("base type without tp_new")));
            return core::ptr::null_mut();
        }
        let tp_new = unsafe { core::mem::transmute::<*mut c_void, newfunc>(slot) };
        unsafe {
            tp_new(
                subtype,
                args.as_object().as_raw().cast_mut(),
                core::ptr::null_mut(),
            )
        }
    }

    /// Returns `(subtype.__name__, args, kwds is NULL)`.
    unsafe extern "C" fn echo_new(
        subtype: *mut PyTypeObject,
        args: *mut PyObject,
        kwds: *mut PyObject,
    ) -> *mut PyObject {
        assert!(current_vm_is_set());
        with_current_vm(|vm| {
            let subtype = unsafe { &*subtype };
            let args = unsafe { &*args }.to_owned();
            let echo = vm.ctx.new_tuple(vec![
                vm.ctx.new_str(subtype.name().to_string()).into(),
                args,
                vm.ctx.new_bool(kwds.is_null()).into(),
            ]);
            PyObjectRef::from(echo).into_raw().as_ptr()
        })
    }

    /// Returns an empty tuple, so it is told apart from `echo_new` by result.
    unsafe extern "C" fn other_new(
        _subtype: *mut PyTypeObject,
        _args: *mut PyObject,
        _kwds: *mut PyObject,
    ) -> *mut PyObject {
        with_current_vm(|vm| {
            PyObjectRef::from(vm.ctx.new_tuple(vec![]))
                .into_raw()
                .as_ptr()
        })
    }

    unsafe extern "C" fn new_with_object_base(
        subtype: *mut PyTypeObject,
        _args: *mut PyObject,
        _kwds: *mut PyObject,
    ) -> *mut PyObject {
        with_current_vm(|vm| {
            call_base_tp_new(vm, vm.ctx.types.object_type, subtype, &vm.ctx.empty_tuple)
        })
    }

    unsafe extern "C" fn new_with_int_base(
        subtype: *mut PyTypeObject,
        _args: *mut PyObject,
        _kwds: *mut PyObject,
    ) -> *mut PyObject {
        with_current_vm(|vm| {
            call_base_tp_new(vm, vm.ctx.types.int_type, subtype, &vm.ctx.empty_tuple)
        })
    }

    /// Same as [`new_with_int_base`], but the base receives `("nope",)`.
    unsafe extern "C" fn new_with_int_base_nope(
        subtype: *mut PyTypeObject,
        _args: *mut PyObject,
        _kwds: *mut PyObject,
    ) -> *mut PyObject {
        with_current_vm(|vm| {
            let args = vm.ctx.new_tuple(vec![vm.ctx.new_str("nope").into()]);
            call_base_tp_new(vm, vm.ctx.types.int_type, subtype, &args)
        })
    }

    fn heap_type(name: &str, base: &Py<PyType>, vm: &VirtualMachine) -> PyTypeRef {
        PyType::new_simple_heap(name, base, &vm.ctx).unwrap()
    }

    fn call(ty: &Py<PyType>, args: FuncArgs, vm: &VirtualMachine) -> PyRef<PyTuple> {
        ty.as_object()
            .call(args, vm)
            .unwrap()
            .downcast::<PyTuple>()
            .unwrap()
    }

    fn echoed_name(echo: &PyTuple) -> String {
        let name: PyStrRef = echo[0].clone().downcast().unwrap();
        name.to_string()
    }

    #[test]
    fn c_tp_new_is_called() {
        Python::attach(|_py| {
            with_current_vm(|vm| {
                let ty = heap_type("CType", vm.ctx.types.object_type, vm);
                set_tp_new(vm, &ty, echo_new).unwrap();

                let echo = call(&ty, FuncArgs::default(), vm);
                assert_eq!(echoed_name(&echo), "CType");
                assert_eq!(echo[1].clone().downcast::<PyTuple>().unwrap().len(), 0);
                // No keywords were passed, so kwds must be NULL.
                assert!(echo[2].clone().try_to_bool(vm).unwrap());

                let echo = call(&ty, FuncArgs::from(vec![vm.ctx.new_int(1).into()]), vm);
                assert_eq!(echo[1].clone().downcast::<PyTuple>().unwrap().len(), 1);

                let kwargs: KwArgs =
                    core::iter::once(("k".to_owned(), vm.ctx.new_int(2).into())).collect();
                let echo = call(&ty, FuncArgs::new(vec![], kwargs), vm);
                assert!(!echo[2].clone().try_to_bool(vm).unwrap());
            })
        })
    }

    /// A subclass reaches the C slot through the inherited slot pair, and the
    /// type it is instantiated with is the one handed to the slot.
    #[test]
    fn c_tp_new_is_inherited() {
        Python::attach(|_py| {
            with_current_vm(|vm| {
                let base = heap_type("CBase", vm.ctx.types.object_type, vm);
                set_tp_new(vm, &base, echo_new).unwrap();
                let sub = heap_type("CSub", &base, vm);

                assert!(sub.slots.c_slots().and_then(|c| c.new.load()).is_some());
                assert_eq!(echoed_name(&call(&sub, FuncArgs::default(), vm)), "CSub");
            })
        })
    }

    /// Every C type reaches its slot through one shared trampoline, so the
    /// "is not safe" check has to compare the C functions behind it, not the
    /// trampoline. Without that, `CBase.__new__(CSub)` would silently run
    /// CSub's tp_new where the caller asked for CBase's.
    #[test]
    fn cross_type_dunder_new_call_is_rejected() {
        Python::attach(|_py| {
            with_current_vm(|vm| {
                let base = heap_type("COuter", vm.ctx.types.object_type, vm);
                set_tp_new(vm, &base, echo_new).unwrap();
                let sub = heap_type("CInner", &base, vm);
                set_tp_new(vm, &sub, other_new).unwrap();

                // Each type reaches its own C function.
                assert_eq!(echoed_name(&call(&base, FuncArgs::default(), vm)), "COuter");
                assert!(call(&sub, FuncArgs::default(), vm).is_empty());

                // But COuter.__new__(CInner) must not reach CInner's.
                let dunder_new = base
                    .as_object()
                    .get_attr(identifier!(vm, __new__), vm)
                    .unwrap();
                let err = dunder_new.call((sub,), vm).unwrap_err();
                let msg = err.as_object().str(vm).unwrap().to_string();
                assert!(
                    msg.contains("is not safe"),
                    "expected an is-not-safe error, got {msg}"
                );
            })
        })
    }

    /// `__new__` reaches the slot through `PyType::__new__`, so a direct call
    /// is argument-checked instead of handing the raw pointer to the callee.
    #[test]
    fn direct_dunder_new_call_is_checked() {
        Python::attach(|_py| {
            with_current_vm(|vm| {
                let ty = heap_type("CChecked", vm.ctx.types.object_type, vm);
                set_tp_new(vm, &ty, echo_new).unwrap();
                let dunder_new = ty
                    .as_object()
                    .get_attr(identifier!(vm, __new__), vm)
                    .unwrap();

                // No type argument at all.
                assert!(dunder_new.call((), vm).is_err());
                // First argument is not a type.
                assert!(dunder_new.call((vm.ctx.new_int(42),), vm).is_err());
                // First argument is a type, but not a subtype of this one.
                assert!(
                    dunder_new
                        .call((vm.ctx.types.dict_type.to_owned(),), vm)
                        .is_err()
                );

                // The type itself and a subclass of it are accepted.
                let echo = dunder_new
                    .call((ty.clone(),), vm)
                    .unwrap()
                    .downcast::<PyTuple>()
                    .unwrap();
                assert_eq!(echoed_name(&echo), "CChecked");

                let sub = heap_type("CCheckedSub", &ty, vm);
                let echo = dunder_new
                    .call((sub,), vm)
                    .unwrap()
                    .downcast::<PyTuple>()
                    .unwrap();
                assert_eq!(echoed_name(&echo), "CCheckedSub");
            })
        })
    }

    /// A Python-level `__new__` on a subclass replaces the inherited C slot.
    #[test]
    fn python_subclass_new_overrides_the_slot() {
        Python::attach(|_py| {
            with_current_vm(|vm| {
                let base = heap_type("COverBase", vm.ctx.types.object_type, vm);
                set_tp_new(vm, &base, echo_new).unwrap();
                let sub = heap_type("COverSub", &base, vm);

                let py_new = vm.ctx.new_method_def(
                    "__new__",
                    |_args: FuncArgs, vm: &VirtualMachine| -> PyResult {
                        Ok(vm.ctx.new_str("from python").into())
                    },
                    PyMethodFlags::STATIC,
                    None,
                );
                let py_new = py_new.build_function(vm, None);
                sub.as_object()
                    .set_attr(identifier!(vm, __new__), py_new, vm)
                    .unwrap();

                assert!(sub.slots.c_slots().is_none());
                let obj = sub.as_object().call((), vm).unwrap();
                let obj: PyStrRef = obj.downcast().unwrap();
                assert_eq!(obj.to_string(), "from python");
            })
        })
    }

    #[test]
    fn get_slot_round_trips() {
        Python::attach(|_py| {
            with_current_vm(|vm| {
                let ty = heap_type("CGet", vm.ctx.types.object_type, vm);
                // Inherits object's Rust tp_new until a C function is installed.
                assert_eq!(unsafe { PyType_GetSlot(&*ty, Py_tp_new) }, unsafe {
                    PyType_GetSlot(vm.ctx.types.object_type, Py_tp_new)
                },);

                set_tp_new(vm, &ty, echo_new).unwrap();
                assert_eq!(
                    unsafe { PyType_GetSlot(&*ty, Py_tp_new) },
                    echo_new as *mut c_void
                );

                // Inherited by pointer, as tp_new is.
                let sub = heap_type("CGetSub", &ty, vm);
                assert_eq!(
                    unsafe { PyType_GetSlot(&*sub, Py_tp_new) },
                    echo_new as *mut c_void
                );
            })
        })
    }

    #[test]
    fn rust_int_tp_new_builds_an_int_and_a_subclass() {
        Python::attach(|_py| {
            with_current_vm(|vm| {
                let int_type = vm.ctx.types.int_type;
                let tp_new = load_tp_new(int_type);

                let args = vm.ctx.new_tuple(vec![vm.ctx.new_str("42").into()]);
                let value: PyRef<PyInt> = call_tp_new(tp_new, int_type, args).downcast().unwrap();
                assert_eq!(value.as_bigint().to_string(), "42");

                let sub = heap_type("SubInt", int_type, vm);
                let args = vm.ctx.new_tuple(vec![vm.ctx.new_str("42").into()]);
                let obj = call_tp_new(tp_new, &sub, args);
                assert!(obj.class().is(&sub));
            })
        })
    }

    #[test]
    fn rust_object_tp_new_is_callable() {
        Python::attach(|_py| {
            with_current_vm(|vm| {
                let object_type = vm.ctx.types.object_type;
                let tp_new = load_tp_new(object_type);
                let obj = call_tp_new(tp_new, object_type, vm.ctx.new_tuple(vec![]));
                assert!(obj.class().is(object_type));
            })
        })
    }

    #[test]
    fn inherited_int_tp_new_matches_int() {
        Python::attach(|_py| {
            with_current_vm(|vm| {
                let int_type = vm.ctx.types.int_type;
                let int_slot = unsafe { PyType_GetSlot(int_type, Py_tp_new) };
                let sub = heap_type("InheritedInt", int_type, vm);
                assert!(!int_slot.is_null());
                assert_eq!(unsafe { PyType_GetSlot(&*sub, Py_tp_new) }, int_slot);
            })
        })
    }

    #[test]
    fn python_level_new_is_callable_from_c() {
        Python::attach(|_py| {
            with_current_vm(|vm| {
                let sub = heap_type("PyNew", vm.ctx.types.object_type, vm);
                let py_new = vm.ctx.new_method_def(
                    "__new__",
                    |_args: FuncArgs, vm: &VirtualMachine| -> PyResult {
                        Ok(vm.ctx.new_str("from python").into())
                    },
                    PyMethodFlags::STATIC,
                    None,
                );
                let py_new = py_new.build_function(vm, None);
                sub.as_object()
                    .set_attr(identifier!(vm, __new__), py_new, vm)
                    .unwrap();

                let tp_new = load_tp_new(&sub);
                let obj = call_tp_new(tp_new, &sub, vm.ctx.new_tuple(vec![]));
                let obj: PyStrRef = obj.downcast().unwrap();
                assert_eq!(obj.to_string(), "from python");
            })
        })
    }

    #[test]
    fn rust_int_tp_new_sets_value_error() {
        Python::attach(|_py| {
            with_current_vm(|vm| {
                let int_type = vm.ctx.types.int_type;
                let tp_new = load_tp_new(int_type);
                let args = vm
                    .ctx
                    .new_tuple(vec![vm.ctx.new_str("not a number").into()]);
                let ptr = unsafe {
                    tp_new(
                        core::ptr::from_ref(int_type).cast_mut(),
                        args.as_object().as_raw().cast_mut(),
                        core::ptr::null_mut(),
                    )
                };
                assert!(ptr.is_null());
                let exc = vm.take_raised_exception().expect("exception pending");
                assert!(exc.fast_isinstance(vm.ctx.exceptions.value_error));
            })
        })
    }

    /// pyo3's `PyNativeTypeInitializer` calls the base `tp_new` with the subtype
    /// it was given.
    #[test]
    fn extension_tp_new_reaches_object_tp_new_with_its_subtype() {
        Python::attach(|_py| {
            with_current_vm(|vm| {
                let ext = heap_type("Ext", vm.ctx.types.object_type, vm);
                set_tp_new(vm, &ext, new_with_object_base).unwrap();

                let obj = ext.as_object().call((), vm).unwrap();
                assert!(obj.class().is(&ext));

                let sub = heap_type("ExtSub", &ext, vm);
                let sub_obj = sub.as_object().call((), vm).unwrap();
                assert!(sub_obj.class().is(&sub));
            })
        })
    }

    #[test]
    fn extension_tp_new_reaches_int_tp_new_with_its_subtype() {
        Python::attach(|_py| {
            with_current_vm(|vm| {
                let ext = heap_type("Ext", vm.ctx.types.int_type, vm);
                set_tp_new(vm, &ext, new_with_int_base).unwrap();

                let obj = ext.as_object().call((), vm).unwrap();
                assert!(obj.class().is(&ext));
                let value: PyRef<PyInt> = obj.downcast().unwrap();
                assert_eq!(value.as_bigint().to_string(), "0");

                let sub = heap_type("ExtIntSub", &ext, vm);
                let sub_obj = sub.as_object().call((), vm).unwrap();
                assert!(sub_obj.class().is(&sub));
            })
        })
    }

    #[test]
    fn extension_type_reports_its_own_c_tp_new_not_the_base() {
        Python::attach(|_py| {
            with_current_vm(|vm| {
                let object_type = vm.ctx.types.object_type;
                let ext = heap_type("Ext", object_type, vm);
                set_tp_new(vm, &ext, new_with_object_base).unwrap();

                let ext_slot = unsafe { PyType_GetSlot(&*ext, Py_tp_new) };
                let base_slot = unsafe { PyType_GetSlot(object_type, Py_tp_new) };
                assert_eq!(ext_slot, new_with_object_base as *mut c_void);
                assert_ne!(ext_slot, base_slot);
                assert!(!base_slot.is_null());
            })
        })
    }

    #[test]
    fn extension_tp_new_propagates_base_error() {
        Python::attach(|_py| {
            with_current_vm(|vm| {
                let ext = heap_type("Ext", vm.ctx.types.int_type, vm);
                set_tp_new(vm, &ext, new_with_int_base_nope).unwrap();

                let err = ext.as_object().call((), vm).unwrap_err();
                assert!(err.fast_isinstance(vm.ctx.exceptions.value_error));
            })
        })
    }
}
