use super::{PyStr, PyStrInterned, PyType};
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    builtins::{PyTypeRef, builtin_func::PyNativeMethod, type_},
    class::PyClassImpl,
    common::hash::PyHash,
    convert::{ToPyObject, ToPyResult},
    function::{FuncArgs, PyMethodDef, PyMethodFlags, PySetterValue},
    types::{
        Callable, Comparable, DelFunc, DescrGetFunc, DescrSetFunc, GenericMethod, GetDescriptor,
        GetattroFunc, HashFunc, Hashable, InitFunc, IterFunc, IterNextFunc, PyComparisonOp,
        Representable, RichCompareFunc, SetattroFunc, StringifyFunc,
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
    pub common: PyDescriptor,
    pub method: &'static PyMethodDef,
    // vectorcall: vector_call_func,
    pub objclass: &'static Py<PyType>, // TODO: move to tp_members
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
            objclass: typ,
        }
    }
}

impl PyPayload for PyMethodDescriptor {
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.method_descriptor_type
    }
}

impl std::fmt::Debug for PyMethodDescriptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "method descriptor for '{}'", self.common.name)
    }
}

impl GetDescriptor for PyMethodDescriptor {
    fn descr_get(
        zelf: PyObjectRef,
        obj: Option<PyObjectRef>,
        cls: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let descr = Self::_as_pyref(&zelf, vm).unwrap();
        let bound = match obj {
            Some(obj) => {
                if descr.method.flags.contains(PyMethodFlags::METHOD) {
                    if cls.is_some_and(|c| c.fast_isinstance(vm.ctx.types.type_type)) {
                        obj
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
                    unimplemented!()
                }
            }
            None if descr.method.flags.contains(PyMethodFlags::CLASS) => cls.unwrap(),
            None => return Ok(zelf),
        };
        // Ok(descr.method.build_bound_method(&vm.ctx, bound, class).into())
        Ok(descr.bind(bound, &vm.ctx).into())
    }
}

impl Callable for PyMethodDescriptor {
    type Args = FuncArgs;
    #[inline]
    fn call(zelf: &Py<Self>, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        (zelf.method.func)(vm, args)
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
    const fn __name__(&self) -> &'static PyStrInterned {
        self.common.name
    }

    #[pygetset]
    fn __qualname__(&self) -> String {
        format!("{}.{}", self.common.typ.name(), &self.common.name)
    }

    #[pygetset]
    const fn __doc__(&self) -> Option<&'static str> {
        self.method.doc
    }

    #[pygetset]
    fn __text_signature__(&self) -> Option<String> {
        self.method.doc.and_then(|doc| {
            type_::get_text_signature_from_internal_doc(self.method.name, doc)
                .map(|signature| signature.to_string())
        })
    }

    #[pygetset]
    fn __objclass__(&self) -> PyTypeRef {
        self.objclass.to_owned()
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
            &zelf.method.name,
            zelf.common.typ.name()
        ))
    }
}

#[derive(Debug)]
pub enum MemberKind {
    Bool = 14,
    ObjectEx = 16,
}

pub type MemberSetterFunc = Option<fn(&VirtualMachine, PyObjectRef, PySetterValue) -> PyResult<()>>;

pub enum MemberGetter {
    Getter(fn(&VirtualMachine, PyObjectRef) -> PyResult),
    Offset(usize),
}

pub enum MemberSetter {
    Setter(MemberSetterFunc),
    Offset(usize),
}

pub struct PyMemberDef {
    pub name: String,
    pub kind: MemberKind,
    pub getter: MemberGetter,
    pub setter: MemberSetter,
    pub doc: Option<String>,
}

impl PyMemberDef {
    fn get(&self, obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        match self.getter {
            MemberGetter::Getter(getter) => (getter)(vm, obj),
            MemberGetter::Offset(offset) => get_slot_from_object(obj, offset, self, vm),
        }
    }

    fn set(
        &self,
        obj: PyObjectRef,
        value: PySetterValue<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        match self.setter {
            MemberSetter::Setter(setter) => match setter {
                Some(setter) => (setter)(vm, obj, value),
                None => Err(vm.new_attribute_error("readonly attribute")),
            },
            MemberSetter::Offset(offset) => set_slot_at_object(obj, offset, self, value, vm),
        }
    }
}

