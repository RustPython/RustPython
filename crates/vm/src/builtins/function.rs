#[cfg(feature = "jit")]
mod jit;

use super::{
    PyAsyncGen, PyCode, PyCoroutine, PyDictRef, PyGenerator, PyList, PyModule, PyStr, PyStrRef,
    PyTuple, PyTupleRef, PyType, object,
};
use crate::common::hash::PyHash;
use crate::common::lock::PyMutex;
use crate::function::ArgMapping;
use crate::object::{PyAtomicRef, Traverse, TraverseFn};
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    bytecode,
    class::PyClassImpl,
    common::wtf8::{Wtf8Buf, wtf8_concat},
    frame::{Frame, FrameRef},
    function::{FuncArgs, OptionalArg, PyComparisonValue, PySetterValue},
    scope::Scope,
    types::{
        Callable, Comparable, Constructor, GetAttr, GetDescriptor, Hashable, PyComparisonOp,
        Representable,
    },
};
use core::sync::atomic::{AtomicU32, Ordering::Relaxed};
use itertools::Itertools;
#[cfg(feature = "jit")]
use rustpython_jit::CompiledCode;

fn format_missing_args(
    qualname: impl core::fmt::Display,
    kind: &str,
    missing: &mut Vec<impl core::fmt::Display>,
) -> String {
    let count = missing.len();
    let last = if missing.len() > 1 {
        missing.pop()
    } else {
        None
    };
    let (and, right): (&str, String) = if let Some(last) = last {
        (
            if missing.len() == 1 {
                "' and '"
            } else {
                "', and '"
            },
            format!("{last}"),
        )
    } else {
        ("", String::new())
    };
    format!(
        "{qualname}() missing {count} required {kind} argument{}: '{}{}{right}'",
        if count == 1 { "" } else { "s" },
        missing.iter().join("', '"),
        and,
    )
}

#[pyclass(module = false, name = "function", traverse = "manual")]
#[derive(Debug)]
pub struct PyFunction {
    code: PyAtomicRef<PyCode>,
    globals: PyDictRef,
    builtins: PyObjectRef,
    pub(crate) closure: Option<PyRef<PyTuple<PyCellRef>>>,
    defaults_and_kwdefaults: PyMutex<(Option<PyTupleRef>, Option<PyDictRef>)>,
    name: PyMutex<PyStrRef>,
    qualname: PyMutex<PyStrRef>,
    type_params: PyMutex<PyTupleRef>,
    annotations: PyMutex<Option<PyDictRef>>,
    annotate: PyMutex<Option<PyObjectRef>>,
    module: PyMutex<PyObjectRef>,
    doc: PyMutex<PyObjectRef>,
    func_version: AtomicU32,
    #[cfg(feature = "jit")]
    jitted_code: PyMutex<Option<CompiledCode>>,
}

static FUNC_VERSION_COUNTER: AtomicU32 = AtomicU32::new(1);

/// Atomically allocate the next function version, returning 0 if exhausted.
/// Once the counter wraps to 0, it stays at 0 permanently.
fn next_func_version() -> u32 {
    FUNC_VERSION_COUNTER
        .fetch_update(Relaxed, Relaxed, |v| (v != 0).then(|| v.wrapping_add(1)))
        .unwrap_or(0)
}

unsafe impl Traverse for PyFunction {
    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        self.globals.traverse(tracer_fn);
        if let Some(closure) = self.closure.as_ref() {
            closure.as_untyped().traverse(tracer_fn);
        }
        self.defaults_and_kwdefaults.traverse(tracer_fn);
        // Traverse additional fields that may contain references
        self.type_params.lock().traverse(tracer_fn);
        self.annotations.lock().traverse(tracer_fn);
        self.module.lock().traverse(tracer_fn);
        self.doc.lock().traverse(tracer_fn);
    }

    fn clear(&mut self, out: &mut Vec<crate::PyObjectRef>) {
        // Pop closure if present (equivalent to Py_CLEAR(func_closure))
        if let Some(closure) = self.closure.take() {
            out.push(closure.into());
        }

        // Pop defaults and kwdefaults
        if let Some(mut guard) = self.defaults_and_kwdefaults.try_lock() {
            if let Some(defaults) = guard.0.take() {
                out.push(defaults.into());
            }
            if let Some(kwdefaults) = guard.1.take() {
                out.push(kwdefaults.into());
            }
        }

        // Clear annotations and annotate (Py_CLEAR)
        if let Some(mut guard) = self.annotations.try_lock()
            && let Some(annotations) = guard.take()
        {
            out.push(annotations.into());
        }
        if let Some(mut guard) = self.annotate.try_lock()
            && let Some(annotate) = guard.take()
        {
            out.push(annotate);
        }

        // Clear module, doc, and type_params (Py_CLEAR)
        if let Some(mut guard) = self.module.try_lock() {
            let old_module =
                core::mem::replace(&mut *guard, Context::genesis().none.to_owned().into());
            out.push(old_module);
        }
        if let Some(mut guard) = self.doc.try_lock() {
            let old_doc =
                core::mem::replace(&mut *guard, Context::genesis().none.to_owned().into());
            out.push(old_doc);
        }
        if let Some(mut guard) = self.type_params.try_lock() {
            let old_type_params =
                core::mem::replace(&mut *guard, Context::genesis().empty_tuple.to_owned());
            out.push(old_type_params.into());
        }

        // Replace name and qualname with empty string to break potential str subclass cycles
        // name and qualname could be str subclasses, so they could have reference cycles
        if let Some(mut guard) = self.name.try_lock() {
            let old_name = core::mem::replace(&mut *guard, Context::genesis().empty_str.to_owned());
            out.push(old_name.into());
        }
        if let Some(mut guard) = self.qualname.try_lock() {
            let old_qualname =
                core::mem::replace(&mut *guard, Context::genesis().empty_str.to_owned());
            out.push(old_qualname.into());
        }

        // Note: globals, builtins, code are NOT cleared (required to be non-NULL)
    }
}

