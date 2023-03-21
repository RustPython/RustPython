use std::ops::Deref;

use crate::{
    builtins::{int, PyByteArray, PyBytes, PyComplex, PyFloat, PyInt, PyIntRef, PyStr},
    common::int::bytes_to_int,
    function::ArgBytesLike,
    stdlib::warnings,
    AsObject, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, TryFromBorrowedObject,
    VirtualMachine,
};

pub type PyNumberUnaryFunc<R = PyObjectRef> = fn(PyNumber, &VirtualMachine) -> PyResult<R>;
pub type PyNumberBinaryFunc = fn(&PyObject, &PyObject, &VirtualMachine) -> PyResult;

impl PyObject {
    #[inline]
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
            Ok(PyInt::from(i).into_ref(&vm.ctx))
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
    #[allow(clippy::declare_interior_mutable_const)]
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

    // PyNumber_Check
    pub fn check(obj: &PyObject) -> bool {
        let methods = &obj.class().slots.number;
        methods.int.load().is_some()
            || methods.index.load().is_some()
            || methods.float.load().is_some()
            || obj.payload_is::<PyComplex>()
    }
}

impl PyNumber<'_> {
    // PyIndex_Check
    pub fn is_index(self) -> bool {
        self.class().slots.number.index.load().is_some()
    }

    #[inline]
    pub fn int(self, vm: &VirtualMachine) -> Option<PyResult<PyIntRef>> {
        self.class().slots.number.int.load().map(|f| {
            let ret = f(self, vm)?;
            let value = if !ret.class().is(PyInt::class(&vm.ctx)) {
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
        self.class().slots.number.index.load().map(|f| {
            let ret = f(self, vm)?;
            let value = if !ret.class().is(PyInt::class(&vm.ctx)) {
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
        self.class().slots.number.float.load().map(|f| {
            let ret = f(self, vm)?;
            let value = if !ret.class().is(PyFloat::class(&vm.ctx)) {
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
