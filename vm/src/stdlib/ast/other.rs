use super::*;
use num_traits::ToPrimitive;
use rustpython_compiler_core::{SourceFile, bytecode};

impl Node for ruff::ConversionFlag {
    fn ast_to_object(self, vm: &VirtualMachine, _source_file: &SourceFile) -> PyObjectRef {
        vm.ctx.new_int(self as u8).into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        _source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        i32::try_from_object(vm, object)?
            .to_u32()
            .and_then(bytecode::ConversionFlag::from_op_arg)
            .map(|flag| match flag {
                bytecode::ConversionFlag::None => Self::None,
                bytecode::ConversionFlag::Str => Self::Str,
                bytecode::ConversionFlag::Ascii => Self::Ascii,
                bytecode::ConversionFlag::Repr => Self::Repr,
            })
            .ok_or_else(|| vm.new_value_error("invalid conversion flag"))
    }
}

// /// This is just a string, not strictly an AST node. But it makes AST conversions easier.
impl Node for ruff::name::Name {
    fn ast_to_object(self, vm: &VirtualMachine, _source_file: &SourceFile) -> PyObjectRef {
        vm.ctx.new_str(self.as_str()).to_pyobject(vm)
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        _source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        match object.downcast::<PyStr>() {
            Ok(name) => Ok(Self::new(name)),
            Err(_) => Err(vm.new_value_error("expected str for name")),
        }
    }
}

impl Node for ruff::Decorator {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        ruff::Expr::ast_to_object(self.expression, vm, source_file)
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let expression = ruff::Expr::ast_from_object(vm, source_file, object)?;
        let range = expression.range();
        Ok(Self { expression, range })
    }
}

// product
impl Node for ruff::Alias {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            name,
            asname,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeAlias::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("name", name.ast_to_object(vm, source_file), vm)
            .unwrap();
        dict.set_item("asname", asname.ast_to_object(vm, source_file), vm)
            .unwrap();
        node_add_location(&dict, _range, vm, source_file);
        node.into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            name: Node::ast_from_object(
                vm,
                source_file,
                get_node_field(vm, &object, "name", "alias")?,
            )?,
            asname: get_node_field_opt(vm, &object, "asname")?
                .map(|obj| Node::ast_from_object(vm, source_file, obj))
                .transpose()?,
            range: range_from_object(vm, source_file, object, "alias")?,
        })
    }
}

// product
impl Node for ruff::WithItem {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            context_expr,
            optional_vars,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeWithItem::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item(
            "context_expr",
            context_expr.ast_to_object(vm, source_file),
            vm,
        )
        .unwrap();
        dict.set_item(
            "optional_vars",
            optional_vars.ast_to_object(vm, source_file),
            vm,
        )
        .unwrap();
        node.into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            context_expr: Node::ast_from_object(
                vm,
                source_file,
                get_node_field(vm, &object, "context_expr", "withitem")?,
            )?,
            optional_vars: get_node_field_opt(vm, &object, "optional_vars")?
                .map(|obj| Node::ast_from_object(vm, source_file, obj))
                .transpose()?,
            range: Default::default(),
        })
    }
}
