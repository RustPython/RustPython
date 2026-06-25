use super::*;
use crate::stdlib::_ast::argument::{
    KeywordArguments, PositionalArguments, merge_class_def_args, split_class_def_args,
};
use crate::stdlib::_ast::exception::except_handler_from_object_unvalidated_range;
use crate::stdlib::_ast::type_parameters::type_params_from_field;
use rustpython_compiler_core::SourceFile;

fn runtime_decorator_expr_list(values: &[Option<ast::Decorator>]) -> Vec<Option<ast::Expr>> {
    values
        .iter()
        .map(|value| value.as_ref().map(|decorator| decorator.expression.clone()))
        .collect()
}

fn lower_runtime_decorator_list(values: Vec<Option<ast::Decorator>>) -> Vec<ast::Decorator> {
    values
        .into_iter()
        .map(|value| {
            value.unwrap_or_else(|| ast::Decorator {
                range: Default::default(),
                node_index: Default::default(),
                expression: runtime_null_expr_placeholder(),
            })
        })
        .collect()
}

fn definition_range_from_name(
    source_file: &SourceFile,
    name_start: TextSize,
    end: TextSize,
    keyword: &str,
) -> TextRange {
    let source_code = source_file.to_source_code();
    let line = source_code.line_index(name_start);
    let line_start = source_code.line_start(line);
    let keyword_start = source_code
        .slice(TextRange::new(line_start, name_start))
        .rfind(keyword)
        .map_or(line_start, |offset| {
            line_start + TextSize::new(offset as u32)
        });
    TextRange::new(keyword_start, end)
}

fn runtime_stmt_type_comment(
    vm: &VirtualMachine,
    type_comment: Option<PyObjectRef>,
) -> (Option<Box<str>>, Option<Vec<u8>>) {
    type_comment.map_or((None, None), |type_comment| {
        super::constant::runtime_string_from_pyobject(vm, type_comment)
    })
}

fn runtime_stmt_type_comment_object(
    vm: &VirtualMachine,
    value: Option<Box<str>>,
    bytes: Option<Vec<u8>>,
) -> PyObjectRef {
    super::constant::runtime_stmt_type_comment_object(vm, value, bytes)
        .unwrap_or_else(|| vm.ctx.none())
}

