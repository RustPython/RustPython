use super::dict::{PyDict, PyDictRef};
use super::list::PyList;
use super::pybool;
use super::pystr::PyStrRef;
use super::pytype::PyTypeRef;
use crate::builtins::pytype::PyType;
use crate::common::hash::PyHash;
use crate::function::FuncArgs;
use crate::pyobject::{
    BorrowValue, Either, IdProtocol, ItemProtocol, PyArithmaticValue, PyAttributes, PyClassImpl,
    PyComparisonValue, PyContext, PyObject, PyObjectRef, PyResult, PyValue, TryFromObject,
    TypeProtocol,
};
use crate::slots::PyComparisonOp;
use crate::vm::VirtualMachine;

/// The most base type
#[pyclass(module = false, name = "object")]
#[derive(Debug)]
pub struct PyBaseObject;

impl PyValue for PyBaseObject {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.object_type
    }
}

#[pyimpl(flags(BASETYPE))]
impl PyBaseObject {
    #[pyslot]
    fn tp_new(mut args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        // more or less __new__ operator
        let cls = PyTypeRef::try_from_object(vm, args.shift())?;
        let dict = if cls.is(&vm.ctx.types.object_type) {
            None
        } else {
            Some(vm.ctx.new_dict())
        };
        Ok(PyObject::new(PyBaseObject, cls, dict))
    }

