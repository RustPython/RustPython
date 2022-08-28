use crate::{
    builtins::{
        int, type_::PointerSlot, PyByteArray, PyBytes, PyComplex, PyFloat, PyInt, PyIntRef, PyStr,
    },
    function::ArgBytesLike,
    stdlib::warnings,
    AsObject, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, TryFromBorrowedObject,
    VirtualMachine,
};
use crossbeam_utils::atomic::AtomicCell;
use std::ptr;

type UnaryFunc<R = PyObjectRef> = AtomicCell<Option<fn(PyNumber, &VirtualMachine) -> PyResult<R>>>;
type BinaryFunc<R = PyObjectRef> =
    AtomicCell<Option<fn(PyNumber, &PyObject, &VirtualMachine) -> PyResult<R>>>;

impl PyObject {
    #[inline]
    pub fn to_number(&self) -> PyNumber<'_> {
        PyNumber::from(self)
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
            match int::bytes_to_int(lit, base) {
                Some(i) => Ok(PyInt::from(i).into_ref(vm)),
                None => Err(vm.new_value_error(format!(
                    "invalid literal for int() with base {}: {}",
                    base,
                    obj.repr(vm)?,
                ))),
            }
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
// #[repr(C)]
pub struct PyNumberMethods {
    /* Number implementations must check *both*
    arguments for proper type and implement the necessary conversions
    in the slot functions themselves. */
    pub add: BinaryFunc,
    pub subtract: BinaryFunc,
    pub multiply: BinaryFunc,
    pub remainder: BinaryFunc,
    pub divmod: BinaryFunc,
    pub power: BinaryFunc,
    pub negative: UnaryFunc,
    pub positive: UnaryFunc,
    pub absolute: UnaryFunc,
    pub boolean: UnaryFunc<bool>,
    pub invert: UnaryFunc,
    pub lshift: BinaryFunc,
    pub rshift: BinaryFunc,
    pub and: BinaryFunc,
    pub xor: BinaryFunc,
    pub or: BinaryFunc,
    pub int: UnaryFunc<PyRef<PyInt>>,
    pub float: UnaryFunc<PyRef<PyFloat>>,

    pub inplace_add: BinaryFunc,
    pub inplace_subtract: BinaryFunc,
    pub inplace_multiply: BinaryFunc,
    pub inplace_remainder: BinaryFunc,
    pub inplace_divmod: BinaryFunc,
    pub inplace_power: BinaryFunc,
    pub inplace_lshift: BinaryFunc,
    pub inplace_rshift: BinaryFunc,
    pub inplace_and: BinaryFunc,
    pub inplace_xor: BinaryFunc,
    pub inplace_or: BinaryFunc,

    pub floor_divide: BinaryFunc,
    pub true_divide: BinaryFunc,
    pub inplace_floor_divide: BinaryFunc,
    pub inplace_true_divide: BinaryFunc,

    pub index: UnaryFunc<PyRef<PyInt>>,

