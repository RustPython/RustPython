use super::*;
use crate::stdlib::ast::argument::{merge_class_def_args, split_class_def_args};
use rustpython_compiler_core::SourceFile;

// sum
impl Node for ast::Stmt {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        match self {
            Self::FunctionDef(cons) => cons.ast_to_object(vm, source_file),
            Self::ClassDef(cons) => cons.ast_to_object(vm, source_file),
            Self::Return(cons) => cons.ast_to_object(vm, source_file),
            Self::Delete(cons) => cons.ast_to_object(vm, source_file),
            Self::Assign(cons) => cons.ast_to_object(vm, source_file),
            Self::TypeAlias(cons) => cons.ast_to_object(vm, source_file),
            Self::AugAssign(cons) => cons.ast_to_object(vm, source_file),
            Self::AnnAssign(cons) => cons.ast_to_object(vm, source_file),
            Self::For(cons) => cons.ast_to_object(vm, source_file),
            Self::While(cons) => cons.ast_to_object(vm, source_file),
            Self::If(cons) => cons.ast_to_object(vm, source_file),
            Self::With(cons) => cons.ast_to_object(vm, source_file),
            Self::Match(cons) => cons.ast_to_object(vm, source_file),
            Self::Raise(cons) => cons.ast_to_object(vm, source_file),
            Self::Try(cons) => cons.ast_to_object(vm, source_file),
            Self::Assert(cons) => cons.ast_to_object(vm, source_file),
            Self::Import(cons) => cons.ast_to_object(vm, source_file),
            Self::ImportFrom(cons) => cons.ast_to_object(vm, source_file),
            Self::Global(cons) => cons.ast_to_object(vm, source_file),
            Self::Nonlocal(cons) => cons.ast_to_object(vm, source_file),
            Self::Expr(cons) => cons.ast_to_object(vm, source_file),
            Self::Pass(cons) => cons.ast_to_object(vm, source_file),
            Self::Break(cons) => cons.ast_to_object(vm, source_file),
            Self::Continue(cons) => cons.ast_to_object(vm, source_file),
            Self::IpyEscapeCommand(_) => {
                unimplemented!("IPython escape command is not allowed in Python AST")
            }
        }
    }

    #[allow(clippy::if_same_then_else)]
    fn ast_from_object(
        _vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        let _cls = _object.class();
        Ok(if _cls.is(pyast::NodeStmtFunctionDef::static_type()) {
            Self::FunctionDef(ast::StmtFunctionDef::ast_from_object(
                _vm,
                source_file,
                _object,
            )?)
        } else if _cls.is(pyast::NodeStmtAsyncFunctionDef::static_type()) {
            Self::FunctionDef(ast::StmtFunctionDef::ast_from_object(
                _vm,
                source_file,
                _object,
            )?)
        } else if _cls.is(pyast::NodeStmtClassDef::static_type()) {
            Self::ClassDef(ast::StmtClassDef::ast_from_object(
                _vm,
                source_file,
                _object,
            )?)
        } else if _cls.is(pyast::NodeStmtReturn::static_type()) {
            Self::Return(ast::StmtReturn::ast_from_object(_vm, source_file, _object)?)
        } else if _cls.is(pyast::NodeStmtDelete::static_type()) {
            Self::Delete(ast::StmtDelete::ast_from_object(_vm, source_file, _object)?)
        } else if _cls.is(pyast::NodeStmtAssign::static_type()) {
            Self::Assign(ast::StmtAssign::ast_from_object(_vm, source_file, _object)?)
        } else if _cls.is(pyast::NodeStmtTypeAlias::static_type()) {
            Self::TypeAlias(ast::StmtTypeAlias::ast_from_object(
                _vm,
                source_file,
                _object,
            )?)
        } else if _cls.is(pyast::NodeStmtAugAssign::static_type()) {
            Self::AugAssign(ast::StmtAugAssign::ast_from_object(
                _vm,
                source_file,
                _object,
            )?)
        } else if _cls.is(pyast::NodeStmtAnnAssign::static_type()) {
            Self::AnnAssign(ast::StmtAnnAssign::ast_from_object(
                _vm,
                source_file,
                _object,
            )?)
        } else if _cls.is(pyast::NodeStmtFor::static_type()) {
            Self::For(ast::StmtFor::ast_from_object(_vm, source_file, _object)?)
        } else if _cls.is(pyast::NodeStmtAsyncFor::static_type()) {
            Self::For(ast::StmtFor::ast_from_object(_vm, source_file, _object)?)
        } else if _cls.is(pyast::NodeStmtWhile::static_type()) {
            Self::While(ast::StmtWhile::ast_from_object(_vm, source_file, _object)?)
        } else if _cls.is(pyast::NodeStmtIf::static_type()) {
            Self::If(ast::StmtIf::ast_from_object(_vm, source_file, _object)?)
        } else if _cls.is(pyast::NodeStmtWith::static_type()) {
            Self::With(ast::StmtWith::ast_from_object(_vm, source_file, _object)?)
        } else if _cls.is(pyast::NodeStmtAsyncWith::static_type()) {
            Self::With(ast::StmtWith::ast_from_object(_vm, source_file, _object)?)
        } else if _cls.is(pyast::NodeStmtMatch::static_type()) {
            Self::Match(ast::StmtMatch::ast_from_object(_vm, source_file, _object)?)
        } else if _cls.is(pyast::NodeStmtRaise::static_type()) {
            Self::Raise(ast::StmtRaise::ast_from_object(_vm, source_file, _object)?)
        } else if _cls.is(pyast::NodeStmtTry::static_type()) {
            Self::Try(ast::StmtTry::ast_from_object(_vm, source_file, _object)?)
        } else if _cls.is(pyast::NodeStmtTryStar::static_type()) {
            Self::Try(ast::StmtTry::ast_from_object(_vm, source_file, _object)?)
        } else if _cls.is(pyast::NodeStmtAssert::static_type()) {
            Self::Assert(ast::StmtAssert::ast_from_object(_vm, source_file, _object)?)
        } else if _cls.is(pyast::NodeStmtImport::static_type()) {
            Self::Import(ast::StmtImport::ast_from_object(_vm, source_file, _object)?)
        } else if _cls.is(pyast::NodeStmtImportFrom::static_type()) {
            Self::ImportFrom(ast::StmtImportFrom::ast_from_object(
                _vm,
                source_file,
                _object,
            )?)
        } else if _cls.is(pyast::NodeStmtGlobal::static_type()) {
            Self::Global(ast::StmtGlobal::ast_from_object(_vm, source_file, _object)?)
        } else if _cls.is(pyast::NodeStmtNonlocal::static_type()) {
            Self::Nonlocal(ast::StmtNonlocal::ast_from_object(
                _vm,
                source_file,
                _object,
            )?)
        } else if _cls.is(pyast::NodeStmtExpr::static_type()) {
            Self::Expr(ast::StmtExpr::ast_from_object(_vm, source_file, _object)?)
        } else if _cls.is(pyast::NodeStmtPass::static_type()) {
            Self::Pass(ast::StmtPass::ast_from_object(_vm, source_file, _object)?)
        } else if _cls.is(pyast::NodeStmtBreak::static_type()) {
            Self::Break(ast::StmtBreak::ast_from_object(_vm, source_file, _object)?)
        } else if _cls.is(pyast::NodeStmtContinue::static_type()) {
            Self::Continue(ast::StmtContinue::ast_from_object(
                _vm,
                source_file,
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
impl Node for ast::StmtFunctionDef {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
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
            pyast::NodeStmtFunctionDef::static_type().to_owned()
        } else {
            pyast::NodeStmtAsyncFunctionDef::static_type().to_owned()
        };

        let node = NodeAst.into_ref_with_type(vm, cls).unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("name", vm.ctx.new_str(name.as_str()).to_pyobject(vm), vm)
            .unwrap();
        dict.set_item("args", parameters.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item("body", body.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item(
            "decorator_list",
            decorator_list.ast_to_object(vm, source_file),
            vm,
        )
        .unwrap();
        dict.set_item("returns", returns.ast_to_object(vm, source_file), vm)
            .unwrap();
        // TODO: Ruff ignores type_comment during parsing
        // dict.set_item("type_comment", type_comment.ast_to_object(_vm), _vm)
        //     .unwrap();
        dict.set_item(
            "type_params",
            type_params.ast_to_object(vm, source_file),
            vm,
        )
        .unwrap();
        node_add_location(&dict, _range, vm, source_file);
        node.into()
    }
    fn ast_from_object(
        _vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        let _cls = _object.class();
        let is_async = _cls.is(pyast::NodeStmtAsyncFunctionDef::static_type());
        Ok(Self {
            node_index: Default::default(),
            name: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field(_vm, &_object, "name", "FunctionDef")?,
            )?,
            parameters: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field(_vm, &_object, "args", "FunctionDef")?,
            )?,
            body: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field(_vm, &_object, "body", "FunctionDef")?,
            )?,
            decorator_list: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field(_vm, &_object, "decorator_list", "FunctionDef")?,
            )?,
            returns: get_node_field_opt(_vm, &_object, "returns")?
                .map(|obj| Node::ast_from_object(_vm, source_file, obj))
                .transpose()?,
            // TODO: Ruff ignores type_comment during parsing
            // type_comment: get_node_field_opt(_vm, &_object, "type_comment")?
            //     .map(|obj| Node::ast_from_object(_vm, obj))
            //     .transpose()?,
            type_params: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field_opt(_vm, &_object, "type_params")?.unwrap_or_else(|| _vm.ctx.none()),
            )?,
            range: range_from_object(_vm, source_file, _object, "FunctionDef")?,
            is_async,
        })
    }
}

