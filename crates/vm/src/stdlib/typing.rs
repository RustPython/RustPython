// spell-checker:ignore typevarobject funcobj
use crate::{
    Context, PyResult, VirtualMachine, builtins::pystr::AsPyStr, class::PyClassImpl,
    function::IntoFuncArgs,
};

pub use crate::stdlib::typevar::{
    Generic, ParamSpec, ParamSpecArgs, ParamSpecKwargs, TypeVar, TypeVarTuple,
    set_typeparam_default,
};
pub(crate) use decl::module_def;
pub use decl::*;

/// Initialize typing types (call extend_class)
pub fn init(ctx: &Context) {
    NoDefault::extend_class(ctx, ctx.types.typing_no_default_type);
}

pub fn call_typing_func_object<'a>(
    vm: &VirtualMachine,
    func_name: impl AsPyStr<'a>,
    args: impl IntoFuncArgs,
) -> PyResult {
    let module = vm.import("typing", 0)?;
    let func = module.get_attr(func_name.as_pystr(&vm.ctx), vm)?;
    func.call(args, vm)
}

#[pymodule(name = "_typing", with(super::typevar::typevar))]
pub(crate) mod decl {
    use crate::{
        AsObject, Py, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine, atomic_func,
        builtins::{PyGenericAlias, PyStrRef, PyTuple, PyTupleRef, PyType, PyTypeRef, type_},
        function::FuncArgs,
        protocol::{PyMappingMethods, PyNumberMethods},
        types::{AsMapping, AsNumber, Constructor, Hashable, Iterable, Representable},
    };
    use std::sync::LazyLock;

    #[pyfunction]
    pub(crate) fn _idfunc(args: FuncArgs, _vm: &VirtualMachine) -> PyObjectRef {
        args.args[0].clone()
    }

