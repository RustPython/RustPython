use super::{
    PY_CF_ALLOW_INCOMPLETE_INPUT, PY_CF_ALLOW_TOP_LEVEL_AWAIT, PY_CF_DONT_IMPLY_DEDENT,
    PY_CF_IGNORE_COOKIE, PY_CF_ONLY_AST, PY_CF_OPTIMIZED_AST, PY_CF_SOURCE_IS_UTF8,
    PY_CF_TYPE_COMMENTS,
};

#[pymodule]
pub(crate) mod _ast {
    use crate::{
        AsObject, Context, Py, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
        builtins::{PyStr, PyStrRef, PyTupleRef, PyType, PyTypeRef},
        class::{PyClassImpl, StaticType},
        function::{FuncArgs, KwArgs, PyMethodDef, PyMethodFlags},
        stdlib::ast::repr,
        types::{Constructor, Initializer},
        warn,
    };
    use indexmap::IndexMap;
    #[pyattr]
    #[pyclass(module = "_ast", name = "AST")]
    #[derive(Debug, PyPayload)]
    pub(crate) struct NodeAst;

    #[pyclass(with(Constructor, Initializer), flags(BASETYPE, HAS_DICT))]
    impl NodeAst {
        #[extend_class]
        fn extend_class(ctx: &Context, class: &'static Py<PyType>) {
            let empty_tuple = ctx.empty_tuple.clone();
            class.set_str_attr("_fields", empty_tuple.clone(), ctx);
            class.set_str_attr("_attributes", empty_tuple.clone(), ctx);
            class.set_str_attr("__match_args__", empty_tuple.clone(), ctx);

            const AST_REDUCE: PyMethodDef = PyMethodDef::new_const(
                "__reduce__",
                |zelf: PyObjectRef, vm: &VirtualMachine| -> PyResult<PyTupleRef> {
                    ast_reduce(zelf, vm)
                },
                PyMethodFlags::METHOD,
                None,
            );
            const AST_REPLACE: PyMethodDef = PyMethodDef::new_const(
                "__replace__",
                |zelf: PyObjectRef, args: FuncArgs, vm: &VirtualMachine| -> PyResult {
                    ast_replace(zelf, args, vm)
                },
                PyMethodFlags::METHOD,
                None,
            );

            class.set_str_attr("__reduce__", AST_REDUCE.to_proper_method(class, ctx), ctx);
            class.set_str_attr("__replace__", AST_REPLACE.to_proper_method(class, ctx), ctx);
            class.slots.repr.store(Some(ast_repr));
        }

        #[pyattr]
        fn _fields(ctx: &Context) -> PyTupleRef {
            ctx.empty_tuple.clone()
        }

        #[pyattr]
        fn _attributes(ctx: &Context) -> PyTupleRef {
            ctx.empty_tuple.clone()
        }

        #[pyattr]
        fn __match_args__(ctx: &Context) -> PyTupleRef {
            ctx.empty_tuple.clone()
        }

        #[pymethod]
        fn __reduce__(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
            ast_reduce(zelf, vm)
        }

        #[pymethod]
        fn __replace__(zelf: PyObjectRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            ast_replace(zelf, args, vm)
        }
    }

    pub(crate) fn ast_reduce(zelf: PyObjectRef, vm: &VirtualMachine) -> PyResult<PyTupleRef> {
        let dict = zelf.as_object().dict();
        let cls = zelf.class();
        let type_obj: PyObjectRef = cls.to_owned().into();

        let Some(dict) = dict else {
            return Ok(vm.ctx.new_tuple(vec![type_obj]));
        };

        let fields = cls.get_attr(vm.ctx.intern_str("_fields"));
        if let Some(fields) = fields {
            let fields: Vec<PyStrRef> = fields.try_to_value(vm)?;
            let mut positional: Vec<PyObjectRef> = Vec::new();
            for field in fields {
                if dict.get_item_opt::<str>(field.as_str(), vm)?.is_some() {
                    positional.push(vm.ctx.none());
                } else {
                    break;
                }
            }
            let args: PyObjectRef = vm.ctx.new_tuple(positional).into();
            let dict_obj: PyObjectRef = dict.into();
            return Ok(vm.ctx.new_tuple(vec![type_obj, args, dict_obj]));
        }

        Ok(vm
            .ctx
            .new_tuple(vec![type_obj, vm.ctx.new_tuple(vec![]).into(), dict.into()]))
    }

