use super::*;

// sum
impl Node for ruff::BoolOp {
    fn ast_to_object(self, vm: &VirtualMachine, _source_code: &SourceCodeOwned) -> PyObjectRef {
        let node_type = match self {
            ruff::BoolOp::And => pyast::NodeBoolOpAnd::static_type(),
            ruff::BoolOp::Or => pyast::NodeBoolOpOr::static_type(),
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
        Ok(if _cls.is(pyast::NodeBoolOpAnd::static_type()) {
            ruff::BoolOp::And
        } else if _cls.is(pyast::NodeBoolOpOr::static_type()) {
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
            ruff::Operator::Add => pyast::NodeOperatorAdd::static_type(),
            ruff::Operator::Sub => pyast::NodeOperatorSub::static_type(),
            ruff::Operator::Mult => pyast::NodeOperatorMult::static_type(),
            ruff::Operator::MatMult => pyast::NodeOperatorMatMult::static_type(),
            ruff::Operator::Div => pyast::NodeOperatorDiv::static_type(),
            ruff::Operator::Mod => pyast::NodeOperatorMod::static_type(),
            ruff::Operator::Pow => pyast::NodeOperatorPow::static_type(),
            ruff::Operator::LShift => pyast::NodeOperatorLShift::static_type(),
            ruff::Operator::RShift => pyast::NodeOperatorRShift::static_type(),
            ruff::Operator::BitOr => pyast::NodeOperatorBitOr::static_type(),
            ruff::Operator::BitXor => pyast::NodeOperatorBitXor::static_type(),
            ruff::Operator::BitAnd => pyast::NodeOperatorBitAnd::static_type(),
            ruff::Operator::FloorDiv => pyast::NodeOperatorFloorDiv::static_type(),
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
        Ok(if _cls.is(pyast::NodeOperatorAdd::static_type()) {
            ruff::Operator::Add
        } else if _cls.is(pyast::NodeOperatorSub::static_type()) {
            ruff::Operator::Sub
        } else if _cls.is(pyast::NodeOperatorMult::static_type()) {
            ruff::Operator::Mult
        } else if _cls.is(pyast::NodeOperatorMatMult::static_type()) {
            ruff::Operator::MatMult
        } else if _cls.is(pyast::NodeOperatorDiv::static_type()) {
            ruff::Operator::Div
        } else if _cls.is(pyast::NodeOperatorMod::static_type()) {
            ruff::Operator::Mod
        } else if _cls.is(pyast::NodeOperatorPow::static_type()) {
            ruff::Operator::Pow
        } else if _cls.is(pyast::NodeOperatorLShift::static_type()) {
            ruff::Operator::LShift
        } else if _cls.is(pyast::NodeOperatorRShift::static_type()) {
            ruff::Operator::RShift
        } else if _cls.is(pyast::NodeOperatorBitOr::static_type()) {
            ruff::Operator::BitOr
        } else if _cls.is(pyast::NodeOperatorBitXor::static_type()) {
            ruff::Operator::BitXor
        } else if _cls.is(pyast::NodeOperatorBitAnd::static_type()) {
            ruff::Operator::BitAnd
        } else if _cls.is(pyast::NodeOperatorFloorDiv::static_type()) {
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
            ruff::UnaryOp::Invert => pyast::NodeUnaryOpInvert::static_type(),
            ruff::UnaryOp::Not => pyast::NodeUnaryOpNot::static_type(),
            ruff::UnaryOp::UAdd => pyast::NodeUnaryOpUAdd::static_type(),
            ruff::UnaryOp::USub => pyast::NodeUnaryOpUSub::static_type(),
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
        Ok(if _cls.is(pyast::NodeUnaryOpInvert::static_type()) {
            ruff::UnaryOp::Invert
        } else if _cls.is(pyast::NodeUnaryOpNot::static_type()) {
            ruff::UnaryOp::Not
        } else if _cls.is(pyast::NodeUnaryOpUAdd::static_type()) {
            ruff::UnaryOp::UAdd
        } else if _cls.is(pyast::NodeUnaryOpUSub::static_type()) {
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
            ruff::CmpOp::Eq => pyast::NodeCmpOpEq::static_type(),
            ruff::CmpOp::NotEq => pyast::NodeCmpOpNotEq::static_type(),
            ruff::CmpOp::Lt => pyast::NodeCmpOpLt::static_type(),
            ruff::CmpOp::LtE => pyast::NodeCmpOpLtE::static_type(),
            ruff::CmpOp::Gt => pyast::NodeCmpOpGt::static_type(),
            ruff::CmpOp::GtE => pyast::NodeCmpOpGtE::static_type(),
            ruff::CmpOp::Is => pyast::NodeCmpOpIs::static_type(),
            ruff::CmpOp::IsNot => pyast::NodeCmpOpIsNot::static_type(),
            ruff::CmpOp::In => pyast::NodeCmpOpIn::static_type(),
            ruff::CmpOp::NotIn => pyast::NodeCmpOpNotIn::static_type(),
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
        Ok(if _cls.is(pyast::NodeCmpOpEq::static_type()) {
            ruff::CmpOp::Eq
        } else if _cls.is(pyast::NodeCmpOpNotEq::static_type()) {
            ruff::CmpOp::NotEq
        } else if _cls.is(pyast::NodeCmpOpLt::static_type()) {
            ruff::CmpOp::Lt
        } else if _cls.is(pyast::NodeCmpOpLtE::static_type()) {
            ruff::CmpOp::LtE
        } else if _cls.is(pyast::NodeCmpOpGt::static_type()) {
            ruff::CmpOp::Gt
        } else if _cls.is(pyast::NodeCmpOpGtE::static_type()) {
            ruff::CmpOp::GtE
        } else if _cls.is(pyast::NodeCmpOpIs::static_type()) {
            ruff::CmpOp::Is
        } else if _cls.is(pyast::NodeCmpOpIsNot::static_type()) {
            ruff::CmpOp::IsNot
        } else if _cls.is(pyast::NodeCmpOpIn::static_type()) {
            ruff::CmpOp::In
        } else if _cls.is(pyast::NodeCmpOpNotIn::static_type()) {
            ruff::CmpOp::NotIn
        } else {
            return Err(_vm.new_type_error(format!(
                "expected some sort of cmpop, but got {}",
                _object.repr(_vm)?
            )));
        })
    }
}
