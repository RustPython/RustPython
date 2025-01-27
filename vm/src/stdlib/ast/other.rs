use super::*;
use num_traits::ToPrimitive;

impl Node for ruff::ConversionFlag {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_int(self as u8).into()
    }

    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        i32::try_from_object(vm, object)?
            .to_u32()
            .and_then(ruff::ConversionFlag::from_op_arg)
            .ok_or_else(|| vm.new_value_error("invalid conversion flag".to_owned()))
    }
}

/// This is just a string, not strictly an AST node. But it makes AST conversions easier.
// impl Node for ruff::name::Name {
//     fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
//         vm.ctx.new_str(self.as_str()).to_pyobject(vm)
//     }

//     fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
//         match object.downcast::<PyStr>() {
//             Ok(name) => Ok(Self::new(name)),
//             Err(_) => Err(vm.new_value_error("expected str for name".to_owned())),
//         }
//     }
// }

impl Node for ruff::Decorator {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        todo!()
    }

    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        todo!()
    }
}

// product
impl Node for ruff::Alias {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            name,
            asname,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, gen::NodeAlias::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("name", name.ast_to_object(vm), vm).unwrap();
        dict.set_item("asname", asname.ast_to_object(vm), vm)
            .unwrap();
        node_add_location(&dict, _range, vm);
        node.into()
    }
    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            name: Node::ast_from_object(vm, get_node_field(vm, &object, "name", "alias")?)?,
            asname: get_node_field_opt(vm, &object, "asname")?
                .map(|obj| Node::ast_from_object(vm, obj))
                .transpose()?,
            range: range_from_object(vm, object, "alias")?,
        })
    }
}
// product
impl Node for ruff::WithItem {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            context_expr,
            optional_vars,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, gen::NodeWithItem::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("context_expr", context_expr.ast_to_object(vm), vm)
            .unwrap();
        dict.set_item("optional_vars", optional_vars.ast_to_object(vm), vm)
            .unwrap();
        node.into()
    }
    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            context_expr: Node::ast_from_object(
                vm,
                get_node_field(vm, &object, "context_expr", "withitem")?,
            )?,
            optional_vars: get_node_field_opt(vm, &object, "optional_vars")?
                .map(|obj| Node::ast_from_object(vm, obj))
                .transpose()?,
            range: Default::default(),
        })
    }
}
