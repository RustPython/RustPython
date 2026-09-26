use super::{PyStr, PyStrInterned, PyTuple, PyType};
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    builtins::{PyTypeRef, builtin_func::PyNativeMethod, type_},
    class::PyClassImpl,
    common::hash::PyHash,
    convert::{ToPyObject, ToPyResult},
    function::{ArgSize, Callee, FuncArgs, PyMethodDef, PyMethodFlags, PySetterValue},
    protocol::{PyNumberBinaryFunc, PyNumberTernaryFunc, PyNumberUnaryFunc},
    types::{
        Callable, Comparable, DelFunc, DescrGetFunc, DescrSetFunc, GenericMethod, GetDescriptor,
        GetattroFunc, HashFunc, Hashable, InitFunc, IterFunc, IterNextFunc, MapAssSubscriptFunc,
        MapLenFunc, MapSubscriptFunc, PyComparisonOp, Representable, RichCompareFunc,
        SeqAssItemFunc, SeqConcatFunc, SeqContainsFunc, SeqItemFunc, SeqLenFunc, SeqRepeatFunc,
        SetattroFunc, StringifyFunc,
    },
};
use rustpython_common::lock::PyRwLock;

#[derive(Debug)]
pub struct PyDescriptor {
    pub typ: &'static Py<PyType>,
    pub name: &'static PyStrInterned,
    pub qualname: PyRwLock<Option<String>>,
}

#[derive(Debug)]
pub struct PyDescriptorOwned {
    pub typ: PyRef<PyType>,
    pub name: &'static PyStrInterned,
    pub qualname: PyRwLock<Option<String>>,
}

#[pyclass(name = "method_descriptor", module = false)]
pub struct PyMethodDescriptor {
    #[pymember(name = "__objclass__", path = "typ")]
    #[pymember(name = "__name__", path = "name")]
    pub common: PyDescriptor,
    pub method: &'static PyMethodDef,
    // vectorcall: vector_call_func,
    /// Prevent HeapMethodDef from being freed while this descriptor references it
    pub(crate) _method_def_owner: Option<PyObjectRef>,
}

impl PyMethodDescriptor {
    pub fn new(method: &'static PyMethodDef, typ: &'static Py<PyType>, ctx: &Context) -> Self {
        Self {
            common: PyDescriptor {
                typ,
                name: ctx.intern_str(method.name),
                qualname: PyRwLock::new(None),
            },
            method,
            _method_def_owner: None,
        }
    }
}

impl PyPayload for PyMethodDescriptor {
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.method_descriptor_type
    }
}

impl core::fmt::Debug for PyMethodDescriptor {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "method descriptor for '{}'", self.common.name)
    }
}

impl GetDescriptor for PyMethodDescriptor {
    fn descr_get(
        zelf: &PyObject,
        obj: Option<&PyObject>,
        cls: Option<&PyObject>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let descr = Self::_as_pyref(zelf, vm).unwrap();
        let bound = match obj {
            Some(obj) => {
                if descr.method.flags.contains(PyMethodFlags::METHOD) {
                    if cls
                        .as_ref()
                        .is_none_or(|c| c.fast_isinstance(vm.ctx.types.type_type))
                    {
                        obj.to_owned()
                    } else {
                        return Err(vm.new_type_error(format!(
                            "descriptor '{}' needs a type, not '{}', as arg 2",
                            descr.common.name.as_str(),
                            obj.class().name()
                        )));
                    }
                } else if descr.method.flags.contains(PyMethodFlags::CLASS) {
                    obj.class().to_owned().into()
                } else {
                    obj.to_owned()
                }
            }
            None if descr.method.flags.contains(PyMethodFlags::CLASS) => cls.unwrap().to_owned(),
            None => return Ok(zelf.to_owned()),
        };
        Ok(descr.bind(bound, &vm.ctx).into())
    }
}

impl Callable for PyMethodDescriptor {
    type Args = FuncArgs;
    #[inline]
    fn call(zelf: &Py<Self>, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        (zelf.method.func)(
            vm,
            args,
            Callee::named(zelf.method.name).with_instance_arg(true),
        )
    }
}

impl PyMethodDescriptor {
    pub fn bind(&self, obj: PyObjectRef, ctx: &Context) -> PyRef<PyNativeMethod> {
        self.method.build_bound_method(ctx, obj, self.common.typ)
    }
}

#[pyclass(
    with(GetDescriptor, Callable, Representable),
    flags(METHOD_DESCRIPTOR, DISALLOW_INSTANTIATION)
)]
impl PyMethodDescriptor {
    #[pygetset]
    fn __qualname__(&self) -> String {
        format!("{}.{}", self.common.typ.name(), self.common.name)
    }

    #[pygetset]
    fn __doc__(&self) -> Option<&'static str> {
        let doc = self.method.doc?;
        type_::get_doc_from_internal_doc(self.method.name, doc)
    }

    #[pygetset]
    fn __text_signature__(&self) -> Option<&'static str> {
        let doc = self.method.doc?;
        type_::get_text_signature_from_internal_doc(self.method.name, doc)
    }

    #[pymethod]
    fn __reduce__(
        &self,
        vm: &VirtualMachine,
    ) -> (Option<PyObjectRef>, (Option<PyObjectRef>, &'static str)) {
        let builtins_getattr = vm.builtins.get_attr("getattr", vm).ok();
        let classname = vm.builtins.get_attr(&self.common.typ.__name__(vm), vm).ok();
        (builtins_getattr, (classname, self.method.name))
    }
}

impl Representable for PyMethodDescriptor {
    #[inline]
    fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
        Ok(format!(
            "<method '{}' of '{}' objects>",
            zelf.method.name,
            zelf.common.typ.name()
        ))
    }
}

/// METH_CLASS descriptors. Same layout as method_descriptor; a distinct type.
#[pyclass(name = "classmethod_descriptor", module = false)]
pub struct PyClassMethodDescriptor {
    #[pymember(name = "__objclass__", path = "typ")]
    #[pymember(name = "__name__", path = "name")]
    pub common: PyDescriptor,
    pub method: &'static PyMethodDef,
    pub(crate) _method_def_owner: Option<PyObjectRef>,
}

impl PyClassMethodDescriptor {
    pub fn new(method: &'static PyMethodDef, typ: &'static Py<PyType>, ctx: &Context) -> Self {
        Self {
            common: PyDescriptor {
                typ,
                name: ctx.intern_str(method.name),
                qualname: PyRwLock::new(None),
            },
            method,
            _method_def_owner: None,
        }
    }

    pub fn bind(&self, obj: PyObjectRef, ctx: &Context) -> PyRef<PyNativeMethod> {
        self.method.build_bound_method(ctx, obj, self.common.typ)
    }
}

impl PyPayload for PyClassMethodDescriptor {
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.classmethod_descriptor_type
    }
}