// sum
impl Node for ast::Stmt {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        match self {
            Self::FunctionDef(cons) => cons.ast_to_object(vm, source_file),
            Self::ClassDef(cons) => cons.ast_to_object(vm, source_file),
            Self::Return(cons) => cons.ast_to_object(vm, source_file),
            Self::Delete(cons) => cons.ast_to_object(vm, source_file),
            Self::Assign(cons) => cons.ast_to_object(vm, source_file),
            Self::TypeAlias(cons) => cons.ast_to_object(vm, source_file),
            Self::AugAssign(cons) => cons.ast_to_object(vm, source_file),
            Self::AnnAssign(cons) => cons.ast_to_object(vm, source_file),
            Self::For(cons) => cons.ast_to_object(vm, source_file),
            Self::While(cons) => cons.ast_to_object(vm, source_file),
            Self::If(cons) => cons.ast_to_object(vm, source_file),
            Self::With(cons) => cons.ast_to_object(vm, source_file),
            Self::Match(cons) => cons.ast_to_object(vm, source_file),
            Self::Raise(cons) => cons.ast_to_object(vm, source_file),
            Self::Try(cons) => cons.ast_to_object(vm, source_file),
            Self::Assert(cons) => cons.ast_to_object(vm, source_file),
            Self::Import(cons) => cons.ast_to_object(vm, source_file),
            Self::ImportFrom(cons) => cons.ast_to_object(vm, source_file),
            Self::Global(cons) => cons.ast_to_object(vm, source_file),
            Self::Nonlocal(cons) => cons.ast_to_object(vm, source_file),
            Self::Expr(cons) => cons.ast_to_object(vm, source_file),
            Self::Pass(cons) => cons.ast_to_object(vm, source_file),
            Self::Break(cons) => cons.ast_to_object(vm, source_file),
            Self::Continue(cons) => cons.ast_to_object(vm, source_file),
            Self::IpyEscapeCommand(_) => {
                unreachable!("IPython escape command is not part of Python AST")
            }
        }
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        if vm.is_none(&object) {
            return Err(vm.new_value_error("None disallowed in statement list"));
        }
        enum StmtKind {
            FunctionDef { is_async: bool },
            ClassDef,
            Return,
            Delete,
            Assign,
            TypeAlias,
            AugAssign,
            AnnAssign,
            For { is_async: bool },
            While,
            If,
            With { is_async: bool },
            Match,
            Raise,
            Try { is_star: bool },
            Assert,
            Import,
            ImportFrom,
            Global,
            Nonlocal,
            Expr,
            Pass,
            Break,
            Continue,
        }
        let kind = if is_node_instance(vm, &object, pyast::NodeStmtFunctionDef::static_type())? {
            StmtKind::FunctionDef { is_async: false }
        } else if is_node_instance(vm, &object, pyast::NodeStmtAsyncFunctionDef::static_type())? {
            StmtKind::FunctionDef { is_async: true }
        } else if is_node_instance(vm, &object, pyast::NodeStmtClassDef::static_type())? {
            StmtKind::ClassDef
        } else if is_node_instance(vm, &object, pyast::NodeStmtReturn::static_type())? {
            StmtKind::Return
        } else if is_node_instance(vm, &object, pyast::NodeStmtDelete::static_type())? {
            StmtKind::Delete
        } else if is_node_instance(vm, &object, pyast::NodeStmtAssign::static_type())? {
            StmtKind::Assign
        } else if is_node_instance(vm, &object, pyast::NodeStmtTypeAlias::static_type())? {
            StmtKind::TypeAlias
        } else if is_node_instance(vm, &object, pyast::NodeStmtAugAssign::static_type())? {
            StmtKind::AugAssign
        } else if is_node_instance(vm, &object, pyast::NodeStmtAnnAssign::static_type())? {
            StmtKind::AnnAssign
        } else if is_node_instance(vm, &object, pyast::NodeStmtFor::static_type())? {
            StmtKind::For { is_async: false }
        } else if is_node_instance(vm, &object, pyast::NodeStmtAsyncFor::static_type())? {
            StmtKind::For { is_async: true }
        } else if is_node_instance(vm, &object, pyast::NodeStmtWhile::static_type())? {
            StmtKind::While
        } else if is_node_instance(vm, &object, pyast::NodeStmtIf::static_type())? {
            StmtKind::If
        } else if is_node_instance(vm, &object, pyast::NodeStmtWith::static_type())? {
            StmtKind::With { is_async: false }
        } else if is_node_instance(vm, &object, pyast::NodeStmtAsyncWith::static_type())? {
            StmtKind::With { is_async: true }
        } else if is_node_instance(vm, &object, pyast::NodeStmtMatch::static_type())? {
            StmtKind::Match
        } else if is_node_instance(vm, &object, pyast::NodeStmtRaise::static_type())? {
            StmtKind::Raise
        } else if is_node_instance(vm, &object, pyast::NodeStmtTry::static_type())? {
            StmtKind::Try { is_star: false }
        } else if is_node_instance(vm, &object, pyast::NodeStmtTryStar::static_type())? {
            StmtKind::Try { is_star: true }
        } else if is_node_instance(vm, &object, pyast::NodeStmtAssert::static_type())? {
            StmtKind::Assert
        } else if is_node_instance(vm, &object, pyast::NodeStmtImport::static_type())? {
            StmtKind::Import
        } else if is_node_instance(vm, &object, pyast::NodeStmtImportFrom::static_type())? {
            StmtKind::ImportFrom
        } else if is_node_instance(vm, &object, pyast::NodeStmtGlobal::static_type())? {
            StmtKind::Global
        } else if is_node_instance(vm, &object, pyast::NodeStmtNonlocal::static_type())? {
            StmtKind::Nonlocal
        } else if is_node_instance(vm, &object, pyast::NodeStmtExpr::static_type())? {
            StmtKind::Expr
        } else if is_node_instance(vm, &object, pyast::NodeStmtPass::static_type())? {
            StmtKind::Pass
        } else if is_node_instance(vm, &object, pyast::NodeStmtBreak::static_type())? {
            StmtKind::Break
        } else if is_node_instance(vm, &object, pyast::NodeStmtContinue::static_type())? {
            StmtKind::Continue
        } else {
            return Err(vm.new_type_error(format!(
                "expected some sort of stmt, but got {}",
                object.repr(vm)?
            )));
        };
        let range = stmt_range_from_object(vm, source_file, object.clone())?;
        Ok(match kind {
            StmtKind::FunctionDef { is_async } => Self::FunctionDef(
                stmt_function_def_from_object_with_range(vm, source_file, object, range, is_async)?,
            ),
            StmtKind::ClassDef => Self::ClassDef(stmt_class_def_from_object_with_range(
                vm,
                source_file,
                object,
                range,
            )?),
            StmtKind::Return => Self::Return(stmt_return_from_object_with_range(
                vm,
                source_file,
                object,
                range,
            )?),
            StmtKind::Delete => Self::Delete(stmt_delete_from_object_with_range(
                vm,
                source_file,
                object,
                range,
            )?),
            StmtKind::Assign => Self::Assign(stmt_assign_from_object_with_range(
                vm,
                source_file,
                object,
                range,
            )?),
            StmtKind::TypeAlias => Self::TypeAlias(stmt_type_alias_from_object_with_range(
                vm,
                source_file,
                object,
                range,
            )?),
            StmtKind::AugAssign => Self::AugAssign(stmt_aug_assign_from_object_with_range(
                vm,
                source_file,
                object,
                range,
            )?),
            StmtKind::AnnAssign => Self::AnnAssign(stmt_ann_assign_from_object_with_range(
                vm,
                source_file,
                object,
                range,
            )?),
            StmtKind::For { is_async } => Self::For(stmt_for_from_object_with_range(
                vm,
                source_file,
                object,
                range,
                is_async,
            )?),
            StmtKind::While => Self::While(stmt_while_from_object_with_range(
                vm,
                source_file,
                object,
                range,
            )?),
            StmtKind::If => Self::If(elif_else_clause::ast_from_object_with_range(
                vm,
                source_file,
                object,
                range,
            )?),
            StmtKind::With { is_async } => Self::With(stmt_with_from_object_with_range(
                vm,
                source_file,
                object,
                range,
                is_async,
            )?),
            StmtKind::Match => Self::Match(stmt_match_from_object_with_range(
                vm,
                source_file,
                object,
                range,
            )?),
            StmtKind::Raise => Self::Raise(stmt_raise_from_object_with_range(
                vm,
                source_file,
                object,
                range,
            )?),
            StmtKind::Try { is_star } => Self::Try(stmt_try_from_object_with_range(
                vm,
                source_file,
                object,
                range,
                is_star,
            )?),
            StmtKind::Assert => Self::Assert(stmt_assert_from_object_with_range(
                vm,
                source_file,
                object,
                range,
            )?),
            StmtKind::Import => Self::Import(stmt_import_from_object_with_range(
                vm,
                source_file,
                object,
                range,
            )?),
            StmtKind::ImportFrom => Self::ImportFrom(stmt_import_from_from_object_with_range(
                vm,
                source_file,
                object,
                range,
            )?),
            StmtKind::Global => Self::Global(stmt_global_from_object_with_range(
                vm,
                source_file,
                object,
                range,
            )?),
            StmtKind::Nonlocal => Self::Nonlocal(stmt_nonlocal_from_object_with_range(
                vm,
                source_file,
                object,
                range,
            )?),
            StmtKind::Expr => Self::Expr(stmt_expr_from_object_with_range(
                vm,
                source_file,
                object,
                range,
            )?),
            StmtKind::Pass => Self::Pass(stmt_pass_from_object_with_range(range)),
            StmtKind::Break => Self::Break(stmt_break_from_object_with_range(range)),
            StmtKind::Continue => Self::Continue(stmt_continue_from_object_with_range(range)),
        })
    }
}

