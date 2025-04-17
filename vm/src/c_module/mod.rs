use std::borrow::Cow;
use std::collections::HashMap;
use crate::builtins::PyModule;
use crate::{PyRef, VirtualMachine};

pub type CModuleInitFunc = Box<py_dyn_fn!(dyn Fn(&VirtualMachine) -> PyRef<PyModule>)>;
pub type CModuleMap = HashMap<Cow<'static, str>, CModuleInitFunc, ahash::RandomState>;
