use super::*;
use rustpython_compiler_core::SourceFile;

pub(super) struct PositionalArguments {
    pub node_index: ast::AtomicNodeIndex,
    pub field: super::constant::PublicAstExprListField,
    pub range: TextRange,
    pub args: Box<[ast::Expr]>,
}

impl PositionalArguments {
    pub(super) fn ast_from_field(
        ctx: &AstFromObjectContext<'_>,
        source_file: &SourceFile,
        object: &PyObject,
        field: &'static str,
        typ: &str,
    ) -> PyResult<Self> {
        let args: Vec<Option<ast::Expr>> =
            get_node_list_field(ctx, source_file, object, field, typ)?;
        let public_field = match field {
            "bases" => super::constant::PublicAstExprListField::Bases,
            _ => super::constant::PublicAstExprListField::Args,
        };
        let (node_index, args) = public_expr_boxed_slice_from_values(ctx, public_field, args);
        Ok(Self {
            node_index,
            field: public_field,
            args,
            range: TextRange::default(),
        })
    }
}

impl Node for PositionalArguments {
    fn ast_to_object(self, to_ctx: &AstToObjectContext<'_>) -> PyObjectRef {
        let _vm = to_ctx.vm;
        let _source_file = to_ctx.source_file;
        let Self {
            node_index,
            field,
            args,
            range: _,
        } = self;
        super::constant::public_ast_expr_list_object(to_ctx, node_index.load(), field).map_or_else(
            || BoxedSlice(args).ast_to_object(to_ctx),
            |values| values.values.ast_to_object(to_ctx),
        )
    }

    fn ast_from_object(
        ctx: &AstFromObjectContext<'_>,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let args: BoxedSlice<_> = Node::ast_from_object(ctx, source_file, object)?;
        Ok(Self {
            node_index: Default::default(),
            field: super::constant::PublicAstExprListField::Args,
            args: args.0,
            range: TextRange::default(),
        })
    }
}

pub(super) struct KeywordArguments {
    pub range: TextRange,
    pub keywords: Box<[ast::Keyword]>,
}

impl KeywordArguments {
    pub(super) fn ast_from_field(
        ctx: &AstFromObjectContext<'_>,
        source_file: &SourceFile,
        object: &PyObject,
        field: &'static str,
        typ: &str,
    ) -> PyResult<Self> {
        Ok(Self {
            keywords: get_node_boxed_slice_field(ctx, source_file, object, field, typ)?,
            range: TextRange::default(),
        })
    }
}

impl Node for KeywordArguments {
    fn ast_to_object(self, to_ctx: &AstToObjectContext<'_>) -> PyObjectRef {
        let _vm = to_ctx.vm;
        let _source_file = to_ctx.source_file;
        let Self { keywords, range: _ } = self;
        // TODO: use range
        BoxedSlice(keywords).ast_to_object(to_ctx)
    }

    fn ast_from_object(
        ctx: &AstFromObjectContext<'_>,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let keywords: BoxedSlice<_> = Node::ast_from_object(ctx, source_file, object)?;
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
    let range = pos_args.range.cover(key_args.range);

    ast::Arguments {
        node_index: pos_args.node_index,
        range,
        args: pos_args.args,
        keywords: key_args.keywords.into(),
    }
}

pub(super) fn split_function_call_arguments(
    args: ast::Arguments,
) -> (PositionalArguments, KeywordArguments) {
    let ast::Arguments {
        node_index,
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
        node_index,
        field: super::constant::PublicAstExprListField::Args,
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
        keywords: keywords.into(),
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
        node_index,
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
        node_index,
        field: super::constant::PublicAstExprListField::Bases,
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
        keywords: keywords.into(),
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

    let (node_index, args) = if let Some(positional_arguments) = positional_arguments {
        (positional_arguments.node_index, positional_arguments.args)
    } else {
        (Default::default(), vec![].into_boxed_slice())
    };
    let keywords = if let Some(keyword_arguments) = keyword_arguments {
        keyword_arguments.keywords
    } else {
        vec![].into_boxed_slice()
    };

    Some(Box::new(ast::Arguments {
        node_index,
        range: Default::default(), // TODO
        args,
        keywords: keywords.into(),
    }))
}
