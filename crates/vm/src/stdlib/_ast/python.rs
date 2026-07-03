use super::{
    PY_CF_ALLOW_INCOMPLETE_INPUT, PY_CF_ALLOW_TOP_LEVEL_AWAIT, PY_CF_DONT_IMPLY_DEDENT,
    PY_CF_IGNORE_COOKIE, PY_CF_ONLY_AST, PY_CF_OPTIMIZED_AST, PY_CF_SOURCE_IS_UTF8,
    PY_CF_TYPE_COMMENTS,
};

#[pymodule]
pub(crate) mod _ast {
    use crate::{
        AsObject, Context, Py, PyObject, PyObjectRef, PyPayload, PyRef, PyResult, VirtualMachine,
        builtins::{PyDictRef, PySet, PyStr, PyTupleRef, PyType, PyTypeRef, PyUtf8Str},
        class::{PyClassImpl, StaticType},
        function::{ArgIterable, FuncArgs, KwArgs, PyMethodDef, PyMethodFlags},
        stdlib::_ast::repr,
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
            // AST types are mutable (heap types, not IMMUTABLETYPE)
            // Safety: called during type initialization before any concurrent access
            unsafe {
                let flags = &class.slots.flags as *const crate::types::PyTypeFlags
                    as *mut crate::types::PyTypeFlags;
                (*flags).remove(crate::types::PyTypeFlags::IMMUTABLETYPE);
            }
            let empty_tuple = ctx.empty_tuple.clone();
            class.set_str_attr("_fields", empty_tuple.clone(), ctx);
            class.set_str_attr("_attributes", empty_tuple.clone(), ctx);
            class.set_str_attr("__match_args__", empty_tuple, ctx);

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
            const AST_DEEPCOPY: PyMethodDef = PyMethodDef::new_const(
                "__deepcopy__",
                |zelf: PyObjectRef, memo: PyObjectRef, vm: &VirtualMachine| -> PyResult {
                    ast_deepcopy(zelf, memo, vm)
                },
                PyMethodFlags::METHOD,
                None,
            );

            class.set_str_attr("__reduce__", AST_REDUCE.to_proper_method(class, ctx), ctx);
            class.set_str_attr("__replace__", AST_REPLACE.to_proper_method(class, ctx), ctx);
            class.set_str_attr(
                "__deepcopy__",
                AST_DEEPCOPY.to_proper_method(class, ctx),
                ctx,
            );
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

        #[pymethod]
        fn __deepcopy__(zelf: PyObjectRef, memo: PyObjectRef, vm: &VirtualMachine) -> PyResult {
            ast_deepcopy(zelf, memo, vm)
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
            let fields = fields.sequence_unchecked();
            let numfields = fields.length(vm)?;
            let mut positional: Vec<PyObjectRef> = Vec::new();
            for i in 0..numfields {
                let field = fields.get_item(i as isize, vm)?;
                if dict.get_item_opt(&*field, vm)?.is_some() {
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

    fn ast_replace_update_payload(
        payload: &PyDictRef,
        keys: Option<&PyObjectRef>,
        dict: &PyDictRef,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let Some(keys) = keys else {
            return Ok(());
        };
        let keys = keys.sequence_unchecked();
        let num_keys = keys.length(vm)?;
        for i in 0..num_keys {
            let key = keys.get_item(i as isize, vm)?;
            if let Some(value) = dict.get_item_opt(&*key, vm)? {
                payload.set_item(&*key, value, vm)?;
            }
        }
        Ok(())
    }

    fn ast_replace_set_update(
        expecting: &PyRef<PySet>,
        iterable: Option<&PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let Some(iterable) = iterable else {
            return Ok(());
        };
        let iterable = iterable.clone().try_into_value::<ArgIterable>(vm)?;
        for item in iterable.iter(vm)? {
            expecting.add(item?, vm)?;
        }
        Ok(())
    }

    fn ast_replace_set_discard(
        expecting: &PyRef<PySet>,
        key: &PyObject,
        vm: &VirtualMachine,
    ) -> PyResult<bool> {
        let contained = expecting
            .as_object()
            .sequence_unchecked()
            .contains(key, vm)?;
        if contained {
            vm.call_method(expecting.as_object(), "discard", (key.to_owned(),))?;
        }
        Ok(contained)
    }

    fn ast_replace_set_difference_update(
        expecting: &PyRef<PySet>,
        iterable: Option<&PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let Some(iterable) = iterable else {
            return Ok(());
        };
        let iterable = iterable.clone().try_into_value::<ArgIterable>(vm)?;
        for item in iterable.iter(vm)? {
            let item = item?;
            ast_replace_set_discard(expecting, &item, vm)?;
        }
        Ok(())
    }

    fn ast_set_attr(
        obj: &PyObject,
        name: &PyObject,
        value: impl Into<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<()> {
        let name = name
            .to_owned()
            .downcast::<PyStr>()
            .map_err(|_| vm.new_type_error("attribute name must be string"))?;
        obj.set_attr(&name, value, vm)
    }

    pub(crate) fn ast_replace(zelf: PyObjectRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
        if !args.args.is_empty() {
            return Err(vm.new_type_error("__replace__() takes no positional arguments"));
        }

        let cls = zelf.class();
        let fields = cls.get_attr(vm.ctx.intern_str("_fields"));
        let attributes = cls.get_attr(vm.ctx.intern_str("_attributes"));
        let dict = zelf.as_object().dict();

        let expecting = PySet::default().into_ref(&vm.ctx);
        ast_replace_set_update(&expecting, fields.as_ref(), vm)?;
        ast_replace_set_update(&expecting, attributes.as_ref(), vm)?;

        for (key, _value) in &args.kwargs {
            let key_obj: PyObjectRef = vm.ctx.new_str(key.as_str()).into();
            if !ast_replace_set_discard(&expecting, &key_obj, vm)? {
                return Err(vm.new_type_error(format!(
                    "{}.__replace__ got an unexpected keyword argument '{}'.",
                    cls.name(),
                    key
                )));
            }
        }

        if let Some(dict) = dict.as_ref() {
            for (key, _value) in dict.items_vec() {
                ast_replace_set_discard(&expecting, &key, vm)?;
            }
            ast_replace_set_difference_update(&expecting, attributes.as_ref(), vm)?;
        }

        // Discard optional fields (T | None).
        if let Some(field_types) = cls.get_attr(vm.ctx.intern_str("_field_types"))
            && let Ok(field_types) = field_types.downcast::<crate::builtins::PyDict>()
        {
            for (key, value) in field_types.items_vec() {
                if value.fast_isinstance(vm.ctx.types.union_type) {
                    ast_replace_set_discard(&expecting, &key, vm)?;
                }
            }
        }

        let remaining = expecting.elements();
        if !remaining.is_empty() {
            let mut names = Vec::with_capacity(remaining.len());
            for name in &remaining {
                names.push(name.repr(vm)?.to_string());
            }
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
            ast_replace_update_payload(&payload, fields.as_ref(), &dict, vm)?;
            ast_replace_update_payload(&payload, attributes.as_ref(), &dict, vm)?;
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
                    .downcast::<PyUtf8Str>()
                    .map_err(|_| vm.new_type_error("keywords must be strings"))?;
                Ok((key.as_str().to_owned(), value))
            })
            .collect::<PyResult<IndexMap<String, PyObjectRef>>>()?;
        let result = type_obj.call(FuncArgs::new(vec![], KwArgs::new(kwargs)), vm)?;
        Ok(result)
    }

    pub(crate) fn ast_deepcopy(
        zelf: PyObjectRef,
        memo: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult {
        let memo_dict: PyDictRef = memo
            .clone()
            .downcast()
            .map_err(|_| vm.new_type_error("__deepcopy__() memo must be a dict"))?;
        let memo_key: PyObjectRef = vm.ctx.new_int(zelf.get_id() as i64).into();

        if let Some(existing) = memo_dict.get_item_opt(&*memo_key, vm)? {
            return Ok(existing);
        }

        let cls = zelf.class();
        let copied_dict = if cls
            .slots
            .flags
            .contains(crate::types::PyTypeFlags::HAS_DICT)
        {
            Some(vm.ctx.new_dict())
        } else {
            None
        };
        let copied = vm.ctx.new_base_object(cls.to_owned(), copied_dict.clone());

        memo_dict.set_item(&*memo_key, copied.clone(), vm)?;

        if let (Some(src_dict), Some(dst_dict)) = (zelf.as_object().dict(), copied_dict) {
            let deepcopy = vm.import("copy", 0)?.get_attr("deepcopy", vm)?;
            for (key, value) in src_dict.items_vec() {
                let copied_value = deepcopy.call((value, memo.clone()), vm)?;
                dst_dict.set_item(&*key, copied_value, vm)?;
            }
        }

        Ok(copied)
    }

    pub(crate) fn ast_repr(zelf: &crate::PyObject, vm: &VirtualMachine) -> PyResult<PyRef<PyStr>> {
        let repr = repr::repr_ast_node(vm, &zelf.to_owned(), 3)?;
        Ok(vm.ctx.new_str(repr))
    }

    impl Constructor for NodeAst {
        type Args = FuncArgs;

        fn slot_new(cls: PyTypeRef, args: FuncArgs, vm: &VirtualMachine) -> PyResult {
            // Keep _instance for parser-internal shared operator/context nodes,
            // but match CPython's public constructor behavior by allocating a
            // fresh object for Python-level ast.Load()/ast.Add()/... calls.
            // Returning the cached singleton here makes user-added attributes
            // like `parent` leak across unrelated trees and breaks deepcopy.
            // AST nodes accept extra arguments (unlike object.__new__).
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
            unreachable!("NodeAst construction is handled by slot_new")
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
            let fields_seq = fields.sequence_unchecked();
            let numfields = fields_seq.length(vm)?;
            let remaining_fields = PySet::default().into_ref(&vm.ctx);
            ast_replace_set_update(&remaining_fields, Some(&fields), vm)?;
            let n_args = args.args.len();
            if n_args > numfields {
                return Err(vm.new_type_error(format!(
                    "{} constructor takes at most {} positional argument{}",
                    zelf.class().name(),
                    numfields,
                    if numfields == 1 { "" } else { "s" },
                )));
            }

            let mut attributes: Option<PyObjectRef> = None;

            for (i, arg) in args.args.into_iter().enumerate() {
                let name = fields_seq.get_item(i as isize, vm)?;
                ast_set_attr(&zelf, &name, arg, vm)?;
                ast_replace_set_discard(&remaining_fields, &name, vm)?;
            }
            for (key, value) in args.kwargs {
                let key_obj: PyObjectRef = vm.ctx.new_str(key.as_str()).into();
                let contains = fields_seq.contains(&key_obj, vm)?;
                if contains {
                    if !ast_replace_set_discard(&remaining_fields, &key_obj, vm)? {
                        return Err(vm.new_type_error(format!(
                            "{} got multiple values for argument '{}'",
                            zelf.class().name(),
                            key
                        )));
                    }
                } else {
                    let attrs = if let Some(attributes) = &attributes {
                        attributes
                    } else {
                        let attrs = zelf
                            .class()
                            .get_attr(vm.ctx.intern_str("_attributes"))
                            .ok_or_else(|| {
                                vm.new_attribute_error(format!(
                                    "type object '{}' has no attribute '_attributes'",
                                    zelf.class().name()
                                ))
                            })?;
                        attributes = Some(attrs);
                        attributes.as_ref().unwrap()
                    };
                    if !attrs.sequence_unchecked().contains(&key_obj, vm)? {
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

                zelf.set_attr(vm.ctx.intern_str(key), value, vm)?;
            }

            // Use _field_types to determine defaults for unset fields.
            // Only built-in AST node classes have _field_types populated.
            let field_types = zelf.class().get_attr(vm.ctx.intern_str("_field_types"));
            if let Some(Ok(ft_dict)) =
                field_types.map(|ft| ft.downcast::<crate::builtins::PyDict>())
            {
                let expr_ctx_type: PyObjectRef =
                    super::super::pyast::NodeExprContext::make_static_type().into();

                for field in remaining_fields.elements() {
                    if let Some(ftype) = ft_dict.get_item_opt(&*field, vm)? {
                        if ftype.fast_isinstance(vm.ctx.types.union_type) {
                            // Optional field (T | None) — no default
                        } else if ftype.fast_isinstance(vm.ctx.types.generic_alias_type) {
                            // List field (list[T]) — default to []
                            let empty_list: PyObjectRef = vm.ctx.new_list(vec![]).into();
                            ast_set_attr(&zelf, &field, empty_list, vm)?;
                        } else if ftype.is(&expr_ctx_type) {
                            // expr_context — default to Load()
                            let load_type =
                                super::super::pyast::NodeExprContextLoad::make_static_type();
                            let load_instance = load_type
                                .get_attr(vm.ctx.intern_str("_instance"))
                                .unwrap_or_else(|| {
                                    vm.ctx.new_base_object(load_type, Some(vm.ctx.new_dict()))
                                });
                            ast_set_attr(&zelf, &field, load_instance, vm)?;
                        } else {
                            // Required field missing: emit DeprecationWarning.
                            let field_repr = field.repr(vm)?;
                            let message = vm.ctx.new_str(format!(
                                "{}.__init__ missing 1 required positional argument: {}. \
This will become an error in Python 3.15.",
                                zelf.class().name(),
                                field_repr
                            ));
                            warn::warn(
                                message.into(),
                                Some(vm.ctx.exceptions.deprecation_warning.to_owned()),
                                1,
                                None,
                                vm,
                            )?;
                        }
                    } else {
                        let field_repr = field.repr(vm)?;
                        let message = vm.ctx.new_str(format!(
                            "Field {} is missing from {}._field_types. \
This will become an error in Python 3.15.",
                            field_repr,
                            zelf.class().name()
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
            .map_err(|_| vm.new_type_error("AST is not a type"))?;
        let ctx = &vm.ctx;
        let empty_tuple = ctx.empty_tuple.clone();
        let set_empty_annotations = |typ: &Py<PyType>| {
            typ.set_str_attr("__annotations__", ctx.new_dict(), ctx);
        };
        set_empty_annotations(&ast_type);
        ast_type.set_str_attr("_fields", empty_tuple.clone(), ctx);
        ast_type.set_str_attr("_attributes", empty_tuple.clone(), ctx);
        ast_type.set_str_attr("__match_args__", empty_tuple, ctx);
        for typ in [
            super::super::pyast::NodeMod::static_type(),
            super::super::pyast::NodeStmt::static_type(),
            super::super::pyast::NodeExpr::static_type(),
            super::super::pyast::NodeExprContext::static_type(),
            super::super::pyast::NodeBoolOp::static_type(),
            super::super::pyast::NodeOperator::static_type(),
            super::super::pyast::NodeUnaryOp::static_type(),
            super::super::pyast::NodeCmpOp::static_type(),
            super::super::pyast::NodeExceptHandler::static_type(),
            super::super::pyast::NodePattern::static_type(),
            super::super::pyast::NodeTypeIgnore::static_type(),
            super::super::pyast::NodeTypeParam::static_type(),
        ] {
            set_empty_annotations(typ);
        }

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
