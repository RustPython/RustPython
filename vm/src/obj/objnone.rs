use super::objproperty::PyPropertyRef;
use super::objstr::PyStringRef;
use super::objtype::{class_get_attr, class_has_attr, PyClassRef};
use crate::pyobject::{
    IntoPyObject, PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue, TryFromObject,
    TypeProtocol,
};
use crate::vm::VirtualMachine;

#[pyclass(name = "NoneType")]
#[derive(Debug)]
pub struct PyNone;
pub type PyNoneRef = PyRef<PyNone>;

impl PyValue for PyNone {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.none().class()
    }
}

// This allows a built-in function to not return a value, mapping to
// Python's behavior of returning `None` in this situation.
impl IntoPyObject for () {
    fn into_pyobject(self, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.none())
    }
}

impl<T: IntoPyObject> IntoPyObject for Option<T> {
    fn into_pyobject(self, vm: &VirtualMachine) -> PyResult {
        match self {
            Some(x) => x.into_pyobject(vm),
            None => Ok(vm.ctx.none()),
        }
    }
}

#[pyimpl]
impl PyNoneRef {
    #[pyslot(new)]
    fn tp_new(_: PyClassRef, vm: &VirtualMachine) -> PyNoneRef {
        vm.ctx.none.clone()
    }

    #[pymethod(name = "__repr__")]
    fn repr(self, _vm: &VirtualMachine) -> PyResult<String> {
        Ok("None".to_string())
    }

    #[pymethod(name = "__bool__")]
    fn bool(self, _vm: &VirtualMachine) -> PyResult<bool> {
        Ok(false)
    }

    #[pymethod(name = "__getattribute__")]
    fn get_attribute(self, name: PyStringRef, vm: &VirtualMachine) -> PyResult {
        vm_trace!("None.__getattribute__({:?}, {:?})", self, name);
        let cls = self.class();

        // Properties use a comparision with None to determine if they are either invoked by am
        // instance binding or a class binding. But if the object itself is None then this detection
        // won't work. Instead we call a special function on property that bypasses this check, as
        // we are invoking it as a instance binding.
        //
        // In CPython they instead call the slot tp_descr_get with NULL to indicates it's an
        // instance binding.
        // https://github.com/python/cpython/blob/master/Objects/typeobject.c#L3281
        fn call_descriptor(
            descriptor: PyObjectRef,
            get_func: PyObjectRef,
            obj: PyObjectRef,
            cls: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult {
            if let Ok(property) = PyPropertyRef::try_from_object(vm, descriptor.clone()) {
                property.instance_binding_get(obj, vm)
            } else {
                vm.invoke(&get_func, vec![descriptor, obj, cls])
            }
        }

        if let Some(attr) = class_get_attr(&cls, name.as_str()) {
            let attr_class = attr.class();
            if class_has_attr(&attr_class, "__set__") {
                if let Some(get_func) = class_get_attr(&attr_class, "__get__") {
                    return call_descriptor(
                        attr,
                        get_func,
                        self.into_object(),
                        cls.into_object(),
                        vm,
                    );
                }
            }
        }

        // None has no attributes and cannot have attributes set on it.
        // if let Some(obj_attr) = self.as_object().get_attr(name.as_str()) {
        //     Ok(obj_attr)
        // } else
        if let Some(attr) = class_get_attr(&cls, name.as_str()) {
            let attr_class = attr.class();
            if let Some(get_func) = class_get_attr(&attr_class, "__get__") {
                call_descriptor(attr, get_func, self.into_object(), cls.into_object(), vm)
            } else {
                Ok(attr)
            }
        } else if let Some(getter) = class_get_attr(&cls, "__getattr__") {
            vm.invoke(&getter, vec![self.into_object(), name.into_object()])
        } else {
            Err(vm.new_attribute_error(format!("{} has no attribute '{}'", self.as_object(), name)))
        }
    }

    #[pymethod(name = "__eq__")]
    fn eq(self, rhs: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if vm.is_none(&rhs) {
            vm.ctx.new_bool(true)
        } else {
            vm.ctx.not_implemented()
        }
    }

    #[pymethod(name = "__ne__")]
    fn ne(self, rhs: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if vm.is_none(&rhs) {
            vm.ctx.new_bool(false)
        } else {
            vm.ctx.not_implemented()
        }
    }
}

pub fn init(context: &PyContext) {
    PyNoneRef::extend_class(context, &context.none.class());
}
