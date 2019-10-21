//! Implement virtual machine to run instructions.
//!
//! See also:
//!   https://github.com/ProgVal/pythonvm-rust/blob/master/src/processor/mod.rs
//!

use std::cell::{Cell, Ref, RefCell};
use std::collections::hash_map::HashMap;
use std::collections::hash_set::HashSet;
use std::fmt;
use std::rc::Rc;
use std::sync::{Mutex, MutexGuard};

use arr_macro::arr;
use num_bigint::BigInt;
use num_traits::ToPrimitive;
#[cfg(feature = "rustpython-compiler")]
use rustpython_compiler::{compile, error::CompileError};

use crate::builtins::{self, to_ascii};
use crate::bytecode;
use crate::frame::{ExecutionResult, Frame, FrameRef};
use crate::frozen;
use crate::function::PyFuncArgs;
use crate::import;
use crate::obj::objbool;
use crate::obj::objbuiltinfunc::PyBuiltinFunction;
use crate::obj::objcode::{PyCode, PyCodeRef};
use crate::obj::objdict::PyDictRef;
use crate::obj::objfunction::{PyFunction, PyMethod};
use crate::obj::objgenerator::PyGenerator;
use crate::obj::objint::PyInt;
use crate::obj::objiter;
use crate::obj::objmodule::{self, PyModule};
use crate::obj::objsequence;
use crate::obj::objstr::{PyString, PyStringRef};
use crate::obj::objtuple::PyTupleRef;
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
    pub stdlib_inits: RefCell<HashMap<String, stdlib::StdlibInitFunc>>,
    pub ctx: PyContext,
    pub frames: RefCell<Vec<FrameRef>>,
    pub wasm_id: Option<String>,
    pub exceptions: RefCell<Vec<PyObjectRef>>,
    pub frozen: RefCell<HashMap<String, bytecode::FrozenModule>>,
    pub import_func: RefCell<PyObjectRef>,
    pub profile_func: RefCell<PyObjectRef>,
    pub trace_func: RefCell<PyObjectRef>,
    pub use_tracing: RefCell<bool>,
    pub signal_handlers: RefCell<[PyObjectRef; NSIG]>,
    pub settings: PySettings,
    pub recursion_limit: Cell<usize>,
}

pub const NSIG: usize = 64;

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
        }
    }
}

