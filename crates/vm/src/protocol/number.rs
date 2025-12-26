use std::ops::Deref;

use crossbeam_utils::atomic::AtomicCell;

use crate::{
    AsObject, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, TryFromBorrowedObject,
    VirtualMachine,
    builtins::{
        PyBaseExceptionRef, PyByteArray, PyBytes, PyComplex, PyFloat, PyInt, PyIntRef, PyStr, int,
    },
    common::int::{BytesToIntError, bytes_to_int},
    function::ArgBytesLike,
    object::{Traverse, TraverseFn},
    stdlib::warnings,
};

pub type PyNumberUnaryFunc<R = PyObjectRef> = fn(PyNumber<'_>, &VirtualMachine) -> PyResult<R>;
pub type PyNumberBinaryFunc = fn(&PyObject, &PyObject, &VirtualMachine) -> PyResult;
pub type PyNumberTernaryFunc = fn(&PyObject, &PyObject, &PyObject, &VirtualMachine) -> PyResult;

impl PyObject {
    #[inline]
    pub const fn number(&self) -> PyNumber<'_> {
        PyNumber { obj: self }
    }

    pub fn try_index_opt(&self, vm: &VirtualMachine) -> Option<PyResult<PyIntRef>> {
        if let Some(i) = self.downcast_ref_if_exact::<PyInt>(vm) {
            Some(Ok(i.to_owned()))
        } else if let Some(i) = self.downcast_ref::<PyInt>() {
            Some(Ok(vm.ctx.new_bigint(i.as_bigint())))
        } else {
            self.number().index(vm)
        }
    }

    #[inline]
    pub fn try_index(&self, vm: &VirtualMachine) -> PyResult<PyIntRef> {
        self.try_index_opt(vm).transpose()?.ok_or_else(|| {
            vm.new_type_error(format!(
                "'{}' object cannot be interpreted as an integer",
                self.class()
            ))
        })
    }

    pub fn try_int(&self, vm: &VirtualMachine) -> PyResult<PyIntRef> {
        fn try_convert(obj: &PyObject, lit: &[u8], vm: &VirtualMachine) -> PyResult<PyIntRef> {
            let base = 10;
            let digit_limit = vm.state.int_max_str_digits.load();

            let i = bytes_to_int(lit, base, digit_limit)
                .map_err(|e| handle_bytes_to_int_err(e, obj, vm))?;
            Ok(PyInt::from(i).into_ref(&vm.ctx))
        }

        if let Some(i) = self.downcast_ref_if_exact::<PyInt>(vm) {
            Ok(i.to_owned())
        } else if let Some(i) = self.number().int(vm).or_else(|| self.try_index_opt(vm)) {
            i
        } else if let Ok(Some(f)) = vm.get_special_method(self, identifier!(vm, __trunc__)) {
            warnings::warn(
                vm.ctx.exceptions.deprecation_warning,
                "The delegation of int() to __trunc__ is deprecated.".to_owned(),
                1,
                vm,
            )?;
            let ret = f.invoke((), vm)?;
            ret.try_index(vm).map_err(|_| {
                vm.new_type_error(format!(
                    "__trunc__ returned non-Integral (type {})",
                    ret.class()
                ))
            })
        } else if let Some(s) = self.downcast_ref::<PyStr>() {
            try_convert(self, s.as_wtf8().trim().as_bytes(), vm)
        } else if let Some(bytes) = self.downcast_ref::<PyBytes>() {
            try_convert(self, bytes, vm)
        } else if let Some(bytearray) = self.downcast_ref::<PyByteArray>() {
            try_convert(self, &bytearray.borrow_buf(), vm)
        } else if let Ok(buffer) = ArgBytesLike::try_from_borrowed_object(vm, self) {
            // TODO: replace to PyBuffer
            try_convert(self, &buffer.borrow_buf(), vm)
        } else {
            Err(vm.new_type_error(format!(
                "int() argument must be a string, a bytes-like object or a real number, not '{}'",
                self.class()
            )))
        }
    }

    pub fn try_float_opt(&self, vm: &VirtualMachine) -> Option<PyResult<PyRef<PyFloat>>> {
        if let Some(float) = self.downcast_ref_if_exact::<PyFloat>(vm) {
            Some(Ok(float.to_owned()))
        } else if let Some(f) = self.number().float(vm) {
            Some(f)
        } else {
            self.try_index_opt(vm)
                .map(|i| Ok(vm.ctx.new_float(int::try_to_float(i?.as_bigint(), vm)?)))
        }
    }

    #[inline]
    pub fn try_float(&self, vm: &VirtualMachine) -> PyResult<PyRef<PyFloat>> {
        self.try_float_opt(vm).ok_or_else(|| {
            vm.new_type_error(format!("must be real number, not {}", self.class()))
        })?
    }
}

