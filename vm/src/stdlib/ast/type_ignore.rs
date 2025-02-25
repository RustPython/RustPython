use super::*;

pub(super) enum TypeIgnore {
    TypeIgnore(TypeIgnoreTypeIgnore),
}

// sum
impl Node for TypeIgnore {
    fn ast_to_object(self, vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        match self {
            TypeIgnore::TypeIgnore(cons) => cons.ast_to_object(vm, source_code),
        }
    }
    fn ast_from_object(
        _vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        let _cls = _object.class();
        Ok(if _cls.is(gen::NodeTypeIgnoreTypeIgnore::static_type()) {
            TypeIgnore::TypeIgnore(TypeIgnoreTypeIgnore::ast_from_object(
                _vm,
                source_code,
                _object,
            )?)
        } else {
            return Err(_vm.new_type_error(format!(
                "expected some sort of type_ignore, but got {}",
                _object.repr(_vm)?
            )));
        })
    }
}

pub(super) struct TypeIgnoreTypeIgnore {
    range: TextRange,
    lineno: PyRefExact<PyInt>,
    tag: PyRefExact<PyStr>,
}

// constructor
impl Node for TypeIgnoreTypeIgnore {
    fn ast_to_object(self, vm: &VirtualMachine, _source_code: &SourceCodeOwned) -> PyObjectRef {
        let Self {
            lineno,
            tag,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, gen::NodeTypeIgnoreTypeIgnore::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("lineno", lineno.to_pyobject(vm), vm).unwrap();
        dict.set_item("tag", tag.to_pyobject(vm), vm).unwrap();
        node.into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        _source_code: &SourceCodeOwned,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            lineno: get_node_field(vm, &object, "lineno", "TypeIgnore")?
                .downcast_exact(vm)
                .unwrap(),
            tag: get_node_field(vm, &object, "tag", "TypeIgnore")?
                .downcast_exact(vm)
                .unwrap(),
            range: Default::default(),
        })
    }
}
