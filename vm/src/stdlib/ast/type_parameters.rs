use super::*;

impl Node for ruff::TypeParams {
    fn ast_to_object(self, vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        self.type_params.ast_to_object(vm, source_code)
    }

    fn ast_from_object(
        _vm: &VirtualMachine,
        _source_code: &SourceCodeOwned,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        let type_params: Vec<ruff::TypeParam> = Node::ast_from_object(_vm, _source_code, _object)?;
        let range = Option::zip(type_params.first(), type_params.last())
            .map(|(first, last)| first.range().cover(last.range()))
            .unwrap_or_default();
        Ok(Self { type_params, range })
    }

    fn is_none(&self) -> bool {
        self.type_params.is_empty()
    }
}
// sum
impl Node for ruff::TypeParam {
    fn ast_to_object(self, vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        match self {
            Self::TypeVar(cons) => cons.ast_to_object(vm, source_code),
            Self::ParamSpec(cons) => cons.ast_to_object(vm, source_code),
            Self::TypeVarTuple(cons) => cons.ast_to_object(vm, source_code),
        }
    }

    fn ast_from_object(
        _vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        let _cls = _object.class();
        Ok(if _cls.is(pyast::NodeTypeParamTypeVar::static_type()) {
            ruff::TypeParam::TypeVar(ruff::TypeParamTypeVar::ast_from_object(
                _vm,
                source_code,
                _object,
            )?)
        } else if _cls.is(pyast::NodeTypeParamParamSpec::static_type()) {
            ruff::TypeParam::ParamSpec(ruff::TypeParamParamSpec::ast_from_object(
                _vm,
                source_code,
                _object,
            )?)
        } else if _cls.is(pyast::NodeTypeParamTypeVarTuple::static_type()) {
            ruff::TypeParam::TypeVarTuple(ruff::TypeParamTypeVarTuple::ast_from_object(
                _vm,
                source_code,
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
impl Node for ruff::TypeParamTypeVar {
    fn ast_to_object(self, _vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let Self {
            name,
            bound,
            range: _range,
            default: _,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, pyast::NodeTypeParamTypeVar::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("name", name.ast_to_object(_vm, source_code), _vm)
            .unwrap();
        dict.set_item("bound", bound.ast_to_object(_vm, source_code), _vm)
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
            name: Node::ast_from_object(
                _vm,
                source_code,
                get_node_field(_vm, &_object, "name", "TypeVar")?,
            )?,
            bound: get_node_field_opt(_vm, &_object, "bound")?
                .map(|obj| Node::ast_from_object(_vm, source_code, obj))
                .transpose()?,
            default: Node::ast_from_object(
                _vm,
                source_code,
                get_node_field(_vm, &_object, "default_value", "TypeVar")?,
            )?,
            range: range_from_object(_vm, source_code, _object, "TypeVar")?,
        })
    }
}
// constructor
impl Node for ruff::TypeParamParamSpec {
    fn ast_to_object(self, _vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let Self {
            name,
            range: _range,
            default,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, pyast::NodeTypeParamParamSpec::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("name", name.ast_to_object(_vm, source_code), _vm)
            .unwrap();
        dict.set_item(
            "default_value",
            default.ast_to_object(_vm, source_code),
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
        Ok(Self {
            name: Node::ast_from_object(
                _vm,
                source_code,
                get_node_field(_vm, &_object, "name", "ParamSpec")?,
            )?,
            default: Node::ast_from_object(
                _vm,
                source_code,
                get_node_field(_vm, &_object, "default_value", "ParamSpec")?,
            )?,
            range: range_from_object(_vm, source_code, _object, "ParamSpec")?,
        })
    }
}
// constructor
impl Node for ruff::TypeParamTypeVarTuple {
    fn ast_to_object(self, _vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let Self {
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
        dict.set_item("name", name.ast_to_object(_vm, source_code), _vm)
            .unwrap();
        dict.set_item(
            "default_value",
            default.ast_to_object(_vm, source_code),
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
        Ok(Self {
            name: Node::ast_from_object(
                _vm,
                source_code,
                get_node_field(_vm, &_object, "name", "TypeVarTuple")?,
            )?,
            default: Node::ast_from_object(
                _vm,
                source_code,
                get_node_field(_vm, &_object, "default_value", "TypeVarTuple")?,
            )?,
            range: range_from_object(_vm, source_code, _object, "TypeVarTuple")?,
        })
    }
}
