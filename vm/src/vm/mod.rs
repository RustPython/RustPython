//! Implement virtual machine to run instructions.
//!
//! See also:
//!   <https://github.com/ProgVal/pythonvm-rust/blob/master/src/processor/mod.rs>

#[cfg(feature = "rustpython-compiler")]
mod compile;
mod context;
mod interpreter;
mod method;
mod setting;
pub mod thread;
mod vm_new;
mod vm_object;
mod vm_ops;

use crate::{
    builtins::{
        code::PyCode,
        pystr::AsPyStr,
        tuple::{PyTuple, PyTupleTyped},
        PyBaseExceptionRef, PyDictRef, PyInt, PyList, PyModule, PyStr, PyStrInterned, PyStrRef,
        PyTypeRef,
    },
    codecs::CodecsRegistry,
    common::{hash::HashSecret, lock::PyMutex, rc::PyRc},
    convert::ToPyObject,
    frame::{ExecutionResult, Frame, FrameRef},
    frozen::FrozenModule,
    function::{ArgMapping, FuncArgs, PySetterValue},
    import,
    protocol::PyIterIter,
    scope::Scope,
    signal, stdlib,
    warn::WarningsState,
    AsObject, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult,
};
use crossbeam_utils::atomic::AtomicCell;
#[cfg(unix)]
use nix::{
    sys::signal::{kill, sigaction, SaFlags, SigAction, SigSet, Signal::SIGINT},
    unistd::getpid,
};
use std::sync::atomic::AtomicBool;
use std::{
    borrow::Cow,
    cell::{Cell, Ref, RefCell},
    collections::{HashMap, HashSet},
};

pub use context::Context;
pub use interpreter::Interpreter;
pub(crate) use method::PyMethod;
pub use setting::Settings;

// Objects are live when they are on stack, or referenced by a name (for now)

/// Top level container of a python virtual machine. In theory you could
/// create more instances of this struct and have them operate fully isolated.
///
/// To construct this, please refer to the [`Interpreter`](Interpreter)
pub struct VirtualMachine {
    pub builtins: PyRef<PyModule>,
    pub sys_module: PyRef<PyModule>,
    pub ctx: PyRc<Context>,
    pub frames: RefCell<Vec<FrameRef>>,
    pub wasm_id: Option<String>,
    exceptions: RefCell<ExceptionStack>,
    pub import_func: PyObjectRef,
    pub profile_func: RefCell<PyObjectRef>,
    pub trace_func: RefCell<PyObjectRef>,
    pub use_tracing: Cell<bool>,
    pub recursion_limit: Cell<usize>,
    pub(crate) signal_handlers: Option<Box<RefCell<[Option<PyObjectRef>; signal::NSIG]>>>,
    pub(crate) signal_rx: Option<signal::UserSignalReceiver>,
    pub repr_guards: RefCell<HashSet<usize>>,
    pub state: PyRc<PyGlobalState>,
    pub initialized: bool,
    recursion_depth: Cell<usize>,
}

#[derive(Debug, Default)]
struct ExceptionStack {
    exc: Option<PyBaseExceptionRef>,
    prev: Option<Box<ExceptionStack>>,
}

pub struct PyGlobalState {
    pub settings: Settings,
    pub module_inits: stdlib::StdlibMap,
    pub frozen: HashMap<&'static str, FrozenModule, ahash::RandomState>,
    pub stacksize: AtomicCell<usize>,
    pub thread_count: AtomicCell<usize>,
    pub hash_secret: HashSecret,
    pub atexit_funcs: PyMutex<Vec<(PyObjectRef, FuncArgs)>>,
    pub codec_registry: CodecsRegistry,
    pub finalizing: AtomicBool,
    pub warnings: WarningsState,
    pub override_frozen_modules: AtomicCell<isize>,
    pub before_forkers: PyMutex<Vec<PyObjectRef>>,
    pub after_forkers_child: PyMutex<Vec<PyObjectRef>>,
    pub after_forkers_parent: PyMutex<Vec<PyObjectRef>>,
    pub int_max_str_digits: AtomicCell<usize>,
}