// constructor
fn stmt_function_def_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
    is_async: bool,
) -> PyResult<ast::StmtFunctionDef> {
    let typ = if is_async {
        "AsyncFunctionDef"
    } else {
        "FunctionDef"
    };
    let name = get_required_identifier_field(vm, source_file, &object, "name", typ)?;
    let parameters = Node::ast_from_object(
        vm,
        source_file,
        get_node_field_required(vm, &object, "args", typ)?,
    )?;
    let body: Vec<Option<ast::Stmt>> = get_node_list_field(vm, source_file, &object, "body", typ)?;
    let decorator_list: Vec<Option<ast::Decorator>> =
        get_node_list_field(vm, source_file, &object, "decorator_list", typ)?;
    let runtime_decorator_exprs = runtime_decorator_expr_list(&decorator_list);
    let runtime_body = runtime_stmt_list_metadata(&body);
    let runtime_decorator_list = runtime_expr_list_metadata(&runtime_decorator_exprs);
    let body = lower_runtime_stmt_list(body);
    let decorator_list = lower_runtime_decorator_list(decorator_list);
    let returns = get_node_field_opt(vm, &object, "returns")?
        .map(|obj| Node::ast_from_object(vm, source_file, obj))
        .transpose()?;
    let (runtime_type_comment, runtime_type_comment_bytes) =
        runtime_stmt_type_comment(vm, get_ast_string_field_opt(vm, &object, "type_comment")?);
    let type_params = type_params_from_field(vm, source_file, &object, "type_params", typ)?;
    Ok(ast::StmtFunctionDef {
        node_index: Default::default(),
        name,
        parameters,
        body,
        decorator_list,
        returns,
        type_params,
        range,
        is_async,
        runtime_decorator_list,
        runtime_type_comment,
        runtime_type_comment_bytes,
        runtime_body,
    })
}

impl Node for ast::StmtFunctionDef {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            name,
            parameters,
            body,
            decorator_list,
            returns,
            type_params,
            is_async,
            range,
            runtime_decorator_list,
            runtime_type_comment,
            runtime_type_comment_bytes,
            runtime_body,
        } = self;
        let range = definition_range_from_name(
            source_file,
            name.range.start(),
            range.end(),
            if is_async { "async" } else { "def" },
        );

        let cls = if !is_async {
            pyast::NodeStmtFunctionDef::static_type().to_owned()
        } else {
            pyast::NodeStmtAsyncFunctionDef::static_type().to_owned()
        };

        let node = NodeAst.into_ref_with_type(vm, cls).unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("name", vm.ctx.new_str(name.as_str()).to_pyobject(vm), vm)
            .unwrap();
        dict.set_item("args", parameters.ast_to_object(vm, source_file), vm)
            .unwrap();
        let body = runtime_body.map_or_else(
            || body.ast_to_object(vm, source_file),
            |values| values.ast_to_object(vm, source_file),
        );
        dict.set_item("body", body, vm).unwrap();
        dict.set_item(
            "decorator_list",
            runtime_decorator_list.map_or_else(
                || decorator_list.ast_to_object(vm, source_file),
                |values| values.ast_to_object(vm, source_file),
            ),
            vm,
        )
        .unwrap();
        dict.set_item("returns", returns.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item(
            "type_comment",
            runtime_stmt_type_comment_object(vm, runtime_type_comment, runtime_type_comment_bytes),
            vm,
        )
        .unwrap();
        dict.set_item(
            "type_params",
            type_params.map_or_else(
                || vm.ctx.new_list(vec![]).into(),
                |tp| tp.ast_to_object(vm, source_file),
            ),
            vm,
        )
        .unwrap();
        node_add_location(&dict, range, vm, source_file);
        node.into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        let is_async =
            is_node_instance(vm, &_object, pyast::NodeStmtAsyncFunctionDef::static_type())?;
        let typ = if is_async {
            "AsyncFunctionDef"
        } else {
            "FunctionDef"
        };
        let range = range_from_object(vm, source_file, _object.clone(), typ)?;
        stmt_function_def_from_object_with_range(vm, source_file, _object, range, is_async)
    }
}

// constructor
fn stmt_class_def_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::StmtClassDef> {
    let name = get_required_identifier_field(vm, source_file, &object, "name", "ClassDef")?;
    let bases = PositionalArguments::ast_from_field(vm, source_file, &object, "bases", "ClassDef")?;
    let keywords =
        KeywordArguments::ast_from_field(vm, source_file, &object, "keywords", "ClassDef")?;
    let body: Vec<Option<ast::Stmt>> =
        get_node_list_field(vm, source_file, &object, "body", "ClassDef")?;
    let decorator_list: Vec<Option<ast::Decorator>> =
        get_node_list_field(vm, source_file, &object, "decorator_list", "ClassDef")?;
    let runtime_decorator_exprs = runtime_decorator_expr_list(&decorator_list);
    let runtime_body = runtime_stmt_list_metadata(&body);
    let runtime_decorator_list = runtime_expr_list_metadata(&runtime_decorator_exprs);
    let body = lower_runtime_stmt_list(body);
    let decorator_list = lower_runtime_decorator_list(decorator_list);
    let type_params = type_params_from_field(vm, source_file, &object, "type_params", "ClassDef")?;
    Ok(ast::StmtClassDef {
        node_index: Default::default(),
        name,
        arguments: merge_class_def_args(Some(bases), Some(keywords)),
        body,
        decorator_list,
        type_params,
        range,
        runtime_decorator_list,
        runtime_body,
    })
}

