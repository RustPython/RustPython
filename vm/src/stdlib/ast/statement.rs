use super::*;
use crate::stdlib::ast::argument::{merge_class_def_args, split_class_def_args};
// sum
impl Node for ruff::Stmt {
    fn ast_to_object(self, vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        match self {
            ruff::Stmt::FunctionDef(cons) => cons.ast_to_object(vm, source_code),
            ruff::Stmt::ClassDef(cons) => cons.ast_to_object(vm, source_code),
            ruff::Stmt::Return(cons) => cons.ast_to_object(vm, source_code),
            ruff::Stmt::Delete(cons) => cons.ast_to_object(vm, source_code),
            ruff::Stmt::Assign(cons) => cons.ast_to_object(vm, source_code),
            ruff::Stmt::TypeAlias(cons) => cons.ast_to_object(vm, source_code),
            ruff::Stmt::AugAssign(cons) => cons.ast_to_object(vm, source_code),
            ruff::Stmt::AnnAssign(cons) => cons.ast_to_object(vm, source_code),
            ruff::Stmt::For(cons) => cons.ast_to_object(vm, source_code),
            ruff::Stmt::While(cons) => cons.ast_to_object(vm, source_code),
            ruff::Stmt::If(cons) => cons.ast_to_object(vm, source_code),
            ruff::Stmt::With(cons) => cons.ast_to_object(vm, source_code),
            ruff::Stmt::Match(cons) => cons.ast_to_object(vm, source_code),
            ruff::Stmt::Raise(cons) => cons.ast_to_object(vm, source_code),
            ruff::Stmt::Try(cons) => cons.ast_to_object(vm, source_code),
            ruff::Stmt::Assert(cons) => cons.ast_to_object(vm, source_code),
            ruff::Stmt::Import(cons) => cons.ast_to_object(vm, source_code),
            ruff::Stmt::ImportFrom(cons) => cons.ast_to_object(vm, source_code),
            ruff::Stmt::Global(cons) => cons.ast_to_object(vm, source_code),
            ruff::Stmt::Nonlocal(cons) => cons.ast_to_object(vm, source_code),
            ruff::Stmt::Expr(cons) => cons.ast_to_object(vm, source_code),
            ruff::Stmt::Pass(cons) => cons.ast_to_object(vm, source_code),
            ruff::Stmt::Break(cons) => cons.ast_to_object(vm, source_code),
            ruff::Stmt::Continue(cons) => cons.ast_to_object(vm, source_code),
            ruff::Stmt::IpyEscapeCommand(_) => todo!(),
        }
    }

