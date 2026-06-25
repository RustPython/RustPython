use super::*;
use rustpython_compiler_core::SourceFile;

// product
impl Node for ast::Parameters {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            posonlyargs,
            args,
            vararg,
            kwonlyargs,
            kwarg,
            range: _,
            runtime_defaults,
        } = self;
        let (posonlyargs, args, mut defaults) =
            extract_positional_parameter_defaults(posonlyargs, args);
        defaults.runtime_defaults = runtime_defaults;
        let (kwonlyargs, kw_defaults) = extract_keyword_parameter_defaults(kwonlyargs);
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeArguments::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item(
            "posonlyargs",
            posonlyargs.ast_to_object(vm, source_file),
            vm,
        )
        .unwrap();
        dict.set_item("args", args.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item("vararg", vararg.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item("kwonlyargs", kwonlyargs.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item(
            "kw_defaults",
            kw_defaults.ast_to_object(vm, source_file),
            vm,
        )
        .unwrap();
        dict.set_item("kwarg", kwarg.ast_to_object(vm, source_file), vm)
            .unwrap();
        let runtime_defaults = defaults.runtime_defaults.take();
        let defaults = runtime_defaults.map_or_else(
            || defaults.ast_to_object(vm, source_file),
            |values| values.ast_to_object(vm, source_file),
        );
        dict.set_item("defaults", defaults, vm).unwrap();
        node.into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let posonlyargs = PositionalParameters::ast_from_field(
            vm,
            source_file,
            &object,
            "posonlyargs",
            "arguments",
        )?;
        let args =
            PositionalParameters::ast_from_field(vm, source_file, &object, "args", "arguments")?;
        let vararg = get_node_field_opt(vm, &object, "vararg")?
            .map(|obj| Node::ast_from_object(vm, source_file, obj))
            .transpose()?;
        let kwonlyargs =
            KeywordParameters::ast_from_field(vm, source_file, &object, "kwonlyargs", "arguments")?;
        let kw_defaults = ParameterDefaults::ast_from_field(
            vm,
            source_file,
            &object,
            "kw_defaults",
            "arguments",
        )?;
        let kwarg = get_node_field_opt(vm, &object, "kwarg")?
            .map(|obj| Node::ast_from_object(vm, source_file, obj))
            .transpose()?;
        let defaults = ParameterDefaults::ast_from_field_preserve_none(
            vm,
            source_file,
            &object,
            "defaults",
            "arguments",
        )?;

        let ParameterDefaults {
            runtime_defaults,
            defaults,
            _range: _,
        } = defaults;
        let kwonlyargs = merge_keyword_parameter_defaults(vm, kwonlyargs, kw_defaults)?;
        let (posonlyargs, args) =
            merge_positional_parameter_defaults(vm, posonlyargs, args, defaults)?;

        Ok(Self {
            node_index: Default::default(),
            posonlyargs,
            args,
            vararg,
            kwonlyargs,
            kwarg,
            range: Default::default(),
            runtime_defaults,
        })
    }

    fn is_none(&self) -> bool {
        self.is_empty()
    }
}

// product
impl Node for ast::Parameter {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            name,
            annotation,
            // type_comment,
            range,
            runtime_type_comment,
            runtime_type_comment_bytes,
        } = self;

        // ruff covers the ** in range but python expects it to start at the ident
        let range = TextRange::new(name.start(), range.end());

        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeArg::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("arg", name.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item("annotation", annotation.ast_to_object(vm, source_file), vm)
            .unwrap();
        let type_comment = super::constant::runtime_string_object(
            vm,
            runtime_type_comment,
            runtime_type_comment_bytes,
        )
        .unwrap_or_else(|| vm.ctx.none());
        dict.set_item("type_comment", type_comment, vm).unwrap();
        node_add_location(&dict, range, vm, source_file);
        node.into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        let name = get_required_identifier_field(vm, source_file, &_object, "arg", "arg")?;
        let annotation = get_node_field_opt(vm, &_object, "annotation")?
            .map(|obj| Node::ast_from_object(vm, source_file, obj))
            .transpose()?;
        let type_comment = get_ast_string_field_opt(vm, &_object, "type_comment")?;
        let (runtime_type_comment, runtime_type_comment_bytes) = type_comment
            .map_or((None, None), |type_comment| {
                super::constant::runtime_string_from_pyobject(vm, type_comment)
            });
        let range = range_from_object(vm, source_file, _object, "arg")?;
        Ok(Self {
            node_index: Default::default(),
            name,
            annotation,
            range,
            runtime_type_comment,
            runtime_type_comment_bytes,
        })
    }
}

