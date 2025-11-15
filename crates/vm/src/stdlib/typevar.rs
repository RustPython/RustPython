// spell-checker:ignore typevarobject funcobj
use crate::{
    AsObject, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
    builtins::{PyTupleRef, PyTypeRef, pystr::AsPyStr},
    common::lock::PyMutex,
    function::{FuncArgs, IntoFuncArgs, PyComparisonValue},
    protocol::PyNumberMethods,
    types::{AsNumber, Comparable, Constructor, Iterable, PyComparisonOp, Representable},
};

pub(crate) fn _call_typing_func_object<'a>(
    vm: &VirtualMachine,
    func_name: impl AsPyStr<'a>,
    args: impl IntoFuncArgs,
) -> PyResult {
    let module = vm.import("typing", 0)?;
    let func = module.get_attr(func_name.as_pystr(&vm.ctx), vm)?;
    func.call(args, vm)
}

fn type_check(arg: PyObjectRef, msg: &str, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
    // Calling typing.py here leads to bootstrapping problems
    if vm.is_none(&arg) {
        return Ok(arg.class().to_owned().into());
    }
    let message_str: PyObjectRef = vm.ctx.new_str(msg).into();
    _call_typing_func_object(vm, "_type_check", (arg, message_str))
}

/// Get the module of the caller frame, similar to CPython's caller() function.
/// Returns the module name or None if not found.
///
/// Note: CPython's implementation (in typevarobject.c) gets the module from the
/// frame's function object using PyFunction_GetModule(f->f_funcobj). However,
/// RustPython's Frame doesn't store a reference to the function object, so we
/// get the module name from the frame's globals dictionary instead.
fn caller(vm: &VirtualMachine) -> Option<PyObjectRef> {
    let frame = vm.current_frame()?;

    // In RustPython, we get the module name from frame's globals
    // This is similar to CPython's sys._getframe().f_globals.get('__name__')
    frame.globals.get_item("__name__", vm).ok()
}

/// Set __module__ attribute for an object based on the caller's module.
/// This follows CPython's behavior for TypeVar and similar objects.
fn set_module_from_caller(obj: &PyObjectRef, vm: &VirtualMachine) -> PyResult<()> {
    // Note: CPython gets module from frame->f_funcobj, but RustPython's Frame
    // architecture is different - we use globals['__name__'] instead
    if let Some(module_name) = caller(vm) {
        // Special handling for certain module names
        if let Ok(name_str) = module_name.str(vm) {
            let name = name_str.as_str();
            // CPython sets __module__ to None for builtins and <...> modules
            // Also set to None for exec contexts (no __name__ in globals means exec)
            if name == "builtins" || name.starts_with('<') {
                // Don't set __module__ attribute at all (CPython behavior)
                // This allows the typing module to handle it
                return Ok(());
            }
        }
        obj.set_attr("__module__", module_name, vm)?;
    } else {
        // If no module name is found (e.g., in exec context), set __module__ to None
        obj.set_attr("__module__", vm.ctx.none(), vm)?;
    }
    Ok(())
}

#[pyclass(name = "TypeVar", module = "typing")]
#[derive(Debug, PyPayload)]
#[allow(dead_code)]
pub struct TypeVar {
    name: PyObjectRef, // TODO PyStrRef?
    bound: parking_lot::Mutex<PyObjectRef>,
    evaluate_bound: PyObjectRef,
    constraints: parking_lot::Mutex<PyObjectRef>,
    evaluate_constraints: PyObjectRef,
    default_value: parking_lot::Mutex<PyObjectRef>,
    evaluate_default: PyMutex<PyObjectRef>,
    covariant: bool,
    contravariant: bool,
    infer_variance: bool,
}
#[pyclass(flags(HAS_DICT), with(AsNumber, Constructor, Representable))]
impl TypeVar {
    #[pymethod]
    fn __mro_entries__(&self, _bases: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        Err(vm.new_type_error("Cannot subclass an instance of TypeVar"))
    }

    #[pygetset]
    fn __name__(&self) -> PyObjectRef {
        self.name.clone()
    }

