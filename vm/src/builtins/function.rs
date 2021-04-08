#[cfg(feature = "jit")]
mod jitfunc;

use super::code::PyCodeRef;
use super::dict::PyDictRef;
use super::pystr::PyStrRef;
use super::pytype::PyTypeRef;
use super::tuple::{PyTupleRef, PyTupleTyped};
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
    globals: PyDictRef,
    closure: Option<PyTupleTyped<PyCellRef>>,
    defaults_and_kwdefaults: PyMutex<(Option<PyTupleRef>, Option<PyDictRef>)>,
    name: PyMutex<PyStrRef>,
}

impl PyFunction {
    pub(crate) fn new(
        code: PyCodeRef,
        globals: PyDictRef,
        closure: Option<PyTupleTyped<PyCellRef>>,
        defaults: Option<PyTupleRef>,
        kw_only_defaults: Option<PyDictRef>,
    ) -> Self {
        let name = PyMutex::new(code.obj_name.clone());
        PyFunction {
            code,
            #[cfg(feature = "jit")]
            jitted_code: OnceCell::new(),
            globals,
            closure,
            defaults_and_kwdefaults: PyMutex::new((defaults, kw_only_defaults)),
            name,
        }
    }

    fn fill_locals_from_args(
        &self,
        frame: &Frame,
        func_args: FuncArgs,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let code = &*self.code;
        let nargs = func_args.args.len();
        let nexpected_args = code.arg_count;
        let total_args = code.arg_count + code.kwonlyarg_count;
        // let arg_names = self.code.arg_names();

        // This parses the arguments from args and kwargs into
        // the proper variables keeping into account default values
        // and starargs and kwargs.
        // See also: PyEval_EvalCodeWithName in cpython:
        // https://github.com/python/cpython/blob/master/Python/ceval.c#L3681

        let mut fastlocals = frame.fastlocals.lock();

        let mut args_iter = func_args.args.into_iter();

        // Copy positional arguments into local variables
        // zip short-circuits if either iterator returns None, which is the behavior we want --
        // only fill as much as there is to fill with as much as we have
        for (local, arg) in Iterator::zip(
            fastlocals.iter_mut().take(nexpected_args),
            args_iter.by_ref().take(nargs),
        ) {
            *local = Some(arg);
        }

        let mut vararg_offset = total_args;
        // Pack other positional arguments in to *args:
        if code.flags.contains(bytecode::CodeFlags::HAS_VARARGS) {
            let vararg_value = vm.ctx.new_tuple(args_iter.collect());
            fastlocals[vararg_offset] = Some(vararg_value);
            vararg_offset += 1;
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
        let kwargs = if code.flags.contains(bytecode::CodeFlags::HAS_VARKEYWORDS) {
            let d = vm.ctx.new_dict();
            fastlocals[vararg_offset] = Some(d.clone().into_object());
            Some(d)
        } else {
            None
        };

        let argpos = |range: std::ops::Range<_>, name: &str| {
            code.varnames
                .iter()
                .enumerate()
                .skip(range.start)
                .take(range.end - range.start)
                .find(|(_, s)| s.borrow_value() == name)
                .map(|(p, _)| p)
        };

        let mut posonly_passed_as_kwarg = Vec::new();
        // Handle keyword arguments
        for (name, value) in func_args.kwargs {
            // Check if we have a parameter with this name:
            if let Some(pos) = argpos(code.posonlyarg_count..total_args, &name) {
                let slot = &mut fastlocals[pos];
                if slot.is_some() {
                    return Err(
                        vm.new_type_error(format!("Got multiple values for argument '{}'", name))
                    );
                }
                *slot = Some(value);
            } else if argpos(0..code.posonlyarg_count, &name).is_some() {
                posonly_passed_as_kwarg.push(name);
            } else if let Some(kwargs) = kwargs.as_ref() {
                kwargs.set_item(name, value, vm)?;
            } else {
                return Err(
                    vm.new_type_error(format!("got an unexpected keyword argument '{}'", name))
                );
            }
        }
        if !posonly_passed_as_kwarg.is_empty() {
            return Err(vm.new_type_error(format!(
                "{}() got some positional-only arguments passed as keyword arguments: '{}'",
                &self.code.obj_name,
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
        if nargs < nexpected_args {
            let defaults = get_defaults!().0.as_ref().map(|tup| tup.borrow_value());
            let ndefs = defaults.map_or(0, |d| d.len());

            let nrequired = code.arg_count - ndefs;

            // Given the number of defaults available, check all the arguments for which we
            // _don't_ have defaults; if any are missing, raise an exception
            let mut missing = vec![];
            for i in nargs..nrequired {
                if fastlocals[i].is_none() {
                    missing.push(&code.varnames[i]);
                }
            }
            if !missing.is_empty() {
                return Err(vm.new_type_error(format!(
                    "Missing {} required positional arguments: {}",
                    missing.len(),
                    missing.iter().format(", ")
                )));
            }

            if let Some(defaults) = defaults {
                let n = std::cmp::min(nargs, nexpected_args);
                let i = n.saturating_sub(nrequired);

                // We have sufficient defaults, so iterate over the corresponding names and use
                // the default if we don't already have a value
                for i in i..defaults.len() {
                    let slot = &mut fastlocals[nrequired + i];
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
                .skip(code.arg_count)
                .take(code.kwonlyarg_count)
            {
                if slot.is_none() {
                    if let Some(defaults) = &get_defaults!().1 {
                        if let Some(default) = defaults.get_item_option(kwarg.clone(), vm)? {
                            *slot = Some(default);
                            continue;
                        }
                    }

                    // No default value and not specified.
                    return Err(vm.new_type_error(format!(
                        "Missing required kw only argument: '{}'",
                        kwarg
                    )));
                }
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

    pub fn invoke_with_locals(
        &self,
        func_args: FuncArgs,
        locals: Option<PyDictRef>,
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

        let locals = if self.code.flags.contains(bytecode::CodeFlags::NEW_LOCALS) {
            vm.ctx.new_dict()
        } else {
            locals.unwrap_or_else(|| self.globals.clone())
        };

        // Construct frame:
        let frame = Frame::new(
            code.clone(),
            Scope::new(Some(locals), self.globals.clone()),
            vm.builtins.dict().unwrap(),
            self.closure.as_ref().map_or(&[], |c| c.borrow_value()),
            vm,
        )
        .into_ref(vm);

        self.fill_locals_from_args(&frame, func_args, vm)?;

        // If we have a generator, create a new generator
        let is_gen = code.flags.contains(bytecode::CodeFlags::IS_GENERATOR);
        let is_coro = code.flags.contains(bytecode::CodeFlags::IS_COROUTINE);
        match (is_gen, is_coro) {
            (true, false) => Ok(PyGenerator::new(frame, self.name()).into_object(vm)),
            (false, true) => Ok(PyCoroutine::new(frame, self.name()).into_object(vm)),
            (true, true) => Ok(PyAsyncGen::new(frame, self.name()).into_object(vm)),
            (false, false) => vm.run_frame_full(frame),
        }
    }

    pub fn invoke(&self, func_args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        self.invoke_with_locals(func_args, None, vm)
    }
}

impl PyValue for PyFunction {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.function_type
    }
}

#[pyimpl(with(SlotDescriptor, Callable), flags(HAS_DICT, METHOD_DESCR))]
impl PyFunction {
    #[pyproperty(magic)]
    fn code(&self) -> PyCodeRef {
        self.code.clone()
    }

    #[pyproperty(magic)]
    fn defaults(&self) -> Option<PyTupleRef> {
        self.defaults_and_kwdefaults.lock().0.clone()
    }
    #[pyproperty(magic, setter)]
    fn set_defaults(&self, defaults: Option<PyTupleRef>) {
        self.defaults_and_kwdefaults.lock().0 = defaults
    }

    #[pyproperty(magic)]
    fn kwdefaults(&self) -> Option<PyDictRef> {
        self.defaults_and_kwdefaults.lock().1.clone()
    }
    #[pyproperty(magic, setter)]
    fn set_kwdefaults(&self, kwdefaults: Option<PyDictRef>) {
        self.defaults_and_kwdefaults.lock().1 = kwdefaults
    }

    #[pyproperty(magic)]
    fn globals(&self) -> PyDictRef {
        self.globals.clone()
    }

    #[pyproperty(magic)]
    fn closure(&self) -> Option<PyTupleTyped<PyCellRef>> {
        self.closure.clone()
    }

    #[pyproperty(magic)]
    fn name(&self) -> PyStrRef {
        self.name.lock().clone()
    }

    #[pyproperty(magic, setter)]
    fn set_name(&self, name: PyStrRef) {
        *self.name.lock() = name;
    }

    #[pymethod(magic)]
    fn repr(zelf: PyRef<Self>) -> String {
        format!("<function {} at {:#x}>", zelf.name.lock(), zelf.get_id())
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

    #[pyslot]
    fn tp_new(
        cls: PyTypeRef,
        function: PyObjectRef,
        object: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyRef<Self>> {
        PyBoundMethod::new(object, function).into_ref_with_type(vm, cls)
    }

    #[pymethod(magic)]
    fn repr(&self, vm: &VirtualMachine) -> PyResult<String> {
        let funcname = if let Some(qname) =
            vm.get_attribute_opt(self.function.clone(), "__qualname__")?
        {
            Some(qname)
        } else if let Some(name) = vm.get_attribute_opt(self.function.clone(), "__qualname__")? {
            Some(name)
        } else {
            None
        };
        let funcname: Option<PyStrRef> = funcname.and_then(|o| o.downcast().ok());
        Ok(format!(
            "<bound method {} of {}>",
            funcname.as_ref().map_or("?", |s| s.borrow_value()),
            vm.to_repr(&self.object)?.borrow_value(),
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
#[derive(Debug, Default)]
pub(crate) struct PyCell {
    contents: PyMutex<Option<PyObjectRef>>,
}
pub(crate) type PyCellRef = PyRef<PyCell>;

impl PyValue for PyCell {
    fn class(vm: &VirtualMachine) -> &PyTypeRef {
        &vm.ctx.types.cell_type
    }
}

#[pyimpl]
impl PyCell {
    #[pyslot]
    fn tp_new(cls: PyTypeRef, value: OptionalArg, vm: &VirtualMachine) -> PyResult<PyRef<Self>> {
        Self::new(value.into_option()).into_ref_with_type(vm, cls)
    }

    pub fn new(contents: Option<PyObjectRef>) -> Self {
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
