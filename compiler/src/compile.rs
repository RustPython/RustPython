//!
//! Take an AST and transform it into bytecode
//!
//! Inspirational code:
//!   https://github.com/python/cpython/blob/master/Python/compile.c
//!   https://github.com/micropython/micropython/blob/master/py/compile.c

use crate::bytecode::{self, CallType, CodeObject, Instruction, Varargs};
use crate::error::{CompileError, CompileErrorType};
use crate::symboltable::{make_symbol_table, statements_to_symbol_table, SymbolRole, SymbolScope};
use num_complex::Complex64;
use rustpython_parser::{ast, parser};

struct Compiler {
    code_object_stack: Vec<CodeObject>,
    scope_stack: Vec<SymbolScope>,
    nxt_label: usize,
    source_path: Option<String>,
    current_source_location: ast::Location,
    current_qualified_path: Option<String>,
    in_loop: bool,
    in_function_def: bool,
}

/// Compile a given sourcecode into a bytecode object.
pub fn compile(source: &str, mode: &Mode, source_path: String) -> Result<CodeObject, CompileError> {
    match mode {
        Mode::Exec => {
            let ast = parser::parse_program(source)?;
            compile_program(ast, source_path)
        }
        Mode::Eval => {
            let statement = parser::parse_statement(source)?;
            compile_statement_eval(statement, source_path)
        }
        Mode::Single => {
            let ast = parser::parse_program(source)?;
            compile_program_single(ast, source_path)
        }
    }
}

/// A helper function for the shared code of the different compile functions
fn with_compiler(
    source_path: String,
    f: impl FnOnce(&mut Compiler) -> Result<(), CompileError>,
) -> Result<CodeObject, CompileError> {
    let mut compiler = Compiler::new();
    compiler.source_path = Some(source_path);
    compiler.push_new_code_object("<module>".to_string());
    f(&mut compiler)?;
    let code = compiler.pop_code_object();
    trace!("Compilation completed: {:?}", code);
    Ok(code)
}

/// Compile a standard Python program to bytecode
pub fn compile_program(ast: ast::Program, source_path: String) -> Result<CodeObject, CompileError> {
    with_compiler(source_path, |compiler| {
        let symbol_table = make_symbol_table(&ast)?;
        compiler.compile_program(&ast, symbol_table)
    })
}

/// Compile a single Python expression to bytecode
pub fn compile_statement_eval(
    statement: Vec<ast::LocatedStatement>,
    source_path: String,
) -> Result<CodeObject, CompileError> {
    with_compiler(source_path, |compiler| {
        let symbol_table = statements_to_symbol_table(&statement)?;
        compiler.compile_statement_eval(&statement, symbol_table)
    })
}

/// Compile a Python program to bytecode for the context of a REPL
pub fn compile_program_single(
    ast: ast::Program,
    source_path: String,
) -> Result<CodeObject, CompileError> {
    with_compiler(source_path, |compiler| {
        let symbol_table = make_symbol_table(&ast)?;
        compiler.compile_program_single(&ast, symbol_table)
    })
}

pub enum Mode {
    Exec,
    Eval,
    Single,
}

#[derive(Clone, Copy)]
enum EvalContext {
    Statement,
    Expression,
}

type Label = usize;

impl Compiler {
    fn new() -> Self {
        Compiler {
            code_object_stack: Vec::new(),
            scope_stack: Vec::new(),
            nxt_label: 0,
            source_path: None,
            current_source_location: ast::Location::default(),
            current_qualified_path: None,
            in_loop: false,
            in_function_def: false,
        }
    }

    fn push_new_code_object(&mut self, obj_name: String) {
        let line_number = self.get_source_line_number();
        self.code_object_stack.push(CodeObject::new(
            Vec::new(),
            Varargs::None,
            Vec::new(),
            Varargs::None,
            self.source_path.clone().unwrap(),
            line_number,
            obj_name,
        ));
    }

    fn pop_code_object(&mut self) -> CodeObject {
        // self.scope_stack.pop().unwrap();
        self.code_object_stack.pop().unwrap()
    }

    fn compile_program(
        &mut self,
        program: &ast::Program,
        symbol_scope: SymbolScope,
    ) -> Result<(), CompileError> {
        let size_before = self.code_object_stack.len();
        self.scope_stack.push(symbol_scope);
        self.compile_statements(&program.statements)?;
        assert!(self.code_object_stack.len() == size_before);

        // Emit None at end:
        self.emit(Instruction::LoadConst {
            value: bytecode::Constant::None,
        });
        self.emit(Instruction::ReturnValue);
        Ok(())
    }

    fn compile_program_single(
        &mut self,
        program: &ast::Program,
        symbol_scope: SymbolScope,
    ) -> Result<(), CompileError> {
        self.scope_stack.push(symbol_scope);

        let mut emitted_return = false;

        for (i, statement) in program.statements.iter().enumerate() {
            let is_last = i == program.statements.len() - 1;

            if let ast::Statement::Expression { ref expression } = statement.node {
                self.compile_expression(expression)?;

                if is_last {
                    self.emit(Instruction::Duplicate);
                    self.emit(Instruction::PrintExpr);
                    self.emit(Instruction::ReturnValue);
                    emitted_return = true;
                } else {
                    self.emit(Instruction::PrintExpr);
                }
            } else {
                self.compile_statement(&statement)?;
            }
        }

        if !emitted_return {
            self.emit(Instruction::LoadConst {
                value: bytecode::Constant::None,
            });
            self.emit(Instruction::ReturnValue);
        }

        Ok(())
    }

    // Compile statement in eval mode:
    fn compile_statement_eval(
        &mut self,
        statements: &[ast::LocatedStatement],
        symbol_table: SymbolScope,
    ) -> Result<(), CompileError> {
        self.scope_stack.push(symbol_table);
        for statement in statements {
            if let ast::Statement::Expression { ref expression } = statement.node {
                self.compile_expression(expression)?;
            } else {
                return Err(CompileError {
                    error: CompileErrorType::ExpectExpr,
                    location: statement.location.clone(),
                });
            }
        }
        self.emit(Instruction::ReturnValue);
        Ok(())
    }

    fn compile_statements(
        &mut self,
        statements: &[ast::LocatedStatement],
    ) -> Result<(), CompileError> {
        for statement in statements {
            self.compile_statement(statement)?
        }
        Ok(())
    }

    fn scope_for_name(&self, name: &str) -> bytecode::NameScope {
        let role = self.lookup_name(name);
        match role {
            SymbolRole::Global => bytecode::NameScope::Global,
            SymbolRole::Nonlocal => bytecode::NameScope::NonLocal,
            _ => bytecode::NameScope::Local,
        }
    }

    fn load_name(&mut self, name: &str) {
        let scope = self.scope_for_name(name);
        self.emit(Instruction::LoadName {
            name: name.to_string(),
            scope,
        });
    }

    fn store_name(&mut self, name: &str) {
        let scope = self.scope_for_name(name);
        self.emit(Instruction::StoreName {
            name: name.to_string(),
            scope,
        });
    }