impl std::fmt::Debug for PyMemberDef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PyMemberDef")
            .field("name", &self.name)
            .field("kind", &self.kind)
            .field("doc", &self.doc)
            .finish()
    }
}

// = PyMemberDescrObject
#[pyclass(name = "member_descriptor", module = false)]
#[derive(Debug)]
pub struct PyMemberDescriptor {
    pub common: PyDescriptorOwned,
    pub member: PyMemberDef,
}

impl PyPayload for PyMemberDescriptor {
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.member_descriptor_type
    }
}

fn calculate_qualname(descr: &PyDescriptorOwned, vm: &VirtualMachine) -> PyResult<Option<String>> {
    if let Some(qualname) = vm.get_attribute_opt(descr.typ.clone().into(), "__qualname__")? {
        let str = qualname.downcast::<PyStr>().map_err(|_| {
            vm.new_type_error("<descriptor>.__objclass__.__qualname__ is not a unicode object")
        })?;
        Ok(Some(format!("{}.{}", str, descr.name)))
    } else {
        Ok(None)
    }
}

#[pyclass(
    with(GetDescriptor, Representable),
    flags(BASETYPE, DISALLOW_INSTANTIATION)
)]
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

        zelf.member.set(obj, value, vm)
    }
}

// PyMember_GetOne
fn get_slot_from_object(
    obj: PyObjectRef,
    offset: usize,
    member: &PyMemberDef,
    vm: &VirtualMachine,
) -> PyResult {
    let slot = match member.kind {
        MemberKind::Bool => obj
            .get_slot(offset)
            .unwrap_or_else(|| vm.ctx.new_bool(false).into()),
        MemberKind::ObjectEx => obj.get_slot(offset).ok_or_else(|| {
            vm.new_no_attribute_error(obj.clone(), vm.ctx.new_str(member.name.clone()))
        })?,
    };
    Ok(slot)
}

// PyMember_SetOne
fn set_slot_at_object(
    obj: PyObjectRef,
    offset: usize,
    member: &PyMemberDef,
    value: PySetterValue,
    vm: &VirtualMachine,
) -> PyResult<()> {
    match member.kind {
        MemberKind::Bool => {
            match value {
                PySetterValue::Assign(v) => {
                    if !v.class().is(vm.ctx.types.bool_type) {
                        return Err(vm.new_type_error("attribute value type must be bool"));
                    }

                    obj.set_slot(offset, Some(v))
                }
                PySetterValue::Delete => obj.set_slot(offset, None),
            };
        }
        MemberKind::ObjectEx => {
            let value = match value {
                PySetterValue::Assign(v) => Some(v),
                PySetterValue::Delete => None,
            };
            obj.set_slot(offset, value);
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
        zelf: PyObjectRef,
        obj: Option<PyObjectRef>,
        cls: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let descr = Self::_as_pyref(&zelf, vm)?;
        match obj {
            Some(x) => descr.member.get(x, vm),
            None => {
                // When accessed from class (not instance), for __doc__ member descriptor,
                // return the class's docstring if available
                // When accessed from class (not instance), check if the class has
                // an attribute with the same name as this member descriptor
                if let Some(cls) = cls
                    && let Ok(cls_type) = cls.downcast::<PyType>()
                    && let Some(interned) = vm.ctx.interned_str(descr.member.name.as_str())
                    && let Some(attr) = cls_type.attributes.read().get(&interned)
                {
                    return Ok(attr.clone());
                }
                Ok(zelf)
            }
        }
    }
}

pub fn init(ctx: &Context) {
    PyMemberDescriptor::extend_class(ctx, ctx.types.member_descriptor_type);
    PyMethodDescriptor::extend_class(ctx, ctx.types.method_descriptor_type);
    PyWrapper::extend_class(ctx, ctx.types.wrapper_descriptor_type);
    PyMethodWrapper::extend_class(ctx, ctx.types.method_wrapper_type);
}

// PyWrapper - wrapper_descriptor

/// Type-erased slot function - mirrors CPython's void* d_wrapped
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
}