impl core::fmt::Debug for PyClassMethodDescriptor {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "classmethod descriptor for '{}'", self.common.name)
    }
}

impl GetDescriptor for PyClassMethodDescriptor {
    fn descr_get(
        zelf: &PyObject,
        obj: Option<&PyObject>,
        cls: Option<&PyObject>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let descr = Self::_as_pyref(zelf, vm).unwrap();
        let type_obj = match cls {
            Some(typ) => typ.to_owned(),
            None => match obj {
                Some(o) => o.class().to_owned().into(),
                None => {
                    return Err(vm.new_type_error(format!(
                        "descriptor '{}' for type '{}' needs either an object or a type",
                        descr.common.name,
                        descr.common.typ.name()
                    )));
                }
            },
        };
        if !type_obj.fast_isinstance(vm.ctx.types.type_type) {
            return Err(vm.new_type_error(format!(
                "descriptor '{}' for type '{}' needs a type, not '{}' as arg 2",
                descr.common.name,
                descr.common.typ.name(),
                type_obj.class().name()
            )));
        }
        let typ = type_obj.downcast::<PyType>().map_err(|obj| {
            vm.new_type_error(format!(
                "descriptor '{}' for type '{}' needs a type, not '{}' as arg 2",
                descr.common.name,
                descr.common.typ.name(),
                obj.class().name()
            ))
        })?;
        if !typ.fast_issubclass(descr.common.typ) {
            return Err(vm.new_type_error(format!(
                "descriptor '{}' requires a subtype of '{}' but received '{}'",
                descr.common.name,
                descr.common.typ.name(),
                typ.name()
            )));
        }
        Ok(descr.bind(typ.into(), &vm.ctx).into())
    }
}

impl Callable for PyClassMethodDescriptor {
    type Args = FuncArgs;
    fn call(zelf: &Py<Self>, mut args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        let Some(owner) = args.args.first().cloned() else {
            return Err(vm.new_type_error(format!(
                "descriptor '{}' of '{}' object needs an argument",
                zelf.method.name,
                zelf.common.typ.name()
            )));
        };
        let bound = Self::descr_get(zelf.as_object(), None, Some(&owner), vm)?;
        args.args.remove(0);
        bound.call(args, vm)
    }
}

#[pyclass(
    with(GetDescriptor, Callable, Representable),
    flags(DISALLOW_INSTANTIATION)
)]
impl PyClassMethodDescriptor {
    #[pygetset]
    fn __qualname__(&self) -> String {
        format!("{}.{}", self.common.typ.name(), self.common.name)
    }

    #[pygetset]
    fn __doc__(&self) -> Option<&'static str> {
        let doc = self.method.doc?;
        type_::get_doc_from_internal_doc(self.method.name, doc)
    }

    #[pygetset]
    fn __text_signature__(&self) -> Option<&'static str> {
        let doc = self.method.doc?;
        type_::get_text_signature_from_internal_doc(self.method.name, doc)
    }
}

impl Representable for PyClassMethodDescriptor {
    #[inline]
    fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
        Ok(format!(
            "<method '{}' of '{}' objects>",
            zelf.method.name,
            zelf.common.typ.name()
        ))
    }
}

/// Member type. Discriminants match `PyMemberDef.type`.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(i32)]
pub enum MemberKind {
    Int = 1,
    Double = 4,
    Object = 6,
    Uint = 11,
    Bool = 14,
    ObjectEx = 16,
}

impl MemberKind {
    #[must_use]
    pub fn from_i32(value: i32) -> Option<Self> {
        match value {
            1 => Some(Self::Int),
            4 => Some(Self::Double),
            6 => Some(Self::Object),
            11 => Some(Self::Uint),
            14 => Some(Self::Bool),
            16 => Some(Self::ObjectEx),
            _ => None,
        }
    }
}

pub const PY_READONLY: i32 = 1;
#[doc(hidden)]
pub const PY_AUDIT_READ: i32 = 2;
pub const PY_RELATIVE_OFFSET: i32 = 8;

/// A byte at `object + offset` is a bool the generic member code can load.
#[doc(hidden)]
pub trait BoolMember {}
impl BoolMember for bool {}
impl BoolMember for core::sync::atomic::AtomicBool {}

/// A writable bool member. Only an atomic cell may change after publication.
#[doc(hidden)]
pub trait BoolCell: BoolMember {}
impl BoolCell for core::sync::atomic::AtomicBool {}

/// A field at `object + offset` is a C `int`.
#[doc(hidden)]
pub trait IntMember {}
impl IntMember for i32 {}
impl IntMember for core::sync::atomic::AtomicI32 {}

/// A writable `int` member. Only an atomic cell may change after publication.
#[doc(hidden)]
pub trait IntCell: IntMember {}
impl IntCell for core::sync::atomic::AtomicI32 {}

/// A field at `object + offset` is a C `unsigned int`.
#[doc(hidden)]
pub trait UintMember {}
impl UintMember for u32 {}
impl UintMember for core::sync::atomic::AtomicU32 {}

/// A writable `unsigned int` member. Only an atomic cell may change after publication.
#[doc(hidden)]
pub trait UintCell: UintMember {}
impl UintCell for core::sync::atomic::AtomicU32 {}

/// A field at `object + offset` is one object pointer.
///
/// Readonly members may be a plain pointer. Writable members are an atomic
/// cell: `PyAtomicRef<PyObject>` when the pointer is never null, or
/// `PyAtomicRef<Option<PyObject>>` when it may be.
#[doc(hidden)]
pub trait ObjectMember {}
impl ObjectMember for PyObjectRef {}
impl ObjectMember for Option<PyObjectRef> {}
impl<T> ObjectMember for PyRef<T> {}
impl<T> ObjectMember for Option<PyRef<T>> {}
impl ObjectMember for crate::object::PyAtomicRef<PyObject> {}
impl ObjectMember for crate::object::PyAtomicRef<Option<PyObject>> {}
impl<T: PyPayload> ObjectMember for crate::object::PyAtomicRef<Option<T>> {}
impl ObjectMember for &'static crate::builtins::PyStrInterned {}
impl<T: PyPayload> ObjectMember for &'static Py<T> {}

/// A writable object member. The cell owns the pointer and updates it atomically.
#[doc(hidden)]
pub trait ObjectCell: ObjectMember {}
impl ObjectCell for crate::object::PyAtomicRef<PyObject> {}
impl ObjectCell for crate::object::PyAtomicRef<Option<PyObject>> {}

/// A field at `object + offset` is an `f64` or the bits of one.
#[doc(hidden)]
pub trait DoubleMember {}
impl DoubleMember for f64 {}
impl DoubleMember for core::sync::atomic::AtomicU64 {}

