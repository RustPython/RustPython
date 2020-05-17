//! Implement virtual machine to run instructions.
//!
//! See also:
//!   https://github.com/ProgVal/pythonvm-rust/blob/master/src/processor/mod.rs
//!

use std::cell::{Cell, Ref, RefCell};
use std::collections::hash_map::HashMap;
use std::collections::hash_set::HashSet;
use std::sync::{Arc, Mutex, MutexGuard};
use std::{env, fmt};

use arr_macro::arr;
use num_bigint::BigInt;
use num_traits::ToPrimitive;
use once_cell::sync::Lazy;
#[cfg(feature = "rustpython-compiler")]
use rustpython_compiler::{
    compile::{self, CompileOpts},
    error::CompileError,
};

use crate::builtins::{self, to_ascii};
use crate::bytecode;
use crate::exceptions::{self, PyBaseException, PyBaseExceptionRef};
use crate::frame::{ExecutionResult, Frame, FrameRef};
use crate::frozen;
use crate::function::{OptionalArg, PyFuncArgs};
use crate::import;
use crate::obj::objbool;
use crate::obj::objcode::{PyCode, PyCodeRef};
use crate::obj::objdict::PyDictRef;
use crate::obj::objint::{PyInt, PyIntRef};
use crate::obj::objiter;
use crate::obj::objlist::PyList;
use crate::obj::objmodule::{self, PyModule};
use crate::obj::objobject;
use crate::obj::objstr::{PyString, PyStringRef};
use crate::obj::objtuple::PyTuple;
use crate::obj::objtype::{self, PyClassRef};
use crate::pyhash;
use crate::pyobject::{
    IdProtocol, ItemProtocol, PyContext, PyObject, PyObjectRef, PyResult, PyValue, TryFromObject,
    TryIntoRef, TypeProtocol,
};
use crate::scope::Scope;
use crate::stdlib;
use crate::sysmodule;

// use objects::objects;

// Objects are live when they are on stack, or referenced by a name (for now)

/// Top level container of a python virtual machine. In theory you could
/// create more instances of this struct and have them operate fully isolated.
pub struct VirtualMachine {
    pub builtins: PyObjectRef,
    pub sys_module: PyObjectRef,
    pub ctx: Arc<PyContext>,
    pub frames: RefCell<Vec<FrameRef>>,
    pub wasm_id: Option<String>,
    pub exceptions: RefCell<Vec<PyBaseExceptionRef>>,
    pub import_func: PyObjectRef,
    pub profile_func: RefCell<PyObjectRef>,
    pub trace_func: RefCell<PyObjectRef>,
    pub use_tracing: Cell<bool>,
    pub recursion_limit: Cell<usize>,
    pub signal_handlers: Option<RefCell<[PyObjectRef; NSIG]>>,
    pub state: Arc<PyGlobalState>,
    pub initialized: bool,
}

pub struct PyGlobalState {
    pub settings: PySettings,
    pub stdlib_inits: HashMap<String, stdlib::StdlibInitFunc>,
    pub frozen: HashMap<String, bytecode::FrozenModule>,
}

pub const NSIG: usize = 64;

#[derive(Copy, Clone)]
pub enum InitParameter {
    NoInitialize,
    InitializeInternal,
    InitializeExternal,
}

/// Struct containing all kind of settings for the python vm.
pub struct PySettings {
    /// -d command line switch
    pub debug: bool,

    /// -i
    pub inspect: bool,

    /// -O optimization switch counter
    pub optimize: u8,

    /// -s
    pub no_user_site: bool,

    /// -S
    pub no_site: bool,

    /// -E
    pub ignore_environment: bool,

    /// verbosity level (-v switch)
    pub verbose: u8,

    /// -q
    pub quiet: bool,

    /// -B
    pub dont_write_bytecode: bool,

    /// Environment PYTHONPATH and RUSTPYTHONPATH:
    pub path_list: Vec<String>,

    /// sys.argv
    pub argv: Vec<String>,

    /// Initialization parameter to decide to initialize or not,
    /// and to decide the importer required external filesystem access or not
    pub initialization_parameter: InitParameter,
}

/// Trace events for sys.settrace and sys.setprofile.
enum TraceEvent {
    Call,
    Return,
}

impl fmt::Display for TraceEvent {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use TraceEvent::*;
        match self {
            Call => write!(f, "call"),
            Return => write!(f, "return"),
        }
    }
}

/// Sensible default settings.
impl Default for PySettings {
    fn default() -> Self {
        PySettings {
            debug: false,
            inspect: false,
            optimize: 0,
            no_user_site: false,
            no_site: false,
            ignore_environment: false,
            verbose: 0,
            quiet: false,
            dont_write_bytecode: false,
            path_list: vec![],
            argv: vec![],
            initialization_parameter: InitParameter::InitializeExternal,
        }
    }
}

impl VirtualMachine {
    /// Create a new `VirtualMachine` structure.
    pub fn new(settings: PySettings) -> VirtualMachine {
        flame_guard!("new VirtualMachine");
        let ctx = PyContext::new();

        // make a new module without access to the vm; doesn't
        // set __spec__, __loader__, etc. attributes
        let new_module =
            |dict| PyObject::new(PyModule {}, ctx.types.module_type.clone(), Some(dict));

        // Hard-core modules:
        let builtins_dict = ctx.new_dict();
        let builtins = new_module(builtins_dict.clone());
        let sysmod_dict = ctx.new_dict();
        let sysmod = new_module(sysmod_dict.clone());

        let import_func = ctx.none();
        let profile_func = RefCell::new(ctx.none());
        let trace_func = RefCell::new(ctx.none());
        let signal_handlers = RefCell::new(arr![ctx.none(); 64]);
        let initialize_parameter = settings.initialization_parameter;

        let stdlib_inits = stdlib::get_module_inits();
        let frozen = frozen::get_module_inits();

        let mut vm = VirtualMachine {
            builtins: builtins.clone(),
            sys_module: sysmod.clone(),
            ctx: Arc::new(ctx),
            frames: RefCell::new(vec![]),
            wasm_id: None,
            exceptions: RefCell::new(vec![]),
            import_func,
            profile_func,
            trace_func,
            use_tracing: Cell::new(false),
            recursion_limit: Cell::new(if cfg!(debug_assertions) { 256 } else { 512 }),
            signal_handlers: Some(signal_handlers),
            state: Arc::new(PyGlobalState {
                settings,
                stdlib_inits,
                frozen,
            }),
            initialized: false,
        };

        objmodule::init_module_dict(
            &vm,
            &builtins_dict,
            vm.new_str("builtins".to_owned()),
            vm.get_none(),
        );
        objmodule::init_module_dict(
            &vm,
            &sysmod_dict,
            vm.new_str("sys".to_owned()),
            vm.get_none(),
        );
        vm.initialize(initialize_parameter);
        vm
    }

