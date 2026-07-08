use crate::PyObject;
use crate::methodobject::PyMethodDef;
use crate::object::PyType_Slot;
use core::ffi::{c_char, c_int, c_void};
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

    pub(crate) fn as_kind(&self, vm: &VirtualMachine) -> PyResult<PySlotKind> {
        let value_ptr = unsafe { self.value.sl_ptr };
        let is_static = self.is_static();
        let kind = match self.sl_id {
            48 => PySlotKind::Type(PySlotType::Base {
                value: value_ptr.cast(),
                is_static,
            }),
            49 => PySlotKind::Type(PySlotType::Bases {
                value: value_ptr.cast(),
                is_static,
            }),
            83 => PySlotKind::Type(PySlotType::Token {
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
                    >(self.value.sl_func)
                };
                PySlotKind::Module(PySlotModule::Create(create))
            }
            85 => {
                let exec = unsafe {
                    core::mem::transmute::<
                        unsafe extern "C" fn(),
                        unsafe extern "C" fn(*mut PyObject) -> i32,
                    >(self.value.sl_func)
                };
                PySlotKind::Module(PySlotModule::Exec(exec))
            }
            86 => PySlotKind::Module(PySlotModule::MultipleInterpreters(value_ptr)),
            87 => PySlotKind::Module(PySlotModule::Gil {
                gil_used: !value_ptr.is_null(),
            }),
            // 92 => Py_slot_subslots
            93 => PySlotKind::Type(PySlotType::Slots {
                value: value_ptr.cast(),
                is_static,
            }),
            95 => PySlotKind::Type(PySlotType::Name {
                value: value_ptr.cast(),
                is_static,
            }),
            96 => PySlotKind::Type(PySlotType::BasicSize(unsafe { self.value.sl_size })),
            97 => PySlotKind::Type(PySlotType::ExtraBasicSize(unsafe { self.value.sl_size })),
            99 => PySlotKind::Type(PySlotType::Flags(unsafe { self.value.sl_uint64 })),
            100 => PySlotKind::Module(PySlotModule::Name {
                value: value_ptr.cast(),
                is_static,
            }),
            101 => PySlotKind::Module(PySlotModule::Doc {
                value: value_ptr.cast(),
                is_static,
            }),
            // 102 => Py_mod_state_size
            103 => PySlotKind::Module(PySlotModule::Methods(value_ptr.cast())),
            // 104 => Py_mod_state_traverse
            // 105 => Py_mod_state_clear
            // 106 => Py_mod_state_free
            107 => PySlotKind::Type(PySlotType::Metaclass(value_ptr.cast())),
            108 => PySlotKind::Type(PySlotType::Module(value_ptr.cast())),
            109 => PySlotKind::Module(PySlotModule::Abi(unsafe { *value_ptr.cast() })),
            // 110 => Py_mod_token
            id => {
                if self.is_optional() {
                    PySlotKind::Unknown {
                        id,
                        value: value_ptr,
                        is_static,
                    }
                } else {
                    return Err(vm.new_system_error(format!("unsupported required slot: {id}")));
                }
            }
        };
        Ok(kind)
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
#[derive(Debug, Copy, Clone)]
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
    Abi(PyABIInfo),
    MultipleInterpreters(*mut c_void),
    Gil {
        gil_used: bool,
    },
}

#[allow(dead_code)]
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
    Flags(u64),
    BasicSize(isize),
    ExtraBasicSize(isize),
    Metaclass(*mut PyObject),
    Module(*mut PyObject),
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
