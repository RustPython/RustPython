use super::objbytes;
use super::objint;
use super::objstr;
use super::objtype;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{
    IntoPyObject, PyContext, PyObjectRef, PyRef, PyResult, PyValue, TypeProtocol,
};
use crate::vm::VirtualMachine;
use num_bigint::ToBigInt;
use num_rational::Ratio;
use num_traits::ToPrimitive;

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

impl PyFloat {
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

    fn abs(&self, _vm: &VirtualMachine) -> f64 {
        self.value.abs()
    }

    fn add(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
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

    fn bool(&self, _vm: &VirtualMachine) -> bool {
        self.value != 0.0
    }

    fn divmod(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        if objtype::isinstance(&other, &vm.ctx.float_type())
            || objtype::isinstance(&other, &vm.ctx.int_type())
        {
            let r1 = self.floordiv(other.clone(), vm)?;
            let r2 = self.mod_(other, vm)?;
            Ok(vm.ctx.new_tuple(vec![r1, r2]))
        } else {
            Ok(vm.ctx.not_implemented())
        }
    }

    fn floordiv(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
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

    fn new_float(cls: PyClassRef, arg: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyFloatRef> {
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
            return Err(vm.new_type_error(format!("can't convert {} to float", arg.class().name)));
        };
        PyFloat { value }.into_ref_with_type(vm, cls)
    }

    fn mod_(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
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

    fn neg(&self, _vm: &VirtualMachine) -> f64 {
        -self.value
    }

    fn pow(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
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

    fn sub(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
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

    fn rsub(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
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

    fn repr(&self, _vm: &VirtualMachine) -> String {
        self.value.to_string()
    }

    fn truediv(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
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

    fn rtruediv(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
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

    fn mul(&self, other: PyObjectRef, vm: &VirtualMachine) -> PyResult {
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

    fn real(zelf: PyRef<Self>, _vm: &VirtualMachine) -> PyFloatRef {
        zelf
    }

    fn is_integer(&self, _vm: &VirtualMachine) -> bool {
        let v = self.value;
        (v - v.round()).abs() < std::f64::EPSILON
    }

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
    let float_type = &context.float_type;

    let float_doc = "Convert a string or number to a floating point number, if possible.";

    extend_class!(context, float_type, {
        "__eq__" => context.new_rustfunc(PyFloat::eq),
        "__lt__" => context.new_rustfunc(PyFloat::lt),
        "__le__" => context.new_rustfunc(PyFloat::le),
        "__gt__" => context.new_rustfunc(PyFloat::gt),
        "__ge__" => context.new_rustfunc(PyFloat::ge),
        "__abs__" => context.new_rustfunc(PyFloat::abs),
        "__add__" => context.new_rustfunc(PyFloat::add),
        "__radd__" => context.new_rustfunc(PyFloat::add),
        "__bool__" => context.new_rustfunc(PyFloat::bool),
        "__divmod__" => context.new_rustfunc(PyFloat::divmod),
        "__floordiv__" => context.new_rustfunc(PyFloat::floordiv),
        "__new__" => context.new_rustfunc(PyFloat::new_float),
        "__mod__" => context.new_rustfunc(PyFloat::mod_),
        "__neg__" => context.new_rustfunc(PyFloat::neg),
        "__pow__" => context.new_rustfunc(PyFloat::pow),
        "__sub__" => context.new_rustfunc(PyFloat::sub),
        "__rsub__" => context.new_rustfunc(PyFloat::rsub),
        "__repr__" => context.new_rustfunc(PyFloat::repr),
        "__doc__" => context.new_str(float_doc.to_string()),
        "__truediv__" => context.new_rustfunc(PyFloat::truediv),
        "__rtruediv__" => context.new_rustfunc(PyFloat::rtruediv),
        "__mul__" => context.new_rustfunc(PyFloat::mul),
        "__rmul__" => context.new_rustfunc(PyFloat::mul),
        "real" => context.new_property(PyFloat::real),
        "is_integer" => context.new_rustfunc(PyFloat::is_integer),
        "as_integer_ratio" => context.new_rustfunc(PyFloat::as_integer_ratio)
    });
}
