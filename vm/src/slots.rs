use std::cmp::Ordering;

use crate::common::cell::PyRwLock;
use crate::common::hash::PyHash;
use crate::function::{OptionalArg, PyFuncArgs, PyNativeFunc};
use crate::pyobject::{
    IdProtocol, PyComparisonValue, PyObjectRef, PyRef, PyResult, PyValue, TryFromObject,
};
use crate::VirtualMachine;

bitflags! {
    pub struct PyTpFlags: u64 {
        const HEAPTYPE = 1 << 9;
        const BASETYPE = 1 << 10;
        const HAS_DICT = 1 << 40;

        #[cfg(debug_assertions)]
        const _CREATED_WITH_FLAGS = 1 << 63;
    }
}

impl PyTpFlags {
    // CPython default: Py_TPFLAGS_HAVE_STACKLESS_EXTENSION | Py_TPFLAGS_HAVE_VERSION_TAG
    pub const DEFAULT: Self = Self::HEAPTYPE;

    pub fn has_feature(self, flag: Self) -> bool {
        self.contains(flag)
    }

    #[cfg(debug_assertions)]
    pub fn is_created_with_flags(self) -> bool {
        self.contains(Self::_CREATED_WITH_FLAGS)
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
    pub name: PyRwLock<Option<String>>, // tp_name, not class name
    pub new: Option<PyNativeFunc>,
    pub call: Option<PyNativeFunc>,
    pub descr_get: Option<PyDescrGetFunc>,
    pub hash: Option<HashFunc>,
    pub cmp: Option<CmpFunc>,
}

type HashFunc = Box<py_dyn_fn!(dyn Fn(PyObjectRef, &VirtualMachine) -> PyResult<PyHash>)>;
type CmpFunc = Box<
    py_dyn_fn!(
        dyn Fn(
            PyObjectRef,
            PyObjectRef,
            PyComparisonOp,
            &VirtualMachine,
        ) -> PyResult<PyComparisonValue>
    ),
>;

impl PyClassSlots {
    pub fn from_flags(flags: PyTpFlags) -> Self {
        Self {
            flags,
            ..Default::default()
        }
    }
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

pub type PyDescrGetFunc = Box<
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
        let obj = vm.unwrap_or_none(obj);
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

#[pyimpl]
pub trait Hashable: PyValue {
    #[pyslot]
    fn tp_hash(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyHash> {
        let zelf = PyRef::try_from_object(vm, zelf)?;
        Self::hash(zelf, vm)
    }

    #[pymethod(magic)]
    fn hash(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<PyHash>;
}

#[pyimpl]
pub trait Comparable: PyValue {
    #[pyslot]
    fn tp_cmp(
        zelf: PyObjectRef,
        other: PyObjectRef,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        let zelf = PyRef::try_from_object(vm, zelf)?;
        Self::cmp(zelf, other, op, vm)
    }

    fn cmp(
        zelf: PyRef<Self>,
        other: PyObjectRef,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue>;

    #[pymethod(magic)]
    fn eq(
        zelf: PyRef<Self>,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        Self::cmp(zelf, other, PyComparisonOp::Eq, vm)
    }
    #[pymethod(magic)]
    fn ne(
        zelf: PyRef<Self>,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        Self::cmp(zelf, other, PyComparisonOp::Ne, vm)
    }
    #[pymethod(magic)]
    fn lt(
        zelf: PyRef<Self>,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        Self::cmp(zelf, other, PyComparisonOp::Lt, vm)
    }
    #[pymethod(magic)]
    fn le(
        zelf: PyRef<Self>,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        Self::cmp(zelf, other, PyComparisonOp::Le, vm)
    }
    #[pymethod(magic)]
    fn ge(
        zelf: PyRef<Self>,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        Self::cmp(zelf, other, PyComparisonOp::Ge, vm)
    }
    #[pymethod(magic)]
    fn gt(
        zelf: PyRef<Self>,
        other: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        Self::cmp(zelf, other, PyComparisonOp::Gt, vm)
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum PyComparisonOp {
    Lt,
    Le,
    Eq,
    Ne,
    Ge,
    Gt,
}

use PyComparisonOp::*;
impl PyComparisonOp {
    pub fn eq_only(
        self,
        f: impl FnOnce() -> PyResult<PyComparisonValue>,
    ) -> PyResult<PyComparisonValue> {
        match self {
            Self::Eq => f(),
            Self::Ne => f().map(|x| x.map(|eq| !eq)),
            _ => Ok(PyComparisonValue::NotImplemented),
        }
    }

    pub fn eval_ord(self, ord: Ordering) -> bool {
        match ord {
            Ordering::Less => matches!(self, Lt | Le | Ne),
            Ordering::Equal => matches!(self, Le | Eq | Ge),
            Ordering::Greater => matches!(self, Ne | Ge | Gt),
        }
    }

    pub fn swapped(self) -> Self {
        match self {
            Lt => Gt,
            Le => Ge,
            Eq => Eq,
            Ne => Ne,
            Ge => Le,
            Gt => Lt,
        }
    }

    pub fn method_name(self) -> &'static str {
        match self {
            Lt => "__lt__",
            Le => "__le__",
            Eq => "__eq__",
            Ne => "__ne__",
            Ge => "__ge__",
            Gt => "__gt__",
        }
    }

    pub fn operator_token(self) -> &'static str {
        match self {
            Lt => "<",
            Le => "<=",
            Eq => "==",
            Ne => "!=",
            Ge => ">=",
            Gt => ">",
        }
    }

    /// Returns an appropriate return value for the comparison when a and b are the same object, if an
    /// appropriate return value exists.
    pub fn identical_optimization(self, a: &impl IdProtocol, b: &impl IdProtocol) -> Option<bool> {
        self.map_eq(|| a.is(b))
    }

    /// Returns `Some(true)` when self is `Eq` and `f()` returns true. Returns `Some(false)` when self
    /// is `Ne` and `f()` returns true. Otherwise returns `None`.
    pub fn map_eq(self, f: impl FnOnce() -> bool) -> Option<bool> {
        match self {
            Self::Eq => {
                if f() {
                    Some(true)
                } else {
                    None
                }
            }
            Self::Ne => {
                if f() {
                    None
                } else {
                    Some(false)
                }
            }
            _ => None,
        }
    }
}