    pub fn initialize(&mut self, initialize_parameter: InitParameter) {
        flame_guard!("init VirtualMachine");

        match initialize_parameter {
            InitParameter::NoInitialize => {}
            _ => {
                if self.initialized {
                    panic!("Double Initialize Error");
                }

                builtins::make_module(self, self.builtins.clone());
                sysmodule::make_module(self, self.sys_module.clone(), self.builtins.clone());

                let mut inner_init = || -> PyResult<()> {
                    #[cfg(not(target_arch = "wasm32"))]
                    import::import_builtin(self, "signal")?;

                    import::init_importlib(self, initialize_parameter)?;

                    #[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
                    {
                        let io = self.import("io", &[], 0)?;
                        let io_open = self.get_attribute(io.clone(), "open")?;
                        let set_stdio = |name, fd, mode: &str| {
                            let stdio = self.invoke(
                                &io_open,
                                vec![self.new_int(fd), self.new_str(mode.to_owned())],
                            )?;
                            self.set_attr(
                                &self.sys_module,
                                format!("__{}__", name), // e.g. __stdin__
                                stdio.clone(),
                            )?;
                            self.set_attr(&self.sys_module, name, stdio)?;
                            Ok(())
                        };
                        set_stdio("stdin", 0, "r")?;
                        set_stdio("stdout", 1, "w")?;
                        set_stdio("stderr", 2, "w")?;

                        let open_wrapper = self.get_attribute(io, "OpenWrapper")?;
                        self.set_attr(&self.builtins, "open", open_wrapper)?;
                    }

                    Ok(())
                };

                let res = inner_init();

                self.expect_pyresult(res, "initializiation failed");

                self.initialized = true;
            }
        }
    }

    pub(crate) fn new_thread(&self) -> VirtualMachine {
        VirtualMachine {
            builtins: self.builtins.clone(),
            sys_module: self.sys_module.clone(),
            ctx: self.ctx.clone(),
            frames: RefCell::new(vec![]),
            wasm_id: self.wasm_id.clone(),
            exceptions: RefCell::new(vec![]),
            import_func: self.import_func.clone(),
            profile_func: RefCell::new(self.get_none()),
            trace_func: RefCell::new(self.get_none()),
            use_tracing: Cell::new(false),
            recursion_limit: self.recursion_limit.clone(),
            signal_handlers: None,
            state: self.state.clone(),
            initialized: self.initialized,
        }
    }

    pub fn run_code_obj(&self, code: PyCodeRef, scope: Scope) -> PyResult {
        let frame = Frame::new(code, scope).into_ref(self);
        self.run_frame_full(frame)
    }

    pub fn run_frame_full(&self, frame: FrameRef) -> PyResult {
        match self.run_frame(frame)? {
            ExecutionResult::Return(value) => Ok(value),
            _ => panic!("Got unexpected result from function"),
        }
    }

    pub fn with_frame<R, F: FnOnce(FrameRef) -> PyResult<R>>(
        &self,
        frame: FrameRef,
        f: F,
    ) -> PyResult<R> {
        self.check_recursive_call("")?;
        self.frames.borrow_mut().push(frame.clone());
        let result = f(frame);
        self.frames.borrow_mut().pop();
        result
    }

    pub fn run_frame(&self, frame: FrameRef) -> PyResult<ExecutionResult> {
        self.with_frame(frame, |f| f.run(self))
    }

    fn check_recursive_call(&self, _where: &str) -> PyResult<()> {
        if self.frames.borrow().len() > self.recursion_limit.get() {
            Err(self.new_recursion_error(format!("maximum recursion depth exceeded {}", _where)))
        } else {
            Ok(())
        }
    }

    pub fn current_frame(&self) -> Option<Ref<FrameRef>> {
        let frames = self.frames.borrow();
        if frames.is_empty() {
            None
        } else {
            Some(Ref::map(self.frames.borrow(), |frames| {
                frames.last().unwrap()
            }))
        }
    }

    pub fn current_scope(&self) -> Ref<Scope> {
        let frame = self
            .current_frame()
            .expect("called current_scope but no frames on the stack");
        Ref::map(frame, |f| &f.scope)
    }

    pub fn try_class(&self, module: &str, class: &str) -> PyResult<PyClassRef> {
        let class = self
            .get_attribute(self.import(module, &[], 0)?, class)?
            .downcast()
            .expect("not a class");
        Ok(class)
    }

    pub fn class(&self, module: &str, class: &str) -> PyClassRef {
        let module = self
            .import(module, &[], 0)
            .unwrap_or_else(|_| panic!("unable to import {}", module));
        let class = self
            .get_attribute(module.clone(), class)
            .unwrap_or_else(|_| panic!("module {} has no class {}", module, class));
        class.downcast().expect("not a class")
    }

    /// Create a new python string object.
    pub fn new_str(&self, s: String) -> PyObjectRef {
        self.ctx.new_str(s)
    }

    /// Create a new python int object.
    #[inline]
    pub fn new_int<T: Into<BigInt> + ToPrimitive>(&self, i: T) -> PyObjectRef {
        self.ctx.new_int(i)
    }

    /// Create a new python bool object.
    #[inline]
    pub fn new_bool(&self, b: bool) -> PyObjectRef {
        self.ctx.new_bool(b)
    }

    pub fn new_module(&self, name: &str, dict: PyDictRef) -> PyObjectRef {
        objmodule::init_module_dict(self, &dict, self.new_str(name.to_owned()), self.get_none());
        PyObject::new(PyModule {}, self.ctx.types.module_type.clone(), Some(dict))
    }

    /// Instantiate an exception with arguments.
    /// This function should only be used with builtin exception types; if a user-defined exception
    /// type is passed in, it may not be fully initialized; try using [`exceptions::invoke`](invoke)
    /// or [`exceptions::ExceptionCtor`](ctor) instead.
    ///
    /// [invoke]: rustpython_vm::exceptions::invoke
    /// [ctor]: rustpython_vm::exceptions::ExceptionCtor
    pub fn new_exception(
        &self,
        exc_type: PyClassRef,
        args: Vec<PyObjectRef>,
    ) -> PyBaseExceptionRef {
        // TODO: add repr of args into logging?
        vm_trace!("New exception created: {}", exc_type.name);
        PyBaseException::new(args, self)
            .into_ref_with_type_unchecked(exc_type, Some(self.ctx.new_dict()))
    }

