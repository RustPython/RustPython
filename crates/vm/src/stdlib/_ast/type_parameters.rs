use super::*;
use rustpython_compiler_core::SourceFile;

impl Node for ast::TypeParams {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        self.runtime_type_params.map_or_else(
            || self.type_params.ast_to_object(vm, source_file),
            |values| values.ast_to_object(vm, source_file),
        )
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(type_params_from_values(
            vm,
            Node::ast_from_object(vm, source_file, object)?,
        ))
    }

    fn is_none(&self) -> bool {
        self.type_params.is_empty() && self.runtime_type_params.is_none()
    }
}

pub(super) fn type_params_from_field(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    object: &PyObject,
    field: &'static str,
    typ: &str,
) -> PyResult<Option<Box<ast::TypeParams>>> {
    let type_params: Vec<Option<ast::TypeParam>> =
        get_node_list_field(vm, source_file, object, field, typ)?;
    let type_params = type_params_from_values(vm, type_params);
    Ok((!type_params.is_none()).then_some(Box::new(type_params)))
}

fn type_params_from_values(
    _vm: &VirtualMachine,
    values: Vec<Option<ast::TypeParam>>,
) -> ast::TypeParams {
    let runtime_type_params = values.iter().any(Option::is_none).then(|| values.clone());
    let type_params = lower_nullable_type_params(&values);
    let range = Option::zip(type_params.first(), type_params.last())
        .map(|(first, last)| first.range().cover(last.range()))
        .unwrap_or_default();
    ast::TypeParams {
        node_index: Default::default(),
        type_params,
        range,
        runtime_type_params,
    }
}

fn lower_nullable_type_params(values: &[Option<ast::TypeParam>]) -> Vec<ast::TypeParam> {
    values.iter().filter_map(Clone::clone).collect()
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
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        if vm.is_none(&object) {
            return Err(vm.new_type_error(format!(
                "expected some sort of type_param, but got {}",
                object.repr(vm)?
            )));
        }
        enum TypeParamKind {
            TypeVar,
            ParamSpec,
            TypeVarTuple,
        }
        let kind = if is_node_instance(vm, &object, pyast::NodeTypeParamTypeVar::static_type())? {
            TypeParamKind::TypeVar
        } else if is_node_instance(vm, &object, pyast::NodeTypeParamParamSpec::static_type())? {
            TypeParamKind::ParamSpec
        } else if is_node_instance(vm, &object, pyast::NodeTypeParamTypeVarTuple::static_type())? {
            TypeParamKind::TypeVarTuple
        } else {
            return Err(vm.new_type_error(format!(
                "expected some sort of type_param, but got {}",
                object.repr(vm)?
            )));
        };
        let range = type_param_range_from_object(vm, source_file, object.clone())?;
        Ok(match kind {
            TypeParamKind::TypeVar => Self::TypeVar(type_var_from_object_with_range(
                vm,
                source_file,
                object,
                range,
            )?),
            TypeParamKind::ParamSpec => Self::ParamSpec(param_spec_from_object_with_range(
                vm,
                source_file,
                object,
                range,
            )?),
            TypeParamKind::TypeVarTuple => Self::TypeVarTuple(
                type_var_tuple_from_object_with_range(vm, source_file, object, range)?,
            ),
        })
    }
}

// constructor
fn type_var_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::TypeParamTypeVar> {
    Ok(ast::TypeParamTypeVar {
        node_index: Default::default(),
        name: get_required_identifier_field(vm, source_file, &object, "name", "TypeVar")?,
        bound: get_node_field_opt(vm, &object, "bound")?
            .map(|obj| Node::ast_from_object(vm, source_file, obj))
            .transpose()?,
        default: get_node_field_opt(vm, &object, "default_value")?
            .map(|obj| Node::ast_from_object(vm, source_file, obj))
            .transpose()?,
        range,
    })
}

impl Node for ast::TypeParamTypeVar {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            name,
            bound,
            range,
            default,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeTypeParamTypeVar::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("name", name.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item("bound", bound.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item("default_value", default.ast_to_object(vm, source_file), vm)
            .unwrap();
        node_add_location(&dict, range, vm, source_file);
        node.into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = type_param_range_from_object(vm, source_file, object.clone())?;
        type_var_from_object_with_range(vm, source_file, object, range)
    }
}

// constructor
fn param_spec_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::TypeParamParamSpec> {
    Ok(ast::TypeParamParamSpec {
        node_index: Default::default(),
        name: get_required_identifier_field(vm, source_file, &object, "name", "ParamSpec")?,
        default: get_node_field_opt(vm, &object, "default_value")?
            .map(|obj| Node::ast_from_object(vm, source_file, obj))
            .transpose()?,
        range,
    })
}

impl Node for ast::TypeParamParamSpec {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            name,
            range,
            default,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeTypeParamParamSpec::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("name", name.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item("default_value", default.ast_to_object(vm, source_file), vm)
            .unwrap();
        node_add_location(&dict, range, vm, source_file);
        node.into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = type_param_range_from_object(vm, source_file, object.clone())?;
        param_spec_from_object_with_range(vm, source_file, object, range)
    }
}

// constructor
fn type_var_tuple_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::TypeParamTypeVarTuple> {
    Ok(ast::TypeParamTypeVarTuple {
        node_index: Default::default(),
        name: get_required_identifier_field(vm, source_file, &object, "name", "TypeVarTuple")?,
        default: get_node_field_opt(vm, &object, "default_value")?
            .map(|obj| Node::ast_from_object(vm, source_file, obj))
            .transpose()?,
        range,
    })
}

impl Node for ast::TypeParamTypeVarTuple {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            name,
            range,
            default,
        } = self;
        let node = NodeAst
            .into_ref_with_type(
                vm,
                pyast::NodeTypeParamTypeVarTuple::static_type().to_owned(),
            )
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("name", name.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item("default_value", default.ast_to_object(vm, source_file), vm)
            .unwrap();
        node_add_location(&dict, range, vm, source_file);
        node.into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = type_param_range_from_object(vm, source_file, object.clone())?;
        type_var_tuple_from_object_with_range(vm, source_file, object, range)
    }
}