/// A writable double member. The cell stores the `f64` bits and may change
/// after publication, so only an atomic 64-bit cell is accepted.
#[doc(hidden)]
pub trait DoubleCell: DoubleMember {}
impl DoubleCell for core::sync::atomic::AtomicU64 {}

/// Where `PyMemberDef.offset` points.
///
/// `Offset` is a byte offset from the object to the field.
/// `TupleItem` is a struct-sequence element index: the elements live in the
/// tuple payload, not as separately addressable fields.
#[derive(Clone, Copy, Debug)]
pub enum MemberAccess {
    Offset,
    TupleItem,
}

/// Same fields as `PyMemberDef`: name, type, offset, flags, doc.
pub struct PyMemberDef {
    pub name: String,
    pub kind: MemberKind,
    pub offset: isize,
    pub flags: i32,
    pub doc: Option<String>,
}

impl PyMemberDef {
    pub(crate) fn readonly(&self) -> bool {
        self.flags & PY_READONLY != 0
    }

    pub(crate) fn audit_read(&self) -> bool {
        self.flags & PY_AUDIT_READ != 0
    }
}

impl core::fmt::Debug for PyMemberDef {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PyMemberDef")
            .field("name", &self.name)
            .field("kind", &self.kind)
            .field("offset", &self.offset)
            .field("flags", &self.flags)
            .field("doc", &self.doc)
            .finish()
    }
}

/// Const-constructible member spec. Registered as a `PyMemberDef`.
#[derive(Clone, Copy)]
pub struct PyMemberSpec {
    pub name: &'static str,
    pub kind: MemberKind,
    pub offset: isize,
    pub flags: i32,
    pub doc: Option<&'static str>,
}

impl PyMemberSpec {
    /// Concatenate cfg-gated member groups into one table.
    #[must_use]
    pub const fn concat<const N: usize>(parts: &[&[Self]]) -> [Self; N] {
        const EMPTY: PyMemberSpec = PyMemberSpec {
            name: "",
            kind: MemberKind::Object,
            offset: 0,
            flags: 0,
            doc: None,
        };
        let mut out = [EMPTY; N];
        let mut index = 0;
        let mut part_index = 0;
        while part_index < parts.len() {
            let part = parts[part_index];
            let mut item_index = 0;
            while item_index < part.len() {
                out[index] = part[item_index];
                index += 1;
                item_index += 1;
            }
            part_index += 1;
        }
        out
    }
}

// = PyMemberDescrObject
#[pyclass(name = "member_descriptor", module = false)]
#[derive(Debug)]
pub struct PyMemberDescriptor {
    #[pymember(name = "__objclass__", path = "typ")]
    #[pymember(name = "__name__", path = "name")]
    pub common: PyDescriptorOwned,
    pub member: PyMemberDef,
    pub access: MemberAccess,
}

impl PyMemberDescriptor {
    /// Byte offset of an object-pointer member. `None` for bool, float, and
    /// struct-sequence indexes, which slot specialization must not treat as cells.
    pub(crate) fn slot_offset(&self) -> Option<isize> {
        if matches!(self.access, MemberAccess::TupleItem) {
            return None;
        }
        match self.member.kind {
            MemberKind::Object | MemberKind::ObjectEx => Some(self.member.offset),
            MemberKind::Bool | MemberKind::Double | MemberKind::Int | MemberKind::Uint => None,
        }
    }

    fn get(&self, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if self.member.flags & PY_AUDIT_READ != 0 {
            vm.audit("object.__getattr__", || {
                (obj.clone(), vm.ctx.new_str(self.member.name.as_str()))
            })?;
        }
        match self.access {
            MemberAccess::Offset => member_get_one(&obj, self.member.offset, &self.member, vm),
            MemberAccess::TupleItem => {
                let index = self.member.offset as usize;
                let tuple = obj.downcast_ref::<PyTuple>().ok_or_else(|| {
                    vm.new_type_error("unexpected payload for struct sequence member")
                })?;
                tuple
                    .as_slice()
                    .get(index)
                    .cloned()
                    .ok_or_else(|| vm.new_index_error(format!("tuple index {index} out of range")))
            }
        }
    }

    fn set(
        &self,
        obj: PyObjectRef,
        value: PySetterValue<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        if self.member.readonly() {
            return Err(vm.new_attribute_error("readonly attribute"));
        }
        match self.access {
            MemberAccess::Offset => {
                member_set_one(&obj, self.member.offset, &self.member, value, vm)
            }
            MemberAccess::TupleItem => Err(vm.new_attribute_error("readonly attribute")),
        }
    }
}

impl PyPayload for PyMemberDescriptor {
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.member_descriptor_type
    }
}

fn calculate_qualname(descr: &PyDescriptorOwned, vm: &VirtualMachine) -> PyResult<Option<String>> {
    if let Some(qualname) = vm.get_attribute_opt(descr.typ.as_object(), "__qualname__")? {
        let str = qualname.downcast::<PyStr>().map_err(|_| {
            vm.new_type_error("<descriptor>.__objclass__.__qualname__ is not a unicode object")
        })?;
        Ok(Some(format!("{}.{}", str, descr.name)))
    } else {
        Ok(None)
    }
}

#[pyclass(with(GetDescriptor, Representable), flags(DISALLOW_INSTANTIATION))]
impl PyMemberDescriptor {
    #[pygetset]
    fn __doc__(&self) -> Option<String> {
        self.member.doc.to_owned()
    }

    #[pygetset]
    fn __qualname__(&self, vm: &VirtualMachine) -> PyResult<Option<String>> {
        let qualname = self.common.qualname.read();
        Ok(if qualname.is_none() {
            drop(qualname);
            let calculated = calculate_qualname(&self.common, vm)?;
            calculated.clone_into(&mut self.common.qualname.write());
            calculated
        } else {
            qualname.to_owned()
        })
    }

    #[pymethod]
    fn __reduce__(&self, vm: &VirtualMachine) -> PyResult {
        let builtins_getattr = vm.builtins.get_attr("getattr", vm)?;
        Ok(vm
            .ctx
            .new_tuple(vec![
                builtins_getattr,
                vm.ctx
                    .new_tuple(vec![
                        self.common.typ.clone().into(),
                        vm.ctx.new_str(self.common.name.as_str()).into(),
                    ])
                    .into(),
            ])
            .into())
    }

    #[pyslot]
    fn descr_set(
        zelf: &PyObject,
        obj: PyObjectRef,
        value: PySetterValue<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let zelf = Self::_as_pyref(zelf, vm)?;

        if !obj.class().fast_issubclass(&zelf.common.typ) {
            return Err(vm.new_type_error(format!(
                "descriptor '{}' for '{}' objects doesn't apply to a '{}' object",
                zelf.common.name,
                zelf.common.typ.name(),
                obj.class().name()
            )));
        }

        zelf.set(obj, value, vm)
    }
}

