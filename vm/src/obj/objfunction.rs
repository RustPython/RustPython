#[cfg(feature = "jit")]
mod jitfunc;

use super::objcode::PyCodeRef;
use super::objdict::PyDictRef;
use super::objstr::PyStringRef;
use super::objtuple::PyTupleRef;
use super::objtype::PyClassRef;
use crate::bytecode;
use crate::frame::Frame;
use crate::function::{OptionalArg, PyFuncArgs};
use crate::obj::objasyncgenerator::PyAsyncGen;
use crate::obj::objcoroutine::PyCoroutine;
use crate::obj::objgenerator::PyGenerator;
#[cfg(feature = "jit")]
use crate::pyobject::IntoPyObject;
use crate::pyobject::{
    BorrowValue, IdProtocol, ItemProtocol, PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult,
    PyValue, TypeProtocol,
};
use crate::scope::Scope;
use crate::slots::{SlotCall, SlotDescriptor};
use crate::VirtualMachine;
use itertools::Itertools;
#[cfg(feature = "jit")]
use rustpython_common::cell::OnceCell;
#[cfg(feature = "jit")]
use rustpython_jit::CompiledCode;

pub type PyFunctionRef = PyRef<PyFunction>;

#[pyclass(module = false, name = "function")]
#[derive(Debug)]
pub struct PyFunction {
    code: PyCodeRef,
    #[cfg(feature = "jit")]
    jitted_code: OnceCell<CompiledCode>,
    scope: Scope,
    defaults: Option<PyTupleRef>,
    kw_only_defaults: Option<PyDictRef>,
}

impl SlotDescriptor for PyFunction {
    fn descr_get(
        vm: &VirtualMachine,
        zelf: PyObjectRef,
        obj: Option<PyObjectRef>,
        cls: OptionalArg<PyObjectRef>,
    ) -> PyResult {
        let (zelf, obj) = Self::_unwrap(zelf, obj, vm)?;
        if obj.is(&vm.get_none()) && !Self::_cls_is(&cls, &obj.class()) {
            Ok(zelf.into_object())
        } else {
            Ok(vm.ctx.new_bound_method(zelf.into_object(), obj))
        }
    }
}

impl PyFunction {
    pub fn new(
        code: PyCodeRef,
        scope: Scope,
        defaults: Option<PyTupleRef>,
        kw_only_defaults: Option<PyDictRef>,
    ) -> Self {
        PyFunction {
            code,
            #[cfg(feature = "jit")]
            jitted_code: OnceCell::new(),
            scope,
            defaults,
            kw_only_defaults,
        }
    }

    pub fn scope(&self) -> &Scope {
        &self.scope
    }

    fn fill_locals_from_args(
        &self,
        code_object: &bytecode::CodeObject,
        locals: &PyDictRef,
        func_args: PyFuncArgs,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let nargs = func_args.args.len();
        let nexpected_args = code_object.arg_names.len();
        let posonly_args = &code_object.arg_names[..code_object.posonlyarg_count];

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
            locals.set_item(arg_name.as_str(), arg.clone(), vm)?;
        }

        // Pack other positional arguments in to *args:
        if let Some(ref vararg_name) = code_object.varargs_name {
            let mut last_args = vec![];
            for i in n..nargs {
                let arg = &func_args.args[i];
                last_args.push(arg.clone());
            }
            let vararg_value = vm.ctx.new_tuple(last_args);

            locals.set_item(vararg_name.as_str(), vararg_value, vm)?;
        } else {
            // Check the number of positional arguments
            if nargs > nexpected_args {
                return Err(vm.new_type_error(format!(
                    "Expected {} arguments (got: {})",
                    nexpected_args, nargs
                )));
            }
        }

        // Do we support `**kwargs` ?
        let kwargs = if code_object
            .flags
            .contains(bytecode::CodeFlags::HAS_VARKEYWORDS)
        {
            let d = vm.ctx.new_dict();
            if let Some(ref kwargs_name) = code_object.varkeywords_name {
                locals.set_item(kwargs_name.as_str(), d.as_object().clone(), vm)?;
            }
            Some(d)
        } else {
            None
        };

