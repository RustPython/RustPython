use crate::{
    AsObject, Py, PyObject, PyObjectRef, PyResult, TryFromObject, VirtualMachine,
    builtins::{
        PyBaseExceptionRef, PyCode, PyDict, PyDictRef, PyFunction, PyStrInterned, bool_, float, int,
    },
    bytecode::CodeFlags,
    convert::ToPyObject,
    function::FuncArgs,
};
use num_traits::ToPrimitive;
use rustpython_jit::{AbiValue, Args, ArgsBuilder, CompiledCode, JitArgumentError, JitType};

#[derive(Debug, thiserror::Error)]
pub(super) enum ArgsError {
    #[error("wrong number of arguments passed")]
    WrongNumberOfArgs,
    #[error("argument passed multiple times")]
    ArgPassedMultipleTimes,
    #[error("not a keyword argument")]
    NotAKeywordArg,
    #[error("not all arguments passed")]
    NotAllArgsPassed,
    #[error("integer can't fit into a machine integer")]
    IntOverflow,
    #[error("type can't be used in a jit function")]
    NonJitType,
    #[error("{0}")]
    JitError(#[from] JitArgumentError),
}

impl ToPyObject for AbiValue {
    fn to_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        match self {
            Self::Int(i) => i.to_pyobject(vm),
            Self::Float(f) => f.to_pyobject(vm),
            Self::Bool(b) => b.to_pyobject(vm),
            _ => unimplemented!(),
        }
    }
}

pub(super) fn new_jit_error(msg: String, vm: &VirtualMachine) -> PyBaseExceptionRef {
    let jit_error = vm.ctx.exceptions.jit_error.to_owned();
    vm.new_exception_msg(jit_error, msg.into())
}

fn get_jit_arg_type(dict: &Py<PyDict>, name: &str, vm: &VirtualMachine) -> PyResult<JitType> {
    if let Some(value) = dict.get_item_opt(name, vm)? {
        if value.is(vm.ctx.types.int_type) {
            Ok(JitType::Int)
        } else if value.is(vm.ctx.types.float_type) {
            Ok(JitType::Float)
        } else if value.is(vm.ctx.types.bool_type) {
            Ok(JitType::Bool)
        } else {
            Err(new_jit_error(
                "Jit requires argument to be either int, float or bool".to_owned(),
                vm,
            ))
        }
    } else {
        Err(new_jit_error(
            format!("argument {name} needs annotation"),
            vm,
        ))
    }
}

pub(super) fn get_jit_arg_types(
    func: &Py<PyFunction>,
    vm: &VirtualMachine,
) -> PyResult<Vec<JitType>> {
    let code: &Py<PyCode> = &func.code;
    let arg_names = code.arg_names();

    if code
        .flags
        .intersects(CodeFlags::VARARGS | CodeFlags::VARKEYWORDS)
    {
        return Err(new_jit_error(
            "Can't jit functions with variable number of arguments".to_owned(),
            vm,
        ));
    }

    if arg_names.args.is_empty() && arg_names.kwonlyargs.is_empty() {
        return Ok(Vec::new());
    }

    let func_obj: PyObjectRef = func.as_ref().to_owned();
    let annotations = func_obj.get_attr("__annotations__", vm)?;
    if vm.is_none(&annotations) {
        Err(new_jit_error(
            "Jitting function requires arguments to have annotations".to_owned(),
            vm,
        ))
    } else if let Ok(dict) = PyDictRef::try_from_object(vm, annotations) {
        let mut arg_types = Vec::new();

        for arg in arg_names.args {
            arg_types.push(get_jit_arg_type(&dict, arg.as_str(), vm)?);
        }

        for arg in arg_names.kwonlyargs {
            arg_types.push(get_jit_arg_type(&dict, arg.as_str(), vm)?);
        }

        Ok(arg_types)
    } else {
        Err(vm.new_type_error("Function annotations aren't a dict"))
    }
}