    #[allow(clippy::if_same_then_else)]
    fn ast_from_object(
        _vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        let _cls = _object.class();
        Ok(if _cls.is(gen::NodeStmtFunctionDef::static_type()) {
            ruff::Stmt::FunctionDef(ruff::StmtFunctionDef::ast_from_object(
                _vm,
                source_code,
                _object,
            )?)
        } else if _cls.is(gen::NodeStmtAsyncFunctionDef::static_type()) {
            ruff::Stmt::FunctionDef(ruff::StmtFunctionDef::ast_from_object(
                _vm,
                source_code,
                _object,
            )?)
        } else if _cls.is(gen::NodeStmtClassDef::static_type()) {
            ruff::Stmt::ClassDef(ruff::StmtClassDef::ast_from_object(
                _vm,
                source_code,
                _object,
            )?)
        } else if _cls.is(gen::NodeStmtReturn::static_type()) {
            ruff::Stmt::Return(ruff::StmtReturn::ast_from_object(
                _vm,
                source_code,
                _object,
            )?)
        } else if _cls.is(gen::NodeStmtDelete::static_type()) {
            ruff::Stmt::Delete(ruff::StmtDelete::ast_from_object(
                _vm,
                source_code,
                _object,
            )?)
        } else if _cls.is(gen::NodeStmtAssign::static_type()) {
            ruff::Stmt::Assign(ruff::StmtAssign::ast_from_object(
                _vm,
                source_code,
                _object,
            )?)
        } else if _cls.is(gen::NodeStmtTypeAlias::static_type()) {
            ruff::Stmt::TypeAlias(ruff::StmtTypeAlias::ast_from_object(
                _vm,
                source_code,
                _object,
            )?)
        } else if _cls.is(gen::NodeStmtAugAssign::static_type()) {
            ruff::Stmt::AugAssign(ruff::StmtAugAssign::ast_from_object(
                _vm,
                source_code,
                _object,
            )?)
        } else if _cls.is(gen::NodeStmtAnnAssign::static_type()) {
            ruff::Stmt::AnnAssign(ruff::StmtAnnAssign::ast_from_object(
                _vm,
                source_code,
                _object,
            )?)
        } else if _cls.is(gen::NodeStmtFor::static_type()) {
            ruff::Stmt::For(ruff::StmtFor::ast_from_object(_vm, source_code, _object)?)
        } else if _cls.is(gen::NodeStmtAsyncFor::static_type()) {
            ruff::Stmt::For(ruff::StmtFor::ast_from_object(_vm, source_code, _object)?)
        } else if _cls.is(gen::NodeStmtWhile::static_type()) {
            ruff::Stmt::While(ruff::StmtWhile::ast_from_object(_vm, source_code, _object)?)
        } else if _cls.is(gen::NodeStmtIf::static_type()) {
            ruff::Stmt::If(ruff::StmtIf::ast_from_object(_vm, source_code, _object)?)
        } else if _cls.is(gen::NodeStmtWith::static_type()) {
            ruff::Stmt::With(ruff::StmtWith::ast_from_object(_vm, source_code, _object)?)
        } else if _cls.is(gen::NodeStmtAsyncWith::static_type()) {
            ruff::Stmt::With(ruff::StmtWith::ast_from_object(_vm, source_code, _object)?)
        } else if _cls.is(gen::NodeStmtMatch::static_type()) {
            ruff::Stmt::Match(ruff::StmtMatch::ast_from_object(_vm, source_code, _object)?)
        } else if _cls.is(gen::NodeStmtRaise::static_type()) {
            ruff::Stmt::Raise(ruff::StmtRaise::ast_from_object(_vm, source_code, _object)?)
        } else if _cls.is(gen::NodeStmtTry::static_type()) {
            ruff::Stmt::Try(ruff::StmtTry::ast_from_object(_vm, source_code, _object)?)
        } else if _cls.is(gen::NodeStmtTryStar::static_type()) {
            ruff::Stmt::Try(ruff::StmtTry::ast_from_object(_vm, source_code, _object)?)
        } else if _cls.is(gen::NodeStmtAssert::static_type()) {
            ruff::Stmt::Assert(ruff::StmtAssert::ast_from_object(
                _vm,
                source_code,
                _object,
            )?)
        } else if _cls.is(gen::NodeStmtImport::static_type()) {
            ruff::Stmt::Import(ruff::StmtImport::ast_from_object(
                _vm,
                source_code,
                _object,
            )?)
        } else if _cls.is(gen::NodeStmtImportFrom::static_type()) {
            ruff::Stmt::ImportFrom(ruff::StmtImportFrom::ast_from_object(
                _vm,
                source_code,
                _object,
            )?)
        } else if _cls.is(gen::NodeStmtGlobal::static_type()) {
            ruff::Stmt::Global(ruff::StmtGlobal::ast_from_object(
                _vm,
                source_code,
                _object,
            )?)
        } else if _cls.is(gen::NodeStmtNonlocal::static_type()) {
            ruff::Stmt::Nonlocal(ruff::StmtNonlocal::ast_from_object(
                _vm,
                source_code,
                _object,
            )?)
        } else if _cls.is(gen::NodeStmtExpr::static_type()) {
            ruff::Stmt::Expr(ruff::StmtExpr::ast_from_object(_vm, source_code, _object)?)
        } else if _cls.is(gen::NodeStmtPass::static_type()) {
            ruff::Stmt::Pass(ruff::StmtPass::ast_from_object(_vm, source_code, _object)?)
        } else if _cls.is(gen::NodeStmtBreak::static_type()) {
            ruff::Stmt::Break(ruff::StmtBreak::ast_from_object(_vm, source_code, _object)?)
        } else if _cls.is(gen::NodeStmtContinue::static_type()) {
            ruff::Stmt::Continue(ruff::StmtContinue::ast_from_object(
                _vm,
                source_code,
                _object,
            )?)
        } else {
            return Err(_vm.new_type_error(format!(
                "expected some sort of stmt, but got {}",
                _object.repr(_vm)?
            )));
        })
    }
}
// constructor
impl Node for ruff::StmtFunctionDef {
    fn ast_to_object(self, vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let Self {
            name,
            parameters,
            body,
            decorator_list,
            returns,
            // type_comment,
            type_params,
            is_async,
            range: _range,
        } = self;

        let cls = if !is_async {
            gen::NodeStmtFunctionDef::static_type().to_owned()
        } else {
            gen::NodeStmtAsyncFunctionDef::static_type().to_owned()
        };

        let node = NodeAst.into_ref_with_type(vm, cls).unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("name", vm.ctx.new_str(name.as_str()).to_pyobject(vm), vm)
            .unwrap();
        dict.set_item("args", parameters.ast_to_object(vm, source_code), vm)
            .unwrap();
        dict.set_item("body", body.ast_to_object(vm, source_code), vm)
            .unwrap();
        dict.set_item(
            "decorator_list",
            decorator_list.ast_to_object(vm, source_code),
            vm,
        )
        .unwrap();
        dict.set_item("returns", returns.ast_to_object(vm, source_code), vm)
            .unwrap();
        // TODO: Ruff ignores type_comment during parsing
        // dict.set_item("type_comment", type_comment.ast_to_object(_vm), _vm)
        //     .unwrap();
        dict.set_item(
            "type_params",
            type_params.ast_to_object(vm, source_code),
            vm,
        )
        .unwrap();
        node_add_location(&dict, _range, vm, source_code);
        node.into()
    }
    fn ast_from_object(
        _vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        let _cls = _object.class();
        let is_async = _cls.is(gen::NodeStmtAsyncFunctionDef::static_type());
        Ok(Self {
            name: Node::ast_from_object(
                _vm,
                source_code,
                get_node_field(_vm, &_object, "name", "FunctionDef")?,
            )?,
            parameters: Node::ast_from_object(
                _vm,
                source_code,
                get_node_field(_vm, &_object, "args", "FunctionDef")?,
            )?,
            body: Node::ast_from_object(
                _vm,
                source_code,
                get_node_field(_vm, &_object, "body", "FunctionDef")?,
            )?,
            decorator_list: Node::ast_from_object(
                _vm,
                source_code,
                get_node_field(_vm, &_object, "decorator_list", "FunctionDef")?,
            )?,
            returns: get_node_field_opt(_vm, &_object, "returns")?
                .map(|obj| Node::ast_from_object(_vm, source_code, obj))
                .transpose()?,
            // TODO: Ruff ignores type_comment during parsing
            // type_comment: get_node_field_opt(_vm, &_object, "type_comment")?
            //     .map(|obj| Node::ast_from_object(_vm, obj))
            //     .transpose()?,
            type_params: Node::ast_from_object(
                _vm,
                source_code,
                get_node_field(_vm, &_object, "type_params", "FunctionDef")?,
            )?,
            range: range_from_object(_vm, source_code, _object, "FunctionDef")?,
            is_async,
        })
    }
}
// constructor
impl Node for ruff::StmtClassDef {
    fn ast_to_object(self, _vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let Self {
            name,
            arguments,
            body,
            decorator_list,
            type_params,
            range: _range,
        } = self;
        let (bases, keywords) = split_class_def_args(arguments);
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeStmtClassDef::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("name", name.ast_to_object(_vm, source_code), _vm)
            .unwrap();
        dict.set_item("bases", bases.ast_to_object(_vm, source_code), _vm)
            .unwrap();
        dict.set_item("keywords", keywords.ast_to_object(_vm, source_code), _vm)
            .unwrap();
        dict.set_item("body", body.ast_to_object(_vm, source_code), _vm)
            .unwrap();
        dict.set_item(
            "decorator_list",
            decorator_list.ast_to_object(_vm, source_code),
            _vm,
        )
        .unwrap();
        dict.set_item(
            "type_params",
            type_params.ast_to_object(_vm, source_code),
            _vm,
        )
        .unwrap();
        node_add_location(&dict, _range, _vm, source_code);
        node.into()
    }
    fn ast_from_object(
        _vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        let bases = Node::ast_from_object(
            _vm,
            source_code,
            get_node_field(_vm, &_object, "bases", "ClassDef")?,
        )?;
        let keywords = Node::ast_from_object(
            _vm,
            source_code,
            get_node_field(_vm, &_object, "keywords", "ClassDef")?,
        )?;
        Ok(Self {
            name: Node::ast_from_object(
                _vm,
                source_code,
                get_node_field(_vm, &_object, "name", "ClassDef")?,
            )?,
            arguments: merge_class_def_args(bases, keywords),
            body: Node::ast_from_object(
                _vm,
                source_code,
                get_node_field(_vm, &_object, "body", "ClassDef")?,
            )?,
            decorator_list: Node::ast_from_object(
                _vm,
                source_code,
                get_node_field(_vm, &_object, "decorator_list", "ClassDef")?,
            )?,
            type_params: Node::ast_from_object(
                _vm,
                source_code,
                get_node_field(_vm, &_object, "type_params", "ClassDef")?,
            )?,
            range: range_from_object(_vm, source_code, _object, "ClassDef")?,
        })
    }
}
// constructor
impl Node for ruff::StmtReturn {
    fn ast_to_object(self, _vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let ruff::StmtReturn {
            value,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeStmtReturn::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(_vm, source_code), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm, source_code);
        node.into()
    }
    fn ast_from_object(
        _vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(ruff::StmtReturn {
            value: get_node_field_opt(_vm, &_object, "value")?
                .map(|obj| Node::ast_from_object(_vm, source_code, obj))
                .transpose()?,
            range: range_from_object(_vm, source_code, _object, "Return")?,
        })
    }
}
// constructor
impl Node for ruff::StmtDelete {
    fn ast_to_object(self, _vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let ruff::StmtDelete {
            targets,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeStmtDelete::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("targets", targets.ast_to_object(_vm, source_code), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm, source_code);
        node.into()
    }
    fn ast_from_object(
        _vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(ruff::StmtDelete {
            targets: Node::ast_from_object(
                _vm,
                source_code,
                get_node_field(_vm, &_object, "targets", "Delete")?,
            )?,
            range: range_from_object(_vm, source_code, _object, "Delete")?,
        })
    }
}
// constructor
impl Node for ruff::StmtAssign {
    fn ast_to_object(self, vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let Self {
            targets,
            value,
            // type_comment,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, gen::NodeStmtAssign::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("targets", targets.ast_to_object(vm, source_code), vm)
            .unwrap();
        dict.set_item("value", value.ast_to_object(vm, source_code), vm)
            .unwrap();
        // TODO
        dict.set_item("type_comment", vm.ctx.none(), vm).unwrap();
        node_add_location(&dict, range, vm, source_code);
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            targets: Node::ast_from_object(
                vm,
                source_code,
                get_node_field(vm, &object, "targets", "Assign")?,
            )?,
            value: Node::ast_from_object(
                vm,
                source_code,
                get_node_field(vm, &object, "value", "Assign")?,
            )?,
            // type_comment: get_node_field_opt(_vm, &_object, "type_comment")?
            //     .map(|obj| Node::ast_from_object(_vm, obj))
            //     .transpose()?,
            range: range_from_object(vm, source_code, object, "Assign")?,
        })
    }
}
// constructor
impl Node for ruff::StmtTypeAlias {
    fn ast_to_object(self, _vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let ruff::StmtTypeAlias {
            name,
            type_params,
            value,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeStmtTypeAlias::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("name", name.ast_to_object(_vm, source_code), _vm)
            .unwrap();
        dict.set_item(
            "type_params",
            type_params.ast_to_object(_vm, source_code),
            _vm,
        )
        .unwrap();
        dict.set_item("value", value.ast_to_object(_vm, source_code), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm, source_code);
        node.into()
    }
    fn ast_from_object(
        _vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(ruff::StmtTypeAlias {
            name: Node::ast_from_object(
                _vm,
                source_code,
                get_node_field(_vm, &_object, "name", "TypeAlias")?,
            )?,
            type_params: Node::ast_from_object(
                _vm,
                source_code,
                get_node_field(_vm, &_object, "type_params", "TypeAlias")?,
            )?,
            value: Node::ast_from_object(
                _vm,
                source_code,
                get_node_field(_vm, &_object, "value", "TypeAlias")?,
            )?,
            range: range_from_object(_vm, source_code, _object, "TypeAlias")?,
        })
    }
}
// constructor
impl Node for ruff::StmtAugAssign {
    fn ast_to_object(self, _vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let Self {
            target,
            op,
            value,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeStmtAugAssign::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("target", target.ast_to_object(_vm, source_code), _vm)
            .unwrap();
        dict.set_item("op", op.ast_to_object(_vm, source_code), _vm)
            .unwrap();
        dict.set_item("value", value.ast_to_object(_vm, source_code), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm, source_code);
        node.into()
    }
    fn ast_from_object(
        _vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            target: Node::ast_from_object(
                _vm,
                source_code,
                get_node_field(_vm, &_object, "target", "AugAssign")?,
            )?,
            op: Node::ast_from_object(
                _vm,
                source_code,
                get_node_field(_vm, &_object, "op", "AugAssign")?,
            )?,
            value: Node::ast_from_object(
                _vm,
                source_code,
                get_node_field(_vm, &_object, "value", "AugAssign")?,
            )?,
            range: range_from_object(_vm, source_code, _object, "AugAssign")?,
        })
    }
}
// constructor
impl Node for ruff::StmtAnnAssign {
    fn ast_to_object(self, _vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let Self {
            target,
            annotation,
            value,
            simple,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeStmtAnnAssign::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("target", target.ast_to_object(_vm, source_code), _vm)
            .unwrap();
        dict.set_item(
            "annotation",
            annotation.ast_to_object(_vm, source_code),
            _vm,
        )
        .unwrap();
        dict.set_item("value", value.ast_to_object(_vm, source_code), _vm)
            .unwrap();
        dict.set_item("simple", simple.ast_to_object(_vm, source_code), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm, source_code);
        node.into()
    }
    fn ast_from_object(
        _vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            target: Node::ast_from_object(
                _vm,
                source_code,
                get_node_field(_vm, &_object, "target", "AnnAssign")?,
            )?,
            annotation: Node::ast_from_object(
                _vm,
                source_code,
                get_node_field(_vm, &_object, "annotation", "AnnAssign")?,
            )?,
            value: get_node_field_opt(_vm, &_object, "value")?
                .map(|obj| Node::ast_from_object(_vm, source_code, obj))
                .transpose()?,
            simple: Node::ast_from_object(
                _vm,
                source_code,
                get_node_field(_vm, &_object, "simple", "AnnAssign")?,
            )?,
            range: range_from_object(_vm, source_code, _object, "AnnAssign")?,
        })
    }
}
// constructor
impl Node for ruff::StmtFor {
    fn ast_to_object(self, _vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let Self {
            is_async,
            target,
            iter,
            body,
            orelse,
            // type_comment,
            range: _range,
        } = self;

        let cls = if !is_async {
            gen::NodeStmtFor::static_type().to_owned()
        } else {
            gen::NodeStmtAsyncFor::static_type().to_owned()
        };

        let node = NodeAst.into_ref_with_type(_vm, cls).unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("target", target.ast_to_object(_vm, source_code), _vm)
            .unwrap();
        dict.set_item("iter", iter.ast_to_object(_vm, source_code), _vm)
            .unwrap();
        dict.set_item("body", body.ast_to_object(_vm, source_code), _vm)
            .unwrap();
        dict.set_item("orelse", orelse.ast_to_object(_vm, source_code), _vm)
            .unwrap();
        // dict.set_item("type_comment", type_comment.ast_to_object(_vm), _vm)
        //     .unwrap();
        node_add_location(&dict, _range, _vm, source_code);
        node.into()
    }
    fn ast_from_object(
        _vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        let _cls = _object.class();
        debug_assert!(
            _cls.is(gen::NodeStmtFor::static_type())
                || _cls.is(gen::NodeStmtAsyncFor::static_type())
        );
        let is_async = _cls.is(gen::NodeStmtAsyncFor::static_type());
        Ok(Self {
            target: Node::ast_from_object(
                _vm,
                source_code,
                get_node_field(_vm, &_object, "target", "For")?,
            )?,
            iter: Node::ast_from_object(
                _vm,
                source_code,
                get_node_field(_vm, &_object, "iter", "For")?,
            )?,
            body: Node::ast_from_object(
                _vm,
                source_code,
                get_node_field(_vm, &_object, "body", "For")?,
            )?,
            orelse: Node::ast_from_object(
                _vm,
                source_code,
                get_node_field(_vm, &_object, "orelse", "For")?,
            )?,
            // type_comment: get_node_field_opt(_vm, &_object, "type_comment")?
            //     .map(|obj| Node::ast_from_object(_vm, obj))
            //     .transpose()?,
            range: range_from_object(_vm, source_code, _object, "For")?,
            is_async,
        })
    }
}
// constructor
impl Node for ruff::StmtWhile {
    fn ast_to_object(self, _vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let Self {
            test,
            body,
            orelse,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeStmtWhile::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("test", test.ast_to_object(_vm, source_code), _vm)
            .unwrap();
        dict.set_item("body", body.ast_to_object(_vm, source_code), _vm)
            .unwrap();
        dict.set_item("orelse", orelse.ast_to_object(_vm, source_code), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm, source_code);
        node.into()
    }
    fn ast_from_object(
        _vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            test: Node::ast_from_object(
                _vm,
                source_code,
                get_node_field(_vm, &_object, "test", "While")?,
            )?,
            body: Node::ast_from_object(
                _vm,
                source_code,
                get_node_field(_vm, &_object, "body", "While")?,
            )?,
            orelse: Node::ast_from_object(
                _vm,
                source_code,
                get_node_field(_vm, &_object, "orelse", "While")?,
            )?,
            range: range_from_object(_vm, source_code, _object, "While")?,
        })
    }
}
// constructor
impl Node for ruff::StmtIf {
    fn ast_to_object(self, _vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let Self {
            test,
            body,
            range,
            elif_else_clauses,
        } = self;
        elif_else_clause::ast_to_object(
            ruff::ElifElseClause {
                range,
                test: Some(*test),
                body,
            },
            elif_else_clauses.into_iter(),
            _vm,
            source_code,
        )
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        elif_else_clause::ast_from_object(vm, source_code, object)
    }
}
// constructor
impl Node for ruff::StmtWith {
    fn ast_to_object(self, _vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let Self {
            is_async,
            items,
            body,
            // type_comment,
            range: _range,
        } = self;

        let cls = if !is_async {
            gen::NodeStmtWith::static_type().to_owned()
        } else {
            gen::NodeStmtAsyncWith::static_type().to_owned()
        };

        let node = NodeAst.into_ref_with_type(_vm, cls).unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("items", items.ast_to_object(_vm, source_code), _vm)
            .unwrap();
        dict.set_item("body", body.ast_to_object(_vm, source_code), _vm)
            .unwrap();
        // dict.set_item("type_comment", type_comment.ast_to_object(_vm), _vm)
        //     .unwrap();
        node_add_location(&dict, _range, _vm, source_code);
        node.into()
    }
    fn ast_from_object(
        _vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        let _cls = _object.class();
        debug_assert!(
            _cls.is(gen::NodeStmtWith::static_type())
                || _cls.is(gen::NodeStmtAsyncWith::static_type())
        );
        let is_async = _cls.is(gen::NodeStmtAsyncWith::static_type());
        Ok(Self {
            items: Node::ast_from_object(
                _vm,
                source_code,
                get_node_field(_vm, &_object, "items", "With")?,
            )?,
            body: Node::ast_from_object(
                _vm,
                source_code,
                get_node_field(_vm, &_object, "body", "With")?,
            )?,
            // type_comment: get_node_field_opt(_vm, &_object, "type_comment")?
            //     .map(|obj| Node::ast_from_object(_vm, obj))
            //     .transpose()?,
            range: range_from_object(_vm, source_code, _object, "With")?,
            is_async,
        })
    }
}
// constructor
impl Node for ruff::StmtMatch {
    fn ast_to_object(self, _vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let Self {
            subject,
            cases,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeStmtMatch::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("subject", subject.ast_to_object(_vm, source_code), _vm)
            .unwrap();
        dict.set_item("cases", cases.ast_to_object(_vm, source_code), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm, source_code);
        node.into()
    }
    fn ast_from_object(
        _vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            subject: Node::ast_from_object(
                _vm,
                source_code,
                get_node_field(_vm, &_object, "subject", "Match")?,
            )?,
            cases: Node::ast_from_object(
                _vm,
                source_code,
                get_node_field(_vm, &_object, "cases", "Match")?,
            )?,
            range: range_from_object(_vm, source_code, _object, "Match")?,
        })
    }
}
// constructor
impl Node for ruff::StmtRaise {
    fn ast_to_object(self, _vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let Self {
            exc,
            cause,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeStmtRaise::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("exc", exc.ast_to_object(_vm, source_code), _vm)
            .unwrap();
        dict.set_item("cause", cause.ast_to_object(_vm, source_code), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm, source_code);
        node.into()
    }
    fn ast_from_object(
        _vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            exc: get_node_field_opt(_vm, &_object, "exc")?
                .map(|obj| Node::ast_from_object(_vm, source_code, obj))
                .transpose()?,
            cause: get_node_field_opt(_vm, &_object, "cause")?
                .map(|obj| Node::ast_from_object(_vm, source_code, obj))
                .transpose()?,
            range: range_from_object(_vm, source_code, _object, "Raise")?,
        })
    }
}
// constructor
impl Node for ruff::StmtTry {
    fn ast_to_object(self, _vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let Self {
            body,
            handlers,
            orelse,
            finalbody,
            range: _range,
            is_star,
        } = self;

        // let cls = gen::NodeStmtTry::static_type().to_owned();
        let cls = if is_star {
            gen::NodeStmtTryStar::static_type()
        } else {
            gen::NodeStmtTry::static_type()
        }
        .to_owned();

        let node = NodeAst.into_ref_with_type(_vm, cls).unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("body", body.ast_to_object(_vm, source_code), _vm)
            .unwrap();
        dict.set_item("handlers", handlers.ast_to_object(_vm, source_code), _vm)
            .unwrap();
        dict.set_item("orelse", orelse.ast_to_object(_vm, source_code), _vm)
            .unwrap();
        dict.set_item("finalbody", finalbody.ast_to_object(_vm, source_code), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm, source_code);
        node.into()
    }
    fn ast_from_object(
        _vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        let _cls = _object.class();
        let is_star = _cls.is(gen::NodeStmtTryStar::static_type());
        let _cls = _object.class();
        debug_assert!(
            _cls.is(gen::NodeStmtTry::static_type())
                || _cls.is(gen::NodeStmtTryStar::static_type())
        );

        Ok(Self {
            body: Node::ast_from_object(
                _vm,
                source_code,
                get_node_field(_vm, &_object, "body", "Try")?,
            )?,
            handlers: Node::ast_from_object(
                _vm,
                source_code,
                get_node_field(_vm, &_object, "handlers", "Try")?,
            )?,
            orelse: Node::ast_from_object(
                _vm,
                source_code,
                get_node_field(_vm, &_object, "orelse", "Try")?,
            )?,
            finalbody: Node::ast_from_object(
                _vm,
                source_code,
                get_node_field(_vm, &_object, "finalbody", "Try")?,
            )?,
            range: range_from_object(_vm, source_code, _object, "Try")?,
            is_star,
        })
    }
}
// constructor
impl Node for ruff::StmtAssert {
    fn ast_to_object(self, _vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let ruff::StmtAssert {
            test,
            msg,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeStmtAssert::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("test", test.ast_to_object(_vm, source_code), _vm)
            .unwrap();
        dict.set_item("msg", msg.ast_to_object(_vm, source_code), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm, source_code);
        node.into()
    }
    fn ast_from_object(
        _vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(ruff::StmtAssert {
            test: Node::ast_from_object(
                _vm,
                source_code,
                get_node_field(_vm, &_object, "test", "Assert")?,
            )?,
            msg: get_node_field_opt(_vm, &_object, "msg")?
                .map(|obj| Node::ast_from_object(_vm, source_code, obj))
                .transpose()?,
            range: range_from_object(_vm, source_code, _object, "Assert")?,
        })
    }
}
// constructor
impl Node for ruff::StmtImport {
    fn ast_to_object(self, _vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let ruff::StmtImport {
            names,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeStmtImport::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("names", names.ast_to_object(_vm, source_code), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm, source_code);
        node.into()
    }
    fn ast_from_object(
        _vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(ruff::StmtImport {
            names: Node::ast_from_object(
                _vm,
                source_code,
                get_node_field(_vm, &_object, "names", "Import")?,
            )?,
            range: range_from_object(_vm, source_code, _object, "Import")?,
        })
    }
}
// constructor
impl Node for ruff::StmtImportFrom {
    fn ast_to_object(self, vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let Self {
            module,
            names,
            level,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, gen::NodeStmtImportFrom::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("module", module.ast_to_object(vm, source_code), vm)
            .unwrap();
        dict.set_item("names", names.ast_to_object(vm, source_code), vm)
            .unwrap();
        dict.set_item("level", vm.ctx.new_int(level).to_pyobject(vm), vm)
            .unwrap();
        node_add_location(&dict, range, vm, source_code);
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            module: get_node_field_opt(vm, &_object, "module")?
                .map(|obj| Node::ast_from_object(vm, source_code, obj))
                .transpose()?,
            names: Node::ast_from_object(
                vm,
                source_code,
                get_node_field(vm, &_object, "names", "ImportFrom")?,
            )?,
            level: get_node_field(vm, &_object, "level", "ImportFrom")?
                .downcast_exact::<PyInt>(vm)
                .unwrap()
                .try_to_primitive::<u32>(vm)?,
            range: range_from_object(vm, source_code, _object, "ImportFrom")?,
        })
    }
}
// constructor
impl Node for ruff::StmtGlobal {
    fn ast_to_object(self, _vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let ruff::StmtGlobal {
            names,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeStmtGlobal::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("names", names.ast_to_object(_vm, source_code), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm, source_code);
        node.into()
    }
    fn ast_from_object(
        _vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(ruff::StmtGlobal {
            names: Node::ast_from_object(
                _vm,
                source_code,
                get_node_field(_vm, &_object, "names", "Global")?,
            )?,
            range: range_from_object(_vm, source_code, _object, "Global")?,
        })
    }
}
// constructor
impl Node for ruff::StmtNonlocal {
    fn ast_to_object(self, _vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let ruff::StmtNonlocal {
            names,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeStmtNonlocal::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("names", names.ast_to_object(_vm, source_code), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm, source_code);
        node.into()
    }
    fn ast_from_object(
        _vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(ruff::StmtNonlocal {
            names: Node::ast_from_object(
                _vm,
                source_code,
                get_node_field(_vm, &_object, "names", "Nonlocal")?,
            )?,
            range: range_from_object(_vm, source_code, _object, "Nonlocal")?,
        })
    }
}
// constructor
impl Node for ruff::StmtExpr {
    fn ast_to_object(self, _vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let ruff::StmtExpr {
            value,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeStmtExpr::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(_vm, source_code), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm, source_code);
        node.into()
    }
    fn ast_from_object(
        _vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(ruff::StmtExpr {
            value: Node::ast_from_object(
                _vm,
                source_code,
                get_node_field(_vm, &_object, "value", "Expr")?,
            )?,
            range: range_from_object(_vm, source_code, _object, "Expr")?,
        })
    }
}
// constructor
impl Node for ruff::StmtPass {
    fn ast_to_object(self, _vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let ruff::StmtPass { range: _range } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeStmtPass::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        node_add_location(&dict, _range, _vm, source_code);
        node.into()
    }
    fn ast_from_object(
        _vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(ruff::StmtPass {
            range: range_from_object(_vm, source_code, _object, "Pass")?,
        })
    }
}
// constructor
impl Node for ruff::StmtBreak {
    fn ast_to_object(self, _vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let ruff::StmtBreak { range: _range } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeStmtBreak::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        node_add_location(&dict, _range, _vm, source_code);
        node.into()
    }
    fn ast_from_object(
        _vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(ruff::StmtBreak {
            range: range_from_object(_vm, source_code, _object, "Break")?,
        })
    }
}
// constructor
impl Node for ruff::StmtContinue {
    fn ast_to_object(self, _vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let ruff::StmtContinue { range: _range } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeStmtContinue::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        node_add_location(&dict, _range, _vm, source_code);
        node.into()
    }
    fn ast_from_object(
        _vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(ruff::StmtContinue {
            range: range_from_object(_vm, source_code, _object, "Continue")?,
        })
    }
}