        let mut posonly_passed_as_kwarg = Vec::new();
        // Handle keyword arguments
        for (name, value) in func_args.kwargs {
            // Check if we have a parameter with this name:
            let dict = if code_object.arg_names.contains(&name)
                || code_object.kwonlyarg_names.contains(&name)
            {
                if posonly_args.contains(&name) {
                    posonly_passed_as_kwarg.push(name);
                    continue;
                } else if locals.contains_key(&name, vm) {
                    return Err(
                        vm.new_type_error(format!("Got multiple values for argument '{}'", name))
                    );
                }
                locals
            } else {
                kwargs.as_ref().ok_or_else(|| {
                    vm.new_type_error(format!("Got an unexpected keyword argument '{}'", name))
                })?
            };
            dict.set_item(name.as_str(), value, vm)?;
        }
        if !posonly_passed_as_kwarg.is_empty() {
            return Err(vm.new_type_error(format!(
                "{}() got some positional-only arguments passed as keyword arguments: '{}'",
                &code_object.obj_name,
                posonly_passed_as_kwarg.into_iter().format(", "),
            )));
        }

        // Add missing positional arguments, if we have fewer positional arguments than the
        // function definition calls for
        if nargs < nexpected_args {
            let num_defaults_available =
                self.defaults.as_ref().map_or(0, |d| d.borrow_value().len());

            // Given the number of defaults available, check all the arguments for which we
            // _don't_ have defaults; if any are missing, raise an exception
            let required_args = nexpected_args - num_defaults_available;
            let mut missing = vec![];
            for i in 0..required_args {
                let variable_name = &code_object.arg_names[i];
                if !locals.contains_key(variable_name, vm) {
                    missing.push(variable_name)
                }
            }
            if !missing.is_empty() {
                return Err(vm.new_type_error(format!(
                    "Missing {} required positional arguments: {:?}",
                    missing.len(),
                    missing
                )));
            }
            if let Some(defaults) = &self.defaults {
                let defaults = defaults.borrow_value();
                // We have sufficient defaults, so iterate over the corresponding names and use
                // the default if we don't already have a value
                for (default_index, i) in (required_args..nexpected_args).enumerate() {
                    let arg_name = &code_object.arg_names[i];
                    if !locals.contains_key(arg_name, vm) {
                        locals.set_item(arg_name.as_str(), defaults[default_index].clone(), vm)?;
                    }
                }
            }
        };

        // Check if kw only arguments are all present:
        for arg_name in &code_object.kwonlyarg_names {
            if !locals.contains_key(arg_name, vm) {
                if let Some(kw_only_defaults) = &self.kw_only_defaults {
                    if let Some(default) =
                        kw_only_defaults.get_item_option(arg_name.as_str(), vm)?
                    {
                        locals.set_item(arg_name.as_str(), default, vm)?;
                        continue;
                    }
                }

                // No default value and not specified.
                return Err(
                    vm.new_type_error(format!("Missing required kw only argument: '{}'", arg_name))
                );
            }
        }

        Ok(())
    }

    pub fn invoke_with_scope(
        &self,
        func_args: PyFuncArgs,
        scope: &Scope,
        vm: &VirtualMachine,
    ) -> PyResult {
        #[cfg(feature = "jit")]
        if let Some(jitted_code) = self.jitted_code.get() {
            match jitfunc::get_jit_args(self, &func_args, jitted_code, vm) {
                Ok(args) => {
                    return Ok(jitted_code.invoke(&args).into_pyobject(vm));
                }
                Err(err) => info!(
                    "jit: function `{}` is falling back to being interpreted because of the \
                    error: {}",
                    self.code.obj_name, err
                ),
            }
        }

        let code = &self.code;

        let scope = if self.code.flags.contains(bytecode::CodeFlags::NEW_LOCALS) {
            scope.new_child_scope(&vm.ctx)
        } else {
            scope.clone()
        };

        self.fill_locals_from_args(&code, &scope.get_locals(), func_args, vm)?;

        // Construct frame:
        let frame = Frame::new(code.clone(), scope, vm).into_ref(vm);

        // If we have a generator, create a new generator
        let is_gen = code.flags.contains(bytecode::CodeFlags::IS_GENERATOR);
        let is_coro = code.flags.contains(bytecode::CodeFlags::IS_COROUTINE);
        match (is_gen, is_coro) {
            (true, false) => Ok(PyGenerator::new(frame, vm).into_object()),
            (false, true) => Ok(PyCoroutine::new(frame, vm).into_object()),
            (true, true) => Ok(PyAsyncGen::new(frame, vm).into_object()),
            (false, false) => vm.run_frame_full(frame),
        }
    }

    pub fn invoke(&self, func_args: PyFuncArgs, vm: &VirtualMachine) -> PyResult {
        self.invoke_with_scope(func_args, &self.scope, vm)
    }
}