    #[pygetset]
    fn __constraints__(&self, vm: &VirtualMachine) -> PyResult {
        let mut constraints = self.constraints.lock();
        if !vm.is_none(&constraints) {
            return Ok(constraints.clone());
        }
        let r = if !vm.is_none(&self.evaluate_constraints) {
            *constraints = self.evaluate_constraints.call((), vm)?;
            constraints.clone()
        } else {
            vm.ctx.empty_tuple.clone().into()
        };
        Ok(r)
    }

    #[pygetset]
    fn __bound__(&self, vm: &VirtualMachine) -> PyResult {
        let mut bound = self.bound.lock();
        if !vm.is_none(&bound) {
            return Ok(bound.clone());
        }
        let r = if !vm.is_none(&self.evaluate_bound) {
            *bound = self.evaluate_bound.call((), vm)?;
            bound.clone()
        } else {
            vm.ctx.none()
        };
        Ok(r)
    }

    #[pygetset]
    const fn __covariant__(&self) -> bool {
        self.covariant
    }

    #[pygetset]
    const fn __contravariant__(&self) -> bool {
        self.contravariant
    }

    #[pygetset]
    const fn __infer_variance__(&self) -> bool {
        self.infer_variance
    }

    #[pygetset]
    fn __default__(&self, vm: &VirtualMachine) -> PyResult {
        let mut default_value = self.default_value.lock();
        // Check if default_value is NoDefault (not just None)
        if !default_value.is(&vm.ctx.typing_no_default) {
            return Ok(default_value.clone());
        }
        let evaluate_default = self.evaluate_default.lock();
        if !vm.is_none(&evaluate_default) {
            *default_value = evaluate_default.call((), vm)?;
            Ok(default_value.clone())
        } else {
            // Return NoDefault singleton
            Ok(vm.ctx.typing_no_default.clone().into())
        }
    }

    #[pymethod]
    fn __typing_subst__(
        zelf: crate::PyRef<Self>,
        arg: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult {
        let self_obj: PyObjectRef = zelf.into();
        _call_typing_func_object(vm, "_typevar_subst", (self_obj, arg))
    }

    #[pymethod]
    fn __reduce__(&self) -> PyObjectRef {
        self.name.clone()
    }

    #[pymethod]
    fn has_default(&self, vm: &VirtualMachine) -> bool {
        if !vm.is_none(&self.evaluate_default.lock()) {
            return true;
        }
        let default_value = self.default_value.lock();
        // Check if default_value is not NoDefault
        !default_value.is(&vm.ctx.typing_no_default)
    }

    #[pymethod]
    fn __typing_prepare_subst__(
        zelf: crate::PyRef<Self>,
        alias: PyObjectRef,
        args: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult {
        // Convert args to tuple if needed
        let args_tuple = if let Ok(tuple) = args.try_to_ref::<rustpython_vm::builtins::PyTuple>(vm)
        {
            tuple
        } else {
            return Ok(args);
        };

        // Get alias.__parameters__
        let parameters = alias.get_attr(identifier!(vm, __parameters__), vm)?;
        let params_tuple: PyTupleRef = parameters.try_into_value(vm)?;

        // Find our index in parameters
        let self_obj: PyObjectRef = zelf.to_owned().into();
        let param_index = params_tuple.iter().position(|p| p.is(&self_obj));

        if let Some(index) = param_index {
            // Check if we have enough arguments
            if args_tuple.len() <= index && zelf.has_default(vm) {
                // Need to add default value
                let mut new_args: Vec<PyObjectRef> = args_tuple.iter().cloned().collect();

                // Add default value at the correct position
                while new_args.len() <= index {
                    // For the current parameter, add its default
                    if new_args.len() == index {
                        let default_val = zelf.__default__(vm)?;
                        new_args.push(default_val);
                    } else {
                        // This shouldn't happen in well-formed code
                        break;
                    }
                }

                return Ok(rustpython_vm::builtins::PyTuple::new_ref(new_args, &vm.ctx).into());
            }
        }

        // No changes needed
        Ok(args)
    }
}

impl Representable for TypeVar {
    #[inline(always)]
    fn repr_str(zelf: &crate::Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
        let name = zelf.name.str(vm)?;
        let repr = if zelf.covariant {
            format!("+{name}")
        } else if zelf.contravariant {
            format!("-{name}")
        } else {
            format!("~{name}")
        };
        Ok(repr)
    }
}

impl AsNumber for TypeVar {
    fn as_number() -> &'static PyNumberMethods {
        static AS_NUMBER: PyNumberMethods = PyNumberMethods {
            or: Some(|a, b, vm| {
                _call_typing_func_object(vm, "_make_union", (a.to_owned(), b.to_owned()))
            }),
            ..PyNumberMethods::NOT_IMPLEMENTED
        };
        &AS_NUMBER
    }
}

impl Constructor for TypeVar {
    type Args = FuncArgs;