#[derive(Default)]
pub struct PyNumberMethods {
    /* Number implementations must check *both*
    arguments for proper type and implement the necessary conversions
    in the slot functions themselves. */
    pub add: Option<PyNumberBinaryFunc>,
    pub subtract: Option<PyNumberBinaryFunc>,
    pub multiply: Option<PyNumberBinaryFunc>,
    pub remainder: Option<PyNumberBinaryFunc>,
    pub divmod: Option<PyNumberBinaryFunc>,
    pub power: Option<PyNumberTernaryFunc>,
    pub negative: Option<PyNumberUnaryFunc>,
    pub positive: Option<PyNumberUnaryFunc>,
    pub absolute: Option<PyNumberUnaryFunc>,
    pub boolean: Option<PyNumberUnaryFunc<bool>>,
    pub invert: Option<PyNumberUnaryFunc>,
    pub lshift: Option<PyNumberBinaryFunc>,
    pub rshift: Option<PyNumberBinaryFunc>,
    pub and: Option<PyNumberBinaryFunc>,
    pub xor: Option<PyNumberBinaryFunc>,
    pub or: Option<PyNumberBinaryFunc>,
    pub int: Option<PyNumberUnaryFunc>,
    pub float: Option<PyNumberUnaryFunc>,

    pub inplace_add: Option<PyNumberBinaryFunc>,
    pub inplace_subtract: Option<PyNumberBinaryFunc>,
    pub inplace_multiply: Option<PyNumberBinaryFunc>,
    pub inplace_remainder: Option<PyNumberBinaryFunc>,
    pub inplace_power: Option<PyNumberTernaryFunc>,
    pub inplace_lshift: Option<PyNumberBinaryFunc>,
    pub inplace_rshift: Option<PyNumberBinaryFunc>,
    pub inplace_and: Option<PyNumberBinaryFunc>,
    pub inplace_xor: Option<PyNumberBinaryFunc>,
    pub inplace_or: Option<PyNumberBinaryFunc>,

    pub floor_divide: Option<PyNumberBinaryFunc>,
    pub true_divide: Option<PyNumberBinaryFunc>,
    pub inplace_floor_divide: Option<PyNumberBinaryFunc>,
    pub inplace_true_divide: Option<PyNumberBinaryFunc>,

    pub index: Option<PyNumberUnaryFunc>,

    pub matrix_multiply: Option<PyNumberBinaryFunc>,
    pub inplace_matrix_multiply: Option<PyNumberBinaryFunc>,
}

impl PyNumberMethods {
    /// this is NOT a global variable
    pub const NOT_IMPLEMENTED: Self = Self {
        add: None,
        subtract: None,
        multiply: None,
        remainder: None,
        divmod: None,
        power: None,
        negative: None,
        positive: None,
        absolute: None,
        boolean: None,
        invert: None,
        lshift: None,
        rshift: None,
        and: None,
        xor: None,
        or: None,
        int: None,
        float: None,
        inplace_add: None,
        inplace_subtract: None,
        inplace_multiply: None,
        inplace_remainder: None,
        inplace_power: None,
        inplace_lshift: None,
        inplace_rshift: None,
        inplace_and: None,
        inplace_xor: None,
        inplace_or: None,
        floor_divide: None,
        true_divide: None,
        inplace_floor_divide: None,
        inplace_true_divide: None,
        index: None,
        matrix_multiply: None,
        inplace_matrix_multiply: None,
    };

    pub fn not_implemented() -> &'static Self {
        static GLOBAL_NOT_IMPLEMENTED: PyNumberMethods = PyNumberMethods::NOT_IMPLEMENTED;
        &GLOBAL_NOT_IMPLEMENTED
    }
}

#[derive(Copy, Clone)]
pub enum PyNumberBinaryOp {
    Add,
    Subtract,
    Multiply,
    Remainder,
    Divmod,
    Lshift,
    Rshift,
    And,
    Xor,
    Or,
    InplaceAdd,
    InplaceSubtract,
    InplaceMultiply,
    InplaceRemainder,
    InplaceLshift,
    InplaceRshift,
    InplaceAnd,
    InplaceXor,
    InplaceOr,
    FloorDivide,
    TrueDivide,
    InplaceFloorDivide,
    InplaceTrueDivide,
    MatrixMultiply,
    InplaceMatrixMultiply,
}

