use super::objbytes;
use super::objint;
use super::objstr;
use super::objtype;
use crate::function::OptionalArg;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{
    IdProtocol, IntoPyObject, PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue,
    TypeProtocol,
};
use crate::vm::VirtualMachine;
use num_bigint::{BigInt, ToBigInt};
use num_rational::Ratio;
use num_traits::{ToPrimitive, Zero};

/// Convert a string or number to a floating point number, if possible.
#[pyclass(name = "float")]
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct PyFloat {
    value: f64,
}

impl PyFloat {
    pub fn to_f64(&self) -> f64 {
        self.value
    }
}

impl PyValue for PyFloat {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.float_type()
    }
}

impl IntoPyObject for f64 {
    fn into_pyobject(self, vm: &VirtualMachine) -> PyResult {
        Ok(vm.ctx.new_float(self))
    }
}

impl From<f64> for PyFloat {
    fn from(value: f64) -> Self {
        PyFloat { value }
    }
}

pub fn try_float(value: &PyObjectRef, vm: &VirtualMachine) -> PyResult<Option<f64>> {
    Ok(if objtype::isinstance(&value, &vm.ctx.float_type()) {
        Some(get_value(&value))
    } else if objtype::isinstance(&value, &vm.ctx.int_type()) {
        Some(objint::get_float_value(&value, vm)?)
    } else {
        None
    })
}

fn inner_div(v1: f64, v2: f64, vm: &VirtualMachine) -> PyResult<f64> {
    if v2 != 0.0 {
        Ok(v1 / v2)
    } else {
        Err(vm.new_zero_division_error("float division by zero".to_string()))
    }
}

fn inner_mod(v1: f64, v2: f64, vm: &VirtualMachine) -> PyResult<f64> {
    if v2 != 0.0 {
        Ok(v1 % v2)
    } else {
        Err(vm.new_zero_division_error("float mod by zero".to_string()))
    }
}

fn try_to_bigint(value: f64, vm: &VirtualMachine) -> PyResult<BigInt> {
    match value.to_bigint() {
        Some(int) => Ok(int),
        None => {
            if value.is_infinite() {
                Err(vm.new_overflow_error(
                    "OverflowError: cannot convert float NaN to integer".to_string(),
                ))
            } else if value.is_nan() {
                Err(vm
                    .new_value_error("ValueError: cannot convert float NaN to integer".to_string()))
            } else {
                // unreachable unless BigInt has a bug
                unreachable!(
                    "A finite float value failed to be converted to bigint: {}",
                    value
                )
            }
        }
    }
}

fn inner_floordiv(v1: f64, v2: f64, vm: &VirtualMachine) -> PyResult<f64> {
    if v2 != 0.0 {
        Ok((v1 / v2).floor())
    } else {
        Err(vm.new_zero_division_error("float floordiv by zero".to_string()))
    }
}

fn inner_divmod(v1: f64, v2: f64, vm: &VirtualMachine) -> PyResult<(f64, f64)> {
    if v2 != 0.0 {
        Ok(((v1 / v2).floor(), v1 % v2))
    } else {
        Err(vm.new_zero_division_error("float divmod()".to_string()))
    }
}

#[pyimpl]
impl PyFloat {
    #[pymethod(name = "__eq__")]
    fn eq(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        let value = self.value;
        let result = if objtype::isinstance(&other, &vm.ctx.float_type()) {
            let other = get_value(&other);
            value == other
        } else if objtype::isinstance(&other, &vm.ctx.int_type()) {
            let other_int = objint::get_value(&other);

            if let (Some(self_int), Some(other_float)) = (value.to_bigint(), other_int.to_f64()) {
                value == other_float && self_int == *other_int
            } else {
                false
            }
        } else {
            return vm.ctx.not_implemented();
        };
        vm.ctx.new_bool(result)
    }

