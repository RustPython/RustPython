use super::*;

// sum
impl Node for ruff::BoolOp {
    fn ast_to_object(self, vm: &VirtualMachine, _source_code: &SourceCodeOwned) -> PyObjectRef {
        let node_type = match self {
            ruff::BoolOp::And => gen::NodeBoolOpAnd::static_type(),
            ruff::BoolOp::Or => gen::NodeBoolOpOr::static_type(),
        };
        NodeAst
            .into_ref_with_type(vm, node_type.to_owned())
            .unwrap()
            .into()
    }
    fn ast_from_object(
        _vm: &VirtualMachine,
        _source_code: &SourceCodeOwned,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        let _cls = _object.class();
        Ok(if _cls.is(gen::NodeBoolOpAnd::static_type()) {
            ruff::BoolOp::And
        } else if _cls.is(gen::NodeBoolOpOr::static_type()) {
            ruff::BoolOp::Or
        } else {
            return Err(_vm.new_type_error(format!(
                "expected some sort of boolop, but got {}",
                _object.repr(_vm)?
            )));
        })
    }
}
// sum
impl Node for ruff::Operator {
    fn ast_to_object(self, vm: &VirtualMachine, _source_code: &SourceCodeOwned) -> PyObjectRef {
        let node_type = match self {
            ruff::Operator::Add => gen::NodeOperatorAdd::static_type(),
            ruff::Operator::Sub => gen::NodeOperatorSub::static_type(),
            ruff::Operator::Mult => gen::NodeOperatorMult::static_type(),
            ruff::Operator::MatMult => gen::NodeOperatorMatMult::static_type(),
            ruff::Operator::Div => gen::NodeOperatorDiv::static_type(),
            ruff::Operator::Mod => gen::NodeOperatorMod::static_type(),
            ruff::Operator::Pow => gen::NodeOperatorPow::static_type(),
            ruff::Operator::LShift => gen::NodeOperatorLShift::static_type(),
            ruff::Operator::RShift => gen::NodeOperatorRShift::static_type(),
            ruff::Operator::BitOr => gen::NodeOperatorBitOr::static_type(),
            ruff::Operator::BitXor => gen::NodeOperatorBitXor::static_type(),
            ruff::Operator::BitAnd => gen::NodeOperatorBitAnd::static_type(),
            ruff::Operator::FloorDiv => gen::NodeOperatorFloorDiv::static_type(),
        };
        NodeAst
            .into_ref_with_type(vm, node_type.to_owned())
            .unwrap()
            .into()
    }
    fn ast_from_object(
        _vm: &VirtualMachine,
        _source_code: &SourceCodeOwned,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        let _cls = _object.class();
        Ok(if _cls.is(gen::NodeOperatorAdd::static_type()) {
            ruff::Operator::Add
        } else if _cls.is(gen::NodeOperatorSub::static_type()) {
            ruff::Operator::Sub
        } else if _cls.is(gen::NodeOperatorMult::static_type()) {
            ruff::Operator::Mult
        } else if _cls.is(gen::NodeOperatorMatMult::static_type()) {
            ruff::Operator::MatMult
        } else if _cls.is(gen::NodeOperatorDiv::static_type()) {
            ruff::Operator::Div
        } else if _cls.is(gen::NodeOperatorMod::static_type()) {
            ruff::Operator::Mod
        } else if _cls.is(gen::NodeOperatorPow::static_type()) {
            ruff::Operator::Pow
        } else if _cls.is(gen::NodeOperatorLShift::static_type()) {
            ruff::Operator::LShift
        } else if _cls.is(gen::NodeOperatorRShift::static_type()) {
            ruff::Operator::RShift
        } else if _cls.is(gen::NodeOperatorBitOr::static_type()) {
            ruff::Operator::BitOr
        } else if _cls.is(gen::NodeOperatorBitXor::static_type()) {
            ruff::Operator::BitXor
        } else if _cls.is(gen::NodeOperatorBitAnd::static_type()) {
            ruff::Operator::BitAnd
        } else if _cls.is(gen::NodeOperatorFloorDiv::static_type()) {
            ruff::Operator::FloorDiv
        } else {
            return Err(_vm.new_type_error(format!(
                "expected some sort of operator, but got {}",
                _object.repr(_vm)?
            )));
        })
    }
}
// sum
impl Node for ruff::UnaryOp {
    fn ast_to_object(self, vm: &VirtualMachine, _source_code: &SourceCodeOwned) -> PyObjectRef {
        let node_type = match self {
            ruff::UnaryOp::Invert => gen::NodeUnaryOpInvert::static_type(),
            ruff::UnaryOp::Not => gen::NodeUnaryOpNot::static_type(),
            ruff::UnaryOp::UAdd => gen::NodeUnaryOpUAdd::static_type(),
            ruff::UnaryOp::USub => gen::NodeUnaryOpUSub::static_type(),
        };
        NodeAst
            .into_ref_with_type(vm, node_type.to_owned())
            .unwrap()
            .into()
    }
    fn ast_from_object(
        _vm: &VirtualMachine,
        _source_code: &SourceCodeOwned,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        let _cls = _object.class();
        Ok(if _cls.is(gen::NodeUnaryOpInvert::static_type()) {
            ruff::UnaryOp::Invert
        } else if _cls.is(gen::NodeUnaryOpNot::static_type()) {
            ruff::UnaryOp::Not
        } else if _cls.is(gen::NodeUnaryOpUAdd::static_type()) {
            ruff::UnaryOp::UAdd
        } else if _cls.is(gen::NodeUnaryOpUSub::static_type()) {
            ruff::UnaryOp::USub
        } else {
            return Err(_vm.new_type_error(format!(
                "expected some sort of unaryop, but got {}",
                _object.repr(_vm)?
            )));
        })
    }
}
// sum
impl Node for ruff::CmpOp {
    fn ast_to_object(self, vm: &VirtualMachine, _source_code: &SourceCodeOwned) -> PyObjectRef {
        let node_type = match self {
            ruff::CmpOp::Eq => gen::NodeCmpOpEq::static_type(),
            ruff::CmpOp::NotEq => gen::NodeCmpOpNotEq::static_type(),
            ruff::CmpOp::Lt => gen::NodeCmpOpLt::static_type(),
            ruff::CmpOp::LtE => gen::NodeCmpOpLtE::static_type(),
            ruff::CmpOp::Gt => gen::NodeCmpOpGt::static_type(),
            ruff::CmpOp::GtE => gen::NodeCmpOpGtE::static_type(),
            ruff::CmpOp::Is => gen::NodeCmpOpIs::static_type(),
            ruff::CmpOp::IsNot => gen::NodeCmpOpIsNot::static_type(),
            ruff::CmpOp::In => gen::NodeCmpOpIn::static_type(),
            ruff::CmpOp::NotIn => gen::NodeCmpOpNotIn::static_type(),
        };
        NodeAst
            .into_ref_with_type(vm, node_type.to_owned())
            .unwrap()
            .into()
    }
    fn ast_from_object(
        _vm: &VirtualMachine,
        _source_code: &SourceCodeOwned,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        let _cls = _object.class();
        Ok(if _cls.is(gen::NodeCmpOpEq::static_type()) {
            ruff::CmpOp::Eq
        } else if _cls.is(gen::NodeCmpOpNotEq::static_type()) {
            ruff::CmpOp::NotEq
        } else if _cls.is(gen::NodeCmpOpLt::static_type()) {
            ruff::CmpOp::Lt
        } else if _cls.is(gen::NodeCmpOpLtE::static_type()) {
            ruff::CmpOp::LtE
        } else if _cls.is(gen::NodeCmpOpGt::static_type()) {
            ruff::CmpOp::Gt
        } else if _cls.is(gen::NodeCmpOpGtE::static_type()) {
            ruff::CmpOp::GtE
        } else if _cls.is(gen::NodeCmpOpIs::static_type()) {
            ruff::CmpOp::Is
        } else if _cls.is(gen::NodeCmpOpIsNot::static_type()) {
            ruff::CmpOp::IsNot
        } else if _cls.is(gen::NodeCmpOpIn::static_type()) {
            ruff::CmpOp::In
        } else if _cls.is(gen::NodeCmpOpNotIn::static_type()) {
            ruff::CmpOp::NotIn
        } else {
            return Err(_vm.new_type_error(format!(
                "expected some sort of cmpop, but got {}",
                _object.repr(_vm)?
            )));
        })
    }
}
