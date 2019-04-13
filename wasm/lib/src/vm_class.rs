use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use js_sys::{Object, Reflect, SyntaxError, TypeError};
use wasm_bindgen::prelude::*;

use rustpython_vm::compile;
use rustpython_vm::frame::{NameProtocol, Scope};
use rustpython_vm::function::PyFuncArgs;
use rustpython_vm::pyobject::{PyObject, PyObjectPayload, PyObjectRef, PyResult};
use rustpython_vm::VirtualMachine;

use crate::browser_module::setup_browser_module;
use crate::convert;
use crate::wasm_builtins;

pub(crate) struct StoredVirtualMachine {
    pub vm: VirtualMachine,
    pub scope: RefCell<Scope>,
    /// you can put a Rc in here, keep it as a Weak, and it'll be held only for
    /// as long as the StoredVM is alive
    held_objects: RefCell<Vec<PyObjectRef>>,
}

impl StoredVirtualMachine {
    fn new(id: String, inject_browser_module: bool) -> StoredVirtualMachine {
        let mut vm = VirtualMachine::new();
        let scope = vm.ctx.new_scope();
        if inject_browser_module {
            setup_browser_module(&vm);
        }
        vm.wasm_id = Some(id);
        StoredVirtualMachine {
            vm,
            scope: RefCell::new(scope),
            held_objects: RefCell::new(Vec::new()),
        }
    }
}

// It's fine that it's thread local, since WASM doesn't even have threads yet. thread_local!
// probably gets compiled down to a normal-ish static varible, like Atomic* types do:
// https://rustwasm.github.io/2018/10/24/multithreading-rust-and-wasm.html#atomic-instructions
thread_local! {
    static STORED_VMS: RefCell<HashMap<String, Rc<StoredVirtualMachine>>> = RefCell::default();
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

    pub fn valid(&self) -> bool {
        STORED_VMS.with(|cell| cell.borrow().contains_key(&self.id))
    }

    pub(crate) fn push_held_rc(
        &self,
        obj: PyObjectRef,
    ) -> Result<Weak<PyObject<dyn PyObjectPayload>>, JsValue> {
        self.with(|stored_vm| {
            let weak = Rc::downgrade(&obj);
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
        self.with(
            move |StoredVirtualMachine {
                      ref vm, ref scope, ..
                  }| {
                let value = convert::js_to_py(vm, value);
                scope.borrow_mut().store_name(&vm, &name, value);
            },
        )
    }

    #[wasm_bindgen(js_name = setStdout)]
    pub fn set_stdout(&self, stdout: JsValue) -> Result<(), JsValue> {
        self.with(move |StoredVirtualMachine { ref vm, .. }| {
            fn error() -> JsValue {
                TypeError::new("Unknown stdout option, please pass a function or 'console'").into()
            }
            let print_fn: PyObjectRef = if let Some(s) = stdout.as_string() {
                match s.as_str() {
                    "console" => vm.ctx.new_rustfunc(wasm_builtins::builtin_print_console),
                    _ => return Err(error()),
                }
            } else if stdout.is_function() {
                let func = js_sys::Function::from(stdout);
                vm.ctx
                    .new_rustfunc(move |vm: &VirtualMachine, args: PyFuncArgs| -> PyResult {
                        func.call1(
                            &JsValue::UNDEFINED,
                            &wasm_builtins::format_print_args(vm, args)?.into(),
                        )
                        .map_err(|err| convert::js_to_py(vm, err))?;
                        Ok(vm.get_none())
                    })
            } else if stdout.is_undefined() || stdout.is_null() {
                fn noop(vm: &VirtualMachine, _args: PyFuncArgs) -> PyResult {
                    Ok(vm.get_none())
                }
                vm.ctx.new_rustfunc(noop)
            } else {
                return Err(error());
            };
            vm.set_attr(&vm.builtins, "print", print_fn).unwrap();
            Ok(())
        })?
    }

    #[wasm_bindgen(js_name = injectModule)]
    pub fn inject_module(&self, name: String, module: Object) -> Result<(), JsValue> {
        self.with(|StoredVirtualMachine { ref vm, .. }| {
            let mut module_items: HashMap<String, PyObjectRef> = HashMap::new();
            for entry in convert::object_entries(&module) {
                let (key, value) = entry?;
                let key = Object::from(key).to_string();
                module_items.insert(key.into(), convert::js_to_py(vm, value));
            }

            let mod_name = name.clone();

            let stdlib_init_fn = move |vm: &VirtualMachine| {
                let module = vm.ctx.new_module(&name, vm.ctx.new_dict());
                for (key, value) in module_items.clone() {
                    vm.set_attr(&module, key, value).unwrap();
                }
                module
            };

            vm.stdlib_inits
                .borrow_mut()
                .insert(mod_name, Box::new(stdlib_init_fn));

            Ok(())
        })?
    }

    fn run(&self, mut source: String, mode: compile::Mode) -> Result<JsValue, JsValue> {
        self.assert_valid()?;
        self.with_unchecked(
            |StoredVirtualMachine {
                 ref vm, ref scope, ..
             }| {
                source.push('\n');
                let code = compile::compile(vm, &source, &mode, "<wasm>".to_string());
                let code = code.map_err(|err| {
                    let js_err = SyntaxError::new(&format!("Error parsing Python code: {}", err));
                    if let rustpython_vm::error::CompileError::Parse(ref parse_error) = err {
                        use rustpython_parser::error::ParseError;
                        if let ParseError::EOF(Some(ref loc))
                        | ParseError::ExtraToken((ref loc, ..))
                        | ParseError::InvalidToken(ref loc)
                        | ParseError::UnrecognizedToken((ref loc, ..), _) = parse_error
                        {
                            let _ = Reflect::set(
                                &js_err,
                                &"row".into(),
                                &(loc.get_row() as u32).into(),
                            );
                            let _ = Reflect::set(
                                &js_err,
                                &"col".into(),
                                &(loc.get_column() as u32).into(),
                            );
                        }
                        if let ParseError::ExtraToken((_, _, ref loc))
                        | ParseError::UnrecognizedToken((_, _, ref loc), _) = parse_error
                        {
                            let _ = Reflect::set(
                                &js_err,
                                &"endrow".into(),
                                &(loc.get_row() as u32).into(),
                            );
                            let _ = Reflect::set(
                                &js_err,
                                &"endcol".into(),
                                &(loc.get_column() as u32).into(),
                            );
                        }
                    }
                    js_err
                })?;
                let result = vm.run_code_obj(code, scope.borrow().clone());
                convert::pyresult_to_jsresult(vm, result)
            },
        )
    }

    pub fn exec(&self, source: String) -> Result<JsValue, JsValue> {
        self.run(source, compile::Mode::Exec)
    }

    pub fn eval(&self, source: String) -> Result<JsValue, JsValue> {
        self.run(source, compile::Mode::Eval)
    }

    #[wasm_bindgen(js_name = execSingle)]
    pub fn exec_single(&self, source: String) -> Result<JsValue, JsValue> {
        self.run(source, compile::Mode::Single)
    }
}
