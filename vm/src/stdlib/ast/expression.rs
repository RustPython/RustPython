use super::*;
use crate::builtins::{PyComplex, PyTuple};
use crate::stdlib::ast::argument::{merge_function_call_arguments, split_function_call_arguments};

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
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        let _cls = _object.class();
        Ok(if _cls.is(gen::NodeExprBoolOp::static_type()) {
            ruff::Expr::BoolOp(ruff::ExprBoolOp::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeExprNamedExpr::static_type()) {
            ruff::Expr::Named(ruff::ExprNamed::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeExprBinOp::static_type()) {
            ruff::Expr::BinOp(ruff::ExprBinOp::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeExprUnaryOp::static_type()) {
            ruff::Expr::UnaryOp(ruff::ExprUnaryOp::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeExprLambda::static_type()) {
            ruff::Expr::Lambda(ruff::ExprLambda::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeExprIfExp::static_type()) {
            ruff::Expr::If(ruff::ExprIf::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeExprDict::static_type()) {
            ruff::Expr::Dict(ruff::ExprDict::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeExprSet::static_type()) {
            ruff::Expr::Set(ruff::ExprSet::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeExprListComp::static_type()) {
            ruff::Expr::ListComp(ruff::ExprListComp::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeExprSetComp::static_type()) {
            ruff::Expr::SetComp(ruff::ExprSetComp::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeExprDictComp::static_type()) {
            ruff::Expr::DictComp(ruff::ExprDictComp::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeExprGeneratorExp::static_type()) {
            ruff::Expr::Generator(ruff::ExprGenerator::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeExprAwait::static_type()) {
            ruff::Expr::Await(ruff::ExprAwait::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeExprYield::static_type()) {
            ruff::Expr::Yield(ruff::ExprYield::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeExprYieldFrom::static_type()) {
            ruff::Expr::YieldFrom(ruff::ExprYieldFrom::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeExprCompare::static_type()) {
            ruff::Expr::Compare(ruff::ExprCompare::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeExprCall::static_type()) {
            ruff::Expr::Call(ruff::ExprCall::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeExprAttribute::static_type()) {
            ruff::Expr::Attribute(ruff::ExprAttribute::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeExprSubscript::static_type()) {
            ruff::Expr::Subscript(ruff::ExprSubscript::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeExprStarred::static_type()) {
            ruff::Expr::Starred(ruff::ExprStarred::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeExprName::static_type()) {
            ruff::Expr::Name(ruff::ExprName::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeExprList::static_type()) {
            ruff::Expr::List(ruff::ExprList::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeExprTuple::static_type()) {
            ruff::Expr::Tuple(ruff::ExprTuple::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeExprSlice::static_type()) {
            ruff::Expr::Slice(ruff::ExprSlice::ast_from_object(_vm, _object)?)
        } else {
            return Err(_vm.new_type_error(format!(
                "expected some sort of expr, but got {}",
                _object.repr(_vm)?
            )));
        })
    }
}
// constructor
impl Node for ruff::ExprBoolOp {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let ruff::ExprBoolOp {
            op,
            values,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeExprBoolOp::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("op", op.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("values", values.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(ruff::ExprBoolOp {
            op: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "op", "BoolOp")?)?,
            values: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "values", "BoolOp")?)?,
            range: range_from_object(_vm, _object, "BoolOp")?,
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
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            left,
            op,
            right,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeExprBinOp::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("left", left.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("op", op.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("right", right.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            left: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "left", "BinOp")?)?,
            op: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "op", "BinOp")?)?,
            right: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "right", "BinOp")?)?,
            range: range_from_object(_vm, _object, "BinOp")?,
        })
    }
}
// constructor
impl Node for ruff::ExprUnaryOp {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            op,
            operand,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeExprUnaryOp::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("op", op.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("operand", operand.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            op: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "op", "UnaryOp")?)?,
            operand: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "operand", "UnaryOp")?,
            )?,
            range: range_from_object(_vm, _object, "UnaryOp")?,
        })
    }
}
// constructor
impl Node for ruff::ExprLambda {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            parameters,
            body,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeExprLambda::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("args", parameters.ast_to_object(_vm), _vm)
            .unwrap();
        dict.set_item("body", body.ast_to_object(_vm), _vm).unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            parameters: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "args", "Lambda")?,
            )?,
            body: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "body", "Lambda")?)?,
            range: range_from_object(_vm, _object, "Lambda")?,
        })
    }
}
// constructor
impl Node for ruff::ExprIf {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            test,
            body,
            orelse,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeExprIfExp::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("test", test.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("body", body.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("orelse", orelse.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            test: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "test", "IfExp")?)?,
            body: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "body", "IfExp")?)?,
            orelse: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "orelse", "IfExp")?)?,
            range: range_from_object(_vm, _object, "IfExp")?,
        })
    }
}
// constructor
impl Node for ruff::ExprDict {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            items,
            range: _range,
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
            .into_ref_with_type(_vm, gen::NodeExprDict::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("keys", keys.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("values", values.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        let keys: Vec<Option<ruff::Expr>> =
            Node::ast_from_object(_vm, get_node_field(_vm, &_object, "keys", "Dict")?)?;
        let values: Vec<_> =
            Node::ast_from_object(_vm, get_node_field(_vm, &_object, "values", "Dict")?)?;
        let items = keys
            .into_iter()
            .zip(values.into_iter())
            .map(|(key, value)| ruff::DictItem { key, value })
            .collect();
        Ok(Self {
            items,
            range: range_from_object(_vm, _object, "Dict")?,
        })
    }
}
// constructor
impl Node for ruff::ExprSet {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let ruff::ExprSet {
            elts,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeExprSet::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("elts", elts.ast_to_object(_vm), _vm).unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(ruff::ExprSet {
            elts: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "elts", "Set")?)?,
            range: range_from_object(_vm, _object, "Set")?,
        })
    }
}
// constructor
impl Node for ruff::ExprListComp {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let ruff::ExprListComp {
            elt,
            generators,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeExprListComp::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("elt", elt.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("generators", generators.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(ruff::ExprListComp {
            elt: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "elt", "ListComp")?)?,
            generators: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "generators", "ListComp")?,
            )?,
            range: range_from_object(_vm, _object, "ListComp")?,
        })
    }
}
// constructor
impl Node for ruff::ExprSetComp {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let ruff::ExprSetComp {
            elt,
            generators,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeExprSetComp::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("elt", elt.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("generators", generators.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(ruff::ExprSetComp {
            elt: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "elt", "SetComp")?)?,
            generators: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "generators", "SetComp")?,
            )?,
            range: range_from_object(_vm, _object, "SetComp")?,
        })
    }
}
// constructor
impl Node for ruff::ExprDictComp {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let ruff::ExprDictComp {
            key,
            value,
            generators,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeExprDictComp::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("key", key.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("value", value.ast_to_object(_vm), _vm)
            .unwrap();
        dict.set_item("generators", generators.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(ruff::ExprDictComp {
            key: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "key", "DictComp")?)?,
            value: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "value", "DictComp")?)?,
            generators: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "generators", "DictComp")?,
            )?,
            range: range_from_object(_vm, _object, "DictComp")?,
        })
    }
}
// constructor
impl Node for ruff::ExprGenerator {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            elt,
            generators,
            range: _range,
            parenthesized: _,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeExprGeneratorExp::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("elt", elt.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("generators", generators.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            elt: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "elt", "GeneratorExp")?)?,
            generators: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "generators", "GeneratorExp")?,
            )?,
            range: range_from_object(_vm, _object, "GeneratorExp")?,
            // TODO: Is this correct?
            parenthesized: true,
        })
    }
}
// constructor
impl Node for ruff::ExprAwait {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let ruff::ExprAwait {
            value,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeExprAwait::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(ruff::ExprAwait {
            value: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "value", "Await")?)?,
            range: range_from_object(_vm, _object, "Await")?,
        })
    }
}
// constructor
impl Node for ruff::ExprYield {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let ruff::ExprYield {
            value,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeExprYield::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(ruff::ExprYield {
            value: get_node_field_opt(_vm, &_object, "value")?
                .map(|obj| Node::ast_from_object(_vm, obj))
                .transpose()?,
            range: range_from_object(_vm, _object, "Yield")?,
        })
    }
}
// constructor
impl Node for ruff::ExprYieldFrom {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let ruff::ExprYieldFrom {
            value,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeExprYieldFrom::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(ruff::ExprYieldFrom {
            value: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "value", "YieldFrom")?,
            )?,
            range: range_from_object(_vm, _object, "YieldFrom")?,
        })
    }
}
// constructor
impl Node for ruff::ExprCompare {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            left,
            ops,
            comparators,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeExprCompare::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("left", left.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("ops", BoxedSlice(ops).ast_to_object(_vm), _vm)
            .unwrap();
        dict.set_item(
            "comparators",
            BoxedSlice(comparators).ast_to_object(_vm),
            _vm,
        )
        .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            left: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "left", "Compare")?)?,
            ops: {
                let ops: BoxedSlice<_> =
                    Node::ast_from_object(_vm, get_node_field(_vm, &_object, "ops", "Compare")?)?;
                ops.0
            },
            comparators: {
                let comparators: BoxedSlice<_> = Node::ast_from_object(
                    _vm,
                    get_node_field(_vm, &_object, "comparators", "Compare")?,
                )?;
                comparators.0
            },
            range: range_from_object(_vm, _object, "Compare")?,
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
        if !positional_arguments.args.is_empty() {
            dict.set_item("args", positional_arguments.ast_to_object(vm), vm)
                .unwrap();
        }
        if !keyword_arguments.keywords.is_empty() {
            dict.set_item("keywords", keyword_arguments.ast_to_object(vm), vm)
                .unwrap();
        }
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

