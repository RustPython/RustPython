use std::ops::Deref;

use crossbeam_utils::atomic::AtomicCell;

use crate::{
    builtins::{int, PyByteArray, PyBytes, PyComplex, PyFloat, PyInt, PyIntRef, PyStr, PyType},
    common::int::bytes_to_int,
    function::ArgBytesLike,
    stdlib::warnings,
    AsObject, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, TryFromBorrowedObject,
    VirtualMachine,
};

pub type PyNumberUnaryFunc<R = PyObjectRef> = fn(PyNumber, &VirtualMachine) -> PyResult<R>;
pub type PyNumberBinaryFunc = fn(&PyObject, &PyObject, &VirtualMachine) -> PyResult;

#[derive(Copy, Clone)]
pub struct PyNumber<'a>(&'a PyObject);

impl<'a> Deref for PyNumber<'a> {
    type Target = PyObject;

    fn deref(&self) -> &Self::Target {
        self.0
    }
}

impl<'a> PyNumber<'a> {
    pub(crate) fn obj(self) -> &'a PyObject {
        self.0
    }
}

macro_rules! load_pynumber_method {
    ($cls:expr, $x:ident, $y:ident) => {{
        let class = $cls;
        if let Some(ext) = class.heaptype_ext.as_ref() {
            ext.number_slots.$y.load()
        } else if let Some(methods) = class.slots.as_number {
            methods.$x
        } else {
            None
        }
    }};
    ($cls:expr, $x:ident) => {{
        load_pynumber_method!($cls, $x, $x)
    }};
}

impl PyNumber<'_> {
    pub fn check(obj: &PyObject) -> bool {
        let class = obj.class();
        if let Some(ext) = class.heaptype_ext.as_ref() {
            if ext.number_slots.int.load().is_some()
                || ext.number_slots.index.load().is_some()
                || ext.number_slots.float.load().is_some()
            {
                return true;
            }
        }
        if let Some(methods) = class.slots.as_number {
            if methods.int.is_some() || methods.index.is_some() || methods.float.is_some() {
                return true;
            }
        }
        obj.payload_is::<PyComplex>()
    }

    pub fn is_index(self) -> bool {
        load_pynumber_method!(self.class(), index).is_some()
    }

    #[inline]
    pub fn int(self, vm: &VirtualMachine) -> Option<PyResult<PyIntRef>> {
        load_pynumber_method!(self.class(), int).map(|f| {
            let ret = f(self, vm)?;
            let value = if !ret.class().is(PyInt::class(vm)) {
                warnings::warn(
                    vm.ctx.exceptions.deprecation_warning,
                    format!(
                        "__int__ returned non-int (type {}).  \
                The ability to return an instance of a strict subclass of int \
                is deprecated, and may be removed in a future version of Python.",
                        ret.class()
                    ),
                    1,
                    vm,
                )?;
                vm.ctx.new_bigint(ret.as_bigint())
            } else {
                ret
            };
            Ok(value)
        })
    }

    #[inline]
    pub fn index(self, vm: &VirtualMachine) -> Option<PyResult<PyIntRef>> {
        load_pynumber_method!(self.class(), index).map(|f| {
            let ret = f(self, vm)?;
            let value = if !ret.class().is(PyInt::class(vm)) {
                warnings::warn(
                    vm.ctx.exceptions.deprecation_warning,
                    format!(
                        "__index__ returned non-int (type {}).  \
                The ability to return an instance of a strict subclass of int \
                is deprecated, and may be removed in a future version of Python.",
                        ret.class()
                    ),
                    1,
                    vm,
                )?;
                vm.ctx.new_bigint(ret.as_bigint())
            } else {
                ret
            };
            Ok(value)
        })
    }

    #[inline]
    pub fn float(self, vm: &VirtualMachine) -> Option<PyResult<PyRef<PyFloat>>> {
        load_pynumber_method!(self.class(), float).map(|f| {
            let ret = f(self, vm)?;
            let value = if !ret.class().is(PyFloat::class(vm)) {
                warnings::warn(
                    vm.ctx.exceptions.deprecation_warning,
                    format!(
                        "__float__ returned non-float (type {}).  \
                The ability to return an instance of a strict subclass of float \
                is deprecated, and may be removed in a future version of Python.",
                        ret.class()
                    ),
                    1,
                    vm,
                )?;
                vm.ctx.new_float(ret.to_f64())
            } else {
                ret
            };
            Ok(value)
        })
    }
}