pub fn process_hash_secret_seed() -> u32 {
    use once_cell::sync::OnceCell;
    static SEED: OnceCell<u32> = OnceCell::new();
    *SEED.get_or_init(rand::random)
}

impl VirtualMachine {
    /// Create a new `VirtualMachine` structure.
    fn new(settings: Settings, ctx: PyRc<Context>) -> VirtualMachine {
        flame_guard!("new VirtualMachine");

        // make a new module without access to the vm; doesn't
        // set __spec__, __loader__, etc. attributes
        let new_module = |def| {
            PyRef::new_ref(
                PyModule::from_def(def),
                ctx.types.module_type.to_owned(),
                Some(ctx.new_dict()),
            )
        };

        // Hard-core modules:
        let builtins = new_module(stdlib::builtins::__module_def(&ctx));
        let sys_module = new_module(stdlib::sys::__module_def(&ctx));

        let import_func = ctx.none();
        let profile_func = RefCell::new(ctx.none());
        let trace_func = RefCell::new(ctx.none());
        // hack to get around const array repeat expressions, rust issue #79270
        const NONE: Option<PyObjectRef> = None;
        // putting it in a const optimizes better, prevents linear initialization of the array
        #[allow(clippy::declare_interior_mutable_const)]
        const SIGNAL_HANDLERS: RefCell<[Option<PyObjectRef>; signal::NSIG]> =
            RefCell::new([NONE; signal::NSIG]);
        let signal_handlers = Some(Box::new(SIGNAL_HANDLERS));

        let module_inits = stdlib::get_module_inits();

        let seed = match settings.hash_seed {
            Some(seed) => seed,
            None => process_hash_secret_seed(),
        };
        let hash_secret = HashSecret::new(seed);

        let codec_registry = CodecsRegistry::new(&ctx);

        let warnings = WarningsState::init_state(&ctx);

        let int_max_str_digits = AtomicCell::new(match settings.int_max_str_digits {
            -1 => 4300,
            other => other,
        } as usize);
        let mut vm = VirtualMachine {
            builtins,
            sys_module,
            ctx,
            frames: RefCell::new(vec![]),
            wasm_id: None,
            exceptions: RefCell::default(),
            import_func,
            profile_func,
            trace_func,
            use_tracing: Cell::new(false),
            recursion_limit: Cell::new(if cfg!(debug_assertions) { 256 } else { 1000 }),
            signal_handlers,
            signal_rx: None,
            repr_guards: RefCell::default(),
            state: PyRc::new(PyGlobalState {
                settings,
                module_inits,
                frozen: HashMap::default(),
                stacksize: AtomicCell::new(0),
                thread_count: AtomicCell::new(0),
                hash_secret,
                atexit_funcs: PyMutex::default(),
                codec_registry,
                finalizing: AtomicBool::new(false),
                warnings,
                override_frozen_modules: AtomicCell::new(0),
                before_forkers: PyMutex::default(),
                after_forkers_child: PyMutex::default(),
                after_forkers_parent: PyMutex::default(),
                int_max_str_digits,
            }),
            initialized: false,
            recursion_depth: Cell::new(0),
        };

        if vm.state.hash_secret.hash_str("")
            != vm
                .ctx
                .interned_str("")
                .expect("empty str must be interned")
                .hash(&vm)
        {
            panic!("Interpreters in same process must share the hash seed");
        }

        let frozen = core_frozen_inits().collect();
        PyRc::get_mut(&mut vm.state).unwrap().frozen = frozen;

        vm.builtins.init_dict(
            vm.ctx.intern_str("builtins"),
            Some(vm.ctx.intern_str(stdlib::builtins::DOC.unwrap()).to_owned()),
            &vm,
        );
        vm.sys_module.init_dict(
            vm.ctx.intern_str("sys"),
            Some(vm.ctx.intern_str(stdlib::sys::DOC.unwrap()).to_owned()),
            &vm,
        );
        // let name = vm.sys_module.get_attr("__name__", &vm).unwrap();
        vm
    }

