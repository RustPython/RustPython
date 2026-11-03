use super::AstSourceContext;
use super::*;
use crate::stdlib::_ast::argument::{
    KeywordArguments, PositionalArguments, merge_function_call_arguments,
    split_function_call_arguments,
};

// sum
impl Node for ast::Expr {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &AstSourceContext) -> PyObjectRef {
        match self {
            Self::BoolOp(cons) => cons.ast_to_object(vm, source_file),
            Self::Name(cons) => cons.ast_to_object(vm, source_file),
            Self::BinOp(cons) => cons.ast_to_object(vm, source_file),
            Self::UnaryOp(cons) => cons.ast_to_object(vm, source_file),
            Self::Lambda(cons) => cons.ast_to_object(vm, source_file),
            Self::If(cons) => cons.ast_to_object(vm, source_file),
            Self::Dict(cons) => cons.ast_to_object(vm, source_file),
            Self::Set(cons) => cons.ast_to_object(vm, source_file),
            Self::ListComp(cons) => cons.ast_to_object(vm, source_file),
            Self::SetComp(cons) => cons.ast_to_object(vm, source_file),
            Self::DictComp(cons) => cons.ast_to_object(vm, source_file),
            Self::Generator(cons) => cons.ast_to_object(vm, source_file),
            Self::Await(cons) => cons.ast_to_object(vm, source_file),
            Self::Yield(cons) => cons.ast_to_object(vm, source_file),
            Self::YieldFrom(cons) => cons.ast_to_object(vm, source_file),
            Self::Compare(cons) => cons.ast_to_object(vm, source_file),
            Self::Call(cons) => cons.ast_to_object(vm, source_file),
            Self::Constant(cons) => constant::expr_constant_to_object(vm, source_file, cons),
            Self::Attribute(cons) => cons.ast_to_object(vm, source_file),
            Self::Subscript(cons) => cons.ast_to_object(vm, source_file),
            Self::Starred(cons) => cons.ast_to_object(vm, source_file),
            Self::List(cons) => cons.ast_to_object(vm, source_file),
            Self::Tuple(cons) => cons.ast_to_object(vm, source_file),
            Self::Slice(cons) => cons.ast_to_object(vm, source_file),
            Self::NumberLiteral(cons) => constant::number_literal_to_object(vm, source_file, cons),
            Self::StringLiteral(cons) => constant::string_literal_to_object(vm, source_file, cons),
            Self::FString(cons) => string::fstring_to_object(vm, source_file, cons),
            Self::TString(cons) => string::tstring_to_object(vm, source_file, cons),
            Self::BytesLiteral(cons) => constant::bytes_literal_to_object(vm, source_file, cons),
            Self::BooleanLiteral(cons) => {
                constant::boolean_literal_to_object(vm, source_file, cons)
            }
            Self::NoneLiteral(cons) => constant::none_literal_to_object(vm, source_file, cons),
            Self::EllipsisLiteral(cons) => {
                constant::ellipsis_literal_to_object(vm, source_file, cons)
            }
            Self::Named(cons) => cons.ast_to_object(vm, source_file),
            Self::IpyEscapeCommand(_) => {
                unreachable!("IPython escape command is not part of Python AST")
            }
        }
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &AstSourceContext,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        if vm.is_none(&object) {
            return Err(vm.new_type_error(format!(
                "expected some sort of expr, but got {}",
                object.repr(vm)?
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
        let kind = if is_node_instance(vm, &object, pyast::NodeExprBoolOp::static_type())? {
            ExprKind::BoolOp
        } else if is_node_instance(vm, &object, pyast::NodeExprNamedExpr::static_type())? {
            ExprKind::Named
        } else if is_node_instance(vm, &object, pyast::NodeExprBinOp::static_type())? {
            ExprKind::BinOp
        } else if is_node_instance(vm, &object, pyast::NodeExprUnaryOp::static_type())? {
            ExprKind::UnaryOp
        } else if is_node_instance(vm, &object, pyast::NodeExprLambda::static_type())? {
            ExprKind::Lambda
        } else if is_node_instance(vm, &object, pyast::NodeExprIfExp::static_type())? {
            ExprKind::If
        } else if is_node_instance(vm, &object, pyast::NodeExprDict::static_type())? {
            ExprKind::Dict
        } else if is_node_instance(vm, &object, pyast::NodeExprSet::static_type())? {
            ExprKind::Set
        } else if is_node_instance(vm, &object, pyast::NodeExprListComp::static_type())? {
            ExprKind::ListComp
        } else if is_node_instance(vm, &object, pyast::NodeExprSetComp::static_type())? {
            ExprKind::SetComp
        } else if is_node_instance(vm, &object, pyast::NodeExprDictComp::static_type())? {
            ExprKind::DictComp
        } else if is_node_instance(vm, &object, pyast::NodeExprGeneratorExp::static_type())? {
            ExprKind::Generator
        } else if is_node_instance(vm, &object, pyast::NodeExprAwait::static_type())? {
            ExprKind::Await
        } else if is_node_instance(vm, &object, pyast::NodeExprYield::static_type())? {
            ExprKind::Yield
        } else if is_node_instance(vm, &object, pyast::NodeExprYieldFrom::static_type())? {
            ExprKind::YieldFrom
        } else if is_node_instance(vm, &object, pyast::NodeExprCompare::static_type())? {
            ExprKind::Compare
        } else if is_node_instance(vm, &object, pyast::NodeExprCall::static_type())? {
            ExprKind::Call
        } else if is_node_instance(vm, &object, pyast::NodeExprFormattedValue::static_type())? {
            ExprKind::FormattedValue
        } else if is_node_instance(vm, &object, pyast::NodeExprInterpolation::static_type())? {
            ExprKind::Interpolation
        } else if is_node_instance(vm, &object, pyast::NodeExprJoinedStr::static_type())? {
            ExprKind::JoinedStr
        } else if is_node_instance(vm, &object, pyast::NodeExprTemplateStr::static_type())? {
            ExprKind::TemplateStr
        } else if is_node_instance(vm, &object, pyast::NodeExprConstant::static_type())? {
            ExprKind::Constant
        } else if is_node_instance(vm, &object, pyast::NodeExprAttribute::static_type())? {
            ExprKind::Attribute
        } else if is_node_instance(vm, &object, pyast::NodeExprSubscript::static_type())? {
            ExprKind::Subscript
        } else if is_node_instance(vm, &object, pyast::NodeExprStarred::static_type())? {
            ExprKind::Starred
        } else if is_node_instance(vm, &object, pyast::NodeExprName::static_type())? {
            ExprKind::Name
        } else if is_node_instance(vm, &object, pyast::NodeExprList::static_type())? {
            ExprKind::List
        } else if is_node_instance(vm, &object, pyast::NodeExprTuple::static_type())? {
            ExprKind::Tuple
        } else if is_node_instance(vm, &object, pyast::NodeExprSlice::static_type())? {
            ExprKind::Slice
        } else {
            return Err(vm.new_type_error(format!(
                "expected some sort of expr, but got {}",
                object.repr(vm)?
            )));
        };
        let range = expr_range_from_object(vm, source_file, &object)?;
        let expression = match kind {
            ExprKind::BoolOp => Self::BoolOp(expr_bool_op_from_object_with_range(
                vm,
                source_file,
                &object,
                range,
            )?),
            ExprKind::Named => Self::Named(expr_named_from_object_with_range(
                vm,
                source_file,
                &object,
                range,
            )?),
            ExprKind::BinOp => Self::BinOp(expr_bin_op_from_object_with_range(
                vm,
                source_file,
                &object,
                range,
            )?),
            ExprKind::UnaryOp => Self::UnaryOp(expr_unary_op_from_object_with_range(
                vm,
                source_file,
                &object,
                range,
            )?),
            ExprKind::Lambda => Self::Lambda(expr_lambda_from_object_with_range(
                vm,
                source_file,
                &object,
                range,
            )?),
            ExprKind::If => Self::If(expr_if_from_object_with_range(
                vm,
                source_file,
                &object,
                range,
            )?),
            ExprKind::Dict => Self::Dict(expr_dict_from_object_with_range(
                vm,
                source_file,
                &object,
                range,
            )?),
            ExprKind::Set => Self::Set(expr_set_from_object_with_range(
                vm,
                source_file,
                &object,
                range,
            )?),
            ExprKind::ListComp => Self::ListComp(expr_list_comp_from_object_with_range(
                vm,
                source_file,
                &object,
                range,
            )?),
            ExprKind::SetComp => Self::SetComp(expr_set_comp_from_object_with_range(
                vm,
                source_file,
                &object,
                range,
            )?),
            ExprKind::DictComp => Self::DictComp(expr_dict_comp_from_object_with_range(
                vm,
                source_file,
                &object,
                range,
            )?),
            ExprKind::Generator => Self::Generator(expr_generator_from_object_with_range(
                vm,
                source_file,
                &object,
                range,
            )?),
            ExprKind::Await => Self::Await(expr_await_from_object_with_range(
                vm,
                source_file,
                &object,
                range,
            )?),
            ExprKind::Yield => Self::Yield(expr_yield_from_object_with_range(
                vm,
                source_file,
                &object,
                range,
            )?),
            ExprKind::YieldFrom => Self::YieldFrom(expr_yield_from_from_object_with_range(
                vm,
                source_file,
                &object,
                range,
            )?),
            ExprKind::Compare => Self::Compare(expr_compare_from_object_with_range(
                vm,
                source_file,
                &object,
                range,
            )?),
            ExprKind::Call => Self::Call(expr_call_from_object_with_range(
                vm,
                source_file,
                &object,
                range,
            )?),
            ExprKind::FormattedValue => {
                let formatted = string::formatted_value_from_object_with_range(
                    vm,
                    source_file,
                    &object,
                    range,
                )?;
                string::formatted_value_to_expr(true, formatted)
            }
            ExprKind::Interpolation => {
                let interpolation = string::tstring_interpolation_from_object_with_range(
                    vm,
                    source_file,
                    &object,
                    range,
                )?;
                string::interpolation_to_expr(vm, source_file, interpolation)?
            }
            ExprKind::JoinedStr => {
                string::joined_str_from_object_with_range(vm, source_file, &object, range)?
                    .into_expr(true)
            }
            ExprKind::TemplateStr => {
                let template =
                    string::template_str_from_object_with_range(vm, source_file, &object, range)?;
                string::template_str_to_expr(vm, source_file, template)?
            }
            ExprKind::Constant => {
                constant::constant_from_object_with_range(vm, source_file, &object, range)?
                    .into_expr()
            }
            ExprKind::Attribute => Self::Attribute(expr_attribute_from_object_with_range(
                vm,
                source_file,
                &object,
                range,
            )?),
            ExprKind::Subscript => Self::Subscript(expr_subscript_from_object_with_range(
                vm,
                source_file,
                &object,
                range,
            )?),
            ExprKind::Starred => Self::Starred(expr_starred_from_object_with_range(
                vm,
                source_file,
                &object,
                range,
            )?),
            ExprKind::Name => Self::Name(expr_name_from_object_with_range(
                vm,
                source_file,
                &object,
                range,
            )?),
            ExprKind::List => Self::List(expr_list_from_object_with_range(
                vm,
                source_file,
                &object,
                range,
            )?),
            ExprKind::Tuple => Self::Tuple(expr_tuple_from_object_with_range(
                vm,
                source_file,
                &object,
                range,
            )?),
            ExprKind::Slice => Self::Slice(expr_slice_from_object_with_range(
                vm,
                source_file,
                &object,
                range,
            )?),
        };
        ast::HasNodeIndex::node_index(&expression).set(range.index);
        Ok(expression)
    }
}

// constructor
fn expr_bool_op_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &AstSourceContext,
    object: &PyObject,
    range: AstNodeRange,
) -> PyResult<ast::ExprBoolOp> {
    let values: Vec<Option<ast::Expr>> =
        get_node_list_field(vm, source_file, object, "values", "BoolOp")?;
    let (runtime_values, values) = runtime_expr_list_from_values(values);
    Ok(ast::ExprBoolOp {
        node_index: range.node_index(),
        op: Node::ast_from_object(
            vm,
            source_file,
            get_node_field_required(vm, object, "op", "BoolOp")?,
        )?,
        values,
        range: range.range,
        runtime_values,
    })
}

impl Node for ast::ExprBoolOp {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &AstSourceContext) -> PyObjectRef {
        let Self {
            node_index,
            op,
            values,
            range,
            runtime_values,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprBoolOp::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("op", op.ast_to_object(vm, source_file), vm)
            .unwrap();
        let values = runtime_values.map_or_else(
            || values.ast_to_object(vm, source_file),
            |values| values.ast_to_object(vm, source_file),
        );
        dict.set_item("values", values, vm).unwrap();
        node_add_location_for_node(&dict, range, vm, source_file, node_index.load());
        node.into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &AstSourceContext,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, &object, "BoolOp")?;
        expr_bool_op_from_object_with_range(vm, source_file, &object, range)
    }
}

// constructor
fn expr_named_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &AstSourceContext,
    object: &PyObject,
    range: AstNodeRange,
) -> PyResult<ast::ExprNamed> {
    Ok(ast::ExprNamed {
        node_index: range.node_index(),
        target: get_required_node_field(vm, source_file, object, "target", "NamedExpr")?,
        value: get_required_node_field(vm, source_file, object, "value", "NamedExpr")?,
        range: range.range,
    })
}

