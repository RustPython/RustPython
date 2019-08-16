use std::fmt;
use std::rc::Rc;

use crate::obj::objdict::PyDictRef;
use crate::pyobject::{ItemProtocol, PyContext, PyObjectRef, PyResult};
use crate::vm::VirtualMachine;

/*
 * So a scope is a linked list of scopes.
 * When a name is looked up, it is check in its scope.
 */
#[derive(Debug)]
struct RcListNode<T> {
    elem: T,
    next: Option<Rc<RcListNode<T>>>,
}

#[derive(Debug, Clone)]
struct RcList<T> {
    head: Option<Rc<RcListNode<T>>>,
}

struct Iter<'a, T: 'a> {
    next: Option<&'a RcListNode<T>>,
}

impl<T> RcList<T> {
    pub fn new() -> Self {
        RcList { head: None }
    }

    pub fn insert(self, elem: T) -> Self {
        RcList {
            head: Some(Rc::new(RcListNode {
                elem,
                next: self.head,
            })),
        }
    }

    #[cfg_attr(feature = "flame-it", flame("RcList"))]
    pub fn iter(&self) -> Iter<T> {
        Iter {
            next: self.head.as_ref().map(|node| &**node),
        }
    }
}

impl<'a, T> Iterator for Iter<'a, T> {
    type Item = &'a T;

    #[cfg_attr(feature = "flame-it", flame("Iter"))]
    fn next(&mut self) -> Option<Self::Item> {
        self.next.map(|node| {
            self.next = node.next.as_ref().map(|node| &**node);
            &node.elem
        })
    }
}

#[derive(Clone)]
pub struct Scope {
    locals: RcList<PyDictRef>,
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
            Some(dict) => RcList::new().insert(dict),
            None => RcList::new(),
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
                .clone()
                .set_item("__builtins__", vm.builtins.clone(), vm)
                .unwrap();
        }
        Scope::new(locals, globals, vm)
    }

    pub fn get_locals(&self) -> PyDictRef {
        match self.locals.iter().next() {
            Some(dict) => dict.clone(),
            None => self.globals.clone(),
        }
    }

    pub fn get_only_locals(&self) -> Option<PyDictRef> {
        self.locals.iter().next().cloned()
    }

    pub fn new_child_scope_with_locals(&self, locals: PyDictRef) -> Scope {
        Scope {
            locals: self.locals.clone().insert(locals),
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
    fn load_cell(&self, vm: &VirtualMachine, name: &str) -> Option<PyObjectRef>;
    fn store_cell(&self, vm: &VirtualMachine, name: &str, value: PyObjectRef);
    fn load_global(&self, vm: &VirtualMachine, name: &str) -> Option<PyObjectRef>;
    fn store_global(&self, vm: &VirtualMachine, name: &str, value: PyObjectRef);
}

impl NameProtocol for Scope {
    #[cfg_attr(feature = "flame-it", flame("Scope"))]
    fn load_name(&self, vm: &VirtualMachine, name: &str) -> Option<PyObjectRef> {
        for dict in self.locals.iter() {
            if let Some(value) = dict.get_item_option(&name.to_string(), vm).unwrap() {
                return Some(value);
            }
        }

        if let Some(value) = self.globals.get_item_option(&name.to_string(), vm).unwrap() {
            return Some(value);
        }

        vm.get_attribute(vm.builtins.clone(), name).ok()
    }

    #[cfg_attr(feature = "flame-it", flame("Scope"))]
    fn load_cell(&self, vm: &VirtualMachine, name: &str) -> Option<PyObjectRef> {
        for dict in self.locals.iter().skip(1) {
            if let Some(value) = dict.get_item_option(&name.to_string(), vm).unwrap() {
                return Some(value);
            }
        }
        None
    }

    fn store_cell(&self, vm: &VirtualMachine, name: &str, value: PyObjectRef) {
        self.locals
            .iter()
            .nth(1)
            .expect("no outer scope for non-local")
            .set_item(name, value, vm)
            .unwrap();
    }

    fn store_name(&self, vm: &VirtualMachine, key: &str, value: PyObjectRef) {
        self.get_locals().set_item(key, value, vm).unwrap();
    }

    fn delete_name(&self, vm: &VirtualMachine, key: &str) -> PyResult {
        self.get_locals().del_item(key, vm)
    }

    #[cfg_attr(feature = "flame-it", flame("Scope"))]
    fn load_global(&self, vm: &VirtualMachine, name: &str) -> Option<PyObjectRef> {
        self.globals.get_item_option(&name.to_string(), vm).unwrap()
    }

    fn store_global(&self, vm: &VirtualMachine, name: &str, value: PyObjectRef) {
        self.globals.set_item(name, value, vm).unwrap();
    }
}