    #[pymethod(name = "__lt__")]
    fn lt(&self, i2: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        let v1 = self.value;
        if objtype::isinstance(&i2, &vm.ctx.float_type()) {
            vm.ctx.new_bool(v1 < get_value(&i2))
        } else if objtype::isinstance(&i2, &vm.ctx.int_type()) {
            vm.ctx
                .new_bool(v1 < objint::get_value(&i2).to_f64().unwrap())
        } else {
            vm.ctx.not_implemented()
        }
    }

    #[pymethod(name = "__le__")]
    fn le(&self, i2: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        let v1 = self.value;
        if objtype::isinstance(&i2, &vm.ctx.float_type()) {
            vm.ctx.new_bool(v1 <= get_value(&i2))
        } else if objtype::isinstance(&i2, &vm.ctx.int_type()) {
            vm.ctx
                .new_bool(v1 <= objint::get_value(&i2).to_f64().unwrap())
        } else {
            vm.ctx.not_implemented()
        }
    }

    #[pymethod(name = "__gt__")]
    fn gt(&self, i2: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        let v1 = self.value;
        if objtype::isinstance(&i2, &vm.ctx.float_type()) {
            vm.ctx.new_bool(v1 > get_value(&i2))
        } else if objtype::isinstance(&i2, &vm.ctx.int_type()) {
            vm.ctx
                .new_bool(v1 > objint::get_value(&i2).to_f64().unwrap())
        } else {
            vm.ctx.not_implemented()
        }
    }

    #[pymethod(name = "__ge__")]
    fn ge(&self, i2: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        let v1 = self.value;
        if objtype::isinstance(&i2, &vm.ctx.float_type()) {
            vm.ctx.new_bool(v1 >= get_value(&i2))
        } else if objtype::isinstance(&i2, &vm.ctx.int_type()) {
            vm.ctx
                .new_bool(v1 >= objint::get_value(&i2).to_f64().unwrap())
        } else {
            vm.ctx.not_implemented()
        }
    }

    #[pymethod(name = "__abs__")]
    fn abs(&self, _vm: &VirtualMachine) -> f64 {
        self.value.abs()
    }

    #[pymethod(name = "__add__")]
    fn add(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        try_float(&other, vm)?.map_or_else(
            || Ok(vm.ctx.not_implemented()),
            |other| (self.value + other).into_pyobject(vm),
        )
    }

    #[pymethod(name = "__radd__")]
    fn radd(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.add(other, vm)
    }

    #[pymethod(name = "__bool__")]
    fn bool(&self, _vm: &VirtualMachine) -> bool {
        self.value != 0.0
    }

    #[pymethod(name = "__divmod__")]
    fn divmod(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        try_float(&other, vm)?.map_or_else(
            || Ok(vm.ctx.not_implemented()),
            |other| {
                let (r1, r2) = inner_divmod(self.value, other, vm)?;
                Ok(vm
                    .ctx
                    .new_tuple(vec![vm.ctx.new_float(r1), vm.ctx.new_float(r2)]))
            },
        )
    }

    #[pymethod(name = "__rdivmod__")]
    fn rdivmod(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        try_float(&other, vm)?.map_or_else(
            || Ok(vm.ctx.not_implemented()),
            |other| {
                let (r1, r2) = inner_divmod(other, self.value, vm)?;
                Ok(vm
                    .ctx
                    .new_tuple(vec![vm.ctx.new_float(r1), vm.ctx.new_float(r2)]))
            },
        )
    }

    #[pymethod(name = "__floordiv__")]
    fn floordiv(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        try_float(&other, vm)?.map_or_else(
            || Ok(vm.ctx.not_implemented()),
            |other| inner_floordiv(self.value, other, vm)?.into_pyobject(vm),
        )
    }

    #[pymethod(name = "__rfloordiv__")]
    fn rfloordiv(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        try_float(&other, vm)?.map_or_else(
            || Ok(vm.ctx.not_implemented()),
            |other| inner_floordiv(other, self.value, vm)?.into_pyobject(vm),
        )
    }