fn member_addr(obj: &PyObject, offset: isize) -> *mut u8 {
    (obj as *const PyObject as *const u8).wrapping_add(offset as usize) as *mut u8
}

fn warn_member(vm: &VirtualMachine, message: &str) -> PyResult<()> {
    crate::warn::warn(
        vm.ctx.new_str(message).into(),
        Some(vm.ctx.exceptions.runtime_warning.to_owned()),
        1,
        None,
        vm,
    )
}

fn load_i32(obj: &PyObject, offset: isize, readonly: bool) -> i32 {
    let addr = member_addr(obj, offset);
    if readonly {
        // SAFETY: a readonly int member addresses an `i32` that is not written
        // after publication.
        unsafe { addr.cast::<i32>().read() }
    } else {
        // SAFETY: a writable int member addresses an aligned `AtomicI32`.
        unsafe {
            (*addr.cast::<core::sync::atomic::AtomicI32>())
                .load(core::sync::atomic::Ordering::Relaxed)
        }
    }
}

fn load_u32(obj: &PyObject, offset: isize, readonly: bool) -> u32 {
    let addr = member_addr(obj, offset);
    if readonly {
        // SAFETY: a readonly uint member addresses a `u32` that is not written
        // after publication.
        unsafe { addr.cast::<u32>().read() }
    } else {
        // SAFETY: a writable uint member addresses an aligned `AtomicU32`.
        unsafe {
            (*addr.cast::<core::sync::atomic::AtomicU32>())
                .load(core::sync::atomic::Ordering::Relaxed)
        }
    }
}

fn member_as_c_long(value: &PyObject, vm: &VirtualMachine) -> PyResult<core::ffi::c_long> {
    let int_obj = value.try_index(vm)?;
    core::ffi::c_long::try_from(int_obj.as_bigint())
        .map_err(|_| vm.new_overflow_error("Python int too large to convert to C long"))
}

fn member_uint_value(
    value: &PyObject,
    vm: &VirtualMachine,
) -> PyResult<(u32, Option<&'static str>)> {
    let int_obj = value.try_index(vm)?;
    let big = int_obj.as_bigint();
    if big.sign() == malachite_bigint::Sign::Minus {
        let long_val = core::ffi::c_long::try_from(big)
            .map_err(|_| vm.new_overflow_error("Python int too large to convert to C long"))?;
        let stored = (long_val as core::ffi::c_ulong) as u32;
        return Ok((stored, Some("Writing negative value into unsigned field")));
    }
    let ulong_val = core::ffi::c_ulong::try_from(big)
        .map_err(|_| vm.new_overflow_error("Python int too large to convert to C unsigned long"))?;
    let stored = ulong_val as u32;
    let warning = (ulong_val > u32::MAX as core::ffi::c_ulong)
        .then_some("Truncation of value to unsigned int");
    Ok((stored, warning))
}

// PyMember_GetOne. `offset` is a byte offset from the object to the field.
fn member_get_one(
    obj: &PyObject,
    offset: isize,
    member: &PyMemberDef,
    vm: &VirtualMachine,
) -> PyResult {
    let value = match member.kind {
        MemberKind::Object => obj.get_slot(offset).unwrap_or_else(|| vm.ctx.none()),
        MemberKind::ObjectEx => obj.get_slot(offset).ok_or_else(|| {
            vm.new_attribute_error(format!(
                "'{}' object has no attribute '{}'",
                obj.class().fully_qualified_name(vm),
                member.name
            ))
        })?,
        MemberKind::Bool => {
            // SAFETY: a bool member addresses an `AtomicBool` or a `bool`
            // (one byte, naturally aligned). The macro rejects any other field
            // type. A plain `bool` is not written after the object is published.
            let raw = unsafe {
                (*member_addr(obj, offset).cast::<core::sync::atomic::AtomicBool>())
                    .load(core::sync::atomic::Ordering::Relaxed)
            };
            vm.ctx.new_bool(raw).into()
        }
        MemberKind::Int => vm
            .ctx
            .new_int(load_i32(obj, offset, member.readonly()))
            .into(),
        MemberKind::Uint => vm
            .ctx
            .new_int(load_u32(obj, offset, member.readonly()))
            .into(),
        MemberKind::Double => {
            let raw = if member.readonly() {
                // SAFETY: a readonly double member addresses an `f64` that is
                // not written after publication. A plain `f64` may be only
                // 4-byte aligned, so this is a plain read and not an atomic
                // access.
                unsafe { member_addr(obj, offset).cast::<f64>().read() }
            } else {
                // SAFETY: a writable double member, including a C-API member
                // without `Py_READONLY`, addresses an aligned atomic 64-bit
                // cell holding the `f64` bits. The macro accepts only
                // `DoubleCell` (`AtomicU64`).
                let bits = unsafe {
                    (*member_addr(obj, offset).cast::<core::sync::atomic::AtomicU64>())
                        .load(core::sync::atomic::Ordering::Relaxed)
                };
                f64::from_bits(bits)
            };
            vm.ctx.new_float(raw).into()
        }
    };
    Ok(value)
}