impl PyObject {
    pub fn to_number(&self) -> PyNumber {
        PyNumber(self)
    }

    pub fn try_index_opt(&self, vm: &VirtualMachine) -> Option<PyResult<PyIntRef>> {
        if let Some(i) = self.downcast_ref_if_exact::<PyInt>(vm) {
            Some(Ok(i.to_owned()))
        } else if let Some(i) = self.payload::<PyInt>() {
            Some(Ok(vm.ctx.new_bigint(i.as_bigint())))
        } else {
            self.to_number().index(vm)
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
            let i = bytes_to_int(lit, base).ok_or_else(|| {
                let repr = match obj.repr(vm) {
                    Ok(repr) => repr,
                    Err(err) => return err,
                };
                vm.new_value_error(format!(
                    "invalid literal for int() with base {}: {}",
                    base, repr,
                ))
            })?;
            Ok(PyInt::from(i).into_ref(vm))
        }

        if let Some(i) = self.downcast_ref_if_exact::<PyInt>(vm) {
            Ok(i.to_owned())
        } else if let Some(i) = self.to_number().int(vm).or_else(|| self.try_index_opt(vm)) {
            i
        } else if let Ok(Ok(f)) = vm.get_special_method(self.to_owned(), identifier!(vm, __trunc__))
        {
            // TODO: Deprecate in 3.11
            // warnings::warn(
            //     vm.ctx.exceptions.deprecation_warning.clone(),
            //     "The delegation of int() to __trunc__ is deprecated.".to_owned(),
            //     1,
            //     vm,
            // )?;
            let ret = f.invoke((), vm)?;
            ret.try_index(vm).map_err(|_| {
                vm.new_type_error(format!(
                    "__trunc__ returned non-Integral (type {})",
                    ret.class()
                ))
            })
        } else if let Some(s) = self.payload::<PyStr>() {
            try_convert(self, s.as_str().as_bytes(), vm)
        } else if let Some(bytes) = self.payload::<PyBytes>() {
            try_convert(self, bytes, vm)
        } else if let Some(bytearray) = self.payload::<PyByteArray>() {
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
        } else if let Some(f) = self.to_number().float(vm) {
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
    pub power: Option<PyNumberBinaryFunc>,
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
    pub int: Option<PyNumberUnaryFunc<PyRef<PyInt>>>,
    pub float: Option<PyNumberUnaryFunc<PyRef<PyFloat>>>,

    pub inplace_add: Option<PyNumberBinaryFunc>,
    pub inplace_subtract: Option<PyNumberBinaryFunc>,
    pub inplace_multiply: Option<PyNumberBinaryFunc>,
    pub inplace_remainder: Option<PyNumberBinaryFunc>,
    pub inplace_power: Option<PyNumberBinaryFunc>,
    pub inplace_lshift: Option<PyNumberBinaryFunc>,
    pub inplace_rshift: Option<PyNumberBinaryFunc>,
    pub inplace_and: Option<PyNumberBinaryFunc>,
    pub inplace_xor: Option<PyNumberBinaryFunc>,
    pub inplace_or: Option<PyNumberBinaryFunc>,

    pub floor_divide: Option<PyNumberBinaryFunc>,
    pub true_divide: Option<PyNumberBinaryFunc>,
    pub inplace_floor_divide: Option<PyNumberBinaryFunc>,
    pub inplace_true_divide: Option<PyNumberBinaryFunc>,

    pub index: Option<PyNumberUnaryFunc<PyRef<PyInt>>>,

    pub matrix_multiply: Option<PyNumberBinaryFunc>,
    pub inplace_matrix_multiply: Option<PyNumberBinaryFunc>,
}

impl PyNumberMethods {
    /// this is NOT a global variable
    // TODO: weak order read for performance
    pub const NOT_IMPLEMENTED: PyNumberMethods = PyNumberMethods {
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
}

#[derive(Copy, Clone)]
pub enum PyNumberBinaryOp {
    Add,
    Subtract,
    Multiply,
    Remainder,
    Divmod,
    Power,
    Lshift,
    Rshift,
    And,
    Xor,
    Or,
    InplaceAdd,
    InplaceSubtract,
    InplaceMultiply,
    InplaceRemainder,
    InplacePower,
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

impl PyNumberBinaryOp {
    pub fn left(self, cls: &PyType) -> Option<PyNumberBinaryFunc> {
        use PyNumberBinaryOp::*;
        match self {
            Add => load_pynumber_method!(cls, add),
            Subtract => load_pynumber_method!(cls, subtract),
            Multiply => load_pynumber_method!(cls, multiply),
            Remainder => load_pynumber_method!(cls, remainder),
            Divmod => load_pynumber_method!(cls, divmod),
            Power => load_pynumber_method!(cls, power),
            Lshift => load_pynumber_method!(cls, lshift),
            Rshift => load_pynumber_method!(cls, rshift),
            And => load_pynumber_method!(cls, and),
            Xor => load_pynumber_method!(cls, xor),
            Or => load_pynumber_method!(cls, or),
            InplaceAdd => load_pynumber_method!(cls, inplace_add),
            InplaceSubtract => load_pynumber_method!(cls, inplace_subtract),
            InplaceMultiply => load_pynumber_method!(cls, inplace_multiply),
            InplaceRemainder => load_pynumber_method!(cls, inplace_remainder),
            InplacePower => load_pynumber_method!(cls, inplace_power),
            InplaceLshift => load_pynumber_method!(cls, inplace_lshift),
            InplaceRshift => load_pynumber_method!(cls, inplace_rshift),
            InplaceAnd => load_pynumber_method!(cls, inplace_and),
            InplaceXor => load_pynumber_method!(cls, inplace_xor),
            InplaceOr => load_pynumber_method!(cls, inplace_or),
            FloorDivide => load_pynumber_method!(cls, floor_divide),
            TrueDivide => load_pynumber_method!(cls, true_divide),
            InplaceFloorDivide => load_pynumber_method!(cls, inplace_floor_divide),
            InplaceTrueDivide => load_pynumber_method!(cls, inplace_true_divide),
            MatrixMultiply => load_pynumber_method!(cls, matrix_multiply),
            InplaceMatrixMultiply => load_pynumber_method!(cls, inplace_matrix_multiply),
        }
    }

    pub fn right(self, cls: &PyType) -> Option<PyNumberBinaryFunc> {
        use PyNumberBinaryOp::*;
        match self {
            Add => load_pynumber_method!(cls, add, right_add),
            Subtract => load_pynumber_method!(cls, subtract, right_subtract),
            Multiply => load_pynumber_method!(cls, multiply, right_multiply),
            Remainder => load_pynumber_method!(cls, remainder, right_remainder),
            Divmod => load_pynumber_method!(cls, divmod, right_divmod),
            Power => load_pynumber_method!(cls, power, right_power),
            Lshift => load_pynumber_method!(cls, lshift, right_lshift),
            Rshift => load_pynumber_method!(cls, rshift, right_rshift),
            And => load_pynumber_method!(cls, and, right_and),
            Xor => load_pynumber_method!(cls, xor, right_xor),
            Or => load_pynumber_method!(cls, or, right_or),
            FloorDivide => load_pynumber_method!(cls, floor_divide, right_floor_divide),
            TrueDivide => load_pynumber_method!(cls, true_divide, right_true_divide),
            MatrixMultiply => load_pynumber_method!(cls, matrix_multiply, right_matrix_multiply),
            _ => None,
        }
    }
}

#[derive(Default)]
pub struct PyNumberSlots {
    pub add: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub subtract: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub multiply: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub remainder: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub divmod: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub power: AtomicCell<Option<PyNumberBinaryFunc>>,
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
    pub int: AtomicCell<Option<PyNumberUnaryFunc<PyRef<PyInt>>>>,
    pub float: AtomicCell<Option<PyNumberUnaryFunc<PyRef<PyFloat>>>>,

    pub right_add: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub right_subtract: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub right_multiply: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub right_remainder: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub right_divmod: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub right_power: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub right_lshift: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub right_rshift: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub right_and: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub right_xor: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub right_or: AtomicCell<Option<PyNumberBinaryFunc>>,

    pub inplace_add: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub inplace_subtract: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub inplace_multiply: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub inplace_remainder: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub inplace_power: AtomicCell<Option<PyNumberBinaryFunc>>,
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

    pub index: AtomicCell<Option<PyNumberUnaryFunc<PyRef<PyInt>>>>,

    pub matrix_multiply: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub right_matrix_multiply: AtomicCell<Option<PyNumberBinaryFunc>>,
    pub inplace_matrix_multiply: AtomicCell<Option<PyNumberBinaryFunc>>,
}

impl From<&PyNumberMethods> for PyNumberSlots {
    fn from(value: &PyNumberMethods) -> Self {
        // right_* functions will use the same left function as PyNumberMethods garrentee to
        // support both f(self, other) and f(other, self)
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