impl Node for ast::ExprNamed {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &AstSourceContext) -> PyObjectRef {
        let Self {
            node_index,
            target,
            value,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprNamedExpr::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("target", target.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item("value", value.ast_to_object(vm, source_file), vm)
            .unwrap();
        node_add_location_for_node(&dict, range, vm, source_file, node_index.load());
        node.into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &AstSourceContext,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, &object, "NamedExpr")?;
        expr_named_from_object_with_range(vm, source_file, &object, range)
    }
}

// constructor
fn expr_bin_op_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &AstSourceContext,
    object: &PyObject,
    range: AstNodeRange,
) -> PyResult<ast::ExprBinOp> {
    Ok(ast::ExprBinOp {
        node_index: range.node_index(),
        left: get_required_node_field(vm, source_file, object, "left", "BinOp")?,
        op: Node::ast_from_object(
            vm,
            source_file,
            get_node_field_required(vm, object, "op", "BinOp")?,
        )?,
        right: get_required_node_field(vm, source_file, object, "right", "BinOp")?,
        range: range.range,
    })
}

impl Node for ast::ExprBinOp {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &AstSourceContext) -> PyObjectRef {
        let Self {
            node_index,
            left,
            op,
            right,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprBinOp::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("left", left.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item("op", op.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item("right", right.ast_to_object(vm, source_file), vm)
            .unwrap();
        node_add_location_for_node(&dict, range, vm, source_file, node_index.load());
        node.into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &AstSourceContext,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, &object, "BinOp")?;
        expr_bin_op_from_object_with_range(vm, source_file, &object, range)
    }
}

// constructor
fn expr_unary_op_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &AstSourceContext,
    object: &PyObject,
    range: AstNodeRange,
) -> PyResult<ast::ExprUnaryOp> {
    Ok(ast::ExprUnaryOp {
        node_index: range.node_index(),
        op: Node::ast_from_object(
            vm,
            source_file,
            get_node_field_required(vm, object, "op", "UnaryOp")?,
        )?,
        operand: get_required_node_field(vm, source_file, object, "operand", "UnaryOp")?,
        range: range.range,
    })
}