impl Node for ast::StmtClassDef {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            name,
            arguments,
            body,
            decorator_list,
            type_params,
            range,
            runtime_decorator_list,
            runtime_body,
        } = self;
        let (bases, keywords) = split_class_def_args(arguments);
        let range =
            definition_range_from_name(source_file, name.range.start(), range.end(), "class");
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeStmtClassDef::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("name", name.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item(
            "bases",
            bases.map_or_else(
                || vm.ctx.new_list(vec![]).into(),
                |b| b.ast_to_object(vm, source_file),
            ),
            vm,
        )
        .unwrap();
        dict.set_item(
            "keywords",
            keywords.map_or_else(
                || vm.ctx.new_list(vec![]).into(),
                |k| k.ast_to_object(vm, source_file),
            ),
            vm,
        )
        .unwrap();
        let body = runtime_body.map_or_else(
            || body.ast_to_object(vm, source_file),
            |values| values.ast_to_object(vm, source_file),
        );
        dict.set_item("body", body, vm).unwrap();
        dict.set_item(
            "decorator_list",
            runtime_decorator_list.map_or_else(
                || decorator_list.ast_to_object(vm, source_file),
                |values| values.ast_to_object(vm, source_file),
            ),
            vm,
        )
        .unwrap();
        dict.set_item(
            "type_params",
            type_params.map_or_else(
                || vm.ctx.new_list(vec![]).into(),
                |tp| tp.ast_to_object(vm, source_file),
            ),
            vm,
        )
        .unwrap();
        node_add_location(&dict, range, vm, source_file);
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, _object.clone(), "ClassDef")?;
        stmt_class_def_from_object_with_range(vm, source_file, _object, range)
    }
}
// constructor
fn stmt_return_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::StmtReturn> {
    Ok(ast::StmtReturn {
        node_index: Default::default(),
        value: get_node_field_opt(vm, &object, "value")?
            .map(|obj| Node::ast_from_object(vm, source_file, obj))
            .transpose()?,
        range,
    })
}

impl Node for ast::StmtReturn {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            value,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeStmtReturn::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
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
        let range = range_from_object(vm, source_file, _object.clone(), "Return")?;
        stmt_return_from_object_with_range(vm, source_file, _object, range)
    }
}
// constructor
fn stmt_delete_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::StmtDelete> {
    let targets: Vec<Option<ast::Expr>> =
        get_node_list_field(vm, source_file, &object, "targets", "Delete")?;
    let (runtime_targets, targets) = runtime_expr_list_from_values(targets);
    Ok(ast::StmtDelete {
        node_index: Default::default(),
        targets,
        range,
        runtime_targets,
    })
}

impl Node for ast::StmtDelete {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            targets,
            range: _range,
            runtime_targets,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeStmtDelete::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        let targets = runtime_targets.map_or_else(
            || targets.ast_to_object(vm, source_file),
            |values| values.ast_to_object(vm, source_file),
        );
        dict.set_item("targets", targets, vm).unwrap();
        node_add_location(&dict, _range, vm, source_file);
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, _object.clone(), "Delete")?;
        stmt_delete_from_object_with_range(vm, source_file, _object, range)
    }
}

// constructor
fn stmt_assign_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::StmtAssign> {
    let targets: Vec<Option<ast::Expr>> =
        get_node_list_field(vm, source_file, &object, "targets", "Assign")?;
    let (runtime_targets, targets) = runtime_expr_list_from_values(targets);
    let value = get_required_node_field(vm, source_file, &object, "value", "Assign")?;
    let (runtime_type_comment, runtime_type_comment_bytes) =
        runtime_stmt_type_comment(vm, get_ast_string_field_opt(vm, &object, "type_comment")?);
    Ok(ast::StmtAssign {
        node_index: Default::default(),
        targets,
        value,
        range,
        runtime_targets,
        runtime_type_comment,
        runtime_type_comment_bytes,
    })
}

impl Node for ast::StmtAssign {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            targets,
            value,
            range,
            runtime_targets,
            runtime_type_comment,
            runtime_type_comment_bytes,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeStmtAssign::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        let targets = runtime_targets.map_or_else(
            || targets.ast_to_object(vm, source_file),
            |values| values.ast_to_object(vm, source_file),
        );
        dict.set_item("targets", targets, vm).unwrap();
        dict.set_item("value", value.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item(
            "type_comment",
            runtime_stmt_type_comment_object(vm, runtime_type_comment, runtime_type_comment_bytes),
            vm,
        )
        .unwrap();
        node_add_location(&dict, range, vm, source_file);
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, object.clone(), "Assign")?;
        stmt_assign_from_object_with_range(vm, source_file, object, range)
    }
}

// constructor
fn stmt_type_alias_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::StmtTypeAlias> {
    Ok(ast::StmtTypeAlias {
        node_index: Default::default(),
        name: get_required_node_field(vm, source_file, &object, "name", "TypeAlias")?,
        type_params: type_params_from_field(vm, source_file, &object, "type_params", "TypeAlias")?,
        value: get_required_node_field(vm, source_file, &object, "value", "TypeAlias")?,
        range,
    })
}

impl Node for ast::StmtTypeAlias {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            name,
            type_params,
            value,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeStmtTypeAlias::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("name", name.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item(
            "type_params",
            type_params.map_or_else(
                || vm.ctx.new_list(Vec::new()).into(),
                |tp| tp.ast_to_object(vm, source_file),
            ),
            vm,
        )
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
        let range = range_from_object(vm, source_file, _object.clone(), "TypeAlias")?;
        stmt_type_alias_from_object_with_range(vm, source_file, _object, range)
    }
}

