use super::objbytes;
use super::objint;
use super::objstr;
use super::objtype;
use crate::pyobject::{
    IntoPyObject, PyContext, PyObject, PyObjectRef, PyRef, PyResult, PyValue, TypeProtocol,
};
use crate::vm::VirtualMachine;
use num_bigint::ToBigInt;
use num_rational::Ratio;
use num_traits::ToPrimitive;

#[derive(Debug, Copy, Clone, PartialEq)]
pub struct PyFloat {
    value: f64,
}

impl PyValue for PyFloat {
    fn class(vm: &VirtualMachine) -> PyObjectRef {
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

pub type PyFloatRef = PyRef<PyFloat>;

impl PyFloatRef {
    fn eq(self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
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

    fn lt(self, i2: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
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

    fn le(self, i2: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
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

    fn gt(self, i2: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
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

    fn ge(self, i2: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
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

    fn abs(self, _vm: &VirtualMachine) -> f64 {
        self.value.abs()
    }

    fn add(self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        let v1 = self.value;
        if objtype::isinstance(&other, &vm.ctx.float_type()) {
            vm.ctx.new_float(v1 + get_value(&other))
        } else if objtype::isinstance(&other, &vm.ctx.int_type()) {
            vm.ctx
                .new_float(v1 + objint::get_value(&other).to_f64().unwrap())
        } else {
            vm.ctx.not_implemented()
        }
    }

    fn divmod(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if objtype::isinstance(&other, &vm.ctx.float_type())
            || objtype::isinstance(&other, &vm.ctx.int_type())
        {
            let r1 = PyFloatRef::floordiv(self.clone(), other.clone(), vm)?;
            let r2 = PyFloatRef::mod_(self, other, vm)?;
            Ok(vm.ctx.new_tuple(vec![r1, r2]))
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    fn floordiv(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let v1 = self.value;
        let v2 = if objtype::isinstance(&other, &vm.ctx.float_type) {
            get_value(&other)
        } else if objtype::isinstance(&other, &vm.ctx.int_type) {
            objint::get_value(&other).to_f64().ok_or_else(|| {
                vm.new_overflow_error("int too large to convert to float".to_string())
            })?
        } else {
            return Ok(vm.ctx.not_implemented());
        };

        if v2 != 0.0 {
            Ok(vm.ctx.new_float((v1 / v2).floor()))
        } else {
            Err(vm.new_zero_division_error("float floordiv by zero".to_string()))
        }
    }

    fn new_float(cls: PyObjectRef, arg: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let value = if objtype::isinstance(&arg, &vm.ctx.float_type()) {
            get_value(&arg)
        } else if objtype::isinstance(&arg, &vm.ctx.int_type()) {
            match objint::get_value(&arg).to_f64() {
                Some(f) => f,
                None => {
                    return Err(
                        vm.new_overflow_error("int too large to convert to float".to_string())
                    );
                }
            }
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
            let type_name = objtype::get_type_name(&arg.typ());
            return Err(vm.new_type_error(format!("can't convert {} to float", type_name)));
        };
        Ok(PyObject::new(PyFloat { value }, cls.clone()))
    }

    fn mod_(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let v1 = self.value;
        let v2 = if objtype::isinstance(&other, &vm.ctx.float_type) {
            get_value(&other)
        } else if objtype::isinstance(&other, &vm.ctx.int_type) {
            objint::get_value(&other).to_f64().ok_or_else(|| {
                vm.new_overflow_error("int too large to convert to float".to_string())
            })?
        } else {
            return Ok(vm.ctx.not_implemented());
        };

        if v2 != 0.0 {
            Ok(vm.ctx.new_float(v1 % v2))
        } else {
            Err(vm.new_zero_division_error("float mod by zero".to_string()))
        }
    }

    fn neg(self, _vm: &VirtualMachine) -> f64 {
        -self.value
    }

    fn pow(self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        let v1 = self.value;
        if objtype::isinstance(&other, &vm.ctx.float_type()) {
            vm.ctx.new_float(v1.powf(get_value(&other)))
        } else if objtype::isinstance(&other, &vm.ctx.int_type()) {
            let result = v1.powf(objint::get_value(&other).to_f64().unwrap());
            vm.ctx.new_float(result)
        } else {
            vm.ctx.not_implemented()
        }
    }

    fn sub(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let v1 = self.value;
        if objtype::isinstance(&other, &vm.ctx.float_type()) {
            Ok(vm.ctx.new_float(v1 - get_value(&other)))
        } else if objtype::isinstance(&other, &vm.ctx.int_type()) {
            Ok(vm
                .ctx
                .new_float(v1 - objint::get_value(&other).to_f64().unwrap()))
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    fn rsub(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let v1 = self.value;
        if objtype::isinstance(&other, &vm.ctx.float_type()) {
            Ok(vm.ctx.new_float(get_value(&other) - v1))
        } else if objtype::isinstance(&other, &vm.ctx.int_type()) {
            Ok(vm
                .ctx
                .new_float(objint::get_value(&other).to_f64().unwrap() - v1))
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    fn repr(self, _vm: &VirtualMachine) -> String {
        self.value.to_string()
    }

    fn truediv(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let v1 = self.value;
        let v2 = if objtype::isinstance(&other, &vm.ctx.float_type) {
            get_value(&other)
        } else if objtype::isinstance(&other, &vm.ctx.int_type) {
            objint::get_value(&other).to_f64().ok_or_else(|| {
                vm.new_overflow_error("int too large to convert to float".to_string())
            })?
        } else {
            return Ok(vm.ctx.not_implemented());
        };

        if v2 != 0.0 {
            Ok(vm.ctx.new_float(v1 / v2))
        } else {
            Err(vm.new_zero_division_error("float division by zero".to_string()))
        }
    }

    fn rtruediv(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let v1 = self.value;
        let v2 = if objtype::isinstance(&other, &vm.ctx.float_type) {
            get_value(&other)
        } else if objtype::isinstance(&other, &vm.ctx.int_type) {
            objint::get_value(&other).to_f64().ok_or_else(|| {
                vm.new_overflow_error("int too large to convert to float".to_string())
            })?
        } else {
            return Ok(vm.ctx.not_implemented());
        };

        if v1 != 0.0 {
            Ok(vm.ctx.new_float(v2 / v1))
        } else {
            Err(vm.new_zero_division_error("float division by zero".to_string()))
        }
    }

    fn mul(self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        let v1 = self.value;
        if objtype::isinstance(&other, &vm.ctx.float_type) {
            Ok(vm.ctx.new_float(v1 * get_value(&other)))
        } else if objtype::isinstance(&other, &vm.ctx.int_type) {
            Ok(vm
                .ctx
                .new_float(v1 * objint::get_value(&other).to_f64().unwrap()))
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    fn is_integer(self, _vm: &VirtualMachine) -> bool {
        let v = self.value;
        (v - v.round()).abs() < std::f64::EPSILON
    }

    fn real(self, _vm: &VirtualMachine) -> Self {
        self
    }

    fn as_integer_ratio(self, vm: &VirtualMachine) -> PyResult {
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
    let float_type = &context.float_type;

    let float_doc = "Convert a string or number to a floating point number, if possible.";

    context.set_attr(&float_type, "__eq__", context.new_rustfunc(PyFloatRef::eq));
    context.set_attr(&float_type, "__lt__", context.new_rustfunc(PyFloatRef::lt));
    context.set_attr(&float_type, "__le__", context.new_rustfunc(PyFloatRef::le));
    context.set_attr(&float_type, "__gt__", context.new_rustfunc(PyFloatRef::gt));
    context.set_attr(&float_type, "__ge__", context.new_rustfunc(PyFloatRef::ge));
    context.set_attr(&float_type, "__abs__", context.new_rustfunc(PyFloatRef::abs));
    context.set_attr(&float_type, "__add__", context.new_rustfunc(PyFloatRef::add));
    context.set_attr(&float_type, "__radd__", context.new_rustfunc(PyFloatRef::add));
    context.set_attr(&float_type, "__divmod__", context.new_rustfunc(PyFloatRef::divmod));
    context.set_attr(&float_type, "__floordiv__", context.new_rustfunc(PyFloatRef::floordiv));
    context.set_attr(&float_type, "__new__", context.new_rustfunc(PyFloatRef::new_float));
    context.set_attr(&float_type, "__mod__", context.new_rustfunc(PyFloatRef::mod_));
    context.set_attr(&float_type, "__neg__", context.new_rustfunc(PyFloatRef::neg));
    context.set_attr(&float_type, "__pow__", context.new_rustfunc(PyFloatRef::pow));
    context.set_attr(&float_type, "__sub__", context.new_rustfunc(PyFloatRef::sub));
    context.set_attr(&float_type, "__rsub__", context.new_rustfunc(PyFloatRef::rsub));
    context.set_attr(&float_type, "__repr__", context.new_rustfunc(PyFloatRef::repr));
    context.set_attr(&float_type, "__doc__", context.new_str(float_doc.to_string()));
    context.set_attr(&float_type, "__truediv__", context.new_rustfunc(PyFloatRef::truediv));
    context.set_attr(&float_type, "__rtruediv__", context.new_rustfunc(PyFloatRef::rtruediv));
    context.set_attr(&float_type, "__mul__", context.new_rustfunc(PyFloatRef::mul));
    context.set_attr(&float_type, "__rmul__", context.new_rustfunc(PyFloatRef::mul));
    context.set_attr(&float_type, "real", context.new_property(PyFloatRef::real));
    context.set_attr(&float_type, "is_integer", context.new_rustfunc(PyFloatRef::is_integer));
    context.set_attr(&float_type, "as_integer_ratio", context.new_rustfunc(PyFloatRef::as_integer_ratio));
}