impl Node for ast::ExprUnaryOp {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &AstSourceContext) -> PyObjectRef {
        let Self {
            node_index,
            op,
            operand,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprUnaryOp::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("op", op.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item("operand", operand.ast_to_object(vm, source_file), vm)
            .unwrap();
        node_add_location_for_node(&dict, range, vm, source_file, node_index.load());
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &AstSourceContext,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, &object, "UnaryOp")?;
        expr_unary_op_from_object_with_range(vm, source_file, &object, range)
    }
}

// constructor
fn expr_lambda_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &AstSourceContext,
    object: &PyObject,
    range: AstNodeRange,
) -> PyResult<ast::ExprLambda> {
    Ok(ast::ExprLambda {
        node_index: range.node_index(),
        parameters: Node::ast_from_object(
            vm,
            source_file,
            get_node_field_required(vm, object, "args", "Lambda")?,
        )?,
        body: get_required_node_field(vm, source_file, object, "body", "Lambda")?,
        range: range.range,
    })
}

impl Node for ast::ExprLambda {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &AstSourceContext) -> PyObjectRef {
        let Self {
            node_index,
            parameters,
            body,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprLambda::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        let args = match parameters {
            Some(params) => params.ast_to_object(vm, source_file),
            None => empty_arguments_object(vm),
        };
        dict.set_item("args", args, vm).unwrap();
        dict.set_item("body", body.ast_to_object(vm, source_file), vm)
            .unwrap();
        node_add_location_for_node(&dict, _range, vm, source_file, node_index.load());
        node.into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &AstSourceContext,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, &object, "Lambda")?;
        expr_lambda_from_object_with_range(vm, source_file, &object, range)
    }
}

