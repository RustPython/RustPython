use crate::function::PyFuncArgs;
use crate::obj::objproperty::PyPropertyRef;
use crate::obj::objstr::PyStringRef;
use crate::pyobject::{
    AttributeProtocol, IntoPyObject, PyContext, PyObjectRef, PyRef, PyResult, PyValue,
    TryFromObject, TypeProtocol,
};
use crate::vm::VirtualMachine;

#[derive(Clone, Debug)]
pub struct PyNone;
pub type PyNoneRef = PyRef<PyNone>;

impl PyValue for PyNone {
    fn class(vm: &VirtualMachine) -> Vec<PyObjectRef> {
        vec![vm.ctx.none().typ()]
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

impl PyNoneRef {
    fn repr(self, _vm: &VirtualMachine) -> PyResult<String> {
        Ok("None".to_string())
    }

    fn bool(self, _vm: &VirtualMachine) -> PyResult<bool> {
        Ok(false)
    }

    fn get_attribute(self, name: PyStringRef, vm: &VirtualMachine) -> PyResult {
        trace!("None.__getattribute__({:?}, {:?})", self, name);
        let cls = self.typ().into_object();

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
                vm.invoke(get_func, vec![descriptor, obj, cls])
            }
        }

        if let Some(attr) = cls.get_attr(&name.value) {
            let attr_class = attr.typ();
            if attr_class.has_attr("__set__") {
                if let Some(get_func) = attr_class.get_attr("__get__") {
                    return call_descriptor(attr, get_func, self.into_object(), cls.clone(), vm);
                }
            }
        }

        if let Some(obj_attr) = self.as_object().get_attr(&name.value) {
            Ok(obj_attr)
        } else if let Some(attr) = cls.get_attr(&name.value) {
            let attr_class = attr.typ();
            if let Some(get_func) = attr_class.get_attr("__get__") {
                call_descriptor(attr, get_func, self.into_object(), cls.clone(), vm)
            } else {
                Ok(attr)
            }
        } else if let Some(getter) = cls.get_attr("__getattr__") {
            vm.invoke(getter, vec![self.into_object(), name.into_object()])
        } else {
            Err(vm.new_attribute_error(format!("{} has no attribute '{}'", self.as_object(), name)))
        }
    }
}

fn none_new(vm: &VirtualMachine, args: PyFuncArgs) -> PyResult {
    arg_check!(
        vm,
        args,
        required = [(_zelf, Some(vm.ctx.type_type.clone()))]
    );
    Ok(vm.get_none())
}

pub fn init(context: &PyContext) {
    extend_class!(context, &context.none.typ(), {
        "__new__" => context.new_rustfunc(none_new),
        "__repr__" => context.new_rustfunc(PyNoneRef::repr),
        "__bool__" => context.new_rustfunc(PyNoneRef::bool),
        "__getattribute__" => context.new_rustfunc(PyNoneRef::get_attribute)
    });
}
