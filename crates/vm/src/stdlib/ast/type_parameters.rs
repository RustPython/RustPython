use super::*;
use rustpython_compiler_core::SourceFile;

impl Node for ast::TypeParams {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        self.type_params.ast_to_object(vm, source_file)
    }

    fn ast_from_object(
        _vm: &VirtualMachine,
        _source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        let type_params: Vec<ast::TypeParam> = Node::ast_from_object(_vm, _source_file, _object)?;
        let range = Option::zip(type_params.first(), type_params.last())
            .map(|(first, last)| first.range().cover(last.range()))
            .unwrap_or_default();
        Ok(Self {
            node_index: Default::default(),
            type_params,
            range,
        })
    }

    fn is_none(&self) -> bool {
        self.type_params.is_empty()
    }
}

// sum
impl Node for ast::TypeParam {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        match self {
            Self::TypeVar(cons) => cons.ast_to_object(vm, source_file),
            Self::ParamSpec(cons) => cons.ast_to_object(vm, source_file),
            Self::TypeVarTuple(cons) => cons.ast_to_object(vm, source_file),
        }
    }

    fn ast_from_object(
        _vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        let _cls = _object.class();
        Ok(if _cls.is(pyast::NodeTypeParamTypeVar::static_type()) {
            Self::TypeVar(ast::TypeParamTypeVar::ast_from_object(
                _vm,
                source_file,
                _object,
            )?)
        } else if _cls.is(pyast::NodeTypeParamParamSpec::static_type()) {
            Self::ParamSpec(ast::TypeParamParamSpec::ast_from_object(
                _vm,
                source_file,
                _object,
            )?)
        } else if _cls.is(pyast::NodeTypeParamTypeVarTuple::static_type()) {
            Self::TypeVarTuple(ast::TypeParamTypeVarTuple::ast_from_object(
                _vm,
                source_file,
                _object,
            )?)
        } else {
            return Err(_vm.new_type_error(format!(
                "expected some sort of type_param, but got {}",
                _object.repr(_vm)?
            )));
        })
    }
}

// constructor
impl Node for ast::TypeParamTypeVar {
    fn ast_to_object(self, _vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            name,
            bound,
            range: _range,
            default: _,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, pyast::NodeTypeParamTypeVar::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("name", name.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        dict.set_item("bound", bound.ast_to_object(_vm, source_file), _vm)
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
                get_node_field(_vm, &_object, "name", "TypeVar")?,
            )?,
            bound: get_node_field_opt(_vm, &_object, "bound")?
                .map(|obj| Node::ast_from_object(_vm, source_file, obj))
                .transpose()?,
            default: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field(_vm, &_object, "default_value", "TypeVar")?,
            )?,
            range: range_from_object(_vm, source_file, _object, "TypeVar")?,
        })
    }
}

// constructor
impl Node for ast::TypeParamParamSpec {
    fn ast_to_object(self, _vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            name,
            range: _range,
            default,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, pyast::NodeTypeParamParamSpec::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("name", name.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        dict.set_item(
            "default_value",
            default.ast_to_object(_vm, source_file),
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
        Ok(Self {
            node_index: Default::default(),
            name: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field(_vm, &_object, "name", "ParamSpec")?,
            )?,
            default: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field(_vm, &_object, "default_value", "ParamSpec")?,
            )?,
            range: range_from_object(_vm, source_file, _object, "ParamSpec")?,
        })
    }
}

// constructor
impl Node for ast::TypeParamTypeVarTuple {
    fn ast_to_object(self, _vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            name,
            range: _range,
            default,
        } = self;
        let node = NodeAst
            .into_ref_with_type(
                _vm,
                pyast::NodeTypeParamTypeVarTuple::static_type().to_owned(),
            )
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("name", name.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        dict.set_item(
            "default_value",
            default.ast_to_object(_vm, source_file),
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
        Ok(Self {
            node_index: Default::default(),
            name: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field(_vm, &_object, "name", "TypeVarTuple")?,
            )?,
            default: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field(_vm, &_object, "default_value", "TypeVarTuple")?,
            )?,
            range: range_from_object(_vm, source_file, _object, "TypeVarTuple")?,
        })
    }
}
