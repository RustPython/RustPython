use crate::{
    builtins::{int, PyByteArray, PyBytes, PyComplex, PyFloat, PyInt, PyIntRef, PyStr},
    common::borrow::BorrowedValue,
    function::ArgBytesLike,
    stdlib::warnings,
    AsObject, PyObject, PyPayload, PyRef, PyResult, TryFromBorrowedObject, VirtualMachine,
};

#[allow(clippy::type_complexity)]
#[derive(Default, Clone)]
pub struct PyNumberMethods {
    /* Number implementations must check *both*
    arguments for proper type and implement the necessary conversions
    in the slot functions themselves. */
    pub add: Option<fn(&PyNumber, &PyObject, &VirtualMachine) -> PyResult>,
    pub subtract: Option<fn(&PyNumber, &PyObject, &VirtualMachine) -> PyResult>,
    pub multiply: Option<fn(&PyNumber, &PyObject, &VirtualMachine) -> PyResult>,
    pub remainder: Option<fn(&PyNumber, &PyObject, &VirtualMachine) -> PyResult>,
    pub divmod: Option<fn(&PyNumber, &PyObject, &VirtualMachine) -> PyResult>,
    pub power: Option<fn(&PyNumber, &PyObject, &VirtualMachine) -> PyResult>,
    pub negative: Option<fn(&PyNumber, &VirtualMachine) -> PyResult>,
    pub positive: Option<fn(&PyNumber, &VirtualMachine) -> PyResult>,
    pub absolute: Option<fn(&PyNumber, &VirtualMachine) -> PyResult>,
    pub boolean: Option<fn(&PyNumber, &VirtualMachine) -> PyResult<bool>>,
    pub invert: Option<fn(&PyNumber, &VirtualMachine) -> PyResult>,
    pub lshift: Option<fn(&PyNumber, &PyObject, &VirtualMachine) -> PyResult>,
    pub rshift: Option<fn(&PyNumber, &PyObject, &VirtualMachine) -> PyResult>,
    pub and: Option<fn(&PyNumber, &PyObject, &VirtualMachine) -> PyResult>,
    pub xor: Option<fn(&PyNumber, &PyObject, &VirtualMachine) -> PyResult>,
    pub or: Option<fn(&PyNumber, &PyObject, &VirtualMachine) -> PyResult>,
    pub int: Option<fn(&PyNumber, &VirtualMachine) -> PyResult<PyIntRef>>,
    pub float: Option<fn(&PyNumber, &VirtualMachine) -> PyResult<PyRef<PyFloat>>>,

    pub inplace_add: Option<fn(&PyNumber, &PyObject, &VirtualMachine) -> PyResult>,
    pub inplace_subtract: Option<fn(&PyNumber, &PyObject, &VirtualMachine) -> PyResult>,
    pub inplace_multiply: Option<fn(&PyNumber, &PyObject, &VirtualMachine) -> PyResult>,
    pub inplace_remainder: Option<fn(&PyNumber, &PyObject, &VirtualMachine) -> PyResult>,
    pub inplace_divmod: Option<fn(&PyNumber, &PyObject, &VirtualMachine) -> PyResult>,
    pub inplace_power: Option<fn(&PyNumber, &PyObject, &VirtualMachine) -> PyResult>,
    pub inplace_lshift: Option<fn(&PyNumber, &PyObject, &VirtualMachine) -> PyResult>,
    pub inplace_rshift: Option<fn(&PyNumber, &PyObject, &VirtualMachine) -> PyResult>,
    pub inplace_and: Option<fn(&PyNumber, &PyObject, &VirtualMachine) -> PyResult>,
    pub inplace_xor: Option<fn(&PyNumber, &PyObject, &VirtualMachine) -> PyResult>,
    pub inplace_or: Option<fn(&PyNumber, &PyObject, &VirtualMachine) -> PyResult>,

    pub floor_divide: Option<fn(&PyNumber, &PyObject, &VirtualMachine) -> PyResult>,
    pub true_divide: Option<fn(&PyNumber, &PyObject, &VirtualMachine) -> PyResult>,
    pub inplace_floor_divide: Option<fn(&PyNumber, &PyObject, &VirtualMachine) -> PyResult>,
    pub inplace_true_divide: Option<fn(&PyNumber, &PyObject, &VirtualMachine) -> PyResult>,

    pub index: Option<fn(&PyNumber, &VirtualMachine) -> PyResult<PyIntRef>>,

    pub matrix_multiply: Option<fn(&PyNumber, &PyObject, &VirtualMachine) -> PyResult>,
    pub inplace_matrix_multiply: Option<fn(&PyNumber, &PyObject, &VirtualMachine) -> PyResult>,
}

impl PyNumberMethods {
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
        inplace_divmod: None,
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

pub struct PyNumber<'a> {
    pub obj: &'a PyObject,
    pub methods: BorrowedValue<'a, PyNumberMethods>,
}

impl<'a> PyNumber<'a> {
    pub fn new(obj: &'a PyObject, vm: &VirtualMachine) -> Self {
        Self {
            obj,
            methods: Self::find_methods(obj, vm),
        }
    }