// constructor
fn stmt_aug_assign_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::StmtAugAssign> {
    Ok(ast::StmtAugAssign {
        node_index: Default::default(),
        target: get_required_node_field(vm, source_file, &object, "target", "AugAssign")?,
        op: Node::ast_from_object(
            vm,
            source_file,
            get_node_field_required(vm, &object, "op", "AugAssign")?,
        )?,
        value: get_required_node_field(vm, source_file, &object, "value", "AugAssign")?,
        range,
    })
}

impl Node for ast::StmtAugAssign {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            target,
            op,
            value,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeStmtAugAssign::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("target", target.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item("op", op.ast_to_object(vm, source_file), vm)
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
        let range = range_from_object(vm, source_file, _object.clone(), "AugAssign")?;
        stmt_aug_assign_from_object_with_range(vm, source_file, _object, range)
    }
}

// constructor
fn stmt_ann_assign_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::StmtAnnAssign> {
    let simple = node_object_to_i32(vm, get_node_field(vm, &object, "simple", "AnnAssign")?)?;
    let runtime_simple = if simple != 0 && simple != 1 {
        Some(simple)
    } else {
        None
    };
    Ok(ast::StmtAnnAssign {
        node_index: Default::default(),
        target: get_required_node_field(vm, source_file, &object, "target", "AnnAssign")?,
        annotation: get_required_node_field(vm, source_file, &object, "annotation", "AnnAssign")?,
        value: get_node_field_opt(vm, &object, "value")?
            .map(|obj| Node::ast_from_object(vm, source_file, obj))
            .transpose()?,
        simple: simple != 0,
        range,
        runtime_simple,
    })
}

impl Node for ast::StmtAnnAssign {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            target,
            annotation,
            value,
            simple,
            range: _range,
            runtime_simple,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeStmtAnnAssign::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("target", target.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item("annotation", annotation.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item("value", value.ast_to_object(vm, source_file), vm)
            .unwrap();
        let simple = runtime_simple.map_or_else(
            || simple.ast_to_object(vm, source_file),
            |simple| vm.ctx.new_int(simple).into(),
        );
        dict.set_item("simple", simple, vm).unwrap();
        node_add_location(&dict, _range, vm, source_file);
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, _object.clone(), "AnnAssign")?;
        stmt_ann_assign_from_object_with_range(vm, source_file, _object, range)
    }
}

// constructor
fn stmt_for_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
    is_async: bool,
) -> PyResult<ast::StmtFor> {
    let typ = if is_async { "AsyncFor" } else { "For" };
    let target = get_required_node_field(vm, source_file, &object, "target", typ)?;
    let iter = get_required_node_field(vm, source_file, &object, "iter", typ)?;
    let body: Vec<Option<ast::Stmt>> = get_node_list_field(vm, source_file, &object, "body", typ)?;
    let orelse: Vec<Option<ast::Stmt>> =
        get_node_list_field(vm, source_file, &object, "orelse", typ)?;
    let runtime_body = runtime_stmt_list_metadata(&body);
    let runtime_orelse = runtime_stmt_list_metadata(&orelse);
    let body = lower_runtime_stmt_list(body);
    let orelse = lower_runtime_stmt_list(orelse);
    let (runtime_type_comment, runtime_type_comment_bytes) =
        runtime_stmt_type_comment(vm, get_ast_string_field_opt(vm, &object, "type_comment")?);
    Ok(ast::StmtFor {
        node_index: Default::default(),
        target,
        iter,
        body,
        orelse,
        range,
        is_async,
        runtime_type_comment,
        runtime_type_comment_bytes,
        runtime_body,
        runtime_orelse,
    })
}

impl Node for ast::StmtFor {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            is_async,
            target,
            iter,
            body,
            orelse,
            range: _range,
            runtime_type_comment,
            runtime_type_comment_bytes,
            runtime_body,
            runtime_orelse,
        } = self;

        let cls = if !is_async {
            pyast::NodeStmtFor::static_type().to_owned()
        } else {
            pyast::NodeStmtAsyncFor::static_type().to_owned()
        };

        let node = NodeAst.into_ref_with_type(vm, cls).unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("target", target.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item("iter", iter.ast_to_object(vm, source_file), vm)
            .unwrap();
        let body = runtime_body.map_or_else(
            || body.ast_to_object(vm, source_file),
            |values| values.ast_to_object(vm, source_file),
        );
        dict.set_item("body", body, vm).unwrap();
        let orelse = runtime_orelse.map_or_else(
            || orelse.ast_to_object(vm, source_file),
            |values| values.ast_to_object(vm, source_file),
        );
        dict.set_item("orelse", orelse, vm).unwrap();
        dict.set_item(
            "type_comment",
            runtime_stmt_type_comment_object(vm, runtime_type_comment, runtime_type_comment_bytes),
            vm,
        )
        .unwrap();
        node_add_location(&dict, _range, vm, source_file);
        node.into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        debug_assert!(
            is_node_instance(vm, &_object, pyast::NodeStmtFor::static_type())?
                || is_node_instance(vm, &_object, pyast::NodeStmtAsyncFor::static_type())?
        );
        let is_async = is_node_instance(vm, &_object, pyast::NodeStmtAsyncFor::static_type())?;
        let typ = if is_async { "AsyncFor" } else { "For" };
        let range = range_from_object(vm, source_file, _object.clone(), typ)?;
        stmt_for_from_object_with_range(vm, source_file, _object, range, is_async)
    }
}

// constructor
fn stmt_while_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::StmtWhile> {
    let body: Vec<Option<ast::Stmt>> =
        get_node_list_field(vm, source_file, &object, "body", "While")?;
    let orelse: Vec<Option<ast::Stmt>> =
        get_node_list_field(vm, source_file, &object, "orelse", "While")?;
    let runtime_body = runtime_stmt_list_metadata(&body);
    let runtime_orelse = runtime_stmt_list_metadata(&orelse);
    Ok(ast::StmtWhile {
        node_index: Default::default(),
        test: get_required_node_field(vm, source_file, &object, "test", "While")?,
        body: lower_runtime_stmt_list(body),
        orelse: lower_runtime_stmt_list(orelse),
        range,
        runtime_body,
        runtime_orelse,
    })
}

