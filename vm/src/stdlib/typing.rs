pub(crate) use _typing::make_module;

#[pymodule]
pub(crate) mod _typing {
    use crate::{
        PyObjectRef, PyPayload, PyResult, VirtualMachine,
        builtins::{PyGenericAlias, PyTupleRef, PyTypeRef, pystr::AsPyStr},
        function::{FuncArgs, IntoFuncArgs},
    };

    pub(crate) fn _call_typing_func_object<'a>(
        _vm: &VirtualMachine,
        _func_name: impl AsPyStr<'a>,
        _args: impl IntoFuncArgs,
    ) -> PyResult {
        todo!("does this work????");
        // let module = vm.import("typing", 0)?;
        // let module = vm.import("_pycodecs", None, 0)?;
        // let func = module.get_attr(func_name, vm)?;
        // func.call(args, vm)
    }

    #[pyfunction]
    pub(crate) fn _idfunc(args: FuncArgs, _vm: &VirtualMachine) -> PyObjectRef {
        args.args[0].clone()
    }

    #[pyattr]
    #[pyclass(name = "TypeVar")]
    #[derive(Debug, PyPayload)]
    #[allow(dead_code)]
    pub(crate) struct TypeVar {
        name: PyObjectRef, // TODO PyStrRef?
        bound: parking_lot::Mutex<PyObjectRef>,
        evaluate_bound: PyObjectRef,
        constraints: parking_lot::Mutex<PyObjectRef>,
        evaluate_constraints: PyObjectRef,
        covariant: bool,
        contravariant: bool,
        infer_variance: bool,
    }
    #[pyclass(flags(BASETYPE))]
    impl TypeVar {
        pub(crate) fn _bound(&self, vm: &VirtualMachine) -> PyResult {
            let mut bound = self.bound.lock();
            if !vm.is_none(&bound) {
                return Ok(bound.clone());
            }
            if !vm.is_none(&self.evaluate_bound) {
                *bound = self.evaluate_bound.call((), vm)?;
                Ok(bound.clone())
            } else {
                Ok(vm.ctx.none())
            }
        }

        #[pygetset(magic)]
        fn name(&self) -> PyObjectRef {
            self.name.clone()
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
            covariant: false,
            contravariant: false,
            infer_variance: true,
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

    #[pyattr]
    #[pyclass(name = "NoDefault")]
    #[derive(Debug, PyPayload)]
    #[allow(dead_code)]
    pub(crate) struct NoDefault {
        name: PyObjectRef,
    }

    #[pyclass(flags(BASETYPE))]
    impl NoDefault {}

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
