use super::*;
use crate::stdlib::_ast::type_ignore::TypeIgnore;
use rustpython_compiler_core::SourceFile;

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
    Module(ModModule),
    Interactive(ModInteractive),
    Expression(ast::ModExpression),
    FunctionType(ModFunctionType),
}

// sum
impl Node for Mod {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        match self {
            Self::Module(cons) => cons.ast_to_object(vm, source_file),
            Self::Interactive(cons) => cons.ast_to_object(vm, source_file),
            Self::Expression(cons) => cons.ast_to_object(vm, source_file),
            Self::FunctionType(cons) => cons.ast_to_object(vm, source_file),
        }
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        Ok(
            if object.is_instance(pyast::NodeModModule::static_type().as_object(), vm)? {
                Self::Module(ModModule::ast_from_object(vm, source_file, object)?)
            } else if object
                .is_instance(pyast::NodeModInteractive::static_type().as_object(), vm)?
            {
                Self::Interactive(ModInteractive::ast_from_object(vm, source_file, object)?)
            } else if object.is_instance(pyast::NodeModExpression::static_type().as_object(), vm)? {
                Self::Expression(ast::ModExpression::ast_from_object(
                    vm,
                    source_file,
                    object,
                )?)
            } else if object
                .is_instance(pyast::NodeModFunctionType::static_type().as_object(), vm)?
            {
                Self::FunctionType(ModFunctionType::ast_from_object(vm, source_file, object)?)
            } else {
                return Err(vm.new_type_error(format!(
                    "expected some sort of mod, but got {}",
                    object.repr(vm)?
                )));
            },
        )
    }
}

pub(super) struct ModModule {
    pub(crate) module: ast::ModModule,
    pub(crate) type_ignores: Vec<TypeIgnore>,
}

// constructor
impl Node for ModModule {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            module,
            type_ignores,
        } = self;
        let ast::ModModule {
            node_index: _,
            body,
            range: _,
            runtime_body,
        } = module;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeModModule::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        let body = runtime_body.map_or_else(
            || body.ast_to_object(vm, source_file),
            |values| values.ast_to_object(vm, source_file),
        );
        dict.set_item("body", body, vm).unwrap();
        dict.set_item(
            "type_ignores",
            type_ignores.ast_to_object(vm, source_file),
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
        let body: Vec<Option<ast::Stmt>> =
            get_node_list_field(vm, source_file, &object, "body", "Module")?;
        let (runtime_body, body) = runtime_stmt_list_from_values(body);
        let type_ignores = get_node_list_field(vm, source_file, &object, "type_ignores", "Module")?;
        Ok(Self {
            module: ast::ModModule {
                node_index: Default::default(),
                body,
                range: Default::default(),
                runtime_body,
            },
            type_ignores,
        })
    }
}

pub(super) struct ModInteractive {
    pub(crate) range: TextRange,
    pub(crate) body: ast::Suite,
    pub(crate) runtime_body: Option<Vec<Option<ast::Stmt>>>,
}

// constructor
impl Node for ModInteractive {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            body,
            range: _,
            runtime_body,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeModInteractive::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        let body = runtime_body.map_or_else(
            || body.ast_to_object(vm, source_file),
            |values| values.ast_to_object(vm, source_file),
        );
        dict.set_item("body", body, vm).unwrap();
        node.into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let body: Vec<Option<ast::Stmt>> =
            get_node_list_field(vm, source_file, &object, "body", "Interactive")?;
        let (runtime_body, body) = runtime_stmt_list_from_values(body);
        Ok(Self {
            body,
            range: Default::default(),
            runtime_body,
        })
    }
}

// constructor
impl Node for ast::ModExpression {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            node_index: _,
            body,
            range: _,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeModExpression::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        dict.set_item("body", body.ast_to_object(vm, source_file), vm)
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
            body: get_required_node_field(vm, source_file, &object, "body", "Expression")?,
            range: Default::default(),
        })
    }
}

pub(super) struct ModFunctionType {
    pub(crate) argtypes: Box<[ast::Expr]>,
    pub(crate) returns: ast::Expr,
    pub(crate) runtime_argtypes: Option<Vec<Option<ast::Expr>>>,
}

// constructor
impl Node for ModFunctionType {
    fn ast_to_object(self, vm: &VirtualMachine, source_file: &SourceFile) -> PyObjectRef {
        let Self {
            argtypes,
            returns,
            runtime_argtypes,
        } = self;
        let node = NodeAst
            .into_ref_with_type(vm, pyast::NodeModFunctionType::static_type().to_owned())
            .unwrap();
        let dict = node.as_object().dict().unwrap();
        let argtypes = runtime_argtypes.map_or_else(
            || BoxedSlice(argtypes).ast_to_object(vm, source_file),
            |values| values.ast_to_object(vm, source_file),
        );
        dict.set_item("argtypes", argtypes, vm).unwrap();
        dict.set_item("returns", returns.ast_to_object(vm, source_file), vm)
            .unwrap();
        node.into()
    }

    fn ast_from_object(
        vm: &VirtualMachine,
        source_file: &SourceFile,
        object: PyObjectRef,
    ) -> PyResult<Self> {
        let argtypes: Vec<Option<ast::Expr>> =
            get_node_list_field(vm, source_file, &object, "argtypes", "FunctionType")?;
        let (runtime_argtypes, argtypes) = runtime_expr_list_from_values(argtypes);
        Ok(Self {
            argtypes: argtypes.into_boxed_slice(),
            returns: get_required_node_field(vm, source_file, &object, "returns", "FunctionType")?,
            runtime_argtypes,
        })
    }
}
