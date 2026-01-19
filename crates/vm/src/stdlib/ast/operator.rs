use super::*;
use rustpython_compiler_core::SourceFile;

// sum
impl Node for ast::BoolOp {
    fn ast_to_object(self, vm: &VirtualMachine, _source_file: &SourceFile) -> PyObjectRef {
        let node_type = match self {
            Self::And => pyast::NodeBoolOpAnd::static_type(),
            Self::Or => pyast::NodeBoolOpOr::static_type(),
        };
        NodeAst
            .into_ref_with_type(vm, node_type.to_owned())
            .unwrap()
            .into()
    }

    fn ast_from_object(
        _vm: &VirtualMachine,
        _source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        let _cls = _object.class();
        Ok(if _cls.is(pyast::NodeBoolOpAnd::static_type()) {
            Self::And
        } else if _cls.is(pyast::NodeBoolOpOr::static_type()) {
            Self::Or
        } else {
            return Err(_vm.new_type_error(format!(
                "expected some sort of boolop, but got {}",
                _object.repr(_vm)?
            )));
        })
    }
}

// sum
impl Node for ast::Operator {
    fn ast_to_object(self, vm: &VirtualMachine, _source_file: &SourceFile) -> PyObjectRef {
        let node_type = match self {
            Self::Add => pyast::NodeOperatorAdd::static_type(),
            Self::Sub => pyast::NodeOperatorSub::static_type(),
            Self::Mult => pyast::NodeOperatorMult::static_type(),
            Self::MatMult => pyast::NodeOperatorMatMult::static_type(),
            Self::Div => pyast::NodeOperatorDiv::static_type(),
            Self::Mod => pyast::NodeOperatorMod::static_type(),
            Self::Pow => pyast::NodeOperatorPow::static_type(),
            Self::LShift => pyast::NodeOperatorLShift::static_type(),
            Self::RShift => pyast::NodeOperatorRShift::static_type(),
            Self::BitOr => pyast::NodeOperatorBitOr::static_type(),
            Self::BitXor => pyast::NodeOperatorBitXor::static_type(),
            Self::BitAnd => pyast::NodeOperatorBitAnd::static_type(),
            Self::FloorDiv => pyast::NodeOperatorFloorDiv::static_type(),
        };
        NodeAst
            .into_ref_with_type(vm, node_type.to_owned())
            .unwrap()
            .into()
    }

    fn ast_from_object(
        _vm: &VirtualMachine,
        _source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        let _cls = _object.class();
        Ok(if _cls.is(pyast::NodeOperatorAdd::static_type()) {
            Self::Add
        } else if _cls.is(pyast::NodeOperatorSub::static_type()) {
            Self::Sub
        } else if _cls.is(pyast::NodeOperatorMult::static_type()) {
            Self::Mult
        } else if _cls.is(pyast::NodeOperatorMatMult::static_type()) {
            Self::MatMult
        } else if _cls.is(pyast::NodeOperatorDiv::static_type()) {
            Self::Div
        } else if _cls.is(pyast::NodeOperatorMod::static_type()) {
            Self::Mod
        } else if _cls.is(pyast::NodeOperatorPow::static_type()) {
            Self::Pow
        } else if _cls.is(pyast::NodeOperatorLShift::static_type()) {
            Self::LShift
        } else if _cls.is(pyast::NodeOperatorRShift::static_type()) {
            Self::RShift
        } else if _cls.is(pyast::NodeOperatorBitOr::static_type()) {
            Self::BitOr
        } else if _cls.is(pyast::NodeOperatorBitXor::static_type()) {
            Self::BitXor
        } else if _cls.is(pyast::NodeOperatorBitAnd::static_type()) {
            Self::BitAnd
        } else if _cls.is(pyast::NodeOperatorFloorDiv::static_type()) {
            Self::FloorDiv
        } else {
            return Err(_vm.new_type_error(format!(
                "expected some sort of operator, but got {}",
                _object.repr(_vm)?
            )));
        })
    }
}

// sum
impl Node for ast::UnaryOp {
    fn ast_to_object(self, vm: &VirtualMachine, _source_file: &SourceFile) -> PyObjectRef {
        let node_type = match self {
            Self::Invert => pyast::NodeUnaryOpInvert::static_type(),
            Self::Not => pyast::NodeUnaryOpNot::static_type(),
            Self::UAdd => pyast::NodeUnaryOpUAdd::static_type(),
            Self::USub => pyast::NodeUnaryOpUSub::static_type(),
        };
        NodeAst
            .into_ref_with_type(vm, node_type.to_owned())
            .unwrap()
            .into()
    }

    fn ast_from_object(
        _vm: &VirtualMachine,
        _source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        let _cls = _object.class();
        Ok(if _cls.is(pyast::NodeUnaryOpInvert::static_type()) {
            Self::Invert
        } else if _cls.is(pyast::NodeUnaryOpNot::static_type()) {
            Self::Not
        } else if _cls.is(pyast::NodeUnaryOpUAdd::static_type()) {
            Self::UAdd
        } else if _cls.is(pyast::NodeUnaryOpUSub::static_type()) {
            Self::USub
        } else {
            return Err(_vm.new_type_error(format!(
                "expected some sort of unaryop, but got {}",
                _object.repr(_vm)?
            )));
        })
    }
}

// sum
impl Node for ast::CmpOp {
    fn ast_to_object(self, vm: &VirtualMachine, _source_file: &SourceFile) -> PyObjectRef {
        let node_type = match self {
            Self::Eq => pyast::NodeCmpOpEq::static_type(),
            Self::NotEq => pyast::NodeCmpOpNotEq::static_type(),
            Self::Lt => pyast::NodeCmpOpLt::static_type(),
            Self::LtE => pyast::NodeCmpOpLtE::static_type(),
            Self::Gt => pyast::NodeCmpOpGt::static_type(),
            Self::GtE => pyast::NodeCmpOpGtE::static_type(),
            Self::Is => pyast::NodeCmpOpIs::static_type(),
            Self::IsNot => pyast::NodeCmpOpIsNot::static_type(),
            Self::In => pyast::NodeCmpOpIn::static_type(),
            Self::NotIn => pyast::NodeCmpOpNotIn::static_type(),
        };
        NodeAst
            .into_ref_with_type(vm, node_type.to_owned())
            .unwrap()
            .into()
    }

    fn ast_from_object(
        _vm: &VirtualMachine,
        _source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        let _cls = _object.class();
        Ok(if _cls.is(pyast::NodeCmpOpEq::static_type()) {
            Self::Eq
        } else if _cls.is(pyast::NodeCmpOpNotEq::static_type()) {
            Self::NotEq
        } else if _cls.is(pyast::NodeCmpOpLt::static_type()) {
            Self::Lt
        } else if _cls.is(pyast::NodeCmpOpLtE::static_type()) {
            Self::LtE
        } else if _cls.is(pyast::NodeCmpOpGt::static_type()) {
            Self::Gt
        } else if _cls.is(pyast::NodeCmpOpGtE::static_type()) {
            Self::GtE
        } else if _cls.is(pyast::NodeCmpOpIs::static_type()) {
            Self::Is
        } else if _cls.is(pyast::NodeCmpOpIsNot::static_type()) {
            Self::IsNot
        } else if _cls.is(pyast::NodeCmpOpIn::static_type()) {
            Self::In
        } else if _cls.is(pyast::NodeCmpOpNotIn::static_type()) {
            Self::NotIn
        } else {
            return Err(_vm.new_type_error(format!(
                "expected some sort of cmpop, but got {}",
                _object.repr(_vm)?
            )));
        })
    }
}
