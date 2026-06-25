use super::*;
use crate::stdlib::_ast::argument::{
    KeywordArguments, PositionalArguments, merge_function_call_arguments,
    split_function_call_arguments,
};
use rustpython_compiler_core::SourceFile;

// sum
impl Node for ast::Expr {
    fn ast_to_object(self, to_ctx: &AstToObjectContext<'_>) -> PyObjectRef {
        let _vm = to_ctx.vm;
        let _source_file = to_ctx.source_file;
        match self {
            Self::BoolOp(cons) => cons.ast_to_object(to_ctx),
            Self::Name(cons) => cons.ast_to_object(to_ctx),
            Self::BinOp(cons) => cons.ast_to_object(to_ctx),
            Self::UnaryOp(cons) => cons.ast_to_object(to_ctx),
            Self::Lambda(cons) => cons.ast_to_object(to_ctx),
            Self::If(cons) => cons.ast_to_object(to_ctx),
            Self::Dict(cons) => cons.ast_to_object(to_ctx),
            Self::Set(cons) => cons.ast_to_object(to_ctx),
            Self::ListComp(cons) => cons.ast_to_object(to_ctx),
            Self::SetComp(cons) => cons.ast_to_object(to_ctx),
            Self::DictComp(cons) => cons.ast_to_object(to_ctx),
            Self::Generator(cons) => cons.ast_to_object(to_ctx),
            Self::Await(cons) => cons.ast_to_object(to_ctx),
            Self::Yield(cons) => cons.ast_to_object(to_ctx),
            Self::YieldFrom(cons) => cons.ast_to_object(to_ctx),
            Self::Compare(cons) => cons.ast_to_object(to_ctx),
            Self::Call(cons) => cons.ast_to_object(to_ctx),
            Self::Constant(cons) => constant::expr_constant_to_object(to_ctx, cons),
            Self::Attribute(cons) => cons.ast_to_object(to_ctx),
            Self::Subscript(cons) => cons.ast_to_object(to_ctx),
            Self::Starred(cons) => cons.ast_to_object(to_ctx),
            Self::List(cons) => cons.ast_to_object(to_ctx),
            Self::Tuple(cons) => cons.ast_to_object(to_ctx),
            Self::Slice(cons) => cons.ast_to_object(to_ctx),
            Self::NumberLiteral(cons) => constant::number_literal_to_object(to_ctx, cons),
            Self::StringLiteral(cons) => constant::string_literal_to_object(to_ctx, cons),
            Self::FString(cons) => string::fstring_to_object(to_ctx, cons),
            Self::TString(cons) => string::tstring_to_object(to_ctx, cons),
            Self::BytesLiteral(cons) => constant::bytes_literal_to_object(to_ctx, cons),
            Self::BooleanLiteral(cons) => constant::boolean_literal_to_object(to_ctx, cons),
            Self::NoneLiteral(cons) => constant::none_literal_to_object(to_ctx, cons),
            Self::EllipsisLiteral(cons) => constant::ellipsis_literal_to_object(to_ctx, cons),
            Self::Named(cons) => cons.ast_to_object(to_ctx),
            Self::IpyEscapeCommand(_) => {
                unreachable!("IPython escape command is not part of Python AST")
            }
        }
    }