pub(crate) struct Constant {
    range: TextRange,
    value: ConstantLiteral,
}

impl Constant {
    fn new_str(value: &str, range: TextRange) -> Self {
        Self {
            range,
            value: ConstantLiteral::Str(value.to_string()),
        }
    }

    fn new_int(value: ruff::Int, range: TextRange) -> Self {
        Self {
            range,
            value: ConstantLiteral::Int(value),
        }
    }

    fn new_float(value: f64, range: TextRange) -> Self {
        Self {
            range,
            value: ConstantLiteral::Float(value),
        }
    }
    fn new_complex(real: f64, imag: f64, range: TextRange) -> Self {
        Self {
            range,
            value: ConstantLiteral::Complex { real, imag },
        }
    }

    fn new_bytes(value: impl Iterator<Item = u8>, range: TextRange) -> Self {
        Self {
            range,
            value: ConstantLiteral::Bytes(value.collect()),
        }
    }

    fn new_bool(value: bool, range: TextRange) -> Self {
        Self {
            range,
            value: ConstantLiteral::Bool(value),
        }
    }

    fn new_none(range: TextRange) -> Self {
        Self {
            range,
            value: ConstantLiteral::None,
        }
    }

    fn new_ellipsis(range: TextRange) -> Self {
        Self {
            range,
            value: ConstantLiteral::Ellipsis,
        }
    }
}

