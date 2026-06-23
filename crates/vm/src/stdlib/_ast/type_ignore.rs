use super::*;
use rustpython_compiler_core::SourceFile;

pub(super) enum TypeIgnore {
    None,
    TypeIgnore(TypeIgnoreTypeIgnore),
}

// sum
impl Node for TypeIgnore {
    fn ast_to_object(self, to_ctx: &AstToObjectContext<'_>) -> PyObjectRef {
        let vm = to_ctx.vm;
        let _source_file = to_ctx.source_file;
        match self {
            Self::None => vm.ctx.none(),
            Self::TypeIgnore(cons) => cons.ast_to_object(to_ctx),
        }
    }
    fn ast_from_object(
        ctx: &AstFromObjectContext<'_>,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(if ctx.is_none(&object) {
            Self::None
        } else if is_node_instance(ctx, &object, pyast::NodeTypeIgnoreTypeIgnore::static_type())? {
            Self::TypeIgnore(TypeIgnoreTypeIgnore::ast_from_object(
                ctx,
                source_file,
                object,
            )?)
        } else {
            return Err(ctx.new_type_error(format!(
                "expected some sort of type_ignore, but got {}",
                object.repr(ctx)?
            )));
        })
    }
}

pub(super) struct TypeIgnoreTypeIgnore {
    lineno: i32,
    tag: PyObjectRef,
}

// constructor
impl Node for TypeIgnoreTypeIgnore {
    fn ast_to_object(self, to_ctx: &AstToObjectContext<'_>) -> PyObjectRef {
        let vm = to_ctx.vm;
        let source_file = to_ctx.source_file;
        let Self { lineno, tag } = self;
        let node = NodeAst
            .into_ref_with_type(
                vm,
                pyast::NodeTypeIgnoreTypeIgnore::static_type().to_owned(),
            )
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("lineno", vm.ctx.new_int(lineno).into(), vm)
            .unwrap();
        dict.set_item("tag", tag, vm).unwrap();
        let _ = source_file;
        node.into()
    }

    fn ast_from_object(
        ctx: &AstFromObjectContext<'_>,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let _ = source_file;
        Ok(Self {
            lineno: get_int_field(ctx, &object, "lineno", "TypeIgnore")?,
            tag: node_object_to_ast_string(
                ctx,
                get_node_field(ctx, &object, "tag", "TypeIgnore")?,
            )?,
        })
    }
}