    fn ast_from_object(
        ctx: &AstFromObjectContext<'_>,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        if ctx.is_none(&object) {
            return Err(ctx.new_type_error(format!(
                "expected some sort of expr, but got {}",
                object.repr(ctx)?
            )));
        }
        enum ExprKind {
            BoolOp,
            Named,
            BinOp,
            UnaryOp,
            Lambda,
            If,
            Dict,
            Set,
            ListComp,
            SetComp,
            DictComp,
            Generator,
            Await,
            Yield,
            YieldFrom,
            Compare,
            Call,
            FormattedValue,
            Interpolation,
            JoinedStr,
            TemplateStr,
            Constant,
            Attribute,
            Subscript,
            Starred,
            Name,
            List,
            Tuple,
            Slice,
        }
        let kind = if is_node_instance(ctx, &object, pyast::NodeExprBoolOp::static_type())? {
            ExprKind::BoolOp
        } else if is_node_instance(ctx, &object, pyast::NodeExprNamedExpr::static_type())? {
            ExprKind::Named
        } else if is_node_instance(ctx, &object, pyast::NodeExprBinOp::static_type())? {
            ExprKind::BinOp
        } else if is_node_instance(ctx, &object, pyast::NodeExprUnaryOp::static_type())? {
            ExprKind::UnaryOp
        } else if is_node_instance(ctx, &object, pyast::NodeExprLambda::static_type())? {
            ExprKind::Lambda
        } else if is_node_instance(ctx, &object, pyast::NodeExprIfExp::static_type())? {
            ExprKind::If
        } else if is_node_instance(ctx, &object, pyast::NodeExprDict::static_type())? {
            ExprKind::Dict
        } else if is_node_instance(ctx, &object, pyast::NodeExprSet::static_type())? {
            ExprKind::Set
        } else if is_node_instance(ctx, &object, pyast::NodeExprListComp::static_type())? {
            ExprKind::ListComp
        } else if is_node_instance(ctx, &object, pyast::NodeExprSetComp::static_type())? {
            ExprKind::SetComp
        } else if is_node_instance(ctx, &object, pyast::NodeExprDictComp::static_type())? {
            ExprKind::DictComp
        } else if is_node_instance(ctx, &object, pyast::NodeExprGeneratorExp::static_type())? {
            ExprKind::Generator
        } else if is_node_instance(ctx, &object, pyast::NodeExprAwait::static_type())? {
            ExprKind::Await
        } else if is_node_instance(ctx, &object, pyast::NodeExprYield::static_type())? {
            ExprKind::Yield
        } else if is_node_instance(ctx, &object, pyast::NodeExprYieldFrom::static_type())? {
            ExprKind::YieldFrom
        } else if is_node_instance(ctx, &object, pyast::NodeExprCompare::static_type())? {
            ExprKind::Compare
        } else if is_node_instance(ctx, &object, pyast::NodeExprCall::static_type())? {
            ExprKind::Call
        } else if is_node_instance(ctx, &object, pyast::NodeExprFormattedValue::static_type())? {
            ExprKind::FormattedValue
        } else if is_node_instance(ctx, &object, pyast::NodeExprInterpolation::static_type())? {
            ExprKind::Interpolation
        } else if is_node_instance(ctx, &object, pyast::NodeExprJoinedStr::static_type())? {
            ExprKind::JoinedStr
        } else if is_node_instance(ctx, &object, pyast::NodeExprTemplateStr::static_type())? {
            ExprKind::TemplateStr
        } else if is_node_instance(ctx, &object, pyast::NodeExprConstant::static_type())? {
            ExprKind::Constant
        } else if is_node_instance(ctx, &object, pyast::NodeExprAttribute::static_type())? {
            ExprKind::Attribute
        } else if is_node_instance(ctx, &object, pyast::NodeExprSubscript::static_type())? {
            ExprKind::Subscript
        } else if is_node_instance(ctx, &object, pyast::NodeExprStarred::static_type())? {
            ExprKind::Starred
        } else if is_node_instance(ctx, &object, pyast::NodeExprName::static_type())? {
            ExprKind::Name
        } else if is_node_instance(ctx, &object, pyast::NodeExprList::static_type())? {
            ExprKind::List
        } else if is_node_instance(ctx, &object, pyast::NodeExprTuple::static_type())? {
            ExprKind::Tuple
        } else if is_node_instance(ctx, &object, pyast::NodeExprSlice::static_type())? {
            ExprKind::Slice
        } else {
            return Err(ctx.new_type_error(format!(
                "expected some sort of expr, but got {}",
                object.repr(ctx)?
            )));
        };
        let range = expr_range_from_object(ctx, source_file, object.clone())?;
        Ok(match kind {
            ExprKind::BoolOp => Self::BoolOp(expr_bool_op_from_object_with_range(
                ctx,
                source_file,
                object,
                range,
            )?),
            ExprKind::Named => Self::Named(expr_named_from_object_with_range(
                ctx,
                source_file,
                object,
                range,
            )?),
            ExprKind::BinOp => Self::BinOp(expr_bin_op_from_object_with_range(
                ctx,
                source_file,
                object,
                range,
            )?),
            ExprKind::UnaryOp => Self::UnaryOp(expr_unary_op_from_object_with_range(
                ctx,
                source_file,
                object,
                range,
            )?),
            ExprKind::Lambda => Self::Lambda(expr_lambda_from_object_with_range(
                ctx,
                source_file,
                object,
                range,
            )?),
            ExprKind::If => Self::If(expr_if_from_object_with_range(
                ctx,
                source_file,
                object,
                range,
            )?),
            ExprKind::Dict => Self::Dict(expr_dict_from_object_with_range(
                ctx,
                source_file,
                object,
                range,
            )?),
            ExprKind::Set => Self::Set(expr_set_from_object_with_range(
                ctx,
                source_file,
                object,
                range,
            )?),
            ExprKind::ListComp => Self::ListComp(expr_list_comp_from_object_with_range(
                ctx,
                source_file,
                object,
                range,
            )?),
            ExprKind::SetComp => Self::SetComp(expr_set_comp_from_object_with_range(
                ctx,
                source_file,
                object,
                range,
            )?),
            ExprKind::DictComp => Self::DictComp(expr_dict_comp_from_object_with_range(
                ctx,
                source_file,
                object,
                range,
            )?),
            ExprKind::Generator => Self::Generator(expr_generator_from_object_with_range(
                ctx,
                source_file,
                object,
                range,
            )?),
            ExprKind::Await => Self::Await(expr_await_from_object_with_range(
                ctx,
                source_file,
                object,
                range,
            )?),
            ExprKind::Yield => Self::Yield(expr_yield_from_object_with_range(
                ctx,
                source_file,
                object,
                range,
            )?),
            ExprKind::YieldFrom => Self::YieldFrom(expr_yield_from_from_object_with_range(
                ctx,
                source_file,
                object,
                range,
            )?),
            ExprKind::Compare => Self::Compare(expr_compare_from_object_with_range(
                ctx,
                source_file,
                object,
                range,
            )?),
            ExprKind::Call => Self::Call(expr_call_from_object_with_range(
                ctx,
                source_file,
                object,
                range,
            )?),
            ExprKind::FormattedValue => {
                let formatted = string::formatted_value_from_object_with_range(
                    ctx,
                    source_file,
                    object,
                    range,
                )?;
                string::formatted_value_to_expr(Some(ctx), formatted)
            }
            ExprKind::Interpolation => {
                let interpolation = string::tstring_interpolation_from_object_with_range(
                    ctx,
                    source_file,
                    object,
                    range,
                )?;
                string::interpolation_to_expr(ctx, source_file, interpolation)?
            }
            ExprKind::JoinedStr => {
                string::joined_str_from_object_with_range(ctx, source_file, object, range)?
                    .into_expr(Some(ctx))
            }
            ExprKind::TemplateStr => {
                let template =
                    string::template_str_from_object_with_range(ctx, source_file, object, range)?;
                string::template_str_to_expr(ctx, source_file, template)?
            }
            ExprKind::Constant => {
                constant::constant_from_object_with_range(ctx, source_file, object, range)?
                    .into_expr(Some(ctx))
            }
            ExprKind::Attribute => Self::Attribute(expr_attribute_from_object_with_range(
                ctx,
                source_file,
                object,
                range,
            )?),
            ExprKind::Subscript => Self::Subscript(expr_subscript_from_object_with_range(
                ctx,
                source_file,
                object,
                range,
            )?),
            ExprKind::Starred => Self::Starred(expr_starred_from_object_with_range(
                ctx,
                source_file,
                object,
                range,
            )?),
            ExprKind::Name => Self::Name(expr_name_from_object_with_range(
                ctx,
                source_file,
                object,
                range,
            )?),
            ExprKind::List => Self::List(expr_list_from_object_with_range(
                ctx,
                source_file,
                object,
                range,
            )?),
            ExprKind::Tuple => Self::Tuple(expr_tuple_from_object_with_range(
                ctx,
                source_file,
                object,
                range,
            )?),
            ExprKind::Slice => Self::Slice(expr_slice_from_object_with_range(
                ctx,
                source_file,
                object,
                range,
            )?),
        })
    }
}

