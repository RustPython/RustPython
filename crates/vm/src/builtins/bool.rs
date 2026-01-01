use super::{PyInt, PyStrRef, PyType, PyTypeRef};
use crate::common::format::FormatSpec;
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyResult, TryFromBorrowedObject, VirtualMachine,
    class::PyClassImpl,
    convert::{IntoPyException, ToPyObject, ToPyResult},
    function::{FuncArgs, OptionalArg},
    protocol::PyNumberMethods,
    types::{AsNumber, Constructor, Representable},
};
use core::fmt::{Debug, Formatter};
use num_traits::Zero;

impl ToPyObject for bool {
    fn to_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_bool(self).into()
    }
}

impl<'a> TryFromBorrowedObject<'a> for bool {
    fn try_from_borrowed_object(vm: &VirtualMachine, obj: &'a PyObject) -> PyResult<Self> {
        // Python takes integers as a legit bool value
        match obj.downcast_ref::<PyInt>() {
            Some(int_obj) => {
                let int_val = int_obj.as_bigint();
                Ok(!int_val.is_zero())
            }
            None => {
                Err(vm.new_type_error(format!("Expected type bool, not {}", obj.class().name())))
            }
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

        let slots = &self.class().slots;

        // 1. Try nb_bool slot first
        if let Some(nb_bool) = slots.as_number.boolean.load() {
            return nb_bool(self.as_object().number(), vm);
        }

        // 2. Try mp_length slot (mapping protocol)
        if let Some(mp_length) = slots.as_mapping.length.load() {
            let len = mp_length(self.as_object().mapping_unchecked(), vm)?;
            return Ok(len != 0);
        }

        // 3. Try sq_length slot (sequence protocol)
        if let Some(sq_length) = slots.as_sequence.length.load() {
            let len = sq_length(self.as_object().sequence_unchecked(), vm)?;
            return Ok(len != 0);
        }

        // 4. Default: objects without __bool__ or __len__ are truthy
        Ok(true)
    }
}

#[pyclass(name = "bool", module = false, base = PyInt, ctx = "bool_type")]
#[repr(transparent)]
pub struct PyBool(pub PyInt);

impl Debug for PyBool {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
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
}

impl PyBool {
    pub(crate) fn __or__(lhs: PyObjectRef, rhs: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
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

    pub(crate) fn __and__(lhs: PyObjectRef, rhs: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
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

    pub(crate) fn __xor__(lhs: PyObjectRef, rhs: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
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
