#![allow(non_snake_case)]

use std::cell::{Ref, RefCell};

use super::os;
use crate::function::OptionalArg;
use crate::obj::objstr::PyStringRef;
use crate::obj::objtype::PyClassRef;
use crate::pyobject::{PyClassImpl, PyObjectRef, PyRef, PyResult, PyValue, TryFromObject};
use crate::VirtualMachine;

use winreg::RegKey;

#[pyclass]
#[derive(Debug)]
struct PyHKEY {
    key: RefCell<RegKey>,
}
type PyHKEYRef = PyRef<PyHKEY>;

impl PyValue for PyHKEY {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.class("winreg", "HKEYType")
    }
}

#[pyimpl]
impl PyHKEY {
    fn new(key: RegKey) -> Self {
        Self {
            key: RefCell::new(key),
        }
    }

    fn key(&self) -> Ref<RegKey> {
        self.key.borrow()
    }

    #[pymethod]
    fn Close(&self) {
        let null_key = RegKey::predef(0 as winreg::HKEY);
        let key = self.key.replace(null_key);
        drop(key);
    }
    #[pymethod]
    fn Detach(&self) -> usize {
        let null_key = RegKey::predef(0 as winreg::HKEY);
        let key = self.key.replace(null_key);
        let handle = key.raw_handle();
        std::mem::forget(key);
        handle as usize
    }

    #[pymethod(magic)]
    fn bool(&self) -> bool {
        !self.key().raw_handle().is_null()
    }
    #[pymethod(magic)]
    fn enter(zelf: PyRef<Self>) -> PyRef<Self> {
        zelf
    }
    #[pymethod(magic)]
    fn exit(&self, _cls: PyObjectRef, _exc: PyObjectRef, _tb: PyObjectRef) {
        self.Close();
    }
}

enum Hkey {
    PyHKEY(PyHKEYRef),
    Constant(winreg::HKEY),
}
impl TryFromObject for Hkey {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        obj.downcast()
            .map(Self::PyHKEY)
            .or_else(|o| usize::try_from_object(vm, o).map(|i| Self::Constant(i as winreg::HKEY)))
    }
}
impl Hkey {
    fn with_key<R>(&self, f: impl FnOnce(&RegKey) -> R) -> R {
        match self {
            Self::PyHKEY(py) => f(&py.key()),
            Self::Constant(hkey) => {
                let k = RegKey::predef(*hkey);
                let res = f(&k);
                std::mem::forget(k);
                res
            }
        }
    }
}

fn winreg_OpenKey(
    key: Hkey,
    subkey: Option<PyStringRef>,
    reserved: OptionalArg<i32>,
    access: OptionalArg<u32>,
    vm: &VirtualMachine,
) -> PyResult<PyHKEY> {
    let reserved = reserved.unwrap_or(0);
    let access = access.unwrap_or(winreg::enums::KEY_READ);
    if reserved != 0 {
        // RegKey::open_subkey* doesn't have a reserved param, so this'll do
        return Err(vm.new_value_error("reserved param must be 0".to_owned()));
    }

    let subkey = subkey.as_ref().map_or("", |s| s.as_str());
    let key = key
        .with_key(|k| k.open_subkey_with_flags(subkey, access))
        .map_err(|e| os::convert_io_error(vm, e))?;

    Ok(PyHKEY::new(key))
}

fn winreg_QueryValue(
    key: Hkey,
    subkey: Option<PyStringRef>,
    vm: &VirtualMachine,
) -> PyResult<String> {
    let subkey = subkey.as_ref().map_or("", |s| s.as_str());
    key.with_key(|k| k.get_value(subkey))
        .map_err(|e| os::convert_io_error(vm, e))
}

fn winreg_QueryValueEx(
    key: Hkey,
    subkey: Option<PyStringRef>,
    vm: &VirtualMachine,
) -> PyResult<(usize, Vec<u8>)> {
    let subkey = subkey.as_ref().map_or("", |s| s.as_str());
    key.with_key(|k| k.get_raw_value(subkey))
        .map(|regval| (regval.vtype as usize, regval.bytes))
        .map_err(|e| os::convert_io_error(vm, e))
}

fn winreg_CloseKey(key: PyHKEYRef) {
    key.Close();
}

pub fn make_module(vm: &VirtualMachine) -> PyObjectRef {
    let ctx = &vm.ctx;
    let hkey_type = PyHKEY::make_class(ctx);
    let module = py_module!(vm, "winreg", {
        "HKEYType" => hkey_type,
        "OpenKey" => ctx.new_function(winreg_OpenKey),
        "QueryValue" => ctx.new_function(winreg_QueryValue),
        "QueryValueEx" => ctx.new_function(winreg_QueryValueEx),
        "CloseKey" => ctx.new_function(winreg_CloseKey),
    });

    macro_rules! add_hkey_constants {
        ($($name:ident),*) => {
            extend_module!(vm, module, {
                $((stringify!($name)) => ctx.new_int(winreg::enums::$name as usize)),*
            })
        };
    }

    add_hkey_constants!(
        HKEY_CLASSES_ROOT,
        HKEY_CURRENT_USER,
        HKEY_LOCAL_MACHINE,
        HKEY_USERS,
        HKEY_PERFORMANCE_DATA,
        HKEY_CURRENT_CONFIG,
        HKEY_DYN_DATA
    );

    module
}
