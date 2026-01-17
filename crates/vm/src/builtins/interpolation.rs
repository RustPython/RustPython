use super::{PyStr, PyStrRef, PyTupleRef, PyType, tuple::IntoPyTuple};
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    class::PyClassImpl,
    common::hash::PyHash,
    convert::ToPyObject,
    function::{OptionalArg, PyComparisonValue},
    types::{Comparable, Constructor, Hashable, PyComparisonOp, Representable},
};

/// Interpolation object for t-strings (PEP 750).
///
/// Represents an interpolated expression within a template string.
#[pyclass(module = "string.templatelib", name = "Interpolation")]
#[derive(Debug, Clone)]
pub struct PyInterpolation {
    pub value: PyObjectRef,
    pub expression: PyStrRef,
    pub conversion: PyObjectRef, // None or 's', 'r', 'a'
    pub format_spec: PyStrRef,
}

impl PyPayload for PyInterpolation {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.interpolation_type
    }
}

impl PyInterpolation {
    pub fn new(
        value: PyObjectRef,
        expression: PyStrRef,
        conversion: PyObjectRef,
        format_spec: PyStrRef,
        vm: &VirtualMachine,
    ) -> PyResult<Self> {
        // Validate conversion like _PyInterpolation_Build does
        let is_valid = vm.is_none(&conversion)
            || conversion
                .downcast_ref::<PyStr>()
                .is_some_and(|s| matches!(s.as_str(), "s" | "r" | "a"));
        if !is_valid {
            return Err(vm.new_exception_msg(
                vm.ctx.exceptions.system_error.to_owned(),
                "Interpolation() argument 'conversion' must be one of 's', 'a' or 'r'".to_owned(),
            ));
        }
        Ok(Self {
            value,
            expression,
            conversion,
            format_spec,
        })
    }
}

impl Constructor for PyInterpolation {
    type Args = InterpolationArgs;

    fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
        let conversion = match args.conversion {
            OptionalArg::Present(c) => {
                if vm.is_none(&c) {
                    vm.ctx.none()
                } else {
                    let s = c.downcast::<PyStr>().map_err(|_| {
                        vm.new_type_error(
                            "Interpolation() argument 'conversion' must be str or None",
                        )
                    })?;
                    let s_str = s.as_str();
                    if s_str.len() != 1 || !matches!(s_str.chars().next(), Some('s' | 'r' | 'a')) {
                        return Err(vm.new_value_error(
                            "Interpolation() argument 'conversion' must be one of 's', 'a' or 'r'",
                        ));
                    }
                    s.into()
                }
            }
            OptionalArg::Missing => vm.ctx.none(),
        };

        let expression = args
            .expression
            .unwrap_or_else(|| vm.ctx.empty_str.to_owned());
        let format_spec = args
            .format_spec
            .unwrap_or_else(|| vm.ctx.empty_str.to_owned());

        Ok(PyInterpolation {
            value: args.value,
            expression,
            conversion,
            format_spec,
        })
    }
}

#[derive(FromArgs)]
pub struct InterpolationArgs {
    #[pyarg(positional)]
    value: PyObjectRef,
    #[pyarg(any, optional)]
    expression: OptionalArg<PyStrRef>,
    #[pyarg(any, optional)]
    conversion: OptionalArg<PyObjectRef>,
    #[pyarg(any, optional)]
    format_spec: OptionalArg<PyStrRef>,
}

#[pyclass(with(Constructor, Comparable, Hashable, Representable))]
impl PyInterpolation {
    #[pyattr]
    fn __match_args__(ctx: &Context) -> PyTupleRef {
        ctx.new_tuple(vec![
            ctx.intern_str("value").to_owned().into(),
            ctx.intern_str("expression").to_owned().into(),
            ctx.intern_str("conversion").to_owned().into(),
            ctx.intern_str("format_spec").to_owned().into(),
        ])
    }

    #[pygetset]
    fn value(&self) -> PyObjectRef {
        self.value.clone()
    }

    #[pygetset]
    fn expression(&self) -> PyStrRef {
        self.expression.clone()
    }

    #[pygetset]
    fn conversion(&self) -> PyObjectRef {
        self.conversion.clone()
    }

    #[pygetset]
    fn format_spec(&self) -> PyStrRef {
        self.format_spec.clone()
    }

    #[pymethod]
    fn __reduce__(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyTupleRef {
        let cls = zelf.class().to_owned();
        let args = (
            zelf.value.clone(),
            zelf.expression.clone(),
            zelf.conversion.clone(),
            zelf.format_spec.clone(),
        );
        (cls, args.to_pyobject(vm)).into_pytuple(vm)
    }
}

impl Comparable for PyInterpolation {
    fn cmp(
        zelf: &Py<Self>,
        other: &PyObject,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        op.eq_only(|| {
            let other = class_or_notimplemented!(Self, other);

            let eq = vm.bool_eq(&zelf.value, &other.value)?
                && vm.bool_eq(zelf.expression.as_object(), other.expression.as_object())?
                && vm.bool_eq(&zelf.conversion, &other.conversion)?
                && vm.bool_eq(zelf.format_spec.as_object(), other.format_spec.as_object())?;

            Ok(eq.into())
        })
    }
}

impl Hashable for PyInterpolation {
    fn hash(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyHash> {
        // Hash based on (value, expression, conversion, format_spec)
        let value_hash = zelf.value.hash(vm)?;
        let expr_hash = zelf.expression.as_object().hash(vm)?;
        let conv_hash = zelf.conversion.hash(vm)?;
        let spec_hash = zelf.format_spec.as_object().hash(vm)?;

        // Combine hashes
        Ok(value_hash
            .wrapping_add(expr_hash.wrapping_mul(3))
            .wrapping_add(conv_hash.wrapping_mul(5))
            .wrapping_add(spec_hash.wrapping_mul(7)))
    }
}

impl Representable for PyInterpolation {
    #[inline]
    fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
        let value_repr = zelf.value.repr(vm)?;
        let expr_repr = zelf.expression.repr(vm)?;

        let conv_str = if vm.is_none(&zelf.conversion) {
            "None".to_owned()
        } else {
            zelf.conversion.repr(vm)?.as_str().to_owned()
        };

        let spec_repr = zelf.format_spec.repr(vm)?;

        Ok(format!(
            "Interpolation({}, {}, {}, {})",
            value_repr.as_str(),
            expr_repr.as_str(),
            conv_str,
            spec_repr.as_str()
        ))
    }
}

pub fn init(context: &Context) {
    PyInterpolation::extend_class(context, context.types.interpolation_type);
}