    pub(crate) fn ast_replace(zelf: PyObjectRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        if !args.args.is_empty() {
            return Err(vm.new_type_error("__replace__() takes no positional arguments".to_owned()));
        }

        let cls = zelf.class();
        let fields = cls.get_attr(vm.ctx.intern_str("_fields"));
        let attributes = cls.get_attr(vm.ctx.intern_str("_attributes"));
        let dict = zelf.as_object().dict();

        let mut expecting: std::collections::HashSet<String> = std::collections::HashSet::new();
        if let Some(fields) = fields.clone() {
            let fields: Vec<PyStrRef> = fields.try_to_value(vm)?;
            for field in fields {
                expecting.insert(field.as_str().to_owned());
            }
        }
        if let Some(attributes) = attributes.clone() {
            let attributes: Vec<PyStrRef> = attributes.try_to_value(vm)?;
            for attr in attributes {
                expecting.insert(attr.as_str().to_owned());
            }
        }

        for (key, _value) in &args.kwargs {
            if !expecting.remove(key) {
                return Err(vm.new_type_error(format!(
                    "{}.__replace__ got an unexpected keyword argument '{}'.",
                    cls.name(),
                    key
                )));
            }
        }

        if let Some(dict) = dict.as_ref() {
            for (key, _value) in dict.items_vec() {
                if let Ok(key) = key.downcast::<PyStr>() {
                    expecting.remove(key.as_str());
                }
            }
            if let Some(attributes) = attributes.clone() {
                let attributes: Vec<PyStrRef> = attributes.try_to_value(vm)?;
                for attr in attributes {
                    expecting.remove(attr.as_str());
                }
            }
        }

        // Discard optional fields (T | None).
        if let Some(field_types) = cls.get_attr(vm.ctx.intern_str("_field_types"))
            && let Ok(field_types) = field_types.downcast::<crate::builtins::PyDict>()
        {
            for (key, value) in field_types.items_vec() {
                let Ok(key) = key.downcast::<PyStr>() else {
                    continue;
                };
                if value.fast_isinstance(vm.ctx.types.union_type) {
                    expecting.remove(key.as_str());
                }
            }
        }

        if !expecting.is_empty() {
            let mut names: Vec<String> = expecting
                .into_iter()
                .map(|name| format!("{name:?}"))
                .collect();
            names.sort();
            let missing = names.join(", ");
            let count = names.len();
            return Err(vm.new_type_error(format!(
                "{}.__replace__ missing {} keyword argument{}: {}.",
                cls.name(),
                count,
                if count == 1 { "" } else { "s" },
                missing
            )));
        }

        let payload = vm.ctx.new_dict();
        if let Some(dict) = dict {
            if let Some(fields) = fields.clone() {
                let fields: Vec<PyStrRef> = fields.try_to_value(vm)?;
                for field in fields {
                    if let Some(value) = dict.get_item_opt::<str>(field.as_str(), vm)? {
                        payload.set_item(field.as_object(), value, vm)?;
                    }
                }
            }
            if let Some(attributes) = attributes.clone() {
                let attributes: Vec<PyStrRef> = attributes.try_to_value(vm)?;
                for attr in attributes {
                    if let Some(value) = dict.get_item_opt::<str>(attr.as_str(), vm)? {
                        payload.set_item(attr.as_object(), value, vm)?;
                    }
                }
            }
        }
        for (key, value) in args.kwargs {
            payload.set_item(vm.ctx.intern_str(key), value, vm)?;
        }

        let type_obj: PyObjectRef = cls.to_owned().into();
        let kwargs = payload
            .items_vec()
            .into_iter()
            .map(|(key, value)| {
                let key = key
                    .downcast::<PyStr>()
                    .map_err(|_| vm.new_type_error("keywords must be strings".to_owned()))?;
                Ok((key.as_str().to_owned(), value))
            })
            .collect::<PyResult<IndexMap<String, PyObjectRef>>>()?;
        let result = type_obj.call(FuncArgs::new(vec![], KwArgs::new(kwargs)), vm)?;
        Ok(result)
    }

