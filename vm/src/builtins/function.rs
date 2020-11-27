#[cfg(feature = "jit")]
mod jitfunc;

use super::code::PyCodeRef;
use super::dict::PyDictRef;
use super::pystr::PyStrRef;
use super::pytype::PyTypeRef;
use super::tuple::PyTupleRef;
use crate::builtins::asyncgenerator::PyAsyncGen;
use crate::builtins::coroutine::PyCoroutine;
use crate::builtins::generator::PyGenerator;
use crate::bytecode;
use crate::common::lock::PyMutex;
use crate::frame::Frame;
use crate::function::{FuncArgs, OptionalArg};
#[cfg(feature = "jit")]
use crate::pyobject::IntoPyObject;
use crate::pyobject::{
    BorrowValue, IdProtocol, ItemProtocol, PyClassImpl, PyComparisonValue, PyContext, PyObjectRef,
    PyRef, PyResult, PyValue, TypeProtocol,
};
use crate::scope::Scope;
use crate::slots::{Callable, Comparable, PyComparisonOp, SlotDescriptor, SlotGetattro};
use crate::VirtualMachine;
use itertools::Itertools;
#[cfg(feature = "jit")]
use rustpython_common::lock::OnceCell;
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
        locals: &PyDictRef,
        func_args: FuncArgs,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let nargs = func_args.args.len();
        let nexpected_args = self.code.arg_count;
        let arg_names = self.code.arg_names();

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

        let mut args_iter = func_args.args.into_iter();

        // Copy positional arguments into local variables
        for (arg, arg_name) in args_iter.by_ref().take(n).zip(arg_names.args) {
            locals.set_item(arg_name.clone(), arg, vm)?;
        }

        // Pack other positional arguments in to *args:
        if let Some(vararg_name) = arg_names.vararg {
            let vararg_value = vm.ctx.new_tuple(args_iter.collect());
            locals.set_item(vararg_name.clone(), vararg_value, vm)?;
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
        let kwargs = if let Some(varkwarg) = arg_names.varkwarg {
            let d = vm.ctx.new_dict();
            locals.set_item(varkwarg.clone(), d.as_object().clone(), vm)?;
            Some(d)
        } else {
            None
        };

        let contains_arg =
            |names: &[PyStrRef], name: &str| names.iter().any(|s| s.borrow_value() == name);

        let mut posonly_passed_as_kwarg = Vec::new();
        // Handle keyword arguments
        for (name, value) in func_args.kwargs {
            // Check if we have a parameter with this name:
            let dict = if contains_arg(arg_names.args, &name)
                || contains_arg(arg_names.kwonlyargs, &name)
            {
                if contains_arg(arg_names.posonlyargs, &name) {
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
            dict.set_item(vm.intern_string(name).into_object(), value, vm)?;
        }
        if !posonly_passed_as_kwarg.is_empty() {
            return Err(vm.new_type_error(format!(
                "{}() got some positional-only arguments passed as keyword arguments: '{}'",
                &self.code.obj_name,
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
                let variable_name = &arg_names.args[i];
                if !locals.contains_key(variable_name.clone(), vm) {
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
                    let arg_name = &arg_names.args[i];
                    if !locals.contains_key(arg_name.clone(), vm) {
                        locals.set_item(arg_name.clone(), defaults[default_index].clone(), vm)?;
                    }
                }
            }
        };

        // Check if kw only arguments are all present:
        for arg_name in arg_names.kwonlyargs {
            if !locals.contains_key(arg_name.clone(), vm) {
                if let Some(kw_only_defaults) = &self.kw_only_defaults {
                    if let Some(default) = kw_only_defaults.get_item_option(arg_name.clone(), vm)? {
                        locals.set_item(arg_name.clone(), default, vm)?;
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
        func_args: FuncArgs,
        scope: &Scope,
        vm: &VirtualMachine,
    ) -> PyResult {
        #[cfg(feature = "jit")]
        if let Some(jitted_code) = self.jitted_code.get() {
            match jitfunc::get_jit_args(self, &func_args, jitted_code, vm) {
                Ok(args) => {
                    return Ok(args.invoke().into_pyobject(vm));
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

        self.fill_locals_from_args(&scope.get_locals(), func_args, vm)?;

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

    pub fn invoke(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        self.invoke_with_scope(func_args, &self.scope, vm)
    }
}

impl PyValue for PyFunction {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.function_type
    }
}

#[pyimpl(with(SlotDescriptor, Callable), flags(HAS_DICT))]
impl PyFunction {
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

impl SlotDescriptor for PyFunction {
    fn descr_get(
        zelf: PyObjectRef,
        obj: Option<PyObjectRef>,
        cls: Option<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult {
        let (zelf, obj) = Self::_unwrap(zelf, obj, vm)?;
        if vm.is_none(&obj) && !Self::_cls_is(&cls, &obj.class()) {
            Ok(zelf.into_object())
        } else {
            Ok(vm.ctx.new_bound_method(zelf.into_object(), obj))
        }
    }
}

impl Callable for PyFunction {
    fn call(zelf: &PyRef<Self>, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        zelf.invoke(args, vm)
    }
}

#[pyclass(module = false, name = "method")]
#[derive(Debug)]
pub struct PyBoundMethod {
    // TODO: these shouldn't be public
    object: PyObjectRef,
    pub function: PyObjectRef,
}

impl Callable for PyBoundMethod {
    fn call(zelf: &PyRef<Self>, mut args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        args.prepend_arg(zelf.object.clone());
        vm.invoke(&zelf.function, args)
    }
}

impl Comparable for PyBoundMethod {
    fn cmp(
        zelf: &PyRef<Self>,
        other: &PyObjectRef,
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

impl SlotGetattro for PyBoundMethod {
    fn getattro(zelf: PyRef<Self>, name: PyStrRef, vm: &VirtualMachine) -> PyResult {
        if let Some(obj) = zelf.get_class_attr(name.borrow_value()) {
            return vm.call_if_get_descriptor(obj, zelf.into_object());
        }
        vm.get_attribute(zelf.function.clone(), name)
    }
}

#[pyimpl(with(Callable, Comparable, SlotGetattro), flags(HAS_DICT))]
impl PyBoundMethod {
    pub fn new(object: PyObjectRef, function: PyObjectRef) -> Self {
        PyBoundMethod { object, function }
    }

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
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.bound_method_type
    }
}

#[pyclass(module = false, name = "cell")]
#[derive(Debug)]
pub(crate) struct PyCell {
    contents: PyMutex<Option<PyObjectRef>>,
}

impl PyValue for PyCell {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.cell_type
    }
}

#[pyimpl]
impl PyCell {
    #[pyslot]
    fn tp_new(cls: PyTypeRef, value: OptionalArg, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        Self {
            contents: PyMutex::new(value.into_option()),
        }
        .into_ref_with_type(vm, cls)
    }

    pub fn get(&self) -> Option<PyObjectRef> {
        self.contents.lock().clone()
    }
    pub fn set(&self, x: Option<PyObjectRef>) {
        *self.contents.lock() = x;
    }

    #[pyproperty]
    fn cell_contents(&self, vm: &VirtualMachine) -> PyResult {
        self.get()
            .ok_or_else(|| vm.new_value_error("Cell is empty".to_owned()))
    }
    #[pyproperty(setter)]
    fn set_cell_contents(&self, x: PyObjectRef) {
        self.set(Some(x))
    }
    #[pyproperty(deleter)]
    fn del_cell_contents(&self) {
        self.set(None)
    }
}

pub fn init(context: &PyContext) {
    PyFunction::extend_class(context, &context.types.function_type);
    PyBoundMethod::extend_class(context, &context.types.bound_method_type);
    PyCell::extend_class(context, &context.types.cell_type);
}