    fn py_new(cls: PyTypeRef, args: Self::Args, vm: &VirtualMachine) -> PyResult {
        let mut kwargs = args.kwargs;
        // Parse arguments manually
        let (name, constraints) = if args.args.is_empty() {
            // Check if name is provided as keyword argument
            if let Some(name) = kwargs.swap_remove("name") {
                (name, vec![])
            } else {
                return Err(
                    vm.new_type_error("TypeVar() missing required argument: 'name' (pos 1)")
                );
            }
        } else if args.args.len() == 1 {
            (args.args[0].clone(), vec![])
        } else {
            let name = args.args[0].clone();
            let constraints = args.args[1..].to_vec();
            (name, constraints)
        };

        let bound = kwargs.swap_remove("bound");
        let covariant = kwargs
            .swap_remove("covariant")
            .map(|v| v.try_to_bool(vm))
            .transpose()?
            .unwrap_or(false);
        let contravariant = kwargs
            .swap_remove("contravariant")
            .map(|v| v.try_to_bool(vm))
            .transpose()?
            .unwrap_or(false);
        let infer_variance = kwargs
            .swap_remove("infer_variance")
            .map(|v| v.try_to_bool(vm))
            .transpose()?
            .unwrap_or(false);
        let default = kwargs.swap_remove("default");

        // Check for unexpected keyword arguments
        if !kwargs.is_empty() {
            let unexpected_keys: Vec<String> = kwargs.keys().map(|s| s.to_string()).collect();
            return Err(vm.new_type_error(format!(
                "TypeVar() got unexpected keyword argument(s): {}",
                unexpected_keys.join(", ")
            )));
        }

        // Check for invalid combinations
        if covariant && contravariant {
            return Err(vm.new_value_error("Bivariant type variables are not supported."));
        }

        if infer_variance && (covariant || contravariant) {
            return Err(vm.new_value_error("Variance cannot be specified with infer_variance"));
        }

        // Handle constraints and bound
        let (constraints_obj, evaluate_constraints) = if !constraints.is_empty() {
            // Check for single constraint
            if constraints.len() == 1 {
                return Err(vm.new_type_error("A single constraint is not allowed"));
            }
            if bound.is_some() {
                return Err(vm.new_type_error("Constraints cannot be used with bound"));
            }
            let constraints_tuple = vm.ctx.new_tuple(constraints);
            (constraints_tuple.clone().into(), constraints_tuple.into())
        } else {
            (vm.ctx.none(), vm.ctx.none())
        };

        // Handle bound
        let (bound_obj, evaluate_bound) = if let Some(bound) = bound {
            if vm.is_none(&bound) {
                (vm.ctx.none(), vm.ctx.none())
            } else {
                // Type check the bound
                let bound = type_check(bound, "Bound must be a type.", vm)?;
                (bound, vm.ctx.none())
            }
        } else {
            (vm.ctx.none(), vm.ctx.none())
        };

        // Handle default value
        let (default_value, evaluate_default) = if let Some(default) = default {
            (default, vm.ctx.none())
        } else {
            // If no default provided, use NoDefault singleton
            (vm.ctx.typing_no_default.clone().into(), vm.ctx.none())
        };

        let typevar = Self {
            name,
            bound: parking_lot::Mutex::new(bound_obj),
            evaluate_bound,
            constraints: parking_lot::Mutex::new(constraints_obj),
            evaluate_constraints,
            default_value: parking_lot::Mutex::new(default_value),
            evaluate_default: PyMutex::new(evaluate_default),
            covariant,
            contravariant,
            infer_variance,
        };

        let obj = typevar.into_ref_with_type(vm, cls)?;
        let obj_ref: PyObjectRef = obj.into();
        set_module_from_caller(&obj_ref, vm)?;
        Ok(obj_ref)
    }
}

impl TypeVar {
    pub fn new(
        vm: &VirtualMachine,
        name: PyObjectRef,
        evaluate_bound: PyObjectRef,
        evaluate_constraints: PyObjectRef,
    ) -> Self {
        Self {
            name,
            bound: parking_lot::Mutex::new(vm.ctx.none()),
            evaluate_bound,
            constraints: parking_lot::Mutex::new(vm.ctx.none()),
            evaluate_constraints,
            default_value: parking_lot::Mutex::new(vm.ctx.typing_no_default.clone().into()),
            evaluate_default: PyMutex::new(vm.ctx.none()),
            covariant: false,
            contravariant: false,
            infer_variance: false,
        }
    }
}

#[pyclass(name = "ParamSpec", module = "typing")]
#[derive(Debug, PyPayload)]
#[allow(dead_code)]
pub struct ParamSpec {
    name: PyObjectRef,
    bound: Option<PyObjectRef>,
    default_value: PyObjectRef,
    evaluate_default: PyMutex<PyObjectRef>,
    covariant: bool,
    contravariant: bool,
    infer_variance: bool,
}

#[pyclass(flags(HAS_DICT), with(AsNumber, Constructor, Representable))]
impl ParamSpec {
    #[pymethod]
    fn __mro_entries__(&self, _bases: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        Err(vm.new_type_error("Cannot subclass an instance of ParamSpec"))
    }

