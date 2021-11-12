use std::borrow::Cow;

use crate::{
    builtins::{int, PyByteArray, PyBytes, PyComplex, PyFloat, PyInt, PyIntRef, PyStr},
    common::lock::OnceCell,
    function::ArgBytesLike,
    AsObject, PyObject, PyPayload, PyRef, PyResult, TryFromBorrowedObject, VirtualMachine,
};

#[allow(clippy::type_complexity)]
#[derive(Clone)]
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
    pub inplace_substract: Option<fn(&PyNumber, &PyObject, &VirtualMachine) -> PyResult>,
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
    pub inplace_true_devide: Option<fn(&PyNumber, &PyObject, &VirtualMachine) -> PyResult>,

    pub index: Option<fn(&PyNumber, &VirtualMachine) -> PyResult<PyIntRef>>,

    pub matrix_multiply: Option<fn(&PyNumber, &PyObject, &VirtualMachine) -> PyResult>,
    pub inplace_matrix_multiply: Option<fn(&PyNumber, &PyObject, &VirtualMachine) -> PyResult>,
}

impl PyNumberMethods {
    pub const fn not_implemented() -> &'static Self {
        &NOT_IMPLEMENTED
    }
}

pub struct PyNumber<'a> {
    pub obj: &'a PyObject,
    // some fast path do not need methods, so we do lazy initialize
    methods: OnceCell<Cow<'static, PyNumberMethods>>,
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
    pub fn methods(&self, vm: &VirtualMachine) -> &PyNumberMethods {
        &*self.methods_cow(vm)
    }

    pub fn methods_cow(&self, vm: &VirtualMachine) -> &Cow<'static, PyNumberMethods> {
        self.methods.get_or_init(|| {
            self.obj
                .class()
                .mro_find_map(|x| x.slots.as_number.load())
                .map(|f| f(self.obj, vm))
                .unwrap_or_else(|| Cow::Borrowed(PyNumberMethods::not_implemented()))
        })
    }

    // PyNumber_Check
    pub fn check(&self, vm: &VirtualMachine) -> bool {
        let methods = self.methods(vm);
        methods.int.is_some()
            || methods.index.is_some()
            || methods.float.is_some()
            || self.obj.payload_is::<PyComplex>()
    }

    // PyIndex_Check
    pub fn is_index(&self, vm: &VirtualMachine) -> bool {
        self.methods(vm).index.is_some()
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

        if self.obj.class().is(PyInt::class(vm)) {
            Ok(unsafe { self.obj.downcast_unchecked_ref::<PyInt>() }.to_owned())
        } else if let Some(f) = self.methods(vm).int {
            f(self, vm)
        } else if let Some(f) = self.methods(vm).index {
            f(self, vm)
        } else if let Ok(Ok(f)) = vm.get_special_method(self.obj.to_owned(), "__trunc__") {
            let r = f.invoke((), vm)?;
            PyNumber::from(r.as_ref()).index(vm).map_err(|_| {
                vm.new_type_error("__trunc__ returned non-Integral (type NonIntegral)".to_string())
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

    pub fn index(&self, vm: &VirtualMachine) -> PyResult<PyIntRef> {
        if self.obj.class().is(PyInt::class(vm)) {
            Ok(unsafe { self.obj.downcast_unchecked_ref::<PyInt>() }.to_owned())
        } else if let Some(f) = self.methods(vm).index {
            f(self, vm)
        } else {
            Err(vm.new_type_error(format!(
                "'{}' object cannot be interpreted as an integer",
                self.obj.class()
            )))
        }
    }
}

const NOT_IMPLEMENTED: PyNumberMethods = PyNumberMethods {
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
    inplace_substract: None,
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
    inplace_true_devide: None,
    index: None,
    matrix_multiply: None,
    inplace_matrix_multiply: None,
};
