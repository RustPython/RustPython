use super::*;
use crate::stdlib::ast::argument::{merge_function_call_arguments, split_function_call_arguments};
use crate::stdlib::ast::constant::Constant;
use crate::stdlib::ast::string::JoinedStr;

// sum
impl Node for ruff::Expr {
    fn ast_to_object(self, vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        match self {
            ruff::Expr::BoolOp(cons) => cons.ast_to_object(vm, source_code),
            ruff::Expr::Name(cons) => cons.ast_to_object(vm, source_code),
            ruff::Expr::BinOp(cons) => cons.ast_to_object(vm, source_code),
            ruff::Expr::UnaryOp(cons) => cons.ast_to_object(vm, source_code),
            ruff::Expr::Lambda(cons) => cons.ast_to_object(vm, source_code),
            ruff::Expr::If(cons) => cons.ast_to_object(vm, source_code),
            ruff::Expr::Dict(cons) => cons.ast_to_object(vm, source_code),
            ruff::Expr::Set(cons) => cons.ast_to_object(vm, source_code),
            ruff::Expr::ListComp(cons) => cons.ast_to_object(vm, source_code),
            ruff::Expr::SetComp(cons) => cons.ast_to_object(vm, source_code),
            ruff::Expr::DictComp(cons) => cons.ast_to_object(vm, source_code),
            ruff::Expr::Generator(cons) => cons.ast_to_object(vm, source_code),
            ruff::Expr::Await(cons) => cons.ast_to_object(vm, source_code),
            ruff::Expr::Yield(cons) => cons.ast_to_object(vm, source_code),
            ruff::Expr::YieldFrom(cons) => cons.ast_to_object(vm, source_code),
            ruff::Expr::Compare(cons) => cons.ast_to_object(vm, source_code),
            ruff::Expr::Call(cons) => cons.ast_to_object(vm, source_code),
            ruff::Expr::Attribute(cons) => cons.ast_to_object(vm, source_code),
            ruff::Expr::Subscript(cons) => cons.ast_to_object(vm, source_code),
            ruff::Expr::Starred(cons) => cons.ast_to_object(vm, source_code),
            ruff::Expr::List(cons) => cons.ast_to_object(vm, source_code),
            ruff::Expr::Tuple(cons) => cons.ast_to_object(vm, source_code),
            ruff::Expr::Slice(cons) => cons.ast_to_object(vm, source_code),
            ruff::Expr::NumberLiteral(cons) => {
                constant::number_literal_to_object(vm, source_code, cons)
            }
            ruff::Expr::StringLiteral(cons) => {
                constant::string_literal_to_object(vm, source_code, cons)
            }
            ruff::Expr::FString(cons) => string::fstring_to_object(vm, source_code, cons),
            ruff::Expr::BytesLiteral(cons) => {
                constant::bytes_literal_to_object(vm, source_code, cons)
            }
            ruff::Expr::BooleanLiteral(cons) => {
                constant::boolean_literal_to_object(vm, source_code, cons)
            }
            ruff::Expr::NoneLiteral(cons) => {
                constant::none_literal_to_object(vm, source_code, cons)
            }
            ruff::Expr::EllipsisLiteral(cons) => {
                constant::ellipsis_literal_to_object(vm, source_code, cons)
            }
            ruff::Expr::Named(cons) => cons.ast_to_object(vm, source_code),
            ruff::Expr::IpyEscapeCommand(_) => {
                unimplemented!("IPython escape command is not allowed in Python AST")
            }
        }
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let cls = object.class();
        Ok(if cls.is(pyast::NodeExprBoolOp::static_type()) {
            ruff::Expr::BoolOp(ruff::ExprBoolOp::ast_from_object(vm, source_code, object)?)
        } else if cls.is(pyast::NodeExprNamedExpr::static_type()) {
            ruff::Expr::Named(ruff::ExprNamed::ast_from_object(vm, source_code, object)?)
        } else if cls.is(pyast::NodeExprBinOp::static_type()) {
            ruff::Expr::BinOp(ruff::ExprBinOp::ast_from_object(vm, source_code, object)?)
        } else if cls.is(pyast::NodeExprUnaryOp::static_type()) {
            ruff::Expr::UnaryOp(ruff::ExprUnaryOp::ast_from_object(vm, source_code, object)?)
        } else if cls.is(pyast::NodeExprLambda::static_type()) {
            ruff::Expr::Lambda(ruff::ExprLambda::ast_from_object(vm, source_code, object)?)
        } else if cls.is(pyast::NodeExprIfExp::static_type()) {
            ruff::Expr::If(ruff::ExprIf::ast_from_object(vm, source_code, object)?)
        } else if cls.is(pyast::NodeExprDict::static_type()) {
            ruff::Expr::Dict(ruff::ExprDict::ast_from_object(vm, source_code, object)?)
        } else if cls.is(pyast::NodeExprSet::static_type()) {
            ruff::Expr::Set(ruff::ExprSet::ast_from_object(vm, source_code, object)?)
        } else if cls.is(pyast::NodeExprListComp::static_type()) {
            ruff::Expr::ListComp(ruff::ExprListComp::ast_from_object(
                vm,
                source_code,
                object,
            )?)
        } else if cls.is(pyast::NodeExprSetComp::static_type()) {
            ruff::Expr::SetComp(ruff::ExprSetComp::ast_from_object(vm, source_code, object)?)
        } else if cls.is(pyast::NodeExprDictComp::static_type()) {
            ruff::Expr::DictComp(ruff::ExprDictComp::ast_from_object(
                vm,
                source_code,
                object,
            )?)
        } else if cls.is(pyast::NodeExprGeneratorExp::static_type()) {
            ruff::Expr::Generator(ruff::ExprGenerator::ast_from_object(
                vm,
                source_code,
                object,
            )?)
        } else if cls.is(pyast::NodeExprAwait::static_type()) {
            ruff::Expr::Await(ruff::ExprAwait::ast_from_object(vm, source_code, object)?)
        } else if cls.is(pyast::NodeExprYield::static_type()) {
            ruff::Expr::Yield(ruff::ExprYield::ast_from_object(vm, source_code, object)?)
        } else if cls.is(pyast::NodeExprYieldFrom::static_type()) {
            ruff::Expr::YieldFrom(ruff::ExprYieldFrom::ast_from_object(
                vm,
                source_code,
                object,
            )?)
        } else if cls.is(pyast::NodeExprCompare::static_type()) {
            ruff::Expr::Compare(ruff::ExprCompare::ast_from_object(vm, source_code, object)?)
        } else if cls.is(pyast::NodeExprCall::static_type()) {
            ruff::Expr::Call(ruff::ExprCall::ast_from_object(vm, source_code, object)?)
        } else if cls.is(pyast::NodeExprAttribute::static_type()) {
            ruff::Expr::Attribute(ruff::ExprAttribute::ast_from_object(
                vm,
                source_code,
                object,
            )?)
        } else if cls.is(pyast::NodeExprSubscript::static_type()) {
            ruff::Expr::Subscript(ruff::ExprSubscript::ast_from_object(
                vm,
                source_code,
                object,
            )?)
        } else if cls.is(pyast::NodeExprStarred::static_type()) {
            ruff::Expr::Starred(ruff::ExprStarred::ast_from_object(vm, source_code, object)?)
        } else if cls.is(pyast::NodeExprName::static_type()) {
            ruff::Expr::Name(ruff::ExprName::ast_from_object(vm, source_code, object)?)
        } else if cls.is(pyast::NodeExprList::static_type()) {
            ruff::Expr::List(ruff::ExprList::ast_from_object(vm, source_code, object)?)
        } else if cls.is(pyast::NodeExprTuple::static_type()) {
            ruff::Expr::Tuple(ruff::ExprTuple::ast_from_object(vm, source_code, object)?)
        } else if cls.is(pyast::NodeExprSlice::static_type()) {
            ruff::Expr::Slice(ruff::ExprSlice::ast_from_object(vm, source_code, object)?)
        } else if cls.is(pyast::NodeExprConstant::static_type()) {
            Constant::ast_from_object(vm, source_code, object)?.into_expr()
        } else if cls.is(pyast::NodeExprJoinedStr::static_type()) {
            JoinedStr::ast_from_object(vm, source_code, object)?.into_expr()
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
    fn ast_to_object(self, vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let Self { op, values, range } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprBoolOp::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("op", op.ast_to_object(vm, source_code), vm)
            .unwrap();
        dict.set_item("values", values.ast_to_object(vm, source_code), vm)
            .unwrap();
        node_add_location(&dict, range, vm, source_code);
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            op: Node::ast_from_object(
                vm,
                source_code,
                get_node_field(vm, &object, "op", "BoolOp")?,
            )?,
            values: Node::ast_from_object(
                vm,
                source_code,
                get_node_field(vm, &object, "values", "BoolOp")?,
            )?,
            range: range_from_object(vm, source_code, object, "BoolOp")?,
        })
    }
}
// constructor
impl Node for ruff::ExprNamed {
    fn ast_to_object(self, vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let Self {
            target,
            value,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprNamedExpr::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("target", target.ast_to_object(vm, source_code), vm)
            .unwrap();
        dict.set_item("value", value.ast_to_object(vm, source_code), vm)
            .unwrap();
        node_add_location(&dict, range, vm, source_code);
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            target: Node::ast_from_object(
                vm,
                source_code,
                get_node_field(vm, &object, "target", "NamedExpr")?,
            )?,
            value: Node::ast_from_object(
                vm,
                source_code,
                get_node_field(vm, &object, "value", "NamedExpr")?,
            )?,
            range: range_from_object(vm, source_code, object, "NamedExpr")?,
        })
    }
}
// constructor
impl Node for ruff::ExprBinOp {
    fn ast_to_object(self, vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let Self {
            left,
            op,
            right,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprBinOp::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("left", left.ast_to_object(vm, source_code), vm)
            .unwrap();
        dict.set_item("op", op.ast_to_object(vm, source_code), vm)
            .unwrap();
        dict.set_item("right", right.ast_to_object(vm, source_code), vm)
            .unwrap();
        node_add_location(&dict, range, vm, source_code);
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            left: Node::ast_from_object(
                vm,
                source_code,
                get_node_field(vm, &object, "left", "BinOp")?,
            )?,
            op: Node::ast_from_object(
                vm,
                source_code,
                get_node_field(vm, &object, "op", "BinOp")?,
            )?,
            right: Node::ast_from_object(
                vm,
                source_code,
                get_node_field(vm, &object, "right", "BinOp")?,
            )?,
            range: range_from_object(vm, source_code, object, "BinOp")?,
        })
    }
}
// constructor
impl Node for ruff::ExprUnaryOp {
    fn ast_to_object(self, vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let Self { op, operand, range } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprUnaryOp::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("op", op.ast_to_object(vm, source_code), vm)
            .unwrap();
        dict.set_item("operand", operand.ast_to_object(vm, source_code), vm)
            .unwrap();
        node_add_location(&dict, range, vm, source_code);
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            op: Node::ast_from_object(
                vm,
                source_code,
                get_node_field(vm, &object, "op", "UnaryOp")?,
            )?,
            operand: Node::ast_from_object(
                vm,
                source_code,
                get_node_field(vm, &object, "operand", "UnaryOp")?,
            )?,
            range: range_from_object(vm, source_code, object, "UnaryOp")?,
        })
    }
}
// constructor
impl Node for ruff::ExprLambda {
    fn ast_to_object(self, vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let Self {
            parameters,
            body,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprLambda::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("args", parameters.ast_to_object(vm, source_code), vm)
            .unwrap();
        dict.set_item("body", body.ast_to_object(vm, source_code), vm)
            .unwrap();
        node_add_location(&dict, _range, vm, source_code);
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            parameters: Node::ast_from_object(
                vm,
                source_code,
                get_node_field(vm, &object, "args", "Lambda")?,
            )?,
            body: Node::ast_from_object(
                vm,
                source_code,
                get_node_field(vm, &object, "body", "Lambda")?,
            )?,
            range: range_from_object(vm, source_code, object, "Lambda")?,
        })
    }
}
// constructor
impl Node for ruff::ExprIf {
    fn ast_to_object(self, vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let Self {
            test,
            body,
            orelse,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprIfExp::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("test", test.ast_to_object(vm, source_code), vm)
            .unwrap();
        dict.set_item("body", body.ast_to_object(vm, source_code), vm)
            .unwrap();
        dict.set_item("orelse", orelse.ast_to_object(vm, source_code), vm)
            .unwrap();
        node_add_location(&dict, range, vm, source_code);
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            test: Node::ast_from_object(
                vm,
                source_code,
                get_node_field(vm, &object, "test", "IfExp")?,
            )?,
            body: Node::ast_from_object(
                vm,
                source_code,
                get_node_field(vm, &object, "body", "IfExp")?,
            )?,
            orelse: Node::ast_from_object(
                vm,
                source_code,
                get_node_field(vm, &object, "orelse", "IfExp")?,
            )?,
            range: range_from_object(vm, source_code, object, "IfExp")?,
        })
    }
}
// constructor
impl Node for ruff::ExprDict {
    fn ast_to_object(self, vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
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
            .into_ref_with_type(vm, pyast::NodeExprDict::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("keys", keys.ast_to_object(vm, source_code), vm)
            .unwrap();
        dict.set_item("values", values.ast_to_object(vm, source_code), vm)
            .unwrap();
        node_add_location(&dict, range, vm, source_code);
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let keys: Vec<Option<ruff::Expr>> = Node::ast_from_object(
            vm,
            source_code,
            get_node_field(vm, &object, "keys", "Dict")?,
        )?;
        let values: Vec<_> = Node::ast_from_object(
            vm,
            source_code,
            get_node_field(vm, &object, "values", "Dict")?,
        )?;
        let items = keys
            .into_iter()
            .zip(values)
            .map(|(key, value)| ruff::DictItem { key, value })
            .collect();
        Ok(Self {
            items,
            range: range_from_object(vm, source_code, object, "Dict")?,
        })
    }
}
// constructor
impl Node for ruff::ExprSet {
    fn ast_to_object(self, vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let Self { elts, range } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprSet::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("elts", elts.ast_to_object(vm, source_code), vm)
            .unwrap();
        node_add_location(&dict, range, vm, source_code);
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            elts: Node::ast_from_object(
                vm,
                source_code,
                get_node_field(vm, &object, "elts", "Set")?,
            )?,
            range: range_from_object(vm, source_code, object, "Set")?,
        })
    }
}
// constructor
impl Node for ruff::ExprListComp {
    fn ast_to_object(self, vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let Self {
            elt,
            generators,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprListComp::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("elt", elt.ast_to_object(vm, source_code), vm)
            .unwrap();
        dict.set_item("generators", generators.ast_to_object(vm, source_code), vm)
            .unwrap();
        node_add_location(&dict, range, vm, source_code);
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            elt: Node::ast_from_object(
                vm,
                source_code,
                get_node_field(vm, &object, "elt", "ListComp")?,
            )?,
            generators: Node::ast_from_object(
                vm,
                source_code,
                get_node_field(vm, &object, "generators", "ListComp")?,
            )?,
            range: range_from_object(vm, source_code, object, "ListComp")?,
        })
    }
}
// constructor
impl Node for ruff::ExprSetComp {
    fn ast_to_object(self, vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let Self {
            elt,
            generators,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprSetComp::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("elt", elt.ast_to_object(vm, source_code), vm)
            .unwrap();
        dict.set_item("generators", generators.ast_to_object(vm, source_code), vm)
            .unwrap();
        node_add_location(&dict, range, vm, source_code);
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            elt: Node::ast_from_object(
                vm,
                source_code,
                get_node_field(vm, &object, "elt", "SetComp")?,
            )?,
            generators: Node::ast_from_object(
                vm,
                source_code,
                get_node_field(vm, &object, "generators", "SetComp")?,
            )?,
            range: range_from_object(vm, source_code, object, "SetComp")?,
        })
    }
}
// constructor
impl Node for ruff::ExprDictComp {
    fn ast_to_object(self, vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let Self {
            key,
            value,
            generators,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprDictComp::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("key", key.ast_to_object(vm, source_code), vm)
            .unwrap();
        dict.set_item("value", value.ast_to_object(vm, source_code), vm)
            .unwrap();
        dict.set_item("generators", generators.ast_to_object(vm, source_code), vm)
            .unwrap();
        node_add_location(&dict, range, vm, source_code);
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            key: Node::ast_from_object(
                vm,
                source_code,
                get_node_field(vm, &object, "key", "DictComp")?,
            )?,
            value: Node::ast_from_object(
                vm,
                source_code,
                get_node_field(vm, &object, "value", "DictComp")?,
            )?,
            generators: Node::ast_from_object(
                vm,
                source_code,
                get_node_field(vm, &object, "generators", "DictComp")?,
            )?,
            range: range_from_object(vm, source_code, object, "DictComp")?,
        })
    }
}
// constructor
impl Node for ruff::ExprGenerator {
    fn ast_to_object(self, vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let Self {
            elt,
            generators,
            range,
            parenthesized: _,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprGeneratorExp::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("elt", elt.ast_to_object(vm, source_code), vm)
            .unwrap();
        dict.set_item("generators", generators.ast_to_object(vm, source_code), vm)
            .unwrap();
        node_add_location(&dict, range, vm, source_code);
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            elt: Node::ast_from_object(
                vm,
                source_code,
                get_node_field(vm, &object, "elt", "GeneratorExp")?,
            )?,
            generators: Node::ast_from_object(
                vm,
                source_code,
                get_node_field(vm, &object, "generators", "GeneratorExp")?,
            )?,
            range: range_from_object(vm, source_code, object, "GeneratorExp")?,
            // TODO: Is this correct?
            parenthesized: true,
        })
    }
}
// constructor
impl Node for ruff::ExprAwait {
    fn ast_to_object(self, vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let Self { value, range } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprAwait::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(vm, source_code), vm)
            .unwrap();
        node_add_location(&dict, range, vm, source_code);
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            value: Node::ast_from_object(
                vm,
                source_code,
                get_node_field(vm, &object, "value", "Await")?,
            )?,
            range: range_from_object(vm, source_code, object, "Await")?,
        })
    }
}
// constructor
impl Node for ruff::ExprYield {
    fn ast_to_object(self, vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let ruff::ExprYield { value, range } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprYield::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(vm, source_code), vm)
            .unwrap();
        node_add_location(&dict, range, vm, source_code);
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(ruff::ExprYield {
            value: get_node_field_opt(vm, &object, "value")?
                .map(|obj| Node::ast_from_object(vm, source_code, obj))
                .transpose()?,
            range: range_from_object(vm, source_code, object, "Yield")?,
        })
    }
}
// constructor
impl Node for ruff::ExprYieldFrom {
    fn ast_to_object(self, vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let Self { value, range } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprYieldFrom::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(vm, source_code), vm)
            .unwrap();
        node_add_location(&dict, range, vm, source_code);
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            value: Node::ast_from_object(
                vm,
                source_code,
                get_node_field(vm, &object, "value", "YieldFrom")?,
            )?,
            range: range_from_object(vm, source_code, object, "YieldFrom")?,
        })
    }
}
// constructor
impl Node for ruff::ExprCompare {
    fn ast_to_object(self, vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let Self {
            left,
            ops,
            comparators,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprCompare::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("left", left.ast_to_object(vm, source_code), vm)
            .unwrap();
        dict.set_item("ops", BoxedSlice(ops).ast_to_object(vm, source_code), vm)
            .unwrap();
        dict.set_item(
            "comparators",
            BoxedSlice(comparators).ast_to_object(vm, source_code),
            vm,
        )
        .unwrap();
        node_add_location(&dict, range, vm, source_code);
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            left: Node::ast_from_object(
                vm,
                source_code,
                get_node_field(vm, &object, "left", "Compare")?,
            )?,
            ops: {
                let ops: BoxedSlice<_> = Node::ast_from_object(
                    vm,
                    source_code,
                    get_node_field(vm, &object, "ops", "Compare")?,
                )?;
                ops.0
            },
            comparators: {
                let comparators: BoxedSlice<_> = Node::ast_from_object(
                    vm,
                    source_code,
                    get_node_field(vm, &object, "comparators", "Compare")?,
                )?;
                comparators.0
            },
            range: range_from_object(vm, source_code, object, "Compare")?,
        })
    }
}
// constructor
impl Node for ruff::ExprCall {
    fn ast_to_object(self, vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let Self {
            func,
            arguments,
            range,
        } = self;
        let (positional_arguments, keyword_arguments) = split_function_call_arguments(arguments);
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprCall::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("func", func.ast_to_object(vm, source_code), vm)
            .unwrap();
        dict.set_item(
            "args",
            positional_arguments.ast_to_object(vm, source_code),
            vm,
        )
        .unwrap();
        dict.set_item(
            "keywords",
            keyword_arguments.ast_to_object(vm, source_code),
            vm,
        )
        .unwrap();
        node_add_location(&dict, range, vm, source_code);
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            func: Node::ast_from_object(
                vm,
                source_code,
                get_node_field(vm, &object, "func", "Call")?,
            )?,
            arguments: merge_function_call_arguments(
                Node::ast_from_object(
                    vm,
                    source_code,
                    get_node_field(vm, &object, "args", "Call")?,
                )?,
                Node::ast_from_object(
                    vm,
                    source_code,
                    get_node_field(vm, &object, "keywords", "Call")?,
                )?,
            ),
            range: range_from_object(vm, source_code, object, "Call")?,
        })
    }
}