    pub matrix_multiply: BinaryFunc,
    pub inplace_matrix_multiply: BinaryFunc,
}

impl PyNumberMethods {
    /// this is NOT a global variable
    // TODO: weak order read for performance
    #[allow(clippy::declare_interior_mutable_const)]
    pub const NOT_IMPLEMENTED: PyNumberMethods = PyNumberMethods {
        add: AtomicCell::new(None),
        subtract: AtomicCell::new(None),
        multiply: AtomicCell::new(None),
        remainder: AtomicCell::new(None),
        divmod: AtomicCell::new(None),
        power: AtomicCell::new(None),
        negative: AtomicCell::new(None),
        positive: AtomicCell::new(None),
        absolute: AtomicCell::new(None),
        boolean: AtomicCell::new(None),
        invert: AtomicCell::new(None),
        lshift: AtomicCell::new(None),
        rshift: AtomicCell::new(None),
        and: AtomicCell::new(None),
        xor: AtomicCell::new(None),
        or: AtomicCell::new(None),
        int: AtomicCell::new(None),
        float: AtomicCell::new(None),
        inplace_add: AtomicCell::new(None),
        inplace_subtract: AtomicCell::new(None),
        inplace_multiply: AtomicCell::new(None),
        inplace_remainder: AtomicCell::new(None),
        inplace_divmod: AtomicCell::new(None),
        inplace_power: AtomicCell::new(None),
        inplace_lshift: AtomicCell::new(None),
        inplace_rshift: AtomicCell::new(None),
        inplace_and: AtomicCell::new(None),
        inplace_xor: AtomicCell::new(None),
        inplace_or: AtomicCell::new(None),
        floor_divide: AtomicCell::new(None),
        true_divide: AtomicCell::new(None),
        inplace_floor_divide: AtomicCell::new(None),
        inplace_true_divide: AtomicCell::new(None),
        index: AtomicCell::new(None),
        matrix_multiply: AtomicCell::new(None),
        inplace_matrix_multiply: AtomicCell::new(None),
    };
}

pub enum PyNumberMethodsOffset {
    Add,
    Subtract,
    Multiply,
    Remainder,
    Divmod,
    Power,
    Negative,
    Positive,
    Absolute,
    Boolean,
    Invert,
    Lshift,
    Rshift,
    And,
    Xor,
    Or,
    Int,
    Float,
    InplaceAdd,
    InplaceSubtract,
    InplaceMultiply,
    InplaceRemainder,
    InplaceDivmod,
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
    Index,
    MatrixMultiply,
    InplaceMatrixMultiply,
}

impl PyNumberMethodsOffset {
    pub fn method(&self, methods: &PyNumberMethods, vm: &VirtualMachine) -> PyResult<&BinaryFunc> {
        use PyNumberMethodsOffset::*;
        unsafe {
            match self {
                // BinaryFunc
                Add => ptr::addr_of!(methods.add),
                Subtract => ptr::addr_of!(methods.subtract),
                Multiply => ptr::addr_of!(methods.multiply),
                Remainder => ptr::addr_of!(methods.remainder),
                Divmod => ptr::addr_of!(methods.divmod),
                Power => ptr::addr_of!(methods.power),
                Lshift => ptr::addr_of!(methods.lshift),
                Rshift => ptr::addr_of!(methods.rshift),
                And => ptr::addr_of!(methods.and),
                Xor => ptr::addr_of!(methods.xor),
                Or => ptr::addr_of!(methods.or),
                InplaceAdd => ptr::addr_of!(methods.inplace_add),
                InplaceSubtract => ptr::addr_of!(methods.inplace_subtract),
                InplaceMultiply => ptr::addr_of!(methods.inplace_multiply),
                InplaceRemainder => ptr::addr_of!(methods.inplace_remainder),
                InplaceDivmod => ptr::addr_of!(methods.inplace_divmod),
                InplacePower => ptr::addr_of!(methods.inplace_power),
                InplaceLshift => ptr::addr_of!(methods.inplace_lshift),
                InplaceRshift => ptr::addr_of!(methods.inplace_rshift),
                InplaceAnd => ptr::addr_of!(methods.inplace_and),
                InplaceXor => ptr::addr_of!(methods.inplace_xor),
                InplaceOr => ptr::addr_of!(methods.inplace_or),
                FloorDivide => ptr::addr_of!(methods.floor_divide),
                TrueDivide => ptr::addr_of!(methods.true_divide),
                InplaceFloorDivide => ptr::addr_of!(methods.inplace_floor_divide),
                InplaceTrueDivide => ptr::addr_of!(methods.inplace_true_divide),
                MatrixMultiply => ptr::addr_of!(methods.matrix_multiply),
                InplaceMatrixMultiply => ptr::addr_of!(methods.inplace_matrix_multiply),
                // UnaryFunc
                Negative => ptr::null(),
                Positive => ptr::null(),
                Absolute => ptr::null(),
                Boolean => ptr::null(),
                Invert => ptr::null(),
                Int => ptr::null(),
                Float => ptr::null(),
                Index => ptr::null(),
            }
            .as_ref()
            .ok_or_else(|| {
                vm.new_value_error("No unaryop supported for PyNumberMethodsOffset".to_owned())
            })
        }
    }
}

#[derive(Copy, Clone)]
pub struct PyNumber<'a> {
    pub obj: &'a PyObject,
    methods: &'a PyNumberMethods,
}

impl<'a> From<&'a PyObject> for PyNumber<'a> {
    fn from(obj: &'a PyObject) -> Self {
        static GLOBAL_NOT_IMPLEMENTED: PyNumberMethods = PyNumberMethods::NOT_IMPLEMENTED;
        Self {
            obj,
            methods: Self::find_methods(obj)
                .map_or(&GLOBAL_NOT_IMPLEMENTED, |m| unsafe { m.borrow_static() }),
        }
    }
}

impl PyNumber<'_> {
    fn find_methods(obj: &PyObject) -> Option<PointerSlot<PyNumberMethods>> {
        obj.class().mro_find_map(|x| x.slots.as_number.load())
    }

    pub fn methods<'a>(
        &'a self,
        op_slot: &'a PyNumberMethodsOffset,
        vm: &VirtualMachine,
    ) -> PyResult<&BinaryFunc> {
        op_slot.method(self.methods, vm)
    }

    // PyNumber_Check
    pub fn check(obj: &PyObject) -> bool {
        let Some(methods) = Self::find_methods(obj) else {
            return false;
        };
        let methods = methods.as_ref();
        methods.int.load().is_some()
            || methods.index.load().is_some()
            || methods.float.load().is_some()
            || obj.payload_is::<PyComplex>()
    }

    // PyIndex_Check
    pub fn is_index(&self) -> bool {
        self.methods.index.load().is_some()
    }

    #[inline]
    pub fn int(self, vm: &VirtualMachine) -> Option<PyResult<PyIntRef>> {
        self.methods.int.load().map(|f| {
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
        self.methods.index.load().map(|f| {
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
        self.methods.float.load().map(|f| {
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
