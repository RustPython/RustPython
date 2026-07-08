use crate::PyObject;
use crate::methodobject::PyMethodDef;
use crate::object::PyType_Slot;
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

#[derive(Debug, Copy, Clone)]
pub(crate) enum PySlotModule {
    Create(unsafe extern "C" fn(spec: *mut PyObject, def: *mut c_void) -> *mut PyObject),
    Exec(unsafe extern "C" fn(*mut PyObject) -> c_int),
    Name {
        value: *const c_char,
        is_static: bool,
    },
    Doc {
        value: *const c_char,
        is_static: bool,
    },
    Methods(*mut PyMethodDef),
    Abi {
        value: *const PyABIInfo,
        is_static: bool,
    },
    MultipleInterpreters {
        value: *mut c_void,
        is_static: bool,
    },
    Gil {
        gil_used: bool,
        is_static: bool,
    },
}

#[derive(Debug, Copy, Clone)]
pub(crate) enum PySlotType {
    Base {
        value: *mut PyObject,
        is_static: bool,
    },
    Bases {
        value: *mut PyObject,
        is_static: bool,
    },
    Token {
        value: *mut c_void,
        is_static: bool,
    },
    Slots {
        value: *mut PyType_Slot,
        is_static: bool,
    },
    Name {
        value: *const c_char,
        is_static: bool,
    },
    Flags {
        value: u64,
    },
    ExtraBasicSize(isize),
    Metaclass {
        value: *mut PyObject,
        is_static: bool,
    },
    Module {
        value: *mut PyObject,
        is_static: bool,
    },
}

#[allow(dead_code)]
#[derive(Debug, Copy, Clone)]
pub(crate) enum PySlotKind {
    Module(PySlotModule),
    Type(PySlotType),
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
            Self::Module(module) => match module {
                PySlotModule::Create(_) | PySlotModule::Exec(_) | PySlotModule::Methods(_) => true,
                PySlotModule::Name { is_static, .. }
                | PySlotModule::Doc { is_static, .. }
                | PySlotModule::Abi { is_static, .. }
                | PySlotModule::MultipleInterpreters { is_static, .. }
                | PySlotModule::Gil { is_static, .. } => *is_static,
            },
            Self::Type(ty) => match ty {
                PySlotType::Flags { .. } | PySlotType::ExtraBasicSize(_) => true,
                PySlotType::Base { is_static, .. }
                | PySlotType::Bases { is_static, .. }
                | PySlotType::Token { is_static, .. }
                | PySlotType::Slots { is_static, .. }
                | PySlotType::Name { is_static, .. }
                | PySlotType::Metaclass { is_static, .. }
                | PySlotType::Module { is_static, .. } => *is_static,
            },
            Self::Unknown { is_static, .. } => *is_static,
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
            48 => Self::Type(PySlotType::Base {
                value: value_ptr.cast(),
                is_static,
            }),
            49 => Self::Type(PySlotType::Bases {
                value: value_ptr.cast(),
                is_static,
            }),
            83 => Self::Type(PySlotType::Token {
                value: value_ptr,
                is_static,
            }),
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
                Self::Module(PySlotModule::Create(create))
            }
            85 => {
                let exec = unsafe {
                    core::mem::transmute::<
                        unsafe extern "C" fn(),
                        unsafe extern "C" fn(*mut rustpython_vm::PyObject) -> i32,
                    >(slot.value.sl_func)
                };
                Self::Module(PySlotModule::Exec(exec))
            }
            86 => Self::Module(PySlotModule::MultipleInterpreters {
                value: value_ptr,
                is_static,
            }),
            87 => Self::Module(PySlotModule::Gil {
                gil_used: !value_ptr.is_null(),
                is_static,
            }),
            93 => Self::Type(PySlotType::Slots {
                value: value_ptr.cast(),
                is_static,
            }),
            95 => Self::Type(PySlotType::Name {
                value: value_ptr.cast(),
                is_static,
            }),
            97 => Self::Type(PySlotType::ExtraBasicSize(unsafe { slot.value.sl_size })),
            99 => Self::Type(PySlotType::Flags {
                value: unsafe { slot.value.sl_uint64 },
            }),
            107 => Self::Type(PySlotType::Metaclass {
                value: value_ptr.cast(),
                is_static,
            }),
            108 => Self::Type(PySlotType::Module {
                value: value_ptr.cast(),
                is_static,
            }),
            109 => Self::Module(PySlotModule::Abi {
                value: value_ptr.cast(),
                is_static,
            }),
            100 => Self::Module(PySlotModule::Name {
                value: value_ptr.cast(),
                is_static,
            }),
            101 => Self::Module(PySlotModule::Doc {
                value: value_ptr.cast(),
                is_static,
            }),
            103 => Self::Module(PySlotModule::Methods(value_ptr.cast())),
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
