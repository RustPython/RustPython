use std::ptr::NonNull;

use crossbeam_utils::atomic::AtomicCell;
use once_cell::sync::OnceCell;

use crate::{
    builtins::{int, PyByteArray, PyBytes, PyComplex, PyFloat, PyInt, PyIntRef, PyStr},
    function::ArgBytesLike,
    stdlib::warnings,
    AsObject, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, TryFromBorrowedObject,
    VirtualMachine,
};

type UnaryFunc<R = PyObjectRef> = AtomicCell<Option<fn(&PyNumber, &VirtualMachine) -> PyResult<R>>>;
type BinaryFunc<R = PyObjectRef> =
    AtomicCell<Option<fn(&PyNumber, &PyObject, &VirtualMachine) -> PyResult<R>>>;

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
    // some fast path do not need methods, so we do lazy initialize
    methods: OnceCell<NonNull<PyNumberMethods>>,
}

impl<'a> From<&'a PyObject> for PyNumber<'a> {
    fn from(obj: &'a PyObject) -> Self {
        Self {
            obj,
            methods: OnceCell::new(),
        }
    }
}

impl PyNumber<'_> {
    pub fn methods(&self) -> &PyNumberMethods {
        static GLOBAL_NOT_IMPLEMENTED: PyNumberMethods = PyNumberMethods::NOT_IMPLEMENTED;
        let as_number = self.methods.get_or_init(|| {
            Self::find_methods(self.obj).unwrap_or_else(|| NonNull::from(&GLOBAL_NOT_IMPLEMENTED))
        });
        unsafe { as_number.as_ref() }
    }

    fn find_methods(obj: &PyObject) -> Option<NonNull<PyNumberMethods>> {
        obj.class().mro_find_map(|x| x.slots.as_number.load())
    }

    // PyNumber_Check
    pub fn check(obj: &PyObject) -> bool {
        let num = PyNumber::from(obj);
        let methods = num.methods();
        methods.int.load().is_some()
            || methods.index.load().is_some()
            || methods.float.load().is_some()
            || obj.payload_is::<PyComplex>()
    }

    // PyIndex_Check
    pub fn is_index(&self) -> bool {
        self.methods().index.load().is_some()
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
        } else if let Some(f) = self.methods().int.load() {
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
        } else if self.methods().index.load().is_some() {
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
            PyNumber::from(ret.as_ref()).index(vm).map_err(|_| {
                vm.new_type_error(format!(
                    "__trunc__ returned non-Integral (type {})",
                    ret.class()
                ))
            })
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
        } else if let Some(f) = self.methods().index.load() {
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
        } else if let Some(f) = self.methods().float.load() {
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
        } else if self.methods().index.load().is_some() {
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
}