    pub(crate) fn ast_repr(zelf: &crate::PyObject, vm: &VirtualMachine) -> PyResult<PyRef<PyStr>> {
        let repr = repr::repr_ast_node(vm, &zelf.to_owned(), 3)?;
        Ok(vm.ctx.new_str(repr))
    }

    impl Constructor for NodeAst {
        type Args = FuncArgs;

        fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            if args.args.is_empty()
                && args.kwargs.is_empty()
                && let Some(instance) = cls.get_attr(vm.ctx.intern_str("_instance"))
            {
                return Ok(instance);
            }

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

            // type.__call__ does not invoke slot_init after slot_new
            // for types with a custom slot_new, so we must call it here.
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
            let fields = zelf
                .class()
                .get_attr(vm.ctx.intern_str("_fields"))
                .ok_or_else(|| {
                    let module = zelf
                        .class()
                        .get_attr(vm.ctx.intern_str("__module__"))
                        .and_then(|obj| obj.try_to_value::<String>(vm).ok())
                        .unwrap_or_else(|| "ast".to_owned());
                    vm.new_attribute_error(format!(
                        "type object '{}.{}' has no attribute '_fields'",
                        module,
                        zelf.class().name()
                    ))
                })?;
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
            let mut attributes: Option<Vec<PyStrRef>> = None;

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

