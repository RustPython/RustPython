use super::*;
use rustpython_compiler_core::SourceFile;

impl Node for ast::TypeParams {
    fn ast_to_object(self, to_ctx: &AstToObjectContext<'_>) -> PyObjectRef {
        let _vm = to_ctx.vm;
        let _source_file = to_ctx.source_file;
        super::constant::public_ast_type_param_list_object(to_ctx, self.node_index.load())
            .map_or_else(
                || self.type_params.ast_to_object(to_ctx),
                |values| values.values.ast_to_object(to_ctx),
            )
    }

    fn ast_from_object(
        ctx: &AstFromObjectContext<'_>,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(type_params_from_values(
            ctx,
            Node::ast_from_object(ctx, source_file, object)?,
        ))
    }

    fn is_none(&self) -> bool {
        self.type_params.is_empty() && self.node_index.load() == ast::NodeIndex::NONE
    }
}

pub(super) fn type_params_from_field(
    ctx: &AstFromObjectContext<'_>,
    source_file: &SourceFile,
    object: &PyObject,
    field: &'static str,
    typ: &str,
) -> PyResult<Option<Box<ast::TypeParams>>> {
    let type_params: Vec<Option<ast::TypeParam>> =
        get_node_list_field(ctx, source_file, object, field, typ)?;
    let type_params = type_params_from_values(ctx, type_params);
    Ok((!type_params.is_none()).then_some(Box::new(type_params)))
}

fn type_params_from_values(
    ctx: &AstFromObjectContext<'_>,
    values: Vec<Option<ast::TypeParam>>,
) -> ast::TypeParams {
    let node_index = if values.iter().any(Option::is_none) {
        let index = super::constant::register_public_ast_type_param_list(ctx, values.clone());
        let node_index = ast::AtomicNodeIndex::NONE;
        node_index.set(index);
        node_index
    } else {
        Default::default()
    };
    let type_params = lower_nullable_type_params(&values);
    let range = Option::zip(type_params.first(), type_params.last())
        .map(|(first, last)| first.range().cover(last.range()))
        .unwrap_or_default();
    ast::TypeParams {
        node_index,
        type_params,
        range,
    }
}

fn lower_nullable_type_params(values: &[Option<ast::TypeParam>]) -> Vec<ast::TypeParam> {
    values.iter().filter_map(Clone::clone).collect()
}

// sum
impl Node for ast::TypeParam {
    fn ast_to_object(self, to_ctx: &AstToObjectContext<'_>) -> PyObjectRef {
        let _source_file = to_ctx.source_file;
        match self {
            Self::TypeVar(cons) => cons.ast_to_object(to_ctx),
            Self::ParamSpec(cons) => cons.ast_to_object(to_ctx),
            Self::TypeVarTuple(cons) => cons.ast_to_object(to_ctx),
        }
    }

    fn ast_from_object(
        ctx: &AstFromObjectContext<'_>,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        if ctx.is_none(&object) {
            return Err(ctx.new_type_error(format!(
                "expected some sort of type_param, but got {}",
                object.repr(ctx)?
            )));
        }
        enum TypeParamKind {
            TypeVar,
            ParamSpec,
            TypeVarTuple,
        }
        let kind = if is_node_instance(ctx, &object, pyast::NodeTypeParamTypeVar::static_type())? {
            TypeParamKind::TypeVar
        } else if is_node_instance(ctx, &object, pyast::NodeTypeParamParamSpec::static_type())? {
            TypeParamKind::ParamSpec
        } else if is_node_instance(
            ctx,
            &object,
            pyast::NodeTypeParamTypeVarTuple::static_type(),
        )? {
            TypeParamKind::TypeVarTuple
        } else {
            return Err(ctx.new_type_error(format!(
                "expected some sort of type_param, but got {}",
                object.repr(ctx)?
            )));
        };
        let range = type_param_range_from_object(ctx, source_file, object.clone())?;
        Ok(match kind {
            TypeParamKind::TypeVar => Self::TypeVar(type_var_from_object_with_range(
                ctx,
                source_file,
                object,
                range,
            )?),
            TypeParamKind::ParamSpec => Self::ParamSpec(param_spec_from_object_with_range(
                ctx,
                source_file,
                object,
                range,
            )?),
            TypeParamKind::TypeVarTuple => Self::TypeVarTuple(
                type_var_tuple_from_object_with_range(ctx, source_file, object, range)?,
            ),
        })
    }
}

