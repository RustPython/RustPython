use num_bigint::Sign;
use num_traits::Zero;

use super::int::PyInt;
use super::pystr::PyStrRef;
use crate::function::OptionalArg;
use crate::pyobject::{
    BorrowValue, IdProtocol, IntoPyObject, PyClassImpl, PyContext, PyObjectRef, PyResult,
    TryFromObject, TypeProtocol,
};
use crate::vm::VirtualMachine;

impl IntoPyObject for bool {
    fn into_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_bool(self)
    }
}

impl TryFromObject for bool {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<bool> {
        if obj.isinstance(&vm.ctx.types.int_type) {
            Ok(get_value(&obj))
        } else {
            Err(vm.new_type_error(format!("Expected type bool, not {}", obj.class().name)))
        }
    }
}

/// Convert Python bool into Rust bool.
pub fn boolval(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<bool> {
    if obj.is(&vm.ctx.true_value) {
        return Ok(true);
    }
    if obj.is(&vm.ctx.false_value) {
        return Ok(false);
    }
    let rs_bool = match vm.get_method(obj.clone(), "__bool__") {
        Some(method_or_err) => {
            // If descriptor returns Error, propagate it further
            let method = method_or_err?;
            let bool_obj = vm.invoke(&method, ())?;
            if !bool_obj.isinstance(&vm.ctx.types.bool_type) {
                return Err(vm.new_type_error(format!(
                    "__bool__ should return bool, returned type {}",
                    bool_obj.class().name
                )));
            }

            get_value(&bool_obj)
        }
        None => match vm.get_method(obj, "__len__") {
            Some(method_or_err) => {
                let method = method_or_err?;
                let bool_obj = vm.invoke(&method, ())?;
                let int_obj = bool_obj.payload::<PyInt>().ok_or_else(|| {
                    vm.new_type_error(format!(
                        "'{}' object cannot be interpreted as an integer",
                        bool_obj.class().name
                    ))
                })?;

                let len_val = int_obj.borrow_value();
                if len_val.sign() == Sign::Minus {
                    return Err(vm.new_value_error("__len__() should return >= 0".to_owned()));
                }
                !len_val.is_zero()
            }
            None => true,
        },
    };
    Ok(rs_bool)
}

/// bool(x) -> bool
///
/// Returns True when the argument x is true, False otherwise.
/// The builtins True and False are the only two instances of the class bool.
/// The class bool is a subclass of the class int, and cannot be subclassed.
#[pyclass(name = "bool", module = false, base = "PyInt")]
pub struct PyBool;

#[pyimpl]
impl PyBool {
    #[pymethod(magic)]
    fn repr(zelf: bool) -> String {
        if zelf { "True" } else { "False" }.to_owned()
    }

    #[pymethod(magic)]
    fn format(obj: PyObjectRef, format_spec: PyStrRef, vm: &VirtualMachine) -> PyResult<PyStrRef> {
        if format_spec.borrow_value().is_empty() {
            vm.to_str(&obj)
        } else {
            Err(vm.new_type_error("unsupported format string passed to bool.__format__".to_owned()))
        }
    }

    #[pymethod(name = "__ror__")]
    #[pymethod(magic)]
    fn or(lhs: PyObjectRef, rhs: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if lhs.isinstance(&vm.ctx.types.bool_type) && rhs.isinstance(&vm.ctx.types.bool_type) {
            let lhs = get_value(&lhs);
            let rhs = get_value(&rhs);
            (lhs || rhs).into_pyobject(vm)
        } else {
            get_py_int(&lhs).or(rhs, vm).into_pyobject(vm)
        }
    }

    #[pymethod(name = "__rand__")]
    #[pymethod(magic)]
    fn and(lhs: PyObjectRef, rhs: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if lhs.isinstance(&vm.ctx.types.bool_type) && rhs.isinstance(&vm.ctx.types.bool_type) {
            let lhs = get_value(&lhs);
            let rhs = get_value(&rhs);
            (lhs && rhs).into_pyobject(vm)
        } else {
            get_py_int(&lhs).and(rhs, vm).into_pyobject(vm)
        }
    }

    #[pymethod(name = "__rxor__")]
    #[pymethod(magic)]
    fn xor(lhs: PyObjectRef, rhs: PyObjectRef, vm: &VirtualMachine) -> PyObjectRef {
        if lhs.isinstance(&vm.ctx.types.bool_type) && rhs.isinstance(&vm.ctx.types.bool_type) {
            let lhs = get_value(&lhs);
            let rhs = get_value(&rhs);
            (lhs ^ rhs).into_pyobject(vm)
        } else {
            get_py_int(&lhs).xor(rhs, vm).into_pyobject(vm)
        }
    }

    #[pyslot]
    fn tp_new(zelf: PyObjectRef, x: OptionalArg<PyObjectRef>, vm: &VirtualMachine) -> PyResult {
        if !zelf.isinstance(&vm.ctx.types.type_type) {
            let actual_type = &zelf.class().name;
            return Err(vm.new_type_error(format!(
                "requires a 'type' object but received a '{}'",
                actual_type
            )));
        }
        let val = match x {
            OptionalArg::Present(val) => boolval(vm, val)?,
            OptionalArg::Missing => false,
        };
        Ok(vm.ctx.new_bool(val))
    }
}

pub(crate) fn init(context: &PyContext) {
    PyBool::extend_class(context, &context.types.bool_type);
}

// pub fn not(vm: &VirtualMachine, obj: &PyObjectRef) -> PyResult<bool> {
//     if obj.isinstance(&vm.ctx.types.bool_type) {
//         let value = get_value(obj);
//         Ok(!value)
//     } else {
//         Err(vm.new_type_error(format!("Can only invert a bool, on {:?}", obj)))
//     }
// }

// Retrieve inner int value:
pub(crate) fn get_value(obj: &PyObjectRef) -> bool {
    !obj.payload::<PyInt>().unwrap().borrow_value().is_zero()
}

fn get_py_int(obj: &PyObjectRef) -> &PyInt {
    &obj.payload::<PyInt>().unwrap()
}

#[derive(Debug, Default, Copy, Clone, PartialEq)]
pub struct IntoPyBool {
    value: bool,
}

impl IntoPyBool {
    pub const TRUE: IntoPyBool = IntoPyBool { value: true };
    pub const FALSE: IntoPyBool = IntoPyBool { value: false };

    pub fn to_bool(self) -> bool {
        self.value
    }
}

impl TryFromObject for IntoPyBool {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        Ok(IntoPyBool {
            value: boolval(vm, obj)?,
        })
    }
}