impl PyFunction {
    #[inline]
    pub(crate) fn new(
        code: PyRef<PyCode>,
        globals: PyDictRef,
        vm: &VirtualMachine,
    ) -> PyResult<Self> {
        let name = PyMutex::new(code.obj_name.to_owned());
        let module = vm.unwrap_or_none(globals.get_item_opt(identifier!(vm, __name__), vm)?);
        let builtins = globals.get_item("__builtins__", vm).unwrap_or_else(|_| {
            // If not in globals, inherit from current execution context
            if let Some(frame) = vm.current_frame() {
                frame.builtins.clone()
            } else {
                vm.builtins.dict().into()
            }
        });
        // If builtins is a module, use its __dict__ instead
        let builtins = if let Some(module) = builtins.downcast_ref::<PyModule>() {
            module.dict().into()
        } else {
            builtins
        };

        // Get docstring from co_consts[0] if HAS_DOCSTRING flag is set
        let doc = if code.code.flags.contains(bytecode::CodeFlags::HAS_DOCSTRING) {
            code.code
                .constants
                .first()
                .map(|c| c.as_object().to_owned())
                .unwrap_or_else(|| vm.ctx.none())
        } else {
            vm.ctx.none()
        };

        let qualname = vm.ctx.new_str(code.qualname.as_str());
        let func = Self {
            code: PyAtomicRef::from(code),
            globals,
            builtins,
            closure: None,
            defaults_and_kwdefaults: PyMutex::new((None, None)),
            name,
            qualname: PyMutex::new(qualname),
            type_params: PyMutex::new(vm.ctx.empty_tuple.clone()),
            annotations: PyMutex::new(None),
            annotate: PyMutex::new(None),
            module: PyMutex::new(module),
            doc: PyMutex::new(doc),
            func_version: AtomicU32::new(next_func_version()),
            #[cfg(feature = "jit")]
            jitted_code: PyMutex::new(None),
        };
        Ok(func)
    }

    fn fill_locals_from_args(
        &self,
        frame: &Frame,
        func_args: FuncArgs,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let code: &Py<PyCode> = &self.code;
        let nargs = func_args.args.len();
        let n_expected_args = code.arg_count as usize;
        let total_args = code.arg_count as usize + code.kwonlyarg_count as usize;
        // let arg_names = self.code.arg_names();

        // This parses the arguments from args and kwargs into
        // the proper variables keeping into account default values
        // and star-args and kwargs.
        // See also: PyEval_EvalCodeWithName in cpython:
        // https://github.com/python/cpython/blob/main/Python/ceval.c#L3681

        // SAFETY: Frame was just created and not yet executing.
        let fastlocals = unsafe { frame.fastlocals_mut() };

        let mut args_iter = func_args.args.into_iter();

        // Copy positional arguments into local variables
        // zip short-circuits if either iterator returns None, which is the behavior we want --
        // only fill as much as there is to fill with as much as we have
        for (local, arg) in Iterator::zip(
            fastlocals.iter_mut().take(n_expected_args),
            args_iter.by_ref().take(nargs),
        ) {
            *local = Some(arg);
        }

        let mut vararg_offset = total_args;
        // Pack other positional arguments in to *args:
        if code.flags.contains(bytecode::CodeFlags::VARARGS) {
            let vararg_value = vm.ctx.new_tuple(args_iter.collect());
            fastlocals[vararg_offset] = Some(vararg_value.into());
            vararg_offset += 1;
        } else {
            // Check the number of positional arguments
            if nargs > n_expected_args {
                let n_defaults = self
                    .defaults_and_kwdefaults
                    .lock()
                    .0
                    .as_ref()
                    .map_or(0, |d| d.len());
                let n_required = n_expected_args - n_defaults;
                let takes_msg = if n_defaults > 0 {
                    format!("from {} to {}", n_required, n_expected_args)
                } else {
                    n_expected_args.to_string()
                };

                // Count keyword-only arguments that were actually provided
                let kw_only_given = if code.kwonlyarg_count > 0 {
                    let start = code.arg_count as usize;
                    let end = start + code.kwonlyarg_count as usize;
                    code.varnames[start..end]
                        .iter()
                        .filter(|name| func_args.kwargs.contains_key(name.as_str()))
                        .count()
                } else {
                    0
                };

                let given_msg = if kw_only_given > 0 {
                    format!(
                        "{} positional argument{} (and {} keyword-only argument{}) were",
                        nargs,
                        if nargs == 1 { "" } else { "s" },
                        kw_only_given,
                        if kw_only_given == 1 { "" } else { "s" },
                    )
                } else {
                    format!("{} {}", nargs, if nargs == 1 { "was" } else { "were" })
                };

                return Err(vm.new_type_error(format!(
                    "{}() takes {} positional argument{} but {} given",
                    self.__qualname__(),
                    takes_msg,
                    if n_expected_args == 1 { "" } else { "s" },
                    given_msg,
                )));
            }
        }

        // Do we support `**kwargs` ?
        let kwargs = if code.flags.contains(bytecode::CodeFlags::VARKEYWORDS) {
            let d = vm.ctx.new_dict();
            fastlocals[vararg_offset] = Some(d.clone().into());
            Some(d)
        } else {
            None
        };

        let arg_pos = |range: core::ops::Range<_>, name: &str| {
            code.varnames
                .iter()
                .enumerate()
                .skip(range.start)
                .take(range.end - range.start)
                .find(|(_, s)| s.as_str() == name)
                .map(|(p, _)| p)
        };

        let mut posonly_passed_as_kwarg = Vec::new();
        // Handle keyword arguments
        for (name, value) in func_args.kwargs {
            // Check if we have a parameter with this name:
            if let Some(pos) = arg_pos(code.posonlyarg_count as usize..total_args, &name) {
                let slot = &mut fastlocals[pos];
                if slot.is_some() {
                    return Err(vm.new_type_error(format!(
                        "{}() got multiple values for argument '{}'",
                        self.__qualname__(),
                        name
                    )));
                }
                *slot = Some(value);
            } else if let Some(kwargs) = kwargs.as_ref() {
                kwargs.set_item(&name, value, vm)?;
            } else if arg_pos(0..code.posonlyarg_count as usize, &name).is_some() {
                posonly_passed_as_kwarg.push(name);
            } else {
                return Err(vm.new_type_error(format!(
                    "{}() got an unexpected keyword argument '{}'",
                    self.__qualname__(),
                    name
                )));
            }
        }
        if !posonly_passed_as_kwarg.is_empty() {
            return Err(vm.new_type_error(format!(
                "{}() got some positional-only arguments passed as keyword arguments: '{}'",
                self.__qualname__(),
                posonly_passed_as_kwarg.into_iter().format(", "),
            )));
        }

        let mut defaults_and_kwdefaults = None;
        // can't be a closure cause it returns a reference to a captured variable :/
        macro_rules! get_defaults {
            () => {{
                defaults_and_kwdefaults
                    .get_or_insert_with(|| self.defaults_and_kwdefaults.lock().clone())
            }};
        }

        // Add missing positional arguments, if we have fewer positional arguments than the
        // function definition calls for
        if nargs < n_expected_args {
            let defaults = get_defaults!().0.as_ref().map(|tup| tup.as_slice());
            let n_defs = defaults.map_or(0, |d| d.len());

            let n_required = code.arg_count as usize - n_defs;

            // Given the number of defaults available, check all the arguments for which we
            // _don't_ have defaults; if any are missing, raise an exception
            let mut missing: Vec<_> = (nargs..n_required)
                .filter_map(|i| {
                    if fastlocals[i].is_none() {
                        Some(&code.varnames[i])
                    } else {
                        None
                    }
                })
                .collect();

            if !missing.is_empty() {
                return Err(vm.new_type_error(format_missing_args(
                    self.__qualname__(),
                    "positional",
                    &mut missing,
                )));
            }

            if let Some(defaults) = defaults {
                let n = core::cmp::min(nargs, n_expected_args);
                let i = n.saturating_sub(n_required);

                // We have sufficient defaults, so iterate over the corresponding names and use
                // the default if we don't already have a value
                for i in i..defaults.len() {
                    let slot = &mut fastlocals[n_required + i];
                    if slot.is_none() {
                        *slot = Some(defaults[i].clone());
                    }
                }
            }
        };

        if code.kwonlyarg_count > 0 {
            let mut missing = Vec::new();
            // Check if kw only arguments are all present:
            for (slot, kwarg) in fastlocals
                .iter_mut()
                .zip(&*code.varnames)
                .skip(code.arg_count as usize)
                .take(code.kwonlyarg_count as usize)
                .filter(|(slot, _)| slot.is_none())
            {
                if let Some(defaults) = &get_defaults!().1
                    && let Some(default) = defaults.get_item_opt(&**kwarg, vm)?
                {
                    *slot = Some(default);
                    continue;
                }

                // No default value and not specified.
                missing.push(kwarg);
            }

            if !missing.is_empty() {
                return Err(vm.new_type_error(format_missing_args(
                    self.__qualname__(),
                    "keyword-only",
                    &mut missing,
                )));
            }
        }

        Ok(())
    }