// constructor
impl Node for ast::StmtClassDef {
    fn ast_to_object(self, _vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            name,
            arguments,
            body,
            decorator_list,
            type_params,
            range: _range,
        } = self;
        let (bases, keywords) = split_class_def_args(arguments);
        let node = NodeAst
            .into_ref_with_type(_vm, pyast::NodeStmtClassDef::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("name", name.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        dict.set_item("bases", bases.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        dict.set_item("keywords", keywords.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        dict.set_item("body", body.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        dict.set_item(
            "decorator_list",
            decorator_list.ast_to_object(_vm, source_file),
            _vm,
        )
        .unwrap();
        dict.set_item(
            "type_params",
            type_params.ast_to_object(_vm, source_file),
            _vm,
        )
        .unwrap();
        node_add_location(&dict, _range, _vm, source_file);
        node.into()
    }
    fn ast_from_object(
        _vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        let bases = Node::ast_from_object(
            _vm,
            source_file,
            get_node_field(_vm, &_object, "bases", "ClassDef")?,
        )?;
        let keywords = Node::ast_from_object(
            _vm,
            source_file,
            get_node_field(_vm, &_object, "keywords", "ClassDef")?,
        )?;
        Ok(Self {
            node_index: Default::default(),
            name: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field(_vm, &_object, "name", "ClassDef")?,
            )?,
            arguments: merge_class_def_args(bases, keywords),
            body: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field(_vm, &_object, "body", "ClassDef")?,
            )?,
            decorator_list: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field(_vm, &_object, "decorator_list", "ClassDef")?,
            )?,
            type_params: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field_opt(_vm, &_object, "type_params")?.unwrap_or_else(|| _vm.ctx.none()),
            )?,
            range: range_from_object(_vm, source_file, _object, "ClassDef")?,
        })
    }
}
// constructor
impl Node for ast::StmtReturn {
    fn ast_to_object(self, _vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            value,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, pyast::NodeStmtReturn::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm, source_file);
        node.into()
    }
    fn ast_from_object(
        _vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            node_index: Default::default(),
            value: get_node_field_opt(_vm, &_object, "value")?
                .map(|obj| Node::ast_from_object(_vm, source_file, obj))
                .transpose()?,
            range: range_from_object(_vm, source_file, _object, "Return")?,
        })
    }
}
// constructor
impl Node for ast::StmtDelete {
    fn ast_to_object(self, _vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            targets,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, pyast::NodeStmtDelete::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("targets", targets.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm, source_file);
        node.into()
    }
    fn ast_from_object(
        _vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            node_index: Default::default(),
            targets: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field(_vm, &_object, "targets", "Delete")?,
            )?,
            range: range_from_object(_vm, source_file, _object, "Delete")?,
        })
    }
}

