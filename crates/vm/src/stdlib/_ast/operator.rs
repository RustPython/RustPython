use super::*;
use rustpython_compiler_core::SourceFile;

// sum
impl Node for ast::BoolOp {
    fn ast_to_object(self, vm: &VirtualMachine, _source_file: &SourceFile) -> PyObjectRef {
        let node_type = match self {
            Self::And => pyast::NodeBoolOpAnd::make_class(&vm.ctx),
            Self::Or => pyast::NodeBoolOpOr::make_class(&vm.ctx),
        };
        singleton_node_to_object(vm, node_type)
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        _source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(
            if is_node_instance(vm, &object, pyast::NodeBoolOpAnd::make_class(&vm.ctx))? {
                Self::And
            } else if is_node_instance(vm, &object, pyast::NodeBoolOpOr::make_class(&vm.ctx))? {
                Self::Or
            } else {
                return Err(vm.new_type_error(format!(
                    "expected some sort of boolop, but got {}",
                    object.repr(vm)?
                )));
            },
        )
    }
}

// sum
impl Node for ast::Operator {
    fn ast_to_object(self, vm: &VirtualMachine, _source_file: &SourceFile) -> PyObjectRef {
        let node_type = match self {
            Self::Add => pyast::NodeOperatorAdd::make_class(&vm.ctx),
            Self::Sub => pyast::NodeOperatorSub::make_class(&vm.ctx),
            Self::Mult => pyast::NodeOperatorMult::make_class(&vm.ctx),
            Self::MatMult => pyast::NodeOperatorMatMult::make_class(&vm.ctx),
            Self::Div => pyast::NodeOperatorDiv::make_class(&vm.ctx),
            Self::Mod => pyast::NodeOperatorMod::make_class(&vm.ctx),
            Self::Pow => pyast::NodeOperatorPow::make_class(&vm.ctx),
            Self::LShift => pyast::NodeOperatorLShift::make_class(&vm.ctx),
            Self::RShift => pyast::NodeOperatorRShift::make_class(&vm.ctx),
            Self::BitOr => pyast::NodeOperatorBitOr::make_class(&vm.ctx),
            Self::BitXor => pyast::NodeOperatorBitXor::make_class(&vm.ctx),
            Self::BitAnd => pyast::NodeOperatorBitAnd::make_class(&vm.ctx),
            Self::FloorDiv => pyast::NodeOperatorFloorDiv::make_class(&vm.ctx),
        };
        singleton_node_to_object(vm, node_type)
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        _source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(
            if is_node_instance(vm, &object, pyast::NodeOperatorAdd::make_class(&vm.ctx))? {
                Self::Add
            } else if is_node_instance(vm, &object, pyast::NodeOperatorSub::make_class(&vm.ctx))? {
                Self::Sub
            } else if is_node_instance(vm, &object, pyast::NodeOperatorMult::make_class(&vm.ctx))? {
                Self::Mult
            } else if is_node_instance(
                vm,
                &object,
                pyast::NodeOperatorMatMult::make_class(&vm.ctx),
            )? {
                Self::MatMult
            } else if is_node_instance(vm, &object, pyast::NodeOperatorDiv::make_class(&vm.ctx))? {
                Self::Div
            } else if is_node_instance(vm, &object, pyast::NodeOperatorMod::make_class(&vm.ctx))? {
                Self::Mod
            } else if is_node_instance(vm, &object, pyast::NodeOperatorPow::make_class(&vm.ctx))? {
                Self::Pow
            } else if is_node_instance(vm, &object, pyast::NodeOperatorLShift::make_class(&vm.ctx))?
            {
                Self::LShift
            } else if is_node_instance(vm, &object, pyast::NodeOperatorRShift::make_class(&vm.ctx))?
            {
                Self::RShift
            } else if is_node_instance(vm, &object, pyast::NodeOperatorBitOr::make_class(&vm.ctx))?
            {
                Self::BitOr
            } else if is_node_instance(vm, &object, pyast::NodeOperatorBitXor::make_class(&vm.ctx))?
            {
                Self::BitXor
            } else if is_node_instance(vm, &object, pyast::NodeOperatorBitAnd::make_class(&vm.ctx))?
            {
                Self::BitAnd
            } else if is_node_instance(
                vm,
                &object,
                pyast::NodeOperatorFloorDiv::make_class(&vm.ctx),
            )? {
                Self::FloorDiv
            } else {
                return Err(vm.new_type_error(format!(
                    "expected some sort of operator, but got {}",
                    object.repr(vm)?
                )));
            },
        )
    }
}