    #[pygetset]
    fn __name__(&self) -> PyObjectRef {
        self.name.clone()
    }

    #[pygetset]
    fn args(zelf: crate::PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        let self_obj: PyObjectRef = zelf.into();
        let psa = ParamSpecArgs {
            __origin__: self_obj,
        };
        Ok(psa.into_ref(&vm.ctx).into())
    }

    #[pygetset]
    fn kwargs(zelf: crate::PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        let self_obj: PyObjectRef = zelf.into();
        let psk = ParamSpecKwargs {
            __origin__: self_obj,
        };
        Ok(psk.into_ref(&vm.ctx).into())
    }

    #[pygetset]
    fn __bound__(&self, vm: &VirtualMachine) -> PyObjectRef {
        if let Some(bound) = self.bound.clone() {
            return bound;
        }
        vm.ctx.none()
    }

    #[pygetset]
    const fn __covariant__(&self) -> bool {
        self.covariant
    }

    #[pygetset]
    const fn __contravariant__(&self) -> bool {
        self.contravariant
    }

    #[pygetset]
    const fn __infer_variance__(&self) -> bool {
        self.infer_variance
    }

    #[pygetset]
    fn __default__(&self, vm: &VirtualMachine) -> PyResult {
        // Check if default_value is NoDefault (not just None)
        if !self.default_value.is(&vm.ctx.typing_no_default) {
            return Ok(self.default_value.clone());
        }
        // handle evaluate_default
        let evaluate_default = self.evaluate_default.lock();
        if !vm.is_none(&evaluate_default) {
            let default_value = evaluate_default.call((), vm)?;
            return Ok(default_value);
        }
        // Return NoDefault singleton
        Ok(vm.ctx.typing_no_default.clone().into())
    }

    #[pygetset]
    fn evaluate_default(&self, _vm: &VirtualMachine) -> PyObjectRef {
        self.evaluate_default.lock().clone()
    }

    #[pymethod]
    fn __reduce__(&self) -> PyResult {
        Ok(self.name.clone())
    }

    #[pymethod]
    fn has_default(&self, vm: &VirtualMachine) -> bool {
        if !vm.is_none(&self.evaluate_default.lock()) {
            return true;
        }
        // Check if default_value is not NoDefault
        !self.default_value.is(&vm.ctx.typing_no_default)
    }

