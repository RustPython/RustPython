use super::*;
use crate::stdlib::ast::type_ignore::TypeIgnore;

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
    fn ast_to_object(self, vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        match self {
            Self::Module(cons) => cons.ast_to_object(vm, source_code),
            Self::Interactive(cons) => cons.ast_to_object(vm, source_code),
            Self::Expression(cons) => cons.ast_to_object(vm, source_code),
            Self::FunctionType(cons) => cons.ast_to_object(vm, source_code),
        }
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let cls = object.class();
        Ok(if cls.is(pyast::NodeModModule::static_type()) {
            Self::Module(ruff::ModModule::ast_from_object(vm, source_code, object)?)
        } else if cls.is(pyast::NodeModInteractive::static_type()) {
            Self::Interactive(ModInteractive::ast_from_object(vm, source_code, object)?)
        } else if cls.is(pyast::NodeModExpression::static_type()) {
            Self::Expression(ruff::ModExpression::ast_from_object(
                vm,
                source_code,
                object,
            )?)
        } else if cls.is(pyast::NodeModFunctionType::static_type()) {
            Self::FunctionType(ModFunctionType::ast_from_object(vm, source_code, object)?)
        } else {
            return Err(vm.new_type_error(format!(
                "expected some sort of mod, but got {}",
                object.repr(vm)?
            )));
        })
    }
}
// constructor
impl Node for ruff::ModModule {
    fn ast_to_object(self, vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let ruff::ModModule {
            body,
            // type_ignores,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeModModule::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("body", body.ast_to_object(vm, source_code), vm)
            .unwrap();
        // TODO: Improve ruff API
        // ruff ignores type_ignore comments currently.
        let type_ignores: Vec<TypeIgnore> = vec![];
        dict.set_item(
            "type_ignores",
            type_ignores.ast_to_object(vm, source_code),
            vm,
        )
        .unwrap();
        node_add_location(&dict, range, vm, source_code);
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(ruff::ModModule {
            body: Node::ast_from_object(
                vm,
                source_code,
                get_node_field(vm, &object, "body", "Module")?,
            )?,
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
    fn ast_to_object(self, vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let Self { body, range } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeModInteractive::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("body", body.ast_to_object(vm, source_code), vm)
            .unwrap();
        node_add_location(&dict, range, vm, source_code);
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            body: Node::ast_from_object(
                vm,
                source_code,
                get_node_field(vm, &object, "body", "Interactive")?,
            )?,
            range: Default::default(),
        })
    }
}
// constructor
impl Node for ruff::ModExpression {
    fn ast_to_object(self, vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let Self { body, range } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeModExpression::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("body", body.ast_to_object(vm, source_code), vm)
            .unwrap();
        node_add_location(&dict, range, vm, source_code);
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(Self {
            body: Node::ast_from_object(
                vm,
                source_code,
                get_node_field(vm, &object, "body", "Expression")?,
            )?,
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
    fn ast_to_object(self, vm: &VirtualMachine, source_code: &SourceCodeOwned) -> PyObjectRef {
        let ModFunctionType {
            argtypes,
            returns,
            range,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeModFunctionType::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item(
            "argtypes",
            BoxedSlice(argtypes).ast_to_object(vm, source_code),
            vm,
        )
        .unwrap();
        dict.set_item("returns", returns.ast_to_object(vm, source_code), vm)
            .unwrap();
        node_add_location(&dict, range, vm, source_code);
        node.into()
    }
    fn ast_from_object(
        vm: &VirtualMachine,
        source_code: &SourceCodeOwned,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(ModFunctionType {
            argtypes: {
                let argtypes: BoxedSlice<_> = Node::ast_from_object(
                    vm,
                    source_code,
                    get_node_field(vm, &object, "argtypes", "FunctionType")?,
                )?;
                argtypes.0
            },
            returns: Node::ast_from_object(
                vm,
                source_code,
                get_node_field(vm, &object, "returns", "FunctionType")?,
            )?,
            range: Default::default(),
        })
    }
}