// constructor
impl Node for ruff::ExprAttribute {
    fn ast_to_object(self, vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let Self {
            value,
            attr,
            ctx,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprAttribute::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(vm, source_code), vm)
            .unwrap();
        dict.set_item("attr", attr.ast_to_object(vm, source_code), vm)
            .unwrap();
        dict.set_item("ctx", ctx.ast_to_object(vm, source_code), vm)
            .unwrap();
        node_add_location(&dict, range, vm, source_code);
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            value: Node::ast_from_object(
                vm,
                source_code,
                get_node_field(vm, &object, "value", "Attribute")?,
            )?,
            attr: Node::ast_from_object(
                vm,
                source_code,
                get_node_field(vm, &object, "attr", "Attribute")?,
            )?,
            ctx: Node::ast_from_object(
                vm,
                source_code,
                get_node_field(vm, &object, "ctx", "Attribute")?,
            )?,
            range: range_from_object(vm, source_code, object, "Attribute")?,
        })
    }
}
// constructor
impl Node for ruff::ExprSubscript {
    fn ast_to_object(self, vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let Self {
            value,
            slice,
            ctx,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprSubscript::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(vm, source_code), vm)
            .unwrap();
        dict.set_item("slice", slice.ast_to_object(vm, source_code), vm)
            .unwrap();
        dict.set_item("ctx", ctx.ast_to_object(vm, source_code), vm)
            .unwrap();
        node_add_location(&dict, _range, vm, source_code);
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            value: Node::ast_from_object(
                vm,
                source_code,
                get_node_field(vm, &object, "value", "Subscript")?,
            )?,
            slice: Node::ast_from_object(
                vm,
                source_code,
                get_node_field(vm, &object, "slice", "Subscript")?,
            )?,
            ctx: Node::ast_from_object(
                vm,
                source_code,
                get_node_field(vm, &object, "ctx", "Subscript")?,
            )?,
            range: range_from_object(vm, source_code, object, "Subscript")?,
        })
    }
}
// constructor
impl Node for ruff::ExprStarred {
    fn ast_to_object(self, vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let Self { value, ctx, range } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprStarred::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(vm, source_code), vm)
            .unwrap();
        dict.set_item("ctx", ctx.ast_to_object(vm, source_code), vm)
            .unwrap();
        node_add_location(&dict, range, vm, source_code);
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            value: Node::ast_from_object(
                vm,
                source_code,
                get_node_field(vm, &object, "value", "Starred")?,
            )?,
            ctx: Node::ast_from_object(
                vm,
                source_code,
                get_node_field(vm, &object, "ctx", "Starred")?,
            )?,
            range: range_from_object(vm, source_code, object, "Starred")?,
        })
    }
}
// constructor
impl Node for ruff::ExprName {
    fn ast_to_object(self, vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let Self { id, ctx, range } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprName::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("id", id.to_pyobject(vm), vm).unwrap();
        dict.set_item("ctx", ctx.ast_to_object(vm, source_code), vm)
            .unwrap();
        node_add_location(&dict, range, vm, source_code);
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            id: get_node_field(vm, &object, "id", "Name")?.try_into_value(vm)?,
            ctx: Node::ast_from_object(
                vm,
                source_code,
                get_node_field(vm, &object, "ctx", "Name")?,
            )?,
            range: range_from_object(vm, source_code, object, "Name")?,
        })
    }
}
// constructor
impl Node for ruff::ExprList {
    fn ast_to_object(self, vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let ruff::ExprList { elts, ctx, range } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprList::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("elts", elts.ast_to_object(vm, source_code), vm)
            .unwrap();
        dict.set_item("ctx", ctx.ast_to_object(vm, source_code), vm)
            .unwrap();
        node_add_location(&dict, range, vm, source_code);
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(ruff::ExprList {
            elts: Node::ast_from_object(
                vm,
                source_code,
                get_node_field(vm, &object, "elts", "List")?,
            )?,
            ctx: Node::ast_from_object(
                vm,
                source_code,
                get_node_field(vm, &object, "ctx", "List")?,
            )?,
            range: range_from_object(vm, source_code, object, "List")?,
        })
    }
}
// constructor
impl Node for ruff::ExprTuple {
    fn ast_to_object(self, vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let Self {
            elts,
            ctx,
            range: _range,
            parenthesized: _,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprTuple::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("elts", elts.ast_to_object(vm, source_code), vm)
            .unwrap();
        dict.set_item("ctx", ctx.ast_to_object(vm, source_code), vm)
            .unwrap();
        node_add_location(&dict, _range, vm, source_code);
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            elts: Node::ast_from_object(
                vm,
                source_code,
                get_node_field(vm, &object, "elts", "Tuple")?,
            )?,
            ctx: Node::ast_from_object(
                vm,
                source_code,
                get_node_field(vm, &object, "ctx", "Tuple")?,
            )?,
            range: range_from_object(vm, source_code, object, "Tuple")?,
            parenthesized: true, // TODO: is this correct?
        })
    }
}
// constructor
impl Node for ruff::ExprSlice {
    fn ast_to_object(self, vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let Self {
            lower,
            upper,
            step,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeExprSlice::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("lower", lower.ast_to_object(vm, source_code), vm)
            .unwrap();
        dict.set_item("upper", upper.ast_to_object(vm, source_code), vm)
            .unwrap();
        dict.set_item("step", step.ast_to_object(vm, source_code), vm)
            .unwrap();
        node_add_location(&dict, _range, vm, source_code);
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            lower: get_node_field_opt(vm, &object, "lower")?
                .map(|obj| Node::ast_from_object(vm, source_code, obj))
                .transpose()?,
            upper: get_node_field_opt(vm, &object, "upper")?
                .map(|obj| Node::ast_from_object(vm, source_code, obj))
                .transpose()?,
            step: get_node_field_opt(vm, &object, "step")?
                .map(|obj| Node::ast_from_object(vm, source_code, obj))
                .transpose()?,
            range: range_from_object(vm, source_code, object, "Slice")?,
        })
    }
}
// sum
impl Node for ruff::ExprContext {
    fn ast_to_object(self, vm: &VirtualMachine, _source_code: &SourceCodeOwned) -> PyObjectRef {
        let node_type = match self {
            ruff::ExprContext::Load => pyast::NodeExprContextLoad::static_type(),
            ruff::ExprContext::Store => pyast::NodeExprContextStore::static_type(),
            ruff::ExprContext::Del => pyast::NodeExprContextDel::static_type(),
            ruff::ExprContext::Invalid => {
                unimplemented!("Invalid expression context is not allowed in Python AST")
            }
        };
        NodeAst
            .into_ref_with_type(vm, node_type.to_owned())
            .unwrap()
            .into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        _source_code: &SourceCodeOwned,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let _cls = object.class();
        Ok(if _cls.is(pyast::NodeExprContextLoad::static_type()) {
            ruff::ExprContext::Load
        } else if _cls.is(pyast::NodeExprContextStore::static_type()) {
            ruff::ExprContext::Store
        } else if _cls.is(pyast::NodeExprContextDel::static_type()) {
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
    fn ast_to_object(self, vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let Self {
            target,
            iter,
            ifs,
            is_async,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeComprehension::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("target", target.ast_to_object(vm, source_code), vm)
            .unwrap();
        dict.set_item("iter", iter.ast_to_object(vm, source_code), vm)
            .unwrap();
        dict.set_item("ifs", ifs.ast_to_object(vm, source_code), vm)
            .unwrap();
        dict.set_item("is_async", is_async.ast_to_object(vm, source_code), vm)
            .unwrap();
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            target: Node::ast_from_object(
                vm,
                source_code,
                get_node_field(vm, &object, "target", "comprehension")?,
            )?,
            iter: Node::ast_from_object(
                vm,
                source_code,
                get_node_field(vm, &object, "iter", "comprehension")?,
            )?,
            ifs: Node::ast_from_object(
                vm,
                source_code,
                get_node_field(vm, &object, "ifs", "comprehension")?,
            )?,
            is_async: Node::ast_from_object(
                vm,
                source_code,
                get_node_field(vm, &object, "is_async", "comprehension")?,
            )?,
            range: Default::default(),
        })
    }
}
