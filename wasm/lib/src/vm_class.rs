use crate::browser_module::setup_browser_module;
use crate::convert;
use crate::wasm_builtins;
use js_sys::{Object, Reflect, SyntaxError, TypeError};
use rustpython_vm::{
    compile,
    frame::{NameProtocol, Scope},
    pyobject::{PyContext, PyFuncArgs, PyObjectRef, PyResult},
    VirtualMachine,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use wasm_bindgen::{prelude::*, JsCast};

pub trait HeldRcInner {}

impl<T> HeldRcInner for T {}

pub(crate) struct StoredVirtualMachine {
    pub vm: VirtualMachine,
    pub scope: Scope,
    /// you can put a Rc in here, keep it as a Weak, and it'll be held only for
    /// as long as the StoredVM is alive
    held_rcs: Vec<Rc<dyn HeldRcInner>>,
}

impl StoredVirtualMachine {
    fn new(id: String, inject_browser_module: bool) -> StoredVirtualMachine {
        let mut vm = VirtualMachine::new();
        let scope = vm.ctx.new_scope();
        if inject_browser_module {
            setup_browser_module(&mut vm);
        }
        vm.wasm_id = Some(id);
        StoredVirtualMachine {
            vm,
            scope,
            held_rcs: vec![],
        }
    }
}

// It's fine that it's thread local, since WASM doesn't even have threads yet. thread_local! probably
// gets compiled down to a normal-ish static varible, like Atomic* types:
// https://rustwasm.github.io/2018/10/24/multithreading-rust-and-wasm.html#atomic-instructions
thread_local! {
    static STORED_VMS: Rc<RefCell<HashMap<String, Rc<RefCell<StoredVirtualMachine>>>>> = Rc::default();
    static ACTIVE_VMS: Rc<RefCell<HashMap<String, *mut VirtualMachine>>> = Rc::default();
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
                vms.insert(id.clone(), Rc::new(RefCell::new(stored_vm)));
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

#[derive(Clone)]
pub struct AccessibleVM {
    weak: Weak<RefCell<StoredVirtualMachine>>,
    id: String,
}

impl AccessibleVM {
    pub fn from_id(id: String) -> AccessibleVM {
        let weak = STORED_VMS
            .with(|cell| Rc::downgrade(cell.borrow().get(&id).expect("WASM VM to be valid")));
        AccessibleVM { weak, id }
    }

    pub fn from_vm(vm: &VirtualMachine) -> AccessibleVM {
        AccessibleVM::from_id(
            vm.wasm_id
                .clone()
                .expect("VM passed to from_vm to have wasm_id be Some()"),
        )
    }

    pub fn upgrade(&self) -> Option<AccessibleVMPtr> {
        let vm_cell = self.weak.upgrade()?;
        let top_level = match vm_cell.try_borrow_mut() {
            Ok(mut vm) => {
                ACTIVE_VMS.with(|cell| {
                    cell.borrow_mut().insert(self.id.clone(), &mut vm.vm);
                });
                true
            }
            Err(_) => false,
        };
        Some(ACTIVE_VMS.with(|cell| {
            let vms = cell.borrow();
            let ptr = vms.get(&self.id).expect("id to be in ACTIVE_VMS");
            let vm = unsafe { &mut **ptr };
            AccessibleVMPtr {
                id: self.id.clone(),
                top_level,
                inner: vm,
            }
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

pub struct AccessibleVMPtr<'a> {
    id: String,
    top_level: bool,
    inner: &'a mut VirtualMachine,
}

impl std::ops::Deref for AccessibleVMPtr<'_> {
    type Target = VirtualMachine;
    fn deref(&self) -> &VirtualMachine {
        &self.inner
    }
}
impl std::ops::DerefMut for AccessibleVMPtr<'_> {
    fn deref_mut(&mut self) -> &mut VirtualMachine {
        &mut self.inner
    }
}

impl Drop for AccessibleVMPtr<'_> {
    fn drop(&mut self) {
        if self.top_level {
            // remove the (now invalid) pointer from the map
            ACTIVE_VMS.with(|cell| cell.borrow_mut().remove(&self.id));
        }
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
        F: FnOnce(&mut StoredVirtualMachine) -> R,
    {
        let stored_vm = STORED_VMS.with(|cell| {
            let mut vms = cell.borrow_mut();
            vms.get_mut(&self.id).unwrap().clone()
        });
        let mut stored_vm = stored_vm.borrow_mut();
        f(&mut stored_vm)
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

    pub(crate) fn push_held_rc<T: HeldRcInner + 'static>(
        &self,
        rc: Rc<T>,
    ) -> Result<Weak<T>, JsValue> {
        self.with(|stored_vm| {
            let weak = Rc::downgrade(&rc);
            stored_vm.held_rcs.push(rc);
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
                      ref mut vm,
                      ref mut scope,
                      ..
                  }| {
                let value = convert::js_to_py(vm, value);
                scope.store_name(&vm, &name, value);
            },
        )
    }

    #[wasm_bindgen(js_name = setStdout)]
    pub fn set_stdout(&self, stdout: JsValue) -> Result<(), JsValue> {
        self.with(
            move |StoredVirtualMachine {
                      ref mut vm,
                      ref mut scope,
                      ..
                  }| {
                fn error() -> JsValue {
                    TypeError::new("Unknown stdout option, please pass a function or 'console'")
                        .into()
                }
                let print_fn: Box<Fn(&mut VirtualMachine, PyFuncArgs) -> PyResult> =
                    if let Some(s) = stdout.as_string() {
                        match s.as_str() {
                            "console" => Box::new(wasm_builtins::builtin_print_console),
                            _ => return Err(error()),
                        }
                    } else if stdout.is_function() {
                        let func = js_sys::Function::from(stdout);
                        Box::new(
                            move |vm: &mut VirtualMachine, args: PyFuncArgs| -> PyResult {
                                func.call1(
                                    &JsValue::UNDEFINED,
                                    &wasm_builtins::format_print_args(vm, args)?.into(),
                                )
                                .map_err(|err| convert::js_to_py(vm, err))?;
                                Ok(vm.get_none())
                            },
                        )
                    } else if stdout.is_undefined() || stdout.is_null() {
                        fn noop(vm: &mut VirtualMachine, _args: PyFuncArgs) -> PyResult {
                            Ok(vm.get_none())
                        }
                        Box::new(noop)
                    } else {
                        return Err(error());
                    };
                scope.store_name(&vm, "print", vm.ctx.new_rustfunc(print_fn));
                Ok(())
            },
        )?
    }

    #[wasm_bindgen(js_name = injectModule)]
    pub fn inject_module(&self, name: String, module: Object) -> Result<(), JsValue> {
        self.with(|StoredVirtualMachine { ref mut vm, .. }| {
            let mut module_items: HashMap<String, PyObjectRef> = HashMap::new();
            for entry in convert::object_entries(&module) {
                let (key, value) = entry?;
                let key = Object::from(key).to_string();
                module_items.insert(key.into(), convert::js_to_py(vm, value));
            }

            let mod_name = name.clone();

            let stdlib_init_fn = move |ctx: &PyContext| {
                let py_mod = ctx.new_module(&name, ctx.new_dict());
                for (key, value) in module_items.clone() {
                    ctx.set_attr(&py_mod, &key, value);
                }
                py_mod
            };

            vm.stdlib_inits.insert(mod_name, Box::new(stdlib_init_fn));

            Ok(())
        })?
    }

    fn run(&self, mut source: String, mode: compile::Mode) -> Result<JsValue, JsValue> {
        self.assert_valid()?;
        self.with_unchecked(
            |StoredVirtualMachine {
                 ref mut vm,
                 ref mut scope,
                 ..
             }| {
                source.push('\n');
                let code =
                    compile::compile(&source, &mode, "<wasm>".to_string(), vm.ctx.code_type());
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
                let result = vm.run_code_obj(code, scope.clone());
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
}
