use super::{PyStrRef, PyType, PyTypeRef};
use crate::{
    Context, Py, PyObjectRef, PyPayload, PyResult, VirtualMachine,
    class::PyClassImpl,
    convert::ToPyObject,
    function::FuncArgs,
    protocol::PyNumberMethods,
    types::{AsNumber, Constructor, Representable},
};

#[pyclass(module = false, name = "NoneType")]
#[derive(Debug)]
pub struct PyNone;

impl PyPayload for PyNone {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.none_type
    }
}

// This allows a built-in function to not return a value, mapping to
// Python's behavior of returning `None` in this situation.
impl ToPyObject for () {
    fn to_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.none()
    }
}

impl<T: ToPyObject> ToPyObject for Option<T> {
    fn to_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        match self {
            Some(x) => x.to_pyobject(vm),
            None => vm.ctx.none(),
        }
    }
}

impl Constructor for PyNone {
    type Args = ();

    fn slot_new(_cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        let _: () = args.bind(vm)?;
        Ok(vm.ctx.none.clone().into())
    }

    fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
        unreachable!("None is a singleton")
    }
}

#[pyclass(with(Constructor, AsNumber, Representable))]
impl PyNone {}

impl Representable for PyNone {
    #[inline]
    fn repr(_zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        Ok(vm.ctx.names.None.to_owned())
    }

    #[cold]
    fn repr_str(_zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
        unreachable!("use repr instead")
    }
}

impl AsNumber for PyNone {
    fn as_number() -> &'static PyNumberMethods {
        static AS_NUMBER: PyNumberMethods = PyNumberMethods {
            boolean: Some(|_number, _vm| Ok(false)),
            ..PyNumberMethods::NOT_IMPLEMENTED
        };
        &AS_NUMBER
    }
}

#[pyclass(module = false, name = "NotImplementedType")]
#[derive(Debug)]
pub struct PyNotImplemented;

impl PyPayload for PyNotImplemented {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.not_implemented_type
    }
}

impl Constructor for PyNotImplemented {
    type Args = ();

    fn slot_new(_cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        let _: () = args.bind(vm)?;
        Ok(vm.ctx.not_implemented.clone().into())
    }

    fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
        unreachable!("PyNotImplemented is a singleton")
    }
}

#[pyclass(with(Constructor, AsNumber, Representable))]
impl PyNotImplemented {
    #[pymethod]
    fn __reduce__(&self, vm: &VirtualMachine) -> PyStrRef {
        vm.ctx.names.NotImplemented.to_owned()
    }
}

impl AsNumber for PyNotImplemented {
    fn as_number() -> &'static PyNumberMethods {
        static AS_NUMBER: PyNumberMethods = PyNumberMethods {
            boolean: Some(|_number, vm| {
                Err(vm.new_type_error(
                    "NotImplemented should not be used in a boolean context".to_owned(),
                ))
            }),
            ..PyNumberMethods::NOT_IMPLEMENTED
        };
        &AS_NUMBER
    }
}

impl Representable for PyNotImplemented {
    #[inline]
    fn repr(_zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        Ok(vm.ctx.names.NotImplemented.to_owned())
    }

    #[cold]
    fn repr_str(_zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
        unreachable!("use repr instead")
    }
}

pub fn init(context: &Context) {
    PyNone::extend_class(context, context.types.none_type);
    PyNotImplemented::extend_class(context, context.types.not_implemented_type);
}