    /// Set function attribute based on MakeFunctionFlags
    pub(crate) fn set_function_attribute(
        &mut self,
        attr: bytecode::MakeFunctionFlag,
        attr_value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        use crate::builtins::PyDict;
        match attr {
            bytecode::MakeFunctionFlag::Defaults => {
                let defaults = match attr_value.downcast::<PyTuple>() {
                    Ok(tuple) => tuple,
                    Err(obj) => {
                        return Err(vm.new_type_error(format!(
                            "__defaults__ must be a tuple, not {}",
                            obj.class().name()
                        )));
                    }
                };
                self.defaults_and_kwdefaults.lock().0 = Some(defaults);
            }
            bytecode::MakeFunctionFlag::KwOnlyDefaults => {
                let kwdefaults = match attr_value.downcast::<PyDict>() {
                    Ok(dict) => dict,
                    Err(obj) => {
                        return Err(vm.new_type_error(format!(
                            "__kwdefaults__ must be a dict, not {}",
                            obj.class().name()
                        )));
                    }
                };
                self.defaults_and_kwdefaults.lock().1 = Some(kwdefaults);
            }
            bytecode::MakeFunctionFlag::Annotations => {
                let annotations = match attr_value.downcast::<PyDict>() {
                    Ok(dict) => dict,
                    Err(obj) => {
                        return Err(vm.new_type_error(format!(
                            "__annotations__ must be a dict, not {}",
                            obj.class().name()
                        )));
                    }
                };
                *self.annotations.lock() = Some(annotations);
            }
            bytecode::MakeFunctionFlag::Closure => {
                let closure_tuple = attr_value
                    .downcast_exact::<PyTuple>(vm)
                    .map_err(|obj| {
                        vm.new_type_error(format!(
                            "closure must be a tuple, not {}",
                            obj.class().name()
                        ))
                    })?
                    .into_pyref();

                self.closure = Some(closure_tuple.try_into_typed::<PyCell>(vm)?);
            }
            bytecode::MakeFunctionFlag::TypeParams => {
                let type_params = attr_value.clone().downcast::<PyTuple>().map_err(|_| {
                    vm.new_type_error(format!(
                        "__type_params__ must be a tuple, not {}",
                        attr_value.class().name()
                    ))
                })?;
                *self.type_params.lock() = type_params;
            }
            bytecode::MakeFunctionFlag::Annotate => {
                if !attr_value.is_callable() {
                    return Err(vm.new_type_error("__annotate__ must be callable"));
                }
                *self.annotate.lock() = Some(attr_value);
            }
        }
        Ok(())
    }
}

impl Py<PyFunction> {
    pub(crate) fn is_optimized_for_call_specialization(&self) -> bool {
        self.code.flags.contains(bytecode::CodeFlags::OPTIMIZED)
    }