// constructor
fn expr_bool_op_from_object_with_range(
    ctx: &AstFromObjectContext<'_>,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::ExprBoolOp> {
    let values: Vec<Option<ast::Expr>> =
        get_node_list_field(ctx, source_file, &object, "values", "BoolOp")?;
    let (runtime_values, values) = public_expr_list_from_values(values);
    Ok(ast::ExprBoolOp {
        node_index: Default::default(),
        op: Node::ast_from_object(
            ctx,
            source_file,
            get_node_field_required(ctx, &object, "op", "BoolOp")?,
        )?,
        values,
        range,
        runtime_values,
    })
}

impl Node for ast::ExprBoolOp {
    fn ast_to_object(self, to_ctx: &AstToObjectContext<'_>) -> PyObjectRef {
        let ctx = to_ctx.vm;
        let source_file = to_ctx.source_file;
        let Self {
            node_index: _,
            op,
            values,
            range,
            runtime_values,
        } = self;
        let node = NodeAst
            .into_ref_with_type(ctx, pyast::NodeExprBoolOp::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("op", op.ast_to_object(to_ctx), ctx).unwrap();
        let values = super::constant::public_ast_expr_list_object(to_ctx, runtime_values)
            .map_or_else(
                || values.ast_to_object(to_ctx),
                |values| values.ast_to_object(to_ctx),
            );
        dict.set_item("values", values, ctx).unwrap();
        node_add_location(&dict, range, ctx, source_file);
        node.into()
    }

    fn ast_from_object(
        ctx: &AstFromObjectContext<'_>,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(ctx, source_file, object.clone(), "BoolOp")?;
        expr_bool_op_from_object_with_range(ctx, source_file, object, range)
    }
}

// constructor
fn expr_named_from_object_with_range(
    ctx: &AstFromObjectContext<'_>,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::ExprNamed> {
    Ok(ast::ExprNamed {
        node_index: Default::default(),
        target: get_required_node_field(ctx, source_file, &object, "target", "NamedExpr")?,
        value: get_required_node_field(ctx, source_file, &object, "value", "NamedExpr")?,
        range,
    })
}

impl Node for ast::ExprNamed {
    fn ast_to_object(self, to_ctx: &AstToObjectContext<'_>) -> PyObjectRef {
        let vm = to_ctx.vm;
        let source_file = to_ctx.source_file;
        let Self {
            node_index: _,
            target,
            value,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprNamedExpr::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("target", target.ast_to_object(to_ctx), vm)
            .unwrap();
        dict.set_item("value", value.ast_to_object(to_ctx), vm)
            .unwrap();
        node_add_location(&dict, range, vm, source_file);
        node.into()
    }

    fn ast_from_object(
        ctx: &AstFromObjectContext<'_>,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(ctx, source_file, object.clone(), "NamedExpr")?;
        expr_named_from_object_with_range(ctx, source_file, object, range)
    }
}

// constructor
fn expr_bin_op_from_object_with_range(
    ctx: &AstFromObjectContext<'_>,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::ExprBinOp> {
    Ok(ast::ExprBinOp {
        node_index: Default::default(),
        left: get_required_node_field(ctx, source_file, &object, "left", "BinOp")?,
        op: Node::ast_from_object(
            ctx,
            source_file,
            get_node_field_required(ctx, &object, "op", "BinOp")?,
        )?,
        right: get_required_node_field(ctx, source_file, &object, "right", "BinOp")?,
        range,
    })
}

impl Node for ast::ExprBinOp {
    fn ast_to_object(self, to_ctx: &AstToObjectContext<'_>) -> PyObjectRef {
        let ctx = to_ctx.vm;
        let source_file = to_ctx.source_file;
        let Self {
            node_index: _,
            left,
            op,
            right,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(ctx, pyast::NodeExprBinOp::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("left", left.ast_to_object(to_ctx), ctx)
            .unwrap();
        dict.set_item("op", op.ast_to_object(to_ctx), ctx).unwrap();
        dict.set_item("right", right.ast_to_object(to_ctx), ctx)
            .unwrap();
        node_add_location(&dict, range, ctx, source_file);
        node.into()
    }

    fn ast_from_object(
        ctx: &AstFromObjectContext<'_>,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(ctx, source_file, object.clone(), "BinOp")?;
        expr_bin_op_from_object_with_range(ctx, source_file, object, range)
    }
}

// constructor
fn expr_unary_op_from_object_with_range(
    ctx: &AstFromObjectContext<'_>,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::ExprUnaryOp> {
    Ok(ast::ExprUnaryOp {
        node_index: Default::default(),
        op: Node::ast_from_object(
            ctx,
            source_file,
            get_node_field_required(ctx, &object, "op", "UnaryOp")?,
        )?,
        operand: get_required_node_field(ctx, source_file, &object, "operand", "UnaryOp")?,
        range,
    })
}

impl Node for ast::ExprUnaryOp {
    fn ast_to_object(self, to_ctx: &AstToObjectContext<'_>) -> PyObjectRef {
        let vm = to_ctx.vm;
        let source_file = to_ctx.source_file;
        let Self {
            node_index: _,
            op,
            operand,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprUnaryOp::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("op", op.ast_to_object(to_ctx), vm).unwrap();
        dict.set_item("operand", operand.ast_to_object(to_ctx), vm)
            .unwrap();
        node_add_location(&dict, range, vm, source_file);
        node.into()
    }
    fn ast_from_object(
        ctx: &AstFromObjectContext<'_>,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(ctx, source_file, object.clone(), "UnaryOp")?;
        expr_unary_op_from_object_with_range(ctx, source_file, object, range)
    }
}

// constructor
fn expr_lambda_from_object_with_range(
    ctx: &AstFromObjectContext<'_>,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::ExprLambda> {
    Ok(ast::ExprLambda {
        node_index: Default::default(),
        parameters: Node::ast_from_object(
            ctx,
            source_file,
            get_node_field_required(ctx, &object, "args", "Lambda")?,
        )?,
        body: get_required_node_field(ctx, source_file, &object, "body", "Lambda")?,
        range,
    })
}

impl Node for ast::ExprLambda {
    fn ast_to_object(self, to_ctx: &AstToObjectContext<'_>) -> PyObjectRef {
        let ctx = to_ctx.vm;
        let source_file = to_ctx.source_file;
        let Self {
            node_index: _,
            parameters,
            body,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(ctx, pyast::NodeExprLambda::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        let args = match parameters {
            Some(params) => params.ast_to_object(to_ctx),
            None => empty_arguments_object(ctx),
        };
        dict.set_item("args", args, ctx).unwrap();
        dict.set_item("body", body.ast_to_object(to_ctx), ctx)
            .unwrap();
        node_add_location(&dict, _range, ctx, source_file);
        node.into()
    }

    fn ast_from_object(
        ctx: &AstFromObjectContext<'_>,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(ctx, source_file, object.clone(), "Lambda")?;
        expr_lambda_from_object_with_range(ctx, source_file, object, range)
    }
}

// constructor
fn expr_if_from_object_with_range(
    ctx: &AstFromObjectContext<'_>,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::ExprIf> {
    Ok(ast::ExprIf {
        node_index: Default::default(),
        test: get_required_node_field(ctx, source_file, &object, "test", "IfExp")?,
        body: get_required_node_field(ctx, source_file, &object, "body", "IfExp")?,
        orelse: get_required_node_field(ctx, source_file, &object, "orelse", "IfExp")?,
        range,
    })
}

impl Node for ast::ExprIf {
    fn ast_to_object(self, to_ctx: &AstToObjectContext<'_>) -> PyObjectRef {
        let vm = to_ctx.vm;
        let source_file = to_ctx.source_file;
        let Self {
            node_index: _,
            test,
            body,
            orelse,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprIfExp::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("test", test.ast_to_object(to_ctx), vm)
            .unwrap();
        dict.set_item("body", body.ast_to_object(to_ctx), vm)
            .unwrap();
        dict.set_item("orelse", orelse.ast_to_object(to_ctx), vm)
            .unwrap();
        node_add_location(&dict, range, vm, source_file);
        node.into()
    }

    fn ast_from_object(
        ctx: &AstFromObjectContext<'_>,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(ctx, source_file, object.clone(), "IfExp")?;
        expr_if_from_object_with_range(ctx, source_file, object, range)
    }
}

// constructor
fn expr_dict_from_object_with_range(
    ctx: &AstFromObjectContext<'_>,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::ExprDict> {
    let keys: Vec<Option<ast::Expr>> =
        get_node_list_field(ctx, source_file, &object, "keys", "Dict")?;
    let values: Vec<Option<ast::Expr>> =
        get_node_list_field(ctx, source_file, &object, "values", "Dict")?;
    if keys.len() != values.len() {
        return Err(ctx.new_value_error("Dict doesn't have the same number of keys as values"));
    }
    let runtime_values = public_expr_list_metadata(&values);
    let items = keys
        .into_iter()
        .zip(lower_public_expr_list(values))
        .map(|(key, value)| ast::DictItem { key, value })
        .collect();
    Ok(ast::ExprDict {
        node_index: Default::default(),
        items,
        range,
        runtime_values,
    })
}

impl Node for ast::ExprDict {
    fn ast_to_object(self, to_ctx: &AstToObjectContext<'_>) -> PyObjectRef {
        let vm = to_ctx.vm;
        let source_file = to_ctx.source_file;
        let Self {
            node_index: _,
            items,
            range,
            runtime_values,
        } = self;
        let (keys, values) =
            items
                .into_iter()
                .fold((vec![], vec![]), |(mut keys, mut values), item| {
                    keys.push(item.key);
                    values.push(item.value);
                    (keys, values)
                });
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprDict::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("keys", keys.ast_to_object(to_ctx), vm)
            .unwrap();
        let values = super::constant::public_ast_expr_list_object(to_ctx, runtime_values)
            .map_or_else(
                || values.ast_to_object(to_ctx),
                |values| values.ast_to_object(to_ctx),
            );
        dict.set_item("values", values, vm).unwrap();
        node_add_location(&dict, range, vm, source_file);
        node.into()
    }

    fn ast_from_object(
        ctx: &AstFromObjectContext<'_>,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(ctx, source_file, object.clone(), "Dict")?;
        expr_dict_from_object_with_range(ctx, source_file, object, range)
    }
}

// constructor
fn expr_set_from_object_with_range(
    ctx: &AstFromObjectContext<'_>,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::ExprSet> {
    let elts: Vec<Option<ast::Expr>> =
        get_node_list_field(ctx, source_file, &object, "elts", "Set")?;
    let (runtime_elts, elts) = public_expr_list_from_values(elts);
    Ok(ast::ExprSet {
        node_index: Default::default(),
        elts,
        range,
        runtime_elts,
    })
}

impl Node for ast::ExprSet {
    fn ast_to_object(self, to_ctx: &AstToObjectContext<'_>) -> PyObjectRef {
        let ctx = to_ctx.vm;
        let source_file = to_ctx.source_file;
        let Self {
            node_index: _,
            elts,
            range,
            runtime_elts,
        } = self;
        let node = NodeAst
            .into_ref_with_type(ctx, pyast::NodeExprSet::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        let elts = super::constant::public_ast_expr_list_object(to_ctx, runtime_elts).map_or_else(
            || elts.ast_to_object(to_ctx),
            |values| values.ast_to_object(to_ctx),
        );
        dict.set_item("elts", elts, ctx).unwrap();
        node_add_location(&dict, range, ctx, source_file);
        node.into()
    }
    fn ast_from_object(
        ctx: &AstFromObjectContext<'_>,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(ctx, source_file, object.clone(), "Set")?;
        expr_set_from_object_with_range(ctx, source_file, object, range)
    }
}

// constructor
fn expr_list_comp_from_object_with_range(
    ctx: &AstFromObjectContext<'_>,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::ExprListComp> {
    Ok(ast::ExprListComp {
        node_index: Default::default(),
        elt: get_required_node_field(ctx, source_file, &object, "elt", "ListComp")?,
        generators: get_node_list_field(ctx, source_file, &object, "generators", "ListComp")?,
        range,
    })
}

impl Node for ast::ExprListComp {
    fn ast_to_object(self, to_ctx: &AstToObjectContext<'_>) -> PyObjectRef {
        let vm = to_ctx.vm;
        let source_file = to_ctx.source_file;
        let Self {
            node_index: _,
            elt,
            generators,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprListComp::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("elt", elt.ast_to_object(to_ctx), vm).unwrap();
        dict.set_item("generators", generators.ast_to_object(to_ctx), vm)
            .unwrap();
        node_add_location(&dict, range, vm, source_file);
        node.into()
    }

    fn ast_from_object(
        ctx: &AstFromObjectContext<'_>,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(ctx, source_file, object.clone(), "ListComp")?;
        expr_list_comp_from_object_with_range(ctx, source_file, object, range)
    }
}

// constructor
fn expr_set_comp_from_object_with_range(
    ctx: &AstFromObjectContext<'_>,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::ExprSetComp> {
    Ok(ast::ExprSetComp {
        node_index: Default::default(),
        elt: get_required_node_field(ctx, source_file, &object, "elt", "SetComp")?,
        generators: get_node_list_field(ctx, source_file, &object, "generators", "SetComp")?,
        range,
    })
}

impl Node for ast::ExprSetComp {
    fn ast_to_object(self, to_ctx: &AstToObjectContext<'_>) -> PyObjectRef {
        let ctx = to_ctx.vm;
        let source_file = to_ctx.source_file;
        let Self {
            node_index: _,
            elt,
            generators,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(ctx, pyast::NodeExprSetComp::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("elt", elt.ast_to_object(to_ctx), ctx)
            .unwrap();
        dict.set_item("generators", generators.ast_to_object(to_ctx), ctx)
            .unwrap();
        node_add_location(&dict, range, ctx, source_file);
        node.into()
    }

    fn ast_from_object(
        ctx: &AstFromObjectContext<'_>,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(ctx, source_file, object.clone(), "SetComp")?;
        expr_set_comp_from_object_with_range(ctx, source_file, object, range)
    }
}

// constructor
fn expr_dict_comp_from_object_with_range(
    ctx: &AstFromObjectContext<'_>,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::ExprDictComp> {
    Ok(ast::ExprDictComp {
        node_index: Default::default(),
        key: get_required_node_field(ctx, source_file, &object, "key", "DictComp")?,
        value: get_required_node_field(ctx, source_file, &object, "value", "DictComp")?,
        generators: get_node_list_field(ctx, source_file, &object, "generators", "DictComp")?,
        range,
    })
}

impl Node for ast::ExprDictComp {
    fn ast_to_object(self, to_ctx: &AstToObjectContext<'_>) -> PyObjectRef {
        let vm = to_ctx.vm;
        let source_file = to_ctx.source_file;
        let Self {
            node_index: _,
            key,
            value,
            generators,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprDictComp::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("key", key.ast_to_object(to_ctx), vm).unwrap();
        dict.set_item("value", value.ast_to_object(to_ctx), vm)
            .unwrap();
        dict.set_item("generators", generators.ast_to_object(to_ctx), vm)
            .unwrap();
        node_add_location(&dict, range, vm, source_file);
        node.into()
    }

    fn ast_from_object(
        ctx: &AstFromObjectContext<'_>,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(ctx, source_file, object.clone(), "DictComp")?;
        expr_dict_comp_from_object_with_range(ctx, source_file, object, range)
    }
}

// constructor
fn expr_generator_from_object_with_range(
    ctx: &AstFromObjectContext<'_>,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::ExprGenerator> {
    Ok(ast::ExprGenerator {
        node_index: Default::default(),
        elt: get_required_node_field(ctx, source_file, &object, "elt", "GeneratorExp")?,
        generators: get_node_list_field(ctx, source_file, &object, "generators", "GeneratorExp")?,
        range,
        // TODO: Is this correct?
        parenthesized: true,
    })
}

impl Node for ast::ExprGenerator {
    fn ast_to_object(self, to_ctx: &AstToObjectContext<'_>) -> PyObjectRef {
        let ctx = to_ctx.vm;
        let source_file = to_ctx.source_file;
        let Self {
            node_index: _,
            elt,
            generators,
            range,
            parenthesized,
        } = self;
        let range = if parenthesized {
            range
        } else {
            TextRange::new(
                range
                    .start()
                    .saturating_sub(ruff_text_size::TextSize::from(1)),
                range.end() + ruff_text_size::TextSize::from(1),
            )
        };
        let node = NodeAst
            .into_ref_with_type(ctx, pyast::NodeExprGeneratorExp::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("elt", elt.ast_to_object(to_ctx), ctx)
            .unwrap();
        dict.set_item("generators", generators.ast_to_object(to_ctx), ctx)
            .unwrap();
        node_add_location(&dict, range, ctx, source_file);
        node.into()
    }

    fn ast_from_object(
        ctx: &AstFromObjectContext<'_>,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(ctx, source_file, object.clone(), "GeneratorExp")?;
        expr_generator_from_object_with_range(ctx, source_file, object, range)
    }
}

// constructor
fn expr_await_from_object_with_range(
    ctx: &AstFromObjectContext<'_>,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::ExprAwait> {
    Ok(ast::ExprAwait {
        node_index: Default::default(),
        value: get_required_node_field(ctx, source_file, &object, "value", "Await")?,
        range,
    })
}

impl Node for ast::ExprAwait {
    fn ast_to_object(self, to_ctx: &AstToObjectContext<'_>) -> PyObjectRef {
        let vm = to_ctx.vm;
        let source_file = to_ctx.source_file;
        let Self {
            node_index: _,
            value,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprAwait::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(to_ctx), vm)
            .unwrap();
        node_add_location(&dict, range, vm, source_file);
        node.into()
    }
    fn ast_from_object(
        ctx: &AstFromObjectContext<'_>,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(ctx, source_file, object.clone(), "Await")?;
        expr_await_from_object_with_range(ctx, source_file, object, range)
    }
}

// constructor
fn expr_yield_from_object_with_range(
    ctx: &AstFromObjectContext<'_>,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::ExprYield> {
    Ok(ast::ExprYield {
        node_index: Default::default(),
        value: get_node_field_opt(ctx, &object, "value")?
            .map(|obj| Node::ast_from_object(ctx, source_file, obj))
            .transpose()?,
        range,
    })
}

impl Node for ast::ExprYield {
    fn ast_to_object(self, to_ctx: &AstToObjectContext<'_>) -> PyObjectRef {
        let ctx = to_ctx.vm;
        let source_file = to_ctx.source_file;
        let Self {
            node_index: _,
            value,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(ctx, pyast::NodeExprYield::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(to_ctx), ctx)
            .unwrap();
        node_add_location(&dict, range, ctx, source_file);
        node.into()
    }

    fn ast_from_object(
        ctx: &AstFromObjectContext<'_>,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(ctx, source_file, object.clone(), "Yield")?;
        expr_yield_from_object_with_range(ctx, source_file, object, range)
    }
}

// constructor
fn expr_yield_from_from_object_with_range(
    ctx: &AstFromObjectContext<'_>,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::ExprYieldFrom> {
    Ok(ast::ExprYieldFrom {
        node_index: Default::default(),
        value: get_required_node_field(ctx, source_file, &object, "value", "YieldFrom")?,
        range,
    })
}

impl Node for ast::ExprYieldFrom {
    fn ast_to_object(self, to_ctx: &AstToObjectContext<'_>) -> PyObjectRef {
        let vm = to_ctx.vm;
        let source_file = to_ctx.source_file;
        let Self {
            node_index: _,
            value,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprYieldFrom::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(to_ctx), vm)
            .unwrap();
        node_add_location(&dict, range, vm, source_file);
        node.into()
    }

    fn ast_from_object(
        ctx: &AstFromObjectContext<'_>,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(ctx, source_file, object.clone(), "YieldFrom")?;
        expr_yield_from_from_object_with_range(ctx, source_file, object, range)
    }
}

// constructor
fn expr_compare_from_object_with_range(
    ctx: &AstFromObjectContext<'_>,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::ExprCompare> {
    let comparators: Vec<Option<ast::Expr>> =
        get_node_list_field(ctx, source_file, &object, "comparators", "Compare")?;
    let (runtime_comparators, comparators) = public_expr_boxed_slice_from_values(comparators);
    Ok(ast::ExprCompare {
        node_index: Default::default(),
        left: get_required_node_field(ctx, source_file, &object, "left", "Compare")?,
        ops: get_node_boxed_slice_field(ctx, source_file, &object, "ops", "Compare")?,
        comparators,
        range,
        runtime_comparators,
    })
}

impl Node for ast::ExprCompare {
    fn ast_to_object(self, to_ctx: &AstToObjectContext<'_>) -> PyObjectRef {
        let ctx = to_ctx.vm;
        let source_file = to_ctx.source_file;
        let Self {
            node_index: _,
            left,
            ops,
            comparators,
            range,
            runtime_comparators,
        } = self;
        let node = NodeAst
            .into_ref_with_type(ctx, pyast::NodeExprCompare::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("left", left.ast_to_object(to_ctx), ctx)
            .unwrap();
        dict.set_item("ops", BoxedSlice(ops).ast_to_object(to_ctx), ctx)
            .unwrap();
        let comparators = super::constant::public_ast_expr_list_object(to_ctx, runtime_comparators)
            .map_or_else(
                || BoxedSlice(comparators).ast_to_object(to_ctx),
                |values| values.ast_to_object(to_ctx),
            );
        dict.set_item("comparators", comparators, ctx).unwrap();
        node_add_location(&dict, range, ctx, source_file);
        node.into()
    }

    fn ast_from_object(
        ctx: &AstFromObjectContext<'_>,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(ctx, source_file, object.clone(), "Compare")?;
        expr_compare_from_object_with_range(ctx, source_file, object, range)
    }
}

// constructor
fn expr_call_from_object_with_range(
    ctx: &AstFromObjectContext<'_>,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::ExprCall> {
    Ok(ast::ExprCall {
        node_index: Default::default(),
        func: get_required_node_field(ctx, source_file, &object, "func", "Call")?,
        arguments: merge_function_call_arguments(
            PositionalArguments::ast_from_field(ctx, source_file, &object, "args", "Call")?,
            KeywordArguments::ast_from_field(ctx, source_file, &object, "keywords", "Call")?,
        ),
        range,
    })
}

impl Node for ast::ExprCall {
    fn ast_to_object(self, to_ctx: &AstToObjectContext<'_>) -> PyObjectRef {
        let vm = to_ctx.vm;
        let source_file = to_ctx.source_file;
        let Self {
            node_index: _,
            func,
            arguments,
            range,
        } = self;
        let (positional_arguments, keyword_arguments) = split_function_call_arguments(arguments);
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprCall::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("func", func.ast_to_object(to_ctx), vm)
            .unwrap();
        dict.set_item("args", positional_arguments.ast_to_object(to_ctx), vm)
            .unwrap();
        dict.set_item("keywords", keyword_arguments.ast_to_object(to_ctx), vm)
            .unwrap();
        node_add_location(&dict, range, vm, source_file);
        node.into()
    }

    fn ast_from_object(
        ctx: &AstFromObjectContext<'_>,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(ctx, source_file, object.clone(), "Call")?;
        expr_call_from_object_with_range(ctx, source_file, object, range)
    }
}

// constructor
fn expr_attribute_from_object_with_range(
    ctx: &AstFromObjectContext<'_>,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::ExprAttribute> {
    Ok(ast::ExprAttribute {
        node_index: Default::default(),
        value: get_required_node_field(ctx, source_file, &object, "value", "Attribute")?,
        attr: get_required_identifier_field(ctx, source_file, &object, "attr", "Attribute")?,
        ctx: Node::ast_from_object(
            ctx,
            source_file,
            get_node_field_required(ctx, &object, "ctx", "Attribute")?,
        )?,
        range,
    })
}

impl Node for ast::ExprAttribute {
    fn ast_to_object(self, to_ctx: &AstToObjectContext<'_>) -> PyObjectRef {
        let vm = to_ctx.vm;
        let source_file = to_ctx.source_file;
        let Self {
            node_index: _,
            value,
            attr,
            ctx,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprAttribute::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(to_ctx), vm)
            .unwrap();
        dict.set_item("attr", attr.ast_to_object(to_ctx), vm)
            .unwrap();
        dict.set_item("ctx", ctx.ast_to_object(to_ctx), vm).unwrap();
        node_add_location(&dict, range, vm, source_file);
        node.into()
    }

    fn ast_from_object(
        ctx: &AstFromObjectContext<'_>,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(ctx, source_file, object.clone(), "Attribute")?;
        expr_attribute_from_object_with_range(ctx, source_file, object, range)
    }
}

// constructor
fn expr_subscript_from_object_with_range(
    ctx: &AstFromObjectContext<'_>,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::ExprSubscript> {
    Ok(ast::ExprSubscript {
        node_index: Default::default(),
        value: get_required_node_field(ctx, source_file, &object, "value", "Subscript")?,
        slice: get_required_node_field(ctx, source_file, &object, "slice", "Subscript")?,
        ctx: Node::ast_from_object(
            ctx,
            source_file,
            get_node_field_required(ctx, &object, "ctx", "Subscript")?,
        )?,
        range,
    })
}

impl Node for ast::ExprSubscript {
    fn ast_to_object(self, to_ctx: &AstToObjectContext<'_>) -> PyObjectRef {
        let vm = to_ctx.vm;
        let source_file = to_ctx.source_file;
        let Self {
            node_index: _,
            value,
            slice,
            ctx,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprSubscript::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(to_ctx), vm)
            .unwrap();
        dict.set_item("slice", slice.ast_to_object(to_ctx), vm)
            .unwrap();
        dict.set_item("ctx", ctx.ast_to_object(to_ctx), vm).unwrap();
        node_add_location(&dict, _range, vm, source_file);
        node.into()
    }
    fn ast_from_object(
        ctx: &AstFromObjectContext<'_>,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(ctx, source_file, object.clone(), "Subscript")?;
        expr_subscript_from_object_with_range(ctx, source_file, object, range)
    }
}

// constructor
fn expr_starred_from_object_with_range(
    ctx: &AstFromObjectContext<'_>,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::ExprStarred> {
    Ok(ast::ExprStarred {
        node_index: Default::default(),
        value: get_required_node_field(ctx, source_file, &object, "value", "Starred")?,
        ctx: Node::ast_from_object(
            ctx,
            source_file,
            get_node_field_required(ctx, &object, "ctx", "Starred")?,
        )?,
        range,
    })
}

impl Node for ast::ExprStarred {
    fn ast_to_object(self, to_ctx: &AstToObjectContext<'_>) -> PyObjectRef {
        let vm = to_ctx.vm;
        let source_file = to_ctx.source_file;
        let Self {
            node_index: _,
            value,
            ctx,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprStarred::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(to_ctx), vm)
            .unwrap();
        dict.set_item("ctx", ctx.ast_to_object(to_ctx), vm).unwrap();
        node_add_location(&dict, range, vm, source_file);
        node.into()
    }
    fn ast_from_object(
        ctx: &AstFromObjectContext<'_>,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(ctx, source_file, object.clone(), "Starred")?;
        expr_starred_from_object_with_range(ctx, source_file, object, range)
    }
}

// constructor
fn expr_name_from_object_with_range(
    ctx: &AstFromObjectContext<'_>,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::ExprName> {
    Ok(ast::ExprName {
        node_index: Default::default(),
        id: get_required_identifier_field(ctx, source_file, &object, "id", "Name")?,
        ctx: Node::ast_from_object(
            ctx,
            source_file,
            get_node_field_required(ctx, &object, "ctx", "Name")?,
        )?,
        range,
    })
}

impl Node for ast::ExprName {
    fn ast_to_object(self, to_ctx: &AstToObjectContext<'_>) -> PyObjectRef {
        let vm = to_ctx.vm;
        let source_file = to_ctx.source_file;
        let Self {
            node_index: _,
            id,
            ctx,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprName::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("id", id.to_pyobject(vm), vm).unwrap();
        dict.set_item("ctx", ctx.ast_to_object(to_ctx), vm).unwrap();
        node_add_location(&dict, range, vm, source_file);
        node.into()
    }

    fn ast_from_object(
        ctx: &AstFromObjectContext<'_>,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(ctx, source_file, object.clone(), "Name")?;
        expr_name_from_object_with_range(ctx, source_file, object, range)
    }
}

// constructor
fn expr_list_from_object_with_range(
    ctx: &AstFromObjectContext<'_>,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::ExprList> {
    let elts: Vec<Option<ast::Expr>> =
        get_node_list_field(ctx, source_file, &object, "elts", "List")?;
    let (runtime_elts, elts) = public_expr_list_from_values(elts);
    Ok(ast::ExprList {
        node_index: Default::default(),
        elts,
        ctx: Node::ast_from_object(
            ctx,
            source_file,
            get_node_field_required(ctx, &object, "ctx", "List")?,
        )?,
        range,
        runtime_elts,
    })
}

impl Node for ast::ExprList {
    fn ast_to_object(self, to_ctx: &AstToObjectContext<'_>) -> PyObjectRef {
        let vm = to_ctx.vm;
        let source_file = to_ctx.source_file;
        let Self {
            node_index: _,
            elts,
            ctx,
            range,
            runtime_elts,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprList::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        let elts = super::constant::public_ast_expr_list_object(to_ctx, runtime_elts).map_or_else(
            || elts.ast_to_object(to_ctx),
            |values| values.ast_to_object(to_ctx),
        );
        dict.set_item("elts", elts, vm).unwrap();
        dict.set_item("ctx", ctx.ast_to_object(to_ctx), vm).unwrap();
        node_add_location(&dict, range, vm, source_file);
        node.into()
    }

    fn ast_from_object(
        ctx: &AstFromObjectContext<'_>,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(ctx, source_file, object.clone(), "List")?;
        expr_list_from_object_with_range(ctx, source_file, object, range)
    }
}

// constructor
fn expr_tuple_from_object_with_range(
    ctx: &AstFromObjectContext<'_>,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::ExprTuple> {
    let elts: Vec<Option<ast::Expr>> =
        get_node_list_field(ctx, source_file, &object, "elts", "Tuple")?;
    let (runtime_elts, elts) = public_expr_list_from_values(elts);
    Ok(ast::ExprTuple {
        node_index: Default::default(),
        elts,
        ctx: Node::ast_from_object(
            ctx,
            source_file,
            get_node_field_required(ctx, &object, "ctx", "Tuple")?,
        )?,
        range,
        parenthesized: true,
        runtime_elts,
    })
}

impl Node for ast::ExprTuple {
    fn ast_to_object(self, to_ctx: &AstToObjectContext<'_>) -> PyObjectRef {
        let vm = to_ctx.vm;
        let source_file = to_ctx.source_file;
        let Self {
            node_index: _,
            elts,
            ctx,
            range: _range,
            parenthesized: _,
            runtime_elts,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprTuple::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        let elts = super::constant::public_ast_expr_list_object(to_ctx, runtime_elts).map_or_else(
            || elts.ast_to_object(to_ctx),
            |values| values.ast_to_object(to_ctx),
        );
        dict.set_item("elts", elts, vm).unwrap();
        dict.set_item("ctx", ctx.ast_to_object(to_ctx), vm).unwrap();
        node_add_location(&dict, _range, vm, source_file);
        node.into()
    }

    fn ast_from_object(
        ctx: &AstFromObjectContext<'_>,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(ctx, source_file, object.clone(), "Tuple")?;
        expr_tuple_from_object_with_range(ctx, source_file, object, range)
    }
}

// constructor
fn expr_slice_from_object_with_range(
    ctx: &AstFromObjectContext<'_>,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::ExprSlice> {
    Ok(ast::ExprSlice {
        node_index: Default::default(),
        lower: get_node_field_opt(ctx, &object, "lower")?
            .map(|obj| Node::ast_from_object(ctx, source_file, obj))
            .transpose()?,
        upper: get_node_field_opt(ctx, &object, "upper")?
            .map(|obj| Node::ast_from_object(ctx, source_file, obj))
            .transpose()?,
        step: get_node_field_opt(ctx, &object, "step")?
            .map(|obj| Node::ast_from_object(ctx, source_file, obj))
            .transpose()?,
        range,
    })
}

impl Node for ast::ExprSlice {
    fn ast_to_object(self, to_ctx: &AstToObjectContext<'_>) -> PyObjectRef {
        let ctx = to_ctx.vm;
        let source_file = to_ctx.source_file;
        let Self {
            node_index: _,
            lower,
            upper,
            step,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(ctx, pyast::NodeExprSlice::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("lower", lower.ast_to_object(to_ctx), ctx)
            .unwrap();
        dict.set_item("upper", upper.ast_to_object(to_ctx), ctx)
            .unwrap();
        dict.set_item("step", step.ast_to_object(to_ctx), ctx)
            .unwrap();
        node_add_location(&dict, _range, ctx, source_file);
        node.into()
    }

    fn ast_from_object(
        ctx: &AstFromObjectContext<'_>,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(ctx, source_file, object.clone(), "Slice")?;
        expr_slice_from_object_with_range(ctx, source_file, object, range)
    }
}

// sum
impl Node for ast::ExprContext {
    fn ast_to_object(self, to_ctx: &AstToObjectContext<'_>) -> PyObjectRef {
        let ctx = to_ctx.vm;
        let _source_file = to_ctx.source_file;
        let node_type = match self {
            Self::Load => pyast::NodeExprContextLoad::static_type(),
            Self::Store => pyast::NodeExprContextStore::static_type(),
            Self::Del => pyast::NodeExprContextDel::static_type(),
            Self::Invalid => {
                unreachable!()
            }
        };
        singleton_node_to_object(ctx, node_type)
    }

    fn ast_from_object(
        ctx: &AstFromObjectContext<'_>,
        _source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(
            if is_node_instance(ctx, &object, pyast::NodeExprContextLoad::static_type())? {
                Self::Load
            } else if is_node_instance(ctx, &object, pyast::NodeExprContextStore::static_type())? {
                Self::Store
            } else if is_node_instance(ctx, &object, pyast::NodeExprContextDel::static_type())? {
                Self::Del
            } else {
                return Err(ctx.new_type_error(format!(
                    "expected some sort of expr_context, but got {}",
                    object.repr(ctx)?
                )));
            },
        )
    }
}

// product
impl Node for ast::Comprehension {
    fn ast_to_object(self, to_ctx: &AstToObjectContext<'_>) -> PyObjectRef {
        let ctx = to_ctx.vm;
        let _source_file = to_ctx.source_file;
        let Self {
            node_index: _,
            target,
            iter,
            ifs,
            is_async,
            range: _range,
            runtime_ifs,
            runtime_is_async,
        } = self;
        let node = NodeAst
            .into_ref_with_type(ctx, pyast::NodeComprehension::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("target", target.ast_to_object(to_ctx), ctx)
            .unwrap();
        dict.set_item("iter", iter.ast_to_object(to_ctx), ctx)
            .unwrap();
        let ifs = super::constant::public_ast_expr_list_object(to_ctx, runtime_ifs).map_or_else(
            || ifs.ast_to_object(to_ctx),
            |values| values.ast_to_object(to_ctx),
        );
        dict.set_item("ifs", ifs, ctx).unwrap();
        let is_async =
            super::constant::public_ast_comprehension_is_async_object(to_ctx, runtime_is_async)
                .map_or_else(
                    || is_async.ast_to_object(to_ctx),
                    |value| ctx.ctx.new_int(value).into(),
                );
        dict.set_item("is_async", is_async, ctx).unwrap();
        node.into()
    }

    fn ast_from_object(
        ctx: &AstFromObjectContext<'_>,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let ifs: Vec<Option<ast::Expr>> =
            get_node_list_field(ctx, source_file, &object, "ifs", "comprehension")?;
        let is_async = node_object_to_i32(
            ctx,
            get_node_field(ctx, &object, "is_async", "comprehension")?,
        )?;
        let runtime_ifs = public_expr_list_metadata(&ifs);
        let runtime_is_async = (is_async != 0 && is_async != 1).then_some(is_async);
        Ok(Self {
            node_index: Default::default(),
            target: get_required_node_field(ctx, source_file, &object, "target", "comprehension")?,
            iter: get_required_node_field(ctx, source_file, &object, "iter", "comprehension")?,
            ifs: lower_public_expr_list(ifs),
            is_async: is_async != 0,
            range: Default::default(),
            runtime_ifs,
            runtime_is_async,
        })
    }
}
