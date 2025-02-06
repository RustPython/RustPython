use super::*;
use crate::stdlib::ast::argument::{merge_function_call_arguments, split_function_call_arguments};
use crate::stdlib::ast::constant::Constant;
use crate::stdlib::ast::string::JoinedStr;

// sum
impl Node for ruff::Expr {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        match self {
            ruff::Expr::BoolOp(cons) => cons.ast_to_object(vm),
            ruff::Expr::Name(cons) => cons.ast_to_object(vm),
            ruff::Expr::BinOp(cons) => cons.ast_to_object(vm),
            ruff::Expr::UnaryOp(cons) => cons.ast_to_object(vm),
            ruff::Expr::Lambda(cons) => cons.ast_to_object(vm),
            ruff::Expr::If(cons) => cons.ast_to_object(vm),
            ruff::Expr::Dict(cons) => cons.ast_to_object(vm),
            ruff::Expr::Set(cons) => cons.ast_to_object(vm),
            ruff::Expr::ListComp(cons) => cons.ast_to_object(vm),
            ruff::Expr::SetComp(cons) => cons.ast_to_object(vm),
            ruff::Expr::DictComp(cons) => cons.ast_to_object(vm),
            ruff::Expr::Generator(cons) => cons.ast_to_object(vm),
            ruff::Expr::Await(cons) => cons.ast_to_object(vm),
            ruff::Expr::Yield(cons) => cons.ast_to_object(vm),
            ruff::Expr::YieldFrom(cons) => cons.ast_to_object(vm),
            ruff::Expr::Compare(cons) => cons.ast_to_object(vm),
            ruff::Expr::Call(cons) => cons.ast_to_object(vm),
            ruff::Expr::Attribute(cons) => cons.ast_to_object(vm),
            ruff::Expr::Subscript(cons) => cons.ast_to_object(vm),
            ruff::Expr::Starred(cons) => cons.ast_to_object(vm),
            ruff::Expr::List(cons) => cons.ast_to_object(vm),
            ruff::Expr::Tuple(cons) => cons.ast_to_object(vm),
            ruff::Expr::Slice(cons) => cons.ast_to_object(vm),
            ruff::Expr::NumberLiteral(cons) => cons.ast_to_object(vm),
            ruff::Expr::StringLiteral(cons) => cons.ast_to_object(vm),
            ruff::Expr::FString(cons) => cons.ast_to_object(vm),
            ruff::Expr::BytesLiteral(cons) => cons.ast_to_object(vm),
            ruff::Expr::BooleanLiteral(cons) => cons.ast_to_object(vm),
            ruff::Expr::NoneLiteral(cons) => cons.ast_to_object(vm),
            ruff::Expr::EllipsisLiteral(cons) => cons.ast_to_object(vm),
            ruff::Expr::Named(cons) => cons.ast_to_object(vm),
            ruff::Expr::IpyEscapeCommand(_) => {
                unimplemented!("IPython escape command is not allowed in Python AST")
            }
        }
    }
    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        let cls = object.class();
        Ok(if cls.is(gen::NodeExprBoolOp::static_type()) {
            ruff::Expr::BoolOp(ruff::ExprBoolOp::ast_from_object(vm, object)?)
        } else if cls.is(gen::NodeExprNamedExpr::static_type()) {
            ruff::Expr::Named(ruff::ExprNamed::ast_from_object(vm, object)?)
        } else if cls.is(gen::NodeExprBinOp::static_type()) {
            ruff::Expr::BinOp(ruff::ExprBinOp::ast_from_object(vm, object)?)
        } else if cls.is(gen::NodeExprUnaryOp::static_type()) {
            ruff::Expr::UnaryOp(ruff::ExprUnaryOp::ast_from_object(vm, object)?)
        } else if cls.is(gen::NodeExprLambda::static_type()) {
            ruff::Expr::Lambda(ruff::ExprLambda::ast_from_object(vm, object)?)
        } else if cls.is(gen::NodeExprIfExp::static_type()) {
            ruff::Expr::If(ruff::ExprIf::ast_from_object(vm, object)?)
        } else if cls.is(gen::NodeExprDict::static_type()) {
            ruff::Expr::Dict(ruff::ExprDict::ast_from_object(vm, object)?)
        } else if cls.is(gen::NodeExprSet::static_type()) {
            ruff::Expr::Set(ruff::ExprSet::ast_from_object(vm, object)?)
        } else if cls.is(gen::NodeExprListComp::static_type()) {
            ruff::Expr::ListComp(ruff::ExprListComp::ast_from_object(vm, object)?)
        } else if cls.is(gen::NodeExprSetComp::static_type()) {
            ruff::Expr::SetComp(ruff::ExprSetComp::ast_from_object(vm, object)?)
        } else if cls.is(gen::NodeExprDictComp::static_type()) {
            ruff::Expr::DictComp(ruff::ExprDictComp::ast_from_object(vm, object)?)
        } else if cls.is(gen::NodeExprGeneratorExp::static_type()) {
            ruff::Expr::Generator(ruff::ExprGenerator::ast_from_object(vm, object)?)
        } else if cls.is(gen::NodeExprAwait::static_type()) {
            ruff::Expr::Await(ruff::ExprAwait::ast_from_object(vm, object)?)
        } else if cls.is(gen::NodeExprYield::static_type()) {
            ruff::Expr::Yield(ruff::ExprYield::ast_from_object(vm, object)?)
        } else if cls.is(gen::NodeExprYieldFrom::static_type()) {
            ruff::Expr::YieldFrom(ruff::ExprYieldFrom::ast_from_object(vm, object)?)
        } else if cls.is(gen::NodeExprCompare::static_type()) {
            ruff::Expr::Compare(ruff::ExprCompare::ast_from_object(vm, object)?)
        } else if cls.is(gen::NodeExprCall::static_type()) {
            ruff::Expr::Call(ruff::ExprCall::ast_from_object(vm, object)?)
        } else if cls.is(gen::NodeExprAttribute::static_type()) {
            ruff::Expr::Attribute(ruff::ExprAttribute::ast_from_object(vm, object)?)
        } else if cls.is(gen::NodeExprSubscript::static_type()) {
            ruff::Expr::Subscript(ruff::ExprSubscript::ast_from_object(vm, object)?)
        } else if cls.is(gen::NodeExprStarred::static_type()) {
            ruff::Expr::Starred(ruff::ExprStarred::ast_from_object(vm, object)?)
        } else if cls.is(gen::NodeExprName::static_type()) {
            ruff::Expr::Name(ruff::ExprName::ast_from_object(vm, object)?)
        } else if cls.is(gen::NodeExprList::static_type()) {
            ruff::Expr::List(ruff::ExprList::ast_from_object(vm, object)?)
        } else if cls.is(gen::NodeExprTuple::static_type()) {
            ruff::Expr::Tuple(ruff::ExprTuple::ast_from_object(vm, object)?)
        } else if cls.is(gen::NodeExprSlice::static_type()) {
            ruff::Expr::Slice(ruff::ExprSlice::ast_from_object(vm, object)?)
        } else if cls.is(gen::NodeExprConstant::static_type()) {
            Constant::ast_from_object(vm, object)?.into_expr()
        } else if cls.is(gen::NodeExprJoinedStr::static_type()) {
            JoinedStr::ast_from_object(vm, object)?.into_expr()
        } else {
            return Err(vm.new_type_error(format!(
                "expected some sort of expr, but got {}",
                object.repr(vm)?
            )));
        })
    }
}
// constructor
impl Node for ruff::ExprBoolOp {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let Self { op, values, range } = self;
        let node = NodeAst
            .into_ref_with_type(vm, gen::NodeExprBoolOp::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("op", op.ast_to_object(vm), vm).unwrap();
        dict.set_item("values", values.ast_to_object(vm), vm)
            .unwrap();
        node_add_location(&dict, range, vm);
        node.into()
    }
    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            op: Node::ast_from_object(vm, get_node_field(vm, &object, "op", "BoolOp")?)?,
            values: Node::ast_from_object(vm, get_node_field(vm, &object, "values", "BoolOp")?)?,
            range: range_from_object(vm, object, "BoolOp")?,
        })
    }
}
// constructor
impl Node for ruff::ExprNamed {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            target,
            value,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, gen::NodeExprNamedExpr::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("target", target.ast_to_object(vm), vm)
            .unwrap();
        dict.set_item("value", value.ast_to_object(vm), vm).unwrap();
        node_add_location(&dict, range, vm);
        node.into()
    }
    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            target: Node::ast_from_object(vm, get_node_field(vm, &object, "target", "NamedExpr")?)?,
            value: Node::ast_from_object(vm, get_node_field(vm, &object, "value", "NamedExpr")?)?,
            range: range_from_object(vm, object, "NamedExpr")?,
        })
    }
}
// constructor
impl Node for ruff::ExprBinOp {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            left,
            op,
            right,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, gen::NodeExprBinOp::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("left", left.ast_to_object(vm), vm).unwrap();
        dict.set_item("op", op.ast_to_object(vm), vm).unwrap();
        dict.set_item("right", right.ast_to_object(vm), vm).unwrap();
        node_add_location(&dict, range, vm);
        node.into()
    }
    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            left: Node::ast_from_object(vm, get_node_field(vm, &object, "left", "BinOp")?)?,
            op: Node::ast_from_object(vm, get_node_field(vm, &object, "op", "BinOp")?)?,
            right: Node::ast_from_object(vm, get_node_field(vm, &object, "right", "BinOp")?)?,
            range: range_from_object(vm, object, "BinOp")?,
        })
    }
}
// constructor
impl Node for ruff::ExprUnaryOp {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let Self { op, operand, range } = self;
        let node = NodeAst
            .into_ref_with_type(vm, gen::NodeExprUnaryOp::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("op", op.ast_to_object(vm), vm).unwrap();
        dict.set_item("operand", operand.ast_to_object(vm), vm)
            .unwrap();
        node_add_location(&dict, range, vm);
        node.into()
    }
    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            op: Node::ast_from_object(vm, get_node_field(vm, &object, "op", "UnaryOp")?)?,
            operand: Node::ast_from_object(vm, get_node_field(vm, &object, "operand", "UnaryOp")?)?,
            range: range_from_object(vm, object, "UnaryOp")?,
        })
    }
}
// constructor
impl Node for ruff::ExprLambda {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            parameters,
            body,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, gen::NodeExprLambda::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("args", parameters.ast_to_object(vm), vm)
            .unwrap();
        dict.set_item("body", body.ast_to_object(vm), vm).unwrap();
        node_add_location(&dict, _range, vm);
        node.into()
    }
    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            parameters: Node::ast_from_object(vm, get_node_field(vm, &object, "args", "Lambda")?)?,
            body: Node::ast_from_object(vm, get_node_field(vm, &object, "body", "Lambda")?)?,
            range: range_from_object(vm, object, "Lambda")?,
        })
    }
}
// constructor
impl Node for ruff::ExprIf {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            test,
            body,
            orelse,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, gen::NodeExprIfExp::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("test", test.ast_to_object(vm), vm).unwrap();
        dict.set_item("body", body.ast_to_object(vm), vm).unwrap();
        dict.set_item("orelse", orelse.ast_to_object(vm), vm)
            .unwrap();
        node_add_location(&dict, range, vm);
        node.into()
    }
    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            test: Node::ast_from_object(vm, get_node_field(vm, &object, "test", "IfExp")?)?,
            body: Node::ast_from_object(vm, get_node_field(vm, &object, "body", "IfExp")?)?,
            orelse: Node::ast_from_object(vm, get_node_field(vm, &object, "orelse", "IfExp")?)?,
            range: range_from_object(vm, object, "IfExp")?,
        })
    }
}
// constructor
impl Node for ruff::ExprDict {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let Self { items, range } = self;
        let (keys, values) =
            items
                .into_iter()
                .fold((vec![], vec![]), |(mut keys, mut values), item| {
                    keys.push(item.key);
                    values.push(item.value);
                    (keys, values)
                });
        let node = NodeAst
            .into_ref_with_type(vm, gen::NodeExprDict::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("keys", keys.ast_to_object(vm), vm).unwrap();
        dict.set_item("values", values.ast_to_object(vm), vm)
            .unwrap();
        node_add_location(&dict, range, vm);
        node.into()
    }
    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        let keys: Vec<Option<ruff::Expr>> =
            Node::ast_from_object(vm, get_node_field(vm, &object, "keys", "Dict")?)?;
        let values: Vec<_> =
            Node::ast_from_object(vm, get_node_field(vm, &object, "values", "Dict")?)?;
        let items = keys
            .into_iter()
            .zip(values)
            .map(|(key, value)| ruff::DictItem { key, value })
            .collect();
        Ok(Self {
            items,
            range: range_from_object(vm, object, "Dict")?,
        })
    }
}
// constructor
impl Node for ruff::ExprSet {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let Self { elts, range } = self;
        let node = NodeAst
            .into_ref_with_type(vm, gen::NodeExprSet::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("elts", elts.ast_to_object(vm), vm).unwrap();
        node_add_location(&dict, range, vm);
        node.into()
    }
    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            elts: Node::ast_from_object(vm, get_node_field(vm, &object, "elts", "Set")?)?,
            range: range_from_object(vm, object, "Set")?,
        })
    }
}
// constructor
impl Node for ruff::ExprListComp {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            elt,
            generators,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, gen::NodeExprListComp::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("elt", elt.ast_to_object(vm), vm).unwrap();
        dict.set_item("generators", generators.ast_to_object(vm), vm)
            .unwrap();
        node_add_location(&dict, range, vm);
        node.into()
    }
    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            elt: Node::ast_from_object(vm, get_node_field(vm, &object, "elt", "ListComp")?)?,
            generators: Node::ast_from_object(
                vm,
                get_node_field(vm, &object, "generators", "ListComp")?,
            )?,
            range: range_from_object(vm, object, "ListComp")?,
        })
    }
}
// constructor
impl Node for ruff::ExprSetComp {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            elt,
            generators,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, gen::NodeExprSetComp::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("elt", elt.ast_to_object(vm), vm).unwrap();
        dict.set_item("generators", generators.ast_to_object(vm), vm)
            .unwrap();
        node_add_location(&dict, range, vm);
        node.into()
    }
    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            elt: Node::ast_from_object(vm, get_node_field(vm, &object, "elt", "SetComp")?)?,
            generators: Node::ast_from_object(
                vm,
                get_node_field(vm, &object, "generators", "SetComp")?,
            )?,
            range: range_from_object(vm, object, "SetComp")?,
        })
    }
}
// constructor
impl Node for ruff::ExprDictComp {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            key,
            value,
            generators,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, gen::NodeExprDictComp::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("key", key.ast_to_object(vm), vm).unwrap();
        dict.set_item("value", value.ast_to_object(vm), vm).unwrap();
        dict.set_item("generators", generators.ast_to_object(vm), vm)
            .unwrap();
        node_add_location(&dict, range, vm);
        node.into()
    }
    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            key: Node::ast_from_object(vm, get_node_field(vm, &object, "key", "DictComp")?)?,
            value: Node::ast_from_object(vm, get_node_field(vm, &object, "value", "DictComp")?)?,
            generators: Node::ast_from_object(
                vm,
                get_node_field(vm, &object, "generators", "DictComp")?,
            )?,
            range: range_from_object(vm, object, "DictComp")?,
        })
    }
}
// constructor
impl Node for ruff::ExprGenerator {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            elt,
            generators,
            range,
            parenthesized: _,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, gen::NodeExprGeneratorExp::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("elt", elt.ast_to_object(vm), vm).unwrap();
        dict.set_item("generators", generators.ast_to_object(vm), vm)
            .unwrap();
        node_add_location(&dict, range, vm);
        node.into()
    }
    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            elt: Node::ast_from_object(vm, get_node_field(vm, &object, "elt", "GeneratorExp")?)?,
            generators: Node::ast_from_object(
                vm,
                get_node_field(vm, &object, "generators", "GeneratorExp")?,
            )?,
            range: range_from_object(vm, object, "GeneratorExp")?,
            // TODO: Is this correct?
            parenthesized: true,
        })
    }
}
// constructor
impl Node for ruff::ExprAwait {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let Self { value, range } = self;
        let node = NodeAst
            .into_ref_with_type(vm, gen::NodeExprAwait::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(vm), vm).unwrap();
        node_add_location(&dict, range, vm);
        node.into()
    }
    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            value: Node::ast_from_object(vm, get_node_field(vm, &object, "value", "Await")?)?,
            range: range_from_object(vm, object, "Await")?,
        })
    }
}
// constructor
impl Node for ruff::ExprYield {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let ruff::ExprYield { value, range } = self;
        let node = NodeAst
            .into_ref_with_type(vm, gen::NodeExprYield::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(vm), vm).unwrap();
        node_add_location(&dict, range, vm);
        node.into()
    }
    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        Ok(ruff::ExprYield {
            value: get_node_field_opt(vm, &object, "value")?
                .map(|obj| Node::ast_from_object(vm, obj))
                .transpose()?,
            range: range_from_object(vm, object, "Yield")?,
        })
    }
}
// constructor
impl Node for ruff::ExprYieldFrom {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let Self { value, range } = self;
        let node = NodeAst
            .into_ref_with_type(vm, gen::NodeExprYieldFrom::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(vm), vm).unwrap();
        node_add_location(&dict, range, vm);
        node.into()
    }
    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            value: Node::ast_from_object(vm, get_node_field(vm, &object, "value", "YieldFrom")?)?,
            range: range_from_object(vm, object, "YieldFrom")?,
        })
    }
}
// constructor
impl Node for ruff::ExprCompare {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            left,
            ops,
            comparators,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, gen::NodeExprCompare::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("left", left.ast_to_object(vm), vm).unwrap();
        dict.set_item("ops", BoxedSlice(ops).ast_to_object(vm), vm)
            .unwrap();
        dict.set_item("comparators", BoxedSlice(comparators).ast_to_object(vm), vm)
            .unwrap();
        node_add_location(&dict, range, vm);
        node.into()
    }
    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            left: Node::ast_from_object(vm, get_node_field(vm, &object, "left", "Compare")?)?,
            ops: {
                let ops: BoxedSlice<_> =
                    Node::ast_from_object(vm, get_node_field(vm, &object, "ops", "Compare")?)?;
                ops.0
            },
            comparators: {
                let comparators: BoxedSlice<_> = Node::ast_from_object(
                    vm,
                    get_node_field(vm, &object, "comparators", "Compare")?,
                )?;
                comparators.0
            },
            range: range_from_object(vm, object, "Compare")?,
        })
    }
}
// constructor
impl Node for ruff::ExprCall {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            func,
            arguments,
            range,
        } = self;
        let (positional_arguments, keyword_arguments) = split_function_call_arguments(arguments);
        let node = NodeAst
            .into_ref_with_type(vm, gen::NodeExprCall::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("func", func.ast_to_object(vm), vm).unwrap();
        dict.set_item("args", positional_arguments.ast_to_object(vm), vm)
            .unwrap();
        dict.set_item("keywords", keyword_arguments.ast_to_object(vm), vm)
            .unwrap();
        node_add_location(&dict, range, vm);
        node.into()
    }
    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            func: Node::ast_from_object(vm, get_node_field(vm, &object, "func", "Call")?)?,
            arguments: merge_function_call_arguments(
                Node::ast_from_object(vm, get_node_field(vm, &object, "args", "Call")?)?,
                Node::ast_from_object(vm, get_node_field(vm, &object, "keywords", "Call")?)?,
            ),
            range: range_from_object(vm, object, "Call")?,
        })
    }
}