    /// set up the encodings search function
    /// init_importlib must be called before this call
    #[cfg(feature = "encodings")]
    fn import_encodings(&mut self) -> PyResult<()> {
        self.import("encodings", None, 0).map_err(|import_err| {
            let rustpythonpath_env = std::env::var("RUSTPYTHONPATH").ok();
            let pythonpath_env = std::env::var("PYTHONPATH").ok();
            let env_set = rustpythonpath_env.as_ref().is_some() || pythonpath_env.as_ref().is_some();
            let path_contains_env = self.state.settings.path_list.iter().any(|s| {
                Some(s.as_str()) == rustpythonpath_env.as_deref() || Some(s.as_str()) == pythonpath_env.as_deref()
            });

            let guide_message = if !env_set {
                "Neither RUSTPYTHONPATH nor PYTHONPATH is set. Try setting one of them to the stdlib directory."
            } else if path_contains_env {
                "RUSTPYTHONPATH or PYTHONPATH is set, but it doesn't contain the encodings library. If you are customizing the RustPython vm/interpreter, try adding the stdlib directory to the path. If you are developing the RustPython interpreter, it might be a bug during development."
            } else {
                "RUSTPYTHONPATH or PYTHONPATH is set, but it wasn't loaded to `Settings::path_list`. If you are going to customize the RustPython vm/interpreter, those environment variables are not loaded in the Settings struct by default. Please try creating a customized instance of the Settings struct. If you are developing the RustPython interpreter, it might be a bug during development."
            };

            let msg = format!(
                "RustPython could not import the encodings module. It usually means something went wrong. Please carefully read the following messages and follow the steps.\n\
                \n\
                {guide_message}\n\
                If you don't have access to a consistent external environment (e.g. targeting wasm, embedding \
                    rustpython in another application), try enabling the `freeze-stdlib` feature.\n\
                If this is intended and you want to exclude the encodings module from your interpreter, please remove the `encodings` feature from `rustpython-vm` crate."
            );

            let err = self.new_runtime_error(msg);
            err.set_cause(Some(import_err));
            err
        })?;
        Ok(())
    }

    fn import_utf8_encodings(&mut self) -> PyResult<()> {
        import::import_frozen(self, "codecs")?;
        // FIXME: See corresponding part of `core_frozen_inits`
        // let encoding_module_name = if cfg!(feature = "freeze-stdlib") {
        //     "encodings.utf_8"
        // } else {
        //     "encodings_utf_8"
        // };
        let encoding_module_name = "encodings_utf_8";
        let encoding_module = import::import_frozen(self, encoding_module_name)?;
        let getregentry = encoding_module.get_attr("getregentry", self)?;
        let codec_info = getregentry.call((), self)?;
        self.state
            .codec_registry
            .register_manual("utf-8", codec_info.try_into_value(self)?)?;
        Ok(())
    }

