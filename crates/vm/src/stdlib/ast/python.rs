use super::{PY_CF_OPTIMIZED_AST, PY_CF_TYPE_COMMENTS, PY_COMPILE_FLAG_AST_ONLY};

#[pymodule]
pub(crate) mod _ast {
    use crate::{
        AsObject, Context, Py, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
        builtins::{PyStrRef, PyTupleRef, PyType, PyTypeRef},
        class::PyClassImpl,
        function::FuncArgs,
        types::{Constructor, Initializer},
    };
    #[pyattr]
    #[pyclass(module = "_ast", name = "AST")]
    #[derive(Debug, PyPayload)]
    pub(crate) struct NodeAst;

    #[pyclass(with(Constructor, Initializer), flags(BASETYPE, HAS_DICT))]
    impl NodeAst {
        #[pyattr]
        fn _fields(ctx: &Context) -> PyTupleRef {
            ctx.empty_tuple.clone()
        }
    }

    impl Constructor for NodeAst {
        type Args = FuncArgs;

        fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            // AST nodes accept extra arguments (unlike object.__new__)
            // This matches CPython's behavior where AST has its own tp_new
            let dict = if cls
                .slots
                .flags
                .contains(crate::types::PyTypeFlags::HAS_DICT)
            {
                Some(vm.ctx.new_dict())
            } else {
                None
            };
            let zelf = vm.ctx.new_base_object(cls, dict);

            // Initialize the instance with the provided arguments
            // FIXME: This is probably incorrect. Please check if init should be called outside of __new__
            Self::slot_init(zelf.clone(), args, vm)?;

            Ok(zelf)
        }

        fn py_new(_cls: &Py<PyType>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<Self> {
            unimplemented!("use slot_new")
        }
    }

    impl Initializer for NodeAst {
        type Args = FuncArgs;

        fn slot_init(zelf: PyObjectRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult<()> {
            let fields = zelf.get_attr("_fields", vm)?;
            let fields: Vec<PyStrRef> = fields.try_to_value(vm)?;
            let n_args = args.args.len();
            if n_args > fields.len() {
                return Err(vm.new_type_error(format!(
                    "{} constructor takes at most {} positional argument{}",
                    zelf.class().name(),
                    fields.len(),
                    if fields.len() == 1 { "" } else { "s" },
                )));
            }

            // Track which fields were set
            let mut set_fields = std::collections::HashSet::new();

            for (name, arg) in fields.iter().zip(args.args) {
                zelf.set_attr(name, arg, vm)?;
                set_fields.insert(name.as_str().to_string());
            }
            for (key, value) in args.kwargs {
                if let Some(pos) = fields.iter().position(|f| f.as_str() == key)
                    && pos < n_args
                {
                    return Err(vm.new_type_error(format!(
                        "{} got multiple values for argument '{}'",
                        zelf.class().name(),
                        key
                    )));
                }
                set_fields.insert(key.clone());
                zelf.set_attr(vm.ctx.intern_str(key), value, vm)?;
            }

            // Use _field_types to determine defaults for unset fields.
            // Only built-in AST node classes have _field_types populated.
            let field_types = zelf.class().get_attr(vm.ctx.intern_str("_field_types"));
            if let Some(Ok(ft_dict)) =
                field_types.map(|ft| ft.downcast::<crate::builtins::PyDict>())
            {
                let expr_ctx_type: PyObjectRef =
                    super::super::pyast::NodeExprContext::make_class(&vm.ctx).into();

                for field in &fields {
                    if set_fields.contains(field.as_str()) {
                        continue;
                    }
                    if let Some(ftype) = ft_dict.get_item_opt::<str>(field.as_str(), vm)? {
                        if ftype.fast_isinstance(vm.ctx.types.union_type) {
                            // Optional field (T | None) — no default
                        } else if ftype.fast_isinstance(vm.ctx.types.generic_alias_type) {
                            // List field (list[T]) — default to []
                            let empty_list: PyObjectRef = vm.ctx.new_list(vec![]).into();
                            zelf.set_attr(vm.ctx.intern_str(field.as_str()), empty_list, vm)?;
                        } else if ftype.is(&expr_ctx_type) {
                            // expr_context — default to Load()
                            let load_type =
                                super::super::pyast::NodeExprContextLoad::make_class(&vm.ctx);
                            let load_instance =
                                vm.ctx.new_base_object(load_type, Some(vm.ctx.new_dict()));
                            zelf.set_attr(vm.ctx.intern_str(field.as_str()), load_instance, vm)?;
                        }
                        // else: required field, no default set
                    }
                }
            }

            Ok(())
        }

        fn init(_zelf: PyRef<Self>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<()> {
            unreachable!("slot_init is defined")
        }
    }

    #[pyattr(name = "PyCF_ONLY_AST")]
    use super::PY_COMPILE_FLAG_AST_ONLY;

    #[pyattr(name = "PyCF_OPTIMIZED_AST")]
    use super::PY_CF_OPTIMIZED_AST;

    #[pyattr(name = "PyCF_TYPE_COMMENTS")]
    use super::PY_CF_TYPE_COMMENTS;

    pub(crate) fn module_exec(
        vm: &VirtualMachine,
        module: &Py<crate::builtins::PyModule>,
    ) -> PyResult<()> {
        __module_exec(vm, module);
        super::super::pyast::extend_module_nodes(vm, module);
        Ok(())
    }
}
