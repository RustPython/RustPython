use super::*;
use rustpython_compiler_core::SourceFile;

// sum
impl Node for ast::BoolOp {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let _source_file = source_file;
        let node_type = match self {
            Self::And => pyast::NodeBoolOpAnd::static_type(),
            Self::Or => pyast::NodeBoolOpOr::static_type(),
        };
        singleton_node_to_object(vm, node_type)
    }

    fn ast_from_object(
        ctx: &VirtualMachine,
        _source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(
            if is_node_instance(ctx, &object, pyast::NodeBoolOpAnd::static_type())? {
                Self::And
            } else if is_node_instance(ctx, &object, pyast::NodeBoolOpOr::static_type())? {
                Self::Or
            } else {
                return Err(ctx.new_type_error(format!(
                    "expected some sort of boolop, but got {}",
                    object.repr(ctx)?
                )));
            },
        )
    }
}

// sum
impl Node for ast::Operator {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let _source_file = source_file;
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
        singleton_node_to_object(vm, node_type)
    }

    fn ast_from_object(
        ctx: &VirtualMachine,
        _source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(
            if is_node_instance(ctx, &object, pyast::NodeOperatorAdd::static_type())? {
                Self::Add
            } else if is_node_instance(ctx, &object, pyast::NodeOperatorSub::static_type())? {
                Self::Sub
            } else if is_node_instance(ctx, &object, pyast::NodeOperatorMult::static_type())? {
                Self::Mult
            } else if is_node_instance(ctx, &object, pyast::NodeOperatorMatMult::static_type())? {
                Self::MatMult
            } else if is_node_instance(ctx, &object, pyast::NodeOperatorDiv::static_type())? {
                Self::Div
            } else if is_node_instance(ctx, &object, pyast::NodeOperatorMod::static_type())? {
                Self::Mod
            } else if is_node_instance(ctx, &object, pyast::NodeOperatorPow::static_type())? {
                Self::Pow
            } else if is_node_instance(ctx, &object, pyast::NodeOperatorLShift::static_type())? {
                Self::LShift
            } else if is_node_instance(ctx, &object, pyast::NodeOperatorRShift::static_type())? {
                Self::RShift
            } else if is_node_instance(ctx, &object, pyast::NodeOperatorBitOr::static_type())? {
                Self::BitOr
            } else if is_node_instance(ctx, &object, pyast::NodeOperatorBitXor::static_type())? {
                Self::BitXor
            } else if is_node_instance(ctx, &object, pyast::NodeOperatorBitAnd::static_type())? {
                Self::BitAnd
            } else if is_node_instance(ctx, &object, pyast::NodeOperatorFloorDiv::static_type())? {
                Self::FloorDiv
            } else {
                return Err(ctx.new_type_error(format!(
                    "expected some sort of operator, but got {}",
                    object.repr(ctx)?
                )));
            },
        )
    }
}

// sum
impl Node for ast::UnaryOp {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let _source_file = source_file;
        let node_type = match self {
            Self::Invert => pyast::NodeUnaryOpInvert::static_type(),
            Self::Not => pyast::NodeUnaryOpNot::static_type(),
            Self::UAdd => pyast::NodeUnaryOpUAdd::static_type(),
            Self::USub => pyast::NodeUnaryOpUSub::static_type(),
        };
        singleton_node_to_object(vm, node_type)
    }

    fn ast_from_object(
        ctx: &VirtualMachine,
        _source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(
            if is_node_instance(ctx, &object, pyast::NodeUnaryOpInvert::static_type())? {
                Self::Invert
            } else if is_node_instance(ctx, &object, pyast::NodeUnaryOpNot::static_type())? {
                Self::Not
            } else if is_node_instance(ctx, &object, pyast::NodeUnaryOpUAdd::static_type())? {
                Self::UAdd
            } else if is_node_instance(ctx, &object, pyast::NodeUnaryOpUSub::static_type())? {
                Self::USub
            } else {
                return Err(ctx.new_type_error(format!(
                    "expected some sort of unaryop, but got {}",
                    object.repr(ctx)?
                )));
            },
        )
    }
}

// sum
impl Node for ast::CmpOp {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let _source_file = source_file;
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
        singleton_node_to_object(vm, node_type)
    }

    fn ast_from_object(
        ctx: &VirtualMachine,
        _source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(
            if is_node_instance(ctx, &object, pyast::NodeCmpOpEq::static_type())? {
                Self::Eq
            } else if is_node_instance(ctx, &object, pyast::NodeCmpOpNotEq::static_type())? {
                Self::NotEq
            } else if is_node_instance(ctx, &object, pyast::NodeCmpOpLt::static_type())? {
                Self::Lt
            } else if is_node_instance(ctx, &object, pyast::NodeCmpOpLtE::static_type())? {
                Self::LtE
            } else if is_node_instance(ctx, &object, pyast::NodeCmpOpGt::static_type())? {
                Self::Gt
            } else if is_node_instance(ctx, &object, pyast::NodeCmpOpGtE::static_type())? {
                Self::GtE
            } else if is_node_instance(ctx, &object, pyast::NodeCmpOpIs::static_type())? {
                Self::Is
            } else if is_node_instance(ctx, &object, pyast::NodeCmpOpIsNot::static_type())? {
                Self::IsNot
            } else if is_node_instance(ctx, &object, pyast::NodeCmpOpIn::static_type())? {
                Self::In
            } else if is_node_instance(ctx, &object, pyast::NodeCmpOpNotIn::static_type())? {
                Self::NotIn
            } else {
                return Err(ctx.new_type_error(format!(
                    "expected some sort of cmpop, but got {}",
                    object.repr(ctx)?
                )));
            },
        )
    }
}
