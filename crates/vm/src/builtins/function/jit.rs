use crate::{
    AsObject, Py, PyObject, PyObjectRef, PyResult, TryFromObject, VirtualMachine,
    builtins::{
        PyBaseExceptionRef, PyDict, PyDictRef, PyFunction, PyStrInterned, bool_, float, int,
    },
    bytecode::CodeFlags,
    convert::ToPyObject,
    function::FuncArgs,
};
use num_traits::ToPrimitive;
use rustpython_jit::{AbiValue, Args, CompiledCode, JitArgumentError, JitType};

#[derive(Debug, thiserror::Error)]
pub enum ArgsError {
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
            AbiValue::Int(i) => i.to_pyobject(vm),
            AbiValue::Float(f) => f.to_pyobject(vm),
            AbiValue::Bool(b) => b.to_pyobject(vm),
            _ => unimplemented!(),
        }
    }
}

pub fn new_jit_error(msg: String, vm: &VirtualMachine) -> PyBaseExceptionRef {
    let jit_error = vm.ctx.exceptions.jit_error.to_owned();
    vm.new_exception_msg(jit_error, msg)
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

pub fn get_jit_arg_types(func: &Py<PyFunction>, vm: &VirtualMachine) -> PyResult<Vec<JitType>> {
    let code = func.code.lock();
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

pub fn jit_ret_type(func: &Py<PyFunction>, vm: &VirtualMachine) -> PyResult<Option<JitType>> {
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
    let nargs = func_args.args.len();

    let code = func.code.lock();
    let arg_names = code.arg_names();
    let arg_count = code.arg_count;
    let posonlyarg_count = code.posonlyarg_count;

    if nargs > arg_count as usize || nargs < posonlyarg_count as usize {
        return Err(ArgsError::WrongNumberOfArgs);
    }

    // Add positional arguments
    for i in 0..nargs {
        jit_args.set(i, get_jit_value(vm, &func_args.args[i])?)?;
    }

    // Handle keyword arguments
    for (name, value) in &func_args.kwargs {
        let arg_pos =
            |args: &[&PyStrInterned], name: &str| args.iter().position(|arg| arg.as_str() == name);
        if let Some(arg_idx) = arg_pos(arg_names.args, name) {
            if jit_args.is_set(arg_idx) {
                return Err(ArgsError::ArgPassedMultipleTimes);
            }
            jit_args.set(arg_idx, get_jit_value(vm, value)?)?;
        } else if let Some(kwarg_idx) = arg_pos(arg_names.kwonlyargs, name) {
            let arg_idx = kwarg_idx + arg_count as usize;
            if jit_args.is_set(arg_idx) {
                return Err(ArgsError::ArgPassedMultipleTimes);
            }
            jit_args.set(arg_idx, get_jit_value(vm, value)?)?;
        } else {
            return Err(ArgsError::NotAKeywordArg);
        }
    }

    let (defaults, kwdefaults) = func.defaults_and_kwdefaults.lock().clone();

    // fill in positional defaults
    if let Some(defaults) = defaults {
        for (i, default) in defaults.iter().enumerate() {
            let arg_idx = i + arg_count as usize - defaults.len();
            if !jit_args.is_set(arg_idx) {
                jit_args.set(arg_idx, get_jit_value(vm, default)?)?;
            }
        }
    }

    // fill in keyword only defaults
    if let Some(kw_only_defaults) = kwdefaults {
        for (i, name) in arg_names.kwonlyargs.iter().enumerate() {
            let arg_idx = i + arg_count as usize;
            if !jit_args.is_set(arg_idx) {
                let default = kw_only_defaults
                    .get_item(&**name, vm)
                    .map_err(|_| ArgsError::NotAllArgsPassed)
                    .and_then(|obj| get_jit_value(vm, &obj))?;
                jit_args.set(arg_idx, default)?;
            }
        }
    }

    drop(code);

    jit_args.into_args().ok_or(ArgsError::NotAllArgsPassed)
}