// constructor
fn expr_if_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &AstSourceContext,
    object: &PyObject,
    range: AstNodeRange,
) -> PyResult<ast::ExprIf> {
    Ok(ast::ExprIf {
        node_index: range.node_index(),
        test: get_required_node_field(vm, source_file, object, "test", "IfExp")?,
        body: get_required_node_field(vm, source_file, object, "body", "IfExp")?,
        orelse: get_required_node_field(vm, source_file, object, "orelse", "IfExp")?,
        range: range.range,
    })
}

impl Node for ast::ExprIf {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &AstSourceContext) -> PyObjectRef {
        let Self {
            node_index,
            test,
            body,
            orelse,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprIfExp::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("test", test.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item("body", body.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item("orelse", orelse.ast_to_object(vm, source_file), vm)
            .unwrap();
        node_add_location_for_node(&dict, range, vm, source_file, node_index.load());
        node.into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &AstSourceContext,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, &object, "IfExp")?;
        expr_if_from_object_with_range(vm, source_file, &object, range)
    }
}

// constructor
fn expr_dict_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &AstSourceContext,
    object: &PyObject,
    range: AstNodeRange,
) -> PyResult<ast::ExprDict> {
    let keys: Vec<Option<ast::Expr>> =
        get_node_list_field(vm, source_file, object, "keys", "Dict")?;
    let values: Vec<Option<ast::Expr>> =
        get_node_list_field(vm, source_file, object, "values", "Dict")?;
    if keys.len() != values.len() {
        return Err(vm.new_value_error("Dict doesn't have the same number of keys as values"));
    }
    let runtime_values = runtime_expr_list_metadata(&values);
    let items = keys
        .into_iter()
        .zip(lower_runtime_expr_list(values))
        .map(|(key, value)| ast::DictItem { key, value })
        .collect();
    Ok(ast::ExprDict {
        node_index: range.node_index(),
        items,
        range: range.range,
        runtime_values,
    })
}

impl Node for ast::ExprDict {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &AstSourceContext) -> PyObjectRef {
        let Self {
            node_index,
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
        dict.set_item("keys", keys.ast_to_object(vm, source_file), vm)
            .unwrap();
        let values = runtime_values.map_or_else(
            || values.ast_to_object(vm, source_file),
            |values| values.ast_to_object(vm, source_file),
        );
        dict.set_item("values", values, vm).unwrap();
        node_add_location_for_node(&dict, range, vm, source_file, node_index.load());
        node.into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &AstSourceContext,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, &object, "Dict")?;
        expr_dict_from_object_with_range(vm, source_file, &object, range)
    }
}

// constructor
fn expr_set_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &AstSourceContext,
    object: &PyObject,
    range: AstNodeRange,
) -> PyResult<ast::ExprSet> {
    let elts: Vec<Option<ast::Expr>> = get_node_list_field(vm, source_file, object, "elts", "Set")?;
    let (runtime_elts, elts) = runtime_expr_list_from_values(elts);
    Ok(ast::ExprSet {
        node_index: range.node_index(),
        elts,
        range: range.range,
        runtime_elts,
    })
}

impl Node for ast::ExprSet {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &AstSourceContext) -> PyObjectRef {
        let Self {
            node_index,
            elts,
            range,
            runtime_elts,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprSet::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        let elts = runtime_elts.map_or_else(
            || elts.ast_to_object(vm, source_file),
            |values| values.ast_to_object(vm, source_file),
        );
        dict.set_item("elts", elts, vm).unwrap();
        node_add_location_for_node(&dict, range, vm, source_file, node_index.load());
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &AstSourceContext,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, &object, "Set")?;
        expr_set_from_object_with_range(vm, source_file, &object, range)
    }
}