impl PyValue for PyFunction {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.types.function_type.clone()
    }
}

#[pyimpl(with(SlotDescriptor), flags(HAS_DICT))]
impl PyFunction {
    #[pyslot]
    #[pymethod(magic)]
    fn call(&self, args: PyFuncArgs, vm: &VirtualMachine) -> PyResult {
        self.invoke(args, vm)
    }

    #[pyproperty(magic)]
    fn code(&self) -> PyCodeRef {
        self.code.clone()
    }

    #[pyproperty(magic)]
    fn defaults(&self) -> Option<PyTupleRef> {
        self.defaults.clone()
    }

    #[pyproperty(magic)]
    fn kwdefaults(&self) -> Option<PyDictRef> {
        self.kw_only_defaults.clone()
    }

    #[pyproperty(magic)]
    fn globals(&self) -> PyDictRef {
        self.scope.globals.clone()
    }

    #[cfg(feature = "jit")]
    #[pymethod(magic)]
    fn jit(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult<()> {
        zelf.jitted_code
            .get_or_try_init(|| {
                let arg_types = jitfunc::get_jit_arg_types(&zelf, vm)?;
                rustpython_jit::compile(&zelf.code.code, &arg_types)
                    .map_err(|err| jitfunc::new_jit_error(err.to_string(), vm))
            })
            .map(drop)
    }
}

#[pyclass(module = false, name = "method")]
#[derive(Debug)]
pub struct PyBoundMethod {
    // TODO: these shouldn't be public
    object: PyObjectRef,
    pub function: PyObjectRef,
}

impl SlotCall for PyBoundMethod {
    fn call(&self, args: PyFuncArgs, vm: &VirtualMachine) -> PyResult {
        let args = args.insert(self.object.clone());
        vm.invoke(&self.function, args)
    }
}

impl PyBoundMethod {
    pub fn new(object: PyObjectRef, function: PyObjectRef) -> Self {
        PyBoundMethod { object, function }
    }
}

#[pyimpl(with(SlotCall), flags(HAS_DICT))]
impl PyBoundMethod {
    #[pymethod(magic)]
    fn repr(&self, vm: &VirtualMachine) -> PyResult<String> {
        Ok(format!(
            "<bound method of {}>",
            vm.to_repr(&self.object)?.borrow_value()
        ))
    }

    #[pyproperty(magic)]
    fn doc(&self, vm: &VirtualMachine) -> PyResult {
        vm.get_attribute(self.function.clone(), "__doc__")
    }

    #[pymethod(magic)]
    fn getattribute(zelf: PyRef<Self>, name: PyStringRef, vm: &VirtualMachine) -> PyResult {
        if let Some(obj) = zelf.get_class_attr(name.borrow_value()) {
            return vm.call_if_get_descriptor(obj, zelf.into_object());
        }
        vm.get_attribute(zelf.function.clone(), name)
    }

    #[pyproperty(magic)]
    fn func(&self) -> PyObjectRef {
        self.function.clone()
    }

    #[pyproperty(magic)]
    fn module(&self, vm: &VirtualMachine) -> Option<PyObjectRef> {
        vm.get_attribute(self.function.clone(), "__module__").ok()
    }
}

impl PyValue for PyBoundMethod {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.types.bound_method_type.clone()
    }
}

pub fn init(context: &PyContext) {
    let function_type = &context.types.function_type;
    PyFunction::extend_class(context, function_type);

    let method_type = &context.types.bound_method_type;
    PyBoundMethod::extend_class(context, method_type);
}