// constructor
impl Node for ruff::ExprAttribute {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            value,
            attr,
            ctx,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, gen::NodeExprAttribute::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(vm), vm).unwrap();
        dict.set_item("attr", attr.ast_to_object(vm), vm).unwrap();
        dict.set_item("ctx", ctx.ast_to_object(vm), vm).unwrap();
        node_add_location(&dict, range, vm);
        node.into()
    }
    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            value: Node::ast_from_object(vm, get_node_field(vm, &object, "value", "Attribute")?)?,
            attr: Node::ast_from_object(vm, get_node_field(vm, &object, "attr", "Attribute")?)?,
            ctx: Node::ast_from_object(vm, get_node_field(vm, &object, "ctx", "Attribute")?)?,
            range: range_from_object(vm, object, "Attribute")?,
        })
    }
}
// constructor
impl Node for ruff::ExprSubscript {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            value,
            slice,
            ctx,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, gen::NodeExprSubscript::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(vm), vm).unwrap();
        dict.set_item("slice", slice.ast_to_object(vm), vm).unwrap();
        dict.set_item("ctx", ctx.ast_to_object(vm), vm).unwrap();
        node_add_location(&dict, _range, vm);
        node.into()
    }
    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            value: Node::ast_from_object(vm, get_node_field(vm, &object, "value", "Subscript")?)?,
            slice: Node::ast_from_object(vm, get_node_field(vm, &object, "slice", "Subscript")?)?,
            ctx: Node::ast_from_object(vm, get_node_field(vm, &object, "ctx", "Subscript")?)?,
            range: range_from_object(vm, object, "Subscript")?,
        })
    }
}
// constructor
impl Node for ruff::ExprStarred {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let Self { value, ctx, range } = self;
        let node = NodeAst
            .into_ref_with_type(vm, gen::NodeExprStarred::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(vm), vm).unwrap();
        dict.set_item("ctx", ctx.ast_to_object(vm), vm).unwrap();
        node_add_location(&dict, range, vm);
        node.into()
    }
    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            value: Node::ast_from_object(vm, get_node_field(vm, &object, "value", "Starred")?)?,
            ctx: Node::ast_from_object(vm, get_node_field(vm, &object, "ctx", "Starred")?)?,
            range: range_from_object(vm, object, "Starred")?,
        })
    }
}
// constructor
impl Node for ruff::ExprName {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let Self { id, ctx, range } = self;
        let node = NodeAst
            .into_ref_with_type(vm, gen::NodeExprName::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("id", id.to_pyobject(vm), vm).unwrap();
        dict.set_item("ctx", ctx.ast_to_object(vm), vm).unwrap();
        node_add_location(&dict, range, vm);
        node.into()
    }
    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            id: get_node_field(vm, &object, "id", "Name")?.try_into_value(vm)?,
            ctx: Node::ast_from_object(vm, get_node_field(vm, &object, "ctx", "Name")?)?,
            range: range_from_object(vm, object, "Name")?,
        })
    }
}
// constructor
impl Node for ruff::ExprList {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let ruff::ExprList { elts, ctx, range } = self;
        let node = NodeAst
            .into_ref_with_type(vm, gen::NodeExprList::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("elts", elts.ast_to_object(vm), vm).unwrap();
        dict.set_item("ctx", ctx.ast_to_object(vm), vm).unwrap();
        node_add_location(&dict, range, vm);
        node.into()
    }
    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        Ok(ruff::ExprList {
            elts: Node::ast_from_object(vm, get_node_field(vm, &object, "elts", "List")?)?,
            ctx: Node::ast_from_object(vm, get_node_field(vm, &object, "ctx", "List")?)?,
            range: range_from_object(vm, object, "List")?,
        })
    }
}
// constructor
impl Node for ruff::ExprTuple {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            elts,
            ctx,
            range: _range,
            parenthesized: _,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, gen::NodeExprTuple::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("elts", elts.ast_to_object(vm), vm).unwrap();
        dict.set_item("ctx", ctx.ast_to_object(vm), vm).unwrap();
        node_add_location(&dict, _range, vm);
        node.into()
    }
    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            elts: Node::ast_from_object(vm, get_node_field(vm, &object, "elts", "Tuple")?)?,
            ctx: Node::ast_from_object(vm, get_node_field(vm, &object, "ctx", "Tuple")?)?,
            range: range_from_object(vm, object, "Tuple")?,
            parenthesized: true, // TODO: is this correct?
        })
    }
}
// constructor
impl Node for ruff::ExprSlice {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            lower,
            upper,
            step,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, gen::NodeExprSlice::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("lower", lower.ast_to_object(vm), vm).unwrap();
        dict.set_item("upper", upper.ast_to_object(vm), vm).unwrap();
        dict.set_item("step", step.ast_to_object(vm), vm).unwrap();
        node_add_location(&dict, _range, vm);
        node.into()
    }
    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            lower: get_node_field_opt(vm, &object, "lower")?
                .map(|obj| Node::ast_from_object(vm, obj))
                .transpose()?,
            upper: get_node_field_opt(vm, &object, "upper")?
                .map(|obj| Node::ast_from_object(vm, obj))
                .transpose()?,
            step: get_node_field_opt(vm, &object, "step")?
                .map(|obj| Node::ast_from_object(vm, obj))
                .transpose()?,
            range: range_from_object(vm, object, "Slice")?,
        })
    }
}
// sum
impl Node for ruff::ExprContext {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let node_type = match self {
            ruff::ExprContext::Load => gen::NodeExprContextLoad::static_type(),
            ruff::ExprContext::Store => gen::NodeExprContextStore::static_type(),
            ruff::ExprContext::Del => gen::NodeExprContextDel::static_type(),
            ruff::ExprContext::Invalid => todo!(),
        };
        NodeAst
            .into_ref_with_type(vm, node_type.to_owned())
            .unwrap()
            .into()
    }
    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        let _cls = object.class();
        Ok(if _cls.is(gen::NodeExprContextLoad::static_type()) {
            ruff::ExprContext::Load
        } else if _cls.is(gen::NodeExprContextStore::static_type()) {
            ruff::ExprContext::Store
        } else if _cls.is(gen::NodeExprContextDel::static_type()) {
            ruff::ExprContext::Del
        } else {
            return Err(vm.new_type_error(format!(
                "expected some sort of expr_context, but got {}",
                object.repr(vm)?
            )));
        })
    }
}

