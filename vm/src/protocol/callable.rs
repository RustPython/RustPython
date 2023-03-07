use crate::{
    function::IntoFuncArgs,
    types::GenericMethod,
    {PyObject, PyResult, VirtualMachine}
};

impl PyObject {
    #[inline]
    pub fn to_callable(&self) -> Option<PyCallable<'_>> {
        PyCallable::new(self)
    }

    #[inline]
    pub fn is_callable(&self) -> bool {
        self.to_callable().is_some()
    }
}

pub struct PyCallable<'a> {
    pub obj: &'a PyObject,
    pub call: GenericMethod,
}

impl<'a> PyCallable<'a> {
    pub fn new(obj: &'a PyObject) -> Option<Self> {
        let call = obj.class().mro_find_map(|cls| cls.slots.call.load())?;
        Some(PyCallable { obj, call })
    }

    pub fn invoke(&self, args: impl IntoFuncArgs, vm: &VirtualMachine) -> PyResult {
        (self.call)(self.obj, args.into_args(vm), vm)
    }
}
