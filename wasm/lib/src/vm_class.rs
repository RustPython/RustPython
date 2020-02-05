use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use js_sys::{Object, TypeError};
use wasm_bindgen::prelude::*;

use rustpython_compiler::compile;
use rustpython_vm::function::PyFuncArgs;
use rustpython_vm::pyobject::{
    ItemProtocol, PyObject, PyObjectPayload, PyObjectRef, PyResult, PyValue,
};
use rustpython_vm::scope::{NameProtocol, Scope};
use rustpython_vm::{InitParameter, PySettings, VirtualMachine};

use crate::browser_module::setup_browser_module;
use crate::convert::{self, PyResultExt};
use crate::js_module;
use crate::wasm_builtins;
use rustpython_compiler::mode::Mode;
use rustpython_vm::obj::objstr::PyStringRef;

pub(crate) struct StoredVirtualMachine {
    pub vm: VirtualMachine,
    pub scope: RefCell<Scope>,
    /// you can put a Rc in here, keep it as a Weak, and it'll be held only for
    /// as long as the StoredVM is alive
    held_objects: RefCell<Vec<PyObjectRef>>,
}

impl StoredVirtualMachine {
    fn new(id: String, inject_browser_module: bool) -> StoredVirtualMachine {
        let mut settings = PySettings::default();

        // After js, browser modules injected, the VM will not be initialized.
        settings.initialization_parameter = InitParameter::NoInitialize;

        let mut vm: VirtualMachine = VirtualMachine::new(settings);

        vm.wasm_id = Some(id);
        let scope = vm.new_scope_with_builtins();

        js_module::setup_js_module(&vm);
        if inject_browser_module {
            vm.stdlib_inits.borrow_mut().insert(
                "_window".to_owned(),
                Box::new(|vm| {
                    py_module!(vm, "_window", {
                        "window" => js_module::PyJsValue::new(wasm_builtins::window()).into_ref(vm),
                    })
                }),
            );
            setup_browser_module(&vm);
        }

        vm.initialize(InitParameter::InitializeInternal);

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
                    "console" => vm.ctx.new_method(wasm_builtins::builtin_print_console),
                    _ => return Err(error()),
                }
            } else if stdout.is_function() {
                let func = js_sys::Function::from(stdout);
                vm.ctx
                    .new_method(move |vm: &VirtualMachine, args: PyFuncArgs| -> PyResult {
                        func.call1(
                            &JsValue::UNDEFINED,
                            &wasm_builtins::format_print_args(vm, args)?.into(),
                        )
                        .map_err(|err| convert::js_py_typeerror(vm, err))?;
                        Ok(vm.get_none())
                    })
            } else if stdout.is_null() {
                fn noop(vm: &VirtualMachine, _args: PyFuncArgs) -> PyResult {
                    Ok(vm.get_none())
                }
                vm.ctx.new_method(noop)
            } else if stdout.is_undefined() {
                vm.ctx.new_method(wasm_builtins::builtin_print_console)
            } else {
                return Err(error());
            };
            vm.set_attr(&vm.builtins, "print", print_fn).unwrap();
            Ok(())
        })?
    }

    #[wasm_bindgen(js_name = injectModule)]
    pub fn inject_module(
        &self,
        name: String,
        source: &str,
        imports: Option<Object>,
        strict_private: Option<bool>,
    ) -> Result<(), JsValue> {
        self.with(|StoredVirtualMachine { ref vm, .. }| {
            let strict_private = strict_private.unwrap_or(false);
            let opts = compile::CompileOpts {
                incognito: strict_private,
                ..vm.compile_opts()
            };
            let code = vm
                .compile_with_opts(source, Mode::Exec, name.clone(), opts)
                .map_err(convert::syntax_err)?;
            let attrs = vm.ctx.new_dict();
            attrs
                .set_item("__name__", vm.new_str(name.clone()), vm)
                .to_js(vm)?;

            if let Some(imports) = imports {
                for entry in convert::object_entries(&imports) {
                    let (key, value) = entry?;
                    let key:String = Object::from(key).to_string().into();
                    attrs.set_item(&key, convert::js_to_py(vm, value), vm).to_js(vm)?;
                }
            }

            vm.run_code_obj(code, Scope::new(None, attrs.clone(), vm))
                .to_js(vm)?;

            let module_attrs = if strict_private {
                let all = attrs
                    .get_item_option("__all__", vm)
                    .to_js(vm)?
                    .ok_or_else(|| {
                        TypeError::new(
                            "you must define __all__ in your module if you pass strict_private: true",
                        )
                    })?;
                let all = vm.extract_elements::<PyStringRef>(&all).to_js(vm)?;

                let actual_attrs = vm.ctx.new_dict();
                for member in all {
                    actual_attrs
                        .set_item(member.as_str(), attrs.get_item(member.as_str(), vm).to_js(vm)?, vm)
                        .to_js(vm)?;
                }
                actual_attrs
            } else {
                attrs
            };

            let module = vm.new_module(&name, module_attrs);

            let sys_modules = vm
                .get_attribute(vm.sys_module.clone(), "modules")
                .to_js(vm)?;
            sys_modules.set_item(&name, module, vm).to_js(vm)?;

            Ok(())
        })?
    }

    #[wasm_bindgen(js_name = injectJSModule)]
    pub fn inject_js_module(&self, name: String, module: Object) -> Result<(), JsValue> {
        self.with(|StoredVirtualMachine { ref vm, .. }| {
            let mut module_items: HashMap<String, PyObjectRef> = HashMap::new();
            for entry in convert::object_entries(&module) {
                let (key, value) = entry?;
                let key = Object::from(key).to_string();
                module_items.insert(key.into(), convert::js_to_py(vm, value));
            }

            let mod_name = name.clone();

            let stdlib_init_fn = move |vm: &VirtualMachine| {
                let module = vm.new_module(&name, vm.ctx.new_dict());
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

    pub(crate) fn run(
        &self,
        source: &str,
        mode: compile::Mode,
        source_path: Option<String>,
    ) -> Result<JsValue, JsValue> {
        self.with(
            |StoredVirtualMachine {
                 ref vm, ref scope, ..
             }| {
                let source_path = source_path.unwrap_or_else(|| "<wasm>".to_owned());
                let code = vm.compile(source, mode, source_path);
                let code = code.map_err(convert::syntax_err)?;
                let result = vm.run_code_obj(code, scope.borrow().clone());
                convert::pyresult_to_jsresult(vm, result)
            },
        )?
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