    pub fn invoke_with_locals(
        &self,
        func_args: FuncArgs,
        locals: Option<ArgMapping>,
        vm: &VirtualMachine,
    ) -> PyResult {
        #[cfg(feature = "jit")]
        if let Some(jitted_code) = self.jitted_code.lock().as_ref() {
            use crate::convert::ToPyObject;
            match jit::get_jit_args(self, &func_args, jitted_code, vm) {
                Ok(args) => {
                    return Ok(args.invoke().to_pyobject(vm));
                }
                Err(err) => info!(
                    "jit: function `{}` is falling back to being interpreted because of the \
                    error: {}",
                    self.code.obj_name, err
                ),
            }
        }

        let code: PyRef<PyCode> = (*self.code).to_owned();

        let locals = if code.flags.contains(bytecode::CodeFlags::NEWLOCALS) {
            None
        } else if let Some(locals) = locals {
            Some(locals)
        } else {
            Some(ArgMapping::from_dict_exact(self.globals.clone()))
        };

        let is_gen = code.flags.contains(bytecode::CodeFlags::GENERATOR);
        let is_coro = code.flags.contains(bytecode::CodeFlags::COROUTINE);
        let use_datastack = !(is_gen || is_coro);

        // Construct frame:
        let frame = Frame::new(
            code,
            Scope::new(locals, self.globals.clone()),
            self.builtins.clone(),
            self.closure.as_ref().map_or(&[], |c| c.as_slice()),
            Some(self.to_owned().into()),
            use_datastack,
            vm,
        )
        .into_ref(&vm.ctx);

        self.fill_locals_from_args(&frame, func_args, vm)?;
        match (is_gen, is_coro) {
            (true, false) => {
                let obj = PyGenerator::new(frame.clone(), self.__name__(), self.__qualname__())
                    .into_pyobject(vm);
                frame.set_generator(&obj);
                Ok(obj)
            }
            (false, true) => {
                let obj = PyCoroutine::new(frame.clone(), self.__name__(), self.__qualname__())
                    .into_pyobject(vm);
                frame.set_generator(&obj);
                Ok(obj)
            }
            (true, true) => {
                let obj = PyAsyncGen::new(frame.clone(), self.__name__(), self.__qualname__())
                    .into_pyobject(vm);
                frame.set_generator(&obj);
                Ok(obj)
            }
            (false, false) => {
                let result = vm.run_frame(frame.clone());
                // Release data stack memory after frame execution completes.
                unsafe {
                    if let Some(base) = frame.materialize_localsplus() {
                        vm.datastack_pop(base);
                    }
                }
                result
            }
        }
    }

    #[inline(always)]
    pub fn invoke(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        self.invoke_with_locals(func_args, None, vm)
    }

    /// Returns the function version, or 0 if invalidated.
    #[inline]
    pub fn func_version(&self) -> u32 {
        self.func_version.load(Relaxed)
    }

    /// Returns the current version, assigning a fresh one if previously invalidated.
    /// Returns 0 if the version counter has overflowed.
    /// `_PyFunction_GetVersionForCurrentState`
    pub fn get_version_for_current_state(&self) -> u32 {
        let v = self.func_version.load(Relaxed);
        if v != 0 {
            return v;
        }
        let new_v = next_func_version();
        if new_v == 0 {
            return 0;
        }
        self.func_version.store(new_v, Relaxed);
        new_v
    }

    /// function_kind(SIMPLE_FUNCTION) equivalent for CALL specialization.
    /// Returns true if: CO_OPTIMIZED, no VARARGS, no VARKEYWORDS, no kwonly args.
    pub(crate) fn is_simple_for_call_specialization(&self) -> bool {
        let code: &Py<PyCode> = &self.code;
        let flags = code.flags;
        flags.contains(bytecode::CodeFlags::OPTIMIZED)
            && !flags.intersects(bytecode::CodeFlags::VARARGS | bytecode::CodeFlags::VARKEYWORDS)
            && code.kwonlyarg_count == 0
    }

    /// Check if this function is eligible for exact-args call specialization.
    /// Returns true if: CO_OPTIMIZED, no VARARGS, no VARKEYWORDS, no kwonly args,
    /// and effective_nargs matches co_argcount.
    pub(crate) fn can_specialize_call(&self, effective_nargs: u32) -> bool {
        let code: &Py<PyCode> = &self.code;
        let flags = code.flags;
        flags.contains(bytecode::CodeFlags::OPTIMIZED)
            && !flags.intersects(bytecode::CodeFlags::VARARGS | bytecode::CodeFlags::VARKEYWORDS)
            && code.kwonlyarg_count == 0
            && code.arg_count == effective_nargs
    }

    /// Runtime guard for CALL_*_EXACT_ARGS specialization: check only argcount.
    /// Other invariants are guaranteed by function versioning and specialization-time checks.
    #[inline]
    pub(crate) fn has_exact_argcount(&self, effective_nargs: u32) -> bool {
        self.code.arg_count == effective_nargs
    }

    /// Bytes required for this function's frame on RustPython's thread datastack.
    /// Returns `None` for generator/coroutine code paths that do not push a
    /// regular datastack-backed frame in the fast call path.
    pub(crate) fn datastack_frame_size_bytes(&self) -> Option<usize> {
        datastack_frame_size_bytes_for_code(&self.code)
    }

    pub(crate) fn prepare_exact_args_frame(
        &self,
        mut args: Vec<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> FrameRef {
        let code: PyRef<PyCode> = (*self.code).to_owned();

        debug_assert_eq!(args.len(), code.arg_count as usize);
        debug_assert!(code.flags.contains(bytecode::CodeFlags::OPTIMIZED));
        debug_assert!(
            !code
                .flags
                .intersects(bytecode::CodeFlags::VARARGS | bytecode::CodeFlags::VARKEYWORDS)
        );
        debug_assert_eq!(code.kwonlyarg_count, 0);
        debug_assert!(
            !code
                .flags
                .intersects(bytecode::CodeFlags::GENERATOR | bytecode::CodeFlags::COROUTINE)
        );

        let locals = if code.flags.contains(bytecode::CodeFlags::NEWLOCALS) {
            None
        } else {
            Some(ArgMapping::from_dict_exact(self.globals.clone()))
        };

        let frame = Frame::new(
            code,
            Scope::new(locals, self.globals.clone()),
            self.builtins.clone(),
            self.closure.as_ref().map_or(&[], |c| c.as_slice()),
            Some(self.to_owned().into()),
            true, // Exact-args fast path is only used for non-gen/coro functions.
            vm,
        )
        .into_ref(&vm.ctx);

        {
            let fastlocals = unsafe { frame.fastlocals_mut() };
            for (slot, arg) in fastlocals.iter_mut().zip(args.drain(..)) {
                *slot = Some(arg);
            }
        }

        frame
    }

    /// Fast path for calling a simple function with exact positional args.
    /// Skips FuncArgs allocation, prepend_arg, and fill_locals_from_args.
    /// Only valid when: CO_OPTIMIZED, no VARARGS, no VARKEYWORDS, no kwonlyargs,
    /// and nargs == co_argcount.
    pub fn invoke_exact_args(&self, args: Vec<PyObjectRef>, vm: &VirtualMachine) -> PyResult {
        let code: PyRef<PyCode> = (*self.code).to_owned();

        debug_assert_eq!(args.len(), code.arg_count as usize);
        debug_assert!(code.flags.contains(bytecode::CodeFlags::OPTIMIZED));
        debug_assert!(
            !code
                .flags
                .intersects(bytecode::CodeFlags::VARARGS | bytecode::CodeFlags::VARKEYWORDS)
        );
        debug_assert_eq!(code.kwonlyarg_count, 0);

        // Generator/coroutine code objects are SIMPLE_FUNCTION in call
        // specialization classification, but their call path must still
        // go through invoke() to produce generator/coroutine objects.
        if code
            .flags
            .intersects(bytecode::CodeFlags::GENERATOR | bytecode::CodeFlags::COROUTINE)
        {
            return self.invoke(FuncArgs::from(args), vm);
        }
        let frame = self.prepare_exact_args_frame(args, vm);

        let result = vm.run_frame(frame.clone());
        unsafe {
            if let Some(base) = frame.materialize_localsplus() {
                vm.datastack_pop(base);
            }
        }
        result
    }
}

pub(crate) fn datastack_frame_size_bytes_for_code(code: &Py<PyCode>) -> Option<usize> {
    if code
        .flags
        .intersects(bytecode::CodeFlags::GENERATOR | bytecode::CodeFlags::COROUTINE)
    {
        return None;
    }
    let nlocalsplus = code.localspluskinds.len();
    let capacity = nlocalsplus.checked_add(code.max_stackdepth as usize)?;
    capacity.checked_mul(core::mem::size_of::<usize>())
}

impl PyPayload for PyFunction {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.function_type
    }
}

