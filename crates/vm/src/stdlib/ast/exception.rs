use super::*;
use rustpython_compiler_core::SourceFile;

// sum
impl Node for ast::ExceptHandler {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        match self {
            Self::ExceptHandler(cons) => cons.ast_to_object(vm, source_file),
        }
    }
    fn ast_from_object(
        _vm: &VirtualMachine,
        source_file: &SourceFile,
        _object: PyObjectRef,
    ) -> PyResult<Self> {
        let _cls = _object.class();
        Ok(
            if _cls.is(pyast::NodeExceptHandlerExceptHandler::static_type()) {
                Self::ExceptHandler(ast::ExceptHandlerExceptHandler::ast_from_object(
                    _vm,
                    source_file,
                    _object,
                )?)
            } else {
                return Err(_vm.new_type_error(format!(
                    "expected some sort of excepthandler, but got {}",
                    _object.repr(_vm)?
                )));
            },
        )
    }
}

// constructor
impl Node for ast::ExceptHandlerExceptHandler {
    fn ast_to_object(self, _vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            type_,
            name,
            body,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(
                _vm,
                pyast::NodeExceptHandlerExceptHandler::static_type().to_owned(),
            )
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("type", type_.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        dict.set_item("name", name.ast_to_object(_vm, source_file), _vm)
            .unwrap();
        dict.set_item("body", body.ast_to_object(_vm, source_file), _vm)
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
            type_: get_node_field_opt(_vm, &_object, "type")?
                .map(|obj| Node::ast_from_object(_vm, source_file, obj))
                .transpose()?,
            name: get_node_field_opt(_vm, &_object, "name")?
                .map(|obj| Node::ast_from_object(_vm, source_file, obj))
                .transpose()?,
            body: Node::ast_from_object(
                _vm,
                source_file,
                get_node_field(_vm, &_object, "body", "ExceptHandler")?,
            )?,
            range: range_from_object(_vm, source_file, _object, "ExceptHandler")?,
        })
    }
}
