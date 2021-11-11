use std::borrow::Cow;

use crate::{
    builtins::{int, PyByteArray, PyBytes, PyComplex, PyFloat, PyInt, PyIntRef, PyStr},
    common::{lock::OnceCell, static_cell},
    function::ArgBytesLike,
    IdProtocol, PyObject, PyRef, PyResult, PyValue, TryFromBorrowedObject, TypeProtocol,
    VirtualMachine,
};

#[allow(clippy::type_complexity)]
#[derive(Default, Clone)]
pub struct PyNumberMethods {
    /* Number implementations must check *both*
    arguments for proper type and implement the necessary conversions
    in the slot functions themselves. */
    pub add: Option<fn(&PyNumber, &PyObject, vm: &VirtualMachine) -> PyResult>,
    pub subtract: Option<fn(&PyNumber, &PyObject, vm: &VirtualMachine) -> PyResult>,
    pub multiply: Option<fn(&PyNumber, &PyObject, vm: &VirtualMachine) -> PyResult>,
    pub remainder: Option<fn(&PyNumber, &PyObject, vm: &VirtualMachine) -> PyResult>,
    pub divmod: Option<fn(&PyNumber, &PyObject, vm: &VirtualMachine) -> PyResult>,
    pub power: Option<fn(&PyNumber, &PyObject, vm: &VirtualMachine) -> PyResult>,
    pub negative: Option<fn(&PyNumber, vm: &VirtualMachine) -> PyResult>,
    pub positive: Option<fn(&PyNumber, vm: &VirtualMachine) -> PyResult>,
    pub absolute: Option<fn(&PyNumber, vm: &VirtualMachine) -> PyResult>,
    pub boolean: Option<fn(&PyNumber, vm: &VirtualMachine) -> PyResult<bool>>,
    pub invert: Option<fn(&PyNumber, vm: &VirtualMachine) -> PyResult>,
    pub lshift: Option<fn(&PyNumber, &PyObject, vm: &VirtualMachine) -> PyResult>,
    pub rshift: Option<fn(&PyNumber, &PyObject, vm: &VirtualMachine) -> PyResult>,
    pub and: Option<fn(&PyNumber, &PyObject, vm: &VirtualMachine) -> PyResult>,
    pub xor: Option<fn(&PyNumber, &PyObject, vm: &VirtualMachine) -> PyResult>,
    pub or: Option<fn(&PyNumber, &PyObject, vm: &VirtualMachine) -> PyResult>,
    pub int: Option<fn(&PyNumber, vm: &VirtualMachine) -> PyResult<PyIntRef>>,
    pub float: Option<fn(&PyNumber, vm: &VirtualMachine) -> PyResult<PyRef<PyFloat>>>,

    pub inplace_add: Option<fn(&PyNumber, &PyObject, vm: &VirtualMachine) -> PyResult>,
    pub inplace_substract: Option<fn(&PyNumber, &PyObject, vm: &VirtualMachine) -> PyResult>,
    pub inplace_multiply: Option<fn(&PyNumber, &PyObject, vm: &VirtualMachine) -> PyResult>,
    pub inplace_remainder: Option<fn(&PyNumber, &PyObject, vm: &VirtualMachine) -> PyResult>,
    pub inplace_divmod: Option<fn(&PyNumber, &PyObject, vm: &VirtualMachine) -> PyResult>,
    pub inplace_power: Option<fn(&PyNumber, &PyObject, vm: &VirtualMachine) -> PyResult>,
    pub inplace_lshift: Option<fn(&PyNumber, &PyObject, vm: &VirtualMachine) -> PyResult>,
    pub inplace_rshift: Option<fn(&PyNumber, &PyObject, vm: &VirtualMachine) -> PyResult>,
    pub inplace_and: Option<fn(&PyNumber, &PyObject, vm: &VirtualMachine) -> PyResult>,
    pub inplace_xor: Option<fn(&PyNumber, &PyObject, vm: &VirtualMachine) -> PyResult>,
    pub inplace_or: Option<fn(&PyNumber, &PyObject, vm: &VirtualMachine) -> PyResult>,

    pub floor_divide: Option<fn(&PyNumber, &PyObject, vm: &VirtualMachine) -> PyResult>,
    pub true_divide: Option<fn(&PyNumber, &PyObject, vm: &VirtualMachine) -> PyResult>,
    pub inplace_floor_divide: Option<fn(&PyNumber, &PyObject, vm: &VirtualMachine) -> PyResult>,
    pub inplace_true_devide: Option<fn(&PyNumber, &PyObject, vm: &VirtualMachine) -> PyResult>,

    pub index: Option<fn(&PyNumber, vm: &VirtualMachine) -> PyResult<PyIntRef>>,

    pub matrix_multiply: Option<fn(&PyNumber, &PyObject, vm: &VirtualMachine) -> PyResult>,
    pub inplace_matrix_multiply: Option<fn(&PyNumber, &PyObject, vm: &VirtualMachine) -> PyResult>,
}

impl PyNumberMethods {
    fn not_implemented() -> &'static Self {
        static_cell! {
            static NOT_IMPLEMENTED: PyNumberMethods;
        }
        NOT_IMPLEMENTED.get_or_init(Self::default)
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

impl<'a> PyNumber<'a> {
    pub fn methods(&'a self, vm: &VirtualMachine) -> &'a Cow<'static, PyNumberMethods> {
        self.methods.get_or_init(|| {
            self.obj
                .class()
                .mro_find_map(|x| x.slots.as_number.load())
                .map(|f| f(self.obj, vm))
                .unwrap_or_else(|| Cow::Borrowed(PyNumberMethods::not_implemented()))
        })
    }
}

impl PyNumber<'_> {
    // PyNumber_Check
    pub fn is_numeric(&self, vm: &VirtualMachine) -> bool {
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

    pub fn to_int(&self, vm: &VirtualMachine) -> PyResult<PyIntRef> {
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
            PyNumber::from(r.as_ref()).to_index(vm)
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

    pub fn to_index(&self, vm: &VirtualMachine) -> PyResult<PyIntRef> {
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