#[pyclass(
    with(GetDescriptor, Callable, Representable, Constructor),
    flags(HAS_DICT, HAS_WEAKREF, METHOD_DESCRIPTOR)
)]
impl PyFunction {
    #[pygetset]
    fn __code__(&self) -> PyRef<PyCode> {
        (*self.code).to_owned()
    }

    #[pygetset(setter)]
    fn set___code__(&self, code: PyRef<PyCode>, vm: &VirtualMachine) -> PyResult<()> {
        let n_free = code.freevars.len();
        let n_closure = self.closure.as_ref().map_or(0, |c| c.len());
        if n_closure != n_free {
            return Err(vm.new_value_error(format!(
                "{}() requires a code object with {} free vars, not {}",
                self.qualname.lock(),
                n_closure,
                n_free,
            )));
        }
        #[cfg(feature = "jit")]
        let mut jit_guard = self.jitted_code.lock();
        self.code.swap_to_temporary_refs(code, vm);
        #[cfg(feature = "jit")]
        {
            *jit_guard = None;
        }
        self.func_version.store(0, Relaxed);
        Ok(())
    }

    #[pygetset]
    fn __defaults__(&self) -> Option<PyTupleRef> {
        self.defaults_and_kwdefaults.lock().0.clone()
    }
    #[pygetset(setter)]
    fn set___defaults__(&self, defaults: PySetterValue<Option<PyTupleRef>>) {
        self.defaults_and_kwdefaults.lock().0 = match defaults {
            PySetterValue::Assign(d) => d,
            PySetterValue::Delete => None,
        };
        self.func_version.store(0, Relaxed);
    }

    #[pygetset]
    fn __kwdefaults__(&self) -> Option<PyDictRef> {
        self.defaults_and_kwdefaults.lock().1.clone()
    }
    #[pygetset(setter)]
    fn set___kwdefaults__(&self, kwdefaults: PySetterValue<Option<PyDictRef>>) {
        self.defaults_and_kwdefaults.lock().1 = match kwdefaults {
            PySetterValue::Assign(d) => d,
            PySetterValue::Delete => None,
        };
        self.func_version.store(0, Relaxed);
    }

    // {"__closure__",   T_OBJECT,     OFF(func_closure), READONLY},
    // {"__doc__",       T_OBJECT,     OFF(func_doc), 0},
    // {"__globals__",   T_OBJECT,     OFF(func_globals), READONLY},
    // {"__module__",    T_OBJECT,     OFF(func_module), 0},
    // {"__builtins__",  T_OBJECT,     OFF(func_builtins), READONLY},
    #[pymember]
    fn __globals__(vm: &VirtualMachine, zelf: PyObjectRef) -> PyResult {
        let zelf = Self::_as_pyref(&zelf, vm)?;
        Ok(zelf.globals.clone().into())
    }

    #[pymember]
    fn __closure__(vm: &VirtualMachine, zelf: PyObjectRef) -> PyResult {
        let zelf = Self::_as_pyref(&zelf, vm)?;
        Ok(vm.unwrap_or_none(zelf.closure.clone().map(|x| x.into())))
    }

    #[pymember]
    fn __builtins__(vm: &VirtualMachine, zelf: PyObjectRef) -> PyResult {
        let zelf = Self::_as_pyref(&zelf, vm)?;
        Ok(zelf.builtins.clone())
    }

    #[pygetset]
    fn __name__(&self) -> PyStrRef {
        self.name.lock().clone()
    }

    #[pygetset(setter)]
    fn set___name__(&self, name: PyStrRef) {
        *self.name.lock() = name;
    }

