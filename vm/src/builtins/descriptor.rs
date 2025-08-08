use super::{PyStr, PyStrInterned, PyType};
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    builtins::{PyTypeRef, builtin_func::PyNativeMethod, type_},
    class::PyClassImpl,
    function::{FuncArgs, PyMethodDef, PyMethodFlags, PySetterValue},
    types::{Callable, GetDescriptor, Representable, Unconstructible},
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
    with(GetDescriptor, Callable, Unconstructible, Representable),
    flags(METHOD_DESCRIPTOR)
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

impl Unconstructible for PyMethodDescriptor {}

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

// PyMemberDescrObject in CPython
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

#[pyclass(with(GetDescriptor, Unconstructible, Representable), flags(BASETYPE))]
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

impl Unconstructible for PyMemberDescriptor {}

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
                if let Some(cls) = cls {
                    if let Ok(cls_type) = cls.downcast::<PyType>() {
                        if let Some(interned) = vm.ctx.interned_str(descr.member.name.as_str()) {
                            if let Some(attr) = cls_type.attributes.read().get(&interned) {
                                return Ok(attr.clone());
                            }
                        }
                    }
                }
                Ok(zelf)
            }
        }
    }
}

pub fn init(ctx: &Context) {
    PyMemberDescriptor::extend_class(ctx, ctx.types.member_descriptor_type);
    PyMethodDescriptor::extend_class(ctx, ctx.types.method_descriptor_type);
}