    #[pymethod]
    fn __typing_subst__(
        zelf: crate::PyRef<Self>,
        arg: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult {
        let self_obj: PyObjectRef = zelf.into();
        _call_typing_func_object(vm, "_paramspec_subst", (self_obj, arg))
    }

    #[pymethod]
    fn __typing_prepare_subst__(
        zelf: crate::PyRef<Self>,
        alias: PyObjectRef,
        args: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult {
        let self_obj: PyObjectRef = zelf.into();
        _call_typing_func_object(vm, "_paramspec_prepare_subst", (self_obj, alias, args))
    }
}

impl AsNumber for ParamSpec {
    fn as_number() -> &'static PyNumberMethods {
        static AS_NUMBER: PyNumberMethods = PyNumberMethods {
            or: Some(|a, b, vm| {
                _call_typing_func_object(vm, "_make_union", (a.to_owned(), b.to_owned()))
            }),
            ..PyNumberMethods::NOT_IMPLEMENTED
        };
        &AS_NUMBER
    }
}

impl Constructor for ParamSpec {
    type Args = FuncArgs;

    fn py_new(cls: PyTypeRef, args: Self::Args, vm: &VirtualMachine) -> PyResult {
        let mut kwargs = args.kwargs;
        // Parse arguments manually
        let name = if args.args.is_empty() {
            // Check if name is provided as keyword argument
            if let Some(name) = kwargs.swap_remove("name") {
                name
            } else {
                return Err(
                    vm.new_type_error("ParamSpec() missing required argument: 'name' (pos 1)")
                );
            }
        } else if args.args.len() == 1 {
            args.args[0].clone()
        } else {
            return Err(vm.new_type_error("ParamSpec() takes at most 1 positional argument"));
        };

        let bound = kwargs.swap_remove("bound");
        let covariant = kwargs
            .swap_remove("covariant")
            .map(|v| v.try_to_bool(vm))
            .transpose()?
            .unwrap_or(false);
        let contravariant = kwargs
            .swap_remove("contravariant")
            .map(|v| v.try_to_bool(vm))
            .transpose()?
            .unwrap_or(false);
        let infer_variance = kwargs
            .swap_remove("infer_variance")
            .map(|v| v.try_to_bool(vm))
            .transpose()?
            .unwrap_or(false);
        let default = kwargs.swap_remove("default");

        // Check for unexpected keyword arguments
        if !kwargs.is_empty() {
            let unexpected_keys: Vec<String> = kwargs.keys().map(|s| s.to_string()).collect();
            return Err(vm.new_type_error(format!(
                "ParamSpec() got unexpected keyword argument(s): {}",
                unexpected_keys.join(", ")
            )));
        }

        // Check for invalid combinations
        if covariant && contravariant {
            return Err(vm.new_value_error("Bivariant type variables are not supported."));
        }

        if infer_variance && (covariant || contravariant) {
            return Err(vm.new_value_error("Variance cannot be specified with infer_variance"));
        }

        // Handle default value
        let default_value = default.unwrap_or_else(|| vm.ctx.typing_no_default.clone().into());

        let paramspec = Self {
            name,
            bound,
            default_value,
            evaluate_default: PyMutex::new(vm.ctx.none()),
            covariant,
            contravariant,
            infer_variance,
        };

        let obj = paramspec.into_ref_with_type(vm, cls)?;
        let obj_ref: PyObjectRef = obj.into();
        set_module_from_caller(&obj_ref, vm)?;
        Ok(obj_ref)
    }
}

impl Representable for ParamSpec {
    #[inline(always)]
    fn repr_str(zelf: &crate::Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
        let name = zelf.__name__().str(vm)?;
        Ok(format!("~{name}"))
    }
}

impl ParamSpec {
    pub fn new(name: PyObjectRef, vm: &VirtualMachine) -> Self {
        Self {
            name,
            bound: None,
            default_value: vm.ctx.typing_no_default.clone().into(),
            evaluate_default: PyMutex::new(vm.ctx.none()),
            covariant: false,
            contravariant: false,
            infer_variance: false,
        }
    }
}

#[pyclass(name = "TypeVarTuple", module = "typing")]
#[derive(Debug, PyPayload)]
#[allow(dead_code)]
pub struct TypeVarTuple {
    name: PyObjectRef,
    default_value: parking_lot::Mutex<PyObjectRef>,
    evaluate_default: PyMutex<PyObjectRef>,
}
#[pyclass(flags(HAS_DICT), with(Constructor, Representable, Iterable))]
impl TypeVarTuple {
    #[pygetset]
    fn __name__(&self) -> PyObjectRef {
        self.name.clone()
    }