pub(super) fn jit_ret_type(
    func: &Py<PyFunction>,
    vm: &VirtualMachine,
) -> PyResult<Option<JitType>> {
    let func_obj: PyObjectRef = func.as_ref().to_owned();
    let annotations = func_obj.get_attr("__annotations__", vm)?;
    if vm.is_none(&annotations) {
        Err(new_jit_error(
            "Jitting function requires return type to have annotations".to_owned(),
            vm,
        ))
    } else if let Ok(dict) = PyDictRef::try_from_object(vm, annotations) {
        if dict.contains_key("return", vm) {
            get_jit_arg_type(&dict, "return", vm).map_or(Ok(None), |t| Ok(Some(t)))
        } else {
            Ok(None)
        }
    } else {
        Err(vm.new_type_error("Function annotations aren't a dict"))
    }
}

fn get_jit_value(vm: &VirtualMachine, obj: &PyObject) -> Result<AbiValue, ArgsError> {
    // This does exact type checks as subclasses of int/float can't be passed to jitted functions
    let cls = obj.class();
    if cls.is(vm.ctx.types.int_type) {
        int::get_value(obj)
            .to_i64()
            .map(AbiValue::Int)
            .ok_or(ArgsError::IntOverflow)
    } else if cls.is(vm.ctx.types.float_type) {
        Ok(AbiValue::Float(
            obj.downcast_ref::<float::PyFloat>().unwrap().to_f64(),
        ))
    } else if cls.is(vm.ctx.types.bool_type) {
        Ok(AbiValue::Bool(bool_::get_value(obj)))
    } else {
        Err(ArgsError::NonJitType)
    }
}

/// Where a walk over a call's arguments puts what it finds. The two walks
/// resolve the same parameters from the same sources and must agree on which
/// calls they accept, so they share the walk and differ only here: one fills a
/// compiled function's slots, the other collects the types those slots hold.
trait ArgSink {
    /// Take the object this parameter gets its value from. A parameter the
    /// compiled code cannot carry is rejected here.
    fn put(&mut self, index: usize, value: &PyObject, vm: &VirtualMachine)
    -> Result<(), ArgsError>;
    /// Whether the walk has already filled this parameter, which is what
    /// decides between a default and what the call passed.
    fn is_filled(&self, index: usize) -> bool;
}

impl ArgSink for ArgsBuilder<'_> {
    fn put(
        &mut self,
        index: usize,
        value: &PyObject,
        vm: &VirtualMachine,
    ) -> Result<(), ArgsError> {
        self.set(index, get_jit_value(vm, value)?)?;
        Ok(())
    }

    fn is_filled(&self, index: usize) -> bool {
        self.is_set(index)
    }
}

/// The types a call's arguments would give a compiled function's parameters,
/// in parameter order.
struct ObservedTypes(Vec<Option<JitType>>);

impl ArgSink for ObservedTypes {
    fn put(
        &mut self,
        index: usize,
        value: &PyObject,
        vm: &VirtualMachine,
    ) -> Result<(), ArgsError> {
        // Through `get_jit_value` rather than off the class directly, so that
        // what is observed as an `int` is exactly what would later convert to
        // one - an integer too wide for the machine is not this parameter's
        // type, it is a call the compiled code could not have served.
        let ty = match get_jit_value(vm, value)? {
            AbiValue::Int(_) => JitType::Int,
            AbiValue::Float(_) => JitType::Float,
            AbiValue::Bool(_) => JitType::Bool,
            _ => return Err(ArgsError::NonJitType),
        };
        *self.0.get_mut(index).ok_or(ArgsError::WrongNumberOfArgs)? = Some(ty);
        Ok(())
    }

    fn is_filled(&self, index: usize) -> bool {
        self.0.get(index).is_some_and(Option::is_some)
    }
}