    /// Instantiate an exception with no arguments.
    /// This function should only be used with builtin exception types; if a user-defined exception
    /// type is passed in, it may not be fully initialized; try using [`exceptions::invoke`](invoke)
    /// or [`exceptions::ExceptionCtor`](ctor) instead.
    ///
    /// [invoke]: rustpython_vm::exceptions::invoke
    /// [ctor]: rustpython_vm::exceptions::ExceptionCtor
    pub fn new_exception_empty(&self, exc_type: PyClassRef) -> PyBaseExceptionRef {
        self.new_exception(exc_type, vec![])
    }

    /// Instantiate an exception with `msg` as the only argument.
    /// This function should only be used with builtin exception types; if a user-defined exception
    /// type is passed in, it may not be fully initialized; try using [`exceptions::invoke`](invoke)
    /// or [`exceptions::ExceptionCtor`](ctor) instead.
    ///
    /// [invoke]: rustpython_vm::exceptions::invoke
    /// [ctor]: rustpython_vm::exceptions::ExceptionCtor
    pub fn new_exception_msg(&self, exc_type: PyClassRef, msg: String) -> PyBaseExceptionRef {
        self.new_exception(exc_type, vec![self.new_str(msg)])
    }

    pub fn new_lookup_error(&self, msg: String) -> PyBaseExceptionRef {
        let lookup_error = self.ctx.exceptions.lookup_error.clone();
        self.new_exception_msg(lookup_error, msg)
    }

    pub fn new_attribute_error(&self, msg: String) -> PyBaseExceptionRef {
        let attribute_error = self.ctx.exceptions.attribute_error.clone();
        self.new_exception_msg(attribute_error, msg)
    }

    pub fn new_type_error(&self, msg: String) -> PyBaseExceptionRef {
        let type_error = self.ctx.exceptions.type_error.clone();
        self.new_exception_msg(type_error, msg)
    }

    pub fn new_name_error(&self, msg: String) -> PyBaseExceptionRef {
        let name_error = self.ctx.exceptions.name_error.clone();
        self.new_exception_msg(name_error, msg)
    }

    pub fn new_unsupported_operand_error(
        &self,
        a: PyObjectRef,
        b: PyObjectRef,
        op: &str,
    ) -> PyBaseExceptionRef {
        self.new_type_error(format!(
            "Unsupported operand types for '{}': '{}' and '{}'",
            op,
            a.class().name,
            b.class().name
        ))
    }

    pub fn new_os_error(&self, msg: String) -> PyBaseExceptionRef {
        let os_error = self.ctx.exceptions.os_error.clone();
        self.new_exception_msg(os_error, msg)
    }

    pub fn new_unicode_decode_error(&self, msg: String) -> PyBaseExceptionRef {
        let unicode_decode_error = self.ctx.exceptions.unicode_decode_error.clone();
        self.new_exception_msg(unicode_decode_error, msg)
    }

    pub fn new_unicode_encode_error(&self, msg: String) -> PyBaseExceptionRef {
        let unicode_encode_error = self.ctx.exceptions.unicode_encode_error.clone();
        self.new_exception_msg(unicode_encode_error, msg)
    }

    /// Create a new python ValueError object. Useful for raising errors from
    /// python functions implemented in rust.
    pub fn new_value_error(&self, msg: String) -> PyBaseExceptionRef {
        let value_error = self.ctx.exceptions.value_error.clone();
        self.new_exception_msg(value_error, msg)
    }

    pub fn new_key_error(&self, obj: PyObjectRef) -> PyBaseExceptionRef {
        let key_error = self.ctx.exceptions.key_error.clone();
        self.new_exception(key_error, vec![obj])
    }

    pub fn new_index_error(&self, msg: String) -> PyBaseExceptionRef {
        let index_error = self.ctx.exceptions.index_error.clone();
        self.new_exception_msg(index_error, msg)
    }

    pub fn new_not_implemented_error(&self, msg: String) -> PyBaseExceptionRef {
        let not_implemented_error = self.ctx.exceptions.not_implemented_error.clone();
        self.new_exception_msg(not_implemented_error, msg)
    }

    pub fn new_recursion_error(&self, msg: String) -> PyBaseExceptionRef {
        let recursion_error = self.ctx.exceptions.recursion_error.clone();
        self.new_exception_msg(recursion_error, msg)
    }

    pub fn new_zero_division_error(&self, msg: String) -> PyBaseExceptionRef {
        let zero_division_error = self.ctx.exceptions.zero_division_error.clone();
        self.new_exception_msg(zero_division_error, msg)
    }

    pub fn new_overflow_error(&self, msg: String) -> PyBaseExceptionRef {
        let overflow_error = self.ctx.exceptions.overflow_error.clone();
        self.new_exception_msg(overflow_error, msg)
    }

    #[cfg(feature = "rustpython-compiler")]
    pub fn new_syntax_error(&self, error: &CompileError) -> PyBaseExceptionRef {
        let syntax_error_type = if error.is_indentation_error() {
            self.ctx.exceptions.indentation_error.clone()
        } else if error.is_tab_error() {
            self.ctx.exceptions.tab_error.clone()
        } else {
            self.ctx.exceptions.syntax_error.clone()
        };
        let syntax_error = self.new_exception_msg(syntax_error_type, error.to_string());
        let lineno = self.new_int(error.location.row());
        let offset = self.new_int(error.location.column());
        self.set_attr(syntax_error.as_object(), "lineno", lineno)
            .unwrap();
        self.set_attr(syntax_error.as_object(), "offset", offset)
            .unwrap();
        if let Some(v) = error.statement.as_ref() {
            self.set_attr(syntax_error.as_object(), "text", self.new_str(v.to_owned()))
                .unwrap();
        }
        if let Some(path) = error.source_path.as_ref() {
            self.set_attr(
                syntax_error.as_object(),
                "filename",
                self.new_str(path.to_owned()),
            )
            .unwrap();
        }
        syntax_error
    }

    pub fn new_import_error(&self, msg: String) -> PyBaseExceptionRef {
        let import_error = self.ctx.exceptions.import_error.clone();
        self.new_exception_msg(import_error, msg)
    }

    pub fn new_runtime_error(&self, msg: String) -> PyBaseExceptionRef {
        let runtime_error = self.ctx.exceptions.runtime_error.clone();
        self.new_exception_msg(runtime_error, msg)
    }