    fn compile_statement(&mut self, statement: &ast::LocatedStatement) -> Result<(), CompileError> {
        trace!("Compiling {:?}", statement);
        self.set_source_location(&statement.location);

        match &statement.node {
            ast::Statement::Import { import_parts } => {
                for ast::SingleImport {
                    module,
                    symbols,
                    alias,
                } in import_parts
                {
                    if let Some(alias) = alias {
                        // import module as alias
                        self.emit(Instruction::Import {
                            name: module.clone(),
                            symbols: vec![],
                        });
                        self.store_name(&alias);
                    } else if symbols.is_empty() {
                        // import module
                        self.emit(Instruction::Import {
                            name: module.clone(),
                            symbols: vec![],
                        });
                        self.store_name(&module.clone());
                    } else {
                        let import_star = symbols
                            .iter()
                            .any(|import_symbol| import_symbol.symbol == "*");
                        if import_star {
                            // from module import *
                            self.emit(Instruction::ImportStar {
                                name: module.clone(),
                            });
                        } else {
                            // from module import symbol
                            // from module import symbol as alias
                            let (names, symbols_strings): (Vec<String>, Vec<String>) = symbols
                                .iter()
                                .map(|ast::ImportSymbol { symbol, alias }| {
                                    (
                                        alias.clone().unwrap_or_else(|| symbol.to_string()),
                                        symbol.to_string(),
                                    )
                                })
                                .unzip();
                            self.emit(Instruction::Import {
                                name: module.clone(),
                                symbols: symbols_strings,
                            });
                            names.iter().rev().for_each(|name| self.store_name(&name));
                        }
                    }
                }
            }
            ast::Statement::Expression { expression } => {
                self.compile_expression(expression)?;

                // Pop result of stack, since we not use it:
                self.emit(Instruction::Pop);
            }
            ast::Statement::Global { .. } | ast::Statement::Nonlocal { .. } => {
                // Handled during symbol table construction.
            }
            ast::Statement::If { test, body, orelse } => {
                let end_label = self.new_label();
                match orelse {
                    None => {
                        // Only if:
                        self.compile_test(test, None, Some(end_label), EvalContext::Statement)?;
                        self.compile_statements(body)?;
                        self.set_label(end_label);
                    }
                    Some(statements) => {
                        // if - else:
                        let else_label = self.new_label();
                        self.compile_test(test, None, Some(else_label), EvalContext::Statement)?;
                        self.compile_statements(body)?;
                        self.emit(Instruction::Jump { target: end_label });

                        // else:
                        self.set_label(else_label);
                        self.compile_statements(statements)?;
                    }
                }
                self.set_label(end_label);
            }
            ast::Statement::While { test, body, orelse } => {
                let start_label = self.new_label();
                let else_label = self.new_label();
                let end_label = self.new_label();
                self.emit(Instruction::SetupLoop {
                    start: start_label,
                    end: end_label,
                });

                self.set_label(start_label);

                self.compile_test(test, None, Some(else_label), EvalContext::Statement)?;

                let was_in_loop = self.in_loop;
                self.in_loop = true;
                self.compile_statements(body)?;
                self.in_loop = was_in_loop;
                self.emit(Instruction::Jump {
                    target: start_label,
                });
                self.set_label(else_label);
                self.emit(Instruction::PopBlock);
                if let Some(orelse) = orelse {
                    self.compile_statements(orelse)?;
                }
                self.set_label(end_label);
            }
            ast::Statement::With { items, body } => {
                let end_label = self.new_label();
                for item in items {
                    self.compile_expression(&item.context_expr)?;
                    self.emit(Instruction::SetupWith { end: end_label });
                    match &item.optional_vars {
                        Some(var) => {
                            self.compile_store(var)?;
                        }
                        None => {
                            self.emit(Instruction::Pop);
                        }
                    }
                }

                self.compile_statements(body)?;
                for _ in 0..items.len() {
                    self.emit(Instruction::CleanupWith { end: end_label });
                }
                self.set_label(end_label);
            }
            ast::Statement::For {
                target,
                iter,
                body,
                orelse,
            } => self.compile_for(target, iter, body, orelse)?,
            ast::Statement::AsyncFor { .. } => {
                unimplemented!("async for");
            }
            ast::Statement::Raise { exception, cause } => match exception {
                Some(value) => {
                    self.compile_expression(value)?;
                    match cause {
                        Some(cause) => {
                            self.compile_expression(cause)?;
                            self.emit(Instruction::Raise { argc: 2 });
                        }
                        None => {
                            self.emit(Instruction::Raise { argc: 1 });
                        }
                    }
                }
                None => {
                    self.emit(Instruction::Raise { argc: 0 });
                }
            },
            ast::Statement::Try {
                body,
                handlers,
                orelse,
                finalbody,
            } => self.compile_try_statement(body, handlers, orelse, finalbody)?,
            ast::Statement::FunctionDef {
                name,
                args,
                body,
                decorator_list,
                returns,
            } => self.compile_function_def(name, args, body, decorator_list, returns)?,
            ast::Statement::AsyncFunctionDef { .. } => {
                unimplemented!("async def");
            }
            ast::Statement::ClassDef {
                name,
                body,
                bases,
                keywords,
                decorator_list,
            } => self.compile_class_def(name, body, bases, keywords, decorator_list)?,
            ast::Statement::Assert { test, msg } => {
                // TODO: if some flag, ignore all assert statements!

                let end_label = self.new_label();
                self.compile_test(test, Some(end_label), None, EvalContext::Statement)?;
                self.emit(Instruction::LoadName {
                    name: String::from("AssertionError"),
                    scope: bytecode::NameScope::Local,
                });
                match msg {
                    Some(e) => {
                        self.compile_expression(e)?;
                        self.emit(Instruction::CallFunction {
                            typ: CallType::Positional(1),
                        });
                    }
                    None => {
                        self.emit(Instruction::CallFunction {
                            typ: CallType::Positional(0),
                        });
                    }
                }
                self.emit(Instruction::Raise { argc: 1 });
                self.set_label(end_label);
            }
            ast::Statement::Break => {
                if !self.in_loop {
                    return Err(CompileError {
                        error: CompileErrorType::InvalidBreak,
                        location: statement.location.clone(),
                    });
                }
                self.emit(Instruction::Break);
            }
            ast::Statement::Continue => {
                if !self.in_loop {
                    return Err(CompileError {
                        error: CompileErrorType::InvalidContinue,
                        location: statement.location.clone(),
                    });
                }
                self.emit(Instruction::Continue);
            }
            ast::Statement::Return { value } => {
                if !self.in_function_def {
                    return Err(CompileError {
                        error: CompileErrorType::InvalidReturn,
                        location: statement.location.clone(),
                    });
                }
                match value {
                    Some(v) => {
                        self.compile_expression(v)?;
                    }
                    None => {
                        self.emit(Instruction::LoadConst {
                            value: bytecode::Constant::None,
                        });
                    }
                }

                self.emit(Instruction::ReturnValue);
            }
            ast::Statement::Assign { targets, value } => {
                self.compile_expression(value)?;

                for (i, target) in targets.iter().enumerate() {
                    if i + 1 != targets.len() {
                        self.emit(Instruction::Duplicate);
                    }
                    self.compile_store(target)?;
                }
            }
            ast::Statement::AugAssign { target, op, value } => {
                self.compile_expression(target)?;
                self.compile_expression(value)?;

                // Perform operation:
                self.compile_op(op, true);
                self.compile_store(target)?;
            }
            ast::Statement::Delete { targets } => {
                for target in targets {
                    self.compile_delete(target)?;
                }
            }
            ast::Statement::Pass => {
                self.emit(Instruction::Pass);
            }
        }
        Ok(())
    }

