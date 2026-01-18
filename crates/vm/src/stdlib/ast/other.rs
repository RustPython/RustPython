use super::*;
use rustpython_compiler_core::SourceFile;

impl Node for ast::ConversionFlag {
    fn ast_to_object(self, vm: &VirtualMachine, _source_file: &SourceFile) -> PyObjectRef {
        vm.ctx.new_int(self as u8).into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        _source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        // Python's AST uses ASCII codes: 's', 'r', 'a', -1=None
        // Note: 255 is -1i8 as u8 (ruff's ConversionFlag::None)
        match i32::try_from_object(vm, object)? {
            -1 | 255 => Ok(Self::None),
            x if x == b's' as i32 => Ok(Self::Str),
            x if x == b'r' as i32 => Ok(Self::Repr),
            x if x == b'a' as i32 => Ok(Self::Ascii),
            _ => Err(vm.new_value_error("invalid conversion flag")),
        }
    }
}

// /// This is just a string, not strictly an AST node. But it makes AST conversions easier.
impl Node for ast::name::Name {
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

impl Node for ast::Decorator {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        ast::Expr::ast_to_object(self.expression, vm, source_file)
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let expression = ast::Expr::ast_from_object(vm, source_file, object)?;
        let range = expression.range();
        Ok(Self {
            node_index: Default::default(),
            expression,
            range,
        })
    }
}

// product
impl Node for ast::Alias {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
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
            node_index: Default::default(),
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
impl Node for ast::WithItem {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
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
            node_index: Default::default(),
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