    pub fn find_methods(
        obj: &'a PyObject,
        vm: &VirtualMachine,
    ) -> BorrowedValue<'a, PyNumberMethods> {
        obj.class()
            .mro_find_map(|x| x.slots.as_number.load())
            .map(|f| f(obj, vm))
            .unwrap_or(BorrowedValue::Ref(&PyNumberMethods::NOT_IMPLEMENTED))
    }

    // PyNumber_Check
    pub fn check(obj: &'a PyObject, vm: &VirtualMachine) -> bool {
        let methods = Self::find_methods(obj, vm);
        methods.int.is_some()
            || methods.index.is_some()
            || methods.float.is_some()
            || obj.payload_is::<PyComplex>()
    }
}

impl PyNumber<'_> {
    // PyIndex_Check
    pub fn is_index(&self) -> bool {
        self.methods.index.is_some()
    }

    pub fn int(&self, vm: &VirtualMachine) -> PyResult<PyIntRef> {
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

        if let Some(i) = self.obj.downcast_ref_if_exact::<PyInt>(vm) {
            Ok(i.to_owned())
        } else if let Some(f) = self.methods.int {
            let ret = f(self, vm)?;
            if !ret.class().is(PyInt::class(vm)) {
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
                Ok(vm.ctx.new_bigint(ret.as_bigint()))
            } else {
                Ok(ret)
            }
        } else if self.methods.index.is_some() {
            self.index(vm)
        } else if let Ok(Ok(f)) =
            vm.get_special_method(self.obj.to_owned(), identifier!(vm, __trunc__))
        {
            // TODO: Deprecate in 3.11
            // warnings::warn(
            //     vm.ctx.exceptions.deprecation_warning.clone(),
            //     "The delegation of int() to __trunc__ is deprecated.".to_owned(),
            //     1,
            //     vm,
            // )?;
            let ret = f.invoke((), vm)?;
            let ret = PyNumber::new(ret.as_ref(), vm).index(vm).map_err(|_| {
                vm.new_type_error(format!(
                    "__trunc__ returned non-Integral (type {})",
                    ret.class()
                ))
            });
            ret
        } else if let Some(s) = self.obj.payload::<PyStr>() {
            try_convert(self.obj, s.as_str().as_bytes(), vm)
        } else if let Some(bytes) = self.obj.payload::<PyBytes>() {
            try_convert(self.obj, bytes, vm)
        } else if let Some(bytearray) = self.obj.payload::<PyByteArray>() {
            try_convert(self.obj, &bytearray.borrow_buf(), vm)
        } else if let Ok(buffer) = ArgBytesLike::try_from_borrowed_object(vm, self.obj) {
            // TODO: replace to PyBuffer
            try_convert(self.obj, &buffer.borrow_buf(), vm)
        } else {
            Err(vm.new_type_error(format!(
                "int() argument must be a string, a bytes-like object or a real number, not '{}'",
                self.obj.class()
            )))
        }
    }

    pub fn index_opt(&self, vm: &VirtualMachine) -> PyResult<Option<PyIntRef>> {
        if let Some(i) = self.obj.downcast_ref_if_exact::<PyInt>(vm) {
            Ok(Some(i.to_owned()))
        } else if let Some(i) = self.obj.payload::<PyInt>() {
            Ok(Some(vm.ctx.new_bigint(i.as_bigint())))
        } else if let Some(f) = self.methods.index {
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

    pub fn index(&self, vm: &VirtualMachine) -> PyResult<PyIntRef> {
        self.index_opt(vm)?.ok_or_else(|| {
            vm.new_type_error(format!(
                "'{}' object cannot be interpreted as an integer",
                self.obj.class()
            ))
        })
    }

    pub fn float_opt(&self, vm: &VirtualMachine) -> PyResult<Option<PyRef<PyFloat>>> {
        if let Some(float) = self.obj.downcast_ref_if_exact::<PyFloat>(vm) {
            Ok(Some(float.to_owned()))
        } else if let Some(f) = self.methods.float {
            let ret = f(self, vm)?;
            if !ret.class().is(PyFloat::class(vm)) {
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
                Ok(Some(vm.ctx.new_float(ret.to_f64())))
            } else {
                Ok(Some(ret))
            }
        } else if self.methods.index.is_some() {
            let i = self.index(vm)?;
            let value = int::try_to_float(i.as_bigint(), vm)?;
            Ok(Some(vm.ctx.new_float(value)))
        } else if let Some(value) = self.obj.downcast_ref::<PyFloat>() {
            Ok(Some(vm.ctx.new_float(value.to_f64())))
        } else {
            Ok(None)
        }
    }

    pub fn float(&self, vm: &VirtualMachine) -> PyResult<PyRef<PyFloat>> {
        self.float_opt(vm)?.ok_or_else(|| {
            vm.new_type_error(format!("must be real number, not {}", self.obj.class()))
        })
    }

    // pub fn add(&self, other: &PyObject, vm: &VirtualMachine) -> PyResult {
    //     let slotv = self.methods.add;
    //     if !other.class().is(&self.obj.class()) && other.class().fast_issubclass(&self.obj.class()) {
    //         // fallback?
    //         let other = PyNumber::new(other, vm);
    //         let slotw = other.methods.add;
    //         if let Some(slotw) = slotw {
    //             let ret = slotw(self, other.obj, vm)?;
    //             if !ret.is(&vm.ctx.not_implemented) {
    //                 return Ok(ret);
    //             }
    //         }
    //     }
    // }
}