// constructor
impl Node for ast::StmtAssign {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            targets,
            value,
            // type_comment,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeStmtAssign::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("targets", targets.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item("value", value.ast_to_object(vm, source_file), vm)
            .unwrap();
        // TODO
        dict.set_item("type_comment", vm.ctx.none(), vm).unwrap();
        node_add_location(&dict, range, vm, source_file);
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            node_index: Default::default(),
            targets: Node::ast_from_object(
                vm,
                source_file,
                get_node_field(vm, &object, "targets", "Assign")?,
            )?,
            value: Node::ast_from_object(
                vm,
                source_file,
                get_node_field(vm, &object, "value", "Assign")?,
            )?,
            // type_comment: get_node_field_opt(_vm, &_object, "type_comment")?
            //     .map(|obj| Node::ast_from_object(_vm, obj))
            //     .transpose()?,
            range: range_from_object(vm, source_file, object, "Assign")?,
        })
    }
}

// constructor
impl Node for ast::StmtTypeAlias {
    fn ast_to_object(self, _vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            name,
            type_params,
            value,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, pyast::NodeStmtTypeAlias::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("name", name.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        dict.set_item(
            "type_params",
            type_params.ast_to_object(_vm, source_file),
            _vm,
        )
        .unwrap();
        dict.set_item("value", value.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm, source_file);
        node.into()
    }

    fn ast_from_object(
        _vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            node_index: Default::default(),
            name: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field(_vm, &_object, "name", "TypeAlias")?,
            )?,
            type_params: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field_opt(_vm, &_object, "type_params")?.unwrap_or_else(|| _vm.ctx.none()),
            )?,
            value: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field(_vm, &_object, "value", "TypeAlias")?,
            )?,
            range: range_from_object(_vm, source_file, _object, "TypeAlias")?,
        })
    }
}

