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
use crate::pyobject::{
    IdProtocol, ItemProtocol, PyClassImpl, PyContext, PyObjectRef, PyRef, PyResult, PyValue,
    TypeProtocol,
};
use crate::scope::Scope;
use crate::slots::{SlotCall, SlotDescriptor};
use crate::vm::VirtualMachine;

pub type PyFunctionRef = PyRef<PyFunction>;

#[pyclass]
#[derive(Debug)]
pub struct PyFunction {
    code: PyCodeRef,
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
            locals.set_item(arg_name, arg.clone(), vm)?;
        }

        // Pack other positional arguments in to *args:
        match code_object.varargs {
            bytecode::Varargs::Named(ref vararg_name) => {
                let mut last_args = vec![];
                for i in n..nargs {
                    let arg = &func_args.args[i];
                    last_args.push(arg.clone());
                }
                let vararg_value = vm.ctx.new_tuple(last_args);

                locals.set_item(vararg_name, vararg_value, vm)?;
            }
            bytecode::Varargs::Unnamed | bytecode::Varargs::None => {
                // Check the number of positional arguments
                if nargs > nexpected_args {
                    return Err(vm.new_type_error(format!(
                        "Expected {} arguments (got: {})",
                        nexpected_args, nargs
                    )));
                }
            }
        }

        // Do we support `**kwargs` ?
        let kwargs = match code_object.varkeywords {
            bytecode::Varargs::Named(ref kwargs_name) => {
                let d = vm.ctx.new_dict();
                locals.set_item(kwargs_name, d.as_object().clone(), vm)?;
                Some(d)
            }
            bytecode::Varargs::Unnamed => Some(vm.ctx.new_dict()),
            bytecode::Varargs::None => None,
        };

        // Handle keyword arguments
        for (name, value) in func_args.kwargs {
            // Check if we have a parameter with this name:
            if code_object.arg_names.contains(&name) || code_object.kwonlyarg_names.contains(&name)
            {
                if locals.contains_key(&name, vm) {
                    return Err(
                        vm.new_type_error(format!("Got multiple values for argument '{}'", name))
                    );
                }

                locals.set_item(&name, value, vm)?;
            } else if let Some(d) = &kwargs {
                d.set_item(&name, value, vm)?;
            } else {
                return Err(
                    vm.new_type_error(format!("Got an unexpected keyword argument '{}'", name))
                );
            }
        }

        // Add missing positional arguments, if we have fewer positional arguments than the
        // function definition calls for
        if nargs < nexpected_args {
            let num_defaults_available = self.defaults.as_ref().map_or(0, |d| d.as_slice().len());

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
                let defaults = defaults.as_slice();
                // We have sufficient defaults, so iterate over the corresponding names and use
                // the default if we don't already have a value
                for (default_index, i) in (required_args..nexpected_args).enumerate() {
                    let arg_name = &code_object.arg_names[i];
                    if !locals.contains_key(arg_name, vm) {
                        locals.set_item(arg_name, defaults[default_index].clone(), vm)?;
                    }
                }
            }
        };

        // Check if kw only arguments are all present:
        for arg_name in &code_object.kwonlyarg_names {
            if !locals.contains_key(arg_name, vm) {
                if let Some(kw_only_defaults) = &self.kw_only_defaults {
                    if let Some(default) = kw_only_defaults.get_item_option(arg_name, vm)? {
                        locals.set_item(arg_name, default, vm)?;
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
        let code = &self.code;

        let scope = if self.code.flags.contains(bytecode::CodeFlags::NEW_LOCALS) {
            scope.new_child_scope(&vm.ctx)
        } else {
            scope.clone()
        };

        self.fill_locals_from_args(&code, &scope.get_locals(), func_args, vm)?;

        // Construct frame:
        let frame = Frame::new(code.clone(), scope).into_ref(vm);

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
        vm.ctx.function_type()
    }
}

#[pyimpl(with(SlotDescriptor))]
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
}

#[pyclass]
#[derive(Debug)]
pub struct PyBoundMethod {
    // TODO: these shouldn't be public
    pub object: PyObjectRef,
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

#[pyimpl(with(SlotCall))]
impl PyBoundMethod {
    #[pymethod(magic)]
    fn getattribute(&self, name: PyStringRef, vm: &VirtualMachine) -> PyResult {
        vm.get_attribute(self.function.clone(), name)
    }
}

impl PyValue for PyBoundMethod {
    fn class(vm: &VirtualMachine) -> PyClassRef {
        vm.ctx.bound_method_type()
    }
}

pub fn init(context: &PyContext) {
    let function_type = &context.types.function_type;
    PyFunction::extend_class(context, function_type);

    let method_type = &context.types.bound_method_type;
    PyBoundMethod::extend_class(context, method_type);
}