#[derive(Copy, Clone)]
pub enum PyNumberTernaryOp {
    Power,
    InplacePower,
}

#[derive(Default)]
pub struct PyNumberSlots {
    pub add: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub subtract: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub multiply: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub remainder: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub divmod: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub power: AtomicCell<Option<PyNumberTernaryFunc>>,
    pub negative: AtomicCell<Option<PyNumberUnaryFunc>>,
    pub positive: AtomicCell<Option<PyNumberUnaryFunc>>,
    pub absolute: AtomicCell<Option<PyNumberUnaryFunc>>,
    pub boolean: AtomicCell<Option<PyNumberUnaryFunc<bool>>>,
    pub invert: AtomicCell<Option<PyNumberUnaryFunc>>,
    pub lshift: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub rshift: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub and: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub xor: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub or: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub int: AtomicCell<Option<PyNumberUnaryFunc>>,
    pub float: AtomicCell<Option<PyNumberUnaryFunc>>,

    // Right variants (internal - not exposed in SlotAccessor)
    pub right_add: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub right_subtract: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub right_multiply: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub right_remainder: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub right_divmod: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub right_power: AtomicCell<Option<PyNumberTernaryFunc>>,
    pub right_lshift: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub right_rshift: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub right_and: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub right_xor: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub right_or: AtomicCell<Option<PyNumberBinaryFunc>>,

    pub inplace_add: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub inplace_subtract: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub inplace_multiply: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub inplace_remainder: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub inplace_power: AtomicCell<Option<PyNumberTernaryFunc>>,
    pub inplace_lshift: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub inplace_rshift: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub inplace_and: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub inplace_xor: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub inplace_or: AtomicCell<Option<PyNumberBinaryFunc>>,

    pub floor_divide: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub true_divide: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub right_floor_divide: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub right_true_divide: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub inplace_floor_divide: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub inplace_true_divide: AtomicCell<Option<PyNumberBinaryFunc>>,

    pub index: AtomicCell<Option<PyNumberUnaryFunc>>,

    pub matrix_multiply: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub right_matrix_multiply: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub inplace_matrix_multiply: AtomicCell<Option<PyNumberBinaryFunc>>,
}

impl From<&PyNumberMethods> for PyNumberSlots {
    fn from(value: &PyNumberMethods) -> Self {
        // right_* slots use the same function as left ops for native types
        Self {
            add: AtomicCell::new(value.add),
            subtract: AtomicCell::new(value.subtract),
            multiply: AtomicCell::new(value.multiply),
            remainder: AtomicCell::new(value.remainder),
            divmod: AtomicCell::new(value.divmod),
            power: AtomicCell::new(value.power),
            negative: AtomicCell::new(value.negative),
            positive: AtomicCell::new(value.positive),
            absolute: AtomicCell::new(value.absolute),
            boolean: AtomicCell::new(value.boolean),
            invert: AtomicCell::new(value.invert),
            lshift: AtomicCell::new(value.lshift),
            rshift: AtomicCell::new(value.rshift),
            and: AtomicCell::new(value.and),
            xor: AtomicCell::new(value.xor),
            or: AtomicCell::new(value.or),
            int: AtomicCell::new(value.int),
            float: AtomicCell::new(value.float),
            right_add: AtomicCell::new(value.add),
            right_subtract: AtomicCell::new(value.subtract),
            right_multiply: AtomicCell::new(value.multiply),
            right_remainder: AtomicCell::new(value.remainder),
            right_divmod: AtomicCell::new(value.divmod),
            right_power: AtomicCell::new(value.power),
            right_lshift: AtomicCell::new(value.lshift),
            right_rshift: AtomicCell::new(value.rshift),
            right_and: AtomicCell::new(value.and),
            right_xor: AtomicCell::new(value.xor),
            right_or: AtomicCell::new(value.or),
            inplace_add: AtomicCell::new(value.inplace_add),
            inplace_subtract: AtomicCell::new(value.inplace_subtract),
            inplace_multiply: AtomicCell::new(value.inplace_multiply),
            inplace_remainder: AtomicCell::new(value.inplace_remainder),
            inplace_power: AtomicCell::new(value.inplace_power),
            inplace_lshift: AtomicCell::new(value.inplace_lshift),
            inplace_rshift: AtomicCell::new(value.inplace_rshift),
            inplace_and: AtomicCell::new(value.inplace_and),
            inplace_xor: AtomicCell::new(value.inplace_xor),
            inplace_or: AtomicCell::new(value.inplace_or),
            floor_divide: AtomicCell::new(value.floor_divide),
            true_divide: AtomicCell::new(value.true_divide),
            right_floor_divide: AtomicCell::new(value.floor_divide),
            right_true_divide: AtomicCell::new(value.true_divide),
            inplace_floor_divide: AtomicCell::new(value.inplace_floor_divide),
            inplace_true_divide: AtomicCell::new(value.inplace_true_divide),
            index: AtomicCell::new(value.index),
            matrix_multiply: AtomicCell::new(value.matrix_multiply),
            right_matrix_multiply: AtomicCell::new(value.matrix_multiply),
            inplace_matrix_multiply: AtomicCell::new(value.inplace_matrix_multiply),
        }
    }
}

