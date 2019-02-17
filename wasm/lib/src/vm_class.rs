use convert;
use js_sys::{SyntaxError, TypeError};
use rustpython_vm::{
    compile,
    pyobject::{PyObjectRef, PyRef},
    VirtualMachine,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use wasm_bindgen::prelude::*;
use wasm_builtins;

pub(crate) struct StoredVirtualMachine {
    pub vm: VirtualMachine,
    pub scope: PyObjectRef,
}

impl StoredVirtualMachine {
    fn new() -> StoredVirtualMachine {
        let mut vm = VirtualMachine::new();
        let builtin = vm.get_builtin_scope();
        let scope = vm.context().new_scope(Some(builtin));
        setup_vm_scope(&mut vm, &scope);
        StoredVirtualMachine { vm, scope }
    }
}

fn setup_vm_scope(vm: &mut VirtualMachine, scope: &PyObjectRef) {
    vm.ctx.set_attr(
        scope,
        "print",
        vm.ctx.new_rustfunc(wasm_builtins::builtin_print_console),
    );
}

// It's fine that it's thread local, since WASM doesn't even have threads yet
thread_local! {
    static STORED_VMS: PyRef<HashMap<String, PyRef<StoredVirtualMachine>>> = Rc::default();
    static ACTIVE_VMS: PyRef<HashMap<String, *mut VirtualMachine>> = Rc::default();
}

#[wasm_bindgen(js_name = vmStore)]
pub struct VMStore;

#[wasm_bindgen(js_class = vmStore)]
impl VMStore {
    pub fn init(id: String) -> WASMVirtualMachine {
        STORED_VMS.with(|cell| {
            let mut vms = cell.borrow_mut();
            if !vms.contains_key(&id) {
                vms.insert(
                    id.clone(),
                    Rc::new(RefCell::new(StoredVirtualMachine::new())),
                );
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
            use std::collections::hash_map::Entry;
            match cell.borrow_mut().entry(id) {
                Entry::Occupied(o) => {
                    let (_k, stored_vm) = o.remove_entry();
                    // for f in stored_vm.drop_handlers.iter() {
                    //     f();
                    // }
                    // deallocate the VM
                    drop(stored_vm);
                }
                Entry::Vacant(_v) => {}
            }
        });
    }

    pub fn ids() -> Vec<JsValue> {
        STORED_VMS.with(|cell| cell.borrow().keys().map(|k| k.into()).collect())
    }
}

pub(crate) struct AccessibleVM {
    weak: Weak<RefCell<StoredVirtualMachine>>,
    id: String,
}

impl AccessibleVM {
    pub fn from_id(id: String) -> AccessibleVM {
        let weak = STORED_VMS
            .with(|cell| Rc::downgrade(cell.borrow().get(&id).expect("WASM VM to be valid")));
        AccessibleVM { weak, id }
    }

    pub fn upgrade(&self) -> Option<&mut VirtualMachine> {
        let vm_cell = self.weak.upgrade()?;
        match vm_cell.try_borrow_mut() {
            Ok(mut vm) => {
                ACTIVE_VMS.with(|cell| {
                    cell.borrow_mut().insert(self.id.clone(), &mut vm.vm);
                });
            }
            Err(_) => {}
        };
        Some(ACTIVE_VMS.with(|cell| {
            let vms = cell.borrow();
            let ptr = vms.get(&self.id).expect("id to be in ACTIVE_VMS");
            unsafe { &mut **ptr }
        }))
    }
}

impl From<WASMVirtualMachine> for AccessibleVM {
    fn from(vm: WASMVirtualMachine) -> AccessibleVM {
        AccessibleVM::from_id(vm.id)
    }
}
impl From<&WASMVirtualMachine> for AccessibleVM {
    fn from(vm: &WASMVirtualMachine) -> AccessibleVM {
        AccessibleVM::from_id(vm.id.clone())
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
            let mut stored_vm = vms.get_mut(&self.id).unwrap().borrow_mut();
            f(&mut stored_vm)
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
                vm.ctx.set_attr(scope, &name, value);
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
                let code = compile::compile(
                    &source,
                    &compile::Mode::Exec,
                    "<wasm>".to_string(),
                    vm.ctx.code_type(),
                )
                .map_err(|err| SyntaxError::new(&format!("Error parsing Python code: {}", err)))?;
                let result = vm
                    .run_code_obj(code, scope.clone())
                    .map_err(|err| convert::py_str_err(vm, &err))?;
                Ok(convert::py_to_js(vm, result, Some(self.clone())))
            },
        )
    }
}
