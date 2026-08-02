use super::*;
use rustpython_compiler_core::SourceFile;

fn ensure_excepthandler_node(vm: &VirtualMachine, object: &PyObjectRef) -> PyResult<()> {
    if vm.is_none(object)
        || !is_node_instance(
            vm,
            object,
            pyast::NodeExceptHandlerExceptHandler::static_type(),
        )?
    {
        return Err(vm.new_type_error(format!(
            "expected some sort of excepthandler, but got {}",
            object.repr(vm)?
        )));
    }
    Ok(())
}

// sum
impl Node for ast::ExceptHandler {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        match self {
            Self::ExceptHandler(cons) => cons.ast_to_object(vm, source_file),
        }
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        ensure_excepthandler_node(vm, &object)?;
        let range = excepthandler_range_from_object(vm, source_file, object.clone())?;
        Ok(Self::ExceptHandler(except_handler_from_object_with_range(
            vm,
            source_file,
            object,
            range,
        )?))
    }
}

// constructor
fn except_handler_from_object_with_range(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    object: PyObjectRef,
    range: TextRange,
) -> PyResult<ast::ExceptHandlerExceptHandler> {
    let body: Vec<Option<ast::Stmt>> =
        get_node_list_field(vm, source_file, &object, "body", "ExceptHandler")?;
    let (runtime_body, body) = runtime_stmt_list_from_values(body);
    Ok(ast::ExceptHandlerExceptHandler {
        node_index: Default::default(),
        type_: get_node_field_opt(vm, &object, "type")?
            .map(|obj| Node::ast_from_object(vm, source_file, obj))
            .transpose()?,
        name: get_node_field_opt(vm, &object, "name")?
            .map(|obj| Node::ast_from_object(vm, source_file, obj))
            .transpose()?,
        body,
        range,
        runtime_body,
    })
}

pub(super) fn except_handler_from_object_unvalidated_range(
    vm: &VirtualMachine,
    source_file: &SourceFile,
    object: PyObjectRef,
) -> PyResult<ast::ExceptHandler> {
    ensure_excepthandler_node(vm, &object)?;
    let range = excepthandler_range_from_object_unvalidated(vm, source_file, object.clone())?;
    Ok(ast::ExceptHandler::ExceptHandler(
        except_handler_from_object_with_range(vm, source_file, object, range)?,
    ))
}

impl Node for ast::ExceptHandlerExceptHandler {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            type_,
            name,
            body,
            range,
            runtime_body,
        } = self;
        let node = NodeAst
            .into_ref_with_type(
                vm,
                pyast::NodeExceptHandlerExceptHandler::static_type().to_owned(),
            )
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("type", type_.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item("name", name.ast_to_object(vm, source_file), vm)
            .unwrap();
        let body = runtime_body.map_or_else(
            || body.ast_to_object(vm, source_file),
            |values| values.ast_to_object(vm, source_file),
        );
        dict.set_item("body", body, vm).unwrap();
        node_add_location(&dict, range, vm, source_file);
        node.into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let range = range_from_object(vm, source_file, object.clone(), "ExceptHandler")?;
        except_handler_from_object_with_range(vm, source_file, object, range)
    }
}