impl Node for ast::StmtWhile {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            test,
            body,
            orelse,
            range: _range,
            runtime_body,
            runtime_orelse,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeStmtWhile::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("test", test.ast_to_object(vm, source_file), vm)
            .unwrap();
        let body = runtime_body.map_or_else(
            || body.ast_to_object(vm, source_file),
            |values| values.ast_to_object(vm, source_file),
        );
        dict.set_item("body", body, vm).unwrap();
        let orelse = runtime_orelse.map_or_else(
            || orelse.ast_to_object(vm, source_file),
            |values| values.ast_to_object(vm, source_file),
        );
        dict.set_item("orelse", orelse, vm).unwrap();
        node_add_location(&dict, _range, vm, source_file);
        node.into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, _object.clone(), "While")?;
        stmt_while_from_object_with_range(vm, source_file, _object, range)
    }
}
// constructor
impl Node for ast::StmtIf {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index,
            test,
            body,
            range,
            elif_else_clauses,
            runtime_body,
        } = self;
        elif_else_clause::ast_to_object(
            ast::ElifElseClause {
                node_index,
                range,
                test: Some(*test),
                body,
                runtime_body,
                runtime_orelse: None,
            },
            elif_else_clauses.into_iter(),
            vm,
            source_file,
        )
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, object.clone(), "If")?;
        elif_else_clause::ast_from_object_with_range(vm, source_file, object, range)
    }
}
// constructor
fn stmt_with_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
    is_async: bool,
) -> PyResult<ast::StmtWith> {
    let typ = if is_async { "AsyncWith" } else { "With" };
    let items = get_node_list_field(vm, source_file, &object, "items", typ)?;
    let body: Vec<Option<ast::Stmt>> = get_node_list_field(vm, source_file, &object, "body", typ)?;
    let (runtime_body, body) = runtime_stmt_list_from_values(body);
    let (runtime_type_comment, runtime_type_comment_bytes) =
        runtime_stmt_type_comment(vm, get_ast_string_field_opt(vm, &object, "type_comment")?);
    Ok(ast::StmtWith {
        node_index: Default::default(),
        items,
        body,
        range,
        is_async,
        runtime_type_comment,
        runtime_type_comment_bytes,
        runtime_body,
    })
}

impl Node for ast::StmtWith {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            is_async,
            items,
            body,
            range: _range,
            runtime_type_comment,
            runtime_type_comment_bytes,
            runtime_body,
        } = self;

        let cls = if !is_async {
            pyast::NodeStmtWith::static_type().to_owned()
        } else {
            pyast::NodeStmtAsyncWith::static_type().to_owned()
        };

        let node = NodeAst.into_ref_with_type(vm, cls).unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("items", items.ast_to_object(vm, source_file), vm)
            .unwrap();
        let body = runtime_body.map_or_else(
            || body.ast_to_object(vm, source_file),
            |values| values.ast_to_object(vm, source_file),
        );
        dict.set_item("body", body, vm).unwrap();
        dict.set_item(
            "type_comment",
            runtime_stmt_type_comment_object(vm, runtime_type_comment, runtime_type_comment_bytes),
            vm,
        )
        .unwrap();
        node_add_location(&dict, _range, vm, source_file);
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        debug_assert!(
            is_node_instance(vm, &_object, pyast::NodeStmtWith::static_type())?
                || is_node_instance(vm, &_object, pyast::NodeStmtAsyncWith::static_type())?
        );
        let is_async = is_node_instance(vm, &_object, pyast::NodeStmtAsyncWith::static_type())?;
        let typ = if is_async { "AsyncWith" } else { "With" };
        let range = range_from_object(vm, source_file, _object.clone(), typ)?;
        stmt_with_from_object_with_range(vm, source_file, _object, range, is_async)
    }
}
// constructor
fn stmt_match_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::StmtMatch> {
    Ok(ast::StmtMatch {
        node_index: Default::default(),
        subject: get_required_node_field(vm, source_file, &object, "subject", "Match")?,
        cases: get_node_list_field(vm, source_file, &object, "cases", "Match")?,
        range,
    })
}

impl Node for ast::StmtMatch {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            subject,
            cases,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeStmtMatch::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("subject", subject.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item("cases", cases.ast_to_object(vm, source_file), vm)
            .unwrap();
        node_add_location(&dict, _range, vm, source_file);
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, _object.clone(), "Match")?;
        stmt_match_from_object_with_range(vm, source_file, _object, range)
    }
}
// constructor
fn stmt_raise_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::StmtRaise> {
    Ok(ast::StmtRaise {
        node_index: Default::default(),
        exc: get_node_field_opt(vm, &object, "exc")?
            .map(|obj| Node::ast_from_object(vm, source_file, obj))
            .transpose()?,
        cause: get_node_field_opt(vm, &object, "cause")?
            .map(|obj| Node::ast_from_object(vm, source_file, obj))
            .transpose()?,
        range,
    })
}