// constructor
fn expr_list_comp_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &AstSourceContext,
    object: &PyObject,
    range: AstNodeRange,
) -> PyResult<ast::ExprListComp> {
    Ok(ast::ExprListComp {
        node_index: range.node_index(),
        elt: get_required_node_field(vm, source_file, object, "elt", "ListComp")?,
        generators: get_node_list_field(vm, source_file, object, "generators", "ListComp")?,
        range: range.range,
    })
}

impl Node for ast::ExprListComp {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &AstSourceContext) -> PyObjectRef {
        let Self {
            node_index,
            elt,
            generators,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprListComp::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("elt", elt.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item("generators", generators.ast_to_object(vm, source_file), vm)
            .unwrap();
        node_add_location_for_node(&dict, range, vm, source_file, node_index.load());
        node.into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &AstSourceContext,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, &object, "ListComp")?;
        expr_list_comp_from_object_with_range(vm, source_file, &object, range)
    }
}

// constructor
fn expr_set_comp_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &AstSourceContext,
    object: &PyObject,
    range: AstNodeRange,
) -> PyResult<ast::ExprSetComp> {
    Ok(ast::ExprSetComp {
        node_index: range.node_index(),
        elt: get_required_node_field(vm, source_file, object, "elt", "SetComp")?,
        generators: get_node_list_field(vm, source_file, object, "generators", "SetComp")?,
        range: range.range,
    })
}

impl Node for ast::ExprSetComp {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &AstSourceContext) -> PyObjectRef {
        let Self {
            node_index,
            elt,
            generators,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprSetComp::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("elt", elt.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item("generators", generators.ast_to_object(vm, source_file), vm)
            .unwrap();
        node_add_location_for_node(&dict, range, vm, source_file, node_index.load());
        node.into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &AstSourceContext,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, &object, "SetComp")?;
        expr_set_comp_from_object_with_range(vm, source_file, &object, range)
    }
}

// constructor
fn expr_dict_comp_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &AstSourceContext,
    object: &PyObject,
    range: AstNodeRange,
) -> PyResult<ast::ExprDictComp> {
    // Ruff uses a missing key for unpacking; Python's AST uses a missing value.
    let key = get_required_node_field(vm, source_file, object, "key", "DictComp")?;
    let value = get_node_field_opt(vm, object, "value")?
        .map(|obj| Node::ast_from_object(vm, source_file, obj))
        .transpose()?;
    let (key, value) = match value {
        Some(value) => (Some(key), value),
        None => (None, key),
    };
    Ok(ast::ExprDictComp {
        node_index: range.node_index(),
        key,
        value,
        generators: get_node_list_field(vm, source_file, object, "generators", "DictComp")?
            .into_boxed_slice(),
        range: range.range,
    })
}

impl Node for ast::ExprDictComp {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &AstSourceContext) -> PyObjectRef {
        let Self {
            node_index,
            key,
            value,
            generators,
            range,
        } = self;
        let (key, value) = match key {
            Some(key) => (key, Some(value)),
            None => (value, None),
        };
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprDictComp::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("key", key.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item("value", value.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item(
            "generators",
            BoxedSlice(generators).ast_to_object(vm, source_file),
            vm,
        )
        .unwrap();
        node_add_location_for_node(&dict, range, vm, source_file, node_index.load());
        node.into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &AstSourceContext,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, &object, "DictComp")?;
        expr_dict_comp_from_object_with_range(vm, source_file, &object, range)
    }
}

// constructor
fn expr_generator_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &AstSourceContext,
    object: &PyObject,
    range: AstNodeRange,
) -> PyResult<ast::ExprGenerator> {
    Ok(ast::ExprGenerator {
        node_index: range.node_index(),
        elt: get_required_node_field(vm, source_file, object, "elt", "GeneratorExp")?,
        generators: get_node_list_field(vm, source_file, object, "generators", "GeneratorExp")?,
        range: range.range,
        parenthesized: true,
    })
}

impl Node for ast::ExprGenerator {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &AstSourceContext) -> PyObjectRef {
        let Self {
            node_index,
            elt,
            generators,
            range,
            parenthesized: _,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprGeneratorExp::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("elt", elt.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item("generators", generators.ast_to_object(vm, source_file), vm)
            .unwrap();
        node_add_location_for_node(&dict, range, vm, source_file, node_index.load());
        node.into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &AstSourceContext,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, &object, "GeneratorExp")?;
        expr_generator_from_object_with_range(vm, source_file, &object, range)
    }
}

// constructor
fn expr_await_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &AstSourceContext,
    object: &PyObject,
    range: AstNodeRange,
) -> PyResult<ast::ExprAwait> {
    Ok(ast::ExprAwait {
        node_index: range.node_index(),
        value: get_required_node_field(vm, source_file, object, "value", "Await")?,
        range: range.range,
    })
}

impl Node for ast::ExprAwait {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &AstSourceContext) -> PyObjectRef {
        let Self {
            node_index,
            value,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprAwait::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(vm, source_file), vm)
            .unwrap();
        node_add_location_for_node(&dict, range, vm, source_file, node_index.load());
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &AstSourceContext,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, &object, "Await")?;
        expr_await_from_object_with_range(vm, source_file, &object, range)
    }
}