    #[pygetset]
    fn __default__(&self, vm: &VirtualMachine) -> PyResult {
        let mut default_value = self.default_value.lock();
        // Check if default_value is NoDefault (not just None)
        if !default_value.is(&vm.ctx.typing_no_default) {
            return Ok(default_value.clone());
        }
        let evaluate_default = self.evaluate_default.lock();
        if !vm.is_none(&evaluate_default) {
            *default_value = evaluate_default.call((), vm)?;
            Ok(default_value.clone())
        } else {
            // Return NoDefault singleton
            Ok(vm.ctx.typing_no_default.clone().into())
        }
    }

    #[pymethod]
    fn has_default(&self, vm: &VirtualMachine) -> bool {
        if !vm.is_none(&self.evaluate_default.lock()) {
            return true;
        }
        let default_value = self.default_value.lock();
        // Check if default_value is not NoDefault
        !default_value.is(&vm.ctx.typing_no_default)
    }

    #[pymethod]
    fn __reduce__(&self) -> PyObjectRef {
        self.name.clone()
    }

    #[pymethod]
    fn __mro_entries__(&self, _bases: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        Err(vm.new_type_error("Cannot subclass an instance of TypeVarTuple"))
    }

    #[pymethod]
    fn __typing_subst__(&self, _arg: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        Err(vm.new_type_error("Substitution of bare TypeVarTuple is not supported"))
    }

    #[pymethod]
    fn __typing_prepare_subst__(
        zelf: crate::PyRef<Self>,
        alias: PyObjectRef,
        args: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult {
        let self_obj: PyObjectRef = zelf.into();
        _call_typing_func_object(vm, "_typevartuple_prepare_subst", (self_obj, alias, args))
    }
}

impl Iterable for TypeVarTuple {
    fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
        // When unpacking TypeVarTuple with *, return [Unpack[self]]
        // This is how CPython handles Generic[*Ts]
        let typing = vm.import("typing", 0)?;
        let unpack = typing.get_attr("Unpack", vm)?;
        let zelf_obj: PyObjectRef = zelf.into();
        let unpacked = vm.call_method(&unpack, "__getitem__", (zelf_obj,))?;
        let list = vm.ctx.new_list(vec![unpacked]);
        let list_obj: PyObjectRef = list.into();
        vm.call_method(&list_obj, "__iter__", ())
    }
}

impl Constructor for TypeVarTuple {
    type Args = FuncArgs;

    fn py_new(cls: PyTypeRef, args: Self::Args, vm: &VirtualMachine) -> PyResult {
        let mut kwargs = args.kwargs;
        // Parse arguments manually
        let name = if args.args.is_empty() {
            // Check if name is provided as keyword argument
            if let Some(name) = kwargs.swap_remove("name") {
                name
            } else {
                return Err(
                    vm.new_type_error("TypeVarTuple() missing required argument: 'name' (pos 1)")
                );
            }
        } else if args.args.len() == 1 {
            args.args[0].clone()
        } else {
            return Err(vm.new_type_error("TypeVarTuple() takes at most 1 positional argument"));
        };

        let default = kwargs.swap_remove("default");

        // Check for unexpected keyword arguments
        if !kwargs.is_empty() {
            let unexpected_keys: Vec<String> = kwargs.keys().map(|s| s.to_string()).collect();
            return Err(vm.new_type_error(format!(
                "TypeVarTuple() got unexpected keyword argument(s): {}",
                unexpected_keys.join(", ")
            )));
        }

        // Handle default value
        let (default_value, evaluate_default) = if let Some(default) = default {
            (default, vm.ctx.none())
        } else {
            // If no default provided, use NoDefault singleton
            (vm.ctx.typing_no_default.clone().into(), vm.ctx.none())
        };

        let typevartuple = Self {
            name,
            default_value: parking_lot::Mutex::new(default_value),
            evaluate_default: PyMutex::new(evaluate_default),
        };

        let obj = typevartuple.into_ref_with_type(vm, cls)?;
        let obj_ref: PyObjectRef = obj.into();
        set_module_from_caller(&obj_ref, vm)?;
        Ok(obj_ref)
    }
}

impl Representable for TypeVarTuple {
    #[inline(always)]
    fn repr_str(zelf: &crate::Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
        let name = zelf.name.str(vm)?;
        Ok(name.to_string())
    }
}

impl TypeVarTuple {
    pub fn new(name: PyObjectRef, vm: &VirtualMachine) -> Self {
        Self {
            name,
            default_value: parking_lot::Mutex::new(vm.ctx.typing_no_default.clone().into()),
            evaluate_default: PyMutex::new(vm.ctx.none()),
        }
    }
}

#[pyclass(name = "ParamSpecArgs", module = "typing")]
#[derive(Debug, PyPayload)]
#[allow(dead_code)]
pub struct ParamSpecArgs {
    __origin__: PyObjectRef,
}
#[pyclass(with(Constructor, Representable, Comparable))]
impl ParamSpecArgs {
    #[pymethod]
    fn __mro_entries__(&self, _bases: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        Err(vm.new_type_error("Cannot subclass an instance of ParamSpecArgs"))
    }