    fn initialize(&mut self) {
        flame_guard!("init VirtualMachine");

        if self.initialized {
            panic!("Double Initialize Error");
        }

        stdlib::builtins::init_module(self, &self.builtins);
        stdlib::sys::init_module(self, &self.sys_module, &self.builtins);

        let mut essential_init = || -> PyResult {
            #[cfg(not(target_arch = "wasm32"))]
            import::import_builtin(self, "_signal")?;
            #[cfg(any(feature = "parser", feature = "compiler"))]
            import::import_builtin(self, "_ast")?;
            #[cfg(not(feature = "threading"))]
            import::import_frozen(self, "_thread")?;
            let importlib = import::init_importlib_base(self)?;
            self.import_utf8_encodings()?;

            #[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
            {
                // this isn't fully compatible with CPython; it imports "io" and sets
                // builtins.open to io.OpenWrapper, but this is easier, since it doesn't
                // require the Python stdlib to be present
                let io = import::import_builtin(self, "_io")?;
                let set_stdio = |name, fd, mode: &str| {
                    let stdio = crate::stdlib::io::open(
                        self.ctx.new_int(fd).into(),
                        Some(mode),
                        Default::default(),
                        self,
                    )?;
                    let dunder_name = self.ctx.intern_str(format!("__{name}__"));
                    self.sys_module.set_attr(
                        dunder_name, // e.g. __stdin__
                        stdio.clone(),
                        self,
                    )?;
                    self.sys_module.set_attr(name, stdio, self)?;
                    Ok(())
                };
                set_stdio("stdin", 0, "r")?;
                set_stdio("stdout", 1, "w")?;
                set_stdio("stderr", 2, "w")?;

                let io_open = io.get_attr("open", self)?;
                self.builtins.set_attr("open", io_open, self)?;
            }

            Ok(importlib)
        };

        let res = essential_init();
        let importlib = self.expect_pyresult(res, "essential initialization failed");

        if self.state.settings.allow_external_library && cfg!(feature = "rustpython-compiler") {
            if let Err(e) = import::init_importlib_package(self, importlib) {
                eprintln!("importlib initialization failed. This is critical for many complicated packages.");
                self.print_exception(e);
            }
        }

        #[cfg(feature = "encodings")]
        if cfg!(feature = "freeze-stdlib") || !self.state.settings.path_list.is_empty() {
            if let Err(e) = self.import_encodings() {
                eprintln!(
                    "encodings initialization failed. Only utf-8 encoding will be supported."
                );
                self.print_exception(e);
            }
        } else {
            // Here may not be the best place to give general `path_list` advice,
            // but bare rustpython_vm::VirtualMachine users skipped proper settings must hit here while properly setup vm never enters here.
            eprintln!(
                "feature `encodings` is enabled but `settings.path_list` is empty. \
                Please add the library path to `settings.path_list`. If you intended to disable the entire standard library (including the `encodings` feature), please also make sure to disable the `encodings` feature.\n\
                Tip: You may also want to add `\"\"` to `settings.path_list` in order to enable importing from the current working directory."
            );
        }

        self.initialized = true;
    }

    fn state_mut(&mut self) -> &mut PyGlobalState {
        PyRc::get_mut(&mut self.state)
            .expect("there should not be multiple threads while a user has a mut ref to a vm")
    }

    /// Can only be used in the initialization closure passed to [`Interpreter::with_init`]
    pub fn add_native_module<S>(&mut self, name: S, module: stdlib::StdlibInitFunc)
    where
        S: Into<Cow<'static, str>>,
    {
        self.state_mut().module_inits.insert(name.into(), module);
    }