    #[pyfunction(name = "override")]
    pub(crate) fn r#override(func: PyObjectRef, vm: &VirtualMachine) -> PyResult {
        // Set __override__ attribute to True
        // Skip the attribute silently if it is not writable.
        // AttributeError happens if the object has __slots__ or a
        // read-only property, TypeError if it's a builtin class.
        let _ = func.set_attr("__override__", vm.ctx.true_value.clone(), vm);
        Ok(func)
    }

    #[pyclass(no_attr, name = "NoDefaultType", module = "typing")]
    #[derive(Debug, PyPayload)]
    pub struct NoDefault;

    #[pyclass(with(Constructor, Representable), flags(BASETYPE))]
    impl NoDefault {
        #[pymethod]
        fn __reduce__(&self, _vm: &VirtualMachine) -> String {
            "NoDefault".to_owned()
        }
    }

    impl Constructor for NoDefault {
        type Args = ();

        fn slot_new(_cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            let _: () = args.bind(vm)?;
            Ok(vm.ctx.typing_no_default.clone().into())
        }

        fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
            unreachable!("NoDefault is a singleton, use slot_new")
        }
    }

    impl Representable for NoDefault {
        #[inline(always)]
        fn repr_str(_zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            Ok("typing.NoDefault".to_owned())
        }
    }

    #[pyattr]
    #[pyclass(name, module = "typing")]
    #[derive(Debug, PyPayload)]
    pub(crate) struct TypeAliasType {
        name: PyStrRef,
        type_params: PyTupleRef,
        compute_value: PyObjectRef,
        cached_value: crate::common::lock::PyMutex<Option<PyObjectRef>>,
        module: Option<PyObjectRef>,
    }
    #[pyclass(
        with(Constructor, Representable, Hashable, AsMapping, AsNumber, Iterable),
        flags(IMMUTABLETYPE)
    )]
    impl TypeAliasType {
        /// Create from intrinsic: compute_value is a callable that returns the value
        pub fn new(name: PyStrRef, type_params: PyTupleRef, compute_value: PyObjectRef) -> Self {
            Self {
                name,
                type_params,
                compute_value,
                cached_value: crate::common::lock::PyMutex::new(None),
                module: None,
            }
        }

        /// Create with an eagerly evaluated value (used by constructor)
        fn new_eager(
            name: PyStrRef,
            type_params: PyTupleRef,
            value: PyObjectRef,
            module: Option<PyObjectRef>,
        ) -> Self {
            Self {
                name,
                type_params,
                compute_value: value.clone(),
                cached_value: crate::common::lock::PyMutex::new(Some(value)),
                module,
            }
        }

        #[pygetset]
        fn __name__(&self) -> PyObjectRef {
            self.name.clone().into()
        }

        #[pygetset]
        fn __value__(&self, vm: &VirtualMachine) -> PyResult {
            let cached = self.cached_value.lock().clone();
            if let Some(value) = cached {
                return Ok(value);
            }
            let value = self.compute_value.call((), vm)?;
            *self.cached_value.lock() = Some(value.clone());
            Ok(value)
        }

        #[pygetset]
        fn __type_params__(&self) -> PyTupleRef {
            self.type_params.clone()
        }

        #[pygetset]
        fn __parameters__(&self, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
            // TypeVarTuples must be unpacked in __parameters__
            unpack_typevartuples(&self.type_params, vm).map(|t| t.into())
        }

        #[pygetset]
        fn __module__(&self, vm: &VirtualMachine) -> PyObjectRef {
            if let Some(ref module) = self.module {
                return module.clone();
            }
            // Fall back to compute_value's __module__ (like PyFunction_GetModule)
            if let Ok(module) = self.compute_value.get_attr("__module__", vm) {
                return module;
            }
            vm.ctx.none()
        }

        fn __getitem__(zelf: PyRef<Self>, args: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            if zelf.type_params.is_empty() {
                return Err(
                    vm.new_type_error("Only generic type aliases are subscriptable".to_owned())
                );
            }
            let args_tuple = if let Ok(tuple) = args.try_to_ref::<PyTuple>(vm) {
                tuple.to_owned()
            } else {
                PyTuple::new_ref(vec![args], &vm.ctx)
            };
            let origin: PyObjectRef = zelf.as_object().to_owned();
            Ok(PyGenericAlias::new(origin, args_tuple, false, vm).into_pyobject(vm))
        }

        #[pymethod]
        fn __reduce__(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyObjectRef {
            zelf.name.clone().into()
        }

        #[pymethod]
        fn __typing_unpacked_tuple_args__(&self, vm: &VirtualMachine) -> PyObjectRef {
            vm.ctx.none()
        }

        /// Returns the evaluator for the alias value.
        /// If compute_value is callable (lazy function from compiler), return it.
        /// Otherwise wrap the eagerly-set value in a ConstEvaluator-like callable.
        #[pygetset]
        fn evaluate_value(&self, vm: &VirtualMachine) -> PyResult {
            if self.compute_value.is_callable() {
                return Ok(self.compute_value.clone());
            }
            // For eagerly-evaluated aliases (from constructor), create a callable
            // that returns the value regardless of format argument
            let value = self.compute_value.clone();
            Ok(vm
                .new_function("_ConstEvaluator", move |_args: FuncArgs| -> PyResult {
                    Ok(value.clone())
                })
                .into())
        }

        /// Check type_params ordering: non-default params must precede default params.
        /// Uses __default__ attribute to check if a type param has a default value,
        /// comparing against typing.NoDefault sentinel (like get_type_param_default).
        fn check_type_params(
            type_params: &PyTupleRef,
            vm: &VirtualMachine,
        ) -> PyResult<Option<PyTupleRef>> {
            if type_params.is_empty() {
                return Ok(None);
            }
            let no_default = &vm.ctx.typing_no_default;
            let mut default_seen = false;
            for param in type_params.iter() {
                let dflt = param.get_attr("__default__", vm).map_err(|_| {
                    vm.new_type_error(format!(
                        "Expected a type param, got {}",
                        param
                            .repr(vm)
                            .map(|s| s.to_string())
                            .unwrap_or_else(|_| "?".to_owned())
                    ))
                })?;
                let is_no_default = dflt.is(no_default);
                if is_no_default {
                    if default_seen {
                        return Err(vm.new_type_error(format!(
                            "non-default type parameter '{}' follows default type parameter",
                            param.repr(vm)?
                        )));
                    }
                } else {
                    default_seen = true;
                }
            }
            Ok(Some(type_params.clone()))
        }
    }

    impl Constructor for TypeAliasType {
        type Args = FuncArgs;

        fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            // typealias(name, value, *, type_params=())
            // name and value are positional-or-keyword; type_params is keyword-only.

            // Reject unexpected keyword arguments
            for key in args.kwargs.keys() {
                if key != "name" && key != "value" && key != "type_params" {
                    return Err(vm.new_type_error(format!(
                        "typealias() got an unexpected keyword argument '{key}'"
                    )));
                }
            }

            // Reject too many positional arguments
            if args.args.len() > 2 {
                return Err(vm.new_type_error(format!(
                    "typealias() takes exactly 2 positional arguments ({} given)",
                    args.args.len()
                )));
            }

            // Resolve name: positional[0] or kwarg
            let name = if !args.args.is_empty() {
                if args.kwargs.contains_key("name") {
                    return Err(vm.new_type_error(
                        "argument for typealias() given by name ('name') and position (1)"
                            .to_owned(),
                    ));
                }
                args.args[0].clone()
            } else {
                args.kwargs.get("name").cloned().ok_or_else(|| {
                    vm.new_type_error(
                        "typealias() missing required argument 'name' (pos 1)".to_owned(),
                    )
                })?
            };

            // Resolve value: positional[1] or kwarg
            let value = if args.args.len() >= 2 {
                if args.kwargs.contains_key("value") {
                    return Err(vm.new_type_error(
                        "argument for typealias() given by name ('value') and position (2)"
                            .to_owned(),
                    ));
                }
                args.args[1].clone()
            } else {
                args.kwargs.get("value").cloned().ok_or_else(|| {
                    vm.new_type_error(
                        "typealias() missing required argument 'value' (pos 2)".to_owned(),
                    )
                })?
            };

            let name = name.downcast::<crate::builtins::PyStr>().map_err(|_| {
                vm.new_type_error("typealias() argument 'name' must be str, not int".to_owned())
            })?;

            let type_params = if let Some(tp) = args.kwargs.get("type_params") {
                let tp = tp
                    .clone()
                    .downcast::<crate::builtins::PyTuple>()
                    .map_err(|_| vm.new_type_error("type_params must be a tuple".to_owned()))?;
                Self::check_type_params(&tp, vm)?;
                tp
            } else {
                vm.ctx.empty_tuple.clone()
            };

            // Get caller's module name like caller()
            let module = vm.current_frame().and_then(|f| {
                let func_obj = f.func_obj.as_ref()?;
                func_obj.get_attr("__module__", vm).ok()
            });

            Ok(Self::new_eager(name, type_params, value, module))
        }
    }

    impl Representable for TypeAliasType {
        fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            Ok(zelf.name.as_str().to_owned())
        }
    }

    impl Hashable for TypeAliasType {
        fn hash(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<crate::common::hash::PyHash> {
            Ok(zelf.as_object().get_id() as crate::common::hash::PyHash)
        }
    }

    impl AsMapping for TypeAliasType {
        fn as_mapping() -> &'static PyMappingMethods {
            static AS_MAPPING: LazyLock<PyMappingMethods> = LazyLock::new(|| PyMappingMethods {
                subscript: atomic_func!(|mapping, needle, vm| {
                    let zelf = TypeAliasType::mapping_downcast(mapping);
                    TypeAliasType::__getitem__(zelf.to_owned(), needle.to_owned(), vm)
                }),
                ..PyMappingMethods::NOT_IMPLEMENTED
            });
            &AS_MAPPING
        }
    }

    impl AsNumber for TypeAliasType {
        fn as_number() -> &'static PyNumberMethods {
            static AS_NUMBER: PyNumberMethods = PyNumberMethods {
                or: Some(|a, b, vm| type_::or_(a.to_owned(), b.to_owned(), vm)),
                ..PyNumberMethods::NOT_IMPLEMENTED
            };
            &AS_NUMBER
        }
    }

    impl Iterable for TypeAliasType {
        fn iter(zelf: PyRef<Self>, vm: &VirtualMachine) -> PyResult {
            // Import typing.Unpack and return iter((Unpack[self],))
            let typing = vm.import("typing", 0)?;
            let unpack = typing.get_attr("Unpack", vm)?;
            let zelf_obj: PyObjectRef = zelf.into();
            let unpacked = vm.call_method(&unpack, "__getitem__", (zelf_obj,))?;
            let tuple = PyTuple::new_ref(vec![unpacked], &vm.ctx);
            Ok(tuple.as_object().get_iter(vm)?.into())
        }
    }

    /// Wrap TypeVarTuples in Unpack[], matching unpack_typevartuples()
    fn unpack_typevartuples(type_params: &PyTupleRef, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
        let has_tvt = type_params
            .iter()
            .any(|p| p.downcastable::<crate::stdlib::typevar::TypeVarTuple>());
        if !has_tvt {
            return Ok(type_params.clone());
        }
        let typing = vm.import("typing", 0)?;
        let unpack_cls = typing.get_attr("Unpack", vm)?;
        let new_params: Vec<PyObjectRef> = type_params
            .iter()
            .map(|p| {
                if p.downcastable::<crate::stdlib::typevar::TypeVarTuple>() {
                    vm.call_method(&unpack_cls, "__getitem__", (p.clone(),))
                } else {
                    Ok(p.clone())
                }
            })
            .collect::<PyResult<_>>()?;
        Ok(PyTuple::new_ref(new_params, &vm.ctx))
    }

    pub(crate) fn module_exec(
        vm: &VirtualMachine,
        module: &Py<crate::builtins::PyModule>,
    ) -> PyResult<()> {
        __module_exec(vm, module);

        extend_module!(vm, module, {
            "NoDefault" => vm.ctx.typing_no_default.clone(),
            "Union" => vm.ctx.types.union_type.to_owned(),
        });

        Ok(())
    }
}
