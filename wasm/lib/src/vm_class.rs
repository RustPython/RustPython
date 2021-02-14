use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use js_sys::{Object, TypeError};
use wasm_bindgen::prelude::*;

use rustpython_vm::compile::{self, Mode};
use rustpython_vm::pyobject::{ItemProtocol, PyObjectRef, PyObjectWeak, PyValue};
use rustpython_vm::scope::Scope;
use rustpython_vm::{InitParameter, Interpreter, PySettings, VirtualMachine};

use crate::browser_module::setup_browser_module;
use crate::convert::{self, PyResultExt};
use crate::js_module;
use crate::wasm_builtins;

pub(crate) struct StoredVirtualMachine {
    pub interp: Interpreter,
    pub scope: Scope,
    /// you can put a Rc in here, keep it as a Weak, and it'll be held only for
    /// as long as the StoredVM is alive
    held_objects: RefCell<Vec<PyObjectRef>>,
}

impl StoredVirtualMachine {
    fn new(id: String, inject_browser_module: bool) -> StoredVirtualMachine {
        let mut scope = None;
        let interp = Interpreter::new_with_init(PySettings::default(), |vm| {
            vm.wasm_id = Some(id);

            js_module::setup_js_module(vm);
            if inject_browser_module {
                fn init_window_module(vm: &VirtualMachine) -> PyObjectRef {
                    py_module!(vm, "_window", {
                        "window" => js_module::PyJsValue::new(wasm_builtins::window()).into_ref(vm),
                    })
                }
                vm.add_native_module("_window".to_owned(), Box::new(init_window_module));
                setup_browser_module(vm);
            }

            VM_INIT_FUNCS.with(|cell| {
                for f in cell.borrow().iter() {
                    f(vm)
                }
            });

            scope = Some(vm.new_scope_with_builtins());

            InitParameter::Internal
        });

        StoredVirtualMachine {
            interp,
            scope: scope.unwrap(),
            held_objects: RefCell::new(Vec::new()),
        }
    }
}

/// Add a hook to add builtins or frozen modules to the RustPython VirtualMachine while it's
/// initializing.
pub fn add_init_func(f: fn(&mut VirtualMachine)) {
    VM_INIT_FUNCS.with(|cell| cell.borrow_mut().push(f))
}

// It's fine that it's thread local, since WASM doesn't even have threads yet. thread_local!
// probably gets compiled down to a normal-ish static varible, like Atomic* types do:
// https://rustwasm.github.io/2018/10/24/multithreading-rust-and-wasm.html#atomic-instructions
thread_local! {
    static STORED_VMS: RefCell<HashMap<String, Rc<StoredVirtualMachine>>> = RefCell::default();
    static VM_INIT_FUNCS: RefCell<Vec<fn(&mut VirtualMachine)>> = RefCell::default();
}

pub fn get_vm_id(vm: &VirtualMachine) -> &str {
    vm.wasm_id
        .as_ref()
        .expect("VirtualMachine inside of WASM crate should have wasm_id set")
}
pub(crate) fn stored_vm_from_wasm(wasm_vm: &WASMVirtualMachine) -> Rc<StoredVirtualMachine> {
    STORED_VMS.with(|cell| {
        cell.borrow()
            .get(&wasm_vm.id)
            .expect("VirtualMachine is not valid")
            .clone()
    })
}
pub(crate) fn weak_vm(vm: &VirtualMachine) -> Weak<StoredVirtualMachine> {
    let id = get_vm_id(vm);
    STORED_VMS
        .with(|cell| Rc::downgrade(cell.borrow().get(id).expect("VirtualMachine is not valid")))
}

#[wasm_bindgen(js_name = vmStore)]
pub struct VMStore;

#[wasm_bindgen(js_class = vmStore)]
impl VMStore {
    pub fn init(id: String, inject_browser_module: Option<bool>) -> WASMVirtualMachine {
        STORED_VMS.with(|cell| {
            let mut vms = cell.borrow_mut();
            if !vms.contains_key(&id) {
                let stored_vm =
                    StoredVirtualMachine::new(id.clone(), inject_browser_module.unwrap_or(true));
                vms.insert(id.clone(), Rc::new(stored_vm));
            }
        });
        WASMVirtualMachine { id }
    }

    pub(crate) fn _get(id: String) -> Option<WASMVirtualMachine> {
        STORED_VMS.with(|cell| {
            let vms = cell.borrow();
            if vms.contains_key(&id) {
                Some(WASMVirtualMachine { id })
            } else {
                None
            }
        })
    }

