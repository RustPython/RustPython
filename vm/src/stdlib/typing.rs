use crate::{PyRef, VirtualMachine, stdlib::PyModule};

pub(crate) use _typing::NoDefault;

pub(crate) fn make_module(vm: &VirtualMachine) -> PyRef<PyModule> {
    let module = _typing::make_module(vm);
    extend_module!(vm, &module, {
        "NoDefault" => vm.ctx.typing_no_default.clone(),
    });
    module
}

#[pymodule]
pub(crate) mod _typing {
    use crate::{
        AsObject, PyObjectRef, PyPayload, PyResult, VirtualMachine,
        builtins::{PyGenericAlias, PyTupleRef, PyTypeRef, pystr::AsPyStr},
        function::{FuncArgs, IntoFuncArgs},
        types::{Constructor, Representable},
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

    #[pyfunction]
    pub(crate) fn _idfunc(args: FuncArgs, _vm: &VirtualMachine) -> PyObjectRef {
        args.args[0].clone()
    }

    #[pyattr]
    #[pyclass(name = "TypeVar", module = "typing")]
    #[derive(Debug, PyPayload)]
    #[allow(dead_code)]
    pub(crate) struct TypeVar {
        name: PyObjectRef, // TODO PyStrRef?
        bound: parking_lot::Mutex<PyObjectRef>,
        evaluate_bound: PyObjectRef,
        constraints: parking_lot::Mutex<PyObjectRef>,
        evaluate_constraints: PyObjectRef,
        default_value: parking_lot::Mutex<PyObjectRef>,
        evaluate_default: PyObjectRef,
        covariant: bool,
        contravariant: bool,
        infer_variance: bool,
    }
    #[pyclass(flags(HAS_DICT), with(Constructor, Representable))]
    impl TypeVar {
        #[pymethod(magic)]
        fn mro_entries(&self, _bases: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            Err(vm.new_type_error("Cannot subclass an instance of TypeVar"))
        }

        #[pygetset(magic)]
        fn name(&self) -> PyObjectRef {
            self.name.clone()
        }

        #[pygetset(magic)]
        fn constraints(&self, vm: &VirtualMachine) -> PyResult {
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

        #[pygetset(magic)]
        fn bound(&self, vm: &VirtualMachine) -> PyResult {
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

        #[pygetset(magic)]
        fn covariant(&self) -> bool {
            self.covariant
        }

        #[pygetset(magic)]
        fn contravariant(&self) -> bool {
            self.contravariant
        }

        #[pygetset(magic)]
        fn infer_variance(&self) -> bool {
            self.infer_variance
        }

        #[pygetset(magic)]
        fn default(&self, vm: &VirtualMachine) -> PyResult {
            let mut default_value = self.default_value.lock();
            // Check if default_value is NoDefault (not just None)
            if !default_value.is(&vm.ctx.typing_no_default) {
                return Ok(default_value.clone());
            }
            if !vm.is_none(&self.evaluate_default) {
                *default_value = self.evaluate_default.call((), vm)?;
                Ok(default_value.clone())
            } else {
                // Return NoDefault singleton
                Ok(vm.ctx.typing_no_default.clone().into())
            }
        }

        #[pymethod(magic)]
        fn typing_subst(
            zelf: crate::PyRef<Self>,
            arg: PyObjectRef,
            vm: &VirtualMachine,
        ) -> PyResult {
            let self_obj: PyObjectRef = zelf.into();
            _call_typing_func_object(vm, "_typevar_subst", (self_obj, arg))
        }

        #[pymethod(magic)]
        fn reduce(&self) -> PyObjectRef {
            self.name.clone()
        }

        #[pymethod]
        fn has_default(&self, vm: &VirtualMachine) -> bool {
            if !vm.is_none(&self.evaluate_default) {
                return true;
            }
            let default_value = self.default_value.lock();
            // Check if default_value is not NoDefault
            !default_value.is(&vm.ctx.typing_no_default)
        }
    }

    impl Representable for TypeVar {
        #[inline(always)]
        fn repr_str(zelf: &crate::Py<Self>, vm: &VirtualMachine) -> PyResult<String> {
            let name = zelf.name.str(vm)?;
            let repr = if zelf.covariant {
                format!("+{}", name)
            } else if zelf.contravariant {
                format!("-{}", name)
            } else {
                format!("~{}", name)
            };
            Ok(repr)
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

            let typevar = TypeVar {
                name,
                bound: parking_lot::Mutex::new(bound_obj),
                evaluate_bound,
                constraints: parking_lot::Mutex::new(constraints_obj),
                evaluate_constraints,
                default_value: parking_lot::Mutex::new(default_value),
                evaluate_default,
                covariant,
                contravariant,
                infer_variance,
            };

            let obj = typevar.into_ref_with_type(vm, cls)?;
            let obj_ref: PyObjectRef = obj.into();

            // Set __module__ from the current frame
            if let Some(frame) = vm.current_frame() {
                if let Ok(module_name) = frame.globals.get_item("__name__", vm) {
                    // Don't set __module__ if it's None
                    if !vm.is_none(&module_name) {
                        obj_ref.set_attr("__module__", module_name, vm)?;
                    }
                }
            }

            Ok(obj_ref)
        }
    }

    pub(crate) fn make_typevar(
        vm: &VirtualMachine,
        name: PyObjectRef,
        evaluate_bound: PyObjectRef,
        evaluate_constraints: PyObjectRef,
    ) -> TypeVar {
        TypeVar {
            name,
            bound: parking_lot::Mutex::new(vm.ctx.none()),
            evaluate_bound,
            constraints: parking_lot::Mutex::new(vm.ctx.none()),
            evaluate_constraints,
            default_value: parking_lot::Mutex::new(vm.ctx.none()),
            evaluate_default: vm.ctx.none(),
            covariant: false,
            contravariant: false,
            infer_variance: false,
        }
    }

    #[pyattr]
    #[pyclass(name = "ParamSpec")]
    #[derive(Debug, PyPayload)]
    #[allow(dead_code)]
    pub(crate) struct ParamSpec {
        name: PyObjectRef,
        bound: Option<PyObjectRef>,
        default_value: Option<PyObjectRef>,
        evaluate_default: Option<PyObjectRef>,
        covariant: bool,
        contravariant: bool,
        infer_variance: bool,
    }

    #[pyclass(flags(BASETYPE))]
    impl ParamSpec {
        #[pygetset(magic)]
        fn name(&self) -> PyObjectRef {
            self.name.clone()
        }

        #[pygetset(magic)]
        fn bound(&self, vm: &VirtualMachine) -> PyObjectRef {
            if let Some(bound) = self.bound.clone() {
                return bound;
            }
            vm.ctx.none()
        }

        #[pygetset(magic)]
        fn covariant(&self) -> bool {
            self.covariant
        }

        #[pygetset(magic)]
        fn contravariant(&self) -> bool {
            self.contravariant
        }

        #[pygetset(magic)]
        fn infer_variance(&self) -> bool {
            self.infer_variance
        }

        #[pygetset(magic)]
        fn default(&self, vm: &VirtualMachine) -> PyResult {
            if let Some(default_value) = self.default_value.clone() {
                return Ok(default_value);
            }
            // handle evaluate_default
            if let Some(evaluate_default) = self.evaluate_default.clone() {
                let default_value = vm.call_method(evaluate_default.as_ref(), "__call__", ())?;
                return Ok(default_value);
            }
            // TODO: this isn't up to spec
            Ok(vm.ctx.none())
        }

        #[pygetset]
        fn evaluate_default(&self, vm: &VirtualMachine) -> PyObjectRef {
            if let Some(evaluate_default) = self.evaluate_default.clone() {
                return evaluate_default;
            }
            // TODO: default_value case
            vm.ctx.none()
        }

        #[pymethod(magic)]
        fn reduce(&self) -> PyResult {
            Ok(self.name.clone())
        }

        #[pymethod]
        fn has_default(&self) -> PyResult<bool> {
            // TODO: fix
            Ok(self.evaluate_default.is_some() || self.default_value.is_some())
        }
    }

    pub(crate) fn make_paramspec(name: PyObjectRef) -> ParamSpec {
        ParamSpec {
            name,
            bound: None,
            default_value: None,
            evaluate_default: None,
            covariant: false,
            contravariant: false,
            infer_variance: false,
        }
    }

    #[pyclass(no_attr, name = "NoDefaultType", module = "typing")]
    #[derive(Debug, PyPayload)]
    pub struct NoDefault;

    #[pyclass(with(Constructor), flags(BASETYPE))]
    impl NoDefault {
        #[pymethod(magic)]
        fn reduce(&self, _vm: &VirtualMachine) -> String {
            "NoDefault".to_owned()
        }
    }

    impl Constructor for NoDefault {
        type Args = FuncArgs;

        fn py_new(_cls: PyTypeRef, args: Self::Args, vm: &VirtualMachine) -> PyResult {
            if !args.args.is_empty() || !args.kwargs.is_empty() {
                return Err(vm.new_type_error("NoDefaultType takes no arguments"));
            }

            // Return singleton instance from context
            Ok(vm.ctx.typing_no_default.clone().into())
        }
    }

    impl Representable for NoDefault {
        #[inline(always)]
        fn repr_str(_zelf: &crate::Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            Ok("typing.NoDefault".to_owned())
        }
    }

    #[pyattr]
    #[pyclass(name = "TypeVarTuple")]
    #[derive(Debug, PyPayload)]
    #[allow(dead_code)]
    pub(crate) struct TypeVarTuple {
        name: PyObjectRef,
    }
    #[pyclass(flags(BASETYPE))]
    impl TypeVarTuple {}

    pub(crate) fn make_typevartuple(name: PyObjectRef) -> TypeVarTuple {
        TypeVarTuple { name }
    }

    #[pyattr]
    #[pyclass(name = "ParamSpecArgs")]
    #[derive(Debug, PyPayload)]
    #[allow(dead_code)]
    pub(crate) struct ParamSpecArgs {}
    #[pyclass(flags(BASETYPE))]
    impl ParamSpecArgs {}

    #[pyattr]
    #[pyclass(name = "ParamSpecKwargs")]
    #[derive(Debug, PyPayload)]
    #[allow(dead_code)]
    pub(crate) struct ParamSpecKwargs {}
    #[pyclass(flags(BASETYPE))]
    impl ParamSpecKwargs {}

    #[pyattr]
    #[pyclass(name)]
    #[derive(Debug, PyPayload)]
    #[allow(dead_code)]
    pub(crate) struct TypeAliasType {
        name: PyObjectRef, // TODO PyStrRef?
        type_params: PyTupleRef,
        value: PyObjectRef,
        // compute_value: PyObjectRef,
        // module: PyObjectRef,
    }
    #[pyclass(flags(BASETYPE))]
    impl TypeAliasType {
        pub fn new(
            name: PyObjectRef,
            type_params: PyTupleRef,
            value: PyObjectRef,
        ) -> TypeAliasType {
            TypeAliasType {
                name,
                type_params,
                value,
            }
        }
    }

    #[pyattr]
    #[pyclass(name)]
    #[derive(Debug, PyPayload)]
    #[allow(dead_code)]
    pub(crate) struct Generic {}

    // #[pyclass(with(AsMapping), flags(BASETYPE))]
    #[pyclass(flags(BASETYPE))]
    impl Generic {
        #[pyclassmethod(magic)]
        fn class_getitem(cls: PyTypeRef, args: PyObjectRef, vm: &VirtualMachine) -> PyGenericAlias {
            PyGenericAlias::new(cls, args, vm)
        }
    }

    // impl AsMapping for Generic {
    //     fn as_mapping() -> &'static PyMappingMethods {
    //         static AS_MAPPING: Lazy<PyMappingMethods> = Lazy::new(|| PyMappingMethods {
    //             subscript: atomic_func!(|mapping, needle, vm| {
    //                 call_typing_func_object(vm, "_GenericAlias", (mapping.obj, needle))
    //             }),
    //             ..PyMappingMethods::NOT_IMPLEMENTED
    //         });
    //         &AS_MAPPING
    //     }
    // }
}
