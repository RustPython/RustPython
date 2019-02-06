use convert;
use js_sys::TypeError;
use rustpython_vm::{compile, pyobject::PyObjectRef, VirtualMachine};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use wasm_bindgen::prelude::*;

pub(crate) struct StoredVirtualMachine {
    pub vm: VirtualMachine,
    pub scope: PyObjectRef,
}

impl StoredVirtualMachine {
    fn new() -> StoredVirtualMachine {
        let mut vm = VirtualMachine::new();
        let builtin = vm.get_builtin_scope();
        let scope = vm.context().new_scope(Some(builtin));
        StoredVirtualMachine { vm, scope }
    }
}

// It's fine that it's thread local, since WASM doesn't even have threads yet
thread_local! {
    static STORED_VMS: Rc<RefCell<HashMap<String, StoredVirtualMachine>>> = Rc::default();
}

#[wasm_bindgen(js_name = vmStore)]
pub struct VMStore;

#[wasm_bindgen(js_class = vmStore)]
impl VMStore {
    pub fn init(id: String) -> WASMVirtualMachine {
        STORED_VMS.with(|cell| {
            let mut vms = cell.borrow_mut();
            if !vms.contains_key(&id) {
                vms.insert(id.clone(), StoredVirtualMachine::new());
            }
        });
        WASMVirtualMachine { id }
    }

    pub fn get(id: String) -> JsValue {
        STORED_VMS.with(|cell| {
            let vms = cell.borrow();
            if vms.contains_key(&id) {
                WASMVirtualMachine { id }.into()
            } else {
                JsValue::UNDEFINED
            }
        })
    }

    pub fn destroy(id: String) {
        STORED_VMS.with(|cell| {
            cell.borrow_mut().remove(&id);
        });
    }

    pub fn ids() -> Vec<JsValue> {
        STORED_VMS.with(|cell| cell.borrow().keys().map(|k| k.into()).collect())
    }
}

#[wasm_bindgen(js_name = VirtualMachine)]
#[derive(Clone)]
pub struct WASMVirtualMachine {
    id: String,
}

#[wasm_bindgen(js_class = VirtualMachine)]
impl WASMVirtualMachine {
    pub(crate) fn with_unchecked<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut StoredVirtualMachine) -> R,
    {
        STORED_VMS.with(|cell| {
            let mut vms = cell.borrow_mut();
            let stored_vm = vms.get_mut(&self.id).unwrap();
            f(stored_vm)
        })
    }

    pub(crate) fn with<F, R>(&self, f: F) -> Result<R, JsValue>
    where
        F: FnOnce(&mut StoredVirtualMachine) -> R,
    {
        self.assert_valid()?;
        Ok(self.with_unchecked(f))
    }

    pub fn valid(&self) -> bool {
        STORED_VMS.with(|cell| cell.borrow().contains_key(&self.id))
    }

    pub fn assert_valid(&self) -> Result<(), JsValue> {
        if self.valid() {
            Ok(())
        } else {
            Err(TypeError::new(
                "Invalid VirtualMachine, this VM was destroyed while this reference was still held",
            )
            .into())
        }
    }

    pub fn destroy(&self) -> Result<(), JsValue> {
        self.assert_valid()?;
        VMStore::destroy(self.id.clone());
        Ok(())
    }

    #[wasm_bindgen(js_name = addToScope)]
    pub fn add_to_scope(&self, name: String, value: JsValue) -> Result<(), JsValue> {
        self.with(
            move |StoredVirtualMachine {
                      ref mut vm,
                      ref mut scope,
                  }| {
                let value = convert::js_to_py(vm, value, Some(self.clone()));
                vm.ctx.set_item(scope, &name, value);
            },
        )
    }

    pub fn run(&self, mut source: String) -> Result<JsValue, JsValue> {
        self.assert_valid()?;
        self.with_unchecked(
            |StoredVirtualMachine {
                 ref mut vm,
                 ref mut scope,
             }| {
                source.push('\n');
                let code = compile::compile(vm, &source, &compile::Mode::Single, None)
                    .map_err(|err| convert::py_str_err(vm, &err))?;
                let result = vm
                    .run_code_obj(code, scope.clone())
                    .map_err(|err| convert::py_str_err(vm, &err))?;
                Ok(convert::py_to_js(vm, result, Some(self.clone())))
            },
        )
    }
}
