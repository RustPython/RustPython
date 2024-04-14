use super::{PyDictRef, PyList, PyStr, PyStrRef, PyType, PyTypeRef};
use crate::common::hash::PyHash;
use crate::types::PyTypeFlags;
use crate::{
    class::PyClassImpl,
    function::{Either, FuncArgs, PyArithmeticValue, PyComparisonValue, PySetterValue},
    types::{Constructor, PyComparisonOp},
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyResult, VirtualMachine,
};
use itertools::Itertools;

/// object()
/// --
///
/// The base class of the class hierarchy.
///
/// When called, it accepts no arguments and returns a new featureless
/// instance that has no instance attributes and cannot be given any.
#[pyclass(module = false, name = "object")]
#[derive(Debug)]
pub struct PyBaseObject;

impl PyPayload for PyBaseObject {
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.object_type
    }
}

impl Constructor for PyBaseObject {
    type Args = FuncArgs;

    fn py_new(cls: PyTypeRef, _args: Self::Args, vm: &VirtualMachine) -> PyResult {
        // more or less __new__ operator
        let dict = if cls.is(vm.ctx.types.object_type) {
            None
        } else {
            Some(vm.ctx.new_dict())
        };

        // Ensure that all abstract methods are implemented before instantiating instance.
        if let Some(abs_methods) = cls.get_attr(identifier!(vm, __abstractmethods__)) {
            if let Some(unimplemented_abstract_method_count) = abs_methods.length_opt(vm) {
                let methods: Vec<PyStrRef> = abs_methods.try_to_value(vm)?;
                let methods: String =
                    Itertools::intersperse(methods.iter().map(|name| name.as_str()), "', '")
                        .collect();

                let unimplemented_abstract_method_count = unimplemented_abstract_method_count?;
                let name = cls.name().to_string();

                match unimplemented_abstract_method_count {
                    0 => {}
                    1 => {
                        return Err(vm.new_type_error(format!(
                            "class {} without an implementation for abstract method '{}'",
                            name, methods
                        )));
                    }
                    2.. => {
                        return Err(vm.new_type_error(format!(
                            "class {} without an implementation for abstract methods '{}'",
                            name, methods
                        )));
                    }
                    // TODO: remove `allow` when redox build doesn't complain about it
                    #[allow(unreachable_patterns)]
                    _ => unreachable!(),
                }
            }
        }

        Ok(crate::PyRef::new_ref(PyBaseObject, cls, dict).into())
    }
}