impl std::fmt::Debug for SlotFunc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SlotFunc::Init(_) => write!(f, "SlotFunc::Init(...)"),
            SlotFunc::Hash(_) => write!(f, "SlotFunc::Hash(...)"),
            SlotFunc::Str(_) => write!(f, "SlotFunc::Str(...)"),
            SlotFunc::Repr(_) => write!(f, "SlotFunc::Repr(...)"),
            SlotFunc::Iter(_) => write!(f, "SlotFunc::Iter(...)"),
            SlotFunc::IterNext(_) => write!(f, "SlotFunc::IterNext(...)"),
            SlotFunc::Call(_) => write!(f, "SlotFunc::Call(...)"),
            SlotFunc::Del(_) => write!(f, "SlotFunc::Del(...)"),
            SlotFunc::GetAttro(_) => write!(f, "SlotFunc::GetAttro(...)"),
            SlotFunc::SetAttro(_) => write!(f, "SlotFunc::SetAttro(...)"),
            SlotFunc::DelAttro(_) => write!(f, "SlotFunc::DelAttro(...)"),
            SlotFunc::RichCompare(_, op) => write!(f, "SlotFunc::RichCompare(..., {:?})", op),
            SlotFunc::DescrGet(_) => write!(f, "SlotFunc::DescrGet(...)"),
            SlotFunc::DescrSet(_) => write!(f, "SlotFunc::DescrSet(...)"),
            SlotFunc::DescrDel(_) => write!(f, "SlotFunc::DescrDel(...)"),
        }
    }
}

impl SlotFunc {
    /// Call the wrapped slot function with proper type handling
    pub fn call(&self, obj: PyObjectRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        match self {
            SlotFunc::Init(func) => {
                func(obj, args, vm)?;
                Ok(vm.ctx.none())
            }
            SlotFunc::Hash(func) => {
                if !args.args.is_empty() || !args.kwargs.is_empty() {
                    return Err(
                        vm.new_type_error("__hash__() takes no arguments (1 given)".to_owned())
                    );
                }
                let hash = func(&obj, vm)?;
                Ok(vm.ctx.new_int(hash).into())
            }
            SlotFunc::Repr(func) | SlotFunc::Str(func) => {
                if !args.args.is_empty() || !args.kwargs.is_empty() {
                    let name = match self {
                        SlotFunc::Repr(_) => "__repr__",
                        SlotFunc::Str(_) => "__str__",
                        _ => unreachable!(),
                    };
                    return Err(vm.new_type_error(format!("{name}() takes no arguments (1 given)")));
                }
                let s = func(&obj, vm)?;
                Ok(s.into())
            }
            SlotFunc::Iter(func) => {
                if !args.args.is_empty() || !args.kwargs.is_empty() {
                    return Err(
                        vm.new_type_error("__iter__() takes no arguments (1 given)".to_owned())
                    );
                }
                func(obj, vm)
            }
            SlotFunc::IterNext(func) => {
                if !args.args.is_empty() || !args.kwargs.is_empty() {
                    return Err(
                        vm.new_type_error("__next__() takes no arguments (1 given)".to_owned())
                    );
                }
                func(&obj, vm).to_pyresult(vm)
            }
            SlotFunc::Call(func) => func(&obj, args, vm),
            SlotFunc::Del(func) => {
                if !args.args.is_empty() || !args.kwargs.is_empty() {
                    return Err(
                        vm.new_type_error("__del__() takes no arguments (1 given)".to_owned())
                    );
                }
                func(&obj, vm)?;
                Ok(vm.ctx.none())
            }
            SlotFunc::GetAttro(func) => {
                let (name,): (PyRef<PyStr>,) = args.bind(vm)?;
                func(&obj, &name, vm)
            }
            SlotFunc::SetAttro(func) => {
                let (name, value): (PyRef<PyStr>, PyObjectRef) = args.bind(vm)?;
                func(&obj, &name, PySetterValue::Assign(value), vm)?;
                Ok(vm.ctx.none())
            }
            SlotFunc::DelAttro(func) => {
                let (name,): (PyRef<PyStr>,) = args.bind(vm)?;
                func(&obj, &name, PySetterValue::Delete, vm)?;
                Ok(vm.ctx.none())
            }
            SlotFunc::RichCompare(func, op) => {
                let (other,): (PyObjectRef,) = args.bind(vm)?;
                func(&obj, &other, *op, vm).map(|r| match r {
                    crate::function::Either::A(obj) => obj,
                    crate::function::Either::B(cmp_val) => cmp_val.to_pyobject(vm),
                })
            }
            SlotFunc::DescrGet(func) => {
                let (instance, owner): (PyObjectRef, crate::function::OptionalArg<PyObjectRef>) =
                    args.bind(vm)?;
                let owner = owner.into_option();
                let instance_opt = if vm.is_none(&instance) {
                    None
                } else {
                    Some(instance)
                };
                func(obj, instance_opt, owner, vm)
            }
            SlotFunc::DescrSet(func) => {
                let (instance, value): (PyObjectRef, PyObjectRef) = args.bind(vm)?;
                func(&obj, instance, PySetterValue::Assign(value), vm)?;
                Ok(vm.ctx.none())
            }
            SlotFunc::DescrDel(func) => {
                let (instance,): (PyObjectRef,) = args.bind(vm)?;
                func(&obj, instance, PySetterValue::Delete, vm)?;
                Ok(vm.ctx.none())
            }
        }
    }
}