// sum
impl Node for ast::UnaryOp {
    fn ast_to_object(self, vm: &VirtualMachine, _source_file: &SourceFile) -> PyObjectRef {
        let node_type = match self {
            Self::Invert => pyast::NodeUnaryOpInvert::make_class(&vm.ctx),
            Self::Not => pyast::NodeUnaryOpNot::make_class(&vm.ctx),
            Self::UAdd => pyast::NodeUnaryOpUAdd::make_class(&vm.ctx),
            Self::USub => pyast::NodeUnaryOpUSub::make_class(&vm.ctx),
        };
        singleton_node_to_object(vm, node_type)
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        _source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(
            if is_node_instance(vm, &object, pyast::NodeUnaryOpInvert::make_class(&vm.ctx))? {
                Self::Invert
            } else if is_node_instance(vm, &object, pyast::NodeUnaryOpNot::make_class(&vm.ctx))? {
                Self::Not
            } else if is_node_instance(vm, &object, pyast::NodeUnaryOpUAdd::make_class(&vm.ctx))? {
                Self::UAdd
            } else if is_node_instance(vm, &object, pyast::NodeUnaryOpUSub::make_class(&vm.ctx))? {
                Self::USub
            } else {
                return Err(vm.new_type_error(format!(
                    "expected some sort of unaryop, but got {}",
                    object.repr(vm)?
                )));
            },
        )
    }
}

// sum
impl Node for ast::CmpOp {
    fn ast_to_object(self, vm: &VirtualMachine, _source_file: &SourceFile) -> PyObjectRef {
        let node_type = match self {
            Self::Eq => pyast::NodeCmpOpEq::make_class(&vm.ctx),
            Self::NotEq => pyast::NodeCmpOpNotEq::make_class(&vm.ctx),
            Self::Lt => pyast::NodeCmpOpLt::make_class(&vm.ctx),
            Self::LtE => pyast::NodeCmpOpLtE::make_class(&vm.ctx),
            Self::Gt => pyast::NodeCmpOpGt::make_class(&vm.ctx),
            Self::GtE => pyast::NodeCmpOpGtE::make_class(&vm.ctx),
            Self::Is => pyast::NodeCmpOpIs::make_class(&vm.ctx),
            Self::IsNot => pyast::NodeCmpOpIsNot::make_class(&vm.ctx),
            Self::In => pyast::NodeCmpOpIn::make_class(&vm.ctx),
            Self::NotIn => pyast::NodeCmpOpNotIn::make_class(&vm.ctx),
        };
        singleton_node_to_object(vm, node_type)
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        _source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(
            if is_node_instance(vm, &object, pyast::NodeCmpOpEq::make_class(&vm.ctx))? {
                Self::Eq
            } else if is_node_instance(vm, &object, pyast::NodeCmpOpNotEq::make_class(&vm.ctx))? {
                Self::NotEq
            } else if is_node_instance(vm, &object, pyast::NodeCmpOpLt::make_class(&vm.ctx))? {
                Self::Lt
            } else if is_node_instance(vm, &object, pyast::NodeCmpOpLtE::make_class(&vm.ctx))? {
                Self::LtE
            } else if is_node_instance(vm, &object, pyast::NodeCmpOpGt::make_class(&vm.ctx))? {
                Self::Gt
            } else if is_node_instance(vm, &object, pyast::NodeCmpOpGtE::make_class(&vm.ctx))? {
                Self::GtE
            } else if is_node_instance(vm, &object, pyast::NodeCmpOpIs::make_class(&vm.ctx))? {
                Self::Is
            } else if is_node_instance(vm, &object, pyast::NodeCmpOpIsNot::make_class(&vm.ctx))? {
                Self::IsNot
            } else if is_node_instance(vm, &object, pyast::NodeCmpOpIn::make_class(&vm.ctx))? {
                Self::In
            } else if is_node_instance(vm, &object, pyast::NodeCmpOpNotIn::make_class(&vm.ctx))? {
                Self::NotIn
            } else {
                return Err(vm.new_type_error(format!(
                    "expected some sort of cmpop, but got {}",
                    object.repr(vm)?
                )));
            },
        )
    }
}
