use super::*;
use rustpython_compiler_core::SourceFile;

impl Node for ast::ConversionFlag {
    fn ast_to_object(self, vm: &VirtualMachine, _source_file: &SourceFile) -> PyObjectRef {
        vm.ctx.new_int(self as i8).into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        _source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        match node_object_to_i32(vm, object)? {
            -1 => Ok(Self::None),
            x if x == b's' as i32 => Ok(Self::Str),
            x if x == b'r' as i32 => Ok(Self::Repr),
            x if x == b'a' as i32 => Ok(Self::Ascii),
            x => Err(vm.new_system_error(format!("Unrecognized conversion character {x}"))),
        }
    }
}

// /// This is just a string, not strictly an AST node. But it makes AST conversions easier.
impl Node for ast::name::Name {
    fn ast_to_object(self, vm: &VirtualMachine, _source_file: &SourceFile) -> PyObjectRef {
        vm.ctx.intern_str(self.as_str()).to_object()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        _source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        if !object.class().is(vm.ctx.types.str_type) {
            return Err(vm.new_type_error("AST identifier must be of type str"));
        }
        object
            .downcast::<PyStr>()
            .map(Self::new)
            .map_err(|_| vm.new_type_error("AST identifier must be of type str"))
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
            name: get_required_identifier_field(vm, source_file, &object, "name", "alias")?,
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
            context_expr: get_required_node_field(
                vm,
                source_file,
                &object,
                "context_expr",
                "withitem",
            )?,
            optional_vars: get_node_field_opt(vm, &object, "optional_vars")?
                .map(|obj| Node::ast_from_object(vm, source_file, obj))
                .transpose()?,
            range: Default::default(),
        })
    }
}