/// wrapper_descriptor: wraps a slot function as a Python method
// = PyWrapperDescrObject
#[pyclass(name = "wrapper_descriptor", module = false)]
#[derive(Debug)]
pub struct PyWrapper {
    pub typ: &'static Py<PyType>,
    pub name: &'static PyStrInterned,
    pub wrapped: SlotFunc,
    pub doc: Option<&'static str>,
}

impl PyPayload for PyWrapper {
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.wrapper_descriptor_type
    }
}

impl GetDescriptor for PyWrapper {
    fn descr_get(
        zelf: PyObjectRef,
        obj: Option<PyObjectRef>,
        _cls: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        match obj {
            None => Ok(zelf),
            Some(obj) => {
                let zelf = zelf.downcast::<Self>().unwrap();
                Ok(PyMethodWrapper { wrapper: zelf, obj }.into_pyobject(vm))
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
    fn __name__(&self) -> &'static PyStrInterned {
        self.name
    }

    #[pygetset]
    fn __qualname__(&self) -> String {
        format!("{}.{}", self.typ.name(), self.name)
    }

    #[pygetset]
    fn __objclass__(&self) -> PyTypeRef {
        self.typ.to_owned()
    }

    #[pygetset]
    fn __doc__(&self) -> Option<&'static str> {
        self.doc
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
pub struct PyMethodWrapper {
    pub wrapper: PyRef<PyWrapper>,
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
        zelf.wrapper.wrapped.call(zelf.obj.clone(), args, vm)
    }
}

#[pyclass(
    with(Callable, Representable, Hashable, Comparable),
    flags(DISALLOW_INSTANTIATION)
)]
impl PyMethodWrapper {
    #[pygetset]
    fn __self__(&self) -> PyObjectRef {
        self.obj.clone()
    }

    #[pygetset]
    fn __name__(&self) -> &'static PyStrInterned {
        self.wrapper.name
    }

    #[pygetset]
    fn __objclass__(&self) -> PyTypeRef {
        self.wrapper.typ.to_owned()
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
    fn hash(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyHash> {
        let obj_hash = zelf.obj.hash(vm)?;
        let wrapper_hash = zelf.wrapper.as_object().get_id() as PyHash;
        Ok(obj_hash ^ wrapper_hash)
    }
}

impl Comparable for PyMethodWrapper {
    fn cmp(
        zelf: &Py<Self>,
        other: &PyObject,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<crate::function::PyComparisonValue> {
        op.eq_only(|| {
            let other = class_or_notimplemented!(Self, other);
            let eq = zelf.wrapper.is(&other.wrapper) && vm.bool_eq(&zelf.obj, &other.obj)?;
            Ok(eq.into())
        })
    }
}