// product
impl Node for ruff::Comprehension {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            target,
            iter,
            ifs,
            is_async,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, gen::NodeComprehension::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("target", target.ast_to_object(vm), vm)
            .unwrap();
        dict.set_item("iter", iter.ast_to_object(vm), vm).unwrap();
        dict.set_item("ifs", ifs.ast_to_object(vm), vm).unwrap();
        dict.set_item("is_async", is_async.ast_to_object(vm), vm)
            .unwrap();
        node.into()
    }
    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            target: Node::ast_from_object(
                vm,
                get_node_field(vm, &object, "target", "comprehension")?,
            )?,
            iter: Node::ast_from_object(vm, get_node_field(vm, &object, "iter", "comprehension")?)?,
            ifs: Node::ast_from_object(vm, get_node_field(vm, &object, "ifs", "comprehension")?)?,
            is_async: Node::ast_from_object(
                vm,
                get_node_field(vm, &object, "is_async", "comprehension")?,
            )?,
            range: Default::default(),
        })
    }
}

impl Node for ruff::ExprBytesLiteral {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let Self { range, value } = self;
        let bytes = value.as_slice().iter().flat_map(|b| b.value.iter());
        let c = Constant::new_bytes(bytes.copied(), range);
        c.ast_to_object(vm)
    }

    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        todo!()
    }
}

impl Node for ruff::ExprBooleanLiteral {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let Self { range, value } = self;
        let c = Constant::new_bool(value, range);
        c.ast_to_object(vm)
    }

    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        todo!()
    }
}

impl Node for ruff::ExprNoneLiteral {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let Self { range } = self;
        let c = Constant::new_none(range);
        c.ast_to_object(vm)
    }

    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        todo!()
    }
}

impl Node for ruff::ExprEllipsisLiteral {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let Self { range } = self;
        let c = Constant::new_ellipsis(range);
        c.ast_to_object(vm)
    }

    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        todo!()
    }
}

impl Node for ruff::ExprNumberLiteral {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let Self { range, value } = self;
        let c = match value {
            ruff::Number::Int(n) => Constant::new_int(n, range),
            ruff::Number::Float(n) => Constant::new_float(n, range),
            ruff::Number::Complex { real, imag } => Constant::new_complex(real, imag, range),
        };
        c.ast_to_object(vm)
    }

    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        todo!()
    }
}
