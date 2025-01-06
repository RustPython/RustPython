use super::*;

/// Represents the different types of Python module structures.
///
/// This enum is used to represent the various possible forms of a Python module
/// in an Abstract Syntax Tree (AST). It can correspond to:
///
/// - `Module`: A standard Python script, containing a sequence of statements
///   (e.g., assignments, function calls), possibly with type ignores.
/// - `Interactive`: A representation of code executed in an interactive
///   Python session (e.g., the REPL or Jupyter notebooks), where statements
///   are evaluated one at a time.
/// - `Expression`: A single expression without any surrounding statements.
///   This is typically used in scenarios like `eval()` or in expression-only
///   contexts.
/// - `FunctionType`: A function signature with argument and return type
///   annotations, representing the type hints of a function (e.g., `def add(x: int, y: int) -> int`).
pub(super) enum Mod {
    Module(ruff::ModModule),
    Interactive(ModInteractive),
    Expression(ruff::ModExpression),
    FunctionType(ModFunctionType),
}

// sum
impl Node for Mod {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        match self {
            Self::Module(cons) => cons.ast_to_object(vm),
            Self::Interactive(cons) => cons.ast_to_object(vm),
            Self::Expression(cons) => cons.ast_to_object(vm),
            Self::FunctionType(cons) => cons.ast_to_object(vm),
        }
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        let _cls = _object.class();
        Ok(if _cls.is(gen::NodeModModule::static_type()) {
            Self::Module(ruff::ModModule::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeModInteractive::static_type()) {
            Self::Interactive(ModInteractive::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeModExpression::static_type()) {
            Self::Expression(ruff::ModExpression::ast_from_object(_vm, _object)?)
        } else if _cls.is(gen::NodeModFunctionType::static_type()) {
            Self::FunctionType(ModFunctionType::ast_from_object(_vm, _object)?)
        } else {
            return Err(_vm.new_type_error(format!(
                "expected some sort of mod, but got {}",
                _object.repr(_vm)?
            )));
        })
    }
}
// constructor
impl Node for ruff::ModModule {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let ruff::ModModule {
            body,
            // type_ignores,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, gen::NodeModModule::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("body", body.ast_to_object(vm), vm).unwrap();
        // TODO: ruff ignores type_ignore comments currently.
        // dict.set_item("type_ignores", type_ignores.ast_to_object(_vm), _vm)
        //     .unwrap();
        node.into()
    }
    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        Ok(ruff::ModModule {
            body: Node::ast_from_object(vm, get_node_field(vm, &object, "body", "Module")?)?,
            // type_ignores: Node::ast_from_object(
            //     _vm,
            //     get_node_field(_vm, &_object, "type_ignores", "Module")?,
            // )?,
            range: Default::default(),
        })
    }
}

pub(super) struct ModInteractive {
    pub(crate) range: TextRange,
    pub(crate) body: Vec<ruff::Stmt>,
}

// constructor
impl Node for ModInteractive {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            body,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeModInteractive::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("body", body.ast_to_object(_vm), _vm).unwrap();
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            body: Node::ast_from_object(
                _vm,
                get_node_field(_vm, &_object, "body", "Interactive")?,
            )?,
            range: Default::default(),
        })
    }
}
// constructor
impl Node for ruff::ModExpression {
    fn ast_to_object(self, _vm: &VirtualMachine) -> PyObjectRef {
        let Self {
            body,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(_vm, gen::NodeModExpression::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("body", body.ast_to_object(_vm), _vm).unwrap();
        node.into()
    }
    fn ast_from_object(_vm: &VirtualMachine, _object: PyObjectRef) -> PyResult<Self> {
        Ok(Self {
            body: Node::ast_from_object(_vm, get_node_field(_vm, &_object, "body", "Expression")?)?,
            range: Default::default(),
        })
    }
}

pub(super) struct ModFunctionType {
    pub(crate) argtypes: Box<[ruff::Expr]>,
    pub(crate) returns: ruff::Expr,
    pub(crate) range: TextRange,
}

// constructor
impl Node for ModFunctionType {
    fn ast_to_object(self, vm: &VirtualMachine) -> PyObjectRef {
        let ModFunctionType {
            argtypes,
            returns,
            range: _range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, gen::NodeModFunctionType::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("argtypes", BoxedSlice(argtypes).ast_to_object(vm), vm)
            .unwrap();
        dict.set_item("returns", returns.ast_to_object(vm), vm)
            .unwrap();
        node.into()
    }
    fn ast_from_object(vm: &VirtualMachine, object: PyObjectRef) -> PyResult<Self> {
        Ok(ModFunctionType {
            argtypes: {
                let argtypes: BoxedSlice<_> = Node::ast_from_object(
                    vm,
                    get_node_field(vm, &object, "argtypes", "FunctionType")?,
                )?;
                argtypes.0
            },
            returns: Node::ast_from_object(
                vm,
                get_node_field(vm, &object, "returns", "FunctionType")?,
            )?,
            range: Default::default(),
        })
    }
}
