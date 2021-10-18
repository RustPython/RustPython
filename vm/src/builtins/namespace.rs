use super::PyTypeRef;
use crate::{
    builtins::PyDict,
    function::FuncArgs,
    types::{Comparable, Constructor, PyComparisonOp},
    PyClassImpl, PyComparisonValue, PyContext, PyObjectRef, PyRef, PyResult, PyValue,
    VirtualMachine,
};

/// A simple attribute-based namespace.
///
/// SimpleNamespace(**kwargs)
#[pyclass(module = false, name = "SimpleNamespace")]
#[derive(Debug)]
pub struct PyNamespace;

impl PyValue for PyNamespace {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.namespace_type
    }
}

impl Constructor for PyNamespace {
    type Args = FuncArgs;

    fn py_new(cls: PyTypeRef, _args: Self::Args, vm: &VirtualMachine) -> PyResult {
        PyNamespace {}.into_pyresult_with_type(vm, cls)
    }
}

impl PyNamespace {
    pub fn new_ref(ctx: &PyContext) -> PyRef<Self> {
        PyRef::new_ref(Self, ctx.types.namespace_type.clone(), Some(ctx.new_dict()))
    }
}

#[pyimpl(flags(BASETYPE, HAS_DICT), with(Constructor, Comparable))]
impl PyNamespace {
    #[pymethod(magic)]
    fn init(zelf: PyRef<Self>, args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
        if !args.args.is_empty() {
            return Err(vm.new_type_error("no positional arguments expected".to_owned()));
        }
        for (name, value) in args.kwargs.into_iter() {
            zelf.as_object().set_attr(name, value, vm)?;
        }
        Ok(())
    }
}

impl Comparable for PyNamespace {
    fn cmp(
        zelf: &PyRef<Self>,
        other: &PyObjectRef,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        let other = class_or_notimplemented!(Self, other);
        match (zelf.as_object().dict(), other.as_object().dict()) {
            (Some(d1), Some(d2)) => PyDict::cmp(&d1, d2.as_object(), op, vm),
            _ => Ok(PyComparisonValue::NotImplemented),
        }
    }
}

pub fn init(context: &PyContext) {
    PyNamespace::extend_class(context, &context.types.namespace_type);
}