// product
impl Node for ast::Keyword {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            arg,
            value,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeKeyword::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("arg", arg.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item("value", value.ast_to_object(vm, source_file), vm)
            .unwrap();
        node_add_location(&dict, _range, vm, source_file);
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            node_index: Default::default(),
            arg: get_node_field_opt(vm, &_object, "arg")?
                .map(|obj| Node::ast_from_object(vm, source_file, obj))
                .transpose()?,
            value: get_required_node_field(vm, source_file, &_object, "value", "keyword")?,
            range: range_from_object(vm, source_file, _object, "keyword")?,
        })
    }
}

struct PositionalParameters {
    pub _range: TextRange, // TODO: Use this
    pub args: Box<[ast::Parameter]>,
}

impl PositionalParameters {
    fn ast_from_field(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: &PyObject,
        field: &'static str,
        typ: &str,
    ) -> PyResult<Self> {
        Ok(Self {
            args: get_node_boxed_slice_field(vm, source_file, object, field, typ)?,
            _range: TextRange::default(),
        })
    }
}

impl Node for PositionalParameters {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        BoxedSlice(self.args).ast_to_object(vm, source_file)
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let args: BoxedSlice<_> = Node::ast_from_object(vm, source_file, object)?;
        Ok(Self {
            args: args.0,
            _range: TextRange::default(),
        })
    }
}

struct KeywordParameters {
    pub _range: TextRange, // TODO: Use this
    pub keywords: Box<[ast::Parameter]>,
}

impl KeywordParameters {
    fn ast_from_field(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: &PyObject,
        field: &'static str,
        typ: &str,
    ) -> PyResult<Self> {
        Ok(Self {
            keywords: get_node_boxed_slice_field(vm, source_file, object, field, typ)?,
            _range: TextRange::default(),
        })
    }
}

impl Node for KeywordParameters {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        BoxedSlice(self.keywords).ast_to_object(vm, source_file)
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let keywords: BoxedSlice<_> = Node::ast_from_object(vm, source_file, object)?;
        Ok(Self {
            keywords: keywords.0,
            _range: TextRange::default(),
        })
    }
}

struct ParameterDefaults {
    pub _range: TextRange, // TODO: Use this
    runtime_defaults: Option<Vec<Option<ast::Expr>>>,
    defaults: Box<[Option<Box<ast::Expr>>]>,
}

impl ParameterDefaults {
    fn ast_from_field(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: &PyObject,
        field: &'static str,
        typ: &str,
    ) -> PyResult<Self> {
        Ok(Self {
            defaults: get_node_boxed_slice_field(vm, source_file, object, field, typ)?,
            runtime_defaults: None,
            _range: TextRange::default(),
        })
    }

    fn ast_from_field_preserve_none(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: &PyObject,
        field: &'static str,
        typ: &str,
    ) -> PyResult<Self> {
        let defaults: Vec<Option<Box<ast::Expr>>> =
            get_node_list_field(vm, source_file, object, field, typ)?;
        let runtime_defaults = defaults.iter().any(Option::is_none).then(|| {
            defaults
                .iter()
                .map(|default| default.as_deref().cloned())
                .collect()
        });
        Ok(Self {
            defaults: defaults.into_boxed_slice(),
            runtime_defaults,
            _range: TextRange::default(),
        })
    }
}

impl Node for ParameterDefaults {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        BoxedSlice(self.defaults).ast_to_object(vm, source_file)
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let defaults: BoxedSlice<_> = Node::ast_from_object(vm, source_file, object)?;
        Ok(Self {
            defaults: defaults.0,
            runtime_defaults: None,
            _range: TextRange::default(),
        })
    }
}