// PyMember_SetOne.
fn member_set_one(
    obj: &PyObject,
    offset: isize,
    member: &PyMemberDef,
    value: PySetterValue,
    vm: &VirtualMachine,
) -> PyResult<()> {
    if matches!(value, PySetterValue::Delete)
        && !matches!(member.kind, MemberKind::Object | MemberKind::ObjectEx)
    {
        return Err(vm.new_type_error("can't delete numeric/char attribute"));
    }
    match member.kind {
        MemberKind::Object => match value {
            PySetterValue::Assign(v) => obj.set_slot(offset, Some(v)),
            PySetterValue::Delete => obj.set_slot(offset, None),
        },
        MemberKind::ObjectEx => match value {
            PySetterValue::Assign(v) => obj.set_slot(offset, Some(v)),
            PySetterValue::Delete => {
                if obj.get_slot(offset).is_none() {
                    return Err(vm.new_attribute_error(member.name.clone()));
                }
                obj.set_slot(offset, None);
            }
        },
        MemberKind::Bool => {
            let PySetterValue::Assign(value) = value else {
                return Err(vm.new_type_error("can't delete numeric/char attribute"));
            };
            if !value.class().is(vm.ctx.types.bool_type) {
                return Err(vm.new_type_error("attribute value type must be bool"));
            }
            let stored = value.is(&vm.ctx.true_value);
            // SAFETY: a writable bool member addresses an `AtomicBool`.
            unsafe {
                (*member_addr(obj, offset).cast::<core::sync::atomic::AtomicBool>())
                    .store(stored, core::sync::atomic::Ordering::Relaxed);
            }
        }
        MemberKind::Int => {
            let PySetterValue::Assign(value) = value else {
                return Err(vm.new_type_error("can't delete numeric/char attribute"));
            };
            let long_val = member_as_c_long(&value, vm)?;
            let stored = long_val as i32;
            // SAFETY: a writable int member addresses an aligned `AtomicI32`.
            // Readonly members are rejected before this call.
            unsafe {
                (*member_addr(obj, offset).cast::<core::sync::atomic::AtomicI32>())
                    .store(stored, core::sync::atomic::Ordering::Relaxed);
            }
            let truncated = long_val > i32::MAX as core::ffi::c_long
                || long_val < i32::MIN as core::ffi::c_long;
            if truncated {
                warn_member(vm, "Truncation of value to int")?;
            }
        }
        MemberKind::Uint => {
            let PySetterValue::Assign(value) = value else {
                return Err(vm.new_type_error("can't delete numeric/char attribute"));
            };
            let (stored, warning) = member_uint_value(&value, vm)?;
            // SAFETY: a writable uint member addresses an aligned `AtomicU32`.
            // Readonly members are rejected before this call.
            unsafe {
                (*member_addr(obj, offset).cast::<core::sync::atomic::AtomicU32>())
                    .store(stored, core::sync::atomic::Ordering::Relaxed);
            }
            if let Some(warning) = warning {
                warn_member(vm, warning)?;
            }
        }
        MemberKind::Double => {
            let PySetterValue::Assign(value) = value else {
                return Err(vm.new_type_error("can't delete numeric/char attribute"));
            };
            let number = value.try_float(vm)?.to_f64();
            // SAFETY: a writable double member, including a C-API member
            // without `Py_READONLY`, addresses an aligned atomic 64-bit cell
            // holding the `f64` bits. Readonly members are rejected before
            // this call.
            unsafe {
                (*member_addr(obj, offset).cast::<core::sync::atomic::AtomicU64>())
                    .store(number.to_bits(), core::sync::atomic::Ordering::Relaxed);
            }
        }
    }
    Ok(())
}

impl Representable for PyMemberDescriptor {
    #[inline]
    fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
        Ok(format!(
            "<member '{}' of '{}' objects>",
            zelf.common.name,
            zelf.common.typ.name(),
        ))
    }
}

impl GetDescriptor for PyMemberDescriptor {
    fn descr_get(
        zelf: &PyObject,
        obj: Option<&PyObject>,
        _cls: Option<&PyObject>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let descr = Self::_as_pyref(zelf, vm)?;
        match obj {
            Some(x) => {
                if !x.class().fast_issubclass(&descr.common.typ) {
                    return Err(vm.new_type_error(format!(
                        "descriptor '{}' for '{}' objects doesn't apply to a '{}' object",
                        descr.common.name,
                        descr.common.typ.name(),
                        x.class().name()
                    )));
                }
                descr.get(x.to_owned(), vm)
            }
            None => Ok(zelf.to_owned()),
        }
    }
}

/// Vectorcall for method_descriptor: calls native method directly
fn vectorcall_method_descriptor(
    zelf_obj: &PyObject,
    args: Vec<PyObjectRef>,
    nargs: usize,
    kwnames: Option<&[PyObjectRef]>,
    vm: &VirtualMachine,
) -> PyResult {
    let zelf: &Py<PyMethodDescriptor> = zelf_obj.downcast_ref().unwrap();
    let func_args = FuncArgs::from_vectorcall_owned(args, nargs, kwnames);
    (zelf.method.func)(
        vm,
        func_args,
        Callee::named(zelf.method.name).with_instance_arg(true),
    )
}

/// Vectorcall for wrapper_descriptor: calls wrapped slot function
fn vectorcall_wrapper(
    zelf_obj: &PyObject,
    mut args: Vec<PyObjectRef>,
    nargs: usize,
    kwnames: Option<&[PyObjectRef]>,
    vm: &VirtualMachine,
) -> PyResult {
    let zelf: &Py<PyWrapper> = zelf_obj.downcast_ref().unwrap();
    // First positional arg is self
    if nargs == 0 {
        return Err(vm.new_type_error(format!(
            "descriptor '{}' of '{}' object needs an argument",
            zelf.name.as_str(),
            zelf.typ.name()
        )));
    }
    let obj = args.remove(0);
    if !obj.fast_isinstance(zelf.typ) {
        return Err(vm.new_type_error(format!(
            "descriptor '{}' requires a '{}' object but received a '{}'",
            zelf.name.as_str(),
            zelf.typ.name(),
            obj.class().name()
        )));
    }
    let rest = FuncArgs::from_vectorcall_owned(args, nargs - 1, kwnames);
    zelf.wrapped.call(obj, rest, vm)
}

pub(crate) fn init(ctx: &'static Context) {
    PyMemberDescriptor::extend_class(ctx, ctx.types.member_descriptor_type);
    PyMethodDescriptor::extend_class(ctx, ctx.types.method_descriptor_type);
    ctx.types
        .method_descriptor_type
        .slots
        .vectorcall
        .store(Some(vectorcall_method_descriptor));
    PyClassMethodDescriptor::extend_class(ctx, ctx.types.classmethod_descriptor_type);
    PyWrapper::extend_class(ctx, ctx.types.wrapper_descriptor_type);
    ctx.types
        .wrapper_descriptor_type
        .slots
        .vectorcall
        .store(Some(vectorcall_wrapper));
    PyMethodWrapper::extend_class(ctx, ctx.types.method_wrapper_type);
}

// PyWrapper - wrapper_descriptor

/// Each variant knows how to call the wrapped function with proper types
#[derive(Clone, Copy)]
pub enum SlotFunc {
    // Basic slots
    Init(InitFunc),
    Hash(HashFunc),
    Str(StringifyFunc),
    Repr(StringifyFunc),
    Iter(IterFunc),
    IterNext(IterNextFunc),
    Call(GenericMethod),
    Del(DelFunc),

    // Attribute access slots
    GetAttro(GetattroFunc),
    SetAttro(SetattroFunc), // __setattr__
    DelAttro(SetattroFunc), // __delattr__ (same func type, different PySetterValue)

    // Rich comparison slots (with comparison op)
    RichCompare(RichCompareFunc, PyComparisonOp),

    // Descriptor slots
    DescrGet(DescrGetFunc),
    DescrSet(DescrSetFunc), // __set__
    DescrDel(DescrSetFunc), // __delete__ (same func type, different PySetterValue)

    // Sequence sub-slots (sq_*)
    SeqLength(SeqLenFunc),
    SeqConcat(SeqConcatFunc),
    SeqRepeat(SeqRepeatFunc),
    SeqItem(SeqItemFunc),
    SeqSetItem(SeqAssItemFunc), // __setitem__ (same func type, value = Some)
    SeqDelItem(SeqAssItemFunc), // __delitem__ (same func type, value = None)
    SeqContains(SeqContainsFunc),