    #[pygetset]
    fn __origin__(&self) -> PyObjectRef {
        self.__origin__.clone()
    }
}

impl Constructor for ParamSpecArgs {
    type Args = (PyObjectRef,);

    fn py_new(cls: PyTypeRef, args: Self::Args, vm: &VirtualMachine) -> PyResult {
        let origin = args.0;
        let psa = Self { __origin__: origin };
        psa.into_ref_with_type(vm, cls).map(Into::into)
    }
}

impl Representable for ParamSpecArgs {
    #[inline(always)]
    fn repr_str(zelf: &crate::Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
        // Check if origin is a ParamSpec
        if let Ok(name) = zelf.__origin__.get_attr("__name__", vm) {
            return Ok(format!("{name}.args", name = name.str(vm)?));
        }
        Ok(format!("{:?}.args", zelf.__origin__))
    }
}

impl Comparable for ParamSpecArgs {
    fn cmp(
        zelf: &crate::Py<Self>,
        other: &PyObject,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        fn eq(
            zelf: &crate::Py<ParamSpecArgs>,
            other: PyObjectRef,
            _vm: &VirtualMachine,
        ) -> PyResult<bool> {
            // First check if other is also ParamSpecArgs
            if let Ok(other_args) = other.downcast::<ParamSpecArgs>() {
                // Check if they have the same origin
                return Ok(zelf.__origin__.is(&other_args.__origin__));
            }
            Ok(false)
        }
        match op {
            PyComparisonOp::Eq => {
                if let Ok(result) = eq(zelf, other.to_owned(), vm) {
                    Ok(result.into())
                } else {
                    Ok(PyComparisonValue::NotImplemented)
                }
            }
            PyComparisonOp::Ne => {
                if let Ok(result) = eq(zelf, other.to_owned(), vm) {
                    Ok((!result).into())
                } else {
                    Ok(PyComparisonValue::NotImplemented)
                }
            }
            _ => Ok(PyComparisonValue::NotImplemented),
        }
    }
}

#[pyclass(name = "ParamSpecKwargs", module = "typing")]
#[derive(Debug, PyPayload)]
#[allow(dead_code)]
pub struct ParamSpecKwargs {
    __origin__: PyObjectRef,
}
#[pyclass(with(Constructor, Representable, Comparable))]
impl ParamSpecKwargs {
    #[pymethod]
    fn __mro_entries__(&self, _bases: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        Err(vm.new_type_error("Cannot subclass an instance of ParamSpecKwargs"))
    }

    #[pygetset]
    fn __origin__(&self) -> PyObjectRef {
        self.__origin__.clone()
    }
}

impl Constructor for ParamSpecKwargs {
    type Args = (PyObjectRef,);

