use super::*;

// product
impl Node for ruff::Parameters {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            posonlyargs,
            args,
            vararg,
            kwonlyargs,
            kwarg,
            range,
        } = self;
        let (posonlyargs, args, defaults) =
            extract_positional_parameter_defaults(posonlyargs, args);
        let (kwonlyargs, kw_defaults) = extract_keyword_parameter_defaults(kwonlyargs);
        let node = NodeAst
            .into_ref_with_type(vm, gen::NodeArguments::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        if !posonlyargs.args.is_empty() {
            dict.set_item("posonlyargs", posonlyargs.ast_to_object(vm), vm)
                .unwrap();
        }
        if !args.args.is_empty() {
            dict.set_item("args", args.ast_to_object(vm), vm).unwrap();
        }
        if let Some(vararg) = vararg {
            dict.set_item("vararg", vararg.ast_to_object(vm), vm)
                .unwrap();
        }
        if !kwonlyargs.keywords.is_empty() {
            dict.set_item("kwonlyargs", kwonlyargs.ast_to_object(vm), vm)
                .unwrap();
        }
        if !kw_defaults.defaults.is_empty() {
            dict.set_item("kw_defaults", kw_defaults.ast_to_object(vm), vm)
                .unwrap();
        }
        if let Some(kwarg) = kwarg {
            dict.set_item("kwarg", kwarg.ast_to_object(vm), vm).unwrap();
        }
        if !defaults.defaults.is_empty() {
            dict.set_item("defaults", defaults.ast_to_object(vm), vm)
                .unwrap();
        }
        node_add_location(&dict, range, vm);
        node.into()
    }
    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        let kwonlyargs =
            Node::ast_from_object(vm, get_node_field(vm, &object, "kwonlyargs", "arguments")?)?;
        let kw_defaults =
            Node::ast_from_object(vm, get_node_field(vm, &object, "kw_defaults", "arguments")?)?;
        let kwonlyargs = merge_keyword_parameter_defaults(kwonlyargs, kw_defaults);

        let posonlyargs =
            Node::ast_from_object(vm, get_node_field(vm, &object, "posonlyargs", "arguments")?)?;
        let args = Node::ast_from_object(vm, get_node_field(vm, &object, "args", "arguments")?)?;
        let defaults =
            Node::ast_from_object(vm, get_node_field(vm, &object, "defaults", "arguments")?)?;
        let (posonlyargs, args) = merge_positional_parameter_defaults(posonlyargs, args, defaults);

        Ok(Self {
            posonlyargs,
            args,
            vararg: get_node_field_opt(vm, &object, "vararg")?
                .map(|obj| Node::ast_from_object(vm, obj))
                .transpose()?,
            kwonlyargs,
            kwarg: get_node_field_opt(vm, &object, "kwarg")?
                .map(|obj| Node::ast_from_object(vm, obj))
                .transpose()?,
            range: Default::default(),
        })
    }
}
// product
impl Node for ruff::Parameter {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            name,
            annotation,
            // type_comment,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeArg::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("arg", name.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("annotation", annotation.ast_to_object(_vm), _vm)
            .unwrap();
        // dict.set_item("type_comment", type_comment.ast_to_object(_vm), _vm)
        //     .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            name: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "arg", "arg")?)?,
            annotation: get_node_field_opt(_vm, &_object, "annotation")?
                .map(|obj| Node::ast_from_object(_vm, obj))
                .transpose()?,
            // type_comment: get_node_field_opt(_vm, &_object, "type_comment")?
            //     .map(|obj| Node::ast_from_object(_vm, obj))
            //     .transpose()?,
            range: range_from_object(_vm, _object, "arg")?,
        })
    }
}
impl Node for ruff::ParameterWithDefault {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        todo!()
    }

    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        todo!()
    }
}
// product
impl Node for ruff::Keyword {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            arg,
            value,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeKeyword::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("arg", arg.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("value", value.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            arg: get_node_field_opt(_vm, &_object, "arg")?
                .map(|obj| Node::ast_from_object(_vm, obj))
                .transpose()?,
            value: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "value", "keyword")?)?,
            range: range_from_object(_vm, _object, "keyword")?,
        })
    }
}

