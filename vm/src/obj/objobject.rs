use super::objbool;
use super::objdict::PyDictRef;
use super::objlist::PyList;
use super::objstr::PyStringRef;
use super::objtype::PyClassRef;
use crate::function::{OptionalArg, PyFuncArgs};
use crate::obj::objtype::PyClass;
use crate::pyhash;
use crate::pyobject::{
    IdProtocol, ItemProtocol, PyArithmaticValue::*, PyAttributes, PyClassImpl, PyComparisonValue,
    PyContext, PyObject, PyObjectRef, PyResult, PyValue, TryFromObject, TypeProtocol,
};
use crate::vm::VirtualMachine;

/// The most base type
#[pyclass]
#[derive(Debug)]
pub struct PyBaseObject;

impl PyValue for PyBaseObject {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.object()
    }
}

#[pyimpl(flags(BASETYPE))]
impl PyBaseObject {
    #[pyslot]
    fn tp_new(vm: &VirtualMachine, mut args: PyFuncArgs) -> PyResult {
        // more or less __new__ operator
        let cls = PyClassRef::try_from_object(vm, args.shift())?;
        let dict = if cls.is(&vm.ctx.object()) {
            None
        } else {
            Some(vm.ctx.new_dict())
        };
        Ok(PyObject::new(PyBaseObject, cls, dict))
    }

    #[pymethod(magic)]
    fn eq(zelf: PyObjectRef, other: PyObjectRef) -> PyComparisonValue {
        if zelf.is(&other) {
            Implemented(true)
        } else {
            NotImplemented
        }
    }

    #[pymethod(magic)]
    fn ne(
        zelf: PyObjectRef,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        let eq_method = match vm.get_method(zelf, "__eq__") {
            Some(func) => func?,
            None => return Ok(NotImplemented), // XXX: is this a possible case?
        };
        let eq = vm.invoke(&eq_method, vec![other])?;
        if eq.is(&vm.ctx.not_implemented()) {
            return Ok(NotImplemented);
        }
        let bool_eq = objbool::boolval(vm, eq)?;
        Ok(Implemented(!bool_eq))
    }

    #[pymethod(magic)]
    fn lt(_zelf: PyObjectRef, _other: PyObjectRef) -> PyComparisonValue {
        NotImplemented
    }

    #[pymethod(magic)]
    fn le(_zelf: PyObjectRef, _other: PyObjectRef) -> PyComparisonValue {
        NotImplemented
    }

    #[pymethod(magic)]
    fn gt(_zelf: PyObjectRef, _other: PyObjectRef) -> PyComparisonValue {
        NotImplemented
    }

    #[pymethod(magic)]
    fn ge(_zelf: PyObjectRef, _other: PyObjectRef) -> PyComparisonValue {
        NotImplemented
    }

    #[pymethod(magic)]
    fn hash(zelf: PyObjectRef) -> pyhash::PyHash {
        zelf.get_id() as pyhash::PyHash
    }

    #[pymethod(magic)]
    pub(crate) fn setattr(
        obj: PyObjectRef,
        attr_name: PyStringRef,
        value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        setattr(obj, attr_name, value, vm)
    }

    #[pymethod(magic)]
    fn delattr(obj: PyObjectRef, attr_name: PyStringRef, vm: &VirtualMachine) -> PyResult<()> {
        let cls = obj.class();

        if let Some(attr) = cls.get_attr(attr_name.as_str()) {
            if let Some(descriptor) = attr.class().get_attr("__delete__") {
                return vm.invoke(&descriptor, vec![attr, obj.clone()]).map(|_| ());
            }
        }

        if let Some(dict) = obj.dict() {
            dict.del_item(attr_name.as_str(), vm)?;
            Ok(())
        } else {
            Err(vm.new_attribute_error(format!(
                "'{}' object has no attribute '{}'",
                obj.lease_class().name,
                attr_name.as_str()
            )))
        }
    }

    #[pymethod(magic)]
    fn str(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        vm.call_method(&zelf, "__repr__", vec![])
    }

    #[pymethod(magic)]
    fn repr(zelf: PyObjectRef) -> String {
        format!("<{} object at 0x{:x}>", zelf.lease_class().name, zelf.get_id())
    }

    #[pyclassmethod(magic)]
    fn subclasshook(vm: &VirtualMachine, _args: PyFuncArgs) -> PyResult {
        Ok(vm.ctx.not_implemented())
    }

