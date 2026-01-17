#[cfg(feature = "jit")]
mod jit;

use super::{
    PyAsyncGen, PyCode, PyCoroutine, PyDictRef, PyGenerator, PyModule, PyStr, PyStrRef, PyTuple,
    PyTupleRef, PyType,
};
#[cfg(feature = "jit")]
use crate::common::lock::OnceCell;
use crate::common::lock::PyMutex;
use crate::function::ArgMapping;
use crate::object::{Traverse, TraverseFn};
use crate::{
    AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    bytecode,
    class::PyClassImpl,
    frame::Frame,
    function::{FuncArgs, OptionalArg, PyComparisonValue, PySetterValue},
    scope::Scope,
    types::{
        Callable, Comparable, Constructor, GetAttr, GetDescriptor, PyComparisonOp, Representable,
    },
};
use itertools::Itertools;
#[cfg(feature = "jit")]
use rustpython_jit::CompiledCode;

#[pyclass(module = false, name = "function", traverse = "manual")]
#[derive(Debug)]
pub struct PyFunction {
    code: PyMutex<PyRef<PyCode>>,
    globals: PyDictRef,
    builtins: PyObjectRef,
    closure: Option<PyRef<PyTuple<PyCellRef>>>,
    defaults_and_kwdefaults: PyMutex<(Option<PyTupleRef>, Option<PyDictRef>)>,
    name: PyMutex<PyStrRef>,
    qualname: PyMutex<PyStrRef>,
    type_params: PyMutex<PyTupleRef>,
    annotations: PyMutex<Option<PyDictRef>>,
    annotate: PyMutex<Option<PyObjectRef>>,
    module: PyMutex<PyObjectRef>,
    doc: PyMutex<PyObjectRef>,
    #[cfg(feature = "jit")]
    jitted_code: OnceCell<CompiledCode>,
}