impl VirtualMachine {
    /// Create a new `VirtualMachine` structure.
    pub fn new(settings: PySettings) -> VirtualMachine {
        flame_guard!("init VirtualMachine");
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

        let stdlib_inits = RefCell::new(stdlib::get_module_inits());
        let frozen = RefCell::new(frozen::get_module_inits());
        let import_func = RefCell::new(ctx.none());
        let profile_func = RefCell::new(ctx.none());
        let trace_func = RefCell::new(ctx.none());
        let signal_handlers = RefCell::new(arr![ctx.none(); 64]);

        let vm = VirtualMachine {
            builtins: builtins.clone(),
            sys_module: sysmod.clone(),
            stdlib_inits,
            ctx,
            frames: RefCell::new(vec![]),
            wasm_id: None,
            exceptions: RefCell::new(vec![]),
            frozen,
            import_func,
            profile_func,
            trace_func,
            use_tracing: RefCell::new(false),
            signal_handlers,
            settings,
            recursion_limit: Cell::new(512),
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

        builtins::make_module(&vm, builtins.clone());
        sysmodule::make_module(&vm, sysmod, builtins);

        #[cfg(not(target_arch = "wasm32"))]
        import::import_builtin(&vm, "signal").expect("Couldn't initialize signal module");

        vm
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

    pub fn run_frame(&self, frame: FrameRef) -> PyResult<ExecutionResult> {
        self.check_recursive_call("")?;
        self.frames.borrow_mut().push(frame.clone());
        let result = frame.run(self);
        self.frames.borrow_mut().pop();
        result
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

    #[cfg_attr(feature = "flame-it", flame("VirtualMachine"))]
    pub fn new_exception_obj(&self, exc_type: PyClassRef, args: Vec<PyObjectRef>) -> PyResult {
        // TODO: add repr of args into logging?
        vm_trace!("New exception created: {}", exc_type.name);
        self.invoke(&exc_type.into_object(), args)
    }

    pub fn new_empty_exception(&self, exc_type: PyClassRef) -> PyResult {
        self.new_exception_obj(exc_type, vec![])
    }

    /// Create Python instance of `exc_type` with message as first element of `args` tuple
    pub fn new_exception(&self, exc_type: PyClassRef, msg: String) -> PyObjectRef {
        let pystr_msg = self.new_str(msg);
        self.new_exception_obj(exc_type, vec![pystr_msg]).unwrap()
    }

    pub fn new_lookup_error(&self, msg: String) -> PyObjectRef {
        let lookup_error = self.ctx.exceptions.lookup_error.clone();
        self.new_exception(lookup_error, msg)
    }

    pub fn new_attribute_error(&self, msg: String) -> PyObjectRef {
        let attribute_error = self.ctx.exceptions.attribute_error.clone();
        self.new_exception(attribute_error, msg)
    }

    pub fn new_type_error(&self, msg: String) -> PyObjectRef {
        let type_error = self.ctx.exceptions.type_error.clone();
        self.new_exception(type_error, msg)
    }

    pub fn new_name_error(&self, msg: String) -> PyObjectRef {
        let name_error = self.ctx.exceptions.name_error.clone();
        self.new_exception(name_error, msg)
    }

    pub fn new_unsupported_operand_error(
        &self,
        a: PyObjectRef,
        b: PyObjectRef,
        op: &str,
    ) -> PyObjectRef {
        self.new_type_error(format!(
            "Unsupported operand types for '{}': '{}' and '{}'",
            op,
            a.class().name,
            b.class().name
        ))
    }

    pub fn new_os_error(&self, msg: String) -> PyObjectRef {
        let os_error = self.ctx.exceptions.os_error.clone();
        self.new_exception(os_error, msg)
    }

    pub fn new_unicode_decode_error(&self, msg: String) -> PyObjectRef {
        let unicode_decode_error = self.ctx.exceptions.unicode_decode_error.clone();
        self.new_exception(unicode_decode_error, msg)
    }

    /// Create a new python ValueError object. Useful for raising errors from
    /// python functions implemented in rust.
    pub fn new_value_error(&self, msg: String) -> PyObjectRef {
        let value_error = self.ctx.exceptions.value_error.clone();
        self.new_exception(value_error, msg)
    }

    pub fn new_key_error(&self, obj: PyObjectRef) -> PyObjectRef {
        let key_error = self.ctx.exceptions.key_error.clone();
        self.new_exception_obj(key_error, vec![obj]).unwrap()
    }

    pub fn new_index_error(&self, msg: String) -> PyObjectRef {
        let index_error = self.ctx.exceptions.index_error.clone();
        self.new_exception(index_error, msg)
    }

    pub fn new_not_implemented_error(&self, msg: String) -> PyObjectRef {
        let not_implemented_error = self.ctx.exceptions.not_implemented_error.clone();
        self.new_exception(not_implemented_error, msg)
    }

    pub fn new_recursion_error(&self, msg: String) -> PyObjectRef {
        let recursion_error = self.ctx.exceptions.recursion_error.clone();
        self.new_exception(recursion_error, msg)
    }

    pub fn new_zero_division_error(&self, msg: String) -> PyObjectRef {
        let zero_division_error = self.ctx.exceptions.zero_division_error.clone();
        self.new_exception(zero_division_error, msg)
    }

    pub fn new_overflow_error(&self, msg: String) -> PyObjectRef {
        let overflow_error = self.ctx.exceptions.overflow_error.clone();
        self.new_exception(overflow_error, msg)
    }

    #[cfg(feature = "rustpython-compiler")]
    pub fn new_syntax_error(&self, error: &CompileError) -> PyObjectRef {
        let syntax_error_type = if error.is_indentation_error() {
            self.ctx.exceptions.indentation_error.clone()
        } else if error.is_tab_error() {
            self.ctx.exceptions.tab_error.clone()
        } else {
            self.ctx.exceptions.syntax_error.clone()
        };
        let syntax_error = self.new_exception(syntax_error_type, error.to_string());
        let lineno = self.new_int(error.location.row());
        self.set_attr(&syntax_error, "lineno", lineno).unwrap();
        syntax_error
    }

    pub fn new_import_error(&self, msg: String) -> PyObjectRef {
        let import_error = self.ctx.exceptions.import_error.clone();
        self.new_exception(import_error, msg)
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

    pub fn get_type(&self) -> PyClassRef {
        self.ctx.type_type()
    }

    pub fn get_object(&self) -> PyClassRef {
        self.ctx.object()
    }

    pub fn get_locals(&self) -> PyDictRef {
        self.current_scope().get_locals().clone()
    }

    pub fn context(&self) -> &PyContext {
        &self.ctx
    }

    // Container of the virtual machine state:
    pub fn to_str(&self, obj: &PyObjectRef) -> PyResult<PyStringRef> {
        let str = self.call_method(&obj, "__str__", vec![])?;
        TryFromObject::try_from_object(self, str)
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
                    .map_err(|_| self.new_import_error("__import__ not found".to_string()))?;

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
                        .map(|name| self.new_str(name.to_string()))
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
        if Rc::ptr_eq(&obj.class().into_object(), cls.as_object()) {
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

    pub fn call_get_descriptor(&self, attr: PyObjectRef, obj: PyObjectRef) -> PyResult {
        let attr_class = attr.class();
        if let Some(ref descriptor) = objtype::class_get_attr(&attr_class, "__get__") {
            let cls = obj.class();
            self.invoke(descriptor, vec![attr, obj.clone(), cls.into_object()])
        } else {
            Ok(attr)
        }
    }

    pub fn call_method<T>(&self, obj: &PyObjectRef, method_name: &str, args: T) -> PyResult
    where
        T: Into<PyFuncArgs>,
    {
        flame_guard!(format!("call_method({:?})", method_name));

        // This is only used in the vm for magic methods, which use a greatly simplified attribute lookup.
        let cls = obj.class();
        match objtype::class_get_attr(&cls, method_name) {
            Some(func) => {
                vm_trace!(
                    "vm.call_method {:?} {:?} {:?} -> {:?}",
                    obj,
                    cls,
                    method_name,
                    func
                );
                let wrapped = self.call_get_descriptor(func, obj.clone())?;
                self.invoke(&wrapped, args)
            }
            None => Err(self.new_type_error(format!("Unsupported method: {}", method_name))),
        }
    }

    fn _invoke(&self, func_ref: &PyObjectRef, args: PyFuncArgs) -> PyResult {
        vm_trace!("Invoke: {:?} {:?}", func_ref, args);

        if let Some(py_func) = func_ref.payload() {
            self.trace_event(TraceEvent::Call)?;
            let res = self.invoke_python_function(py_func, args);
            self.trace_event(TraceEvent::Return)?;
            res
        } else if let Some(PyMethod {
            ref function,
            ref object,
        }) = func_ref.payload()
        {
            self.invoke(&function, args.insert(object.clone()))
        } else if let Some(PyBuiltinFunction { ref value }) = func_ref.payload() {
            value(self, args)
        } else if self.is_callable(&func_ref) {
            self.call_method(&func_ref, "__call__", args)
        } else {
            Err(self.new_type_error(format!(
                "'{}' object is not callable",
                func_ref.class().name
            )))
        }
    }

    #[inline]
    pub fn invoke<T>(&self, func_ref: &PyObjectRef, args: T) -> PyResult
    where
        T: Into<PyFuncArgs>,
    {
        let res = self._invoke(func_ref, args.into());
        res
    }

    /// Call registered trace function.
    fn trace_event(&self, event: TraceEvent) -> PyResult<()> {
        if *self.use_tracing.borrow() {
            let frame = self.get_none();
            let event = self.new_str(event.to_string());
            let arg = self.get_none();
            let args = vec![frame, event, arg];

            // temporarily disable tracing, during the call to the
            // tracing function itself.
            let trace_func = self.trace_func.borrow().clone();
            if !self.is_none(&trace_func) {
                self.use_tracing.replace(false);
                let res = self.invoke(&trace_func, args.clone());
                self.use_tracing.replace(true);
                res?;
            }

            let profile_func = self.profile_func.borrow().clone();
            if !self.is_none(&profile_func) {
                self.use_tracing.replace(false);
                let res = self.invoke(&profile_func, args);
                self.use_tracing.replace(true);
                res?;
            }
        }
        Ok(())
    }

    pub fn invoke_python_function(&self, func: &PyFunction, func_args: PyFuncArgs) -> PyResult {
        self.invoke_python_function_with_scope(func, func_args, &func.scope)
    }

    pub fn invoke_python_function_with_scope(
        &self,
        func: &PyFunction,
        func_args: PyFuncArgs,
        scope: &Scope,
    ) -> PyResult {
        let code = &func.code;

        let scope = if func.code.flags.contains(bytecode::CodeFlags::NEW_LOCALS) {
            scope.new_child_scope(&self.ctx)
        } else {
            scope.clone()
        };

        self.fill_locals_from_args(
            &code,
            &scope.get_locals(),
            func_args,
            &func.defaults,
            &func.kw_only_defaults,
        )?;

        // Construct frame:
        let frame = Frame::new(code.clone(), scope).into_ref(self);

        // If we have a generator, create a new generator
        if code.flags.contains(bytecode::CodeFlags::IS_GENERATOR) {
            Ok(PyGenerator::new(frame, self).into_object())
        } else {
            self.run_frame_full(frame)
        }
    }

    fn fill_locals_from_args(
        &self,
        code_object: &bytecode::CodeObject,
        locals: &PyDictRef,
        func_args: PyFuncArgs,
        defaults: &Option<PyTupleRef>,
        kw_only_defaults: &Option<PyDictRef>,
    ) -> PyResult<()> {
        let nargs = func_args.args.len();
        let nexpected_args = code_object.arg_names.len();

        // This parses the arguments from args and kwargs into
        // the proper variables keeping into account default values
        // and starargs and kwargs.
        // See also: PyEval_EvalCodeWithName in cpython:
        // https://github.com/python/cpython/blob/master/Python/ceval.c#L3681

        let n = if nargs > nexpected_args {
            nexpected_args
        } else {
            nargs
        };

        // Copy positional arguments into local variables
        for i in 0..n {
            let arg_name = &code_object.arg_names[i];
            let arg = &func_args.args[i];
            locals.set_item(arg_name, arg.clone(), self)?;
        }

        // Pack other positional arguments in to *args:
        match code_object.varargs {
            bytecode::Varargs::Named(ref vararg_name) => {
                let mut last_args = vec![];
                for i in n..nargs {
                    let arg = &func_args.args[i];
                    last_args.push(arg.clone());
                }
                let vararg_value = self.ctx.new_tuple(last_args);

                locals.set_item(vararg_name, vararg_value, self)?;
            }
            bytecode::Varargs::Unnamed | bytecode::Varargs::None => {
                // Check the number of positional arguments
                if nargs > nexpected_args {
                    return Err(self.new_type_error(format!(
                        "Expected {} arguments (got: {})",
                        nexpected_args, nargs
                    )));
                }
            }
        }

        // Do we support `**kwargs` ?
        let kwargs = match code_object.varkeywords {
            bytecode::Varargs::Named(ref kwargs_name) => {
                let d = self.ctx.new_dict();
                locals.set_item(kwargs_name, d.as_object().clone(), self)?;
                Some(d)
            }
            bytecode::Varargs::Unnamed => Some(self.ctx.new_dict()),
            bytecode::Varargs::None => None,
        };

        // Handle keyword arguments
        for (name, value) in func_args.kwargs {
            // Check if we have a parameter with this name:
            if code_object.arg_names.contains(&name) || code_object.kwonlyarg_names.contains(&name)
            {
                if locals.contains_key(&name, self) {
                    return Err(
                        self.new_type_error(format!("Got multiple values for argument '{}'", name))
                    );
                }

                locals.set_item(&name, value, self)?;
            } else if let Some(d) = &kwargs {
                d.set_item(&name, value, self)?;
            } else {
                return Err(
                    self.new_type_error(format!("Got an unexpected keyword argument '{}'", name))
                );
            }
        }

        // Add missing positional arguments, if we have fewer positional arguments than the
        // function definition calls for
        if nargs < nexpected_args {
            let num_defaults_available = defaults.as_ref().map_or(0, |d| d.elements.len());

            // Given the number of defaults available, check all the arguments for which we
            // _don't_ have defaults; if any are missing, raise an exception
            let required_args = nexpected_args - num_defaults_available;
            let mut missing = vec![];
            for i in 0..required_args {
                let variable_name = &code_object.arg_names[i];
                if !locals.contains_key(variable_name, self) {
                    missing.push(variable_name)
                }
            }
            if !missing.is_empty() {
                return Err(self.new_type_error(format!(
                    "Missing {} required positional arguments: {:?}",
                    missing.len(),
                    missing
                )));
            }
            if let Some(defaults) = defaults {
                let defaults = &defaults.elements;
                // We have sufficient defaults, so iterate over the corresponding names and use
                // the default if we don't already have a value
                for (default_index, i) in (required_args..nexpected_args).enumerate() {
                    let arg_name = &code_object.arg_names[i];
                    if !locals.contains_key(arg_name, self) {
                        locals.set_item(arg_name, defaults[default_index].clone(), self)?;
                    }
                }
            }
        };

        // Check if kw only arguments are all present:
        for arg_name in &code_object.kwonlyarg_names {
            if !locals.contains_key(arg_name, self) {
                if let Some(kw_only_defaults) = kw_only_defaults {
                    if let Some(default) = kw_only_defaults.get_item_option(arg_name, self)? {
                        locals.set_item(arg_name, default, self)?;
                        continue;
                    }
                }

                // No default value and not specified.
                return Err(self
                    .new_type_error(format!("Missing required kw only argument: '{}'", arg_name)));
            }
        }

        Ok(())
    }

    pub fn extract_elements<T: TryFromObject>(&self, value: &PyObjectRef) -> PyResult<Vec<T>> {
        // Extract elements from item, if possible:
        if objtype::isinstance(value, &self.ctx.tuple_type()) {
            objsequence::get_elements_tuple(value)
                .iter()
                .map(|obj| T::try_from_object(self, obj.clone()))
                .collect()
        } else if objtype::isinstance(value, &self.ctx.list_type()) {
            objsequence::get_elements_list(value)
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
        match objtype::class_get_attr(&cls, method_name) {
            Some(method) => self.call_get_descriptor(method, obj.clone()),
            None => Err(self.new_type_error(err_msg())),
        }
    }

    /// May return exception, if `__get__` descriptor raises one
    pub fn get_method(&self, obj: PyObjectRef, method_name: &str) -> Option<PyResult> {
        let cls = obj.class();
        let method = objtype::class_get_attr(&cls, method_name)?;
        Some(self.call_get_descriptor(method, obj.clone()))
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

    pub fn generic_getattribute(
        &self,
        obj: PyObjectRef,
        name_str: PyStringRef,
    ) -> PyResult<Option<PyObjectRef>> {
        let name = name_str.as_str();
        let cls = obj.class();

        if let Some(attr) = objtype::class_get_attr(&cls, &name) {
            let attr_class = attr.class();
            if objtype::class_has_attr(&attr_class, "__set__") {
                if let Some(descriptor) = objtype::class_get_attr(&attr_class, "__get__") {
                    return self
                        .invoke(&descriptor, vec![attr, obj, cls.into_object()])
                        .map(Some);
                }
            }
        }

        let attr = if let Some(ref dict) = obj.dict {
            dict.get_item_option(name_str.as_str(), self)?
        } else {
            None
        };

        if let Some(obj_attr) = attr {
            Ok(Some(obj_attr))
        } else if let Some(attr) = objtype::class_get_attr(&cls, &name) {
            self.call_get_descriptor(attr, obj).map(Some)
        } else if let Some(getter) = objtype::class_get_attr(&cls, "__getattr__") {
            self.invoke(&getter, vec![obj, name_str.into_object()])
                .map(Some)
        } else {
            Ok(None)
        }
    }

    pub fn is_callable(&self, obj: &PyObjectRef) -> bool {
        match_class!(match obj {
            PyFunction => true,
            PyMethod => true,
            PyBuiltinFunction => true,
            obj => objtype::class_has_attr(&obj.class(), "__call__"),
        })
    }

    #[cfg(feature = "rustpython-compiler")]
    pub fn compile(
        &self,
        source: &str,
        mode: compile::Mode,
        source_path: String,
    ) -> Result<PyCodeRef, CompileError> {
        compile::compile(source, mode, source_path, self.settings.optimize)
            .map(|codeobj| PyCode::new(codeobj).into_ref(self))
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

    pub fn _eq(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_reflection(a, b, "__eq__", "__eq__", |vm, a, b| {
            Ok(vm.new_bool(a.is(&b)))
        })
    }

    pub fn _ne(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_reflection(a, b, "__ne__", "__ne__", |vm, a, b| {
            let eq = vm._eq(a, b)?;
            Ok(vm.new_bool(objbool::not(vm, &eq)?))
        })
    }

    pub fn _lt(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_reflection(a, b, "__lt__", "__gt__", |vm, a, b| {
            Err(vm.new_unsupported_operand_error(a, b, "<"))
        })
    }

    pub fn _le(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_reflection(a, b, "__le__", "__ge__", |vm, a, b| {
            Err(vm.new_unsupported_operand_error(a, b, "<="))
        })
    }

    pub fn _gt(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_reflection(a, b, "__gt__", "__lt__", |vm, a, b| {
            Err(vm.new_unsupported_operand_error(a, b, ">"))
        })
    }

    pub fn _ge(&self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        self.call_or_reflection(a, b, "__ge__", "__le__", |vm, a, b| {
            Err(vm.new_unsupported_operand_error(a, b, ">="))
        })
    }

    pub fn _hash(&self, obj: &PyObjectRef) -> PyResult<pyhash::PyHash> {
        let hash_obj = self.call_method(obj, "__hash__", vec![])?;
        if objtype::isinstance(&hash_obj, &self.ctx.int_type()) {
            Ok(hash_obj.payload::<PyInt>().unwrap().hash(self))
        } else {
            Err(self.new_type_error("__hash__ method should return an integer".to_string()))
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

    pub fn push_exception(&self, exc: PyObjectRef) {
        self.exceptions.borrow_mut().push(exc)
    }

    pub fn pop_exception(&self) -> Option<PyObjectRef> {
        self.exceptions.borrow_mut().pop()
    }

    pub fn current_exception(&self) -> Option<PyObjectRef> {
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
}

impl Default for VirtualMachine {
    fn default() -> Self {
        VirtualMachine::new(Default::default())
    }
}

lazy_static! {
    static ref REPR_GUARDS: Mutex<HashSet<usize>> = { Mutex::new(HashSet::new()) };
}

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
        let value = objstr::get_value(&res);
        assert_eq!(value, String::from("Hello Hello Hello Hello "))
    }
}
