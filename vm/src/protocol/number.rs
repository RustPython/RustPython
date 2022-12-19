use crate::{
    builtins::{
        int, type_::PointerSlot, PyByteArray, PyBytes, PyComplex, PyFloat, PyInt, PyIntRef, PyStr,
    },
    function::ArgBytesLike,
    stdlib::warnings,
    AsObject, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, TryFromBorrowedObject,
    VirtualMachine,
};
use rustpython_common::atomic::{PyAtomicFn, Ordering};

pub type NumberUnaryFn<R = PyObjectRef> = PyAtomicFn<Option<fn(&PyNumber, &VirtualMachine) -> PyResult<R>>>;
pub type NumberBinaryFn<R = PyObjectRef> =
    PyAtomicFn<Option<fn(&PyNumber, &PyObject, &VirtualMachine) -> PyResult<R>>>;

impl PyObject {
    #[inline]
    pub fn to_number(&self) -> PyNumber<'_> {
        PyNumber::from(self)
    }

    pub fn try_index_opt(&self, vm: &VirtualMachine) -> Option<PyResult<PyIntRef>> {
        #[allow(clippy::question_mark)]
        Some(if let Some(i) = self.downcast_ref_if_exact::<PyInt>(vm) {
            Ok(i.to_owned())
        } else if let Some(i) = self.payload::<PyInt>() {
            Ok(vm.ctx.new_bigint(i.as_bigint()))
        } else if let Some(i) = self.to_number().index(vm).transpose() {
            i
        } else {
            return None;
        })
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
        } else {
            let number = self.to_number();
            if let Some(i) = number.int(vm)? {
                Ok(i)
            } else if let Some(i) = self.try_index_opt(vm) {
                i
            } else if let Ok(Ok(f)) =
                vm.get_special_method(self.to_owned(), identifier!(vm, __trunc__))
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
    }

    pub fn try_float_opt(&self, vm: &VirtualMachine) -> PyResult<Option<PyRef<PyFloat>>> {
        let value = if let Some(float) = self.downcast_ref_if_exact::<PyFloat>(vm) {
            Some(float.to_owned())
        } else {
            let number = self.to_number();
            #[allow(clippy::manual_map)]
            if let Some(f) = number.float(vm)? {
                Some(f)
            } else if let Some(i) = self.try_index_opt(vm) {
                let value = int::try_to_float(i?.as_bigint(), vm)?;
                Some(vm.ctx.new_float(value))
            } else if let Some(value) = self.downcast_ref::<PyFloat>() {
                Some(vm.ctx.new_float(value.to_f64()))
            } else {
                None
            }
        };
        Ok(value)
    }

    #[inline]
    pub fn try_float(&self, vm: &VirtualMachine) -> PyResult<PyRef<PyFloat>> {
        self.try_float_opt(vm)?
            .ok_or_else(|| vm.new_type_error(format!("must be real number, not {}", self.class())))
    }
}

#[derive(Default)]
pub struct PyNumberMethods {
    /* Number implementations must check *both*
    arguments for proper type and implement the necessary conversions
    in the slot functions themselves. */
    pub add: NumberBinaryFn,
    pub subtract: NumberBinaryFn,
    pub multiply: NumberBinaryFn,
    pub remainder: NumberBinaryFn,
    pub divmod: NumberBinaryFn,
    pub power: NumberBinaryFn,
    pub negative: NumberUnaryFn,
    pub positive: NumberUnaryFn,
    pub absolute: NumberUnaryFn,
    pub boolean: NumberUnaryFn<bool>,
    pub invert: NumberUnaryFn,
    pub lshift: NumberBinaryFn,
    pub rshift: NumberBinaryFn,
    pub and: NumberBinaryFn,
    pub xor: NumberBinaryFn,
    pub or: NumberBinaryFn,
    pub int: NumberUnaryFn<PyRef<PyInt>>,
    pub float: NumberUnaryFn<PyRef<PyFloat>>,

