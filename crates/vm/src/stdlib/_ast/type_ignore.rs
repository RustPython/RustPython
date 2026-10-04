use super::*;
use rustpython_compiler_core::SourceFile;

pub(super) enum TypeIgnore {
    None,
    TypeIgnore(TypeIgnoreTypeIgnore),
}

// sum
impl Node for TypeIgnore {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        match self {
            Self::None => vm.ctx.none(),
            Self::TypeIgnore(cons) => cons.ast_to_object(vm, source_file),
        }
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(if vm.is_none(&object) {
            Self::None
        } else if is_node_instance(vm, &object, pyast::NodeTypeIgnoreTypeIgnore::static_type())? {
            Self::TypeIgnore(TypeIgnoreTypeIgnore::ast_from_object(
                vm,
                source_file,
                object,
            )?)
        } else {
            return Err(vm.new_type_error(format!(
                "expected some sort of type_ignore, but got {}",
                object.repr(vm)?
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
    fn ast_to_object(self, vm: &VirtualMachine, _source_file: &SourceFile) -> PyObjectRef {
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
        node.into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        _source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            lineno: get_int_field(vm, &object, "lineno", "TypeIgnore")?,
            tag: node_object_to_ast_string(vm, get_node_field(vm, &object, "tag", "TypeIgnore")?)?,
        })
    }
}