// constructor
impl Node for ast::StmtAugAssign {
    fn ast_to_object(self, _vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            target,
            op,
            value,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, pyast::NodeStmtAugAssign::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("target", target.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        dict.set_item("op", op.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        dict.set_item("value", value.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm, source_file);
        node.into()
    }
    fn ast_from_object(
        _vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            node_index: Default::default(),
            target: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field(_vm, &_object, "target", "AugAssign")?,
            )?,
            op: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field(_vm, &_object, "op", "AugAssign")?,
            )?,
            value: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field(_vm, &_object, "value", "AugAssign")?,
            )?,
            range: range_from_object(_vm, source_file, _object, "AugAssign")?,
        })
    }
}

// constructor
impl Node for ast::StmtAnnAssign {
    fn ast_to_object(self, _vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            target,
            annotation,
            value,
            simple,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, pyast::NodeStmtAnnAssign::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("target", target.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        dict.set_item(
            "annotation",
            annotation.ast_to_object(_vm, source_file),
            _vm,
        )
        .unwrap();
        dict.set_item("value", value.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        dict.set_item("simple", simple.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm, source_file);
        node.into()
    }
    fn ast_from_object(
        _vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            node_index: Default::default(),
            target: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field(_vm, &_object, "target", "AnnAssign")?,
            )?,
            annotation: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field(_vm, &_object, "annotation", "AnnAssign")?,
            )?,
            value: get_node_field_opt(_vm, &_object, "value")?
                .map(|obj| Node::ast_from_object(_vm, source_file, obj))
                .transpose()?,
            simple: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field(_vm, &_object, "simple", "AnnAssign")?,
            )?,
            range: range_from_object(_vm, source_file, _object, "AnnAssign")?,
        })
    }
}