impl PyNumberSlots {
    pub fn left_binary_op(&self, op_slot: PyNumberBinaryOp) -> Option<PyNumberBinaryFunc> {
        use PyNumberBinaryOp::*;
        match op_slot {
            Add => self.add.load(),
            Subtract => self.subtract.load(),
            Multiply => self.multiply.load(),
            Remainder => self.remainder.load(),
            Divmod => self.divmod.load(),
            Lshift => self.lshift.load(),
            Rshift => self.rshift.load(),
            And => self.and.load(),
            Xor => self.xor.load(),
            Or => self.or.load(),
            InplaceAdd => self.inplace_add.load(),
            InplaceSubtract => self.inplace_subtract.load(),
            InplaceMultiply => self.inplace_multiply.load(),
            InplaceRemainder => self.inplace_remainder.load(),
            InplaceLshift => self.inplace_lshift.load(),
            InplaceRshift => self.inplace_rshift.load(),
            InplaceAnd => self.inplace_and.load(),
            InplaceXor => self.inplace_xor.load(),
            InplaceOr => self.inplace_or.load(),
            FloorDivide => self.floor_divide.load(),
            TrueDivide => self.true_divide.load(),
            InplaceFloorDivide => self.inplace_floor_divide.load(),
            InplaceTrueDivide => self.inplace_true_divide.load(),
            MatrixMultiply => self.matrix_multiply.load(),
            InplaceMatrixMultiply => self.inplace_matrix_multiply.load(),
        }
    }

    pub fn right_binary_op(&self, op_slot: PyNumberBinaryOp) -> Option<PyNumberBinaryFunc> {
        use PyNumberBinaryOp::*;
        match op_slot {
            Add => self.right_add.load(),
            Subtract => self.right_subtract.load(),
            Multiply => self.right_multiply.load(),
            Remainder => self.right_remainder.load(),
            Divmod => self.right_divmod.load(),
            Lshift => self.right_lshift.load(),
            Rshift => self.right_rshift.load(),
            And => self.right_and.load(),
            Xor => self.right_xor.load(),
            Or => self.right_or.load(),
            FloorDivide => self.right_floor_divide.load(),
            TrueDivide => self.right_true_divide.load(),
            MatrixMultiply => self.right_matrix_multiply.load(),
            _ => None,
        }
    }

    pub fn left_ternary_op(&self, op_slot: PyNumberTernaryOp) -> Option<PyNumberTernaryFunc> {
        use PyNumberTernaryOp::*;
        match op_slot {
            Power => self.power.load(),
            InplacePower => self.inplace_power.load(),
        }
    }

    pub fn right_ternary_op(&self, op_slot: PyNumberTernaryOp) -> Option<PyNumberTernaryFunc> {
        use PyNumberTernaryOp::*;
        match op_slot {
            Power => self.right_power.load(),
            _ => None,
        }
    }
}
#[derive(Copy, Clone)]
pub struct PyNumber<'a> {
    pub obj: &'a PyObject,
}

unsafe impl Traverse for PyNumber<'_> {
    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        self.obj.traverse(tracer_fn)
    }
}

impl Deref for PyNumber<'_> {
    type Target = PyObject;

    fn deref(&self) -> &Self::Target {
        self.obj
    }
}