    pub fn add_native_modules<I>(&mut self, iter: I)
    where
        I: IntoIterator<Item = (Cow<'static, str>, stdlib::StdlibInitFunc)>,
    {
        self.state_mut().module_inits.extend(iter);
    }

    /// Can only be used in the initialization closure passed to [`Interpreter::with_init`]
    pub fn add_frozen<I>(&mut self, frozen: I)
    where
        I: IntoIterator<Item = (&'static str, FrozenModule)>,
    {
        self.state_mut().frozen.extend(frozen);
    }

    /// Set the custom signal channel for the interpreter
    pub fn set_user_signal_channel(&mut self, signal_rx: signal::UserSignalReceiver) {
        self.signal_rx = Some(signal_rx);
    }

    pub fn run_code_obj(&self, code: PyRef<PyCode>, scope: Scope) -> PyResult {
        let frame = Frame::new(code, scope, self.builtins.dict(), &[], self).into_ref(&self.ctx);
        self.run_frame(frame)
    }

    #[cold]
    pub fn run_unraisable(&self, e: PyBaseExceptionRef, msg: Option<String>, object: PyObjectRef) {
        let sys_module = self.import("sys", None, 0).unwrap();
        let unraisablehook = sys_module.get_attr("unraisablehook", self).unwrap();

        let exc_type = e.class().to_owned();
        let exc_traceback = e.traceback().to_pyobject(self); // TODO: actual traceback
        let exc_value = e.into();
        let args = stdlib::sys::UnraisableHookArgs {
            exc_type,
            exc_value,
            exc_traceback,
            err_msg: self.new_pyobj(msg),
            object,
        };
        if let Err(e) = unraisablehook.call((args,), self) {
            println!("{}", e.as_object().repr(self).unwrap().as_str());
        }
    }

    #[inline(always)]
    pub fn run_frame(&self, frame: FrameRef) -> PyResult {
        match self.with_frame(frame, |f| f.run(self))? {
            ExecutionResult::Return(value) => Ok(value),
            _ => panic!("Got unexpected result from function"),
        }
    }

    pub fn current_recursion_depth(&self) -> usize {
        self.recursion_depth.get()
    }

    /// Used to run the body of a (possibly) recursive function. It will raise a
    /// RecursionError if recursive functions are nested far too many times,
    /// preventing a stack overflow.
    pub fn with_recursion<R, F: FnOnce() -> PyResult<R>>(&self, _where: &str, f: F) -> PyResult<R> {
        self.check_recursive_call(_where)?;
        self.recursion_depth.set(self.recursion_depth.get() + 1);
        let result = f();
        self.recursion_depth.set(self.recursion_depth.get() - 1);
        result
    }

    pub fn with_frame<R, F: FnOnce(FrameRef) -> PyResult<R>>(
        &self,
        frame: FrameRef,
        f: F,
    ) -> PyResult<R> {
        self.with_recursion("", || {
            self.frames.borrow_mut().push(frame.clone());
            let result = f(frame);
            // defer dec frame
            let _popped = self.frames.borrow_mut().pop();
            result
        })
    }

    /// Returns a basic CompileOpts instance with options accurate to the vm. Used
    /// as the CompileOpts for `vm.compile()`.
    #[cfg(feature = "rustpython-codegen")]
    pub fn compile_opts(&self) -> crate::compiler::CompileOpts {
        crate::compiler::CompileOpts {
            optimize: self.state.settings.optimize,
        }
    }

    // To be called right before raising the recursion depth.
    fn check_recursive_call(&self, _where: &str) -> PyResult<()> {
        if self.recursion_depth.get() >= self.recursion_limit.get() {
            Err(self.new_recursion_error(format!("maximum recursion depth exceeded {_where}")))
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

    pub fn current_locals(&self) -> PyResult<ArgMapping> {
        self.current_frame()
            .expect("called current_locals but no frames on the stack")
            .locals(self)
    }

    pub fn current_globals(&self) -> Ref<PyDictRef> {
        let frame = self
            .current_frame()
            .expect("called current_globals but no frames on the stack");
        Ref::map(frame, |f| &f.globals)
    }

    pub fn try_class(&self, module: &'static str, class: &'static str) -> PyResult<PyTypeRef> {
        let class = self
            .import(module, None, 0)?
            .get_attr(class, self)?
            .downcast()
            .expect("not a class");
        Ok(class)
    }

    pub fn class(&self, module: &'static str, class: &'static str) -> PyTypeRef {
        let module = self
            .import(module, None, 0)
            .unwrap_or_else(|_| panic!("unable to import {module}"));

        let class = module
            .get_attr(class, self)
            .unwrap_or_else(|_| panic!("module {module:?} has no class {class}"));
        class.downcast().expect("not a class")
    }

    #[inline]
    pub fn import<'a>(
        &self,
        module_name: impl AsPyStr<'a>,
        from_list: Option<PyTupleTyped<PyStrRef>>,
        level: usize,
    ) -> PyResult {
        let module_name = module_name.as_pystr(&self.ctx);
        self.import_inner(module_name, from_list, level)
    }

    fn import_inner(
        &self,
        module: &Py<PyStr>,
        from_list: Option<PyTupleTyped<PyStrRef>>,
        level: usize,
    ) -> PyResult {
        // if the import inputs seem weird, e.g a package import or something, rather than just
        // a straight `import ident`
        let weird = module.as_str().contains('.')
            || level != 0
            || from_list.as_ref().map_or(false, |x| !x.is_empty());

        let cached_module = if weird {
            None
        } else {
            let sys_modules = self.sys_module.get_attr("modules", self)?;
            sys_modules.get_item(module, self).ok()
        };

        match cached_module {
            Some(cached_module) => {
                if self.is_none(&cached_module) {
                    Err(self.new_import_error(
                        format!("import of {module} halted; None in sys.modules"),
                        module.to_owned(),
                    ))
                } else {
                    Ok(cached_module)
                }
            }
            None => {
                let import_func = self
                    .builtins
                    .get_attr(identifier!(self, __import__), self)
                    .map_err(|_| {
                        self.new_import_error("__import__ not found".to_owned(), module.to_owned())
                    })?;

                let (locals, globals) = if let Some(frame) = self.current_frame() {
                    (Some(frame.locals.clone()), Some(frame.globals.clone()))
                } else {
                    (None, None)
                };
                let from_list = match from_list {
                    Some(tup) => tup.to_pyobject(self),
                    None => self.new_tuple(()).into(),
                };
                import_func
                    .call((module.to_owned(), globals, locals, from_list, level), self)
                    .map_err(|exc| import::remove_importlib_frames(self, &exc))
            }
        }
    }

    pub fn extract_elements_with<T, F>(&self, value: &PyObject, func: F) -> PyResult<Vec<T>>
    where
        F: Fn(PyObjectRef) -> PyResult<T>,
    {
        // Extract elements from item, if possible:
        let cls = value.class();
        let list_borrow;
        let slice = if cls.is(self.ctx.types.tuple_type) {
            value.payload::<PyTuple>().unwrap().as_slice()
        } else if cls.is(self.ctx.types.list_type) {
            list_borrow = value.payload::<PyList>().unwrap().borrow_vec();
            &list_borrow
        } else {
            return self.map_pyiter(value, func);
        };
        slice.iter().map(|obj| func(obj.clone())).collect()
    }

    pub fn map_iterable_object<F, R>(&self, obj: &PyObject, mut f: F) -> PyResult<PyResult<Vec<R>>>
    where
        F: FnMut(PyObjectRef) -> PyResult<R>,
    {
        match_class!(match obj {
            ref l @ PyList => {
                let mut i: usize = 0;
                let mut results = Vec::with_capacity(l.borrow_vec().len());
                loop {
                    let elem = {
                        let elements = &*l.borrow_vec();
                        if i >= elements.len() {
                            results.shrink_to_fit();
                            return Ok(Ok(results));
                        } else {
                            elements[i].clone()
                        }
                        // free the lock
                    };
                    match f(elem) {
                        Ok(result) => results.push(result),
                        Err(err) => return Ok(Err(err)),
                    }
                    i += 1;
                }
            }
            ref t @ PyTuple => Ok(t.iter().cloned().map(f).collect()),
            // TODO: put internal iterable type
            obj => {
                Ok(self.map_pyiter(obj, f))
            }
        })
    }

    fn map_pyiter<F, R>(&self, value: &PyObject, mut f: F) -> PyResult<Vec<R>>
    where
        F: FnMut(PyObjectRef) -> PyResult<R>,
    {
        let iter = value.to_owned().get_iter(self)?;
        let cap = match self.length_hint_opt(value.to_owned()) {
            Err(e) if e.class().is(self.ctx.exceptions.runtime_error) => return Err(e),
            Ok(Some(value)) => Some(value),
            // Use a power of 2 as a default capacity.
            _ => None,
        };
        // TODO: fix extend to do this check (?), see test_extend in Lib/test/list_tests.py,
        // https://github.com/python/cpython/blob/v3.9.0/Objects/listobject.c#L922-L928
        if let Some(cap) = cap {
            if cap >= isize::max_value() as usize {
                return Ok(Vec::new());
            }
        }

        let mut results = PyIterIter::new(self, iter.as_ref(), cap)
            .map(|element| f(element?))
            .collect::<PyResult<Vec<_>>>()?;
        results.shrink_to_fit();
        Ok(results)
    }

    pub fn get_attribute_opt<'a>(
        &self,
        obj: PyObjectRef,
        attr_name: impl AsPyStr<'a>,
    ) -> PyResult<Option<PyObjectRef>> {
        let attr_name = attr_name.as_pystr(&self.ctx);
        match obj.get_attr_inner(attr_name, self) {
            Ok(attr) => Ok(Some(attr)),
            Err(e) if e.fast_isinstance(self.ctx.exceptions.attribute_error) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn set_attribute_error_context(
        &self,
        exc: &PyBaseExceptionRef,
        obj: PyObjectRef,
        name: PyStrRef,
    ) {
        if exc.class().is(self.ctx.exceptions.attribute_error) {
            let exc = exc.as_object();
            exc.set_attr("name", name, self).unwrap();
            exc.set_attr("obj", obj, self).unwrap();
        }
    }

    // get_method should be used for internal access to magic methods (by-passing
    // the full getattribute look-up.
    pub fn get_method_or_type_error<F>(
        &self,
        obj: PyObjectRef,
        method_name: &'static PyStrInterned,
        err_msg: F,
    ) -> PyResult
    where
        F: FnOnce() -> String,
    {
        let method = obj
            .class()
            .get_attr(method_name)
            .ok_or_else(|| self.new_type_error(err_msg()))?;
        self.call_if_get_descriptor(&method, obj)
    }

    // TODO: remove + transfer over to get_special_method
    pub(crate) fn get_method(
        &self,
        obj: PyObjectRef,
        method_name: &'static PyStrInterned,
    ) -> Option<PyResult> {
        let method = obj.get_class_attr(method_name)?;
        Some(self.call_if_get_descriptor(&method, obj))
    }

    pub(crate) fn get_str_method(&self, obj: PyObjectRef, method_name: &str) -> Option<PyResult> {
        let method_name = self.ctx.interned_str(method_name)?;
        self.get_method(obj, method_name)
    }

    #[inline]
    /// Checks for triggered signals and calls the appropriate handlers. A no-op on
    /// platforms where signals are not supported.
    pub fn check_signals(&self) -> PyResult<()> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            crate::signal::check_signals(self)
        }
        #[cfg(target_arch = "wasm32")]
        {
            Ok(())
        }
    }

    pub(crate) fn push_exception(&self, exc: Option<PyBaseExceptionRef>) {
        let mut excs = self.exceptions.borrow_mut();
        let prev = std::mem::take(&mut *excs);
        excs.prev = Some(Box::new(prev));
        excs.exc = exc
    }

    pub(crate) fn pop_exception(&self) -> Option<PyBaseExceptionRef> {
        let mut excs = self.exceptions.borrow_mut();
        let cur = std::mem::take(&mut *excs);
        *excs = *cur.prev.expect("pop_exception() without nested exc stack");
        cur.exc
    }

    pub(crate) fn take_exception(&self) -> Option<PyBaseExceptionRef> {
        self.exceptions.borrow_mut().exc.take()
    }

    pub(crate) fn current_exception(&self) -> Option<PyBaseExceptionRef> {
        self.exceptions.borrow().exc.clone()
    }

    pub(crate) fn set_exception(&self, exc: Option<PyBaseExceptionRef>) {
        // don't be holding the RefCell guard while __del__ is called
        let prev = std::mem::replace(&mut self.exceptions.borrow_mut().exc, exc);
        drop(prev);
    }

    pub(crate) fn contextualize_exception(&self, exception: &PyBaseExceptionRef) {
        if let Some(context_exc) = self.topmost_exception() {
            if !context_exc.is(exception) {
                let mut o = context_exc.clone();
                while let Some(context) = o.context() {
                    if context.is(exception) {
                        o.set_context(None);
                        break;
                    }
                    o = context;
                }
                exception.set_context(Some(context_exc))
            }
        }
    }

    pub(crate) fn topmost_exception(&self) -> Option<PyBaseExceptionRef> {
        let excs = self.exceptions.borrow();
        let mut cur = &*excs;
        loop {
            if let Some(exc) = &cur.exc {
                return Some(exc.clone());
            }
            cur = cur.prev.as_deref()?;
        }
    }

    pub fn handle_exit_exception(&self, exc: PyBaseExceptionRef) -> u8 {
        if exc.fast_isinstance(self.ctx.exceptions.system_exit) {
            let args = exc.args();
            let msg = match args.as_slice() {
                [] => return 0,
                [arg] => match_class!(match arg {
                    ref i @ PyInt => {
                        use num_traits::cast::ToPrimitive;
                        return i.as_bigint().to_u8().unwrap_or(0);
                    }
                    arg => {
                        if self.is_none(arg) {
                            return 0;
                        } else {
                            arg.str(self).ok()
                        }
                    }
                }),
                _ => args.as_object().repr(self).ok(),
            };
            if let Some(msg) = msg {
                let stderr = stdlib::sys::PyStderr(self);
                writeln!(stderr, "{msg}");
            }
            1
        } else if exc.fast_isinstance(self.ctx.exceptions.keyboard_interrupt) {
            #[allow(clippy::if_same_then_else)]
            {
                self.print_exception(exc);
                #[cfg(unix)]
                {
                    let action = SigAction::new(
                        nix::sys::signal::SigHandler::SigDfl,
                        SaFlags::SA_ONSTACK,
                        SigSet::empty(),
                    );
                    let result = unsafe { sigaction(SIGINT, &action) };
                    if result.is_ok() {
                        interpreter::flush_std(self);
                        kill(getpid(), SIGINT).expect("Expect to be killed.");
                    }

                    (libc::SIGINT as u8) + 128u8
                }
                #[cfg(not(unix))]
                {
                    1
                }
            }
        } else {
            self.print_exception(exc);
            1
        }
    }

    #[doc(hidden)]
    pub fn __module_set_attr(
        &self,
        module: &Py<PyModule>,
        attr_name: &'static PyStrInterned,
        attr_value: impl Into<PyObjectRef>,
    ) -> PyResult<()> {
        let val = attr_value.into();
        module
            .as_object()
            .generic_setattr(attr_name, PySetterValue::Assign(val), self)
    }

    pub fn insert_sys_path(&self, obj: PyObjectRef) -> PyResult<()> {
        let sys_path = self.sys_module.get_attr("path", self).unwrap();
        self.call_method(&sys_path, "insert", (0, obj))?;
        Ok(())
    }

    pub fn run_module(&self, module: &str) -> PyResult<()> {
        let runpy = self.import("runpy", None, 0)?;
        let run_module_as_main = runpy.get_attr("_run_module_as_main", self)?;
        run_module_as_main.call((module,), self)?;
        Ok(())
    }
}

impl AsRef<Context> for VirtualMachine {
    fn as_ref(&self) -> &Context {
        &self.ctx
    }
}

fn core_frozen_inits() -> impl Iterator<Item = (&'static str, FrozenModule)> {
    let iter = std::iter::empty();
    macro_rules! ext_modules {
        ($iter:ident, $($t:tt)*) => {
            let $iter = $iter.chain(py_freeze!($($t)*));
        };
    }

    // keep as example but use file one now
    // ext_modules!(
    //     iter,
    //     source = "initialized = True; print(\"Hello world!\")\n",
    //     module_name = "__hello__",
    // );

    // Python modules that the vm calls into, but are not actually part of the stdlib. They could
    // in theory be implemented in Rust, but are easiest to do in Python for one reason or another.
    // Includes _importlib_bootstrap and _importlib_bootstrap_external
    ext_modules!(
        iter,
        dir = "./Lib/python_builtins",
        crate_name = "rustpython_compiler_core"
    );

    // core stdlib Python modules that the vm calls into, but are still used in Python
    // application code, e.g. copyreg
    // FIXME: Initializing core_modules here results duplicated frozen module generation for core_modules.
    // We need a way to initialize this modules for both `Interpreter::without_stdlib()` and `InterpreterConfig::new().init_stdlib().interpreter()`
    // #[cfg(not(feature = "freeze-stdlib"))]
    ext_modules!(
        iter,
        dir = "./Lib/core_modules",
        crate_name = "rustpython_compiler_core"
    );

    iter
}

#[test]
fn test_nested_frozen() {
    use rustpython_vm as vm;

    vm::Interpreter::with_init(Default::default(), |vm| {
        // vm.add_native_modules(rustpython_stdlib::get_module_inits());
        vm.add_frozen(rustpython_vm::py_freeze!(dir = "../extra_tests/snippets"));
    })
    .enter(|vm| {
        let scope = vm.new_scope_with_builtins();

        let source = "from dir_module.dir_module_inner import value2";
        let code_obj = vm
            .compile(source, vm::compiler::Mode::Exec, "<embedded>".to_owned())
            .map_err(|err| vm.new_syntax_error(&err, Some(source)))
            .unwrap();

        if let Err(e) = vm.run_code_obj(code_obj, scope) {
            vm.print_exception(e);
            panic!();
        }
    })
}
