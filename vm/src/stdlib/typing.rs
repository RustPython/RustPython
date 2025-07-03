// spell-checker:ignore typevarobject funcobj
use crate::{PyPayload, PyRef, VirtualMachine, class::PyClassImpl, stdlib::PyModule};

pub use crate::stdlib::typevar::{
    ParamSpec, ParamSpecArgs, ParamSpecKwargs, TypeVar, TypeVarTuple,
};
pub use decl::*;

pub(crate) fn make_module(vm: &VirtualMachine) -> PyRef<PyModule> {
    let module = decl::make_module(vm);
    TypeVar::make_class(&vm.ctx);
    ParamSpec::make_class(&vm.ctx);
    TypeVarTuple::make_class(&vm.ctx);
    ParamSpecArgs::make_class(&vm.ctx);
    ParamSpecKwargs::make_class(&vm.ctx);
    extend_module!(vm, &module, {
        "NoDefault" => vm.ctx.typing_no_default.clone(),
        "TypeVar" => TypeVar::class(&vm.ctx).to_owned(),
        "ParamSpec" => ParamSpec::class(&vm.ctx).to_owned(),
        "TypeVarTuple" => TypeVarTuple::class(&vm.ctx).to_owned(),
        "ParamSpecArgs" => ParamSpecArgs::class(&vm.ctx).to_owned(),
        "ParamSpecKwargs" => ParamSpecKwargs::class(&vm.ctx).to_owned(),
    });
    module
}

#[pymodule(name = "_typing")]
pub(crate) mod decl {
    use crate::{
        PyObjectRef, PyPayload, PyResult, VirtualMachine,
        builtins::{PyTupleRef, PyTypeRef, pystr::AsPyStr},
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

    #[pyattr]
    #[pyclass(name = "Generic", module = "typing")]
    #[derive(Debug, PyPayload)]
    #[allow(dead_code)]
    pub(crate) struct Generic {}

    // #[pyclass(with(AsMapping), flags(BASETYPE))]
    #[pyclass(flags(BASETYPE))]
    impl Generic {
        #[pyclassmethod]
        fn __class_getitem__(cls: PyTypeRef, args: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            // Convert single arg to FuncArgs
            let func_args = FuncArgs {
                args: vec![args],
                kwargs: Default::default(),
            };
            call_typing_args_kwargs("_generic_class_getitem", cls, func_args, vm)
        }

        #[pyclassmethod]
        fn __init_subclass__(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            call_typing_args_kwargs("_generic_init_subclass", cls, args, vm)
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