impl Node for ast::StmtRaise {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            exc,
            cause,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeStmtRaise::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("exc", exc.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item("cause", cause.ast_to_object(vm, source_file), vm)
            .unwrap();
        node_add_location(&dict, _range, vm, source_file);
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, _object.clone(), "Raise")?;
        stmt_raise_from_object_with_range(vm, source_file, _object, range)
    }
}
// constructor
fn stmt_try_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
    is_star: bool,
) -> PyResult<ast::StmtTry> {
    let typ = if is_star { "TryStar" } else { "Try" };
    let body: Vec<Option<ast::Stmt>> = get_node_list_field(vm, source_file, &object, "body", typ)?;
    let orelse: Vec<Option<ast::Stmt>> =
        get_node_list_field(vm, source_file, &object, "orelse", typ)?;
    let finalbody: Vec<Option<ast::Stmt>> =
        get_node_list_field(vm, source_file, &object, "finalbody", typ)?;
    let (runtime_handler_values, handlers) =
        except_handler_list_from_field(vm, source_file, &object, typ, is_star, range)?;
    let runtime_body = runtime_stmt_list_metadata(&body);
    let runtime_orelse = runtime_stmt_list_metadata(&orelse);
    let runtime_finalbody = runtime_stmt_list_metadata(&finalbody);
    let runtime_handlers = runtime_except_handler_list_metadata(&runtime_handler_values);
    Ok(ast::StmtTry {
        node_index: Default::default(),
        body: lower_runtime_stmt_list(body),
        handlers,
        orelse: lower_runtime_stmt_list(orelse),
        finalbody: lower_runtime_stmt_list(finalbody),
        range,
        is_star,
        runtime_body,
        runtime_handlers,
        runtime_orelse,
        runtime_finalbody,
    })
}

fn except_handler_list_from_field(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    object: &PyObject,
    typ: &str,
    is_try_star: bool,
    range: TextRange,
) -> PyResult<(Vec<Option<ast::ExceptHandler>>, Vec<ast::ExceptHandler>)> {
    let value = get_node_list_field_object(vm, object, "handlers", typ)?;
    let list = value.downcast_ref::<PyList>().unwrap();
    let len = list.borrow_vec().len();
    let mut result = Vec::with_capacity(len);
    let mut runtime_values = Vec::with_capacity(len);
    let recursion_context = format!(" while traversing '{typ}' node");
    for i in 0..len {
        let item = {
            let items = list.borrow_vec();
            if items.len() != len {
                return Err(vm.new_runtime_error(format!(
                    r#"{typ} field "handlers" changed size during iteration"#
                )));
            }
            items[i].clone()
        };
        let runtime_handler = if vm.is_none(&item) {
            None
        } else {
            Some(vm.with_recursion(&recursion_context, || {
                if is_try_star {
                    except_handler_from_object_unvalidated_range(vm, source_file, item)
                } else {
                    Node::ast_from_object(vm, source_file, item)
                }
            })?)
        };
        let handler = runtime_handler.clone().unwrap_or_else(|| {
            ast::ExceptHandler::ExceptHandler(ast::ExceptHandlerExceptHandler {
                node_index: Default::default(),
                range,
                type_: None,
                name: None,
                body: Vec::new(),
                runtime_body: None,
            })
        });
        runtime_values.push(runtime_handler);
        result.push(handler);
        if list.borrow_vec().len() != len {
            return Err(vm.new_runtime_error(format!(
                r#"{typ} field "handlers" changed size during iteration"#
            )));
        }
    }
    Ok((runtime_values, result))
}

impl Node for ast::StmtTry {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            body,
            handlers,
            orelse,
            finalbody,
            range: _range,
            is_star,
            runtime_body,
            runtime_handlers,
            runtime_orelse,
            runtime_finalbody,
        } = self;

        let cls = if is_star {
            pyast::NodeStmtTryStar::static_type()
        } else {
            pyast::NodeStmtTry::static_type()
        }
        .to_owned();

        let node = NodeAst.into_ref_with_type(vm, cls).unwrap();
        let dict = node.as_object().dict().unwrap();
        let body = runtime_body.map_or_else(
            || body.ast_to_object(vm, source_file),
            |values| values.ast_to_object(vm, source_file),
        );
        dict.set_item("body", body, vm).unwrap();
        let handlers = runtime_handlers.map_or_else(
            || handlers.ast_to_object(vm, source_file),
            |values| values.ast_to_object(vm, source_file),
        );
        dict.set_item("handlers", handlers, vm).unwrap();
        let orelse = runtime_orelse.map_or_else(
            || orelse.ast_to_object(vm, source_file),
            |values| values.ast_to_object(vm, source_file),
        );
        dict.set_item("orelse", orelse, vm).unwrap();
        let finalbody = runtime_finalbody.map_or_else(
            || finalbody.ast_to_object(vm, source_file),
            |values| values.ast_to_object(vm, source_file),
        );
        dict.set_item("finalbody", finalbody, vm).unwrap();
        node_add_location(&dict, _range, vm, source_file);
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        let is_star = is_node_instance(vm, &_object, pyast::NodeStmtTryStar::static_type())?;
        debug_assert!(
            is_node_instance(vm, &_object, pyast::NodeStmtTry::static_type())?
                || is_node_instance(vm, &_object, pyast::NodeStmtTryStar::static_type())?
        );
        let typ = if is_star { "TryStar" } else { "Try" };
        let range = range_from_object(vm, source_file, _object.clone(), typ)?;
        stmt_try_from_object_with_range(vm, source_file, _object, range, is_star)
    }
}

// constructor
fn stmt_assert_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::StmtAssert> {
    Ok(ast::StmtAssert {
        node_index: Default::default(),
        test: get_required_node_field(vm, source_file, &object, "test", "Assert")?,
        msg: get_node_field_opt(vm, &object, "msg")?
            .map(|obj| Node::ast_from_object(vm, source_file, obj))
            .transpose()?,
        range,
    })
}

impl Node for ast::StmtAssert {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            test,
            msg,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeStmtAssert::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("test", test.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item("msg", msg.ast_to_object(vm, source_file), vm)
            .unwrap();
        node_add_location(&dict, _range, vm, source_file);
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, _object.clone(), "Assert")?;
        stmt_assert_from_object_with_range(vm, source_file, _object, range)
    }
}
// constructor
fn stmt_import_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::StmtImport> {
    Ok(ast::StmtImport {
        node_index: Default::default(),
        names: get_node_list_field(vm, source_file, &object, "names", "Import")?,
        range,
        is_lazy: false,
    })
}