pub(crate) enum ConstantLiteral {
    None,
    Bool(bool),
    Str(String),
    Bytes(Vec<u8>),
    Int(ruff::Int),
    Tuple(Vec<Constant>),
    Float(f64),
    Complex { real: f64, imag: f64 },
    Ellipsis,
}

// constructor
impl Node for Constant {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let Self { range, value } = self;
        let is_str = matches!(&value, ConstantLiteral::Str(_));
        let mut is_unicode = false;
        let value = match value {
            ConstantLiteral::None => vm.ctx.none(),
            ConstantLiteral::Bool(value) => vm.ctx.new_bool(value).to_pyobject(vm),
            ConstantLiteral::Str(value) => {
                if !value.is_ascii() {
                    is_unicode = true;
                }
                vm.ctx.new_str(value).to_pyobject(vm)
            }
            ConstantLiteral::Bytes(value) => vm.ctx.new_bytes(value).to_pyobject(vm),
            ConstantLiteral::Int(value) => value.ast_to_object(vm),
            ConstantLiteral::Tuple(value) => vm
                .ctx
                .new_tuple(value.into_iter().map(|c| c.ast_to_object(vm)).collect())
                .to_pyobject(vm),
            ConstantLiteral::Float(value) => vm.ctx.new_float(value).into_pyobject(vm),
            ConstantLiteral::Complex { real, imag } => vm
                .ctx
                .new_complex(num_complex::Complex::new(real, imag))
                .into_pyobject(vm),
            ConstantLiteral::Ellipsis => vm.ctx.ellipsis(),
        };
        // TODO: Figure out how this works
        let kind = vm.ctx.new_str("u").to_pyobject(vm);
        let node = NodeAst
            .into_ref_with_type(vm, gen::NodeExprConstant::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value, vm).unwrap();
        if is_str && is_unicode {
            dict.set_item("kind", kind, vm).unwrap();
        }
        node_add_location(&dict, range, vm);
        node.into()
    }

    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        let value = get_node_field(vm, &object, "value", "Constant")?;
        let _cls = object.class();
        let value = if _cls.is(vm.ctx.types.none_type) {
            ConstantLiteral::None
        } else if _cls.is(vm.ctx.types.bool_type) {
            ConstantLiteral::Bool(if value.is(&vm.ctx.true_value) {
                true
            } else if value.is(&vm.ctx.false_value) {
                false
            } else {
                value.try_to_value(vm)?
            })
        } else if _cls.is(vm.ctx.types.str_type) {
            ConstantLiteral::Str(value.try_to_value(vm)?)
        } else if _cls.is(vm.ctx.types.bytes_type) {
            ConstantLiteral::Bytes(value.try_to_value(vm)?)
        } else if _cls.is(vm.ctx.types.int_type) {
            ConstantLiteral::Int(Node::ast_from_object(vm, value)?)
        } else if _cls.is(vm.ctx.types.tuple_type) {
            let tuple = value.downcast::<PyTuple>().map_err(|obj| {
                vm.new_type_error(format!(
                    "Expected type {}, not {}",
                    PyTuple::static_type().name(),
                    obj.class().name()
                ))
            })?;
            let tuple = tuple
                .into_iter()
                .cloned()
                .map(|object| Node::ast_from_object(vm, object))
                .collect::<PyResult<_>>()?;
            ConstantLiteral::Tuple(tuple)
        } else if _cls.is(vm.ctx.types.float_type) {
            let float = value.try_into_value(vm)?;
            ConstantLiteral::Float(float)
        } else if _cls.is(vm.ctx.types.complex_type) {
            let complex = value.try_complex(vm)?;
            let complex = match complex {
                None => {
                    return Err(vm.new_type_error(format!(
                        "Expected type {}, not {}",
                        PyComplex::static_type().name(),
                        value.class().name()
                    )))
                }
                Some((value, _was_coerced)) => value,
            };
            ConstantLiteral::Complex {
                real: complex.re,
                imag: complex.im,
            }
        } else if _cls.is(vm.ctx.types.ellipsis_type) {
            ConstantLiteral::Ellipsis
        } else {
            return Err(vm.new_type_error(format!(
                "expected some sort of expr, but got {}",
                object.repr(vm)?
            )));
        };

        Ok(Self {
            value,
            // kind: get_node_field_opt(_vm, &_object, "kind")?
            //     .map(|obj| Node::ast_from_object(_vm, obj))
            //     .transpose()?,
            range: range_from_object(vm, object, "Constant")?,
        })
    }
}