    #[pymember]
    fn __doc__(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult {
        // When accessed from instance, obj is the PyFunction instance
        if let Ok(func) = obj.downcast::<Self>() {
            let doc = func.doc.lock();
            Ok(doc.clone())
        } else {
            // When accessed from class, return None as there's no instance
            Ok(vm.ctx.none())
        }
    }

    #[pymember(setter)]
    fn set___doc__(vm: &VirtualMachine, zelf: PyObjectRef, value: PySetterValue) -> PyResult<()> {
        let zelf: PyRef<Self> = zelf.downcast().unwrap_or_else(|_| unreachable!());
        let value = value.unwrap_or_none(vm);
        *zelf.doc.lock() = value;
        Ok(())
    }

    #[pygetset]
    fn __module__(&self) -> PyObjectRef {
        self.module.lock().clone()
    }

    #[pygetset(setter)]
    fn set___module__(&self, module: PySetterValue<PyObjectRef>, vm: &VirtualMachine) {
        *self.module.lock() = module.unwrap_or_none(vm);
    }

    #[pygetset]
    fn __annotations__(&self, vm: &VirtualMachine) -> PyResult<PyDictRef> {
        // First check if we have cached annotations
        {
            let annotations = self.annotations.lock();
            if let Some(ref ann) = *annotations {
                return Ok(ann.clone());
            }
        }

        // Check for callable __annotate__ and clone it before calling
        let annotate_fn = {
            let annotate = self.annotate.lock();
            if let Some(ref func) = *annotate
                && func.is_callable()
            {
                Some(func.clone())
            } else {
                None
            }
        };

        // Release locks before calling __annotate__ to avoid deadlock
        if let Some(annotate_fn) = annotate_fn {
            let one = vm.ctx.new_int(1);
            let ann_dict = annotate_fn.call((one,), vm)?;
            let ann_dict = ann_dict
                .downcast::<crate::builtins::PyDict>()
                .map_err(|obj| {
                    vm.new_type_error(format!(
                        "__annotate__ returned non-dict of type '{}'",
                        obj.class().name()
                    ))
                })?;

            // Cache the result
            *self.annotations.lock() = Some(ann_dict.clone());
            return Ok(ann_dict);
        }

        // No __annotate__ or not callable, create empty dict
        let new_dict = vm.ctx.new_dict();
        *self.annotations.lock() = Some(new_dict.clone());
        Ok(new_dict)
    }

    #[pygetset(setter)]
    fn set___annotations__(
        &self,
        value: PySetterValue<Option<PyObjectRef>>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        match value {
            PySetterValue::Assign(Some(value)) => {
                let annotations = value.downcast::<crate::builtins::PyDict>().map_err(|_| {
                    vm.new_type_error("__annotations__ must be set to a dict object")
                })?;
                *self.annotations.lock() = Some(annotations);
                *self.annotate.lock() = None;
            }
            PySetterValue::Assign(None) => {
                *self.annotations.lock() = None;
                *self.annotate.lock() = None;
            }
            PySetterValue::Delete => {
                // del only clears cached annotations; __annotate__ is preserved
                *self.annotations.lock() = None;
            }
        }
        Ok(())
    }

    #[pygetset]
    fn __dict__(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyDictRef> {
        object::object_get_dict(zelf.as_object().to_owned(), vm)
    }

    #[pygetset(setter)]
    fn set___dict__(zelf: &Py<Self>, value: PySetterValue, vm: &VirtualMachine) -> PyResult<()> {
        object::object_generic_set_dict(zelf.as_object().to_owned(), value, vm)
    }

    #[pygetset]
    fn __annotate__(&self, vm: &VirtualMachine) -> PyObjectRef {
        self.annotate
            .lock()
            .clone()
            .unwrap_or_else(|| vm.ctx.none())
    }

    #[pygetset(setter)]
    fn set___annotate__(
        &self,
        value: PySetterValue<Option<PyObjectRef>>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let annotate = match value {
            PySetterValue::Assign(Some(value)) => {
                if !value.is_callable() {
                    return Err(vm.new_type_error("__annotate__ must be callable or None"));
                }
                // Clear cached __annotations__ when __annotate__ is set
                *self.annotations.lock() = None;
                Some(value)
            }
            PySetterValue::Assign(None) => None,
            PySetterValue::Delete => {
                return Err(vm.new_type_error("__annotate__ cannot be deleted"));
            }
        };
        *self.annotate.lock() = annotate;
        Ok(())
    }

    #[pygetset]
    fn __qualname__(&self) -> PyStrRef {
        self.qualname.lock().clone()
    }

    #[pygetset(setter)]
    fn set___qualname__(&self, value: PySetterValue, vm: &VirtualMachine) -> PyResult<()> {
        match value {
            PySetterValue::Assign(value) => {
                let Ok(qualname) = value.downcast::<PyStr>() else {
                    return Err(vm.new_type_error("__qualname__ must be set to a string object"));
                };
                *self.qualname.lock() = qualname;
            }
            PySetterValue::Delete => {
                return Err(vm.new_type_error("__qualname__ must be set to a string object"));
            }
        }
        Ok(())
    }

    #[pygetset]
    fn __type_params__(&self) -> PyTupleRef {
        self.type_params.lock().clone()
    }

    #[pygetset(setter)]
    fn set___type_params__(
        &self,
        value: PySetterValue<PyTupleRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        match value {
            PySetterValue::Assign(value) => {
                *self.type_params.lock() = value;
            }
            PySetterValue::Delete => {
                return Err(vm.new_type_error("__type_params__ must be set to a tuple object"));
            }
        }
        Ok(())
    }

    #[cfg(feature = "jit")]
    #[pymethod]
    fn __jit__(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<()> {
        let mut jit_guard = zelf.jitted_code.lock();
        if jit_guard.is_some() {
            return Ok(());
        }
        let arg_types = jit::get_jit_arg_types(&zelf, vm)?;
        let ret_type = jit::jit_ret_type(&zelf, vm)?;
        let code: &Py<PyCode> = &zelf.code;
        let compiled = rustpython_jit::compile(&code.code, &arg_types, ret_type)
            .map_err(|err| jit::new_jit_error(err.to_string(), vm))?;
        *jit_guard = Some(compiled);
        Ok(())
    }
}

impl GetDescriptor for PyFunction {
    fn descr_get(
        zelf: PyObjectRef,
        obj: Option<PyObjectRef>,
        cls: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let (_zelf, obj) = Self::_unwrap(&zelf, obj, vm)?;
        Ok(if vm.is_none(&obj) && !Self::_cls_is(&cls, obj.class()) {
            zelf
        } else {
            PyBoundMethod::new(obj, zelf).into_ref(&vm.ctx).into()
        })
    }
}

impl Callable for PyFunction {
    type Args = FuncArgs;
    #[inline]
    fn call(zelf: &Py<Self>, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        zelf.invoke(args, vm)
    }
}

impl Representable for PyFunction {
    #[inline]
    fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
        Ok(format!(
            "<function {} at {:#x}>",
            zelf.__qualname__(),
            zelf.get_id()
        ))
    }
}

#[derive(FromArgs)]
pub struct PyFunctionNewArgs {
    #[pyarg(positional)]
    code: PyRef<PyCode>,
    #[pyarg(positional)]
    globals: PyDictRef,
    #[pyarg(any, optional, error_msg = "arg 3 (name) must be None or string")]
    name: OptionalArg<PyStrRef>,
    #[pyarg(any, optional, error_msg = "arg 4 (defaults) must be None or tuple")]
    argdefs: Option<PyTupleRef>,
    #[pyarg(any, optional, error_msg = "arg 5 (closure) must be None or tuple")]
    closure: Option<PyTupleRef>,
    #[pyarg(any, optional, error_msg = "arg 6 (kwdefaults) must be None or dict")]
    kwdefaults: Option<PyDictRef>,
}

impl Constructor for PyFunction {
    type Args = PyFunctionNewArgs;

    fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
        // Handle closure - must be a tuple of cells
        let closure = if let Some(closure_tuple) = args.closure {
            // Check that closure length matches code's free variables
            if closure_tuple.len() != args.code.freevars.len() {
                return Err(vm.new_value_error(format!(
                    "{} requires closure of length {}, not {}",
                    args.code.obj_name,
                    args.code.freevars.len(),
                    closure_tuple.len()
                )));
            }

            // Validate that all items are cells and create typed tuple
            let typed_closure = closure_tuple.try_into_typed::<PyCell>(vm)?;
            Some(typed_closure)
        } else if !args.code.freevars.is_empty() {
            return Err(vm.new_type_error("arg 5 (closure) must be tuple"));
        } else {
            None
        };

        let mut func = Self::new(args.code.clone(), args.globals.clone(), vm)?;
        // Set function name if provided
        if let Some(name) = args.name.into_option() {
            *func.name.lock() = name.clone();
            // Also update qualname to match the name
            *func.qualname.lock() = name;
        }
        // Now set additional attributes directly
        if let Some(closure_tuple) = closure {
            func.closure = Some(closure_tuple);
        }
        if let Some(argdefs) = args.argdefs {
            func.defaults_and_kwdefaults.lock().0 = Some(argdefs);
        }
        if let Some(kwdefaults) = args.kwdefaults {
            func.defaults_and_kwdefaults.lock().1 = Some(kwdefaults);
        }

        Ok(func)
    }
}

#[pyclass(module = false, name = "method", traverse)]
#[derive(Debug)]
pub struct PyBoundMethod {
    object: PyObjectRef,
    function: PyObjectRef,
}

impl Callable for PyBoundMethod {
    type Args = FuncArgs;
    #[inline]
    fn call(zelf: &Py<Self>, mut args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        args.prepend_arg(zelf.object.clone());
        zelf.function.call(args, vm)
    }
}

impl Comparable for PyBoundMethod {
    fn cmp(
        zelf: &Py<Self>,
        other: &PyObject,
        op: PyComparisonOp,
        _vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        op.eq_only(|| {
            let other = class_or_notimplemented!(Self, other);
            Ok(PyComparisonValue::Implemented(
                zelf.function.is(&other.function) && zelf.object.is(&other.object),
            ))
        })
    }
}

impl Hashable for PyBoundMethod {
    fn hash(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<PyHash> {
        let self_hash = crate::common::hash::hash_object_id_raw(zelf.object.get_id());
        let func_hash = zelf.function.hash(vm)?;
        Ok(crate::common::hash::fix_sentinel(self_hash ^ func_hash))
    }
}

impl GetAttr for PyBoundMethod {
    fn getattro(zelf: &Py<Self>, name: &Py<PyStr>, vm: &VirtualMachine) -> PyResult {
        let class_attr = vm
            .ctx
            .interned_str(name)
            .and_then(|attr_name| zelf.get_class_attr(attr_name));
        if let Some(obj) = class_attr {
            return vm.call_if_get_descriptor(&obj, zelf.to_owned().into());
        }
        zelf.function.get_attr(name, vm)
    }
}

impl GetDescriptor for PyBoundMethod {
    fn descr_get(
        zelf: PyObjectRef,
        _obj: Option<PyObjectRef>,
        _cls: Option<PyObjectRef>,
        _vm: &VirtualMachine,
    ) -> PyResult {
        Ok(zelf)
    }
}

#[derive(FromArgs)]
pub struct PyBoundMethodNewArgs {
    #[pyarg(positional)]
    function: PyObjectRef,
    #[pyarg(positional)]
    object: PyObjectRef,
}

impl Constructor for PyBoundMethod {
    type Args = PyBoundMethodNewArgs;

