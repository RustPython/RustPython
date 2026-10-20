use crate::PyObject;
use crate::methodobject::PyMethodDef;
use crate::modsupport::PyABIInfo;
use crate::object::PyType_Slot;
use crate::util::CStrExt;
use alloc::borrow::Cow;
use core::ffi::{c_char, c_int, c_void};
use core::num::TryFromIntError;
use itertools::Either;
use rustpython_vm::builtins::{PyModule, PyType};
use rustpython_vm::types::SlotAccessor;
use rustpython_vm::{Py, PyResult, VirtualMachine};

#[repr(C)]
#[derive(Clone)]
pub struct PySlot {
    pub sl_id: u16,
    pub sl_flags: u16,
    _reserved: u32,
    pub value: PySlotValue,
}

impl TryFrom<&PyType_Slot> for PySlot {
    type Error = TryFromIntError;

    fn try_from(slot: &PyType_Slot) -> Result<Self, Self::Error> {
        Ok(Self {
            sl_id: slot.slot.try_into()?,
            sl_flags: 0,
            _reserved: 0,
            value: slot.pfunc.into(),
        })
    }
}

impl PySlot {
    const SLOT_OPTIONAL: u16 = 0x0001;
    const SLOT_STATIC: u16 = 0x0002;
    const SLOT_INTPTR: u16 = 0x0004;
    pub(crate) const SLOT_END: Self = unsafe { core::mem::zeroed() };

    pub(crate) fn new(id: u16, value: impl Into<PySlotValue>) -> Self {
        Self {
            sl_id: id,
            sl_flags: 0,
            _reserved: 0,
            value: value.into(),
        }
    }

    pub(crate) fn iter<'a>(slots: *const Self) -> impl Iterator<Item = Cow<'a, Self>> {
        let mut stack: Vec<Either<*const Self, *const PyType_Slot>> = vec![Either::Left(slots)];
        core::iter::from_fn(move || {
            loop {
                assert!(
                    stack.len() <= 10,
                    "Too many nested slots, possible infinite loop in slot iteration"
                );
                return match stack.last_mut()? {
                    Either::Left(slot_ptr) => {
                        let slot = unsafe { &**slot_ptr };
                        if slot.sl_id == 0 {
                            stack.pop();
                            continue;
                        }

                        *slot_ptr = unsafe { slot_ptr.add(1) };

                        if slot.sl_id == 92 {
                            let subslots = unsafe { slot.value.sl_ptr }.cast_const().cast::<Self>();
                            if !subslots.is_null() {
                                stack.push(Either::Left(subslots));
                            }
                            continue;
                        }

                        if slot.sl_id == 93 {
                            let type_slots = unsafe { slot.value.sl_ptr }
                                .cast_const()
                                .cast::<PyType_Slot>();
                            if !type_slots.is_null() {
                                stack.push(Either::Right(type_slots));
                            }
                            continue;
                        }

                        Some(Cow::Borrowed(slot))
                    }
                    Either::Right(slot_ptr) => {
                        let type_slot = unsafe { &**slot_ptr };
                        if type_slot.slot == 0 {
                            stack.pop();
                            continue;
                        }

                        *slot_ptr = unsafe { slot_ptr.add(1) };
                        Some(Cow::Owned(type_slot.try_into().expect("invalid slot")))
                    }
                };
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

    pub(crate) fn as_kind<'a>(&self, vm: &VirtualMachine) -> PyResult<PySlotKind<'a>> {
        let value_ptr = unsafe { self.value.sl_ptr };
        let is_static = self.is_static();
        let kind = match self.sl_id {
            1..82 => {
                let slot_id: u8 = self.sl_id.try_into().unwrap();
                let accessor = slot_id.try_into().unwrap();
                PySlotKind::Type(PySlotType::TypeSlot(accessor, value_ptr))
            }
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
            92 | 93 => {
                unreachable!("Py_slot_subslots and Py_tp_slots should be handled in the iterator")
            }
            95 => PySlotKind::Type(PySlotType::Name(unsafe {
                value_ptr.cast::<c_char>().try_as_str(vm)?
            })),
            96 => PySlotKind::Type(PySlotType::BasicSize(unsafe { self.value.sl_size })),
            97 => PySlotKind::Type(PySlotType::ExtraBasicSize(unsafe { self.value.sl_size })),
            99 => PySlotKind::Type(PySlotType::Flags(unsafe { self.value.sl_uint64 })),
            100 => PySlotKind::Module(PySlotModule::Name(unsafe {
                value_ptr.cast::<c_char>().try_as_str(vm)?
            })),
            101 => PySlotKind::Module(PySlotModule::Doc(unsafe {
                value_ptr.cast::<c_char>().try_as_str(vm)?
            })),
            // 102 => Py_mod_state_size
            103 => PySlotKind::Module(PySlotModule::Methods(unsafe { &*value_ptr.cast() })),
            // 104 => Py_mod_state_traverse
            // 105 => Py_mod_state_clear
            // 106 => Py_mod_state_free
            107 => PySlotKind::Type(PySlotType::Metaclass(unsafe { &*value_ptr.cast() })),
            108 => PySlotKind::Type(PySlotType::Module(unsafe { &*value_ptr.cast() })),
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
#[derive(Copy, Clone)]
pub union PySlotValue {
    pub sl_ptr: *mut c_void,
    pub sl_func: unsafe extern "C" fn(),
    pub sl_size: isize,
    pub sl_int64: i64,
    pub sl_uint64: u64,
}

impl<T> From<*mut T> for PySlotValue {
    fn from(ptr: *mut T) -> Self {
        Self { sl_ptr: ptr.cast() }
    }
}

impl From<isize> for PySlotValue {
    fn from(size: isize) -> Self {
        Self { sl_size: size }
    }
}

impl From<u64> for PySlotValue {
    fn from(value: u64) -> Self {
        Self { sl_uint64: value }
    }
}

#[allow(dead_code)]
#[derive(Debug, Copy, Clone)]
pub(crate) enum PySlotModule<'a> {
    Create(unsafe extern "C" fn(spec: *mut PyObject, def: *mut c_void) -> *mut PyObject),
    Exec(unsafe extern "C" fn(*mut PyObject) -> c_int),
    Name(&'a str),
    Doc(&'a str),
    Methods(&'a PyMethodDef),
    Abi(PyABIInfo),
    MultipleInterpreters(*mut c_void),
    Gil { gil_used: bool },
}

#[allow(dead_code)]
#[derive(Debug, Copy, Clone)]
pub(crate) enum PySlotType<'a> {
    TypeSlot(SlotAccessor, *mut c_void),
    Name(&'a str),
    Flags(u64),
    BasicSize(isize),
    ExtraBasicSize(isize),
    Metaclass(&'a Py<PyType>),
    Module(&'a Py<PyModule>),
}

#[allow(dead_code)]
#[derive(Debug, Copy, Clone)]
pub(crate) enum PySlotKind<'a> {
    Module(PySlotModule<'a>),
    Type(PySlotType<'a>),
    Unknown {
        id: u16,
        value: *mut c_void,
        is_static: bool,
    },
}