    pub fn get(id: String) -> JsValue {
        match Self::_get(id) {
            Some(wasm_vm) => wasm_vm.into(),
            None => JsValue::UNDEFINED,
        }
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

#[wasm_bindgen(js_name = VirtualMachine)]
#[derive(Clone)]
pub struct WASMVirtualMachine {
    pub(crate) id: String,
}

#[wasm_bindgen(js_class = VirtualMachine)]
impl WASMVirtualMachine {
    pub(crate) fn with_unchecked<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&StoredVirtualMachine) -> R,
    {
        let stored_vm = STORED_VMS.with(|cell| {
            let mut vms = cell.borrow_mut();
            vms.get_mut(&self.id).unwrap().clone()
        });
        f(&stored_vm)
    }

    pub(crate) fn with<F, R>(&self, f: F) -> Result<R, JsValue>
    where
        F: FnOnce(&StoredVirtualMachine) -> R,
    {
        self.assert_valid()?;
        Ok(self.with_unchecked(f))
    }

    pub(crate) fn with_vm<F, R>(&self, f: F) -> Result<R, JsValue>
    where
        F: FnOnce(&VirtualMachine, &StoredVirtualMachine) -> R,
    {
        self.with(|stored| stored.interp.enter(|vm| f(vm, stored)))
    }

    pub fn valid(&self) -> bool {
        STORED_VMS.with(|cell| cell.borrow().contains_key(&self.id))
    }

    pub(crate) fn push_held_rc(&self, obj: PyObjectRef) -> Result<PyObjectWeak, JsValue> {
        self.with(|stored_vm| {
            let weak = PyObjectRef::downgrade(&obj);
            stored_vm.held_objects.borrow_mut().push(obj);
            weak
        })
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
        self.with_vm(move |vm, StoredVirtualMachine { ref scope, .. }| {
            let value = convert::js_to_py(vm, value);
            scope.globals.set_item(name, value, vm).into_js(vm)
        })?
    }

    #[wasm_bindgen(js_name = setStdout)]
    pub fn set_stdout(&self, stdout: JsValue) -> Result<(), JsValue> {
        self.with_vm(|vm, _| {
            fn error() -> JsValue {
                TypeError::new("Unknown stdout option, please pass a function or 'console'").into()
            }
            use wasm_builtins::make_stdout_object;
            let stdout: PyObjectRef = if let Some(s) = stdout.as_string() {
                match s.as_str() {
                    "console" => make_stdout_object(vm, wasm_builtins::sys_stdout_write_console),
                    _ => return Err(error()),
                }
            } else if stdout.is_function() {
                let func = js_sys::Function::from(stdout);
                make_stdout_object(vm, move |data, vm| {
                    func.call1(&JsValue::UNDEFINED, &data.into())
                        .map_err(|err| convert::js_py_typeerror(vm, err))?;
                    Ok(())
                })
            } else if stdout.is_null() {
                make_stdout_object(vm, |_, _| Ok(()))
            } else if stdout.is_undefined() {
                make_stdout_object(vm, wasm_builtins::sys_stdout_write_console)
            } else {
                return Err(error());
            };
            vm.set_attr(&vm.sys_module, "stdout", stdout).unwrap();
            Ok(())
        })?
    }

    #[wasm_bindgen(js_name = injectModule)]
    pub fn inject_module(
        &self,
        name: String,
        source: &str,
        imports: Option<Object>,
    ) -> Result<(), JsValue> {
        self.with_vm(|vm, _| {
            let code = vm
                .compile(source, Mode::Exec, name.clone())
                .map_err(convert::syntax_err)?;
            let attrs = vm.ctx.new_dict();
            attrs
                .set_item("__name__", vm.ctx.new_str(name.clone()), vm)
                .into_js(vm)?;

            if let Some(imports) = imports {
                for entry in convert::object_entries(&imports) {
                    let (key, value) = entry?;
                    let key: String = Object::from(key).to_string().into();
                    attrs
                        .set_item(key.as_str(), convert::js_to_py(vm, value), vm)
                        .into_js(vm)?;
                }
            }

            vm.run_code_obj(code, Scope::new(None, attrs.clone()))
                .into_js(vm)?;

            let module = vm.new_module(&name, attrs);

            let sys_modules = vm
                .get_attribute(vm.sys_module.clone(), "modules")
                .into_js(vm)?;
            sys_modules.set_item(name, module, vm).into_js(vm)?;

            Ok(())
        })?
    }

    #[wasm_bindgen(js_name = injectJSModule)]
    pub fn inject_js_module(&self, name: String, module: Object) -> Result<(), JsValue> {
        self.with_vm(|vm, _| {
            let py_module = vm.new_module(&name, vm.ctx.new_dict());
            for entry in convert::object_entries(&module) {
                let (key, value) = entry?;
                let key = Object::from(key).to_string();
                extend_module!(vm, py_module, {
                    String::from(key) => convert::js_to_py(vm, value),
                });
            }

            let sys_modules = vm
                .get_attribute(vm.sys_module.clone(), "modules")
                .into_js(vm)?;
            sys_modules.set_item(name, py_module, vm).into_js(vm)?;

            Ok(())
        })?
    }

    pub(crate) fn run(
        &self,
        source: &str,
        mode: compile::Mode,
        source_path: Option<String>,
    ) -> Result<JsValue, JsValue> {
        self.with_vm(|vm, StoredVirtualMachine { ref scope, .. }| {
            let source_path = source_path.unwrap_or_else(|| "<wasm>".to_owned());
            let code = vm.compile(source, mode, source_path);
            let code = code.map_err(convert::syntax_err)?;
            let result = vm.run_code_obj(code, scope.clone());
            convert::pyresult_to_jsresult(vm, result)
        })?
    }

    pub fn exec(&self, source: &str, source_path: Option<String>) -> Result<JsValue, JsValue> {
        self.run(source, compile::Mode::Exec, source_path)
    }

    pub fn eval(&self, source: &str, source_path: Option<String>) -> Result<JsValue, JsValue> {
        self.run(source, compile::Mode::Eval, source_path)
    }

    #[wasm_bindgen(js_name = execSingle)]
    pub fn exec_single(
        &self,
        source: &str,
        source_path: Option<String>,
    ) -> Result<JsValue, JsValue> {
        self.run(source, compile::Mode::Single, source_path)
    }
}
