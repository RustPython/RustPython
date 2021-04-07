use crate::builtins::dict::PyDictRef;
use crate::builtins::function::{PyFunction, PyFunctionRef};
use crate::builtins::{float, int, pybool, PyStrRef};
use crate::bytecode::CodeFlags;
use crate::exceptions::PyBaseExceptionRef;
use crate::function::FuncArgs;
use crate::pyobject::{
    BorrowValue, IdProtocol, IntoPyObject, ItemProtocol, PyObjectRef, PyResult, TryFromObject,
    TypeProtocol,
};
use crate::VirtualMachine;
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

impl IntoPyObject for AbiValue {
    fn into_pyobject(self, vm: &VirtualMachine) -> PyObjectRef {
        match self {
            AbiValue::Int(i) => i.into_pyobject(vm),
            AbiValue::Float(f) => f.into_pyobject(vm),
            AbiValue::Bool(b) => b.into_pyobject(vm),
        }
    }
}

pub fn new_jit_error(msg: String, vm: &VirtualMachine) -> PyBaseExceptionRef {
    let jit_error = vm.ctx.exceptions.jit_error.clone();
    vm.new_exception_msg(jit_error, msg)
}

fn get_jit_arg_type(dict: &PyDictRef, name: &str, vm: &VirtualMachine) -> PyResult<JitType> {
    if let Some(value) = dict.get_item_option(name, vm)? {
        if value.is(&vm.ctx.types.int_type) {
            Ok(JitType::Int)
        } else if value.is(&vm.ctx.types.float_type) {
            Ok(JitType::Float)
        } else if value.is(&vm.ctx.types.bool_type) {
            Ok(JitType::Bool)
        } else {
            Err(new_jit_error(
                "Jit requires argument to be either int or float".to_owned(),
                vm,
            ))
        }
    } else {
        Err(new_jit_error(
            format!("argument {} needs annotation", name),
            vm,
        ))
    }
}

pub fn get_jit_arg_types(func: &PyFunctionRef, vm: &VirtualMachine) -> PyResult<Vec<JitType>> {
    let arg_names = func.code.arg_names();

    if func
        .code
        .flags
        .intersects(CodeFlags::HAS_VARARGS | CodeFlags::HAS_VARKEYWORDS)
    {
        return Err(new_jit_error(
            "Can't jit functions with variable number of arguments".to_owned(),
            vm,
        ));
    }

    if arg_names.args.is_empty() && arg_names.kwonlyargs.is_empty() {
        return Ok(Vec::new());
    }

    let annotations = vm.get_attribute(func.clone().into_object(), "__annotations__")?;
    if vm.is_none(&annotations) {
        Err(new_jit_error(
            "Jitting function requires arguments to have annotations".to_owned(),
            vm,
        ))
    } else if let Ok(dict) = PyDictRef::try_from_object(vm, annotations) {
        let mut arg_types = Vec::new();

        for arg in arg_names.args {
            arg_types.push(get_jit_arg_type(&dict, arg.borrow_value(), vm)?);
        }

        for arg in arg_names.kwonlyargs {
            arg_types.push(get_jit_arg_type(&dict, arg.borrow_value(), vm)?);
        }

        Ok(arg_types)
    } else {
        Err(vm.new_type_error("Function annotations aren't a dict".to_owned()))
    }
}

fn get_jit_value(vm: &VirtualMachine, obj: &PyObjectRef) -> Result<AbiValue, ArgsError> {
    // This does exact type checks as subclasses of int/float can't be passed to jitted functions
    let cls = obj.class();
    if cls.is(&vm.ctx.types.int_type) {
        int::get_value(&obj)
            .to_i64()
            .map(AbiValue::Int)
            .ok_or(ArgsError::IntOverflow)
    } else if cls.is(&vm.ctx.types.float_type) {
        Ok(AbiValue::Float(float::get_value(&obj)))
    } else if cls.is(&vm.ctx.types.bool_type) {
        Ok(AbiValue::Bool(pybool::get_value(&obj)))
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
    let arg_names = func.code.arg_names();

    if nargs > func.code.arg_count || nargs < func.code.posonlyarg_count {
        return Err(ArgsError::WrongNumberOfArgs);
    }

    // Add positional arguments
    for i in 0..nargs {
        jit_args.set(i, get_jit_value(vm, &func_args.args[i])?)?;
    }

    // Handle keyword arguments
    for (name, value) in &func_args.kwargs {
        let arg_pos =
            |args: &[PyStrRef], name: &str| args.iter().position(|arg| arg.borrow_value() == name);
        if let Some(arg_idx) = arg_pos(arg_names.args, name) {
            if jit_args.is_set(arg_idx) {
                return Err(ArgsError::ArgPassedMultipleTimes);
            }
            jit_args.set(arg_idx, get_jit_value(vm, &value)?)?;
        } else if let Some(kwarg_idx) = arg_pos(arg_names.kwonlyargs, name) {
            let arg_idx = kwarg_idx + func.code.arg_count;
            if jit_args.is_set(arg_idx) {
                return Err(ArgsError::ArgPassedMultipleTimes);
            }
            jit_args.set(arg_idx, get_jit_value(vm, &value)?)?;
        } else {
            return Err(ArgsError::NotAKeywordArg);
        }
    }

    let (defaults, kwdefaults) = func.defaults_and_kwdefaults.lock().clone();

    // fill in positional defaults
    if let Some(defaults) = defaults {
        let defaults = defaults.borrow_value();
        for (i, default) in defaults.iter().enumerate() {
            let arg_idx = i + func.code.arg_count - defaults.len();
            if !jit_args.is_set(arg_idx) {
                jit_args.set(arg_idx, get_jit_value(vm, default)?)?;
            }
        }
    }

    // fill in keyword only defaults
    if let Some(kw_only_defaults) = kwdefaults {
        for (i, name) in arg_names.kwonlyargs.iter().enumerate() {
            let arg_idx = i + func.code.arg_count;
            if !jit_args.is_set(arg_idx) {
                let default = kw_only_defaults
                    .get_item(name.clone(), vm)
                    .map_err(|_| ArgsError::NotAllArgsPassed)
                    .and_then(|obj| get_jit_value(vm, &obj))?;
                jit_args.set(arg_idx, default)?;
            }
        }
    }

    jit_args.into_args().ok_or(ArgsError::NotAllArgsPassed)
}