// constructor
fn expr_yield_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &AstSourceContext,
    object: &PyObject,
    range: AstNodeRange,
) -> PyResult<ast::ExprYield> {
    Ok(ast::ExprYield {
        node_index: range.node_index(),
        value: get_node_field_opt(vm, object, "value")?
            .map(|obj| Node::ast_from_object(vm, source_file, obj))
            .transpose()?,
        range: range.range,
    })
}

impl Node for ast::ExprYield {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &AstSourceContext) -> PyObjectRef {
        let Self {
            node_index,
            value,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprYield::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(vm, source_file), vm)
            .unwrap();
        node_add_location_for_node(&dict, range, vm, source_file, node_index.load());
        node.into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &AstSourceContext,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, &object, "Yield")?;
        expr_yield_from_object_with_range(vm, source_file, &object, range)
    }
}

// constructor
fn expr_yield_from_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &AstSourceContext,
    object: &PyObject,
    range: AstNodeRange,
) -> PyResult<ast::ExprYieldFrom> {
    Ok(ast::ExprYieldFrom {
        node_index: range.node_index(),
        value: get_required_node_field(vm, source_file, object, "value", "YieldFrom")?,
        range: range.range,
    })
}

impl Node for ast::ExprYieldFrom {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &AstSourceContext) -> PyObjectRef {
        let Self {
            node_index,
            value,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprYieldFrom::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(vm, source_file), vm)
            .unwrap();
        node_add_location_for_node(&dict, range, vm, source_file, node_index.load());
        node.into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &AstSourceContext,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, &object, "YieldFrom")?;
        expr_yield_from_from_object_with_range(vm, source_file, &object, range)
    }
}

// constructor
fn expr_compare_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &AstSourceContext,
    object: &PyObject,
    range: AstNodeRange,
) -> PyResult<ast::ExprCompare> {
    let left: ast::Expr = get_required_node_field(vm, source_file, object, "left", "Compare")?;
    let comparators: Vec<Option<ast::Expr>> =
        get_node_list_field(vm, source_file, object, "comparators", "Compare")?;
    let (runtime_comparators, comparators) = runtime_expr_boxed_slice_from_values(comparators);
    let mut operands = Vec::with_capacity(comparators.len() + 1);
    operands.push(left);
    operands.extend(comparators);
    Ok(ast::ExprCompare {
        node_index: range.node_index(),
        ops: get_node_boxed_slice_field(vm, source_file, object, "ops", "Compare")?,
        operands: operands.into_boxed_slice(),
        range: range.range,
        runtime_comparators,
    })
}

impl Node for ast::ExprCompare {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &AstSourceContext) -> PyObjectRef {
        let Self {
            node_index,
            ops,
            operands,
            range,
            runtime_comparators,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprCompare::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        let mut operands = operands.into_vec().into_iter();
        let left = operands.next().expect("A comparison has a left operand");
        dict.set_item("left", left.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item("ops", BoxedSlice(ops).ast_to_object(vm, source_file), vm)
            .unwrap();
        let comparators: Box<[ast::Expr]> = operands.collect();
        let comparators = runtime_comparators.map_or_else(
            || BoxedSlice(comparators).ast_to_object(vm, source_file),
            |values| values.ast_to_object(vm, source_file),
        );
        dict.set_item("comparators", comparators, vm).unwrap();
        node_add_location_for_node(&dict, range, vm, source_file, node_index.load());
        node.into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &AstSourceContext,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, &object, "Compare")?;
        expr_compare_from_object_with_range(vm, source_file, &object, range)
    }
}

// constructor
fn expr_call_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &AstSourceContext,
    object: &PyObject,
    range: AstNodeRange,
) -> PyResult<ast::ExprCall> {
    let mut arguments = merge_function_call_arguments(
        PositionalArguments::ast_from_field(vm, source_file, object, "args", "Call")?,
        KeywordArguments::ast_from_field(vm, source_file, object, "keywords", "Call")?,
    );
    arguments.range = range.range;
    Ok(ast::ExprCall {
        node_index: range.node_index(),
        func: get_required_node_field(vm, source_file, object, "func", "Call")?,
        arguments,
        range_start: range.start(),
    })
}

impl Node for ast::ExprCall {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &AstSourceContext) -> PyObjectRef {
        let range = self.range();
        let Self {
            node_index,
            func,
            arguments,
            range_start: _,
        } = self;
        let (positional_arguments, keyword_arguments) = split_function_call_arguments(arguments);
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprCall::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("func", func.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item(
            "args",
            positional_arguments.ast_to_object(vm, source_file),
            vm,
        )
        .unwrap();
        dict.set_item(
            "keywords",
            keyword_arguments.ast_to_object(vm, source_file),
            vm,
        )
        .unwrap();
        node_add_location_for_node(&dict, range, vm, source_file, node_index.load());
        node.into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &AstSourceContext,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, &object, "Call")?;
        expr_call_from_object_with_range(vm, source_file, &object, range)
    }
}

// constructor
fn expr_attribute_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &AstSourceContext,
    object: &PyObject,
    range: AstNodeRange,
) -> PyResult<ast::ExprAttribute> {
    Ok(ast::ExprAttribute {
        node_index: range.node_index(),
        value: get_required_node_field(vm, source_file, object, "value", "Attribute")?,
        attr: get_required_identifier_field(vm, source_file, object, "attr", "Attribute")?,
        ctx: Node::ast_from_object(
            vm,
            source_file,
            get_node_field_required(vm, object, "ctx", "Attribute")?,
        )?,
        range: range.range,
    })
}