unsafe impl Traverse for PyFunction {
    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        self.globals.traverse(tracer_fn);
        if let Some(closure) = self.closure.as_ref() {
            closure.as_untyped().traverse(tracer_fn);
        }
        self.defaults_and_kwdefaults.traverse(tracer_fn);
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
                frame.builtins.clone().into()
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
            code: PyMutex::new(code.clone()),
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
            #[cfg(feature = "jit")]
            jitted_code: OnceCell::new(),
        };
        Ok(func)
    }

    fn fill_locals_from_args(
        &self,
        frame: &Frame,
        func_args: FuncArgs,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let code = &*self.code.lock();
        let nargs = func_args.args.len();
        let n_expected_args = code.arg_count as usize;
        let total_args = code.arg_count as usize + code.kwonlyarg_count as usize;
        // let arg_names = self.code.arg_names();

        // This parses the arguments from args and kwargs into
        // the proper variables keeping into account default values
        // and star-args and kwargs.
        // See also: PyEval_EvalCodeWithName in cpython:
        // https://github.com/python/cpython/blob/main/Python/ceval.c#L3681

        let mut fastlocals = frame.fastlocals.lock();

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
                return Err(vm.new_type_error(format!(
                    "{}() takes {} positional argument{} but {} {} given",
                    self.__qualname__(),
                    takes_msg,
                    if n_expected_args == 1 { "" } else { "s" },
                    nargs,
                    if nargs == 1 { "was" } else { "were" }
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
            let missing_args_len = missing.len();

            if !missing.is_empty() {
                let last = if missing.len() > 1 {
                    missing.pop()
                } else {
                    None
                };

                let (and, right) = if let Some(last) = last {
                    (
                        if missing.len() == 1 {
                            "' and '"
                        } else {
                            "', and '"
                        },
                        last.as_str(),
                    )
                } else {
                    ("", "")
                };

                return Err(vm.new_type_error(format!(
                    "{}() missing {} required positional argument{}: '{}{}{}'",
                    self.__qualname__(),
                    missing_args_len,
                    if missing_args_len == 1 { "" } else { "s" },
                    missing.iter().join("', '"),
                    and,
                    right,
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
            // TODO: compile a list of missing arguments
            // let mut missing = vec![];
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
                return Err(
                    vm.new_type_error(format!("Missing required kw only argument: '{kwarg}'"))
                );
            }
        }

        if let Some(cell2arg) = code.cell2arg.as_deref() {
            for (cell_idx, arg_idx) in cell2arg.iter().enumerate().filter(|(_, i)| **i != -1) {
                let x = fastlocals[*arg_idx as usize].take();
                frame.cells_frees[cell_idx].set(x);
            }
        }

        Ok(())
    }

    /// Set function attribute based on MakeFunctionFlags
    pub(crate) fn set_function_attribute(
        &mut self,
        attr: bytecode::MakeFunctionFlags,
        attr_value: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        use crate::builtins::PyDict;
        if attr == bytecode::MakeFunctionFlags::DEFAULTS {
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
        } else if attr == bytecode::MakeFunctionFlags::KW_ONLY_DEFAULTS {
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
        } else if attr == bytecode::MakeFunctionFlags::ANNOTATIONS {
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
        } else if attr == bytecode::MakeFunctionFlags::CLOSURE {
            // For closure, we need special handling
            // The closure tuple contains cell objects
            let closure_tuple = attr_value
                .clone()
                .downcast_exact::<PyTuple>(vm)
                .map_err(|obj| {
                    vm.new_type_error(format!(
                        "closure must be a tuple, not {}",
                        obj.class().name()
                    ))
                })?
                .into_pyref();

            self.closure = Some(closure_tuple.try_into_typed::<PyCell>(vm)?);
        } else if attr == bytecode::MakeFunctionFlags::TYPE_PARAMS {
            let type_params = attr_value.clone().downcast::<PyTuple>().map_err(|_| {
                vm.new_type_error(format!(
                    "__type_params__ must be a tuple, not {}",
                    attr_value.class().name()
                ))
            })?;
            *self.type_params.lock() = type_params;
        } else if attr == bytecode::MakeFunctionFlags::ANNOTATE {
            // PEP 649: Store the __annotate__ function closure
            if !attr_value.is_callable() {
                return Err(vm.new_type_error("__annotate__ must be callable".to_owned()));
            }
            *self.annotate.lock() = Some(attr_value);
        } else {
            unreachable!("This is a compiler bug");
        }
        Ok(())
    }
}

impl Py<PyFunction> {
    pub fn invoke_with_locals(
        &self,
        func_args: FuncArgs,
        locals: Option<ArgMapping>,
        vm: &VirtualMachine,
    ) -> PyResult {
        #[cfg(feature = "jit")]
        if let Some(jitted_code) = self.jitted_code.get() {
            use crate::convert::ToPyObject;
            match jit::get_jit_args(self, &func_args, jitted_code, vm) {
                Ok(args) => {
                    return Ok(args.invoke().to_pyobject(vm));
                }
                Err(err) => info!(
                    "jit: function `{}` is falling back to being interpreted because of the \
                    error: {}",
                    self.code.lock().obj_name,
                    err
                ),
            }
        }

        let code = self.code.lock().clone();

        let locals = if code.flags.contains(bytecode::CodeFlags::NEWLOCALS) {
            ArgMapping::from_dict_exact(vm.ctx.new_dict())
        } else if let Some(locals) = locals {
            locals
        } else {
            ArgMapping::from_dict_exact(self.globals.clone())
        };

        // Construct frame:
        let frame = Frame::new(
            code.clone(),
            Scope::new(Some(locals), self.globals.clone()),
            vm.builtins.dict(),
            self.closure.as_ref().map_or(&[], |c| c.as_slice()),
            Some(self.to_owned().into()),
            vm,
        )
        .into_ref(&vm.ctx);

        self.fill_locals_from_args(&frame, func_args, vm)?;

        // If we have a generator, create a new generator
        let is_gen = code.flags.contains(bytecode::CodeFlags::GENERATOR);
        let is_coro = code.flags.contains(bytecode::CodeFlags::COROUTINE);
        match (is_gen, is_coro) {
            (true, false) => {
                Ok(PyGenerator::new(frame, self.__name__(), self.__qualname__()).into_pyobject(vm))
            }
            (false, true) => {
                Ok(PyCoroutine::new(frame, self.__name__(), self.__qualname__()).into_pyobject(vm))
            }
            (true, true) => {
                Ok(PyAsyncGen::new(frame, self.__name__(), self.__qualname__()).into_pyobject(vm))
            }
            (false, false) => vm.run_frame(frame),
        }
    }

    #[inline(always)]
    pub fn invoke(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        self.invoke_with_locals(func_args, None, vm)
    }
}

impl PyPayload for PyFunction {
    #[inline]
    fn class(ctx: &Context) -> &'static Py<PyType> {
        ctx.types.function_type
    }
}

#[pyclass(
    with(GetDescriptor, Callable, Representable, Constructor),
    flags(HAS_DICT, METHOD_DESCRIPTOR)
)]
impl PyFunction {
    #[pygetset]
    fn __code__(&self) -> PyRef<PyCode> {
        self.code.lock().clone()
    }

    #[pygetset(setter)]
    fn set___code__(&self, code: PyRef<PyCode>) {
        *self.code.lock() = code;
        // TODO: jit support
        // #[cfg(feature = "jit")]
        // {
        //     // If available, clear cached compiled code.
        //     let _ = self.jitted_code.take();
        // }
    }

    #[pygetset]
    fn __defaults__(&self) -> Option<PyTupleRef> {
        self.defaults_and_kwdefaults.lock().0.clone()
    }
    #[pygetset(setter)]
    fn set___defaults__(&self, defaults: Option<PyTupleRef>) {
        self.defaults_and_kwdefaults.lock().0 = defaults
    }

    #[pygetset]
    fn __kwdefaults__(&self) -> Option<PyDictRef> {
        self.defaults_and_kwdefaults.lock().1.clone()
    }
    #[pygetset(setter)]
    fn set___kwdefaults__(&self, kwdefaults: Option<PyDictRef>) {
        self.defaults_and_kwdefaults.lock().1 = kwdefaults
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
        let annotations = match value {
            PySetterValue::Assign(Some(value)) => {
                let annotations = value.downcast::<crate::builtins::PyDict>().map_err(|_| {
                    vm.new_type_error("__annotations__ must be set to a dict object")
                })?;
                Some(annotations)
            }
            PySetterValue::Assign(None) | PySetterValue::Delete => None,
        };
        *self.annotations.lock() = annotations;

        // Clear __annotate__ when __annotations__ is set
        *self.annotate.lock() = None;
        Ok(())
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
        zelf.jitted_code
            .get_or_try_init(|| {
                let arg_types = jit::get_jit_arg_types(&zelf, vm)?;
                let ret_type = jit::jit_ret_type(&zelf, vm)?;
                let code = zelf.code.lock();
                rustpython_jit::compile(&code.code, &arg_types, ret_type)
                    .map_err(|err| jit::new_jit_error(err.to_string(), vm))
            })
            .map(drop)
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
    #[pyarg(any, optional)]
    name: OptionalArg<PyStrRef>,
    #[pyarg(any, optional)]
    argdefs: Option<PyTupleRef>,
    #[pyarg(any, optional)]
    closure: Option<PyTupleRef>,
    #[pyarg(any, optional)]
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
        _vm: &VirtualMachine,
    ) -> PyResult<Self> {
        Ok(Self::new(object, function))
    }
}

impl PyBoundMethod {
    pub const fn new(object: PyObjectRef, function: PyObjectRef) -> Self {
        Self { object, function }
    }

    #[deprecated(note = "Use `Self::new(object, function).into_ref(ctx)` instead")]
    pub fn new_ref(object: PyObjectRef, function: PyObjectRef, ctx: &Context) -> PyRef<Self> {
        Self::new(object, function).into_ref(ctx)
    }
}

#[pyclass(
    with(Callable, Comparable, GetAttr, Constructor, Representable),
    flags(IMMUTABLETYPE)
)]
impl PyBoundMethod {
    #[pymethod]
    fn __reduce__(
        &self,
        vm: &VirtualMachine,
    ) -> (Option<PyObjectRef>, (PyObjectRef, Option<PyObjectRef>)) {
        let builtins_getattr = vm.builtins.get_attr("getattr", vm).ok();
        let func_self = self.object.clone();
        let func_name = self.function.get_attr("__name__", vm).ok();
        (builtins_getattr, (func_self, func_name))
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

    #[pygetset]
    fn __qualname__(&self, vm: &VirtualMachine) -> PyResult {
        if self
            .function
            .fast_isinstance(vm.ctx.types.builtin_function_or_method_type)
        {
            // Special case: we work with `__new__`, which is not really a method.
            // It is a function, so its `__qualname__` is just `__new__`.
            // We need to add object's part manually.
            let obj_name = vm.get_attribute_opt(self.object.clone(), "__qualname__")?;
            let obj_name: Option<PyStrRef> = obj_name.and_then(|o| o.downcast().ok());
            return Ok(vm
                .ctx
                .new_str(format!(
                    "{}.__new__",
                    obj_name.as_ref().map_or("?", |s| s.as_str())
                ))
                .into());
        }
        self.function.get_attr("__qualname__", vm)
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
    fn repr_str(zelf: &Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
        #[allow(clippy::needless_match)] // False positive on nightly
        let func_name =
            if let Some(qname) = vm.get_attribute_opt(zelf.function.clone(), "__qualname__")? {
                Some(qname)
            } else {
                vm.get_attribute_opt(zelf.function.clone(), "__name__")?
            };
        let func_name: Option<PyStrRef> = func_name.and_then(|o| o.downcast().ok());
        let formatted_func_name = match func_name {
            Some(name) => name.to_string(),
            None => "?".to_string(),
        };
        let object_repr = zelf.object.repr(vm)?;
        Ok(format!(
            "<bound method {formatted_func_name} of {object_repr}>",
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

pub fn init(context: &Context) {
    PyFunction::extend_class(context, context.types.function_type);
    PyBoundMethod::extend_class(context, context.types.bound_method_type);
    PyCell::extend_class(context, context.types.cell_type);
}