    // Mapping sub-slots (mp_*)
    MapLength(MapLenFunc),
    MapSubscript(MapSubscriptFunc),
    MapSetSubscript(MapAssSubscriptFunc), // __setitem__ (same func type, value = Some)
    MapDelSubscript(MapAssSubscriptFunc), // __delitem__ (same func type, value = None)

    // Number sub-slots (nb_*) - grouped by signature
    NumBoolean(PyNumberUnaryFunc<bool>),  // __bool__
    NumUnary(PyNumberUnaryFunc),          // __int__, __float__, __index__
    NumBinary(PyNumberBinaryFunc),        // __add__, __sub__, __mul__, etc.
    NumBinaryRight(PyNumberBinaryFunc),   // __radd__, __rsub__, etc. (swapped args)
    NumTernary(PyNumberTernaryFunc),      // __pow__
    NumTernaryRight(PyNumberTernaryFunc), // __rpow__ (swapped first two args)

    // Buffer protocol
    GetBuffer(crate::types::AsBufferFunc), // __buffer__
    ReleaseBuffer,                         // __release_buffer__
}

impl core::fmt::Debug for SlotFunc {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Init(_) => write!(f, "SlotFunc::Init(...)"),
            Self::Hash(_) => write!(f, "SlotFunc::Hash(...)"),
            Self::Str(_) => write!(f, "SlotFunc::Str(...)"),
            Self::Repr(_) => write!(f, "SlotFunc::Repr(...)"),
            Self::Iter(_) => write!(f, "SlotFunc::Iter(...)"),
            Self::IterNext(_) => write!(f, "SlotFunc::IterNext(...)"),
            Self::Call(_) => write!(f, "SlotFunc::Call(...)"),
            Self::Del(_) => write!(f, "SlotFunc::Del(...)"),
            Self::GetAttro(_) => write!(f, "SlotFunc::GetAttro(...)"),
            Self::SetAttro(_) => write!(f, "SlotFunc::SetAttro(...)"),
            Self::DelAttro(_) => write!(f, "SlotFunc::DelAttro(...)"),
            Self::RichCompare(_, op) => write!(f, "SlotFunc::RichCompare(..., {op:?})"),
            Self::DescrGet(_) => write!(f, "SlotFunc::DescrGet(...)"),
            Self::DescrSet(_) => write!(f, "SlotFunc::DescrSet(...)"),
            Self::DescrDel(_) => write!(f, "SlotFunc::DescrDel(...)"),
            // Sequence sub-slots
            Self::SeqLength(_) => write!(f, "SlotFunc::SeqLength(...)"),
            Self::SeqConcat(_) => write!(f, "SlotFunc::SeqConcat(...)"),
            Self::SeqRepeat(_) => write!(f, "SlotFunc::SeqRepeat(...)"),
            Self::SeqItem(_) => write!(f, "SlotFunc::SeqItem(...)"),
            Self::SeqSetItem(_) => write!(f, "SlotFunc::SeqSetItem(...)"),
            Self::SeqDelItem(_) => write!(f, "SlotFunc::SeqDelItem(...)"),
            Self::SeqContains(_) => write!(f, "SlotFunc::SeqContains(...)"),
            // Mapping sub-slots
            Self::MapLength(_) => write!(f, "SlotFunc::MapLength(...)"),
            Self::MapSubscript(_) => write!(f, "SlotFunc::MapSubscript(...)"),
            Self::MapSetSubscript(_) => write!(f, "SlotFunc::MapSetSubscript(...)"),
            Self::MapDelSubscript(_) => write!(f, "SlotFunc::MapDelSubscript(...)"),
            // Number sub-slots
            Self::NumBoolean(_) => write!(f, "SlotFunc::NumBoolean(...)"),
            Self::NumUnary(_) => write!(f, "SlotFunc::NumUnary(...)"),
            Self::NumBinary(_) => write!(f, "SlotFunc::NumBinary(...)"),
            Self::NumBinaryRight(_) => write!(f, "SlotFunc::NumBinaryRight(...)"),
            Self::NumTernary(_) => write!(f, "SlotFunc::NumTernary(...)"),
            Self::NumTernaryRight(_) => write!(f, "SlotFunc::NumTernaryRight(...)"),
            Self::GetBuffer(_) => write!(f, "SlotFunc::GetBuffer(...)"),
            Self::ReleaseBuffer => write!(f, "SlotFunc::ReleaseBuffer"),
        }
    }
}