impl Node for ast::ExprAttribute {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &AstSourceContext) -> PyObjectRef {
        let Self {
            node_index,
            value,
            attr,
            ctx,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprAttribute::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item("attr", attr.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item("ctx", ctx.ast_to_object(vm, source_file), vm)
            .unwrap();
        node_add_location_for_node(&dict, range, vm, source_file, node_index.load());
        node.into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &AstSourceContext,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, &object, "Attribute")?;
        expr_attribute_from_object_with_range(vm, source_file, &object, range)
    }
}

// constructor
fn expr_subscript_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &AstSourceContext,
    object: &PyObject,
    range: AstNodeRange,
) -> PyResult<ast::ExprSubscript> {
    Ok(ast::ExprSubscript {
        node_index: range.node_index(),
        value: get_required_node_field(vm, source_file, object, "value", "Subscript")?,
        slice: get_required_node_field(vm, source_file, object, "slice", "Subscript")?,
        ctx: Node::ast_from_object(
            vm,
            source_file,
            get_node_field_required(vm, object, "ctx", "Subscript")?,
        )?,
        range: range.range,
    })
}

impl Node for ast::ExprSubscript {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &AstSourceContext) -> PyObjectRef {
        let Self {
            node_index,
            value,
            slice,
            ctx,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprSubscript::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item("slice", slice.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item("ctx", ctx.ast_to_object(vm, source_file), vm)
            .unwrap();
        node_add_location_for_node(&dict, _range, vm, source_file, node_index.load());
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &AstSourceContext,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, &object, "Subscript")?;
        expr_subscript_from_object_with_range(vm, source_file, &object, range)
    }
}

// constructor
fn expr_starred_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &AstSourceContext,
    object: &PyObject,
    range: AstNodeRange,
) -> PyResult<ast::ExprStarred> {
    Ok(ast::ExprStarred {
        node_index: range.node_index(),
        value: get_required_node_field(vm, source_file, object, "value", "Starred")?,
        ctx: Node::ast_from_object(
            vm,
            source_file,
            get_node_field_required(vm, object, "ctx", "Starred")?,
        )?,
        range: range.range,
    })
}

impl Node for ast::ExprStarred {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &AstSourceContext) -> PyObjectRef {
        let Self {
            node_index,
            value,
            ctx,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprStarred::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item("ctx", ctx.ast_to_object(vm, source_file), vm)
            .unwrap();
        node_add_location_for_node(&dict, range, vm, source_file, node_index.load());
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &AstSourceContext,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, &object, "Starred")?;
        expr_starred_from_object_with_range(vm, source_file, &object, range)
    }
}

// constructor
fn expr_name_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &AstSourceContext,
    object: &PyObject,
    range: AstNodeRange,
) -> PyResult<ast::ExprName> {
    Ok(ast::ExprName {
        node_index: range.node_index(),
        id: get_required_identifier_field(vm, source_file, object, "id", "Name")?,
        ctx: Node::ast_from_object(
            vm,
            source_file,
            get_node_field_required(vm, object, "ctx", "Name")?,
        )?,
        range: range.range,
    })
}

impl Node for ast::ExprName {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &AstSourceContext) -> PyObjectRef {
        let Self {
            node_index,
            id,
            ctx,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprName::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("id", id.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item("ctx", ctx.ast_to_object(vm, source_file), vm)
            .unwrap();
        node_add_location_for_node(&dict, range, vm, source_file, node_index.load());
        node.into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &AstSourceContext,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, &object, "Name")?;
        expr_name_from_object_with_range(vm, source_file, &object, range)
    }
}

// constructor
fn expr_list_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &AstSourceContext,
    object: &PyObject,
    range: AstNodeRange,
) -> PyResult<ast::ExprList> {
    let elts: Vec<Option<ast::Expr>> =
        get_node_list_field(vm, source_file, object, "elts", "List")?;
    let (runtime_elts, elts) = runtime_expr_list_from_values(elts);
    Ok(ast::ExprList {
        node_index: range.node_index(),
        elts,
        ctx: Node::ast_from_object(
            vm,
            source_file,
            get_node_field_required(vm, object, "ctx", "List")?,
        )?,
        range: range.range,
        runtime_elts,
    })
}

impl Node for ast::ExprList {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &AstSourceContext) -> PyObjectRef {
        let Self {
            node_index,
            elts,
            ctx,
            range,
            runtime_elts,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprList::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        let elts = runtime_elts.map_or_else(
            || elts.ast_to_object(vm, source_file),
            |values| values.ast_to_object(vm, source_file),
        );
        dict.set_item("elts", elts, vm).unwrap();
        dict.set_item("ctx", ctx.ast_to_object(vm, source_file), vm)
            .unwrap();
        node_add_location_for_node(&dict, range, vm, source_file, node_index.load());
        node.into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &AstSourceContext,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, &object, "List")?;
        expr_list_from_object_with_range(vm, source_file, &object, range)
    }
}