// constructor
impl Node for ruff::ExprAttribute {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            value,
            attr,
            ctx,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeExprAttribute::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(_vm), _vm)
            .unwrap();
        dict.set_item("attr", attr.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("ctx", ctx.ast_to_object(_vm), _vm).unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            value: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "value", "Attribute")?,
            )?,
            attr: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "attr", "Attribute")?)?,
            ctx: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "ctx", "Attribute")?)?,
            range: range_from_object(_vm, _object, "Attribute")?,
        })
    }
}
// constructor
impl Node for ruff::ExprSubscript {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let ruff::ExprSubscript {
            value,
            slice,
            ctx,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeExprSubscript::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(_vm), _vm)
            .unwrap();
        dict.set_item("slice", slice.ast_to_object(_vm), _vm)
            .unwrap();
        dict.set_item("ctx", ctx.ast_to_object(_vm), _vm).unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(ruff::ExprSubscript {
            value: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "value", "Subscript")?,
            )?,
            slice: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "slice", "Subscript")?,
            )?,
            ctx: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "ctx", "Subscript")?)?,
            range: range_from_object(_vm, _object, "Subscript")?,
        })
    }
}
// constructor
impl Node for ruff::ExprStarred {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let ruff::ExprStarred {
            value,
            ctx,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeExprStarred::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(_vm), _vm)
            .unwrap();
        dict.set_item("ctx", ctx.ast_to_object(_vm), _vm).unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(ruff::ExprStarred {
            value: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "value", "Starred")?)?,
            ctx: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "ctx", "Starred")?)?,
            range: range_from_object(_vm, _object, "Starred")?,
        })
    }
}
// constructor
impl Node for ruff::ExprName {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let ruff::ExprName {
            id,
            ctx,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeExprName::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("id", id.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("ctx", ctx.ast_to_object(_vm), _vm).unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(ruff::ExprName {
            id: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "id", "Name")?)?,
            ctx: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "ctx", "Name")?)?,
            range: range_from_object(_vm, _object, "Name")?,
        })
    }
}
// constructor
impl Node for ruff::ExprList {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let ruff::ExprList {
            elts,
            ctx,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeExprList::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("elts", elts.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("ctx", ctx.ast_to_object(_vm), _vm).unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(ruff::ExprList {
            elts: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "elts", "List")?)?,
            ctx: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "ctx", "List")?)?,
            range: range_from_object(_vm, _object, "List")?,
        })
    }
}
// constructor
impl Node for ruff::ExprTuple {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let ruff::ExprTuple {
            elts,
            ctx,
            range: _range,
            parenthesized: _,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeExprTuple::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("elts", elts.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("ctx", ctx.ast_to_object(_vm), _vm).unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(ruff::ExprTuple {
            elts: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "elts", "Tuple")?)?,
            ctx: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "ctx", "Tuple")?)?,
            range: range_from_object(_vm, _object, "Tuple")?,
            parenthesized: true, // TODO: is this correct?
        })
    }
}
// constructor
impl Node for ruff::ExprSlice {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let ruff::ExprSlice {
            lower,
            upper,
            step,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeExprSlice::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("lower", lower.ast_to_object(_vm), _vm)
            .unwrap();
        dict.set_item("upper", upper.ast_to_object(_vm), _vm)
            .unwrap();
        dict.set_item("step", step.ast_to_object(_vm), _vm).unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(ruff::ExprSlice {
            lower: get_node_field_opt(_vm, &_object, "lower")?
                .map(|obj| Node::ast_from_object(_vm, obj))
                .transpose()?,
            upper: get_node_field_opt(_vm, &_object, "upper")?
                .map(|obj| Node::ast_from_object(_vm, obj))
                .transpose()?,
            step: get_node_field_opt(_vm, &_object, "step")?
                .map(|obj| Node::ast_from_object(_vm, obj))
                .transpose()?,
            range: range_from_object(_vm, _object, "Slice")?,
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
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        let _cls = _object.class();
        Ok(if _cls.is(gen::NodeExprContextLoad::static_type()) {
            ruff::ExprContext::Load
        } else if _cls.is(gen::NodeExprContextStore::static_type()) {
            ruff::ExprContext::Store
        } else if _cls.is(gen::NodeExprContextDel::static_type()) {
            ruff::ExprContext::Del
        } else {
            return Err(_vm.new_type_error(format!(
                "expected some sort of expr_context, but got {}",
                _object.repr(_vm)?
            )));
        })
    }
}

// product
impl Node for ruff::Comprehension {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let ruff::Comprehension {
            target,
            iter,
            ifs,
            is_async,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeComprehension::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("target", target.ast_to_object(_vm), _vm)
            .unwrap();
        dict.set_item("iter", iter.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("ifs", ifs.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("is_async", is_async.ast_to_object(_vm), _vm)
            .unwrap();
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(ruff::Comprehension {
            target: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "target", "comprehension")?,
            )?,
            iter: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "iter", "comprehension")?,
            )?,
            ifs: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "ifs", "comprehension")?,
            )?,
            is_async: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "is_async", "comprehension")?,
            )?,
            range: Default::default(),
        })
    }
}

