use super::{PyType, PyTypeRef};
use crate::{
    builtins::PyDict,
    class::PyClassImpl,
    function::{FuncArgs, PyComparisonValue},
    recursion::ReprGuard,
    types::{Comparable, Constructor, Initializer, PyComparisonOp},
    AsObject, Context, Py, PyObject, PyPayload, PyRef, PyResult, VirtualMachine,
};

/// A simple attribute-based namespace.
///
/// SimpleNamespace(**kwargs)
#[pyclass(module = false, name = "SimpleNamespace")]
#[derive(Debug)]
pub struct PyNamespace {}

impl PyPayload for PyNamespace {
    fn class(vm: &VirtualMachine) -> &'static Py<PyType> {
        vm.ctx.types.namespace_type
    }
}

impl Constructor for PyNamespace {
    type Args = FuncArgs;

    fn py_new(cls: PyTypeRef, _args: Self::Args, vm: &VirtualMachine) -> PyResult {
        PyNamespace {}.into_ref_with_type(vm, cls).map(Into::into)
    }
}

impl PyNamespace {
    pub fn new_ref(ctx: &Context) -> PyRef<Self> {
        PyRef::new_ref(
            Self {},
            ctx.types.namespace_type.to_owned(),
            Some(ctx.new_dict()),
        )
    }
}

#[pyclass(flags(BASETYPE, HAS_DICT), with(Constructor, Initializer, Comparable))]
impl PyNamespace {
    #[pymethod(magic)]
    fn repr(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<String> {
        let o = zelf.as_object();
        let name = if o.class().is(vm.ctx.types.namespace_type) {
            "namespace".to_owned()
        } else {
            o.class().slot_name()
        };

        let repr = if let Some(_guard) = ReprGuard::enter(vm, zelf.as_object()) {
            let dict = zelf.as_object().dict().unwrap();
            let mut parts = Vec::with_capacity(dict.len());
            for (key, value) in dict {
                let k = &key.repr(vm)?;
                let key_str = k.as_str();
                let value_repr = value.repr(vm)?;
                parts.push(format!("{}={}", &key_str[1..key_str.len() - 1], value_repr));
            }
            format!("{}({})", name, parts.join(", "))
        } else {
            format!("{name}(...)")
        };
        Ok(repr)
    }
}

impl Initializer for PyNamespace {
    type Args = FuncArgs;

    fn init(zelf: PyRef<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
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
        zelf: &crate::Py<Self>,
        other: &PyObject,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        let other = class_or_notimplemented!(Self, other);
        let (d1, d2) = (
            zelf.as_object().dict().unwrap(),
            other.as_object().dict().unwrap(),
        );
        PyDict::cmp(&d1, d2.as_object(), op, vm)
    }
}

pub fn init(context: &Context) {
    PyNamespace::extend_class(context, context.types.namespace_type);
}