#[pyclass(with(Constructor), flags(BASETYPE))]
impl PyBaseObject {
    #[pyslot]
    fn slot_richcompare(
        zelf: &PyObject,
        other: &PyObject,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<Either<PyObjectRef, PyComparisonValue>> {
        Self::cmp(zelf, other, op, vm).map(Either::B)
    }

    #[inline(always)]
    fn cmp(
        zelf: &PyObject,
        other: &PyObject,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        let res = match op {
            PyComparisonOp::Eq => {
                if zelf.is(other) {
                    PyComparisonValue::Implemented(true)
                } else {
                    PyComparisonValue::NotImplemented
                }
            }
            PyComparisonOp::Ne => {
                let cmp = zelf
                    .class()
                    .mro_find_map(|cls| cls.slots.richcompare.load())
                    .unwrap();
                let value = match cmp(zelf, other, PyComparisonOp::Eq, vm)? {
                    Either::A(obj) => PyArithmeticValue::from_object(vm, obj)
                        .map(|obj| obj.try_to_bool(vm))
                        .transpose()?,
                    Either::B(value) => value,
                };
                value.map(|v| !v)
            }
            _ => PyComparisonValue::NotImplemented,
        };
        Ok(res)
    }

    /// Return self==value.
    #[pymethod(magic)]
    fn eq(
        zelf: PyObjectRef,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        Self::cmp(&zelf, &value, PyComparisonOp::Eq, vm)
    }

    /// Return self!=value.
    #[pymethod(magic)]
    fn ne(
        zelf: PyObjectRef,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        Self::cmp(&zelf, &value, PyComparisonOp::Ne, vm)
    }

    /// Return self<value.
    #[pymethod(magic)]
    fn lt(
        zelf: PyObjectRef,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        Self::cmp(&zelf, &value, PyComparisonOp::Lt, vm)
    }

    /// Return self<=value.
    #[pymethod(magic)]
    fn le(
        zelf: PyObjectRef,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        Self::cmp(&zelf, &value, PyComparisonOp::Le, vm)
    }

    /// Return self>=value.
    #[pymethod(magic)]
    fn ge(
        zelf: PyObjectRef,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        Self::cmp(&zelf, &value, PyComparisonOp::Ge, vm)
    }

    /// Return self>value.
    #[pymethod(magic)]
    fn gt(
        zelf: PyObjectRef,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        Self::cmp(&zelf, &value, PyComparisonOp::Gt, vm)
    }

    /// Implement setattr(self, name, value).
    #[pymethod]
    fn __setattr__(
        obj: PyObjectRef,
        name: PyStrRef,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        obj.generic_setattr(&name, PySetterValue::Assign(value), vm)
    }

    /// Implement delattr(self, name).
    #[pymethod]
    fn __delattr__(obj: PyObjectRef, name: PyStrRef, vm: &VirtualMachine) -> PyResult<()> {
        obj.generic_setattr(&name, PySetterValue::Delete, vm)
    }

    #[pyslot]
    fn slot_setattro(
        obj: &PyObject,
        attr_name: &Py<PyStr>,
        value: PySetterValue,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        obj.generic_setattr(attr_name, value, vm)
    }

    /// Return str(self).
    #[pymethod(magic)]
    fn str(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        // FIXME: try tp_repr first and fallback to object.__repr__
        zelf.repr(vm)
    }

    #[pyslot]
    fn slot_repr(zelf: &PyObject, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        let class = zelf.class();
        match (
            class
                .qualname(vm)
                .downcast_ref::<PyStr>()
                .map(|n| n.as_str()),
            class.module(vm).downcast_ref::<PyStr>().map(|m| m.as_str()),
        ) {
            (None, _) => Err(vm.new_type_error("Unknown qualified name".into())),
            (Some(qualname), Some(module)) if module != "builtins" => Ok(PyStr::from(format!(
                "<{}.{} object at {:#x}>",
                module,
                qualname,
                zelf.get_id()
            ))
            .into_ref(&vm.ctx)),
            _ => Ok(PyStr::from(format!(
                "<{} object at {:#x}>",
                class.slot_name(),
                zelf.get_id()
            ))
            .into_ref(&vm.ctx)),
        }
    }

    /// Return repr(self).
    #[pymethod(magic)]
    fn repr(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        Self::slot_repr(&zelf, vm)
    }

    #[pyclassmethod(magic)]
    fn subclasshook(_args: FuncArgs, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.not_implemented()
    }

    #[pyclassmethod(magic)]
    fn init_subclass(_cls: PyTypeRef) {}

    #[pymethod(magic)]
    pub fn dir(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyList> {
        obj.dir(vm)
    }

    #[pymethod(magic)]
    fn format(obj: PyObjectRef, format_spec: PyStrRef, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        if !format_spec.is_empty() {
            return Err(vm.new_type_error(format!(
                "unsupported format string passed to {}.__format__",
                obj.class().name()
            )));
        }
        obj.str(vm)
    }

    #[pyslot]
    #[pymethod(magic)]
    fn init(_zelf: PyObjectRef, _args: FuncArgs, _vm: &VirtualMachine) -> PyResult<()> {
        Ok(())
    }

    #[pygetset(name = "__class__")]
    fn get_class(obj: PyObjectRef) -> PyTypeRef {
        obj.class().to_owned()
    }

    #[pygetset(name = "__class__", setter)]
    fn set_class(instance: PyObjectRef, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        match value.downcast::<PyType>() {
            Ok(cls) => {
                let both_module = instance.class().fast_issubclass(vm.ctx.types.module_type)
                    && cls.fast_issubclass(vm.ctx.types.module_type);
                let both_mutable = !instance
                    .class()
                    .slots
                    .flags
                    .has_feature(PyTypeFlags::IMMUTABLETYPE)
                    && !cls.slots.flags.has_feature(PyTypeFlags::IMMUTABLETYPE);
                // FIXME(#1979) cls instances might have a payload
                if both_mutable || both_module {
                    instance.set_class(cls, vm);
                    Ok(())
                } else {
                    Err(vm.new_type_error(
                        "__class__ assignment only supported for mutable types or ModuleType subclasses"
                            .to_owned(),
                    ))
                }
            }
            Err(value) => {
                let value_class = value.class();
                let type_repr = &value_class.name();
                Err(vm.new_type_error(format!(
                    "__class__ must be set to a class, not '{type_repr}' object"
                )))
            }
        }
    }

    /// Return getattr(self, name).
    #[pyslot]
    pub(crate) fn getattro(obj: &PyObject, name: &Py<PyStr>, vm: &VirtualMachine) -> PyResult {
        vm_trace!("object.__getattribute__({:?}, {:?})", obj, name);
        obj.as_object().generic_getattr(name, vm)
    }

    #[pymethod(magic)]
    fn getattribute(obj: PyObjectRef, name: PyStrRef, vm: &VirtualMachine) -> PyResult {
        Self::getattro(&obj, &name, vm)
    }

    #[pymethod(magic)]
    fn reduce(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        common_reduce(obj, 0, vm)
    }

    #[pymethod(magic)]
    fn reduce_ex(obj: PyObjectRef, proto: usize, vm: &VirtualMachine) -> PyResult {
        let __reduce__ = identifier!(vm, __reduce__);
        if let Some(reduce) = vm.get_attribute_opt(obj.clone(), __reduce__)? {
            let object_reduce = vm.ctx.types.object_type.get_attr(__reduce__).unwrap();
            let typ_obj: PyObjectRef = obj.class().to_owned().into();
            let class_reduce = typ_obj.get_attr(__reduce__, vm)?;
            if !class_reduce.is(&object_reduce) {
                return reduce.call((), vm);
            }
        }
        common_reduce(obj, proto, vm)
    }

    #[pyslot]
    fn slot_hash(zelf: &PyObject, _vm: &VirtualMachine) -> PyResult<PyHash> {
        Ok(zelf.get_id() as _)
    }

    /// Return hash(self).
    #[pymethod(magic)]
    fn hash(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyHash> {
        Self::slot_hash(&zelf, vm)
    }

    #[pymethod(magic)]
    fn sizeof(zelf: PyObjectRef) -> usize {
        zelf.class().slots.basicsize
    }
}

pub fn object_get_dict(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyDictRef> {
    obj.dict()
        .ok_or_else(|| vm.new_attribute_error("This object has no __dict__".to_owned()))
}
pub fn object_set_dict(obj: PyObjectRef, dict: PyDictRef, vm: &VirtualMachine) -> PyResult<()> {
    obj.set_dict(dict)
        .map_err(|_| vm.new_attribute_error("This object has no __dict__".to_owned()))
}

pub fn init(ctx: &Context) {
    PyBaseObject::extend_class(ctx, ctx.types.object_type);
}

fn common_reduce(obj: PyObjectRef, proto: usize, vm: &VirtualMachine) -> PyResult {
    if proto >= 2 {
        let reducelib = vm.import("__reducelib", 0)?;
        let reduce_2 = reducelib.get_attr("reduce_2", vm)?;
        reduce_2.call((obj,), vm)
    } else {
        let copyreg = vm.import("copyreg", 0)?;
        let reduce_ex = copyreg.get_attr("_reduce_ex", vm)?;
        reduce_ex.call((obj, proto), vm)
    }
}