// constructor
fn type_var_from_object_with_range(
    ctx: &AstFromObjectContext<'_>,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::TypeParamTypeVar> {
    Ok(ast::TypeParamTypeVar {
        node_index: Default::default(),
        name: get_required_identifier_field(ctx, source_file, &object, "name", "TypeVar")?,
        bound: get_node_field_opt(ctx, &object, "bound")?
            .map(|obj| Node::ast_from_object(ctx, source_file, obj))
            .transpose()?,
        default: get_node_field_opt(ctx, &object, "default_value")?
            .map(|obj| Node::ast_from_object(ctx, source_file, obj))
            .transpose()?,
        range,
    })
}

impl Node for ast::TypeParamTypeVar {
    fn ast_to_object(self, to_ctx: &AstToObjectContext<'_>) -> PyObjectRef {
        let vm = to_ctx.vm;
        let source_file = to_ctx.source_file;
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
        dict.set_item("name", name.ast_to_object(to_ctx), vm)
            .unwrap();
        dict.set_item("bound", bound.ast_to_object(to_ctx), vm)
            .unwrap();
        dict.set_item("default_value", default.ast_to_object(to_ctx), vm)
            .unwrap();
        node_add_location(&dict, range, vm, source_file);
        node.into()
    }

    fn ast_from_object(
        ctx: &AstFromObjectContext<'_>,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = type_param_range_from_object(ctx, source_file, object.clone())?;
        type_var_from_object_with_range(ctx, source_file, object, range)
    }
}

// constructor
fn param_spec_from_object_with_range(
    ctx: &AstFromObjectContext<'_>,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::TypeParamParamSpec> {
    Ok(ast::TypeParamParamSpec {
        node_index: Default::default(),
        name: get_required_identifier_field(ctx, source_file, &object, "name", "ParamSpec")?,
        default: get_node_field_opt(ctx, &object, "default_value")?
            .map(|obj| Node::ast_from_object(ctx, source_file, obj))
            .transpose()?,
        range,
    })
}

impl Node for ast::TypeParamParamSpec {
    fn ast_to_object(self, to_ctx: &AstToObjectContext<'_>) -> PyObjectRef {
        let ctx = to_ctx.vm;
        let source_file = to_ctx.source_file;
        let Self {
            node_index: _,
            name,
            range,
            default,
        } = self;
        let node = NodeAst
            .into_ref_with_type(ctx, pyast::NodeTypeParamParamSpec::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("name", name.ast_to_object(to_ctx), ctx)
            .unwrap();
        dict.set_item("default_value", default.ast_to_object(to_ctx), ctx)
            .unwrap();
        node_add_location(&dict, range, ctx, source_file);
        node.into()
    }

    fn ast_from_object(
        ctx: &AstFromObjectContext<'_>,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = type_param_range_from_object(ctx, source_file, object.clone())?;
        param_spec_from_object_with_range(ctx, source_file, object, range)
    }
}

// constructor
fn type_var_tuple_from_object_with_range(
    ctx: &AstFromObjectContext<'_>,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::TypeParamTypeVarTuple> {
    Ok(ast::TypeParamTypeVarTuple {
        node_index: Default::default(),
        name: get_required_identifier_field(ctx, source_file, &object, "name", "TypeVarTuple")?,
        default: get_node_field_opt(ctx, &object, "default_value")?
            .map(|obj| Node::ast_from_object(ctx, source_file, obj))
            .transpose()?,
        range,
    })
}

impl Node for ast::TypeParamTypeVarTuple {
    fn ast_to_object(self, to_ctx: &AstToObjectContext<'_>) -> PyObjectRef {
        let vm = to_ctx.vm;
        let source_file = to_ctx.source_file;
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
        dict.set_item("name", name.ast_to_object(to_ctx), vm)
            .unwrap();
        dict.set_item("default_value", default.ast_to_object(to_ctx), vm)
            .unwrap();
        node_add_location(&dict, range, vm, source_file);
        node.into()
    }

    fn ast_from_object(
        ctx: &AstFromObjectContext<'_>,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = type_param_range_from_object(ctx, source_file, object.clone())?;
        type_var_tuple_from_object_with_range(ctx, source_file, object, range)
    }
}
