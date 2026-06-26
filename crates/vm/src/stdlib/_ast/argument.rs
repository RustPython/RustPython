use super::*;
use rustpython_compiler_core::SourceFile;

pub(super) struct PositionalArguments {
    range: TextRange,
    kind: PositionalArgumentsKind,
}

enum PositionalArgumentsKind {
    Args(Box<[ast::Expr]>),
    RuntimeValues(Vec<Option<ast::Expr>>),
}

impl PositionalArguments {
    pub(super) fn ast_from_field(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: &PyObject,
        field: &'static str,
        typ: &str,
    ) -> PyResult<Self> {
        let values: Vec<Option<ast::Expr>> =
            get_node_list_field(vm, source_file, object, field, typ)?;
        Ok(Self::from_values(TextRange::default(), values))
    }

    fn from_args(range: TextRange, args: Box<[ast::Expr]>) -> Self {
        Self {
            range,
            kind: PositionalArgumentsKind::Args(args),
        }
    }

    fn from_runtime_values(range: TextRange, values: Vec<Option<ast::Expr>>) -> Self {
        Self {
            range,
            kind: PositionalArgumentsKind::RuntimeValues(values),
        }
    }

    fn from_values(range: TextRange, values: Vec<Option<ast::Expr>>) -> Self {
        if values.iter().any(Option::is_none) {
            Self::from_runtime_values(range, values)
        } else {
            Self::from_args(
                range,
                values
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            )
        }
    }

    fn range(&self) -> TextRange {
        self.range
    }

    fn into_args_and_runtime_values(self) -> (Box<[ast::Expr]>, Option<Vec<Option<ast::Expr>>>) {
        match self.kind {
            PositionalArgumentsKind::Args(args) => (args, None),
            PositionalArgumentsKind::RuntimeValues(values) => (
                lower_runtime_expr_list(values.clone()).into_boxed_slice(),
                Some(values),
            ),
        }
    }
}

impl Node for PositionalArguments {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        match self.kind {
            PositionalArgumentsKind::Args(args) => BoxedSlice(args).ast_to_object(vm, source_file),
            PositionalArgumentsKind::RuntimeValues(values) => values.ast_to_object(vm, source_file),
        }
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let args: BoxedSlice<_> = Node::ast_from_object(vm, source_file, object)?;
        Ok(Self::from_args(TextRange::default(), args.0))
    }
}

pub(super) struct KeywordArguments {
    pub range: TextRange,
    pub keywords: Box<[ast::Keyword]>,
}

impl KeywordArguments {
    pub(super) fn ast_from_field(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: &PyObject,
        field: &'static str,
        typ: &str,
    ) -> PyResult<Self> {
        Ok(Self {
            keywords: get_node_boxed_slice_field(vm, source_file, object, field, typ)?,
            range: TextRange::default(),
        })
    }
}

impl Node for KeywordArguments {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self { keywords, range: _ } = self;
        // TODO: use range
        BoxedSlice(keywords).ast_to_object(vm, source_file)
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let keywords: BoxedSlice<_> = Node::ast_from_object(vm, source_file, object)?;
        Ok(Self {
            keywords: keywords.0,
            range: TextRange::default(),
        })
    }
}

pub(super) fn merge_function_call_arguments(
    pos_args: PositionalArguments,
    key_args: KeywordArguments,
) -> ast::Arguments {
    let range = pos_args.range().cover(key_args.range);
    let (args, runtime_args) = pos_args.into_args_and_runtime_values();

    ast::Arguments {
        node_index: Default::default(),
        range,
        args,
        keywords: key_args.keywords,
        runtime_args,
        runtime_bases: None,
    }
}

pub(super) fn split_function_call_arguments(
    args: ast::Arguments,
) -> (PositionalArguments, KeywordArguments) {
    let ast::Arguments {
        range: _,
        args,
        keywords,
        runtime_args,
        runtime_bases: _,
        ..
    } = args;

    let positional_arguments_range = args
        .iter()
        .map(|item| item.range())
        .reduce(|acc, next| acc.cover(next))
        .unwrap_or_default();
    // debug_assert!(range.contains_range(positional_arguments_range));
    let positional_arguments = runtime_args.map_or_else(
        || PositionalArguments::from_args(positional_arguments_range, args),
        |values| PositionalArguments::from_runtime_values(positional_arguments_range, values),
    );

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
    args: Option<Box<ast::Arguments>>,
) -> (Option<PositionalArguments>, Option<KeywordArguments>) {
    let args = match args {
        None => return (None, None),
        Some(args) => *args,
    };
    let ast::Arguments {
        range: _,
        args,
        keywords,
        runtime_args: _,
        runtime_bases,
        ..
    } = args;

    let positional_arguments_range = args
        .iter()
        .map(|item| item.range())
        .reduce(|acc, next| acc.cover(next))
        .unwrap_or_default();
    // debug_assert!(range.contains_range(positional_arguments_range));
    let positional_arguments = runtime_bases.map_or_else(
        || PositionalArguments::from_args(positional_arguments_range, args),
        |values| PositionalArguments::from_runtime_values(positional_arguments_range, values),
    );

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
) -> Option<Box<ast::Arguments>> {
    if positional_arguments.is_none() && keyword_arguments.is_none() {
        return None;
    }

    let (args, runtime_bases) = if let Some(positional_arguments) = positional_arguments {
        positional_arguments.into_args_and_runtime_values()
    } else {
        (vec![].into_boxed_slice(), None)
    };
    let keywords = if let Some(keyword_arguments) = keyword_arguments {
        keyword_arguments.keywords
    } else {
        vec![].into_boxed_slice()
    };

    Some(Box::new(ast::Arguments {
        node_index: Default::default(),
        range: Default::default(), // TODO
        args,
        keywords,
        runtime_args: None,
        runtime_bases,
    }))
}
