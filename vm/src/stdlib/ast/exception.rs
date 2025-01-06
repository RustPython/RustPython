use super::*;

// sum
impl Node for ruff::ExceptHandler {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        match self {
            ruff::ExceptHandler::ExceptHandler(cons) => cons.ast_to_object(vm),
        }
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        let _cls = _object.class();
        Ok(
            if _cls.is(gen::NodeExceptHandlerExceptHandler::static_type()) {
                ruff::ExceptHandler::ExceptHandler(
                    ruff::ExceptHandlerExceptHandler::ast_from_object(_vm, _object)?,
                )
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
impl Node for ruff::ExceptHandlerExceptHandler {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let ruff::ExceptHandlerExceptHandler {
            type_,
            name,
            body,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(
                _vm,
                gen::NodeExceptHandlerExceptHandler::static_type().to_owned(),
            )
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("type", type_.ast_to_object(_vm), _vm)
            .unwrap();
        dict.set_item("name", name.ast_to_object(_vm), _vm).unwrap();
        dict.set_item("body", body.ast_to_object(_vm), _vm).unwrap();
        node_add_location(&dict, _range, _vm);
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(ruff::ExceptHandlerExceptHandler {
            type_: get_node_field_opt(_vm, &_object, "type")?
                .map(|obj| Node::ast_from_object(_vm, obj))
                .transpose()?,
            name: get_node_field_opt(_vm, &_object, "name")?
                .map(|obj| Node::ast_from_object(_vm, obj))
                .transpose()?,
            body: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "body", "ExceptHandler")?,
            )?,
            range: range_from_object(_vm, _object, "ExceptHandler")?,
        })
    }
}