    #[pymethod(magic)]
    pub fn dir(obj: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyList> {
        let attributes: PyAttributes = obj.class().get_attributes();

        let dict = PyDictRef::from_attributes(attributes, vm)?;

        // Get instance attributes:
        if let Some(object_dict) = obj.dict() {
            vm.invoke(
                &vm.get_attribute(dict.clone().into_object(), "update")?,
                object_dict.into_object(),
            )?;
        }

        let attributes: Vec<_> = dict.into_iter().map(|(k, _v)| k).collect();

        Ok(PyList::from(attributes))
    }

    #[pymethod(magic)]
    fn format(
        obj: PyObjectRef,
        format_spec: PyStringRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyStringRef> {
        if format_spec.as_str().is_empty() {
            vm.to_str(&obj)
        } else {
            Err(vm.new_type_error(
                "unsupported format string passed to object.__format__".to_string(),
            ))
        }
    }

    #[pymethod(magic)]
    fn init(vm: &VirtualMachine, _args: PyFuncArgs) -> PyResult {
        Ok(vm.ctx.none())
    }

    #[pyproperty(name = "__class__")]
    fn get_class(obj: PyObjectRef) -> PyObjectRef {
        obj.class().into_object()
    }

    #[pyproperty(name = "__class__", setter)]
    fn set_class(instance: PyObjectRef, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        if instance.payload_is::<PyBaseObject>() {
            match value.downcast_generic::<PyClass>() {
                Ok(cls) => {
                    // FIXME(#1979) cls instances might have a payload
                    instance.typ.store(cls);
                    Ok(())
                }
                Err(value) => {
                    let type_repr = vm.to_pystr(&value.class())?;
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

    #[pyproperty(magic)]
    fn dict(object: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyDictRef> {
        object.dict().ok_or_else(|| {
            vm.new_attribute_error(format!(
                "'{}' object has no attribute '__dict__'",
                object.lease_class().name
            ))
        })
    }

    #[pyproperty(magic, setter)]
    fn set_dict(instance: PyObjectRef, value: PyDictRef, vm: &VirtualMachine) -> PyResult<()> {
        instance.set_dict(value).map_err(|_| {
            vm.new_attribute_error(format!(
                "'{}' object has no attribute '__dict__'",
                instance.lease_class().name
            ))
        })
    }

    #[pymethod(magic)]
    fn getattribute(obj: PyObjectRef, name: PyStringRef, vm: &VirtualMachine) -> PyResult {
        vm_trace!("object.__getattribute__({:?}, {:?})", obj, name);
        vm.generic_getattribute(obj, name)
    }

    #[pymethod(magic)]
    fn reduce(obj: PyObjectRef, proto: OptionalArg<usize>, vm: &VirtualMachine) -> PyResult {
        common_reduce(obj, proto.unwrap_or(0), vm)
    }

    #[pymethod(magic)]
    fn reduce_ex(obj: PyObjectRef, proto: usize, vm: &VirtualMachine) -> PyResult {
        let cls = obj.class();
        if let Some(reduce) = cls.get_attr("__reduce__") {
            let object_reduce = vm.ctx.types.object_type.get_attr("__reduce__").unwrap();
            if !reduce.is(&object_reduce) {
                return vm.invoke(&reduce, vec![]);
            }
        }
        common_reduce(obj, proto, vm)
    }
}

pub(crate) fn setattr(
    obj: PyObjectRef,
    attr_name: PyStringRef,
    value: PyObjectRef,
    vm: &VirtualMachine,
) -> PyResult<()> {
    vm_trace!("object.__setattr__({:?}, {}, {:?})", obj, attr_name, value);
    let cls = obj.class();

    if let Some(attr) = cls.get_attr(attr_name.as_str()) {
        if let Some(descriptor) = attr.class().get_attr("__set__") {
            return vm
                .invoke(&descriptor, vec![attr, obj.clone(), value])
                .map(|_| ());
        }
    }

    if let Some(dict) = obj.dict() {
        dict.set_item(attr_name.as_str(), value, vm)?;
        Ok(())
    } else {
        Err(vm.new_attribute_error(format!(
            "'{}' object has no attribute '{}'",
            obj.lease_class().name,
            attr_name.as_str()
        )))
    }
}

pub fn init(context: &PyContext) {
    PyBaseObject::extend_class(context, &context.types.object_type);
}

fn common_reduce(obj: PyObjectRef, proto: usize, vm: &VirtualMachine) -> PyResult {
    if proto >= 2 {
        let reducelib = vm.import("__reducelib", &[], 0)?;
        let reduce_2 = vm.get_attribute(reducelib, "reduce_2")?;
        vm.invoke(&reduce_2, vec![obj])
    } else {
        let copyreg = vm.import("copyreg", &[], 0)?;
        let reduce_ex = vm.get_attribute(copyreg, "_reduce_ex")?;
        vm.invoke(&reduce_ex, vec![obj, vm.new_int(proto)])
    }
}