    // TODO: #[track_caller] when stabilized
    fn _py_panic_failed(&self, exc: &PyBaseExceptionRef, msg: &str) -> ! {
        #[cfg(not(all(target_arch = "wasm32", not(target_os = "wasi"))))]
        {
            let show_backtrace = env::var_os("RUST_BACKTRACE").map_or(false, |v| &v != "0");
            let after = if show_backtrace {
                exceptions::print_exception(self, exc);
                "exception backtrace above"
            } else {
                "run with RUST_BACKTRACE=1 to see Python backtrace"
            };
            panic!("{}; {}", msg, after)
        }
        #[cfg(all(target_arch = "wasm32", not(target_os = "wasi")))]
        {
            use wasm_bindgen::prelude::*;
            #[wasm_bindgen]
            extern "C" {
                #[wasm_bindgen(js_namespace = console)]
                fn error(s: &str);
            }
            let mut s = Vec::<u8>::new();
            exceptions::write_exception(&mut s, self, exc).unwrap();
            error(std::str::from_utf8(&s).unwrap());
            panic!("{}; exception backtrace above", msg)
        }
    }
    pub fn unwrap_pyresult<T>(&self, result: PyResult<T>) -> T {
        result.unwrap_or_else(|exc| {
            self._py_panic_failed(&exc, "called `vm.unwrap_pyresult()` on an `Err` value")
        })
    }
    pub fn expect_pyresult<T>(&self, result: PyResult<T>, msg: &str) -> T {
        result.unwrap_or_else(|exc| self._py_panic_failed(&exc, msg))
    }

    pub fn new_scope_with_builtins(&self) -> Scope {
        Scope::with_builtins(None, self.ctx.new_dict(), self)
    }

    pub fn get_none(&self) -> PyObjectRef {
        self.ctx.none()
    }

    /// Test whether a python object is `None`.
    pub fn is_none(&self, obj: &PyObjectRef) -> bool {
        obj.is(&self.get_none())
    }
    pub fn option_if_none(&self, obj: PyObjectRef) -> Option<PyObjectRef> {
        if self.is_none(&obj) {
            None
        } else {
            Some(obj)
        }
    }

    pub fn get_type(&self) -> PyClassRef {
        self.ctx.type_type()
    }

    pub fn get_object(&self) -> PyClassRef {
        self.ctx.object()
    }

    pub fn get_locals(&self) -> PyDictRef {
        self.current_scope().get_locals()
    }

    pub fn context(&self) -> &PyContext {
        &self.ctx
    }

    // Container of the virtual machine state:
    pub fn to_str(&self, obj: &PyObjectRef) -> PyResult<PyStringRef> {
        if obj.class().is(&self.ctx.types.str_type) {
            Ok(obj.clone().downcast().unwrap())
        } else {
            let s = self.call_method(&obj, "__str__", vec![])?;
            PyStringRef::try_from_object(self, s)
        }
    }

    pub fn to_pystr<'a, T: Into<&'a PyObjectRef>>(&'a self, obj: T) -> PyResult<String> {
        let py_str_obj = self.to_str(obj.into())?;
        Ok(py_str_obj.as_str().to_owned())
    }

    pub fn to_repr(&self, obj: &PyObjectRef) -> PyResult<PyStringRef> {
        let repr = self.call_method(obj, "__repr__", vec![])?;
        TryFromObject::try_from_object(self, repr)
    }

    pub fn to_ascii(&self, obj: &PyObjectRef) -> PyResult {
        let repr = self.call_method(obj, "__repr__", vec![])?;
        let repr: PyStringRef = TryFromObject::try_from_object(self, repr)?;
        let ascii = to_ascii(repr.as_str());
        Ok(self.new_str(ascii))
    }

    pub fn to_index(&self, obj: &PyObjectRef) -> Option<PyResult<PyIntRef>> {
        Some(
            if let Ok(val) = TryFromObject::try_from_object(self, obj.clone()) {
                Ok(val)
            } else {
                let cls = obj.class();
                if cls.has_attr("__index__") {
                    self.call_method(obj, "__index__", vec![]).and_then(|r| {
                        if let Ok(val) = TryFromObject::try_from_object(self, r) {
                            Ok(val)
                        } else {
                            Err(self.new_type_error(format!(
                                "__index__ returned non-int (type {})",
                                cls.name
                            )))
                        }
                    })
                } else {
                    return None;
                }
            },
        )
    }

    pub fn import(&self, module: &str, from_list: &[String], level: usize) -> PyResult {
        // if the import inputs seem weird, e.g a package import or something, rather than just
        // a straight `import ident`
        let weird = module.contains('.') || level != 0 || !from_list.is_empty();

        let cached_module = if weird {
            None
        } else {
            let sys_modules = self.get_attribute(self.sys_module.clone(), "modules")?;
            sys_modules.get_item(module, self).ok()
        };

        match cached_module {
            Some(module) => Ok(module),
            None => {
                let import_func = self
                    .get_attribute(self.builtins.clone(), "__import__")
                    .map_err(|_| self.new_import_error("__import__ not found".to_owned()))?;

                let (locals, globals) = if let Some(frame) = self.current_frame() {
                    (
                        frame.scope.get_locals().into_object(),
                        frame.scope.globals.clone().into_object(),
                    )
                } else {
                    (self.get_none(), self.get_none())
                };
                let from_list = self.ctx.new_tuple(
                    from_list
                        .iter()
                        .map(|name| self.new_str(name.to_owned()))
                        .collect(),
                );
                self.invoke(
                    &import_func,
                    vec![
                        self.new_str(module.to_owned()),
                        globals,
                        locals,
                        from_list,
                        self.ctx.new_int(level),
                    ],
                )
                .map_err(|exc| import::remove_importlib_frames(self, &exc))
            }
        }
    }

    /// Determines if `obj` is an instance of `cls`, either directly, indirectly or virtually via
    /// the __instancecheck__ magic method.
    pub fn isinstance(&self, obj: &PyObjectRef, cls: &PyClassRef) -> PyResult<bool> {
        // cpython first does an exact check on the type, although documentation doesn't state that
        // https://github.com/python/cpython/blob/a24107b04c1277e3c1105f98aff5bfa3a98b33a0/Objects/abstract.c#L2408
        if Arc::ptr_eq(&obj.class().into_object(), cls.as_object()) {
            Ok(true)
        } else {
            let ret = self.call_method(cls.as_object(), "__instancecheck__", vec![obj.clone()])?;
            objbool::boolval(self, ret)
        }
    }

    /// Determines if `subclass` is a subclass of `cls`, either directly, indirectly or virtually
    /// via the __subclasscheck__ magic method.
    pub fn issubclass(&self, subclass: &PyClassRef, cls: &PyClassRef) -> PyResult<bool> {
        let ret = self.call_method(
            cls.as_object(),
            "__subclasscheck__",
            vec![subclass.clone().into_object()],
        )?;
        objbool::boolval(self, ret)
    }

