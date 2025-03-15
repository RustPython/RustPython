use super::*;

pub(super) struct PositionalArguments {
    pub range: TextRange,
    pub args: Box<[ruff::Expr]>,
}

impl Node for PositionalArguments {
    fn ast_to_object(self, vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let Self { args, range: _ } = self;
        BoxedSlice(args).ast_to_object(vm, source_code)
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let args: BoxedSlice<_> = Node::ast_from_object(vm, source_code, object)?;
        Ok(Self {
            args: args.0,
            range: TextRange::default(), // TODO
        })
    }
}

pub(super) struct KeywordArguments {
    pub range: TextRange,
    pub keywords: Box<[ruff::Keyword]>,
}

impl Node for KeywordArguments {
    fn ast_to_object(self, vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let Self { keywords, range: _ } = self;
        // TODO: use range
        BoxedSlice(keywords).ast_to_object(vm, source_code)
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let keywords: BoxedSlice<_> = Node::ast_from_object(vm, source_code, object)?;
        Ok(Self {
            keywords: keywords.0,
            range: TextRange::default(), // TODO
        })
    }
}

pub(super) fn merge_function_call_arguments(
    pos_args: PositionalArguments,
    key_args: KeywordArguments,
) -> ruff::Arguments {
    let range = pos_args.range.cover(key_args.range);

    ruff::Arguments {
        range,
        args: pos_args.args,
        keywords: key_args.keywords,
    }
}

pub(super) fn split_function_call_arguments(
    args: ruff::Arguments,
) -> (PositionalArguments, KeywordArguments) {
    let ruff::Arguments {
        range: _,
        args,
        keywords,
    } = args;

    let positional_arguments_range = args
        .iter()
        .map(|item| item.range())
        .reduce(|acc, next| acc.cover(next))
        .unwrap_or_default();
    // debug_assert!(range.contains_range(positional_arguments_range));
    let positional_arguments = PositionalArguments {
        range: positional_arguments_range,
        args,
    };

    let keyword_arguments_range = keywords
        .iter()
        .map(|item| item.range())
        .reduce(|acc, next| acc.cover(next))
        .unwrap_or_default();
    // debug_assert!(range.contains_range(keyword_arguments_range));
    let keyword_arguments = KeywordArguments {
        range: keyword_arguments_range,
        keywords,
    };

    (positional_arguments, keyword_arguments)
}

pub(super) fn split_class_def_args(
    args: Option<Box<ruff::Arguments>>,
) -> (Option<PositionalArguments>, Option<KeywordArguments>) {
    let args = match args {
        None => return (None, None),
        Some(args) => *args,
    };
    let ruff::Arguments {
        range: _,
        args,
        keywords,
    } = args;

    let positional_arguments_range = args
        .iter()
        .map(|item| item.range())
        .reduce(|acc, next| acc.cover(next))
        .unwrap_or_default();
    // debug_assert!(range.contains_range(positional_arguments_range));
    let positional_arguments = PositionalArguments {
        range: positional_arguments_range,
        args,
    };

    let keyword_arguments_range = keywords
        .iter()
        .map(|item| item.range())
        .reduce(|acc, next| acc.cover(next))
        .unwrap_or_default();
    // debug_assert!(range.contains_range(keyword_arguments_range));
    let keyword_arguments = KeywordArguments {
        range: keyword_arguments_range,
        keywords,
    };

    (Some(positional_arguments), Some(keyword_arguments))
}

pub(super) fn merge_class_def_args(
    positional_arguments: Option<PositionalArguments>,
    keyword_arguments: Option<KeywordArguments>,
) -> Option<Box<ruff::Arguments>> {
    if positional_arguments.is_none() && keyword_arguments.is_none() {
        return None;
    }

    let args = if let Some(positional_arguments) = positional_arguments {
        positional_arguments.args
    } else {
        vec![].into_boxed_slice()
    };
    let keywords = if let Some(keyword_arguments) = keyword_arguments {
        keyword_arguments.keywords
    } else {
        vec![].into_boxed_slice()
    };

    Some(Box::new(ruff::Arguments {
        range: Default::default(), // TODO
        args,
        keywords,
    }))
}
