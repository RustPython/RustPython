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

type UnaryFunc<R = PyObjectRef> = AtomicCell<Option<fn(&PyNumber, &VirtualMachine) -> PyResult<R>>>;
type BinaryFunc<R = PyObjectRef> =
    AtomicCell<Option<fn(&PyNumber, &PyObject, &VirtualMachine) -> PyResult<R>>>;

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
        methods.int.load().is_some()
            || methods.index.load().is_some()
            || methods.float.load().is_some()
            || obj.payload_is::<PyComplex>()
    }

    // PyIndex_Check
    pub fn is_index(&self) -> bool {
        self.methods().index.load().is_some()
    }

    #[inline]
    pub fn int(&self, vm: &VirtualMachine) -> PyResult<Option<PyIntRef>> {
        Ok(if let Some(f) = self.methods().int.load() {
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
        if let Some(f) = self.methods().index.load() {
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
        Ok(if let Some(f) = self.methods().float.load() {
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