// constructor
fn expr_tuple_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &AstSourceContext,
    object: &PyObject,
    range: AstNodeRange,
) -> PyResult<ast::ExprTuple> {
    let elts: Vec<Option<ast::Expr>> =
        get_node_list_field(vm, source_file, object, "elts", "Tuple")?;
    let (runtime_elts, elts) = runtime_expr_list_from_values(elts);
    Ok(ast::ExprTuple {
        node_index: range.node_index(),
        elts,
        ctx: Node::ast_from_object(
            vm,
            source_file,
            get_node_field_required(vm, object, "ctx", "Tuple")?,
        )?,
        range: range.range,
        parenthesized: true,
        runtime_elts,
    })
}

impl Node for ast::ExprTuple {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &AstSourceContext) -> PyObjectRef {
        let Self {
            node_index,
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
        let elts = runtime_elts.map_or_else(
            || elts.ast_to_object(vm, source_file),
            |values| values.ast_to_object(vm, source_file),
        );
        dict.set_item("elts", elts, vm).unwrap();
        dict.set_item("ctx", ctx.ast_to_object(vm, source_file), vm)
            .unwrap();
        node_add_location_for_node(&dict, _range, vm, source_file, node_index.load());
        node.into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &AstSourceContext,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, &object, "Tuple")?;
        expr_tuple_from_object_with_range(vm, source_file, &object, range)
    }
}

// constructor
fn expr_slice_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &AstSourceContext,
    object: &PyObject,
    range: AstNodeRange,
) -> PyResult<ast::ExprSlice> {
    Ok(ast::ExprSlice {
        node_index: range.node_index(),
        lower: get_node_field_opt(vm, object, "lower")?
            .map(|obj| Node::ast_from_object(vm, source_file, obj))
            .transpose()?,
        upper: get_node_field_opt(vm, object, "upper")?
            .map(|obj| Node::ast_from_object(vm, source_file, obj))
            .transpose()?,
        step: get_node_field_opt(vm, object, "step")?
            .map(|obj| Node::ast_from_object(vm, source_file, obj))
            .transpose()?,
        range: range.range,
    })
}

impl Node for ast::ExprSlice {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &AstSourceContext) -> PyObjectRef {
        let Self {
            node_index,
            lower,
            upper,
            step,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprSlice::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("lower", lower.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item("upper", upper.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item("step", step.ast_to_object(vm, source_file), vm)
            .unwrap();
        node_add_location_for_node(&dict, _range, vm, source_file, node_index.load());
        node.into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &AstSourceContext,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, &object, "Slice")?;
        expr_slice_from_object_with_range(vm, source_file, &object, range)
    }
}

// sum
impl Node for ast::ExprContext {
    fn ast_to_object(self, vm: &VirtualMachine, _source_file: &AstSourceContext) -> PyObjectRef {
        let node_type = match self {
            Self::Load => pyast::NodeExprContextLoad::static_type(),
            Self::Store => pyast::NodeExprContextStore::static_type(),
            Self::Del => pyast::NodeExprContextDel::static_type(),
            Self::Invalid => {
                unreachable!()
            }
        };
        singleton_node_to_object(vm, node_type)
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        _source_file: &AstSourceContext,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(
            if is_node_instance(vm, &object, pyast::NodeExprContextLoad::static_type())? {
                Self::Load
            } else if is_node_instance(vm, &object, pyast::NodeExprContextStore::static_type())? {
                Self::Store
            } else if is_node_instance(vm, &object, pyast::NodeExprContextDel::static_type())? {
                Self::Del
            } else {
                return Err(vm.new_type_error(format!(
                    "expected some sort of expr_context, but got {}",
                    object.repr(vm)?
                )));
            },
        )
    }
}

// product
impl Node for ast::Comprehension {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &AstSourceContext) -> PyObjectRef {
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
            .into_ref_with_type(vm, pyast::NodeComprehension::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("target", target.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item("iter", iter.ast_to_object(vm, source_file), vm)
            .unwrap();
        let ifs = runtime_ifs.map_or_else(
            || ifs.ast_to_object(vm, source_file),
            |values| values.ast_to_object(vm, source_file),
        );
        dict.set_item("ifs", ifs, vm).unwrap();
        let is_async = runtime_is_async.map_or_else(
            || is_async.ast_to_object(vm, source_file),
            |value| vm.ctx.new_int(value).into(),
        );
        dict.set_item("is_async", is_async, vm).unwrap();
        node.into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &AstSourceContext,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let ifs: Vec<Option<ast::Expr>> =
            get_node_list_field(vm, source_file, &object, "ifs", "comprehension")?;
        let is_async_obj = get_node_field(vm, &object, "is_async", "comprehension")?;
        let is_async = node_object_to_i32(vm, &is_async_obj)?;
        let runtime_ifs = runtime_expr_list_metadata(&ifs);
        let runtime_is_async = (is_async != 0 && is_async != 1).then_some(is_async);
        Ok(Self {
            node_index: Default::default(),
            target: get_required_node_field(vm, source_file, &object, "target", "comprehension")?,
            iter: get_required_node_field(vm, source_file, &object, "iter", "comprehension")?,
            ifs: lower_runtime_expr_list(ifs),
            is_async: is_async != 0,
            range: Default::default(),
            runtime_ifs,
            runtime_is_async,
        })
    }
}