    fn compile_delete(&mut self, expression: &ast::Expression) -> Result<(), CompileError> {
        match expression {
            ast::Expression::Identifier { name } => {
                self.emit(Instruction::DeleteName {
                    name: name.to_string(),
                });
            }
            ast::Expression::Attribute { value, name } => {
                self.compile_expression(value)?;
                self.emit(Instruction::DeleteAttr {
                    name: name.to_string(),
                });
            }
            ast::Expression::Subscript { a, b } => {
                self.compile_expression(a)?;
                self.compile_expression(b)?;
                self.emit(Instruction::DeleteSubscript);
            }
            ast::Expression::Tuple { elements } => {
                for element in elements {
                    self.compile_delete(element)?;
                }
            }
            _ => {
                return Err(CompileError {
                    error: CompileErrorType::Delete(expression.name()),
                    location: self.current_source_location.clone(),
                });
            }
        }
        Ok(())
    }

    fn enter_function(
        &mut self,
        name: &str,
        args: &ast::Parameters,
    ) -> Result<bytecode::FunctionOpArg, CompileError> {
        let have_defaults = !args.defaults.is_empty();
        if have_defaults {
            // Construct a tuple:
            let size = args.defaults.len();
            for element in &args.defaults {
                self.compile_expression(element)?;
            }
            self.emit(Instruction::BuildTuple {
                size,
                unpack: false,
            });
        }

        let mut num_kw_only_defaults = 0;
        for (kw, default) in args.kwonlyargs.iter().zip(&args.kw_defaults) {
            if let Some(default) = default {
                self.emit(Instruction::LoadConst {
                    value: bytecode::Constant::String {
                        value: kw.arg.clone(),
                    },
                });
                self.compile_expression(default)?;
                num_kw_only_defaults += 1;
            }
        }
        if num_kw_only_defaults > 0 {
            self.emit(Instruction::BuildMap {
                size: num_kw_only_defaults,
                unpack: false,
            });
        }

        let line_number = self.get_source_line_number();
        self.code_object_stack.push(CodeObject::new(
            args.args.iter().map(|a| a.arg.clone()).collect(),
            Varargs::from(&args.vararg),
            args.kwonlyargs.iter().map(|a| a.arg.clone()).collect(),
            Varargs::from(&args.kwarg),
            self.source_path.clone().unwrap(),
            line_number,
            name.to_string(),
        ));
        self.enter_scope();

        let mut flags = bytecode::FunctionOpArg::empty();
        if have_defaults {
            flags |= bytecode::FunctionOpArg::HAS_DEFAULTS;
        }
        if num_kw_only_defaults > 0 {
            flags |= bytecode::FunctionOpArg::HAS_KW_ONLY_DEFAULTS;
        }

        Ok(flags)
    }

    fn prepare_decorators(
        &mut self,
        decorator_list: &[ast::Expression],
    ) -> Result<(), CompileError> {
        for decorator in decorator_list {
            self.compile_expression(decorator)?;
        }
        Ok(())
    }

    fn apply_decorators(&mut self, decorator_list: &[ast::Expression]) {
        // Apply decorators:
        for _ in decorator_list {
            self.emit(Instruction::CallFunction {
                typ: CallType::Positional(1),
            });
        }
    }

    fn compile_try_statement(
        &mut self,
        body: &[ast::LocatedStatement],
        handlers: &[ast::ExceptHandler],
        orelse: &Option<Vec<ast::LocatedStatement>>,
        finalbody: &Option<Vec<ast::LocatedStatement>>,
    ) -> Result<(), CompileError> {
        let mut handler_label = self.new_label();
        let finally_label = self.new_label();
        let else_label = self.new_label();
        // try:
        self.emit(Instruction::SetupExcept {
            handler: handler_label,
        });
        self.compile_statements(body)?;
        self.emit(Instruction::PopBlock);
        self.emit(Instruction::Jump { target: else_label });

        // except handlers:
        self.set_label(handler_label);
        // Exception is on top of stack now
        handler_label = self.new_label();
        for handler in handlers {
            // If we gave a typ,
            // check if this handler can handle the exception:
            if let Some(exc_type) = &handler.typ {
                // Duplicate exception for test:
                self.emit(Instruction::Duplicate);

                // Check exception type:
                self.emit(Instruction::LoadName {
                    name: String::from("isinstance"),
                    scope: bytecode::NameScope::Local,
                });
                self.emit(Instruction::Rotate { amount: 2 });
                self.compile_expression(exc_type)?;
                self.emit(Instruction::CallFunction {
                    typ: CallType::Positional(2),
                });

                // We cannot handle this exception type:
                self.emit(Instruction::JumpIfFalse {
                    target: handler_label,
                });

                // We have a match, store in name (except x as y)
                if let Some(alias) = &handler.name {
                    self.store_name(alias);
                } else {
                    // Drop exception from top of stack:
                    self.emit(Instruction::Pop);
                }
            } else {
                // Catch all!
                // Drop exception from top of stack:
                self.emit(Instruction::Pop);
            }

            // Handler code:
            self.compile_statements(&handler.body)?;
            self.emit(Instruction::PopException);
            self.emit(Instruction::Jump {
                target: finally_label,
            });

            // Emit a new label for the next handler
            self.set_label(handler_label);
            handler_label = self.new_label();
        }
        self.emit(Instruction::Jump {
            target: handler_label,
        });
        self.set_label(handler_label);
        // If code flows here, we have an unhandled exception,
        // emit finally code and raise again!
        // Duplicate finally code here:
        // TODO: this bytecode is now duplicate, could this be
        // improved?
        if let Some(statements) = finalbody {
            self.compile_statements(statements)?;
        }
        self.emit(Instruction::Raise { argc: 0 });

        // We successfully ran the try block:
        // else:
        self.set_label(else_label);
        if let Some(statements) = orelse {
            self.compile_statements(statements)?;
        }

        // finally:
        self.set_label(finally_label);
        if let Some(statements) = finalbody {
            self.compile_statements(statements)?;
        }
        // unimplemented!();
        Ok(())
    }