    #[pyslot]
    fn tp_cmp(
        zelf: &PyObjectRef,
        other: &PyObjectRef,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<Either<PyObjectRef, PyComparisonValue>> {
        Self::cmp(zelf, other, op, vm).map(Either::B)
    }

    #[inline(always)]
    fn cmp(
        zelf: &PyObjectRef,
        other: &PyObjectRef,
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
                    .mro_find_map(|cls| cls.slots.cmp.load())
                    .unwrap();
                let value = match cmp(zelf, other, PyComparisonOp::Eq, vm)? {
                    Either::A(obj) => PyArithmaticValue::from_object(vm, obj)
                        .map(|obj| pybool::boolval(vm, obj))
                        .transpose()?,
                    Either::B(value) => value,
                };
                value.map(|v| !v)
            }
            _ => PyComparisonValue::NotImplemented,
        };
        Ok(res)
    }

    #[pymethod(magic)]
    fn eq(
        zelf: PyObjectRef,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        Self::cmp(&zelf, &other, PyComparisonOp::Eq, vm)
    }
    #[pymethod(magic)]
    fn ne(
        zelf: PyObjectRef,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        Self::cmp(&zelf, &other, PyComparisonOp::Ne, vm)
    }
    #[pymethod(magic)]
    fn lt(
        zelf: PyObjectRef,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        Self::cmp(&zelf, &other, PyComparisonOp::Lt, vm)
    }
    #[pymethod(magic)]
    fn le(
        zelf: PyObjectRef,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        Self::cmp(&zelf, &other, PyComparisonOp::Le, vm)
    }
    #[pymethod(magic)]
    fn ge(
        zelf: PyObjectRef,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        Self::cmp(&zelf, &other, PyComparisonOp::Ge, vm)
    }
    #[pymethod(magic)]
    fn gt(
        zelf: PyObjectRef,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        Self::cmp(&zelf, &other, PyComparisonOp::Gt, vm)
    }

    #[pymethod]
    fn __setattr__(
        obj: PyObjectRef,
        attr_name: PyStrRef,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        setattr(&obj, attr_name, Some(value), vm)
    }

    #[pymethod]
    fn __delattr__(obj: PyObjectRef, attr_name: PyStrRef, vm: &VirtualMachine) -> PyResult<()> {
        setattr(&obj, attr_name, None, vm)
    }

    #[pyslot]
    fn tp_setattro(
        obj: &PyObjectRef,
        attr_name: PyStrRef,
        value: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        setattr(obj, attr_name, value, vm)
    }

    #[pymethod(magic)]
    fn str(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        vm.to_repr(&zelf)
    }

    #[pymethod(magic)]
    fn repr(zelf: PyObjectRef) -> String {
        format!("<{} object at {:#x}>", zelf.class().name, zelf.get_id())
    }

    #[pyclassmethod(magic)]
    fn subclasshook(_args: FuncArgs, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.not_implemented()
    }

    #[pyclassmethod(magic)]
    fn init_subclass(_cls: PyTypeRef) {}

    #[pymethod(magic)]
    pub fn dir(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyList> {
        let attributes: PyAttributes = obj.class().get_attributes();

        let dict = PyDict::from_attributes(attributes, vm)?.into_ref(vm);

        // Get instance attributes:
        if let Some(object_dict) = obj.dict() {
            vm.call_method(dict.as_object(), "update", (object_dict,))?;
        }

        let attributes: Vec<_> = dict.into_iter().map(|(k, _v)| k).collect();

        Ok(PyList::from(attributes))
    }

    #[pymethod(magic)]
    fn format(obj: PyObjectRef, format_spec: PyStrRef, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        if format_spec.borrow_value().is_empty() {
            vm.to_str(&obj)
        } else {
            Err(vm.new_type_error(
                "unsupported format string passed to object.__format__".to_string(),
            ))
        }
    }

    #[pymethod(magic)]
    fn init(_args: FuncArgs) {}

    #[pyproperty(name = "__class__")]
    fn get_class(obj: PyObjectRef) -> PyObjectRef {
        obj.clone_class().into_object()
    }

    #[pyproperty(name = "__class__", setter)]
    fn set_class(instance: PyObjectRef, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        if instance.payload_is::<PyBaseObject>() {
            match value.downcast::<PyType>() {
                Ok(cls) => {
                    // FIXME(#1979) cls instances might have a payload
                    *instance.class_lock().write() = cls;
                    Ok(())
                }
                Err(value) => {
                    let type_repr = &value.class().name;
                    Err(vm.new_type_error(format!(
                        "__class__ must be set to a class, not '{}' object",
                        type_repr
                    )))
                }
            }
        } else {
            Err(vm.new_type_error(
                "__class__ assignment only supported for types without a payload".to_owned(),
            ))
        }
    }

    #[pymethod(name = "__getattribute__")]
    #[pyslot]
    pub(crate) fn getattro(obj: PyObjectRef, name: PyStrRef, vm: &VirtualMachine) -> PyResult {
        vm_trace!("object.__getattribute__({:?}, {:?})", obj, name);
        vm.generic_getattribute(obj, name)
    }

    #[pymethod(magic)]
    fn reduce(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        common_reduce(obj, 0, vm)
    }

    #[pymethod(magic)]
    fn reduce_ex(obj: PyObjectRef, proto: usize, vm: &VirtualMachine) -> PyResult {
        if let Some(reduce) = vm.get_attribute_opt(obj.clone(), "__reduce__")? {
            let object_reduce = vm.ctx.types.object_type.get_attr("__reduce__").unwrap();
            let class_reduce = vm.get_attribute(obj.clone_class().into_object(), "__reduce__")?;
            if !class_reduce.is(&object_reduce) {
                return vm.invoke(&reduce, ());
            }
        }
        common_reduce(obj, proto, vm)
    }

    #[pyslot]
    fn tp_hash(zelf: &PyObjectRef, _vm: &VirtualMachine) -> PyResult<PyHash> {
        Ok(zelf.get_id() as _)
    }

    #[pymethod(magic)]
    fn hash(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyHash> {
        Self::tp_hash(&zelf, vm)
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

#[cfg_attr(feature = "flame-it", flame)]
pub(crate) fn setattr(
    obj: &PyObjectRef,
    attr_name: PyStrRef,
    value: Option<PyObjectRef>,
    vm: &VirtualMachine,
) -> PyResult<()> {
    vm_trace!("object.__setattr__({:?}, {}, {:?})", obj, attr_name, value);

    if let Some(attr) = obj.get_class_attr(attr_name.borrow_value()) {
        let descr_set = attr.class().mro_find_map(|cls| cls.slots.descr_set.load());
        if let Some(descriptor) = descr_set {
            return descriptor(attr, obj.clone(), value, vm);
        }
    }

    if let Some(dict) = obj.dict() {
        if let Some(value) = value {
            dict.set_item(attr_name, value, vm)?;
        } else {
            dict.del_item(attr_name, vm)?;
        }
        Ok(())
    } else {
        Err(vm.new_attribute_error(format!(
            "'{}' object has no attribute '{}'",
            obj.class().name,
            attr_name.borrow_value()
        )))
    }
}

pub fn init(context: &PyContext) {
    PyBaseObject::extend_class(context, &context.types.object_type);
}

fn common_reduce(obj: PyObjectRef, proto: usize, vm: &VirtualMachine) -> PyResult {
    if proto >= 2 {
        let reducelib = vm.import("__reducelib", None, 0)?;
        let reduce_2 = vm.get_attribute(reducelib, "reduce_2")?;
        vm.invoke(&reduce_2, (obj,))
    } else {
        let copyreg = vm.import("copyreg", None, 0)?;
        let reduce_ex = vm.get_attribute(copyreg, "_reduce_ex")?;
        vm.invoke(&reduce_ex, (obj, proto))
    }
}