// constructor
impl Node for ast::StmtFor {
    fn ast_to_object(self, _vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            is_async,
            target,
            iter,
            body,
            orelse,
            // type_comment,
            range: _range,
        } = self;

        let cls = if !is_async {
            pyast::NodeStmtFor::static_type().to_owned()
        } else {
            pyast::NodeStmtAsyncFor::static_type().to_owned()
        };

        let node = NodeAst.into_ref_with_type(_vm, cls).unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("target", target.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        dict.set_item("iter", iter.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        dict.set_item("body", body.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        dict.set_item("orelse", orelse.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        // dict.set_item("type_comment", type_comment.ast_to_object(_vm), _vm)
        //     .unwrap();
        node_add_location(&dict, _range, _vm, source_file);
        node.into()
    }

    fn ast_from_object(
        _vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        let _cls = _object.class();
        debug_assert!(
            _cls.is(pyast::NodeStmtFor::static_type())
                || _cls.is(pyast::NodeStmtAsyncFor::static_type())
        );
        let is_async = _cls.is(pyast::NodeStmtAsyncFor::static_type());
        Ok(Self {
            node_index: Default::default(),
            target: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field(_vm, &_object, "target", "For")?,
            )?,
            iter: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field(_vm, &_object, "iter", "For")?,
            )?,
            body: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field(_vm, &_object, "body", "For")?,
            )?,
            orelse: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field(_vm, &_object, "orelse", "For")?,
            )?,
            // type_comment: get_node_field_opt(_vm, &_object, "type_comment")?
            //     .map(|obj| Node::ast_from_object(_vm, obj))
            //     .transpose()?,
            range: range_from_object(_vm, source_file, _object, "For")?,
            is_async,
        })
    }
}

// constructor
impl Node for ast::StmtWhile {
    fn ast_to_object(self, _vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            test,
            body,
            orelse,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, pyast::NodeStmtWhile::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("test", test.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        dict.set_item("body", body.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        dict.set_item("orelse", orelse.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm, source_file);
        node.into()
    }

    fn ast_from_object(
        _vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            node_index: Default::default(),
            test: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field(_vm, &_object, "test", "While")?,
            )?,
            body: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field(_vm, &_object, "body", "While")?,
            )?,
            orelse: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field(_vm, &_object, "orelse", "While")?,
            )?,
            range: range_from_object(_vm, source_file, _object, "While")?,
        })
    }
}
// constructor
impl Node for ast::StmtIf {
    fn ast_to_object(self, _vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            test,
            body,
            range,
            elif_else_clauses,
        } = self;
        elif_else_clause::ast_to_object(
            ast::ElifElseClause {
                node_index: Default::default(),
                range,
                test: Some(*test),
                body,
            },
            elif_else_clauses.into_iter(),
            _vm,
            source_file,
        )
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        elif_else_clause::ast_from_object(vm, source_file, object)
    }
}
// constructor
impl Node for ast::StmtWith {
    fn ast_to_object(self, _vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            is_async,
            items,
            body,
            // type_comment,
            range: _range,
        } = self;

        let cls = if !is_async {
            pyast::NodeStmtWith::static_type().to_owned()
        } else {
            pyast::NodeStmtAsyncWith::static_type().to_owned()
        };

