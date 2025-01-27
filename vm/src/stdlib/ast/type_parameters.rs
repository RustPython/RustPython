use super::*;

impl Node for ruff::TypeParams {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        todo!()
    }

    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        todo!()
    }
}
// sum
impl Node for ruff::TypeParam {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        match self {
            Self::TypeVar(cons) => cons.ast_to_object(vm),
            Self::ParamSpec(cons) => cons.ast_to_object(vm),
            Self::TypeVarTuple(cons) => cons.ast_to_object(vm),
        }
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        let _cls = _object.class();
        Ok(if _cls.is(gen::NodeTypeParamTypeVar::static_type()) {
            ruff::TypeParam::TypeVar(ruff::TypeParamTypeVar::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeTypeParamParamSpec::static_type()) {
            ruff::TypeParam::ParamSpec(ruff::TypeParamParamSpec::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeTypeParamTypeVarTuple::static_type()) {
            ruff::TypeParam::TypeVarTuple(ruff::TypeParamTypeVarTuple::ast_from_object(
                _vm, _object,
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
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            name,
            bound,
            range: _range,
            default: _,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeTypeParamTypeVar::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("name", name.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("bound", bound.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            name: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "name", "TypeVar")?)?,
            bound: get_node_field_opt(_vm, &_object, "bound")?
                .map(|obj| Node::ast_from_object(_vm, obj))
                .transpose()?,
            default: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "default_value", "TypeVar")?,
            )?,
            range: range_from_object(_vm, _object, "TypeVar")?,
        })
    }
}
// constructor
impl Node for ruff::TypeParamParamSpec {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            name,
            range: _range,
            default,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeTypeParamParamSpec::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("name", name.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("default_value", default.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            name: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "name", "ParamSpec")?)?,
            default: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "default_value", "ParamSpec")?,
            )?,
            range: range_from_object(_vm, _object, "ParamSpec")?,
        })
    }
}
// constructor
impl Node for ruff::TypeParamTypeVarTuple {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            name,
            range: _range,
            default,
        } = self;
        let node = NodeAst
            .into_ref_with_type(
                _vm,
                gen::NodeTypeParamTypeVarTuple::static_type().to_owned(),
            )
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("name", name.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("default_value", default.ast_to_object(_vm), _vm)
            .unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            name: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "name", "TypeVarTuple")?,
            )?,
            default: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "default_value", "TypeVarTuple")?,
            )?,
            range: range_from_object(_vm, _object, "TypeVarTuple")?,
        })
    }
}