    fn compile_function_def(
        &mut self,
        name: &str,
        args: &ast::Parameters,
        body: &[ast::LocatedStatement],
        decorator_list: &[ast::Expression],
        returns: &Option<ast::Expression>, // TODO: use type hint somehow..
    ) -> Result<(), CompileError> {
        // Create bytecode for this function:
        // remember to restore self.in_loop to the original after the function is compiled
        let was_in_loop = self.in_loop;
        let was_in_function_def = self.in_function_def;
        self.in_loop = false;
        self.in_function_def = true;

        let old_qualified_path = self.current_qualified_path.clone();
        let qualified_name = self.create_qualified_name(name, "");
        self.current_qualified_path = Some(self.create_qualified_name(name, ".<locals>"));

        self.prepare_decorators(decorator_list)?;

        let mut flags = self.enter_function(name, args)?;

        let (new_body, doc_str) = get_doc(body);

        self.compile_statements(new_body)?;

        // Emit None at end:
        self.emit(Instruction::LoadConst {
            value: bytecode::Constant::None,
        });
        self.emit(Instruction::ReturnValue);
        let code = self.pop_code_object();
        self.leave_scope();

        // Prepare type annotations:
        let mut num_annotations = 0;

        // Return annotation:
        if let Some(annotation) = returns {
            // key:
            self.emit(Instruction::LoadConst {
                value: bytecode::Constant::String {
                    value: "return".to_string(),
                },
            });
            // value:
            self.compile_expression(annotation)?;
            num_annotations += 1;
        }

        for arg in args.args.iter() {
            if let Some(annotation) = &arg.annotation {
                self.emit(Instruction::LoadConst {
                    value: bytecode::Constant::String {
                        value: arg.arg.to_string(),
                    },
                });
                self.compile_expression(&annotation)?;
                num_annotations += 1;
            }
        }

        if num_annotations > 0 {
            flags |= bytecode::FunctionOpArg::HAS_ANNOTATIONS;
            self.emit(Instruction::BuildMap {
                size: num_annotations,
                unpack: false,
            });
        }

        self.emit(Instruction::LoadConst {
            value: bytecode::Constant::Code {
                code: Box::new(code),
            },
        });
        self.emit(Instruction::LoadConst {
            value: bytecode::Constant::String {
                value: qualified_name,
            },
        });

        // Turn code object into function object:
        self.emit(Instruction::MakeFunction { flags });
        self.store_docstring(doc_str);
        self.apply_decorators(decorator_list);

        self.store_name(name);

        self.current_qualified_path = old_qualified_path;
        self.in_loop = was_in_loop;
        self.in_function_def = was_in_function_def;
        Ok(())
    }

    fn compile_class_def(
        &mut self,
        name: &str,
        body: &[ast::LocatedStatement],
        bases: &[ast::Expression],
        keywords: &[ast::Keyword],
        decorator_list: &[ast::Expression],
    ) -> Result<(), CompileError> {
        let was_in_loop = self.in_loop;
        self.in_loop = false;

        let old_qualified_path = self.current_qualified_path.clone();
        let qualified_name = self.create_qualified_name(name, "");
        self.current_qualified_path = Some(qualified_name.clone());

        self.prepare_decorators(decorator_list)?;
        self.emit(Instruction::LoadBuildClass);
        let line_number = self.get_source_line_number();
        self.code_object_stack.push(CodeObject::new(
            vec![],
            Varargs::None,
            vec![],
            Varargs::None,
            self.source_path.clone().unwrap(),
            line_number,
            name.to_string(),
        ));
        self.enter_scope();

        let (new_body, doc_str) = get_doc(body);

        self.emit(Instruction::LoadName {
            name: "__name__".to_string(),
            scope: bytecode::NameScope::Local,
        });
        self.emit(Instruction::StoreName {
            name: "__module__".to_string(),
            scope: bytecode::NameScope::Local,
        });
        self.compile_statements(new_body)?;
        self.emit(Instruction::LoadConst {
            value: bytecode::Constant::None,
        });
        self.emit(Instruction::ReturnValue);

        let code = self.pop_code_object();
        self.leave_scope();

        self.emit(Instruction::LoadConst {
            value: bytecode::Constant::Code {
                code: Box::new(code),
            },
        });
        self.emit(Instruction::LoadConst {
            value: bytecode::Constant::String {
                value: name.to_string(),
            },
        });

        // Turn code object into function object:
        self.emit(Instruction::MakeFunction {
            flags: bytecode::FunctionOpArg::empty(),
        });

        self.emit(Instruction::LoadConst {
            value: bytecode::Constant::String {
                value: qualified_name,
            },
        });

        for base in bases {
            self.compile_expression(base)?;
        }

        if !keywords.is_empty() {
            let mut kwarg_names = vec![];
            for keyword in keywords {
                if let Some(name) = &keyword.name {
                    kwarg_names.push(bytecode::Constant::String {
                        value: name.to_string(),
                    });
                } else {
                    // This means **kwargs!
                    panic!("name must be set");
                }
                self.compile_expression(&keyword.value)?;
            }

            self.emit(Instruction::LoadConst {
                value: bytecode::Constant::Tuple {
                    elements: kwarg_names,
                },
            });
            self.emit(Instruction::CallFunction {
                typ: CallType::Keyword(2 + keywords.len() + bases.len()),
            });
        } else {
            self.emit(Instruction::CallFunction {
                typ: CallType::Positional(2 + bases.len()),
            });
        }

        self.store_docstring(doc_str);
        self.apply_decorators(decorator_list);

        self.store_name(name);
        self.current_qualified_path = old_qualified_path;
        self.in_loop = was_in_loop;
        Ok(())
    }

    fn store_docstring(&mut self, doc_str: Option<String>) {
        if let Some(doc_string) = doc_str {
            // Duplicate top of stack (the function or class object)
            self.emit(Instruction::Duplicate);

            // Doc string value:
            self.emit(Instruction::LoadConst {
                value: bytecode::Constant::String {
                    value: doc_string.to_string(),
                },
            });

            self.emit(Instruction::Rotate { amount: 2 });
            self.emit(Instruction::StoreAttr {
                name: "__doc__".to_string(),
            });
        }
    }

    fn compile_for(
        &mut self,
        target: &ast::Expression,
        iter: &ast::Expression,
        body: &[ast::LocatedStatement],
        orelse: &Option<Vec<ast::LocatedStatement>>,
    ) -> Result<(), CompileError> {
        // Start loop
        let start_label = self.new_label();
        let else_label = self.new_label();
        let end_label = self.new_label();
        self.emit(Instruction::SetupLoop {
            start: start_label,
            end: end_label,
        });

        // The thing iterated:
        self.compile_expression(iter)?;

        // Retrieve Iterator
        self.emit(Instruction::GetIter);

        self.set_label(start_label);
        self.emit(Instruction::ForIter { target: else_label });

        // Start of loop iteration, set targets:
        self.compile_store(target)?;

        let was_in_loop = self.in_loop;
        self.in_loop = true;
        self.compile_statements(body)?;
        self.in_loop = was_in_loop;

        self.emit(Instruction::Jump {
            target: start_label,
        });
        self.set_label(else_label);
        self.emit(Instruction::PopBlock);
        if let Some(orelse) = orelse {
            self.compile_statements(orelse)?;
        }
        self.set_label(end_label);
        Ok(())
    }