    #[pymethod(name = "__new__")]
    fn float_new(cls: PyClassRef, arg: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyFloatRef> {
        let value = if objtype::isinstance(&arg, &vm.ctx.float_type()) {
            get_value(&arg)
        } else if objtype::isinstance(&arg, &vm.ctx.int_type()) {
            objint::get_float_value(&arg, vm)?
        } else if objtype::isinstance(&arg, &vm.ctx.str_type()) {
            match lexical::try_parse(objstr::get_value(&arg)) {
                Ok(f) => f,
                Err(_) => {
                    let arg_repr = vm.to_pystr(&arg)?;
                    return Err(vm.new_value_error(format!(
                        "could not convert string to float: {}",
                        arg_repr
                    )));
                }
            }
        } else if objtype::isinstance(&arg, &vm.ctx.bytes_type()) {
            match lexical::try_parse(objbytes::get_value(&arg).as_slice()) {
                Ok(f) => f,
                Err(_) => {
                    let arg_repr = vm.to_pystr(&arg)?;
                    return Err(vm.new_value_error(format!(
                        "could not convert string to float: {}",
                        arg_repr
                    )));
                }
            }
        } else {
            return Err(vm.new_type_error(format!("can't convert {} to float", arg.class().name)));
        };
        PyFloat { value }.into_ref_with_type(vm, cls)
    }

    #[pymethod(name = "__mod__")]
    fn mod_(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        try_float(&other, vm)?.map_or_else(
            || Ok(vm.ctx.not_implemented()),
            |other| inner_mod(self.value, other, vm)?.into_pyobject(vm),
        )
    }

    #[pymethod(name = "__rmod__")]
    fn rmod(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        try_float(&other, vm)?.map_or_else(
            || Ok(vm.ctx.not_implemented()),
            |other| inner_mod(other, self.value, vm)?.into_pyobject(vm),
        )
    }

    #[pymethod(name = "__neg__")]
    fn neg(&self, _vm: &VirtualMachine) -> f64 {
        -self.value
    }

    #[pymethod(name = "__pow__")]
    fn pow(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        try_float(&other, vm)?.map_or_else(
            || Ok(vm.ctx.not_implemented()),
            |other| self.value.powf(other).into_pyobject(vm),
        )
    }

    #[pymethod(name = "__rpow__")]
    fn rpow(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        try_float(&other, vm)?.map_or_else(
            || Ok(vm.ctx.not_implemented()),
            |other| other.powf(self.value).into_pyobject(vm),
        )
    }

    #[pymethod(name = "__sub__")]
    fn sub(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        try_float(&other, vm)?.map_or_else(
            || Ok(vm.ctx.not_implemented()),
            |other| (self.value - other).into_pyobject(vm),
        )
    }

    #[pymethod(name = "__rsub__")]
    fn rsub(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        try_float(&other, vm)?.map_or_else(
            || Ok(vm.ctx.not_implemented()),
            |other| (other - self.value).into_pyobject(vm),
        )
    }

    #[pymethod(name = "__repr__")]
    fn repr(&self, vm: &VirtualMachine) -> String {
        if self.is_integer(vm) {
            format!("{:.1}", self.value)
        } else {
            self.value.to_string()
        }
    }

    #[pymethod(name = "__truediv__")]
    fn truediv(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        try_float(&other, vm)?.map_or_else(
            || Ok(vm.ctx.not_implemented()),
            |other| inner_div(self.value, other, vm)?.into_pyobject(vm),
        )
    }

    #[pymethod(name = "__rtruediv__")]
    fn rtruediv(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        try_float(&other, vm)?.map_or_else(
            || Ok(vm.ctx.not_implemented()),
            |other| inner_div(other, self.value, vm)?.into_pyobject(vm),
        )
    }

    #[pymethod(name = "__mul__")]
    fn mul(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        try_float(&other, vm)?.map_or_else(
            || Ok(vm.ctx.not_implemented()),
            |other| (self.value * other).into_pyobject(vm),
        )
    }

    #[pymethod(name = "__rmul__")]
    fn rmul(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        self.mul(other, vm)
    }

    #[pymethod(name = "__trunc__")]
    fn trunc(&self, vm: &VirtualMachine) -> PyResult<BigInt> {
        try_to_bigint(self.value, vm)
    }

    #[pymethod(name = "__round__")]
    fn round(&self, ndigits: OptionalArg<PyObjectRef>, vm: &VirtualMachine) -> PyResult {
        let ndigits = match ndigits {
            OptionalArg::Missing => None,
            OptionalArg::Present(ref value) => {
                if !vm.get_none().is(value) {
                    let ndigits = if objtype::isinstance(value, &vm.ctx.int_type()) {
                        objint::get_value(value)
                    } else {
                        return Err(vm.new_type_error(format!(
                            "TypeError: '{}' object cannot be interpreted as an integer",
                            value.class().name
                        )));
                    };
                    if ndigits.is_zero() {
                        None
                    } else {
                        Some(ndigits)
                    }
                } else {
                    None
                }
            }
        };
        if ndigits.is_none() {
            let fract = self.value.fract();
            let value = if (fract.abs() - 0.5).abs() < std::f64::EPSILON {
                if self.value.trunc() % 2.0 == 0.0 {
                    self.value - fract
                } else {
                    self.value + fract
                }
            } else {
                self.value.round()
            };
            let int = try_to_bigint(value, vm)?;
            Ok(vm.ctx.new_int(int))
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    #[pymethod(name = "__int__")]
    fn int(&self, vm: &VirtualMachine) -> PyResult<BigInt> {
        self.trunc(vm)
    }

    #[pymethod(name = "__float__")]
    fn float(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyFloatRef {
        zelf
    }

    #[pyproperty(name = "real")]
    fn real(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyFloatRef {
        zelf
    }

    #[pyproperty(name = "imag")]
    fn imag(&self, _vm: &VirtualMachine) -> f64 {
        0.0f64
    }

    #[pymethod(name = "conjugate")]
    fn conjugate(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyFloatRef {
        zelf
    }

    #[pymethod(name = "is_integer")]
    fn is_integer(&self, _vm: &VirtualMachine) -> bool {
        let v = self.value;
        (v - v.round()).abs() < std::f64::EPSILON
    }

    #[pymethod(name = "as_integer_ratio")]
    fn as_integer_ratio(&self, vm: &VirtualMachine) -> PyResult {
        let value = self.value;
        if value.is_infinite() {
            return Err(
                vm.new_overflow_error("cannot convert Infinity to integer ratio".to_string())
            );
        }
        if value.is_nan() {
            return Err(vm.new_value_error("cannot convert NaN to integer ratio".to_string()));
        }

        let ratio = Ratio::from_float(value).unwrap();
        let numer = vm.ctx.new_int(ratio.numer().clone());
        let denom = vm.ctx.new_int(ratio.denom().clone());
        Ok(vm.ctx.new_tuple(vec![numer, denom]))
    }
}

pub type PyFloatRef = PyRef<PyFloat>;

// Retrieve inner float value:
pub fn get_value(obj: &PyObjectRef) -> f64 {
    obj.payload::<PyFloat>().unwrap().value
}

pub fn make_float(vm: &VirtualMachine, obj: &PyObjectRef) -> PyResult<f64> {
    if objtype::isinstance(obj, &vm.ctx.float_type()) {
        Ok(get_value(obj))
    } else if let Ok(method) = vm.get_method(obj.clone(), "__float__") {
        let res = vm.invoke(method, vec![])?;
        Ok(get_value(&res))
    } else {
        Err(vm.new_type_error(format!("Cannot cast {} to float", obj)))
    }
}

#[rustfmt::skip] // to avoid line splitting
pub fn init(context: &PyContext) {
    PyFloat::extend_class(context, &context.float_type);
}