impl<'a> PyNumber<'a> {
    // PyNumber_Check - slots are now inherited
    pub fn check(obj: &PyObject) -> bool {
        let methods = &obj.class().slots.as_number;
        let has_number = methods.int.load().is_some()
            || methods.index.load().is_some()
            || methods.float.load().is_some();
        has_number || obj.downcastable::<PyComplex>()
    }
}

impl PyNumber<'_> {
    // PyIndex_Check
    pub fn is_index(self) -> bool {
        self.class().slots.as_number.index.load().is_some()
    }

    #[inline]
    pub fn int(self, vm: &VirtualMachine) -> Option<PyResult<PyIntRef>> {
        self.class().slots.as_number.int.load().map(|f| {
            let ret = f(self, vm)?;

            if let Some(ret) = ret.downcast_ref_if_exact::<PyInt>(vm) {
                return Ok(ret.to_owned());
            }

            let ret_class = ret.class().to_owned();
            if let Some(ret) = ret.downcast_ref::<PyInt>() {
                warnings::warn(
                    vm.ctx.exceptions.deprecation_warning,
                    format!(
                        "__int__ returned non-int (type {ret_class}).  \
                    The ability to return an instance of a strict subclass of int \
                    is deprecated, and may be removed in a future version of Python."
                    ),
                    1,
                    vm,
                )?;

                Ok(ret.to_owned())
            } else {
                Err(vm.new_type_error(format!(
                    "{}.__int__ returned non-int(type {})",
                    self.class(),
                    ret_class
                )))
            }
        })
    }

    #[inline]
    pub fn index(self, vm: &VirtualMachine) -> Option<PyResult<PyIntRef>> {
        self.class().slots.as_number.index.load().map(|f| {
            let ret = f(self, vm)?;

            if let Some(ret) = ret.downcast_ref_if_exact::<PyInt>(vm) {
                return Ok(ret.to_owned());
            }

            let ret_class = ret.class().to_owned();
            if let Some(ret) = ret.downcast_ref::<PyInt>() {
                warnings::warn(
                    vm.ctx.exceptions.deprecation_warning,
                    format!(
                        "__index__ returned non-int (type {ret_class}).  \
                    The ability to return an instance of a strict subclass of int \
                    is deprecated, and may be removed in a future version of Python."
                    ),
                    1,
                    vm,
                )?;

                Ok(ret.to_owned())
            } else {
                Err(vm.new_type_error(format!(
                    "{}.__index__ returned non-int(type {})",
                    self.class(),
                    ret_class
                )))
            }
        })
    }

    #[inline]
    pub fn float(self, vm: &VirtualMachine) -> Option<PyResult<PyRef<PyFloat>>> {
        self.class().slots.as_number.float.load().map(|f| {
            let ret = f(self, vm)?;

            if let Some(ret) = ret.downcast_ref_if_exact::<PyFloat>(vm) {
                return Ok(ret.to_owned());
            }

            let ret_class = ret.class().to_owned();
            if let Some(ret) = ret.downcast_ref::<PyFloat>() {
                warnings::warn(
                    vm.ctx.exceptions.deprecation_warning,
                    format!(
                        "__float__ returned non-float (type {ret_class}).  \
                    The ability to return an instance of a strict subclass of float \
                    is deprecated, and may be removed in a future version of Python."
                    ),
                    1,
                    vm,
                )?;

                Ok(ret.to_owned())
            } else {
                Err(vm.new_type_error(format!(
                    "{}.__float__ returned non-float(type {})",
                    self.class(),
                    ret_class
                )))
            }
        })
    }
}

pub fn handle_bytes_to_int_err(
    e: BytesToIntError,
    obj: &PyObject,
    vm: &VirtualMachine,
) -> PyBaseExceptionRef {
    match e {
        BytesToIntError::InvalidLiteral { base } => vm.new_value_error(format!(
            "invalid literal for int() with base {base}: {}",
            match obj.repr(vm) {
                Ok(v) => v,
                Err(err) => return err,
            },
        )),
        BytesToIntError::InvalidBase => {
            vm.new_value_error("int() base must be >= 2 and <= 36, or 0")
        }
        BytesToIntError::DigitLimit { got, limit } => vm.new_value_error(format!(
"Exceeds the limit ({limit} digits) for integer string conversion: value has {got} digits; use sys.set_int_max_str_digits() to increase the limit"
                )),
    }
}
