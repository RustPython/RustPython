use super::{PyInt, PyStrRef, PyType, PyTypeRef};
use crate::common::format::FormatSpec;
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult,
    TryFromBorrowedObject, VirtualMachine,
    builtins::PyBaseExceptionRef,
    class::PyClassImpl,
    convert::{IntoPyException, ToPyObject, ToPyResult},
    function::{FuncArgs, OptionalArg},
    protocol::PyNumberMethods,
    types::{AsNumber, Constructor, Representable},
};
use malachite_bigint::Sign;
use num_traits::Zero;
use std::fmt::{Debug, Formatter};

impl ToPyObject for bool {
    fn to_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_bool(self).into()
    }
}

impl<'a> TryFromBorrowedObject<'a> for bool {
    fn try_from_borrowed_object(vm: &VirtualMachine, obj: &'a PyObject) -> PyResult<Self> {
        if obj.fast_isinstance(vm.ctx.types.int_type) {
            Ok(get_value(obj))
        } else {
            Err(vm.new_type_error(format!("Expected type bool, not {}", obj.class().name())))
        }
    }
}

impl PyObjectRef {
    /// Convert Python bool into Rust bool.
    pub fn try_to_bool(self, vm: &VirtualMachine) -> PyResult<bool> {
        if self.is(&vm.ctx.true_value) {
            return Ok(true);
        }
        if self.is(&vm.ctx.false_value) {
            return Ok(false);
        }
        let rs_bool = if let Some(nb_bool) = self.class().slots.as_number.boolean.load() {
            nb_bool(self.as_object().to_number(), vm)?
        } else {
            // TODO: Fully implement AsNumber and remove this block
            match vm.get_method(self.clone(), identifier!(vm, __bool__)) {
                Some(method_or_err) => {
                    // If descriptor returns Error, propagate it further
                    let method = method_or_err?;
                    let bool_obj = method.call((), vm)?;
                    if !bool_obj.fast_isinstance(vm.ctx.types.bool_type) {
                        return Err(vm.new_type_error(format!(
                            "__bool__ should return bool, returned type {}",
                            bool_obj.class().name()
                        )));
                    }

                    get_value(&bool_obj)
                }
                None => match vm.get_method(self, identifier!(vm, __len__)) {
                    Some(method_or_err) => {
                        let method = method_or_err?;
                        let bool_obj = method.call((), vm)?;
                        let int_obj = bool_obj.downcast_ref::<PyInt>().ok_or_else(|| {
                            vm.new_type_error(format!(
                                "'{}' object cannot be interpreted as an integer",
                                bool_obj.class().name()
                            ))
                        })?;

                        let len_val = int_obj.as_bigint();
                        if len_val.sign() == Sign::Minus {
                            return Err(vm.new_value_error("__len__() should return >= 0"));
                        }
                        !len_val.is_zero()
                    }
                    None => true,
                },
            }
        };
        Ok(rs_bool)
    }
}

#[pyclass(name = "bool", module = false, base = PyInt)]
#[repr(transparent)]
pub struct PyBool(pub PyInt);

impl PyPayload for PyBool {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.bool_type
    }

    /// PyBool reuses PyInt's TypeId
    #[inline]
    fn payload_type_id() -> std::any::TypeId {
        std::any::TypeId::of::<PyInt>()
    }

    fn try_downcast_from(obj: &PyObject, vm: &VirtualMachine) -> PyResult<()> {
        if obj.class().is(vm.ctx.types.bool_type) {
            return Ok(());
        }

        #[cold]
        fn raise_downcast_type_error(
            vm: &VirtualMachine,
            class: &Py<PyType>,
            obj: &PyObject,
        ) -> PyBaseExceptionRef {
            vm.new_downcast_type_error(class, obj)
        }
        Err(raise_downcast_type_error(vm, Self::class(&vm.ctx), obj))
    }
}

impl Debug for PyBool {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let value = !self.0.as_bigint().is_zero();
        write!(f, "PyBool({})", value)
    }
}

impl Constructor for PyBool {
    type Args = OptionalArg<PyObjectRef>;

    fn slot_new(zelf: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        let x: Self::Args = args.bind(vm)?;
        if !zelf.fast_isinstance(vm.ctx.types.type_type) {
            let actual_class = zelf.class();
            let actual_type = &actual_class.name();
            return Err(vm.new_type_error(format!(
                "requires a 'type' object but received a '{actual_type}'"
            )));
        }
        let val = x.map_or(Ok(false), |val| val.try_to_bool(vm))?;
        Ok(vm.ctx.new_bool(val).into())
    }

    fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
        unimplemented!("use slot_new")
    }
}

#[pyclass(with(Constructor, AsNumber, Representable), flags(_MATCH_SELF))]
impl PyBool {
    #[pymethod]
    fn __format__(obj: PyObjectRef, spec: PyStrRef, vm: &VirtualMachine) -> PyResult<String> {
        let new_bool = obj.try_to_bool(vm)?;
        FormatSpec::parse(spec.as_str())
            .and_then(|format_spec| format_spec.format_bool(new_bool))
            .map_err(|err| err.into_pyexception(vm))
    }

    #[pymethod(name = "__ror__")]
    #[pymethod]
    fn __or__(lhs: PyObjectRef, rhs: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if lhs.fast_isinstance(vm.ctx.types.bool_type)
            && rhs.fast_isinstance(vm.ctx.types.bool_type)
        {
            let lhs = get_value(&lhs);
            let rhs = get_value(&rhs);
            (lhs || rhs).to_pyobject(vm)
        } else if let Some(lhs) = lhs.downcast_ref::<PyInt>() {
            lhs.__or__(rhs).to_pyobject(vm)
        } else {
            vm.ctx.not_implemented()
        }
    }

    #[pymethod(name = "__rand__")]
    #[pymethod]
    fn __and__(lhs: PyObjectRef, rhs: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if lhs.fast_isinstance(vm.ctx.types.bool_type)
            && rhs.fast_isinstance(vm.ctx.types.bool_type)
        {
            let lhs = get_value(&lhs);
            let rhs = get_value(&rhs);
            (lhs && rhs).to_pyobject(vm)
        } else if let Some(lhs) = lhs.downcast_ref::<PyInt>() {
            lhs.__and__(rhs).to_pyobject(vm)
        } else {
            vm.ctx.not_implemented()
        }
    }

    #[pymethod(name = "__rxor__")]
    #[pymethod]
    fn __xor__(lhs: PyObjectRef, rhs: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if lhs.fast_isinstance(vm.ctx.types.bool_type)
            && rhs.fast_isinstance(vm.ctx.types.bool_type)
        {
            let lhs = get_value(&lhs);
            let rhs = get_value(&rhs);
            (lhs ^ rhs).to_pyobject(vm)
        } else if let Some(lhs) = lhs.downcast_ref::<PyInt>() {
            lhs.__xor__(rhs).to_pyobject(vm)
        } else {
            vm.ctx.not_implemented()
        }
    }
}

impl AsNumber for PyBool {
    fn as_number() -> &'static PyNumberMethods {
        static AS_NUMBER: PyNumberMethods = PyNumberMethods {
            and: Some(|a, b, vm| PyBool::__and__(a.to_owned(), b.to_owned(), vm).to_pyresult(vm)),
            xor: Some(|a, b, vm| PyBool::__xor__(a.to_owned(), b.to_owned(), vm).to_pyresult(vm)),
            or: Some(|a, b, vm| PyBool::__or__(a.to_owned(), b.to_owned(), vm).to_pyresult(vm)),
            ..PyInt::AS_NUMBER
        };
        &AS_NUMBER
    }
}

impl Representable for PyBool {
    #[inline]
    fn slot_repr(zelf: &PyObject, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        let name = if get_value(zelf.as_object()) {
            vm.ctx.names.True
        } else {
            vm.ctx.names.False
        };
        Ok(name.to_owned())
    }

    #[cold]
    fn repr_str(_zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
        unreachable!("use slot_repr instead")
    }
}

pub(crate) fn init(context: &Context) {
    PyBool::extend_class(context, context.types.bool_type);
}

// pub fn not(vm: &VirtualMachine, obj: &PyObject) -> PyResult<bool> {
//     if obj.fast_isinstance(vm.ctx.types.bool_type) {
//         let value = get_value(obj);
//         Ok(!value)
//     } else {
//         Err(vm.new_type_error(format!("Can only invert a bool, on {:?}", obj)))
//     }
// }

// Retrieve inner int value:
pub(crate) fn get_value(obj: &PyObject) -> bool {
    !obj.downcast_ref::<PyBool>()
        .unwrap()
        .0
        .as_bigint()
        .is_zero()
}

impl PyRef<PyBool> {
    #[inline]
    pub fn into_base(self) -> PyRef<PyInt> {
        // SAFETY: PyBool's payload is PyInt
        unsafe { std::mem::transmute(self) }
    }

    #[inline]
    pub fn as_base(&self) -> &PyRef<PyInt> {
        // SAFETY: PyBool's payload is PyInt
        unsafe { std::mem::transmute(self) }
    }
}
