use convert;
use js_sys::TypeError;
use rustpython_vm::{pyobject::PyObjectRef, VirtualMachine};
use std::cell::RefCell;
use std::collections::HashMap;
use wasm_bindgen::prelude::*;

struct StoredVirtualMachine {
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
    static STORED_VMS: RefCell<HashMap<String, StoredVirtualMachine>> = RefCell::default();
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

    pub fn destroy(id: &String) {
        STORED_VMS.with(|cell| {
            cell.borrow_mut().remove(id);
        });
    }

    pub fn ids() -> Vec<JsValue> {
        STORED_VMS.with(|cell| cell.borrow().keys().map(|k| k.into()).collect())
    }
}

#[wasm_bindgen(js_name = VirtualMachine)]
pub struct WASMVirtualMachine {
    id: String,
}

#[wasm_bindgen(js_class = VirtualMachine)]
impl WASMVirtualMachine {
    pub fn valid(&self) -> bool {
        STORED_VMS.with(|cell| cell.borrow().contains_key(&self.id))
    }

    fn assert_valid(&self) -> Result<(), JsValue> {
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
        VMStore::destroy(&self.id);
        Ok(())
    }

    #[wasm_bindgen(js_name = addToScope)]
    pub fn add_to_scope(&self, name: String, value: JsValue) -> Result<(), JsValue> {
        self.assert_valid()?;
        STORED_VMS.with(|cell| {
            let mut vms = cell.borrow_mut();
            let StoredVirtualMachine {
                ref mut vm,
                ref mut scope,
            } = vms.get_mut(&self.id).unwrap();
            let value = convert::js_to_py(vm, value);
            vm.ctx.set_attr(scope, &name, value);
        });
        Ok(())
    }
}
