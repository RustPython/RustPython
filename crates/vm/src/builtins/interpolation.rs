use super::{
    PyStr, PyStrRef, PyTupleRef, PyType, PyTypeRef, genericalias::PyGenericAlias,
    tuple::IntoPyTuple,
};
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    class::PyClassImpl,
    convert::ToPyObject,
    function::{ArgSpec, FuncArgs},
    types::Constructor,
};
use rustpython_common::wtf8::{Wtf8Buf, wtf8_concat};

#[pyclass(module = "string.templatelib", name = "Interpolation")]
#[derive(Debug, Clone)]
pub struct PyInterpolation {
    #[pymember(type = "object_ex")]
    pub value: PyObjectRef,
    #[pymember(type = "object_ex")]
    pub expression: PyObjectRef,
    #[pymember(type = "object_ex")]
    pub conversion: PyObjectRef, // None or 's', 'r', 'a'
    #[pymember(type = "object_ex")]
    pub format_spec: PyStrRef,
}

impl PyPayload for PyInterpolation {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.interpolation_type
    }
}

impl PyInterpolation {
    const INVALID_CONVERSION: &'static str =
        "Interpolation() argument 'conversion' must be one of 's', 'a' or 'r'";

    fn valid_conversion(conversion: &PyObject, vm: &VirtualMachine) -> bool {
        vm.is_none(conversion)
            || conversion
                .downcast_ref::<PyStr>()
                .is_some_and(|s| matches!(s.to_str(), Some("s" | "r" | "a")))
    }

    fn string_argument(
        value: Option<PyObjectRef>,
        name: &str,
        vm: &VirtualMachine,
    ) -> PyResult<PyStrRef> {
        value.map_or_else(
            || Ok(vm.ctx.empty_str.to_owned()),
            |value| {
                value.downcast::<PyStr>().map_err(|value| {
                    let type_name = value.class().slot_name();
                    // _PyArg_BadArgument limits tp_name to 50 UTF-8 bytes,
                    // omitting a partial final code point.
                    let type_name = if vm.is_none(&value) {
                        "None"
                    } else {
                        &type_name[..type_name.floor_char_boundary(50)]
                    };
                    vm.new_type_error(format!(
                        "Interpolation() argument '{name}' must be str, not {type_name}"
                    ))
                })
            },
        )
    }

    pub fn new(
        value: PyObjectRef,
        expression: PyObjectRef,
        conversion: PyObjectRef,
        format_spec: PyStrRef,
        vm: &VirtualMachine,
    ) -> PyResult<Self> {
        // Validate conversion like _PyInterpolation_Build does
        if !Self::valid_conversion(&conversion, vm) {
            return Err(vm.new_system_error(Self::INVALID_CONVERSION));
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
    type Args = FuncArgs;

    fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
        // Clinic binds every argument before converting them in declaration order.
        let mut bound = ArgSpec {
            fname: "Interpolation",
            keywords: &["value", "expression", "conversion", "format_spec"],
            required: 1,
            max_positional: 4,
        }
        .parse(&args, vm)?;
        let value = bound[0].take().unwrap();
        let expression = Self::string_argument(bound[1].take(), "expression", vm)?;
        let conversion = bound[2].take().unwrap_or_else(|| vm.ctx.none());
        if !vm.is_none(&conversion) && conversion.downcast_ref::<PyStr>().is_none() {
            return Err(vm.new_type_error(wtf8_concat!(
                "Interpolation() argument 'conversion' must be str, not ",
                conversion.class().fully_qualified_name(vm)?,
            )));
        }
        if !Self::valid_conversion(&conversion, vm) {
            return Err(vm.new_value_error(Self::INVALID_CONVERSION));
        }
        let format_spec = Self::string_argument(bound[3].take(), "format_spec", vm)?;

        Ok(Self {
            value,
            expression: expression.into(),
            conversion,
            format_spec,
        })
    }
}

#[pyclass(with(Constructor))]
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

    #[pyclassmethod]
    fn __class_getitem__(
        cls: PyTypeRef,
        args: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyGenericAlias> {
        PyGenericAlias::from_args(cls, args, vm)
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

    #[inline]
    #[pymethod]
    fn __repr__(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<Wtf8Buf> {
        let mut result = Wtf8Buf::from("Interpolation(");
        // Release each repr result before invoking the next field's callback.
        result.push_wtf8(zelf.value.repr(vm)?.as_wtf8());
        result.push_str(", ");
        result.push_wtf8(zelf.expression.repr(vm)?.as_wtf8());
        result.push_str(", ");
        result.push_wtf8(zelf.conversion.repr(vm)?.as_wtf8());
        result.push_str(", ");
        result.push_wtf8(zelf.format_spec.as_object().repr(vm)?.as_wtf8());
        result.push_char(')');
        Ok(result)
    }
}

pub(crate) fn init(context: &'static Context) {
    PyInterpolation::extend_class(context, context.types.interpolation_type);
}