    fn py_new(cls: PyTypeRef, args: Self::Args, vm: &VirtualMachine) -> PyResult {
        let origin = args.0;
        let psa = Self { __origin__: origin };
        psa.into_ref_with_type(vm, cls).map(Into::into)
    }
}

impl Representable for ParamSpecKwargs {
    #[inline(always)]
    fn repr_str(zelf: &crate::Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
        // Check if origin is a ParamSpec
        if let Ok(name) = zelf.__origin__.get_attr("__name__", vm) {
            return Ok(format!("{name}.kwargs", name = name.str(vm)?));
        }
        Ok(format!("{:?}.kwargs", zelf.__origin__))
    }
}

impl Comparable for ParamSpecKwargs {
    fn cmp(
        zelf: &crate::Py<Self>,
        other: &PyObject,
        op: PyComparisonOp,
        vm: &VirtualMachine,
    ) -> PyResult<PyComparisonValue> {
        fn eq(
            zelf: &crate::Py<ParamSpecKwargs>,
            other: PyObjectRef,
            _vm: &VirtualMachine,
        ) -> PyResult<bool> {
            // First check if other is also ParamSpecKwargs
            if let Ok(other_kwargs) = other.downcast::<ParamSpecKwargs>() {
                // Check if they have the same origin
                return Ok(zelf.__origin__.is(&other_kwargs.__origin__));
            }
            Ok(false)
        }
        match op {
            PyComparisonOp::Eq => {
                if let Ok(result) = eq(zelf, other.to_owned(), vm) {
                    Ok(result.into())
                } else {
                    Ok(PyComparisonValue::NotImplemented)
                }
            }
            PyComparisonOp::Ne => {
                if let Ok(result) = eq(zelf, other.to_owned(), vm) {
                    Ok((!result).into())
                } else {
                    Ok(PyComparisonValue::NotImplemented)
                }
            }
            _ => Ok(PyComparisonValue::NotImplemented),
        }
    }
}

/// Helper function to call typing module functions with cls as first argument
/// Similar to CPython's call_typing_args_kwargs
fn call_typing_args_kwargs(
    name: &'static str,
    cls: PyTypeRef,
    args: FuncArgs,
    vm: &VirtualMachine,
) -> PyResult {
    let typing = vm.import("typing", 0)?;
    let func = typing.get_attr(name, vm)?;

    // Prepare arguments: (cls, *args)
    let mut call_args = vec![cls.into()];
    call_args.extend(args.args);

    // Call with prepared args and original kwargs
    let func_args = FuncArgs {
        args: call_args,
        kwargs: args.kwargs,
    };

    func.call(func_args, vm)
}

#[pyclass(name = "Generic", module = "typing")]
#[derive(Debug, PyPayload)]
#[allow(dead_code)]
pub struct Generic {}

#[pyclass(flags(BASETYPE))]
impl Generic {
    #[pyclassmethod]
    fn __class_getitem__(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        call_typing_args_kwargs("_generic_class_getitem", cls, args, vm)
    }

    #[pyclassmethod]
    fn __init_subclass__(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        call_typing_args_kwargs("_generic_init_subclass", cls, args, vm)
    }
}

/// Sets the default value for a type parameter, equivalent to CPython's _Py_set_typeparam_default
/// This is used by the CALL_INTRINSIC_2 SetTypeparamDefault instruction
pub fn set_typeparam_default(
    type_param: PyObjectRef,
    evaluate_default: PyObjectRef,
    vm: &VirtualMachine,
) -> PyResult {
    // Inner function to handle common pattern of setting evaluate_default
    fn try_set_default<T>(
        obj: &PyObjectRef,
        evaluate_default: &PyObjectRef,
        get_field: impl FnOnce(&T) -> &PyMutex<PyObjectRef>,
    ) -> bool
    where
        T: PyPayload,
    {
        if let Some(typed_obj) = obj.downcast_ref::<T>() {
            *get_field(typed_obj).lock() = evaluate_default.clone();
            true
        } else {
            false
        }
    }

    // Try each type parameter type
    if try_set_default::<TypeVar>(&type_param, &evaluate_default, |tv| &tv.evaluate_default)
        || try_set_default::<ParamSpec>(&type_param, &evaluate_default, |ps| &ps.evaluate_default)
        || try_set_default::<TypeVarTuple>(&type_param, &evaluate_default, |tvt| {
            &tvt.evaluate_default
        })
    {
        Ok(type_param)
    } else {
        Err(vm.new_type_error(format!(
            "Expected a type param, got {}",
            type_param.class().name()
        )))
    }
}
