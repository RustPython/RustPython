/*! Python `attribute` descriptor class. (PyGetSet)

*/
use super::objtype::PyClassRef;
use crate::function::OptionalArg;
use crate::pyobject::{PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue};
use crate::slots::PyBuiltinDescriptor;
use crate::vm::VirtualMachine;

pub type PyGetter = dyn Fn(PyObjectRef, &VirtualMachine) -> PyResult;
pub type PySetter = dyn Fn(PyObjectRef, PyObjectRef, &VirtualMachine) -> PyResult<()>;

#[pyclass]
pub struct PyGetSet {
    // name: String,
    getter: Box<PyGetter>,
    setter: Box<PySetter>,
    // doc: Option<String>,
}

impl std::fmt::Debug for PyGetSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "PyGetSet {{ getter: {:p}, setter: {:p} }}",
            self.getter, self.setter
        )
    }
}

impl PyValue for PyGetSet {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.getset_type()
    }
}

pub type PyGetSetRef = PyRef<PyGetSet>;

impl PyBuiltinDescriptor for PyGetSet {
    fn get(
        zelf: PyRef<Self>,
        obj: PyObjectRef,
        _cls: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        (zelf.getter)(obj, vm)
    }
}

impl PyGetSet {
    pub fn new(getter: &'static PyGetter, setter: &'static PySetter) -> Self {
        Self {
            getter: Box::new(getter),
            setter: Box::new(setter),
        }
    }
}

#[pyimpl(with(PyBuiltinDescriptor))]
impl PyGetSet {
    // Descriptor methods

    #[pymethod(magic)]
    fn set(&self, obj: PyObjectRef, value: PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
        (self.setter)(obj, value, vm)
    }
}

pub(crate) fn init(context: &PyContext) {
    PyGetSet::extend_class(context, &context.types.getset_type);
}