impl Node for ruff::ExprBytesLiteral {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let Self { range, value } = self;
        let c = Constant::new_bytes(value.bytes(), range);
        c.ast_to_object(vm)
    }

    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        todo!()
    }
}

impl Node for ruff::ExprBooleanLiteral {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let Self { range, value } = self;
        let c = Constant::new_bool(value, range);
        c.ast_to_object(vm)
    }

    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        todo!()
    }
}

impl Node for ruff::ExprNoneLiteral {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let Self { range } = self;
        let c = Constant::new_none(range);
        c.ast_to_object(vm)
    }

    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        todo!()
    }
}

impl Node for ruff::ExprEllipsisLiteral {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let Self { range } = self;
        let c = Constant::new_ellipsis(range);
        c.ast_to_object(vm)
    }

    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
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

    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        todo!()
    }
}

impl Node for ruff::ExprStringLiteral {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let Self { range, value } = self;
        let c = Constant::new_str(value.to_str(), range);
        c.ast_to_object(vm)
    }

    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        todo!()
    }
}

impl Node for ruff::ExprFString {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let Self { range, value } = self;
        let values: Vec<_> = value
            .into_iter()
            .flat_map(fstring_part_to_joined_str_part)
            .collect();
        let values = values.into_boxed_slice();
        let c = JoinedStr { range, values };
        c.ast_to_object(vm)
    }

    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        todo!()
    }
}

