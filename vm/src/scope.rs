use std::fmt;

use crate::builtins::dict::PyDictRef;
use crate::pyobject::{ItemProtocol, PyContext, PyObjectRef, PyResult};
use crate::vm::VirtualMachine;

/*
 * So a scope is a linked list of scopes.
 * When a name is looked up, it is check in its scope.
 */
#[derive(Clone)]
pub struct Scope {
    locals: Vec<PyDictRef>,
    pub globals: PyDictRef,
}

impl fmt::Debug for Scope {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // TODO: have a more informative Debug impl that DOESN'T recurse and cause a stack overflow
        f.write_str("Scope")
    }
}

impl Scope {
    pub fn new(locals: Option<PyDictRef>, globals: PyDictRef, vm: &VirtualMachine) -> Scope {
        let locals = match locals {
            Some(dict) => vec![dict],
            None => vec![],
        };
        let scope = Scope { locals, globals };
        scope.store_name(vm, "__annotations__", vm.ctx.new_dict().into_object());
        scope
    }

    pub fn with_builtins(
        locals: Option<PyDictRef>,
        globals: PyDictRef,
        vm: &VirtualMachine,
    ) -> Scope {
        if !globals.contains_key("__builtins__", vm) {
            globals
                .set_item("__builtins__", vm.builtins.clone(), vm)
                .unwrap();
        }
        Scope::new(locals, globals, vm)
    }

    pub fn get_locals(&self) -> &PyDictRef {
        match self.locals.first() {
            Some(dict) => dict,
            None => &self.globals,
        }
    }

    pub fn get_only_locals(&self) -> Option<PyDictRef> {
        self.locals.first().cloned()
    }

    pub fn new_child_scope_with_locals(&self, locals: PyDictRef) -> Scope {
        let mut new_locals = Vec::with_capacity(self.locals.len() + 1);
        new_locals.push(locals);
        new_locals.extend_from_slice(&self.locals);
        Scope {
            locals: new_locals,
            globals: self.globals.clone(),
        }
    }

    pub fn new_child_scope(&self, ctx: &PyContext) -> Scope {
        self.new_child_scope_with_locals(ctx.new_dict())
    }
}

pub trait NameProtocol {
    fn load_name(&self, vm: &VirtualMachine, name: &str) -> Option<PyObjectRef>;
    fn store_name(&self, vm: &VirtualMachine, name: &str, value: PyObjectRef);
    fn delete_name(&self, vm: &VirtualMachine, name: &str) -> PyResult;
    fn load_local(&self, vm: &VirtualMachine, name: &str) -> Option<PyObjectRef>;
    fn load_cell(&self, vm: &VirtualMachine, name: &str) -> Option<PyObjectRef>;
    fn store_cell(&self, vm: &VirtualMachine, name: &str, value: PyObjectRef);
    fn load_global(&self, vm: &VirtualMachine, name: &str) -> Option<PyObjectRef>;
    fn store_global(&self, vm: &VirtualMachine, name: &str, value: PyObjectRef);
}

impl NameProtocol for Scope {
    #[cfg_attr(feature = "flame-it", flame("Scope"))]
    fn load_name(&self, vm: &VirtualMachine, name: &str) -> Option<PyObjectRef> {
        for dict in self.locals.iter() {
            if let Some(value) = dict.get_item_option(name, vm).unwrap() {
                return Some(value);
            }
        }

        // Fall back to loading a global after all scopes have been searched!
        self.load_global(vm, name)
    }

    #[cfg_attr(feature = "flame-it", flame("Scope"))]
    /// Load a local name. Only check the local dictionary for the given name.
    fn load_local(&self, vm: &VirtualMachine, name: &str) -> Option<PyObjectRef> {
        self.get_locals().get_item_option(name, vm).unwrap()
    }

    #[cfg_attr(feature = "flame-it", flame("Scope"))]
    fn load_cell(&self, vm: &VirtualMachine, name: &str) -> Option<PyObjectRef> {
        for dict in self.locals.iter().skip(1) {
            if let Some(value) = dict.get_item_option(name, vm).unwrap() {
                return Some(value);
            }
        }
        None
    }

    fn store_cell(&self, vm: &VirtualMachine, name: &str, value: PyObjectRef) {
        // find the innermost outer scope that contains the symbol name
        if let Some(locals) = self.locals.iter().rev().find(|l| l.contains_key(name, vm)) {
            // add to the symbol
            locals.set_item(name, value, vm).unwrap();
        } else {
            // somewhat limited solution -> fallback to the old rustpython strategy
            // and store the next outer scope
            // This case is usually considered as a failure case, but kept for the moment
            // to support the scope propagation for named expression assignments to so far
            // unknown names in comprehensions. We need to consider here more context
            // information for correct handling.
            self.locals
                .get(1)
                .expect("no outer scope for non-local")
                .set_item(name, value, vm)
                .unwrap();
        }
    }

    fn store_name(&self, vm: &VirtualMachine, key: &str, value: PyObjectRef) {
        self.get_locals().set_item(key, value, vm).unwrap();
    }

    fn delete_name(&self, vm: &VirtualMachine, key: &str) -> PyResult {
        self.get_locals().del_item(key, vm)
    }

    #[cfg_attr(feature = "flame-it", flame("Scope"))]
    /// Load a global name.
    fn load_global(&self, vm: &VirtualMachine, name: &str) -> Option<PyObjectRef> {
        if let Some(value) = self.globals.get_item_option(name, vm).unwrap() {
            Some(value)
        } else {
            vm.get_attribute(vm.builtins.clone(), name).ok()
        }
    }

    fn store_global(&self, vm: &VirtualMachine, name: &str, value: PyObjectRef) {
        self.globals.set_item(name, value, vm).unwrap();
    }
}
