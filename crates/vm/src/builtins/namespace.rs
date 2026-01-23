use super::{PyTupleRef, PyType, tuple::IntoPyTuple};
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    builtins::PyDict,
    class::PyClassImpl,
    function::{FuncArgs, PyComparisonValue},
    recursion::ReprGuard,
    types::{
        Comparable, Constructor, DefaultConstructor, Initializer, PyComparisonOp, Representable,
    },
};

/// A simple attribute-based namespace.
///
/// SimpleNamespace(**kwargs)
#[pyclass(module = "types", name = "SimpleNamespace")]
#[derive(Copy, Clone, Debug, Default)]
pub struct PyNamespace;

impl PyPayload for PyNamespace {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.namespace_type
    }
}

impl DefaultConstructor for PyNamespace {}

#[pyclass(
    flags(BASETYPE, HAS_DICT),
    with(Constructor, Initializer, Comparable, Representable)
)]
impl PyNamespace {
    #[pymethod]
    fn __reduce__(zelf: PyObjectRef, vm: &VirtualMachine) -> PyTupleRef {
        let dict = zelf.as_object().dict().unwrap();
        let obj = zelf.as_object().to_owned();
        let result: (PyObjectRef, PyObjectRef, PyObjectRef) = (
            obj.class().to_owned().into(),
            vm.new_tuple(()).into(),
            dict.into(),
        );
        result.into_pytuple(vm)
    }

    #[pymethod]
    fn __replace__(zelf: PyObjectRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        if !args.args.is_empty() {
            return Err(vm.new_type_error("__replace__() takes no positional arguments"));
        }

        // Create a new instance of the same type
        let cls: PyObjectRef = zelf.class().to_owned().into();
        let result = cls.call((), vm)?;

        // Copy the current namespace dict to the new instance
        let src_dict = zelf.dict().unwrap();
        let dst_dict = result.dict().unwrap();
        for (key, value) in src_dict {
            dst_dict.set_item(&*key, value, vm)?;
        }

        // Update with the provided kwargs
        for (name, value) in args.kwargs {
            let name = vm.ctx.new_str(name);
            result.set_attr(&name, value, vm)?;
        }

        Ok(result)
    }
}

impl Initializer for PyNamespace {
    type Args = FuncArgs;

    fn init(zelf: PyRef<Self>, args: Self::Args, vm: &VirtualMachine) -> PyResult<()> {
        // SimpleNamespace accepts 0 or 1 positional argument (a mapping)
        if args.args.len() > 1 {
            return Err(vm.new_type_error(format!(
                "{} expected at most 1 positional argument, got {}",
                zelf.class().name(),
                args.args.len()
            )));
        }

        // If there's a positional argument, treat it as a mapping
        if let Some(mapping) = args.args.first() {
            // Convert to dict if not already
            let dict: PyRef<PyDict> = if let Some(d) = mapping.downcast_ref::<PyDict>() {
                d.to_owned()
            } else {
                // Call dict() on the mapping
                let dict_type: PyObjectRef = vm.ctx.types.dict_type.to_owned().into();
                dict_type
                    .call((mapping.clone(),), vm)?
                    .downcast()
                    .map_err(|_| vm.new_type_error("dict() did not return a dict"))?
            };

            // Validate keys are strings and set attributes
            for (key, value) in dict.into_iter() {
                let key_str = key
                    .downcast_ref::<crate::builtins::PyStr>()
                    .ok_or_else(|| {
                        vm.new_type_error(format!(
                            "keywords must be strings, not '{}'",
                            key.class().name()
                        ))
                    })?;
                zelf.as_object().set_attr(key_str, value, vm)?;
            }
        }

        // Apply keyword arguments (these override positional mapping values)
        for (name, value) in args.kwargs {
            let name = vm.ctx.new_str(name);
            zelf.as_object().set_attr(&name, value, vm)?;
        }
        Ok(())
    }
}

impl Comparable for PyNamespace {
    fn cmp(
        zelf: &Py<Self>,
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

impl Representable for PyNamespace {
    #[inline]
    fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
        let o = zelf.as_object();
        let name = if o.class().is(vm.ctx.types.namespace_type) {
            "namespace".to_owned()
        } else {
            o.class().slot_name().to_owned()
        };

        let repr = if let Some(_guard) = ReprGuard::enter(vm, zelf.as_object()) {
            let dict = zelf.as_object().dict().unwrap();
            let mut parts = Vec::with_capacity(dict.__len__());
            for (key, value) in dict {
                let k = key.repr(vm)?;
                let key_str = k.as_wtf8();
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

pub fn init(context: &Context) {
    PyNamespace::extend_class(context, context.types.namespace_type);
}