fn fstring_part_to_joined_str_part(fstring_part: &ruff::FStringPart) -> Vec<JoinedStrPart> {
    match fstring_part {
        ruff::FStringPart::Literal(ruff::StringLiteral {
            range,
            value,
            flags: _, // TODO
        }) => vec![JoinedStrPart::Constant(Constant::new_str(value, *range))],
        ruff::FStringPart::FString(ruff::FString {
            range: _,
            elements,
            flags: _, // TODO
        }) => elements
            .into_iter()
            .map(fstring_element_to_joined_str_part)
            .collect(),
    }
}

fn fstring_element_to_joined_str_part(element: &ruff::FStringElement) -> JoinedStrPart {
    match element {
        ruff::FStringElement::Literal(ruff::FStringLiteralElement { range, value }) => {
            JoinedStrPart::Constant(Constant::new_str(value, *range))
        }
        ruff::FStringElement::Expression(ruff::FStringExpressionElement {
            range,
            expression,
            debug_text: _, // TODO: What is this?
            conversion,
            format_spec,
        }) => JoinedStrPart::FormattedValue(FormattedValue {
            value: expression.clone(),
            conversion: *conversion,
            format_spec: format_spec_helper(format_spec),
            range: *range,
        }),
    }
}

fn format_spec_helper(
    format_spec: &Option<Box<ruff::FStringFormatSpec>>,
) -> Option<Box<JoinedStr>> {
    match format_spec.as_deref() {
        None => None,
        Some(ruff::FStringFormatSpec { range, elements }) => {
            let values: Vec<_> = elements
                .into_iter()
                .map(fstring_element_to_joined_str_part)
                .collect();
            let values = values.into_boxed_slice();
            Some(Box::new(JoinedStr {
                values,
                range: *range,
            }))
        }
    }
}

struct JoinedStr {
    values: Box<[JoinedStrPart]>,
    range: TextRange,
}

// constructor
impl Node for JoinedStr {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let Self { values, range } = self;
        let node = NodeAst
            .into_ref_with_type(vm, gen::NodeExprJoinedStr::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("values", BoxedSlice(values).ast_to_object(vm), vm)
            .unwrap();
        node_add_location(&dict, range, vm);
        node.into()
    }
    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        let values: BoxedSlice<_> =
            Node::ast_from_object(vm, get_node_field(vm, &object, "values", "JoinedStr")?)?;
        Ok(Self {
            values: values.0,
            range: range_from_object(vm, object, "JoinedStr")?,
        })
    }
}

enum JoinedStrPart {
    FormattedValue(FormattedValue),
    Constant(Constant),
}

// constructor
impl Node for JoinedStrPart {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        match self {
            JoinedStrPart::FormattedValue(value) => value.ast_to_object(vm),
            JoinedStrPart::Constant(value) => value.ast_to_object(vm),
        }
    }
    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        let cls = object.class();
        if cls.is(gen::NodeExprFormattedValue::static_type()) {
            Ok(Self::FormattedValue(Node::ast_from_object(vm, object)?))
        } else {
            Ok(Self::Constant(Node::ast_from_object(vm, object)?))
        }
    }
}

struct FormattedValue {
    value: Box<ruff::Expr>,
    conversion: ruff::ConversionFlag,
    format_spec: Option<Box<JoinedStr>>,
    range: TextRange,
}

// constructor
impl Node for FormattedValue {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            value,
            conversion,
            format_spec,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, gen::NodeExprFormattedValue::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(vm), vm).unwrap();
        dict.set_item("conversion", conversion.ast_to_object(vm), vm)
            .unwrap();
        dict.set_item("format_spec", format_spec.ast_to_object(vm), vm)
            .unwrap();
        node_add_location(&dict, range, vm);
        node.into()
    }
    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            value: Node::ast_from_object(
                vm,
                get_node_field(vm, &object, "value", "FormattedValue")?,
            )?,
            conversion: Node::ast_from_object(
                vm,
                get_node_field(vm, &object, "conversion", "FormattedValue")?,
            )?,
            format_spec: get_node_field_opt(vm, &object, "format_spec")?
                .map(|obj| Node::ast_from_object(vm, obj))
                .transpose()?,
            range: range_from_object(vm, object, "FormattedValue")?,
        })
    }
}