struct PositionalParameters {
    pub range: TextRange,
    pub args: Box<[ruff::Parameter]>,
}

impl Node for PositionalParameters {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        BoxedSlice(self.args).ast_to_object(vm)
    }

    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        todo!()
    }
}

struct KeywordParameters {
    pub range: TextRange,
    pub keywords: Box<[ruff::Parameter]>,
}

impl Node for KeywordParameters {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        BoxedSlice(self.keywords).ast_to_object(vm)
    }

    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        todo!()
    }
}

struct ParameterDefaults {
    pub range: TextRange,
    defaults: Box<[Option<Box<ruff::Expr>>]>,
}

impl Node for ParameterDefaults {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        BoxedSlice(self.defaults).ast_to_object(vm)
    }

    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        todo!()
    }
}

fn extract_positional_parameter_defaults(
    pos_only_args: Vec<ruff::ParameterWithDefault>,
    args: Vec<ruff::ParameterWithDefault>,
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
        range: defaults
            .iter()
            .flatten()
            .map(|item| item.range())
            .reduce(|acc, next| acc.cover(next))
            .unwrap_or_default(),
        defaults: defaults.into_boxed_slice(),
    };

    let pos_only_args = PositionalParameters {
        range: pos_only_args
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
        range: args
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
    posonlyargs: PositionalParameters,
    args: PositionalParameters,
    defaults: ParameterDefaults,
) -> (
    Vec<ruff::ParameterWithDefault>,
    Vec<ruff::ParameterWithDefault>,
) {
    let posonlyargs = posonlyargs.args;
    let args = args.args;
    let defaults = defaults.defaults;

    let mut posonlyargs: Vec<_> = <Box<[_]> as IntoIterator>::into_iter(posonlyargs)
        .map(|parameter| ruff::ParameterWithDefault {
            range: Default::default(),
            parameter,
            default: None,
        })
        .collect();
    let mut args: Vec<_> = <Box<[_]> as IntoIterator>::into_iter(args)
        .map(|parameter| ruff::ParameterWithDefault {
            range: Default::default(),
            parameter,
            default: None,
        })
        .collect();

    // If an argument has a default value, insert it
    // Note that "defaults" will only contain default values for the last "n" parameters
    // so we need to skip the first "total_argument_count - n" arguments.
    let default_argument_count = posonlyargs.len() + args.len() - defaults.len();
    for (arg, default) in posonlyargs
        .iter_mut()
        .chain(args.iter_mut())
        .skip(default_argument_count)
        .zip(defaults)
    {
        arg.default = default;
    }

    (posonlyargs, args)
}

fn extract_keyword_parameter_defaults(
    kw_only_args: Vec<ruff::ParameterWithDefault>,
) -> (KeywordParameters, ParameterDefaults) {
    let mut defaults = vec![];
    defaults.extend(kw_only_args.iter().map(|item| item.default.clone()));
    let defaults = ParameterDefaults {
        range: defaults
            .iter()
            .flatten()
            .map(|item| item.range())
            .reduce(|acc, next| acc.cover(next))
            .unwrap_or_default(),
        defaults: defaults.into_boxed_slice(),
    };

    let kw_only_args = KeywordParameters {
        range: kw_only_args
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
    kw_only_args: KeywordParameters,
    defaults: ParameterDefaults,
) -> Vec<ruff::ParameterWithDefault> {
    std::iter::zip(kw_only_args.keywords, defaults.defaults)
        .map(|(parameter, default)| ruff::ParameterWithDefault {
            parameter,
            default,
            range: Default::default(),
        })
        .collect()
}