    pub fn call_get_descriptor_specific(
        &self,
        descr: PyObjectRef,
        obj: Option<PyObjectRef>,
        cls: Option<PyObjectRef>,
    ) -> Option<PyResult> {
        let descr_class = descr.class();
        let slots = descr_class.slots.read().unwrap();
        if let Some(descr_get) = slots.descr_get.as_ref() {
            Some(descr_get(self, descr, obj, OptionalArg::from_option(cls)))
        } else if let Some(ref descriptor) = descr_class.get_attr("__get__") {
            Some(self.invoke(
                descriptor,
                vec![
                    descr,
                    obj.unwrap_or_else(|| self.get_none()),
                    cls.unwrap_or_else(|| self.get_none()),
                ],
            ))
        } else {
            None
        }
    }

    pub fn call_get_descriptor(&self, descr: PyObjectRef, obj: PyObjectRef) -> Option<PyResult> {
        self.call_get_descriptor_specific(descr, Some(obj.clone()), Some(obj.class().into_object()))
    }

    pub fn call_if_get_descriptor(&self, attr: PyObjectRef, obj: PyObjectRef) -> PyResult {
        self.call_get_descriptor(attr.clone(), obj)
            .unwrap_or(Ok(attr))
    }

    pub fn call_method<T>(&self, obj: &PyObjectRef, method_name: &str, args: T) -> PyResult
    where
        T: Into<PyFuncArgs>,
    {
        flame_guard!(format!("call_method({:?})", method_name));

        // This is only used in the vm for magic methods, which use a greatly simplified attribute lookup.
        let cls = obj.class();
        match cls.get_attr(method_name) {
            Some(func) => {
                vm_trace!(
                    "vm.call_method {:?} {:?} {:?} -> {:?}",
                    obj,
                    cls,
                    method_name,
                    func
                );
                let wrapped = self.call_if_get_descriptor(func, obj.clone())?;
                self.invoke(&wrapped, args)
            }
            None => Err(self.new_type_error(format!("Unsupported method: {}", method_name))),
        }
    }

    fn _invoke(&self, callable: &PyObjectRef, args: PyFuncArgs) -> PyResult {
        vm_trace!("Invoke: {:?} {:?}", callable, args);
        if let Some(slot_call) = callable.class().slots.read().unwrap().call.as_ref() {
            self.trace_event(TraceEvent::Call)?;
            let args = args.insert(callable.clone());
            let result = slot_call(self, args);
            self.trace_event(TraceEvent::Return)?;
            result
        } else if callable.class().has_attr("__call__") {
            self.call_method(&callable, "__call__", args)
        } else {
            Err(self.new_type_error(format!(
                "'{}' object is not callable",
                callable.class().name
            )))
        }
    }

    #[inline]
    pub fn invoke<T>(&self, func_ref: &PyObjectRef, args: T) -> PyResult
    where
        T: Into<PyFuncArgs>,
    {
        self._invoke(func_ref, args.into())
    }

    /// Call registered trace function.
    fn trace_event(&self, event: TraceEvent) -> PyResult<()> {
        if self.use_tracing.get() {
            let frame = self.get_none();
            let event = self.new_str(event.to_string());
            let arg = self.get_none();
            let args = vec![frame, event, arg];

            // temporarily disable tracing, during the call to the
            // tracing function itself.
            let trace_func = self.trace_func.borrow().clone();
            if !self.is_none(&trace_func) {
                self.use_tracing.set(false);
                let res = self.invoke(&trace_func, args.clone());
                self.use_tracing.set(true);
                res?;
            }

            let profile_func = self.profile_func.borrow().clone();
            if !self.is_none(&profile_func) {
                self.use_tracing.set(false);
                let res = self.invoke(&profile_func, args);
                self.use_tracing.set(true);
                res?;
            }
        }
        Ok(())
    }

    pub fn extract_elements<T: TryFromObject>(&self, value: &PyObjectRef) -> PyResult<Vec<T>> {
        // Extract elements from item, if possible:
        let cls = value.class();
        if cls.is(&self.ctx.tuple_type()) {
            value
                .payload::<PyTuple>()
                .unwrap()
                .as_slice()
                .iter()
                .map(|obj| T::try_from_object(self, obj.clone()))
                .collect()
        } else if cls.is(&self.ctx.list_type()) {
            value
                .payload::<PyList>()
                .unwrap()
                .borrow_elements()
                .iter()
                .map(|obj| T::try_from_object(self, obj.clone()))
                .collect()
        } else {
            let iter = objiter::get_iter(self, value)?;
            objiter::get_all(self, &iter)
        }
    }

    // get_attribute should be used for full attribute access (usually from user code).
    #[cfg_attr(feature = "flame-it", flame("VirtualMachine"))]
    pub fn get_attribute<T>(&self, obj: PyObjectRef, attr_name: T) -> PyResult
    where
        T: TryIntoRef<PyString>,
    {
        let attr_name = attr_name.try_into_ref(self)?;
        vm_trace!("vm.__getattribute__: {:?} {:?}", obj, attr_name);
        self.call_method(&obj, "__getattribute__", vec![attr_name.into_object()])
    }

    pub fn set_attr<K, V>(&self, obj: &PyObjectRef, attr_name: K, attr_value: V) -> PyResult
    where
        K: TryIntoRef<PyString>,
        V: Into<PyObjectRef>,
    {
        let attr_name = attr_name.try_into_ref(self)?;
        self.call_method(
            obj,
            "__setattr__",
            vec![attr_name.into_object(), attr_value.into()],
        )
    }

    pub fn del_attr(&self, obj: &PyObjectRef, attr_name: PyObjectRef) -> PyResult<()> {
        self.call_method(&obj, "__delattr__", vec![attr_name])?;
        Ok(())
    }

    // get_method should be used for internal access to magic methods (by-passing
    // the full getattribute look-up.
    pub fn get_method_or_type_error<F>(
        &self,
        obj: PyObjectRef,
        method_name: &str,
        err_msg: F,
    ) -> PyResult
    where
        F: FnOnce() -> String,
    {
        let cls = obj.class();
        match cls.get_attr(method_name) {
            Some(method) => self.call_if_get_descriptor(method, obj.clone()),
            None => Err(self.new_type_error(err_msg())),
        }
    }

    /// May return exception, if `__get__` descriptor raises one
    pub fn get_method(&self, obj: PyObjectRef, method_name: &str) -> Option<PyResult> {
        let cls = obj.class();
        let method = cls.get_attr(method_name)?;
        Some(self.call_if_get_descriptor(method, obj.clone()))
    }