    fn py_new(
        _cls: &Py<PyType>,
        Self::Args { function, object }: Self::Args,
        vm: &VirtualMachine,
    ) -> PyResult<Self> {
        if !function.is_callable() {
            return Err(vm.new_type_error("first argument must be callable".to_owned()));
        }
        if vm.is_none(&object) {
            return Err(vm.new_type_error("instance must not be None".to_owned()));
        }
        Ok(Self::new(object, function))
    }
}

impl PyBoundMethod {
    pub const fn new(object: PyObjectRef, function: PyObjectRef) -> Self {
        Self { object, function }
    }

    #[inline]
    pub(crate) fn function_obj(&self) -> &PyObjectRef {
        &self.function
    }

    #[inline]
    pub(crate) fn self_obj(&self) -> &PyObjectRef {
        &self.object
    }

    #[deprecated(note = "Use `Self::new(object, function).into_ref(ctx)` instead")]
    pub fn new_ref(object: PyObjectRef, function: PyObjectRef, ctx: &Context) -> PyRef<Self> {
        Self::new(object, function).into_ref(ctx)
    }
}

#[pyclass(
    with(
        Callable,
        Comparable,
        Hashable,
        GetAttr,
        GetDescriptor,
        Constructor,
        Representable
    ),
    flags(IMMUTABLETYPE, HAS_WEAKREF)
)]
impl PyBoundMethod {
    #[pymethod]
    fn __reduce__(
        &self,
        vm: &VirtualMachine,
    ) -> PyResult<(PyObjectRef, (PyObjectRef, PyObjectRef))> {
        let builtins_getattr = vm.builtins.get_attr("getattr", vm)?;
        let func_self = self.object.clone();
        let func_name = self.function.get_attr("__name__", vm)?;
        Ok((builtins_getattr, (func_self, func_name)))
    }

    #[pygetset]
    fn __doc__(&self, vm: &VirtualMachine) -> PyResult {
        self.function.get_attr("__doc__", vm)
    }

    #[pygetset]
    fn __func__(&self) -> PyObjectRef {
        self.function.clone()
    }

    #[pygetset(name = "__self__")]
    fn get_self(&self) -> PyObjectRef {
        self.object.clone()
    }