impl Node for ast::StmtImport {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            names,
            range: _range,
            is_lazy: _,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeStmtImport::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("names", names.ast_to_object(vm, source_file), vm)
            .unwrap();
        node_add_location(&dict, _range, vm, source_file);
        node.into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, _object.clone(), "Import")?;
        stmt_import_from_object_with_range(vm, source_file, _object, range)
    }
}
// constructor
fn stmt_import_from_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::StmtImportFrom> {
    let (level, raw_level) = import_from_level_from_field(vm, &object)?;
    let runtime_level = raw_level.filter(|level| *level < 0);
    Ok(ast::StmtImportFrom {
        node_index: Default::default(),
        module: get_node_field_opt(vm, &object, "module")?
            .map(|obj| Node::ast_from_object(vm, source_file, obj))
            .transpose()?,
        names: get_node_list_field(vm, source_file, &object, "names", "ImportFrom")?,
        level,
        range,
        is_lazy: false,
        runtime_level,
    })
}

fn import_from_level_from_field(
    vm: &VirtualMachine,
    object: &PyObjectRef,
) -> PyResult<(u32, Option<i32>)> {
    let Some(value) = get_node_field_opt(vm, object, "level")? else {
        return Ok((0, None));
    };
    let level = vm.with_recursion(" while traversing 'ImportFrom' node", || {
        node_object_to_i32(vm, value)
    })?;
    if level < 0 {
        return Ok((0, Some(level)));
    }
    Ok((level as u32, None))
}

impl Node for ast::StmtImportFrom {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            module,
            names,
            level,
            range,
            is_lazy: _,
            runtime_level,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeStmtImportFrom::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("module", module.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item("names", names.ast_to_object(vm, source_file), vm)
            .unwrap();
        let level = runtime_level.map_or_else(
            || vm.ctx.new_int(level).to_pyobject(vm),
            |level| vm.ctx.new_int(level).to_pyobject(vm),
        );
        dict.set_item("level", level, vm).unwrap();
        node_add_location(&dict, range, vm, source_file);
        node.into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, _object.clone(), "ImportFrom")?;
        stmt_import_from_from_object_with_range(vm, source_file, _object, range)
    }
}
// constructor
fn stmt_global_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::StmtGlobal> {
    Ok(ast::StmtGlobal {
        node_index: Default::default(),
        names: get_node_list_field(vm, source_file, &object, "names", "Global")?,
        range,
    })
}

impl Node for ast::StmtGlobal {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            names,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeStmtGlobal::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("names", names.ast_to_object(vm, source_file), vm)
            .unwrap();
        node_add_location(&dict, _range, vm, source_file);
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, _object.clone(), "Global")?;
        stmt_global_from_object_with_range(vm, source_file, _object, range)
    }
}
// constructor
fn stmt_nonlocal_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::StmtNonlocal> {
    Ok(ast::StmtNonlocal {
        node_index: Default::default(),
        names: get_node_list_field(vm, source_file, &object, "names", "Nonlocal")?,
        range,
    })
}

impl Node for ast::StmtNonlocal {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            names,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeStmtNonlocal::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("names", names.ast_to_object(vm, source_file), vm)
            .unwrap();
        node_add_location(&dict, _range, vm, source_file);
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, _object.clone(), "Nonlocal")?;
        stmt_nonlocal_from_object_with_range(vm, source_file, _object, range)
    }
}
// constructor
fn stmt_expr_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::StmtExpr> {
    Ok(ast::StmtExpr {
        node_index: Default::default(),
        value: get_required_node_field(vm, source_file, &object, "value", "Expr")?,
        range,
    })
}

impl Node for ast::StmtExpr {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            value,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeStmtExpr::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
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
        let range = range_from_object(vm, source_file, _object.clone(), "Expr")?;
        stmt_expr_from_object_with_range(vm, source_file, _object, range)
    }
}
// constructor
fn stmt_pass_from_object_with_range(range: TextRange) -> ast::StmtPass {
    ast::StmtPass {
        node_index: Default::default(),
        range,
    }
}

impl Node for ast::StmtPass {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeStmtPass::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        let location = super::text_range_to_source_range(source_file, _range);
        let start_row = location.start.row.get();
        let start_col = location.start.column.get();
        let mut end_row = location.end.row.get();

        // Align with CPython: when docstring optimization replaces a lone
        // docstring with `pass`, the end position is on the same line even if
        // it extends past the physical line length.
        let end_col = if end_row != start_row && _range.len() == TextSize::from(4) {
            end_row = start_row;
            start_col + 4
        } else {
            location.end.column.get()
        };

        dict.set_item("lineno", vm.ctx.new_int(start_row).into(), vm)
            .unwrap();
        dict.set_item("col_offset", vm.ctx.new_int(start_col).into(), vm)
            .unwrap();
        dict.set_item("end_lineno", vm.ctx.new_int(end_row).into(), vm)
            .unwrap();
        dict.set_item("end_col_offset", vm.ctx.new_int(end_col).into(), vm)
            .unwrap();
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, _object, "Pass")?;
        Ok(stmt_pass_from_object_with_range(range))
    }
}
// constructor
fn stmt_break_from_object_with_range(range: TextRange) -> ast::StmtBreak {
    ast::StmtBreak {
        node_index: Default::default(),
        range,
    }
}

impl Node for ast::StmtBreak {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeStmtBreak::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        node_add_location(&dict, _range, vm, source_file);
        node.into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, _object, "Break")?;
        Ok(stmt_break_from_object_with_range(range))
    }
}

// constructor
fn stmt_continue_from_object_with_range(range: TextRange) -> ast::StmtContinue {
    ast::StmtContinue {
        node_index: Default::default(),
        range,
    }
}

impl Node for ast::StmtContinue {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeStmtContinue::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        node_add_location(&dict, _range, vm, source_file);
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, _object, "Continue")?;
        Ok(stmt_continue_from_object_with_range(range))
    }
}