    /// Calls a method on `obj` passing `arg`, if the method exists.
    ///
    /// Otherwise, or if the result is the special `NotImplemented` built-in constant,
    /// calls `unsupported` to determine fallback value.
    pub fn call_or_unsupported<F>(
        &self,
        obj: PyObjectRef,
        arg: PyObjectRef,
        method: &str,
        unsupported: F,
    ) -> PyResult
    where
        F: Fn(&VirtualMachine, PyObjectRef, PyObjectRef) -> PyResult,
    {
        if let Some(method_or_err) = self.get_method(obj.clone(), method) {
            let method = method_or_err?;
            let result = self.invoke(&method, vec![arg.clone()])?;
            if !result.is(&self.ctx.not_implemented()) {
                return Ok(result);
            }
        }
        unsupported(self, obj, arg)
    }

    /// Calls a method, falling back to its reflection with the operands
    /// reversed, and then to the value provided by `unsupported`.
    ///
    /// For example: the following:
    ///
    /// `call_or_reflection(lhs, rhs, "__and__", "__rand__", unsupported)`
    ///
    /// 1. Calls `__and__` with `lhs` and `rhs`.
    /// 2. If above is not implemented, calls `__rand__` with `rhs` and `lhs`.
    /// 3. If above is not implemented, invokes `unsupported` for the result.
    pub fn call_or_reflection(
        &self,
        lhs: PyObjectRef,
        rhs: PyObjectRef,
        default: &str,
        reflection: &str,
        unsupported: fn(&VirtualMachine, PyObjectRef, PyObjectRef) -> PyResult,
    ) -> PyResult {
        // Try to call the default method
        self.call_or_unsupported(lhs, rhs, default, move |vm, lhs, rhs| {
            // Try to call the reflection method
            vm.call_or_unsupported(rhs, lhs, reflection, unsupported)
        })
    }

    pub fn generic_getattribute(&self, obj: PyObjectRef, name: PyStringRef) -> PyResult {
        self.generic_getattribute_opt(obj.clone(), name.clone())?
            .ok_or_else(|| self.new_attribute_error(format!("{} has no attribute '{}'", obj, name)))
    }

    /// CPython _PyObject_GenericGetAttrWithDict
    pub fn generic_getattribute_opt(
        &self,
        obj: PyObjectRef,
        name_str: PyStringRef,
    ) -> PyResult<Option<PyObjectRef>> {
        let name = name_str.as_str();
        let cls = obj.class();

        if let Some(attr) = cls.get_attr(&name) {
            let attr_class = attr.class();
            if attr_class.has_attr("__set__") {
                if let Some(r) = self.call_get_descriptor(attr, obj.clone()) {
                    return r.map(Some);
                }
            }
        }

        let attr = if let Some(dict) = obj.dict() {
            dict.get_item_option(name_str.as_str(), self)?
        } else {
            None
        };

        if let Some(obj_attr) = attr {
            Ok(Some(obj_attr))
        } else if let Some(attr) = cls.get_attr(&name) {
            self.call_if_get_descriptor(attr, obj).map(Some)
        } else if let Some(getter) = cls.get_attr("__getattr__") {
            self.invoke(&getter, vec![obj, name_str.into_object()])
                .map(Some)
        } else {
            Ok(None)
        }
    }

    pub fn is_callable(&self, obj: &PyObjectRef) -> bool {
        obj.class().slots.read().unwrap().call.is_some() || obj.class().has_attr("__call__")
    }