    #[pygetset]
    fn __module__(&self, vm: &VirtualMachine) -> Option<PyObjectRef> {
        self.function.get_attr("__module__", vm).ok()
    }

    #[pymethod]
    fn __dir__(&self, vm: &VirtualMachine) -> PyResult<PyList> {
        let func_dir = vm.dir(Some(self.function.clone()))?;

        let bound_only = [
            "__self__",
            "__func__",
            "__doc__",
            "__module__",
            "__call__",
            "__get__",
            "__repr__",
        ];

        let mut seen = std::collections::HashSet::new();
        let mut result: Vec<PyObjectRef> = Vec::new();

        for item in func_dir.borrow_vec().iter() {
            if let Ok(s) = item.clone().downcast::<PyStr>() {
                seen.insert(s.as_wtf8().to_string());
            }
            result.push(item.clone());
        }

        for name in bound_only {
            if seen.insert(name.to_owned()) {
                result.push(vm.ctx.new_str(name).into());
            }
        }

        Ok(PyList::from(result))
    }
}

impl PyPayload for PyBoundMethod {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.bound_method_type
    }
}

impl Representable for PyBoundMethod {
    #[inline]
    fn repr_wtf8(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<Wtf8Buf> {
        let func_name = if let Some(qname) =
            vm.get_attribute_opt(zelf.function.clone(), identifier!(vm, __qualname__))?
        {
            Some(qname)
        } else {
            vm.get_attribute_opt(zelf.function.clone(), identifier!(vm, __name__))?
        };
        let func_name: Option<PyStrRef> = func_name.and_then(|o| o.downcast().ok());
        let object_repr = zelf.object.repr(vm)?;
        let name = func_name
            .as_ref()
            .map_or_else(|| "?".as_ref(), |s| s.as_wtf8());
        Ok(wtf8_concat!(
            "<bound method ",
            name,
            " of ",
            object_repr.as_wtf8(),
            ">"
        ))
    }
}

#[pyclass(module = false, name = "cell", traverse)]
#[derive(Debug, Default)]
pub(crate) struct PyCell {
    contents: PyMutex<Option<PyObjectRef>>,
}
pub(crate) type PyCellRef = PyRef<PyCell>;

impl PyPayload for PyCell {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.cell_type
    }
}

impl Constructor for PyCell {
    type Args = OptionalArg;

    fn py_new(_cls: &Py<PyType>, value: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
        Ok(Self::new(value.into_option()))
    }
}

#[pyclass(with(Constructor))]
impl PyCell {
    pub const fn new(contents: Option<PyObjectRef>) -> Self {
        Self {
            contents: PyMutex::new(contents),
        }
    }

    pub fn get(&self) -> Option<PyObjectRef> {
        self.contents.lock().clone()
    }
    pub fn set(&self, x: Option<PyObjectRef>) {
        *self.contents.lock() = x;
    }

    #[pygetset]
    fn cell_contents(&self, vm: &VirtualMachine) -> PyResult {
        self.get()
            .ok_or_else(|| vm.new_value_error("Cell is empty"))
    }
    #[pygetset(setter)]
    fn set_cell_contents(&self, x: PySetterValue) {
        match x {
            PySetterValue::Assign(value) => self.set(Some(value)),
            PySetterValue::Delete => self.set(None),
        }
    }
}

/// Vectorcall implementation for PyFunction (PEP 590).
/// Takes owned args to avoid cloning when filling fastlocals.
pub(crate) fn vectorcall_function(
    zelf_obj: &PyObject,
    mut args: Vec<PyObjectRef>,
    nargs: usize,
    kwnames: Option<&[PyObjectRef]>,
    vm: &VirtualMachine,
) -> PyResult {
    let zelf: &Py<PyFunction> = zelf_obj.downcast_ref().unwrap();
    let code: &Py<PyCode> = &zelf.code;

    let has_kwargs = kwnames.is_some_and(|kw| !kw.is_empty());
    let is_simple = !has_kwargs
        && code.flags.contains(bytecode::CodeFlags::OPTIMIZED)
        && !code.flags.contains(bytecode::CodeFlags::VARARGS)
        && !code.flags.contains(bytecode::CodeFlags::VARKEYWORDS)
        && code.kwonlyarg_count == 0
        && !code
            .flags
            .intersects(bytecode::CodeFlags::GENERATOR | bytecode::CodeFlags::COROUTINE);

    if is_simple && nargs == code.arg_count as usize {
        // FAST PATH: simple positional-only call, exact arg count.
        // Move owned args directly into fastlocals — no clone needed.
        args.truncate(nargs);
        let frame = zelf.prepare_exact_args_frame(args, vm);

        let result = vm.run_frame(frame.clone());
        unsafe {
            if let Some(base) = frame.materialize_localsplus() {
                vm.datastack_pop(base);
            }
        }
        return result;
    }

    // SLOW PATH: construct FuncArgs from owned Vec and delegate to invoke()
    let func_args = if has_kwargs {
        FuncArgs::from_vectorcall(&args, nargs, kwnames)
    } else {
        args.truncate(nargs);
        FuncArgs::from(args)
    };
    zelf.invoke(func_args, vm)
}

/// Vectorcall implementation for PyBoundMethod (PEP 590).
fn vectorcall_bound_method(
    zelf_obj: &PyObject,
    mut args: Vec<PyObjectRef>,
    nargs: usize,
    kwnames: Option<&[PyObjectRef]>,
    vm: &VirtualMachine,
) -> PyResult {
    let zelf: &Py<PyBoundMethod> = zelf_obj.downcast_ref().unwrap();

    // Insert self at front of existing Vec (avoids 2nd allocation).
    // O(n) memmove is cheaper than a 2nd heap alloc+dealloc for typical arg counts.
    args.insert(0, zelf.object.clone());
    let new_nargs = nargs + 1;
    zelf.function.vectorcall(args, new_nargs, kwnames, vm)
}

pub fn init(context: &'static Context) {
    PyFunction::extend_class(context, context.types.function_type);
    context
        .types
        .function_type
        .slots
        .vectorcall
        .store(Some(vectorcall_function));

    PyBoundMethod::extend_class(context, context.types.bound_method_type);
    context
        .types
        .bound_method_type
        .slots
        .vectorcall
        .store(Some(vectorcall_bound_method));

    PyCell::extend_class(context, context.types.cell_type);
}
