//! The prelude imports the various objects and traits.
//!
//! The intention is that one can include `use rustpython_vm::prelude::*`.

pub use crate::{
    object::{
        AsObject, Py, PyExact, PyObject, PyObjectRef, PyPayload, PyRef, PyRefExact, PyResult,
        PyWeakRef,
    },
    vm::{Context, Interpreter, Settings, VirtualMachine},
};