impl SlotFunc {
    /// Call the wrapped slot function with proper type handling
    pub fn call(&self, obj: PyObjectRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        match self {
            Self::Init(func) => {
                func(&obj, args, vm)?;
                Ok(vm.ctx.none())
            }
            Self::Hash(func) => {
                if !args.args.is_empty() || !args.kwargs.is_empty() {
                    return Err(vm.new_type_error("__hash__() takes no arguments (1 given)"));
                }
                let hash = func(&obj, vm)?;
                Ok(vm.ctx.new_int(hash).into())
            }
            Self::Repr(func) | Self::Str(func) => {
                if !args.args.is_empty() || !args.kwargs.is_empty() {
                    let name = match self {
                        Self::Repr(_) => "__repr__",
                        Self::Str(_) => "__str__",
                        _ => unreachable!(),
                    };
                    return Err(vm.new_type_error(format!("{name}() takes no arguments (1 given)")));
                }
                let s = func(&obj, vm)?;
                Ok(s.into())
            }
            Self::Iter(func) => {
                if !args.args.is_empty() || !args.kwargs.is_empty() {
                    return Err(vm.new_type_error("__iter__() takes no arguments (1 given)"));
                }
                func(obj, vm)
            }
            Self::IterNext(func) => {
                if !args.args.is_empty() || !args.kwargs.is_empty() {
                    return Err(vm.new_type_error("__next__() takes no arguments (1 given)"));
                }
                func(&obj, vm).to_pyresult(vm)
            }
            Self::Call(func) => func(&obj, args, vm),
            Self::Del(func) => {
                if !args.args.is_empty() || !args.kwargs.is_empty() {
                    return Err(vm.new_type_error("__del__() takes no arguments (1 given)"));
                }
                func(&obj, vm)?;
                Ok(vm.ctx.none())
            }
            Self::GetAttro(func) => {
                let (name,): (PyRef<PyStr>,) = args.bind(vm)?;
                func(&obj, &name, vm)
            }
            Self::SetAttro(func) => {
                let (name, value): (PyRef<PyStr>, PyObjectRef) = args.bind(vm)?;
                crate::types::hackcheck_setattro(&obj, *func, "__setattr__", vm)?;
                func(&obj, &name, PySetterValue::Assign(value), vm)?;
                Ok(vm.ctx.none())
            }
            Self::DelAttro(func) => {
                let (name,): (PyRef<PyStr>,) = args.bind(vm)?;
                crate::types::hackcheck_setattro(&obj, *func, "__delattr__", vm)?;
                func(&obj, &name, PySetterValue::Delete, vm)?;
                Ok(vm.ctx.none())
            }
            Self::RichCompare(func, op) => {
                let (other,): (PyObjectRef,) = args.bind(vm)?;
                func(&obj, &other, *op, vm).map(|r| match r {
                    crate::function::Either::A(obj) => obj,
                    crate::function::Either::B(cmp_val) => cmp_val.to_pyobject(vm),
                })
            }
            Self::DescrGet(func) => {
                let (instance, owner): (PyObjectRef, crate::function::OptionalArg<PyObjectRef>) =
                    args.bind(vm)?;
                let owner = owner.into_option();
                let instance_opt = if vm.is_none(&instance) {
                    None
                } else {
                    Some(instance)
                };
                func(&obj, instance_opt.as_deref(), owner.as_deref(), vm)
            }
            Self::DescrSet(func) => {
                let (instance, value): (PyObjectRef, PyObjectRef) = args.bind(vm)?;
                func(&obj, instance, PySetterValue::Assign(value), vm)?;
                Ok(vm.ctx.none())
            }
            Self::DescrDel(func) => {
                let (instance,): (PyObjectRef,) = args.bind(vm)?;
                func(&obj, instance, PySetterValue::Delete, vm)?;
                Ok(vm.ctx.none())
            }
            // Sequence sub-slots
            Self::SeqLength(func) => {
                args.bind::<()>(vm)?;
                let len = func(obj.sequence_unchecked(), vm)?;
                Ok(vm.ctx.new_int(len).into())
            }
            Self::SeqConcat(func) => {
                let (other,): (PyObjectRef,) = args.bind(vm)?;
                func(obj.sequence_unchecked(), &other, vm)
            }
            Self::SeqRepeat(func) => {
                let (n,): (ArgSize,) = args.bind(vm)?;
                func(obj.sequence_unchecked(), n.into(), vm)
            }
            Self::SeqItem(func) => {
                let (index,): (isize,) = args.bind(vm)?;
                func(obj.sequence_unchecked(), index, vm)
            }
            Self::SeqSetItem(func) => {
                let (index, value): (isize, PyObjectRef) = args.bind(vm)?;
                func(obj.sequence_unchecked(), index, Some(value), vm)?;
                Ok(vm.ctx.none())
            }
            Self::SeqDelItem(func) => {
                let (index,): (isize,) = args.bind(vm)?;
                func(obj.sequence_unchecked(), index, None, vm)?;
                Ok(vm.ctx.none())
            }
            Self::SeqContains(func) => {
                let (item,): (PyObjectRef,) = args.bind(vm)?;
                let result = func(obj.sequence_unchecked(), &item, vm)?;
                Ok(vm.ctx.new_bool(result).into())
            }
            // Mapping sub-slots
            Self::MapLength(func) => {
                args.bind::<()>(vm)?;
                let len = func(obj.mapping_unchecked(), vm)?;
                Ok(vm.ctx.new_int(len).into())
            }
            Self::MapSubscript(func) => {
                let (key,): (PyObjectRef,) = args.bind(vm)?;
                func(obj.mapping_unchecked(), &key, vm)
            }
            Self::MapSetSubscript(func) => {
                let (key, value): (PyObjectRef, PyObjectRef) = args.bind(vm)?;
                func(obj.mapping_unchecked(), &key, Some(value), vm)?;
                Ok(vm.ctx.none())
            }
            Self::MapDelSubscript(func) => {
                let (key,): (PyObjectRef,) = args.bind(vm)?;
                func(obj.mapping_unchecked(), &key, None, vm)?;
                Ok(vm.ctx.none())
            }
            // Number sub-slots
            Self::NumBoolean(func) => {
                args.bind::<()>(vm)?;
                let result = func(obj.number(), vm)?;
                Ok(vm.ctx.new_bool(result).into())
            }
            Self::NumUnary(func) => {
                args.bind::<()>(vm)?;
                func(obj.number(), vm)
            }
            Self::NumBinary(func) => {
                let (other,): (PyObjectRef,) = args.bind(vm)?;
                func(&obj, &other, vm)
            }
            Self::NumBinaryRight(func) => {
                let (other,): (PyObjectRef,) = args.bind(vm)?;
                func(&other, &obj, vm) // Swapped: other op obj
            }
            Self::NumTernary(func) => {
                let (y, z) = pow_args(args, vm)?;
                func(&obj, &y, &z, vm)
            }
            Self::NumTernaryRight(func) => {
                let (y, z) = pow_args(args, vm)?;
                func(&y, &obj, &z, vm)
            }
            // Buffer protocol
            Self::GetBuffer(func) => {
                let (flags_obj,): (PyObjectRef,) = args.bind(vm)?;
                let buffer = func(&obj, parse_buffer_flags(&flags_obj, vm)?, vm)?;
                crate::builtins::PyMemoryView::from_buffer(buffer, vm)
                    .map(|mv| mv.into_pyobject(vm))
            }
            Self::ReleaseBuffer => {
                let (mv_obj,): (PyObjectRef,) = args.bind(vm)?;
                let mv = mv_obj
                    .downcast::<crate::builtins::PyMemoryView>()
                    .map_err(|_| vm.new_type_error("expected a memoryview object"))?;
                crate::builtins::memory::release_buffer_from_python(&obj, &mv, vm)?;
                Ok(vm.ctx.none())
            }
        }
    }
}

/// wrap_ternaryfunc / check_pow_args
fn pow_args(args: FuncArgs, vm: &VirtualMachine) -> PyResult<(PyObjectRef, PyObjectRef)> {
    if let Some(err) = args.check_kwargs_empty(vm) {
        return Err(err);
    }
    let size = args.args.len();
    if !(1..=2).contains(&size) {
        return Err(vm.new_type_error(format!("expected 1 or 2 arguments, got {size}")));
    }
    let y = args.args[0].clone();
    let z = if size == 2 {
        args.args[1].clone()
    } else {
        vm.ctx.none()
    };
    Ok((y, z))
}

/// Parse the `flags` argument of `__buffer__`. wrap_buffer
fn parse_buffer_flags(
    arg: &PyObject,
    vm: &VirtualMachine,
) -> PyResult<crate::protocol::BufferFlags> {
    use num_traits::ToPrimitive;
    let idx = arg.try_index(vm)?;
    let flags = idx
        .as_bigint()
        .to_isize()
        .ok_or_else(|| vm.new_overflow_error("cannot fit 'int' into an index-sized integer"))?;
    let flags =
        i32::try_from(flags).map_err(|_| vm.new_overflow_error("buffer flags out of range"))?;
    Ok(crate::protocol::BufferFlags::from_bits_retain(flags as u32))
}