fn extract_positional_parameter_defaults(
    pos_only_args: Vec<ast::ParameterWithDefault>,
    args: Vec<ast::ParameterWithDefault>,
) -> (
    PositionalParameters,
    PositionalParameters,
    ParameterDefaults,
) {
    let mut defaults = vec![];
    defaults.extend(pos_only_args.iter().map(|item| item.default.clone()));
    defaults.extend(args.iter().map(|item| item.default.clone()));
    // If some positional parameters have no default value,
    // the "defaults" list contains only the defaults of the last "n" parameters.
    // Remove all positional parameters without a default value.
    defaults.retain(Option::is_some);
    let defaults = ParameterDefaults {
        _range: defaults
            .iter()
            .flatten()
            .map(|item| item.range())
            .reduce(|acc, next| acc.cover(next))
            .unwrap_or_default(),
        runtime_defaults: None,
        defaults: defaults.into_boxed_slice(),
    };

    let pos_only_args = PositionalParameters {
        _range: pos_only_args
            .iter()
            .map(|item| item.range())
            .reduce(|acc, next| acc.cover(next))
            .unwrap_or_default(),
        args: {
            let pos_only_args: Vec<_> = pos_only_args
                .iter()
                .map(|item| item.parameter.clone())
                .collect();
            pos_only_args.into_boxed_slice()
        },
    };

    let args = PositionalParameters {
        _range: args
            .iter()
            .map(|item| item.range())
            .reduce(|acc, next| acc.cover(next))
            .unwrap_or_default(),
        args: {
            let args: Vec<_> = args.iter().map(|item| item.parameter.clone()).collect();
            args.into_boxed_slice()
        },
    };

    (pos_only_args, args, defaults)
}

/// Merges the keyword parameters with their default values, opposite of [`extract_positional_parameter_defaults`].
fn merge_positional_parameter_defaults(
    vm: &VirtualMachine,
    posonlyargs: PositionalParameters,
    args: PositionalParameters,
    defaults: Box<[Option<Box<ast::Expr>>]>,
) -> PyResult<(
    Vec<ast::ParameterWithDefault>,
    Vec<ast::ParameterWithDefault>,
)> {
    let posonlyargs = posonlyargs.args;
    let args = args.args;

    let mut posonlyargs: Vec<_> = <Box<[_]> as IntoIterator>::into_iter(posonlyargs)
        .map(|parameter| ast::ParameterWithDefault {
            node_index: Default::default(),
            range: Default::default(),
            parameter,
            default: None,
        })
        .collect();
    let mut args: Vec<_> = <Box<[_]> as IntoIterator>::into_iter(args)
        .map(|parameter| ast::ParameterWithDefault {
            node_index: Default::default(),
            range: Default::default(),
            parameter,
            default: None,
        })
        .collect();

    // If an argument has a default value, insert it
    // Note that "defaults" will only contain default values for the last "n" parameters
    // so we need to skip the first "total_argument_count - n" arguments.
    let total_args = posonlyargs.len() + args.len();
    if defaults.len() > total_args {
        return Err(vm.new_value_error("more positional defaults than args on arguments"));
    }
    let default_argument_count = total_args - defaults.len();
    for (arg, default) in posonlyargs
        .iter_mut()
        .chain(args.iter_mut())
        .skip(default_argument_count)
        .zip(defaults)
    {
        arg.default = default;
    }

    Ok((posonlyargs, args))
}

fn extract_keyword_parameter_defaults(
    kw_only_args: Vec<ast::ParameterWithDefault>,
) -> (KeywordParameters, ParameterDefaults) {
    let mut defaults = vec![];
    defaults.extend(kw_only_args.iter().map(|item| item.default.clone()));
    let defaults = ParameterDefaults {
        _range: defaults
            .iter()
            .flatten()
            .map(|item| item.range())
            .reduce(|acc, next| acc.cover(next))
            .unwrap_or_default(),
        runtime_defaults: None,
        defaults: defaults.into_boxed_slice(),
    };

    let kw_only_args = KeywordParameters {
        _range: kw_only_args
            .iter()
            .map(|item| item.range())
            .reduce(|acc, next| acc.cover(next))
            .unwrap_or_default(),
        keywords: {
            let kw_only_args: Vec<_> = kw_only_args
                .iter()
                .map(|item| item.parameter.clone())
                .collect();
            kw_only_args.into_boxed_slice()
        },
    };

    (kw_only_args, defaults)
}

/// Merges the keyword parameters with their default values, opposite of [`extract_keyword_parameter_defaults`].
fn merge_keyword_parameter_defaults(
    vm: &VirtualMachine,
    kw_only_args: KeywordParameters,
    defaults: ParameterDefaults,
) -> PyResult<Vec<ast::ParameterWithDefault>> {
    if kw_only_args.keywords.len() != defaults.defaults.len() {
        return Err(
            vm.new_value_error("length of kwonlyargs is not the same as kw_defaults on arguments")
        );
    }
    Ok(core::iter::zip(kw_only_args.keywords, defaults.defaults)
        .map(|(parameter, default)| ast::ParameterWithDefault {
            node_index: Default::default(),
            parameter,
            default,
            range: Default::default(),
        })
        .collect::<Vec<_>>())
}
