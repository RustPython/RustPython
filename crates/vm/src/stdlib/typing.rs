// spell-checker:ignore typevarobject funcobj
use crate::{Context, PyPayload, PyRef, VirtualMachine, class::PyClassImpl, stdlib::PyModule};

pub use crate::stdlib::typevar::{
    Generic, ParamSpec, ParamSpecArgs, ParamSpecKwargs, TypeVar, TypeVarTuple,
    set_typeparam_default,
};
pub use decl::*;

/// Initialize typing types (call extend_class)
pub fn init(ctx: &Context) {
    NoDefault::extend_class(ctx, ctx.types.typing_no_default_type);
}

pub(crate) fn make_module(vm: &VirtualMachine) -> PyRef<PyModule> {
    let module = decl::make_module(vm);
    TypeVar::make_class(&vm.ctx);
    ParamSpec::make_class(&vm.ctx);
    TypeVarTuple::make_class(&vm.ctx);
    ParamSpecArgs::make_class(&vm.ctx);
    ParamSpecKwargs::make_class(&vm.ctx);
    Generic::make_class(&vm.ctx);
    extend_module!(vm, &module, {
        "NoDefault" => vm.ctx.typing_no_default.clone(),
        "TypeVar" => TypeVar::class(&vm.ctx).to_owned(),
        "ParamSpec" => ParamSpec::class(&vm.ctx).to_owned(),
        "TypeVarTuple" => TypeVarTuple::class(&vm.ctx).to_owned(),
        "ParamSpecArgs" => ParamSpecArgs::class(&vm.ctx).to_owned(),
        "ParamSpecKwargs" => ParamSpecKwargs::class(&vm.ctx).to_owned(),
        "Generic" => Generic::class(&vm.ctx).to_owned(),
        "Union" => vm.ctx.types.union_type.to_owned(),
    });
    module
}

#[pymodule(name = "_typing")]
pub(crate) mod decl {
    use crate::{
        Py, PyObjectRef, PyPayload, PyResult, VirtualMachine,
        builtins::{PyStrRef, PyTupleRef, PyType, PyTypeRef, pystr::AsPyStr, type_},
        function::{FuncArgs, IntoFuncArgs},
        protocol::PyNumberMethods,
        types::{AsNumber, Constructor, Representable},
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
    #[pyclass(name)]
    #[derive(Debug, PyPayload)]
    #[allow(dead_code)]
    pub(crate) struct TypeAliasType {
        name: PyStrRef,
        type_params: PyTupleRef,
        value: PyObjectRef,
        // compute_value: PyObjectRef,
        // module: PyObjectRef,
    }
    #[pyclass(with(Constructor, Representable, AsNumber), flags(BASETYPE))]
    impl TypeAliasType {
        pub const fn new(name: PyStrRef, type_params: PyTupleRef, value: PyObjectRef) -> Self {
            Self {
                name,
                type_params,
                value,
            }
        }

        #[pygetset]
        fn __name__(&self) -> PyObjectRef {
            self.name.clone().into()
        }

        #[pygetset]
        fn __value__(&self) -> PyObjectRef {
            self.value.clone()
        }

        #[pygetset]
        fn __type_params__(&self) -> PyTupleRef {
            self.type_params.clone()
        }
    }

    impl Constructor for TypeAliasType {
        type Args = FuncArgs;

        fn py_new(_cls: &Py<PyType>, args: Self::Args, vm: &VirtualMachine) -> PyResult<Self> {
            // TypeAliasType(name, value, *, type_params=None)
            if args.args.len() < 2 {
                return Err(vm.new_type_error(format!(
                    "TypeAliasType() missing {} required positional argument{}: {}",
                    2 - args.args.len(),
                    if 2 - args.args.len() == 1 { "" } else { "s" },
                    if args.args.is_empty() {
                        "'name' and 'value'"
                    } else {
                        "'value'"
                    }
                )));
            }
            if args.args.len() > 2 {
                return Err(vm.new_type_error(format!(
                    "TypeAliasType() takes 2 positional arguments but {} were given",
                    args.args.len()
                )));
            }

            let name = args.args[0]
                .clone()
                .downcast::<crate::builtins::PyStr>()
                .map_err(|_| vm.new_type_error("TypeAliasType name must be a string".to_owned()))?;
            let value = args.args[1].clone();

            let type_params = if let Some(tp) = args.kwargs.get("type_params") {
                tp.clone()
                    .downcast::<crate::builtins::PyTuple>()
                    .map_err(|_| vm.new_type_error("type_params must be a tuple".to_owned()))?
            } else {
                vm.ctx.empty_tuple.clone()
            };

            Ok(Self::new(name, type_params, value))
        }
    }

    impl Representable for TypeAliasType {
        fn repr_str(zelf: &Py<Self>, _vm: &VirtualMachine) -> PyResult<String> {
            Ok(zelf.name.as_str().to_owned())
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
