use crate::function::{FunctionBox, OptionalArg, PyFuncArgs, PyNativeFunc};
use crate::pyobject::{IdProtocol, PyObjectRef, PyRef, PyResult, PyValue, TryFromObject};
use crate::VirtualMachine;

bitflags! {
    pub struct PyTpFlags: u64 {
        const BASETYPE = 1 << 10;
    }
}

impl PyTpFlags {
    // CPython default: Py_TPFLAGS_HAVE_STACKLESS_EXTENSION | Py_TPFLAGS_HAVE_VERSION_TAG
    pub const DEFAULT: Self = Self::from_bits_truncate(0);

    pub fn has_feature(self, flag: Self) -> bool {
        self.contains(flag)
    }
}

impl Default for PyTpFlags {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[derive(Default)]
pub struct PyClassSlots {
    pub flags: PyTpFlags,
    pub new: Option<PyNativeFunc>,
    pub call: Option<PyNativeFunc>,
    pub descr_get: Option<PyDescrGetFunc>,
}

impl std::fmt::Debug for PyClassSlots {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PyClassSlots")
    }
}

#[pyimpl]
pub trait SlotCall: PyValue {
    #[pymethod(magic)]
    #[pyslot]
    fn call(&self, args: PyFuncArgs, vm: &VirtualMachine) -> PyResult;
}

pub type PyDescrGetFunc = FunctionBox<
    dyn Fn(&VirtualMachine, PyObjectRef, Option<PyObjectRef>, OptionalArg<PyObjectRef>) -> PyResult
        + Send
        + Sync,
>;

#[pyimpl]
pub trait SlotDescriptor: PyValue {
    #[pyslot]
    fn descr_get(
        vm: &VirtualMachine,
        zelf: PyObjectRef,
        obj: Option<PyObjectRef>,
        cls: OptionalArg<PyObjectRef>,
    ) -> PyResult;

    #[pymethod(magic)]
    fn get(
        zelf: PyObjectRef,
        obj: PyObjectRef,
        cls: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        Self::descr_get(vm, zelf, Some(obj), cls)
    }

    fn _zelf(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        PyRef::<Self>::try_from_object(vm, zelf)
    }

    fn _unwrap(
        zelf: PyObjectRef,
        obj: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<(PyRef<Self>, PyObjectRef)> {
        let zelf = Self::_zelf(zelf, vm)?;
        let obj = obj.unwrap_or_else(|| vm.get_none());
        Ok((zelf, obj))
    }

    fn _check(
        zelf: PyObjectRef,
        obj: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> Result<(PyRef<Self>, PyObjectRef), PyResult> {
        // CPython descr_check
        if let Some(obj) = obj {
            // if (!PyObject_TypeCheck(obj, descr->d_type)) {
            //     PyErr_Format(PyExc_TypeError,
            //                  "descriptor '%V' for '%.100s' objects "
            //                  "doesn't apply to a '%.100s' object",
            //                  descr_name((PyDescrObject *)descr), "?",
            //                  descr->d_type->tp_name,
            //                  obj->ob_type->tp_name);
            //     *pres = NULL;
            //     return 1;
            // } else {
            Ok((Self::_zelf(zelf, vm).unwrap(), obj))
        // }
        } else {
            Err(Ok(zelf))
        }
    }

    fn _cls_is<T>(cls: &OptionalArg<PyObjectRef>, other: &T) -> bool
    where
        T: IdProtocol,
    {
        match cls {
            OptionalArg::Present(cls) => cls.is(other),
            OptionalArg::Missing => false,
        }
    }
}