    pub inplace_add: NumberBinaryFn,
    pub inplace_subtract: NumberBinaryFn,
    pub inplace_multiply: NumberBinaryFn,
    pub inplace_remainder: NumberBinaryFn,
    pub inplace_divmod: NumberBinaryFn,
    pub inplace_power: NumberBinaryFn,
    pub inplace_lshift: NumberBinaryFn,
    pub inplace_rshift: NumberBinaryFn,
    pub inplace_and: NumberBinaryFn,
    pub inplace_xor: NumberBinaryFn,
    pub inplace_or: NumberBinaryFn,

    pub floor_divide: NumberBinaryFn,
    pub true_divide: NumberBinaryFn,
    pub inplace_floor_divide: NumberBinaryFn,
    pub inplace_true_divide: NumberBinaryFn,

    pub index: NumberUnaryFn<PyRef<PyInt>>,

    pub matrix_multiply: NumberBinaryFn,
    pub inplace_matrix_multiply: NumberBinaryFn,
}

impl PyNumberMethods {
    /// this is NOT a global variable
    // TODO: weak order read for performance
    #[allow(clippy::declare_interior_mutable_const)]
    pub const NOT_IMPLEMENTED: PyNumberMethods = PyNumberMethods {
        add: Default::default(),
        subtract: Default::default(),
        multiply: Default::default(),
        remainder: Default::default(),
        divmod: Default::default(),
        power: Default::default(),
        negative: Default::default(),
        positive: Default::default(),
        absolute: Default::default(),
        boolean: Default::default(),
        invert: Default::default(),
        lshift: Default::default(),
        rshift: Default::default(),
        and: Default::default(),
        xor: Default::default(),
        or: Default::default(),
        int: Default::default(),
        float: Default::default(),
        inplace_add: Default::default(),
        inplace_subtract: Default::default(),
        inplace_multiply: Default::default(),
        inplace_remainder: Default::default(),
        inplace_divmod: Default::default(),
        inplace_power: Default::default(),
        inplace_lshift: Default::default(),
        inplace_rshift: Default::default(),
        inplace_and: Default::default(),
        inplace_xor: Default::default(),
        inplace_or: Default::default(),
        floor_divide: Default::default(),
        true_divide: Default::default(),
        inplace_floor_divide: Default::default(),
        inplace_true_divide: Default::default(),
        index: Default::default(),
        matrix_multiply: Default::default(),
        inplace_matrix_multiply: Default::default(),
    };
}

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

    pub fn methods(&self) -> &PyNumberMethods {
        self.methods
    }

    // PyNumber_Check
    pub fn check(obj: &PyObject) -> bool {
        let Some(methods) = Self::find_methods(obj) else {
            return false;
        };
        let methods = methods.as_ref();
        methods.int.load(Ordering::Relaxed).is_some()
            || methods.index.load(Ordering::Relaxed).is_some()
            || methods.float.load(Ordering::Relaxed).is_some()
            || obj.payload_is::<PyComplex>()
    }

    // PyIndex_Check
    pub fn is_index(&self) -> bool {
        self.methods().index.load(Ordering::Relaxed).is_some()
    }

    #[inline]
    pub fn int(&self, vm: &VirtualMachine) -> PyResult<Option<PyIntRef>> {
        Ok(if let Some(f) = self.methods().int.load(Ordering::Relaxed) {
            let ret = f(self, vm)?;
            Some(if !ret.class().is(PyInt::class(vm)) {
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
            })
        } else {
            None
        })
    }

    #[inline]
    pub fn index(&self, vm: &VirtualMachine) -> PyResult<Option<PyIntRef>> {
        if let Some(f) = self.methods().index.load(Ordering::Relaxed) {
            let ret = f(self, vm)?;
            if !ret.class().is(PyInt::class(vm)) {
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
                Ok(Some(vm.ctx.new_bigint(ret.as_bigint())))
            } else {
                Ok(Some(ret))
            }
        } else {
            Ok(None)
        }
    }

    #[inline]
    pub fn float(&self, vm: &VirtualMachine) -> PyResult<Option<PyRef<PyFloat>>> {
        Ok(if let Some(f) = self.methods().float.load(Ordering::Relaxed) {
            let ret = f(self, vm)?;
            Some(if !ret.class().is(PyFloat::class(vm)) {
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
            })
        } else {
            None
        })
    }
}