/// Resolve a call's arguments onto the parameters they fill, positional first,
/// then keyword, then the defaults for whatever is left - the order
/// `fill_locals_from_args` resolves them in, and the order a compiled
/// function's slots are laid out in.
///
/// Unlike `fill_locals_from_args` this raises nothing: a call it turns down
/// goes to the interpreter, which raises whatever the call really deserves.
fn walk_arguments<S: ArgSink>(
    func: &PyFunction,
    func_args: &FuncArgs,
    sink: &mut S,
    vm: &VirtualMachine,
) -> Result<(), ArgsError> {
    let nargs = func_args.args.len();

    let code: &Py<PyCode> = &func.code;
    let arg_names = code.arg_names();
    let arg_count = code.arg_count;
    let posonlyarg_count = code.posonlyarg_count;

    if nargs > arg_count as usize || nargs < posonlyarg_count as usize {
        return Err(ArgsError::WrongNumberOfArgs);
    }

    // Add positional arguments
    for i in 0..nargs {
        sink.put(i, &func_args.args[i], vm)?;
    }

    // Handle keyword arguments
    for (name, value) in &func_args.kwargs {
        let arg_pos =
            |args: &[&PyStrInterned], name: &str| args.iter().position(|arg| arg.as_str() == name);
        // Parameter names are plain identifiers, so a non-UTF-8 (surrogate) key
        // can never match one.
        let name = name.as_str().map_err(|_| ArgsError::NotAKeywordArg)?;
        if let Some(arg_idx) = arg_pos(arg_names.args, name) {
            if sink.is_filled(arg_idx) {
                return Err(ArgsError::ArgPassedMultipleTimes);
            }
            sink.put(arg_idx, value, vm)?;
        } else if let Some(kwarg_idx) = arg_pos(arg_names.kwonlyargs, name) {
            let arg_idx = kwarg_idx + arg_count as usize;
            if sink.is_filled(arg_idx) {
                return Err(ArgsError::ArgPassedMultipleTimes);
            }
            sink.put(arg_idx, value, vm)?;
        } else {
            return Err(ArgsError::NotAKeywordArg);
        }
    }

    // Held rather than cloned: filling a slot from a default reads the object
    // but never runs Python code, so nothing can reach the lock again.
    let defaults_and_kwdefaults = func.defaults_and_kwdefaults.lock();
    let (defaults, kwdefaults) = &*defaults_and_kwdefaults;

    // fill in positional defaults
    if let Some(defaults) = defaults {
        for (i, default) in defaults.iter().enumerate() {
            let arg_idx = i + arg_count as usize - defaults.len();
            if !sink.is_filled(arg_idx) {
                sink.put(arg_idx, default, vm)?;
            }
        }
    }

    // fill in keyword only defaults
    if let Some(kw_only_defaults) = kwdefaults {
        for (i, name) in arg_names.kwonlyargs.iter().enumerate() {
            let arg_idx = i + arg_count as usize;
            if !sink.is_filled(arg_idx) {
                let default = kw_only_defaults
                    .get_item(&**name, vm)
                    .map_err(|_| ArgsError::NotAllArgsPassed)?;
                sink.put(arg_idx, &default, vm)?;
            }
        }
    }

    Ok(())
}

/// The parameter types this call would give the function, for the compiler to
/// specialize on.
///
/// The automatic path takes its types from here rather than from annotations.
/// Almost nothing outside a type-checked codebase is annotated `int`, `float`
/// or `bool` on every parameter - seven functions in the whole standard
/// library are - while every call carries the types it is actually being made
/// with. A guess that turns out wrong costs a failed conversion and a fall
/// back to the interpreter, never a wrong answer.
#[cfg(feature = "jit")]
pub(super) fn observed_arg_types(
    func: &PyFunction,
    func_args: &FuncArgs,
    vm: &VirtualMachine,
) -> Result<Vec<JitType>, ArgsError> {
    let code: &Py<PyCode> = &func.code;
    if code
        .flags
        .intersects(CodeFlags::VARARGS | CodeFlags::VARKEYWORDS)
    {
        return Err(ArgsError::NonJitType);
    }
    let slots = code.arg_count as usize + code.arg_names().kwonlyargs.len();
    let mut observed = ObservedTypes(vec![None; slots]);
    walk_arguments(func, func_args, &mut observed, vm)?;
    observed
        .0
        .into_iter()
        .collect::<Option<Vec<_>>>()
        .ok_or(ArgsError::NotAllArgsPassed)
}

/// Like `fill_locals_from_args` but to populate arguments for calling a jit function.
/// This also doesn't do full error handling but instead return None if anything is wrong. In
/// that case it falls back to the executing the bytecode version which will call
/// `fill_locals_from_args` which will raise the actual exception if needed.
#[cfg(feature = "jit")]
pub(crate) fn get_jit_args<'a>(
    func: &PyFunction,
    func_args: &FuncArgs,
    jitted_code: &'a CompiledCode,
    vm: &VirtualMachine,
) -> Result<Args<'a>, ArgsError> {
    let mut jit_args = jitted_code.args_builder();
    walk_arguments(func, func_args, &mut jit_args, vm)?;
    jit_args.into_args().ok_or(ArgsError::NotAllArgsPassed)
}