    fn compile_chained_comparison(
        &mut self,
        vals: &[ast::Expression],
        ops: &[ast::Comparison],
    ) -> Result<(), CompileError> {
        assert!(!ops.is_empty());
        assert_eq!(vals.len(), ops.len() + 1);

        let to_operator = |op: &ast::Comparison| match op {
            ast::Comparison::Equal => bytecode::ComparisonOperator::Equal,
            ast::Comparison::NotEqual => bytecode::ComparisonOperator::NotEqual,
            ast::Comparison::Less => bytecode::ComparisonOperator::Less,
            ast::Comparison::LessOrEqual => bytecode::ComparisonOperator::LessOrEqual,
            ast::Comparison::Greater => bytecode::ComparisonOperator::Greater,
            ast::Comparison::GreaterOrEqual => bytecode::ComparisonOperator::GreaterOrEqual,
            ast::Comparison::In => bytecode::ComparisonOperator::In,
            ast::Comparison::NotIn => bytecode::ComparisonOperator::NotIn,
            ast::Comparison::Is => bytecode::ComparisonOperator::Is,
            ast::Comparison::IsNot => bytecode::ComparisonOperator::IsNot,
        };

        // a == b == c == d
        // compile into (pseudocode):
        // result = a == b
        // if result:
        //   result = b == c
        //   if result:
        //     result = c == d

        // initialize lhs outside of loop
        self.compile_expression(&vals[0])?;

        let break_label = self.new_label();
        let last_label = self.new_label();

        // for all comparisons except the last (as the last one doesn't need a conditional jump)
        let ops_slice = &ops[0..ops.len()];
        let vals_slice = &vals[1..ops.len()];
        for (op, val) in ops_slice.iter().zip(vals_slice.iter()) {
            self.compile_expression(val)?;
            // store rhs for the next comparison in chain
            self.emit(Instruction::Duplicate);
            self.emit(Instruction::Rotate { amount: 3 });

            self.emit(Instruction::CompareOperation {
                op: to_operator(op),
            });

            // if comparison result is false, we break with this value; if true, try the next one.
            // (CPython compresses these three opcodes into JUMP_IF_FALSE_OR_POP)
            self.emit(Instruction::Duplicate);
            self.emit(Instruction::JumpIfFalse {
                target: break_label,
            });
            self.emit(Instruction::Pop);
        }

        // handle the last comparison
        self.compile_expression(vals.last().unwrap())?;
        self.emit(Instruction::CompareOperation {
            op: to_operator(ops.last().unwrap()),
        });
        self.emit(Instruction::Jump { target: last_label });

        // early exit left us with stack: `rhs, comparison_result`. We need to clean up rhs.
        self.set_label(break_label);
        self.emit(Instruction::Rotate { amount: 2 });
        self.emit(Instruction::Pop);

        self.set_label(last_label);
        Ok(())
    }

    fn compile_store(&mut self, target: &ast::Expression) -> Result<(), CompileError> {
        match target {
            ast::Expression::Identifier { name } => {
                self.store_name(name);
            }
            ast::Expression::Subscript { a, b } => {
                self.compile_expression(a)?;
                self.compile_expression(b)?;
                self.emit(Instruction::StoreSubscript);
            }
            ast::Expression::Attribute { value, name } => {
                self.compile_expression(value)?;
                self.emit(Instruction::StoreAttr {
                    name: name.to_string(),
                });
            }
            ast::Expression::List { elements } | ast::Expression::Tuple { elements } => {
                let mut seen_star = false;

                // Scan for star args:
                for (i, element) in elements.iter().enumerate() {
                    if let ast::Expression::Starred { .. } = element {
                        if seen_star {
                            return Err(CompileError {
                                error: CompileErrorType::StarArgs,
                                location: self.current_source_location.clone(),
                            });
                        } else {
                            seen_star = true;
                            self.emit(Instruction::UnpackEx {
                                before: i,
                                after: elements.len() - i - 1,
                            });
                        }
                    }
                }

                if !seen_star {
                    self.emit(Instruction::UnpackSequence {
                        size: elements.len(),
                    });
                }

                for element in elements {
                    if let ast::Expression::Starred { value } = element {
                        self.compile_store(value)?;
                    } else {
                        self.compile_store(element)?;
                    }
                }
            }
            _ => {
                return Err(CompileError {
                    error: CompileErrorType::Assign(target.name()),
                    location: self.current_source_location.clone(),
                });
            }
        }

        Ok(())
    }

    fn compile_op(&mut self, op: &ast::Operator, inplace: bool) {
        let i = match op {
            ast::Operator::Add => bytecode::BinaryOperator::Add,
            ast::Operator::Sub => bytecode::BinaryOperator::Subtract,
            ast::Operator::Mult => bytecode::BinaryOperator::Multiply,
            ast::Operator::MatMult => bytecode::BinaryOperator::MatrixMultiply,
            ast::Operator::Div => bytecode::BinaryOperator::Divide,
            ast::Operator::FloorDiv => bytecode::BinaryOperator::FloorDivide,
            ast::Operator::Mod => bytecode::BinaryOperator::Modulo,
            ast::Operator::Pow => bytecode::BinaryOperator::Power,
            ast::Operator::LShift => bytecode::BinaryOperator::Lshift,
            ast::Operator::RShift => bytecode::BinaryOperator::Rshift,
            ast::Operator::BitOr => bytecode::BinaryOperator::Or,
            ast::Operator::BitXor => bytecode::BinaryOperator::Xor,
            ast::Operator::BitAnd => bytecode::BinaryOperator::And,
        };
        self.emit(Instruction::BinaryOperation { op: i, inplace });
    }

