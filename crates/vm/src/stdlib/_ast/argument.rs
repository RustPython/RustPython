use thin_vec::ThinVec;

use super::*;
use rustpython_compiler_core::SourceFile;

pub(super) struct PositionalArguments {
    pub range: TextRange,
    pub args: ThinVec<ast::Expr>,
}

impl Node for PositionalArguments {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self { args, range: _ } = self;
        args.ast_to_object(vm, source_file)
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let args: ThinVec<_> = Node::ast_from_object(vm, source_file, object)?;
        Ok(Self {
            args,
            range: TextRange::default(), // TODO
        })
    }
}

pub(super) struct KeywordArguments {
    pub range: TextRange,
    pub keywords: ThinVec<ast::Keyword>,
}

impl Node for KeywordArguments {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self { keywords, range: _ } = self;
        // TODO: use range
        keywords.ast_to_object(vm, source_file)
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let keywords: ThinVec<_> = Node::ast_from_object(vm, source_file, object)?;
        Ok(Self {
            keywords,
            range: TextRange::default(), // TODO
        })
    }
}

pub(super) fn merge_function_call_arguments(
    pos_args: PositionalArguments,
    key_args: KeywordArguments,
) -> ast::Arguments {
    let range = pos_args.range.cover(key_args.range);

    ast::Arguments {
        node_index: Default::default(),
        range,
        args: pos_args.args.into(),
        keywords: key_args.keywords,
    }
}

pub(super) fn split_function_call_arguments(
    args: ast::Arguments,
) -> (PositionalArguments, KeywordArguments) {
    let ast::Arguments {
        node_index: _,
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
        args: args.into(),
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
    args: Option<Box<ast::Arguments>>,
) -> (Option<PositionalArguments>, Option<KeywordArguments>) {
    let args = match args {
        None => return (None, None),
        Some(args) => *args,
    };
    let ast::Arguments {
        node_index: _,
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
        args: args.into(),
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
) -> Option<Box<ast::Arguments>> {
    if positional_arguments.is_none() && keyword_arguments.is_none() {
        return None;
    }

    let args = if let Some(positional_arguments) = positional_arguments {
        positional_arguments.args
    } else {
        ThinVec::new()
    };
    let keywords = if let Some(keyword_arguments) = keyword_arguments {
        keyword_arguments.keywords
    } else {
        ThinVec::new()
    };

    Some(Box::new(ast::Arguments {
        node_index: Default::default(),
        range: Default::default(), // TODO
        args: args.into(),
        keywords,
    }))
}