        let node = NodeAst.into_ref_with_type(_vm, cls).unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("items", items.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        dict.set_item("body", body.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        // dict.set_item("type_comment", type_comment.ast_to_object(_vm), _vm)
        //     .unwrap();
        node_add_location(&dict, _range, _vm, source_file);
        node.into()
    }
    fn ast_from_object(
        _vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        let _cls = _object.class();
        debug_assert!(
            _cls.is(pyast::NodeStmtWith::static_type())
                || _cls.is(pyast::NodeStmtAsyncWith::static_type())
        );
        let is_async = _cls.is(pyast::NodeStmtAsyncWith::static_type());
        Ok(Self {
            node_index: Default::default(),
            items: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field(_vm, &_object, "items", "With")?,
            )?,
            body: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field(_vm, &_object, "body", "With")?,
            )?,
            // type_comment: get_node_field_opt(_vm, &_object, "type_comment")?
            //     .map(|obj| Node::ast_from_object(_vm, obj))
            //     .transpose()?,
            range: range_from_object(_vm, source_file, _object, "With")?,
            is_async,
        })
    }
}
// constructor
impl Node for ast::StmtMatch {
    fn ast_to_object(self, _vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            subject,
            cases,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, pyast::NodeStmtMatch::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("subject", subject.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        dict.set_item("cases", cases.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm, source_file);
        node.into()
    }
    fn ast_from_object(
        _vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            node_index: Default::default(),
            subject: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field(_vm, &_object, "subject", "Match")?,
            )?,
            cases: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field(_vm, &_object, "cases", "Match")?,
            )?,
            range: range_from_object(_vm, source_file, _object, "Match")?,
        })
    }
}
// constructor
impl Node for ast::StmtRaise {
    fn ast_to_object(self, _vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            exc,
            cause,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, pyast::NodeStmtRaise::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("exc", exc.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        dict.set_item("cause", cause.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm, source_file);
        node.into()
    }
    fn ast_from_object(
        _vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            node_index: Default::default(),
            exc: get_node_field_opt(_vm, &_object, "exc")?
                .map(|obj| Node::ast_from_object(_vm, source_file, obj))
                .transpose()?,
            cause: get_node_field_opt(_vm, &_object, "cause")?
                .map(|obj| Node::ast_from_object(_vm, source_file, obj))
                .transpose()?,
            range: range_from_object(_vm, source_file, _object, "Raise")?,
        })
    }
}
// constructor
impl Node for ast::StmtTry {
    fn ast_to_object(self, _vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            body,
            handlers,
            orelse,
            finalbody,
            range: _range,
            is_star,
        } = self;

        // let cls = gen::NodeStmtTry::static_type().to_owned();
        let cls = if is_star {
            pyast::NodeStmtTryStar::static_type()
        } else {
            pyast::NodeStmtTry::static_type()
        }
        .to_owned();