                if fields.iter().all(|field| field.as_str() != key) {
                    let attrs = if let Some(attrs) = &attributes {
                        attrs
                    } else {
                        let attrs = zelf
                            .class()
                            .get_attr(vm.ctx.intern_str("_attributes"))
                            .and_then(|attr| attr.try_to_value::<Vec<PyStrRef>>(vm).ok())
                            .unwrap_or_default();
                        attributes = Some(attrs);
                        attributes.as_ref().unwrap()
                    };
                    if attrs.iter().all(|attr| attr.as_str() != key) {
                        let message = vm.ctx.new_str(format!(
                            "{}.__init__ got an unexpected keyword argument '{}'. \
Support for arbitrary keyword arguments is deprecated and will be removed in Python 3.15.",
                            zelf.class().name(),
                            key
                        ));
                        warn::warn(
                            message.into(),
                            Some(vm.ctx.exceptions.deprecation_warning.to_owned()),
                            1,
                            None,
                            vm,
                        )?;
                    }
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
                            let load_instance = load_type
                                .get_attr(vm.ctx.intern_str("_instance"))
                                .unwrap_or_else(|| {
                                    vm.ctx.new_base_object(load_type, Some(vm.ctx.new_dict()))
                                });
                            zelf.set_attr(vm.ctx.intern_str(field.as_str()), load_instance, vm)?;
                        } else {
                            // Required field missing: emit DeprecationWarning (CPython behavior).
                            let message = vm.ctx.new_str(format!(
                                "{}.__init__ missing 1 required positional argument: '{}'",
                                zelf.class().name(),
                                field.as_str()
                            ));
                            warn::warn(
                                message.into(),
                                Some(vm.ctx.exceptions.deprecation_warning.to_owned()),
                                1,
                                None,
                                vm,
                            )?;
                        }
                    }
                }
            }

            Ok(())
        }

        fn init(_zelf: PyRef<Self>, _args: Self::Args, _vm: &VirtualMachine) -> PyResult<()> {
            unreachable!("slot_init is defined")
        }
    }

    #[pyattr(name = "PyCF_SOURCE_IS_UTF8")]
    use super::PY_CF_SOURCE_IS_UTF8;

    #[pyattr(name = "PyCF_DONT_IMPLY_DEDENT")]
    use super::PY_CF_DONT_IMPLY_DEDENT;

    #[pyattr(name = "PyCF_ONLY_AST")]
    use super::PY_CF_ONLY_AST;

    #[pyattr(name = "PyCF_IGNORE_COOKIE")]
    use super::PY_CF_IGNORE_COOKIE;

    #[pyattr(name = "PyCF_TYPE_COMMENTS")]
    use super::PY_CF_TYPE_COMMENTS;

    #[pyattr(name = "PyCF_ALLOW_TOP_LEVEL_AWAIT")]
    use super::PY_CF_ALLOW_TOP_LEVEL_AWAIT;

    #[pyattr(name = "PyCF_ALLOW_INCOMPLETE_INPUT")]
    use super::PY_CF_ALLOW_INCOMPLETE_INPUT;

    #[pyattr(name = "PyCF_OPTIMIZED_AST")]
    use super::PY_CF_OPTIMIZED_AST;

    pub(crate) fn module_exec(
        vm: &VirtualMachine,
        module: &Py<crate::builtins::PyModule>,
    ) -> PyResult<()> {
        __module_exec(vm, module);
        super::super::pyast::extend_module_nodes(vm, module);

        let ast_type = module
            .get_attr("AST", vm)?
            .downcast::<PyType>()
            .map_err(|_| vm.new_type_error("AST is not a type".to_owned()))?;
        let ctx = &vm.ctx;
        let empty_tuple = ctx.empty_tuple.clone();
        ast_type.set_str_attr("_fields", empty_tuple.clone(), ctx);
        ast_type.set_str_attr("_attributes", empty_tuple.clone(), ctx);
        ast_type.set_str_attr("__match_args__", empty_tuple.clone(), ctx);

        const AST_REDUCE: PyMethodDef = PyMethodDef::new_const(
            "__reduce__",
            |zelf: PyObjectRef, vm: &VirtualMachine| -> PyResult<PyTupleRef> {
                ast_reduce(zelf, vm)
            },
            PyMethodFlags::METHOD,
            None,
        );
        const AST_REPLACE: PyMethodDef = PyMethodDef::new_const(
            "__replace__",
            |zelf: PyObjectRef, args: FuncArgs, vm: &VirtualMachine| -> PyResult {
                ast_replace(zelf, args, vm)
            },
            PyMethodFlags::METHOD,
            None,
        );
        let base_type = NodeAst::static_type();
        ast_type.set_str_attr(
            "__reduce__",
            AST_REDUCE.to_proper_method(base_type, ctx),
            ctx,
        );
        ast_type.set_str_attr(
            "__replace__",
            AST_REPLACE.to_proper_method(base_type, ctx),
            ctx,
        );
        ast_type.slots.repr.store(Some(ast_repr));

        const EXPR_DOC: &str = "expr = BoolOp(boolop op, expr* values)\n\
     | NamedExpr(expr target, expr value)\n\
     | BinOp(expr left, operator op, expr right)\n\
     | UnaryOp(unaryop op, expr operand)\n\
     | Lambda(arguments args, expr body)\n\
     | IfExp(expr test, expr body, expr orelse)\n\
     | Dict(expr?* keys, expr* values)\n\
     | Set(expr* elts)\n\
     | ListComp(expr elt, comprehension* generators)\n\
     | SetComp(expr elt, comprehension* generators)\n\
     | DictComp(expr key, expr value, comprehension* generators)\n\
     | GeneratorExp(expr elt, comprehension* generators)\n\
     | Await(expr value)\n\
     | Yield(expr? value)\n\
     | YieldFrom(expr value)\n\
     | Compare(expr left, cmpop* ops, expr* comparators)\n\
     | Call(expr func, expr* args, keyword* keywords)\n\
     | FormattedValue(expr value, int conversion, expr? format_spec)\n\
     | Interpolation(expr value, constant str, int conversion, expr? format_spec)\n\
     | JoinedStr(expr* values)\n\
     | TemplateStr(expr* values)\n\
     | Constant(constant value, string? kind)\n\
     | Attribute(expr value, identifier attr, expr_context ctx)\n\
     | Subscript(expr value, expr slice, expr_context ctx)\n\
     | Starred(expr value, expr_context ctx)\n\
     | Name(identifier id, expr_context ctx)\n\
     | List(expr* elts, expr_context ctx)\n\
     | Tuple(expr* elts, expr_context ctx)\n\
     | Slice(expr? lower, expr? upper, expr? step)";
        let expr_type = super::super::pyast::NodeExpr::static_type();
        expr_type.set_attr(
            identifier!(vm.ctx, __doc__),
            vm.ctx.new_str(EXPR_DOC).into(),
        );
        Ok(())
    }
}