    fn compile_test(
        &mut self,
        expression: &ast::Expression,
        true_label: Option<Label>,
        false_label: Option<Label>,
        context: EvalContext,
    ) -> Result<(), CompileError> {
        // Compile expression for test, and jump to label if false
        match expression {
            ast::Expression::BoolOp { a, op, b } => match op {
                ast::BooleanOperator::And => {
                    let f = false_label.unwrap_or_else(|| self.new_label());
                    self.compile_test(a, None, Some(f), context)?;
                    self.compile_test(b, true_label, false_label, context)?;
                    if false_label.is_none() {
                        self.set_label(f);
                    }
                }
                ast::BooleanOperator::Or => {
                    let t = true_label.unwrap_or_else(|| self.new_label());
                    self.compile_test(a, Some(t), None, context)?;
                    self.compile_test(b, true_label, false_label, context)?;
                    if true_label.is_none() {
                        self.set_label(t);
                    }
                }
            },
            _ => {
                self.compile_expression(expression)?;
                match context {
                    EvalContext::Statement => {
                        if let Some(true_label) = true_label {
                            self.emit(Instruction::JumpIf { target: true_label });
                        }
                        if let Some(false_label) = false_label {
                            self.emit(Instruction::JumpIfFalse {
                                target: false_label,
                            });
                        }
                    }
                    EvalContext::Expression => {
                        if let Some(true_label) = true_label {
                            self.emit(Instruction::Duplicate);
                            self.emit(Instruction::JumpIf { target: true_label });
                            self.emit(Instruction::Pop);
                        }
                        if let Some(false_label) = false_label {
                            self.emit(Instruction::Duplicate);
                            self.emit(Instruction::JumpIfFalse {
                                target: false_label,
                            });
                            self.emit(Instruction::Pop);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn compile_expression(&mut self, expression: &ast::Expression) -> Result<(), CompileError> {
        trace!("Compiling {:?}", expression);
        match expression {
            ast::Expression::Call {
                function,
                args,
                keywords,
            } => self.compile_call(function, args, keywords)?,
            ast::Expression::BoolOp { .. } => {
                self.compile_test(expression, None, None, EvalContext::Expression)?
            }
            ast::Expression::Binop { a, op, b } => {
                self.compile_expression(a)?;
                self.compile_expression(b)?;

                // Perform operation:
                self.compile_op(op, false);
            }
            ast::Expression::Subscript { a, b } => {
                self.compile_expression(a)?;
                self.compile_expression(b)?;
                self.emit(Instruction::BinaryOperation {
                    op: bytecode::BinaryOperator::Subscript,
                    inplace: false,
                });
            }
            ast::Expression::Unop { op, a } => {
                self.compile_expression(a)?;

                // Perform operation:
                let i = match op {
                    ast::UnaryOperator::Pos => bytecode::UnaryOperator::Plus,
                    ast::UnaryOperator::Neg => bytecode::UnaryOperator::Minus,
                    ast::UnaryOperator::Not => bytecode::UnaryOperator::Not,
                    ast::UnaryOperator::Inv => bytecode::UnaryOperator::Invert,
                };
                let i = Instruction::UnaryOperation { op: i };
                self.emit(i);
            }
            ast::Expression::Attribute { value, name } => {
                self.compile_expression(value)?;
                self.emit(Instruction::LoadAttr {
                    name: name.to_string(),
                });
            }
            ast::Expression::Compare { vals, ops } => {
                self.compile_chained_comparison(vals, ops)?;
            }
            ast::Expression::Number { value } => {
                let const_value = match value {
                    ast::Number::Integer { value } => bytecode::Constant::Integer {
                        value: value.clone(),
                    },
                    ast::Number::Float { value } => bytecode::Constant::Float { value: *value },
                    ast::Number::Complex { real, imag } => bytecode::Constant::Complex {
                        value: Complex64::new(*real, *imag),
                    },
                };
                self.emit(Instruction::LoadConst { value: const_value });
            }
            ast::Expression::List { elements } => {
                let size = elements.len();
                let must_unpack = self.gather_elements(elements)?;
                self.emit(Instruction::BuildList {
                    size,
                    unpack: must_unpack,
                });
            }
            ast::Expression::Tuple { elements } => {
                let size = elements.len();
                let must_unpack = self.gather_elements(elements)?;
                self.emit(Instruction::BuildTuple {
                    size,
                    unpack: must_unpack,
                });
            }
            ast::Expression::Set { elements } => {
                let size = elements.len();
                let must_unpack = self.gather_elements(elements)?;
                self.emit(Instruction::BuildSet {
                    size,
                    unpack: must_unpack,
                });
            }
            ast::Expression::Dict { elements } => {
                let size = elements.len();
                let has_double_star = elements.iter().any(|e| e.0.is_none());
                for (key, value) in elements {
                    if let Some(key) = key {
                        self.compile_expression(key)?;
                        self.compile_expression(value)?;
                        if has_double_star {
                            self.emit(Instruction::BuildMap {
                                size: 1,
                                unpack: false,
                            });
                        }
                    } else {
                        // dict unpacking
                        self.compile_expression(value)?;
                    }
                }
                self.emit(Instruction::BuildMap {
                    size,
                    unpack: has_double_star,
                });
            }
            ast::Expression::Slice { elements } => {
                let size = elements.len();
                for element in elements {
                    self.compile_expression(element)?;
                }
                self.emit(Instruction::BuildSlice { size });
            }
            ast::Expression::Yield { value } => {
                if !self.in_function_def {
                    return Err(CompileError {
                        error: CompileErrorType::InvalidYield,
                        location: self.current_source_location.clone(),
                    });
                }
                self.mark_generator();
                match value {
                    Some(expression) => self.compile_expression(expression)?,
                    None => self.emit(Instruction::LoadConst {
                        value: bytecode::Constant::None,
                    }),
                };
                self.emit(Instruction::YieldValue);
            }
            ast::Expression::Await { .. } => {
                unimplemented!("await");
            }
            ast::Expression::YieldFrom { value } => {
                self.mark_generator();
                self.compile_expression(value)?;
                self.emit(Instruction::GetIter);
                self.emit(Instruction::LoadConst {
                    value: bytecode::Constant::None,
                });
                self.emit(Instruction::YieldFrom);
            }
            ast::Expression::True => {
                self.emit(Instruction::LoadConst {
                    value: bytecode::Constant::Boolean { value: true },
                });
            }
            ast::Expression::False => {
                self.emit(Instruction::LoadConst {
                    value: bytecode::Constant::Boolean { value: false },
                });
            }
            ast::Expression::None => {
                self.emit(Instruction::LoadConst {
                    value: bytecode::Constant::None,
                });
            }
            ast::Expression::Ellipsis => {
                self.emit(Instruction::LoadConst {
                    value: bytecode::Constant::Ellipsis,
                });
            }
            ast::Expression::String { value } => {
                self.compile_string(value)?;
            }
            ast::Expression::Bytes { value } => {
                self.emit(Instruction::LoadConst {
                    value: bytecode::Constant::Bytes {
                        value: value.clone(),
                    },
                });
            }
            ast::Expression::Identifier { name } => {
                self.load_name(name);
            }
            ast::Expression::Lambda { args, body } => {
                let name = "<lambda>".to_string();
                // no need to worry about the self.loop_depth because there are no loops in lambda expressions
                let flags = self.enter_function(&name, args)?;
                self.compile_expression(body)?;
                self.emit(Instruction::ReturnValue);
                let code = self.pop_code_object();
                self.leave_scope();
                self.emit(Instruction::LoadConst {
                    value: bytecode::Constant::Code {
                        code: Box::new(code),
                    },
                });
                self.emit(Instruction::LoadConst {
                    value: bytecode::Constant::String { value: name },
                });
                // Turn code object into function object:
                self.emit(Instruction::MakeFunction { flags });
            }
            ast::Expression::Comprehension { kind, generators } => {
                self.compile_comprehension(kind, generators)?;
            }
            ast::Expression::Starred { value } => {
                self.compile_expression(value)?;
                self.emit(Instruction::Unpack);
                panic!("We should not just unpack a starred args, since the size is unknown.");
            }
            ast::Expression::IfExpression { test, body, orelse } => {
                let no_label = self.new_label();
                let end_label = self.new_label();
                self.compile_test(test, None, Some(no_label), EvalContext::Expression)?;
                self.compile_expression(body)?;
                self.emit(Instruction::Jump { target: end_label });
                self.set_label(no_label);
                self.compile_expression(orelse)?;
                self.set_label(end_label);
            }
        }
        Ok(())
    }

    fn compile_call(
        &mut self,
        function: &ast::Expression,
        args: &[ast::Expression],
        keywords: &[ast::Keyword],
    ) -> Result<(), CompileError> {
        self.compile_expression(function)?;
        let count = args.len() + keywords.len();

        // Normal arguments:
        let must_unpack = self.gather_elements(args)?;
        let has_double_star = keywords.iter().any(|k| k.name.is_none());

        if must_unpack || has_double_star {
            // Create a tuple with positional args:
            self.emit(Instruction::BuildTuple {
                size: args.len(),
                unpack: must_unpack,
            });

            // Create an optional map with kw-args:
            if !keywords.is_empty() {
                for keyword in keywords {
                    if let Some(name) = &keyword.name {
                        self.emit(Instruction::LoadConst {
                            value: bytecode::Constant::String {
                                value: name.to_string(),
                            },
                        });
                        self.compile_expression(&keyword.value)?;
                        if has_double_star {
                            self.emit(Instruction::BuildMap {
                                size: 1,
                                unpack: false,
                            });
                        }
                    } else {
                        // This means **kwargs!
                        self.compile_expression(&keyword.value)?;
                    }
                }

                self.emit(Instruction::BuildMap {
                    size: keywords.len(),
                    unpack: has_double_star,
                });

                self.emit(Instruction::CallFunction {
                    typ: CallType::Ex(true),
                });
            } else {
                self.emit(Instruction::CallFunction {
                    typ: CallType::Ex(false),
                });
            }
        } else {
            // Keyword arguments:
            if !keywords.is_empty() {
                let mut kwarg_names = vec![];
                for keyword in keywords {
                    if let Some(name) = &keyword.name {
                        kwarg_names.push(bytecode::Constant::String {
                            value: name.to_string(),
                        });
                    } else {
                        // This means **kwargs!
                        panic!("name must be set");
                    }
                    self.compile_expression(&keyword.value)?;
                }

                self.emit(Instruction::LoadConst {
                    value: bytecode::Constant::Tuple {
                        elements: kwarg_names,
                    },
                });
                self.emit(Instruction::CallFunction {
                    typ: CallType::Keyword(count),
                });
            } else {
                self.emit(Instruction::CallFunction {
                    typ: CallType::Positional(count),
                });
            }
        }
        Ok(())
    }

    // Given a vector of expr / star expr generate code which gives either
    // a list of expressions on the stack, or a list of tuples.
    fn gather_elements(&mut self, elements: &[ast::Expression]) -> Result<bool, CompileError> {
        // First determine if we have starred elements:
        let has_stars = elements.iter().any(|e| {
            if let ast::Expression::Starred { .. } = e {
                true
            } else {
                false
            }
        });

        for element in elements {
            if let ast::Expression::Starred { value } = element {
                self.compile_expression(value)?;
            } else {
                self.compile_expression(element)?;
                if has_stars {
                    self.emit(Instruction::BuildTuple {
                        size: 1,
                        unpack: false,
                    });
                }
            }
        }

        Ok(has_stars)
    }

    fn compile_comprehension(
        &mut self,
        kind: &ast::ComprehensionKind,
        generators: &[ast::Comprehension],
    ) -> Result<(), CompileError> {
        // We must have at least one generator:
        assert!(!generators.is_empty());

        let name = match kind {
            ast::ComprehensionKind::GeneratorExpression { .. } => "<genexpr>",
            ast::ComprehensionKind::List { .. } => "<listcomp>",
            ast::ComprehensionKind::Set { .. } => "<setcomp>",
            ast::ComprehensionKind::Dict { .. } => "<dictcomp>",
        }
        .to_string();

        let line_number = self.get_source_line_number();
        // Create magnificent function <listcomp>:
        self.code_object_stack.push(CodeObject::new(
            vec![".0".to_string()],
            Varargs::None,
            vec![],
            Varargs::None,
            self.source_path.clone().unwrap(),
            line_number,
            name.clone(),
        ));

        // Create empty object of proper type:
        match kind {
            ast::ComprehensionKind::GeneratorExpression { .. } => {}
            ast::ComprehensionKind::List { .. } => {
                self.emit(Instruction::BuildList {
                    size: 0,
                    unpack: false,
                });
            }
            ast::ComprehensionKind::Set { .. } => {
                self.emit(Instruction::BuildSet {
                    size: 0,
                    unpack: false,
                });
            }
            ast::ComprehensionKind::Dict { .. } => {
                self.emit(Instruction::BuildMap {
                    size: 0,
                    unpack: false,
                });
            }
        }

        let mut loop_labels = vec![];
        for generator in generators {
            if loop_labels.is_empty() {
                // Load iterator onto stack (passed as first argument):
                self.emit(Instruction::LoadName {
                    name: String::from(".0"),
                    scope: bytecode::NameScope::Local,
                });
            } else {
                // Evaluate iterated item:
                self.compile_expression(&generator.iter)?;

                // Get iterator / turn item into an iterator
                self.emit(Instruction::GetIter);
            }

            // Setup for loop:
            let start_label = self.new_label();
            let end_label = self.new_label();
            loop_labels.push((start_label, end_label));
            self.emit(Instruction::SetupLoop {
                start: start_label,
                end: end_label,
            });
            self.set_label(start_label);
            self.emit(Instruction::ForIter { target: end_label });

            self.compile_store(&generator.target)?;

            // Now evaluate the ifs:
            for if_condition in &generator.ifs {
                self.compile_test(
                    if_condition,
                    None,
                    Some(start_label),
                    EvalContext::Statement,
                )?
            }
        }

        match kind {
            ast::ComprehensionKind::GeneratorExpression { element } => {
                self.compile_expression(element)?;
                self.mark_generator();
                self.emit(Instruction::YieldValue);
                self.emit(Instruction::Pop);
            }
            ast::ComprehensionKind::List { element } => {
                self.compile_expression(element)?;
                self.emit(Instruction::ListAppend {
                    i: 1 + generators.len(),
                });
            }
            ast::ComprehensionKind::Set { element } => {
                self.compile_expression(element)?;
                self.emit(Instruction::SetAdd {
                    i: 1 + generators.len(),
                });
            }
            ast::ComprehensionKind::Dict { key, value } => {
                self.compile_expression(value)?;
                self.compile_expression(key)?;

                self.emit(Instruction::MapAdd {
                    i: 1 + generators.len(),
                });
            }
        }

        for (start_label, end_label) in loop_labels.iter().rev() {
            // Repeat:
            self.emit(Instruction::Jump {
                target: *start_label,
            });

            // End of for loop:
            self.set_label(*end_label);
            self.emit(Instruction::PopBlock);
        }

        // Return freshly filled list:
        self.emit(Instruction::ReturnValue);

        // Fetch code for listcomp function:
        let code = self.pop_code_object();

        // List comprehension code:
        self.emit(Instruction::LoadConst {
            value: bytecode::Constant::Code {
                code: Box::new(code),
            },
        });

        // List comprehension function name:
        self.emit(Instruction::LoadConst {
            value: bytecode::Constant::String { value: name },
        });

        // Turn code object into function object:
        self.emit(Instruction::MakeFunction {
            flags: bytecode::FunctionOpArg::empty(),
        });

        // Evaluate iterated item:
        self.compile_expression(&generators[0].iter)?;

        // Get iterator / turn item into an iterator
        self.emit(Instruction::GetIter);

        // Call just created <listcomp> function:
        self.emit(Instruction::CallFunction {
            typ: CallType::Positional(1),
        });
        Ok(())
    }

    fn compile_string(&mut self, string: &ast::StringGroup) -> Result<(), CompileError> {
        match string {
            ast::StringGroup::Joined { values } => {
                for value in values {
                    self.compile_string(value)?;
                }
                self.emit(Instruction::BuildString { size: values.len() })
            }
            ast::StringGroup::Constant { value } => {
                self.emit(Instruction::LoadConst {
                    value: bytecode::Constant::String {
                        value: value.to_string(),
                    },
                });
            }
            ast::StringGroup::FormattedValue {
                value,
                conversion,
                spec,
            } => {
                self.compile_expression(value)?;
                self.emit(Instruction::FormatValue {
                    conversion: *conversion,
                    spec: spec.clone(),
                });
            }
        }
        Ok(())
    }

    // Scope helpers:
    fn enter_scope(&mut self) {
        // println!("Enter scope {:?}", self.scope_stack);
        // Enter first subscope!
        let scope = self.scope_stack.last_mut().unwrap().sub_scopes.remove(0);
        self.scope_stack.push(scope);
    }

    fn leave_scope(&mut self) {
        // println!("Leave scope {:?}", self.scope_stack);
        let scope = self.scope_stack.pop().unwrap();
        assert!(scope.sub_scopes.is_empty());
    }

    fn lookup_name(&self, name: &str) -> &SymbolRole {
        // println!("Looking up {:?}", name);
        let scope = self.scope_stack.last().unwrap();
        scope.lookup(name).unwrap()
    }

    // Low level helper functions:
    fn emit(&mut self, instruction: Instruction) {
        let location = self.current_source_location.clone();
        let cur_code_obj = self.current_code_object();
        cur_code_obj.instructions.push(instruction);
        cur_code_obj.locations.push(location);
        // TODO: insert source filename
    }

    fn current_code_object(&mut self) -> &mut CodeObject {
        self.code_object_stack.last_mut().unwrap()
    }

    // Generate a new label
    fn new_label(&mut self) -> Label {
        let l = self.nxt_label;
        self.nxt_label += 1;
        l
    }

    // Assign current position the given label
    fn set_label(&mut self, label: Label) {
        let position = self.current_code_object().instructions.len();
        // assert!(label not in self.label_map)
        self.current_code_object().label_map.insert(label, position);
    }

    fn set_source_location(&mut self, location: &ast::Location) {
        self.current_source_location = location.clone();
    }

    fn get_source_line_number(&mut self) -> usize {
        self.current_source_location.get_row()
    }

    fn create_qualified_name(&self, name: &str, suffix: &str) -> String {
        if let Some(ref qualified_path) = self.current_qualified_path {
            format!("{}.{}{}", qualified_path, name, suffix)
        } else {
            format!("{}{}", name, suffix)
        }
    }

    fn mark_generator(&mut self) {
        self.current_code_object().is_generator = true;
    }
}

fn get_doc(body: &[ast::LocatedStatement]) -> (&[ast::LocatedStatement], Option<String>) {
    if let Some(val) = body.get(0) {
        if let ast::Statement::Expression { ref expression } = val.node {
            if let ast::Expression::String { ref value } = expression {
                if let ast::StringGroup::Constant { ref value } = value {
                    if let Some((_, body_rest)) = body.split_first() {
                        return (body_rest, Some(value.to_string()));
                    }
                }
            }
        }
    }
    (body, None)
}

#[cfg(test)]
mod tests {
    use super::Compiler;
    use crate::bytecode::CodeObject;
    use crate::bytecode::Constant::*;
    use crate::bytecode::Instruction::*;
    use crate::symboltable::make_symbol_table;
    use rustpython_parser::parser;

    fn compile_exec(source: &str) -> CodeObject {
        let mut compiler = Compiler::new();
        compiler.source_path = Some("source_path".to_string());
        compiler.push_new_code_object("<module>".to_string());
        let ast = parser::parse_program(&source.to_string()).unwrap();
        let symbol_scope = make_symbol_table(&ast).unwrap();
        compiler.compile_program(&ast, symbol_scope).unwrap();
        compiler.pop_code_object()
    }

    #[test]
    fn test_if_ors() {
        let code = compile_exec("if True or False or False:\n pass\n");
        assert_eq!(
            vec![
                LoadConst {
                    value: Boolean { value: true }
                },
                JumpIf { target: 1 },
                LoadConst {
                    value: Boolean { value: false }
                },
                JumpIf { target: 1 },
                LoadConst {
                    value: Boolean { value: false }
                },
                JumpIfFalse { target: 0 },
                Pass,
                LoadConst { value: None },
                ReturnValue
            ],
            code.instructions
        );
    }

    #[test]
    fn test_if_ands() {
        let code = compile_exec("if True and False and False:\n pass\n");
        assert_eq!(
            vec![
                LoadConst {
                    value: Boolean { value: true }
                },
                JumpIfFalse { target: 0 },
                LoadConst {
                    value: Boolean { value: false }
                },
                JumpIfFalse { target: 0 },
                LoadConst {
                    value: Boolean { value: false }
                },
                JumpIfFalse { target: 0 },
                Pass,
                LoadConst { value: None },
                ReturnValue
            ],
            code.instructions
        );
    }

    #[test]
    fn test_if_mixed() {
        let code = compile_exec("if (True and False) or (False and True):\n pass\n");
        assert_eq!(
            vec![
                LoadConst {
                    value: Boolean { value: true }
                },
                JumpIfFalse { target: 2 },
                LoadConst {
                    value: Boolean { value: false }
                },
                JumpIf { target: 1 },
                LoadConst {
                    value: Boolean { value: false }
                },
                JumpIfFalse { target: 0 },
                LoadConst {
                    value: Boolean { value: true }
                },
                JumpIfFalse { target: 0 },
                Pass,
                LoadConst { value: None },
                ReturnValue
            ],
            code.instructions
        );
    }
}