        let node = NodeAst.into_ref_with_type(_vm, cls).unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("body", body.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        dict.set_item("handlers", handlers.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        dict.set_item("orelse", orelse.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        dict.set_item("finalbody", finalbody.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm, source_file);
        node.into()
    }
    fn ast_from_object(
        _vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        let _cls = _object.class();
        let is_star = _cls.is(pyast::NodeStmtTryStar::static_type());
        let _cls = _object.class();
        debug_assert!(
            _cls.is(pyast::NodeStmtTry::static_type())
                || _cls.is(pyast::NodeStmtTryStar::static_type())
        );

        Ok(Self {
            node_index: Default::default(),
            body: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field(_vm, &_object, "body", "Try")?,
            )?,
            handlers: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field(_vm, &_object, "handlers", "Try")?,
            )?,
            orelse: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field(_vm, &_object, "orelse", "Try")?,
            )?,
            finalbody: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field(_vm, &_object, "finalbody", "Try")?,
            )?,
            range: range_from_object(_vm, source_file, _object, "Try")?,
            is_star,
        })
    }
}
// constructor
impl Node for ast::StmtAssert {
    fn ast_to_object(self, _vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            test,
            msg,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, pyast::NodeStmtAssert::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("test", test.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        dict.set_item("msg", msg.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm, source_file);
        node.into()
    }
    fn ast_from_object(
        _vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            node_index: Default::default(),
            test: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field(_vm, &_object, "test", "Assert")?,
            )?,
            msg: get_node_field_opt(_vm, &_object, "msg")?
                .map(|obj| Node::ast_from_object(_vm, source_file, obj))
                .transpose()?,
            range: range_from_object(_vm, source_file, _object, "Assert")?,
        })
    }
}
// constructor
impl Node for ast::StmtImport {
    fn ast_to_object(self, _vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            names,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, pyast::NodeStmtImport::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("names", names.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm, source_file);
        node.into()
    }
    fn ast_from_object(
        _vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            node_index: Default::default(),
            names: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field(_vm, &_object, "names", "Import")?,
            )?,
            range: range_from_object(_vm, source_file, _object, "Import")?,
        })
    }
}
// constructor
impl Node for ast::StmtImportFrom {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            module,
            names,
            level,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeStmtImportFrom::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("module", module.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item("names", names.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item("level", vm.ctx.new_int(level).to_pyobject(vm), vm)
            .unwrap();
        node_add_location(&dict, range, vm, source_file);
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            node_index: Default::default(),
            module: get_node_field_opt(vm, &_object, "module")?
                .map(|obj| Node::ast_from_object(vm, source_file, obj))
                .transpose()?,
            names: Node::ast_from_object(
                vm,
                source_file,
                get_node_field(vm, &_object, "names", "ImportFrom")?,
            )?,
            level: get_node_field_opt(vm, &_object, "level")?
                .map(|obj| -> PyResult<u32> {
                    let int: PyRef<PyInt> = obj.try_into_value(vm)?;
                    int.try_to_primitive(vm)
                })
                .transpose()?
                .unwrap_or(0),
            range: range_from_object(vm, source_file, _object, "ImportFrom")?,
        })
    }
}
// constructor
impl Node for ast::StmtGlobal {
    fn ast_to_object(self, _vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            names,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, pyast::NodeStmtGlobal::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("names", names.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm, source_file);
        node.into()
    }
    fn ast_from_object(
        _vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            node_index: Default::default(),
            names: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field(_vm, &_object, "names", "Global")?,
            )?,
            range: range_from_object(_vm, source_file, _object, "Global")?,
        })
    }
}
// constructor
impl Node for ast::StmtNonlocal {
    fn ast_to_object(self, _vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            names,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, pyast::NodeStmtNonlocal::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("names", names.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm, source_file);
        node.into()
    }
    fn ast_from_object(
        _vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            node_index: Default::default(),
            names: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field(_vm, &_object, "names", "Nonlocal")?,
            )?,
            range: range_from_object(_vm, source_file, _object, "Nonlocal")?,
        })
    }
}
// constructor
impl Node for ast::StmtExpr {
    fn ast_to_object(self, _vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            value,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, pyast::NodeStmtExpr::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("value", value.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm, source_file);
        node.into()
    }
    fn ast_from_object(
        _vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            node_index: Default::default(),
            value: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field(_vm, &_object, "value", "Expr")?,
            )?,
            range: range_from_object(_vm, source_file, _object, "Expr")?,
        })
    }
}
// constructor
impl Node for ast::StmtPass {
    fn ast_to_object(self, _vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, pyast::NodeStmtPass::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        node_add_location(&dict, _range, _vm, source_file);
        node.into()
    }
    fn ast_from_object(
        _vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            node_index: Default::default(),
            range: range_from_object(_vm, source_file, _object, "Pass")?,
        })
    }
}
// constructor
impl Node for ast::StmtBreak {
    fn ast_to_object(self, _vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, pyast::NodeStmtBreak::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        node_add_location(&dict, _range, _vm, source_file);
        node.into()
    }

    fn ast_from_object(
        _vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            node_index: Default::default(),
            range: range_from_object(_vm, source_file, _object, "Break")?,
        })
    }
}

// constructor
impl Node for ast::StmtContinue {
    fn ast_to_object(self, _vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, pyast::NodeStmtContinue::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        node_add_location(&dict, _range, _vm, source_file);
        node.into()
    }
    fn ast_from_object(
        _vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            node_index: Default::default(),
            range: range_from_object(_vm, source_file, _object, "Continue")?,
        })
    }
}
