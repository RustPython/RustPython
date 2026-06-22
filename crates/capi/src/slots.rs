use crate::PyObject;
use crate::methodobject::PyMethodDef;
use crate::object::PyTypeSlot;
use core::ffi::{c_char, c_int, c_void};
use rustpython_vm::builtins::PyBaseExceptionRef;
use rustpython_vm::{PyResult, VirtualMachine};

#[repr(C)]
pub struct PySlot {
    pub sl_id: u16,
    pub sl_flags: u16,
    _reserved: u32,
    pub value: PySlotValue,
}

impl PySlot {
    const SLOT_OPTIONAL: u16 = 0x0001;
    const SLOT_STATIC: u16 = 0x0002;
    const SLOT_INTPTR: u16 = 0x0004;

    pub(crate) fn iter<'a>(mut slots: *const Self) -> impl Iterator<Item = &'a Self> {
        core::iter::from_fn(move || {
            let slot = unsafe { &*slots };
            if slot.sl_id == 0 {
                None
            } else {
                slots = unsafe { slots.add(1) };
                Some(slot)
            }
        })
    }

    #[must_use]
    pub fn is_optional(&self) -> bool {
        self.sl_flags & Self::SLOT_OPTIONAL != 0
    }

    #[must_use]
    pub fn is_static(&self) -> bool {
        self.sl_flags & Self::SLOT_STATIC != 0
    }

    #[must_use]
    pub fn is_intptr(&self) -> bool {
        self.sl_flags & Self::SLOT_INTPTR != 0
    }
}

#[repr(C)]
pub union PySlotValue {
    pub sl_ptr: *mut c_void,
    pub sl_func: unsafe extern "C" fn(),
    pub sl_size: isize,
    pub sl_int64: i64,
    pub sl_uint64: u64,
}

#[repr(C)]
pub struct PyABIInfo {
    pub abiinfo_major_version: u8,
    pub abiinfo_minor_version: u8,
    pub flags: u16,
    pub build_version: u32,
    pub abi_version: u32,
}

impl PyABIInfo {
    #[allow(dead_code)]
    #[must_use]
    pub(crate) fn is_supported(&self) -> bool {
        const PY_ABIINFO_STABLE: u16 = 0x0001;
        const PY_ABIINFO_FREETHREADED: u16 = 0x0004;

        if self.abiinfo_major_version != 1 || self.abiinfo_minor_version != 0 {
            return false;
        }

        // Only accept abi3t
        if self.flags & PY_ABIINFO_STABLE == 0 || self.flags & PY_ABIINFO_FREETHREADED == 0 {
            return false;
        }

        true
    }
}

#[allow(dead_code)]
#[derive(Debug)]
pub(crate) enum PySlotKind {
    ModuleCreate(unsafe extern "C" fn(spec: *mut PyObject, def: *mut c_void) -> *mut PyObject),
    ModuleExec(unsafe extern "C" fn(*mut PyObject) -> c_int),
    ModuleName {
        value: *const c_char,
        is_static: bool,
    },
    ModuleDoc {
        value: *const c_char,
        is_static: bool,
    },
    ModuleMethods(*mut PyMethodDef),
    ModuleAbi {
        value: *const PyABIInfo,
        is_static: bool,
    },
    ModuleMultipleInterpreters {
        value: *mut c_void,
        is_static: bool,
    },
    ModuleGil {
        gil_used: bool,
        is_static: bool,
    },
    TypeBase {
        value: *mut PyObject,
        is_static: bool,
    },
    TypeBases {
        value: *mut PyObject,
        is_static: bool,
    },
    TypeToken {
        value: *mut c_void,
        is_static: bool,
    },
    TypeSlots {
        value: *mut PyTypeSlot,
        is_static: bool,
    },
    TypeName {
        value: *const c_char,
        is_static: bool,
    },
    TypeMetaclass {
        value: *mut PyObject,
        is_static: bool,
    },
    TypeModule {
        value: *mut PyObject,
        is_static: bool,
    },
    Unknown {
        id: u16,
        value: *mut c_void,
        is_static: bool,
    },
}

impl PySlotKind {
    #[allow(dead_code)]
    #[must_use]
    pub(crate) fn is_static(&self) -> bool {
        match self {
            Self::ModuleCreate(_) | Self::ModuleExec(_) | Self::ModuleMethods(_) => true,
            Self::ModuleName { is_static, .. }
            | Self::ModuleDoc { is_static, .. }
            | Self::ModuleAbi { is_static, .. }
            | Self::ModuleMultipleInterpreters { is_static, .. }
            | Self::ModuleGil { is_static, .. }
            | Self::TypeBase { is_static, .. }
            | Self::TypeBases { is_static, .. }
            | Self::TypeToken { is_static, .. }
            | Self::TypeSlots { is_static, .. }
            | Self::TypeName { is_static, .. }
            | Self::TypeMetaclass { is_static, .. }
            | Self::TypeModule { is_static, .. }
            | Self::Unknown { is_static, .. } => *is_static,
        }
    }
}

impl TryFrom<(&PySlot, &VirtualMachine)> for PySlotKind {
    type Error = PyBaseExceptionRef;

    fn try_from((slot, vm): (&PySlot, &VirtualMachine)) -> PyResult<Self> {
        let value_ptr = unsafe { slot.value.sl_ptr };
        let is_static = slot.is_static();
        let kind = match slot.sl_id {
            // Type slots
            48 => Self::TypeBase {
                value: value_ptr.cast(),
                is_static,
            },
            49 => Self::TypeBases {
                value: value_ptr.cast(),
                is_static,
            },
            83 => Self::TypeToken {
                value: value_ptr,
                is_static,
            },
            84 => {
                let create = unsafe {
                    core::mem::transmute::<
                        unsafe extern "C" fn(),
                        unsafe extern "C" fn(
                            spec: *mut PyObject,
                            def: *mut c_void,
                        ) -> *mut PyObject,
                    >(slot.value.sl_func)
                };
                Self::ModuleCreate(create)
            }
            85 => {
                let exec = unsafe {
                    core::mem::transmute::<
                        unsafe extern "C" fn(),
                        unsafe extern "C" fn(*mut rustpython_vm::PyObject) -> i32,
                    >(slot.value.sl_func)
                };
                Self::ModuleExec(exec)
            }
            86 => Self::ModuleMultipleInterpreters {
                value: value_ptr,
                is_static,
            },
            87 => Self::ModuleGil {
                gil_used: !value_ptr.is_null(),
                is_static,
            },
            93 => Self::TypeSlots {
                value: value_ptr.cast(),
                is_static,
            },
            95 => Self::TypeName {
                value: value_ptr.cast(),
                is_static,
            },
            107 => Self::TypeMetaclass {
                value: value_ptr.cast(),
                is_static,
            },
            108 => Self::TypeModule {
                value: value_ptr.cast(),
                is_static,
            },
            109 => Self::ModuleAbi {
                value: value_ptr.cast(),
                is_static,
            },
            100 => Self::ModuleName {
                value: value_ptr.cast(),
                is_static,
            },
            101 => Self::ModuleDoc {
                value: value_ptr.cast(),
                is_static,
            },
            103 => Self::ModuleMethods(value_ptr.cast()),
            id => {
                if slot.is_optional() {
                    Self::Unknown {
                        id,
                        value: value_ptr,
                        is_static,
                    }
                } else {
                    return Err(vm.new_system_error(format!("unsupported required slot id: {id}")));
                }
            }
        };
        Ok(kind)
    }
}