/// wrapper_descriptor: wraps a slot function as a Python method
// = PyWrapperDescrObject
#[pyclass(name = "wrapper_descriptor", module = false)]
#[derive(Debug)]
pub(crate) struct PyWrapper {
    #[pymember(name = "__objclass__")]
    pub typ: &'static Py<PyType>,
    #[pymember(name = "__name__")]
    pub name: &'static PyStrInterned,
    pub wrapped: SlotFunc,
    /// Slot text, including the text signature.
    pub doc: Option<&'static str>,
    /// Plain docstring for this slot when the table has one.
    pub plain_doc: Option<&'static str>,
}

impl PyPayload for PyWrapper {
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.wrapper_descriptor_type
    }
}

impl GetDescriptor for PyWrapper {
    fn descr_get(
        zelf: &PyObject,
        obj: Option<&PyObject>,
        _cls: Option<&PyObject>,
        vm: &VirtualMachine,
    ) -> PyResult {
        match obj {
            None => Ok(zelf.to_owned()),
            Some(obj) => {
                let zelf = zelf.to_owned().downcast::<Self>().unwrap();
                Ok(PyMethodWrapper {
                    wrapper: zelf,
                    obj: obj.to_owned(),
                }
                .into_pyobject(vm))
            }
        }
    }
}

impl Callable for PyWrapper {
    type Args = FuncArgs;

    fn call(zelf: &Py<Self>, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        // list.__init__(l, [1,2,3]) form - first arg is self
        let (obj, rest): (PyObjectRef, FuncArgs) = args.bind(vm)?;

        if !obj.fast_isinstance(zelf.typ) {
            return Err(vm.new_type_error(format!(
                "descriptor '{}' requires a '{}' object but received a '{}'",
                zelf.name.as_str(),
                zelf.typ.name(),
                obj.class().name()
            )));
        }

        zelf.wrapped.call(obj, rest, vm)
    }
}

#[pyclass(
    with(GetDescriptor, Callable, Representable),
    flags(DISALLOW_INSTANTIATION)
)]
impl PyWrapper {
    #[pygetset]
    fn __qualname__(&self) -> String {
        format!("{}.{}", self.typ.name(), self.name)
    }

    #[pygetset]
    fn __doc__(&self) -> Option<&'static str> {
        if let Some(doc) = self.plain_doc {
            return Some(doc);
        }
        let doc = self.doc?;
        type_::get_doc_from_internal_doc(self.name.as_str(), doc)
    }

    #[pygetset]
    fn __text_signature__(&self) -> Option<String> {
        self.doc.and_then(|doc| {
            type_::get_text_signature_from_internal_doc(self.name.as_str(), doc)
                .map(|signature| signature.to_string())
        })
    }
}

impl Representable for PyWrapper {
    #[inline]
    fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
        Ok(format!(
            "<slot wrapper '{}' of '{}' objects>",
            zelf.name.as_str(),
            zelf.typ.name()
        ))
    }
}

// PyMethodWrapper - method-wrapper

/// method-wrapper: a slot wrapper bound to an instance
/// Returned when accessing l.__init__ on an instance
#[pyclass(name = "method-wrapper", module = false, traverse)]
#[derive(Debug)]
pub(crate) struct PyMethodWrapper {
    pub wrapper: PyRef<PyWrapper>,
    #[pymember(name = "__self__")]
    #[pytraverse(skip)]
    pub obj: PyObjectRef,
}

impl PyPayload for PyMethodWrapper {
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.method_wrapper_type
    }
}

impl Callable for PyMethodWrapper {
    type Args = FuncArgs;

    fn call(zelf: &Py<Self>, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        // bpo-37619: Check type compatibility before calling wrapped slot
        if !zelf.obj.fast_isinstance(zelf.wrapper.typ) {
            return Err(vm.new_type_error(format!(
                "descriptor '{}' requires a '{}' object but received a '{}'",
                zelf.wrapper.name.as_str(),
                zelf.wrapper.typ.name(),
                zelf.obj.class().name()
            )));
        }
        zelf.wrapper.wrapped.call(zelf.obj.clone(), args, vm)
    }
}

#[pyclass(
    with(Callable, Representable, Hashable, Comparable),
    flags(DISALLOW_INSTANTIATION)
)]
impl PyMethodWrapper {
    #[pygetset]
    fn __name__(&self) -> &'static PyStrInterned {
        self.wrapper.name
    }

    #[pygetset]
    fn __objclass__(&self) -> PyTypeRef {
        self.wrapper.typ.to_owned()
    }

    #[pygetset]
    fn __qualname__(&self) -> String {
        format!("{}.{}", self.wrapper.typ.name(), self.wrapper.name)
    }

    #[pygetset]
    fn __doc__(&self) -> Option<&'static str> {
        if let Some(doc) = self.wrapper.plain_doc {
            return Some(doc);
        }
        let doc = self.wrapper.doc?;
        type_::get_doc_from_internal_doc(self.wrapper.name.as_str(), doc)
    }

    #[pygetset]
    fn __text_signature__(&self) -> Option<String> {
        self.wrapper.doc.and_then(|doc| {
            type_::get_text_signature_from_internal_doc(self.wrapper.name.as_str(), doc)
                .map(|signature| signature.to_string())
        })
    }

    #[pymethod]
    fn __reduce__(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        let builtins_getattr = vm.builtins.get_attr("getattr", vm)?;
        Ok(vm
            .ctx
            .new_tuple(vec![
                builtins_getattr,
                vm.ctx
                    .new_tuple(vec![
                        zelf.obj.clone(),
                        vm.ctx.new_str(zelf.wrapper.name.as_str()).into(),
                    ])
                    .into(),
            ])
            .into())
    }
}

impl Representable for PyMethodWrapper {
    #[inline]
    fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
        Ok(format!(
            "<method-wrapper '{}' of {} object at {:#x}>",
            zelf.wrapper.name.as_str(),
            zelf.obj.class().name(),
            zelf.obj.get_id()
        ))
    }
}

impl Hashable for PyMethodWrapper {
    fn hash(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<PyHash> {
        // wrapperobject_hash: pointer hash of descr xor self
        let mut hash =
            (zelf.wrapper.as_object().get_id() as PyHash) ^ (zelf.obj.get_id() as PyHash);
        if hash == -1 {
            hash = -2;
        }
        Ok(hash)
    }
}

impl Comparable for PyMethodWrapper {
    fn cmp(
        zelf: &Py<Self>,
        other: &PyObject,
        op: PyComparisonOp,
        _vm: &VirtualMachine,
    ) -> PyResult<crate::function::PyComparisonValue> {
        op.eq_only(|| {
            let other = class_or_notimplemented!(Self, other);
            let eq = zelf.wrapper.is(&other.wrapper) && zelf.obj.is(&other.obj);
            Ok(eq.into())
        })
    }
}