    #[inline]
    /// Checks for triggered signals and calls the appropriate handlers. A no-op on
    /// platforms where signals are not supported.
    pub fn check_signals(&self) -> PyResult<()> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            crate::stdlib::signal::check_signals(self)
        }
        #[cfg(target_arch = "wasm32")]
        {
            Ok(())
        }
    }

    /// Returns a basic CompileOpts instance with options accurate to the vm. Used
    /// as the CompileOpts for `vm.compile()`.
    #[cfg(feature = "rustpython-compiler")]
    pub fn compile_opts(&self) -> CompileOpts {
        CompileOpts {
            optimize: self.state.settings.optimize,
        }
    }

    #[cfg(feature = "rustpython-compiler")]
    pub fn compile(
        &self,
        source: &str,
        mode: compile::Mode,
        source_path: String,
    ) -> Result<PyCodeRef, CompileError> {
        self.compile_with_opts(source, mode, source_path, self.compile_opts())
    }

    #[cfg(feature = "rustpython-compiler")]
    pub fn compile_with_opts(
        &self,
        source: &str,
        mode: compile::Mode,
        source_path: String,
        opts: CompileOpts,
    ) -> Result<PyCodeRef, CompileError> {
        compile::compile(source, mode, source_path, opts)
            .map(|codeobj| PyCode::new(codeobj).into_ref(self))
            .map_err(|mut compile_error| {
                compile_error.update_statement_info(source.trim_end().to_owned());
                compile_error
            })
    }

    fn call_codec_func(
        &self,
        func: &str,
        obj: PyObjectRef,
        encoding: Option<PyStringRef>,
        errors: Option<PyStringRef>,
    ) -> PyResult {
        let codecsmodule = self.import("_codecs", &[], 0)?;
        let func = self.get_attribute(codecsmodule, func)?;
        let mut args = vec![
            obj,
            encoding.map_or_else(|| self.get_none(), |s| s.into_object()),
        ];
        if let Some(errors) = errors {
            args.push(errors.into_object());
        }
        self.invoke(&func, args)
    }

    pub fn decode(
        &self,
        obj: PyObjectRef,
        encoding: Option<PyStringRef>,
        errors: Option<PyStringRef>,
    ) -> PyResult {
        self.call_codec_func("decode", obj, encoding, errors)
    }

    pub fn encode(
        &self,
        obj: PyObjectRef,
        encoding: Option<PyStringRef>,
        errors: Option<PyStringRef>,
    ) -> PyResult {
        self.call_codec_func("encode", obj, encoding, errors)
    }

    pub fn _sub(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_reflection(a, b, "__sub__", "__rsub__", |vm, a, b| {
            Err(vm.new_unsupported_operand_error(a, b, "-"))
        })
    }

    pub fn _isub(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_unsupported(a, b, "__isub__", |vm, a, b| {
            vm.call_or_reflection(a, b, "__sub__", "__rsub__", |vm, a, b| {
                Err(vm.new_unsupported_operand_error(a, b, "-="))
            })
        })
    }

    pub fn _add(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_reflection(a, b, "__add__", "__radd__", |vm, a, b| {
            Err(vm.new_unsupported_operand_error(a, b, "+"))
        })
    }

    pub fn _iadd(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_unsupported(a, b, "__iadd__", |vm, a, b| {
            vm.call_or_reflection(a, b, "__add__", "__radd__", |vm, a, b| {
                Err(vm.new_unsupported_operand_error(a, b, "+="))
            })
        })
    }

    pub fn _mul(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_reflection(a, b, "__mul__", "__rmul__", |vm, a, b| {
            Err(vm.new_unsupported_operand_error(a, b, "*"))
        })
    }

    pub fn _imul(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_unsupported(a, b, "__imul__", |vm, a, b| {
            vm.call_or_reflection(a, b, "__mul__", "__rmul__", |vm, a, b| {
                Err(vm.new_unsupported_operand_error(a, b, "*="))
            })
        })
    }

    pub fn _matmul(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_reflection(a, b, "__matmul__", "__rmatmul__", |vm, a, b| {
            Err(vm.new_unsupported_operand_error(a, b, "@"))
        })
    }

    pub fn _imatmul(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_unsupported(a, b, "__imatmul__", |vm, a, b| {
            vm.call_or_reflection(a, b, "__matmul__", "__rmatmul__", |vm, a, b| {
                Err(vm.new_unsupported_operand_error(a, b, "@="))
            })
        })
    }

    pub fn _truediv(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_reflection(a, b, "__truediv__", "__rtruediv__", |vm, a, b| {
            Err(vm.new_unsupported_operand_error(a, b, "/"))
        })
    }

    pub fn _itruediv(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_unsupported(a, b, "__itruediv__", |vm, a, b| {
            vm.call_or_reflection(a, b, "__truediv__", "__rtruediv__", |vm, a, b| {
                Err(vm.new_unsupported_operand_error(a, b, "/="))
            })
        })
    }

    pub fn _floordiv(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_reflection(a, b, "__floordiv__", "__rfloordiv__", |vm, a, b| {
            Err(vm.new_unsupported_operand_error(a, b, "//"))
        })
    }

    pub fn _ifloordiv(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_unsupported(a, b, "__ifloordiv__", |vm, a, b| {
            vm.call_or_reflection(a, b, "__floordiv__", "__rfloordiv__", |vm, a, b| {
                Err(vm.new_unsupported_operand_error(a, b, "//="))
            })
        })
    }

    pub fn _pow(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_reflection(a, b, "__pow__", "__rpow__", |vm, a, b| {
            Err(vm.new_unsupported_operand_error(a, b, "**"))
        })
    }

    pub fn _ipow(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_unsupported(a, b, "__ipow__", |vm, a, b| {
            vm.call_or_reflection(a, b, "__pow__", "__rpow__", |vm, a, b| {
                Err(vm.new_unsupported_operand_error(a, b, "**="))
            })
        })
    }

    pub fn _mod(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_reflection(a, b, "__mod__", "__rmod__", |vm, a, b| {
            Err(vm.new_unsupported_operand_error(a, b, "%"))
        })
    }

    pub fn _imod(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_unsupported(a, b, "__imod__", |vm, a, b| {
            vm.call_or_reflection(a, b, "__mod__", "__rmod__", |vm, a, b| {
                Err(vm.new_unsupported_operand_error(a, b, "%="))
            })
        })
    }

    pub fn _lshift(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_reflection(a, b, "__lshift__", "__rlshift__", |vm, a, b| {
            Err(vm.new_unsupported_operand_error(a, b, "<<"))
        })
    }

    pub fn _ilshift(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_unsupported(a, b, "__ilshift__", |vm, a, b| {
            vm.call_or_reflection(a, b, "__lshift__", "__rlshift__", |vm, a, b| {
                Err(vm.new_unsupported_operand_error(a, b, "<<="))
            })
        })
    }

    pub fn _rshift(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_reflection(a, b, "__rshift__", "__rrshift__", |vm, a, b| {
            Err(vm.new_unsupported_operand_error(a, b, ">>"))
        })
    }

    pub fn _irshift(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_unsupported(a, b, "__irshift__", |vm, a, b| {
            vm.call_or_reflection(a, b, "__rshift__", "__rrshift__", |vm, a, b| {
                Err(vm.new_unsupported_operand_error(a, b, ">>="))
            })
        })
    }

    pub fn _xor(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_reflection(a, b, "__xor__", "__rxor__", |vm, a, b| {
            Err(vm.new_unsupported_operand_error(a, b, "^"))
        })
    }

    pub fn _ixor(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_unsupported(a, b, "__ixor__", |vm, a, b| {
            vm.call_or_reflection(a, b, "__xor__", "__rxor__", |vm, a, b| {
                Err(vm.new_unsupported_operand_error(a, b, "^="))
            })
        })
    }

    pub fn _or(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_reflection(a, b, "__or__", "__ror__", |vm, a, b| {
            Err(vm.new_unsupported_operand_error(a, b, "|"))
        })
    }

    pub fn _ior(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_unsupported(a, b, "__ior__", |vm, a, b| {
            vm.call_or_reflection(a, b, "__or__", "__ror__", |vm, a, b| {
                Err(vm.new_unsupported_operand_error(a, b, "|="))
            })
        })
    }

    pub fn _and(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_reflection(a, b, "__and__", "__rand__", |vm, a, b| {
            Err(vm.new_unsupported_operand_error(a, b, "&"))
        })
    }

    pub fn _iand(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_unsupported(a, b, "__iand__", |vm, a, b| {
            vm.call_or_reflection(a, b, "__and__", "__rand__", |vm, a, b| {
                Err(vm.new_unsupported_operand_error(a, b, "&="))
            })
        })
    }

    // Perform a comparison, raising TypeError when the requested comparison
    // operator is not supported.
    // see: CPython PyObject_RichCompare
    fn _cmp<F>(
        &self,
        v: PyObjectRef,
        w: PyObjectRef,
        op: &str,
        swap_op: &str,
        default: F,
    ) -> PyResult
    where
        F: Fn(&VirtualMachine, PyObjectRef, PyObjectRef) -> PyResult,
    {
        // TODO: _Py_EnterRecursiveCall(tstate, " in comparison")

        let mut checked_reverse_op = false;
        if !v.typ.is(&w.typ) && objtype::issubclass(&w.class(), &v.class()) {
            if let Some(method_or_err) = self.get_method(w.clone(), swap_op) {
                let method = method_or_err?;
                checked_reverse_op = true;

                let result = self.invoke(&method, vec![v.clone()])?;
                if !result.is(&self.ctx.not_implemented()) {
                    return Ok(result);
                }
            }
        }

        self.call_or_unsupported(v, w, op, |vm, v, w| {
            if !checked_reverse_op {
                self.call_or_unsupported(w, v, swap_op, |vm, v, w| default(vm, v, w))
            } else {
                default(vm, v, w)
            }
        })

        // TODO: _Py_LeaveRecursiveCall(tstate);
    }

    pub fn _eq(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self._cmp(a, b, "__eq__", "__eq__", |vm, a, b| {
            Ok(vm.new_bool(a.is(&b)))
        })
    }

    pub fn _ne(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self._cmp(a, b, "__ne__", "__ne__", |vm, a, b| {
            Ok(vm.new_bool(!a.is(&b)))
        })
    }

    pub fn _lt(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self._cmp(a, b, "__lt__", "__gt__", |vm, a, b| {
            Err(vm.new_unsupported_operand_error(a, b, "<"))
        })
    }

    pub fn _le(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self._cmp(a, b, "__le__", "__ge__", |vm, a, b| {
            Err(vm.new_unsupported_operand_error(a, b, "<="))
        })
    }

    pub fn _gt(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self._cmp(a, b, "__gt__", "__lt__", |vm, a, b| {
            Err(vm.new_unsupported_operand_error(a, b, ">"))
        })
    }

    pub fn _ge(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self._cmp(a, b, "__ge__", "__le__", |vm, a, b| {
            Err(vm.new_unsupported_operand_error(a, b, ">="))
        })
    }

    pub fn _hash(&self, obj: &PyObjectRef) -> PyResult<pyhash::PyHash> {
        let hash_obj = self.call_method(obj, "__hash__", vec![])?;
        if let Some(hash_value) = hash_obj.payload_if_subclass::<PyInt>(self) {
            Ok(hash_value.hash())
        } else {
            Err(self.new_type_error("__hash__ method should return an integer".to_owned()))
        }
    }

    // https://docs.python.org/3/reference/expressions.html#membership-test-operations
    fn _membership_iter_search(&self, haystack: PyObjectRef, needle: PyObjectRef) -> PyResult {
        let iter = objiter::get_iter(self, &haystack)?;
        loop {
            if let Some(element) = objiter::get_next_object(self, &iter)? {
                if self.bool_eq(needle.clone(), element.clone())? {
                    return Ok(self.new_bool(true));
                } else {
                    continue;
                }
            } else {
                return Ok(self.new_bool(false));
            }
        }
    }

    pub fn _membership(&self, haystack: PyObjectRef, needle: PyObjectRef) -> PyResult {
        if let Some(method_or_err) = self.get_method(haystack.clone(), "__contains__") {
            let method = method_or_err?;
            self.invoke(&method, vec![needle])
        } else {
            self._membership_iter_search(haystack, needle)
        }
    }

    pub fn push_exception(&self, exc: PyBaseExceptionRef) {
        self.exceptions.borrow_mut().push(exc)
    }

    pub fn pop_exception(&self) -> Option<PyBaseExceptionRef> {
        self.exceptions.borrow_mut().pop()
    }

    pub fn current_exception(&self) -> Option<PyBaseExceptionRef> {
        self.exceptions.borrow().last().cloned()
    }

    pub fn bool_eq(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult<bool> {
        let eq = self._eq(a, b)?;
        let value = objbool::boolval(self, eq)?;
        Ok(value)
    }

    pub fn identical_or_equal(&self, a: &PyObjectRef, b: &PyObjectRef) -> PyResult<bool> {
        if a.is(b) {
            Ok(true)
        } else {
            self.bool_eq(a.clone(), b.clone())
        }
    }

    pub fn bool_seq_lt(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult<Option<bool>> {
        let value = if objbool::boolval(self, self._lt(a.clone(), b.clone())?)? {
            Some(true)
        } else if !objbool::boolval(self, self._eq(a.clone(), b.clone())?)? {
            Some(false)
        } else {
            None
        };
        Ok(value)
    }

    pub fn bool_seq_gt(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult<Option<bool>> {
        let value = if objbool::boolval(self, self._gt(a.clone(), b.clone())?)? {
            Some(true)
        } else if !objbool::boolval(self, self._eq(a.clone(), b.clone())?)? {
            Some(false)
        } else {
            None
        };
        Ok(value)
    }

    #[doc(hidden)]
    pub fn __module_set_attr(
        &self,
        module: &PyObjectRef,
        attr_name: impl TryIntoRef<PyString>,
        attr_value: impl Into<PyObjectRef>,
    ) -> PyResult<()> {
        let val = attr_value.into();
        objobject::setattr(module.clone(), attr_name.try_into_ref(self)?, val, self)
    }
}

impl Default for VirtualMachine {
    fn default() -> Self {
        VirtualMachine::new(Default::default())
    }
}

static REPR_GUARDS: Lazy<Mutex<HashSet<usize>>> = Lazy::new(Mutex::default);

pub struct ReprGuard {
    id: usize,
}

/// A guard to protect repr methods from recursion into itself,
impl ReprGuard {
    fn get_guards<'a>() -> MutexGuard<'a, HashSet<usize>> {
        REPR_GUARDS.lock().expect("ReprGuard lock poisoned")
    }

    /// Returns None if the guard against 'obj' is still held otherwise returns the guard. The guard
    /// which is released if dropped.
    pub fn enter(obj: &PyObjectRef) -> Option<ReprGuard> {
        let mut guards = ReprGuard::get_guards();

        // Should this be a flag on the obj itself? putting it in a global variable for now until it
        // decided the form of the PyObject. https://github.com/RustPython/RustPython/issues/371
        let id = obj.get_id();
        if guards.contains(&id) {
            return None;
        }
        guards.insert(id);
        Some(ReprGuard { id })
    }
}

impl Drop for ReprGuard {
    fn drop(&mut self) {
        ReprGuard::get_guards().remove(&self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::VirtualMachine;
    use crate::obj::{objint, objstr};
    use num_bigint::ToBigInt;

    #[test]
    fn test_add_py_integers() {
        let vm: VirtualMachine = Default::default();
        let a = vm.ctx.new_int(33_i32);
        let b = vm.ctx.new_int(12_i32);
        let res = vm._add(a, b).unwrap();
        let value = objint::get_value(&res);
        assert_eq!(*value, 45_i32.to_bigint().unwrap());
    }

    #[test]
    fn test_multiply_str() {
        let vm: VirtualMachine = Default::default();
        let a = vm.ctx.new_str(String::from("Hello "));
        let b = vm.ctx.new_int(4_i32);
        let res = vm._mul(a, b).unwrap();
        let value = objstr::borrow_value(&res);
        assert_eq!(value, String::from("Hello Hello Hello Hello "))
    }
}
