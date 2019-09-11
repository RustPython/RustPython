//!
//! Take an AST and transform it into bytecode
//!
//! Inspirational code:
//!   https://github.com/python/cpython/blob/master/Python/compile.c
//!   https://github.com/micropython/micropython/blob/master/py/compile.c

use crate::error::{CompileError, CompileErrorType};
use crate::output_stream::{CodeObjectStream, OutputStream};
use crate::peephole::PeepholeOptimizer;
use crate::symboltable::{
    make_symbol_table, statements_to_symbol_table, Symbol, SymbolScope, SymbolTable,
};
use num_complex::Complex64;
use rustpython_bytecode::bytecode::{self, CallType, CodeObject, Instruction, Label, Varargs};
use rustpython_parser::{ast, parser};

type BasicOutputStream = PeepholeOptimizer<CodeObjectStream>;

/// Main structure holding the state of compilation.
struct Compiler<O: OutputStream = BasicOutputStream> {
    output_stack: Vec<O>,
    symbol_table_stack: Vec<SymbolTable>,
    nxt_label: usize,
    source_path: Option<String>,
    current_source_location: ast::Location,
    current_qualified_path: Option<String>,
    in_loop: bool,
    in_function_def: bool,
    optimize: u8,
}

/// Compile a given sourcecode into a bytecode object.
pub fn compile(
    source: &str,
    mode: Mode,
    source_path: String,
    optimize: u8,
) -> Result<CodeObject, CompileError> {
    match mode {
        Mode::Exec => {
            let ast = parser::parse_program(source)?;
            compile_program(ast, source_path, optimize)
        }
        Mode::Eval => {
            let statement = parser::parse_statement(source)?;
            compile_statement_eval(statement, source_path, optimize)
        }
        Mode::Single => {
            let ast = parser::parse_program(source)?;
            compile_program_single(ast, source_path, optimize)
        }
    }
}

/// A helper function for the shared code of the different compile functions
fn with_compiler(
    source_path: String,
    optimize: u8,
    f: impl FnOnce(&mut Compiler) -> Result<(), CompileError>,
) -> Result<CodeObject, CompileError> {
    let mut compiler = Compiler::new(optimize);
    compiler.source_path = Some(source_path);
    compiler.push_new_code_object("<module>".to_string());
    f(&mut compiler)?;
    let code = compiler.pop_code_object();
    trace!("Compilation completed: {:?}", code);
    Ok(code)
}

/// Compile a standard Python program to bytecode
pub fn compile_program(
    ast: ast::Program,
    source_path: String,
    optimize: u8,
) -> Result<CodeObject, CompileError> {
    with_compiler(source_path, optimize, |compiler| {
        let symbol_table = make_symbol_table(&ast)?;
        compiler.compile_program(&ast, symbol_table)
    })
}

/// Compile a single Python expression to bytecode
pub fn compile_statement_eval(
    statement: Vec<ast::Statement>,
    source_path: String,
    optimize: u8,
) -> Result<CodeObject, CompileError> {
    with_compiler(source_path, optimize, |compiler| {
        let symbol_table = statements_to_symbol_table(&statement)?;
        compiler.compile_statement_eval(&statement, symbol_table)
    })
}

/// Compile a Python program to bytecode for the context of a REPL
pub fn compile_program_single(
    ast: ast::Program,
    source_path: String,
    optimize: u8,
) -> Result<CodeObject, CompileError> {
    with_compiler(source_path, optimize, |compiler| {
        let symbol_table = make_symbol_table(&ast)?;
        compiler.compile_program_single(&ast, symbol_table)
    })
}

#[derive(Clone, Copy)]
pub enum Mode {
    Exec,
    Eval,
    Single,
}

impl std::str::FromStr for Mode {
    type Err = ModeParseError;
    fn from_str(s: &str) -> Result<Self, ModeParseError> {
        match s {
            "exec" => Ok(Mode::Exec),
            "eval" => Ok(Mode::Eval),
            "single" => Ok(Mode::Single),
            _ => Err(ModeParseError { _priv: () }),
        }
    }
}

#[derive(Debug)]
pub struct ModeParseError {
    _priv: (),
}

impl std::fmt::Display for ModeParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, r#"mode should be "exec", "eval", or "single""#)
    }
}

impl<O> Default for Compiler<O>
where
    O: OutputStream,
{
    fn default() -> Self {
        Compiler::new(0)
    }
}

impl<O: OutputStream> Compiler<O> {
    fn new(optimize: u8) -> Self {
        Compiler {
            output_stack: Vec::new(),
            symbol_table_stack: Vec::new(),
            nxt_label: 0,
            source_path: None,
            current_source_location: ast::Location::default(),
            current_qualified_path: None,
            in_loop: false,
            in_function_def: false,
            optimize,
        }
    }

    fn push_output(&mut self, code: CodeObject) {
        self.output_stack.push(code.into());
    }

    fn push_new_code_object(&mut self, obj_name: String) {
        let line_number = self.get_source_line_number();
        self.push_output(CodeObject::new(
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
        self.output_stack.pop().unwrap().into()
    }

    fn compile_program(
        &mut self,
        program: &ast::Program,
        symbol_table: SymbolTable,
    ) -> Result<(), CompileError> {
        let size_before = self.output_stack.len();
        self.symbol_table_stack.push(symbol_table);

        let (statements, doc) = get_doc(&program.statements);
        if let Some(value) = doc {
            self.emit(Instruction::LoadConst {
                value: bytecode::Constant::String { value },
            });
            self.emit(Instruction::StoreName {
                name: "__doc__".to_owned(),
                scope: bytecode::NameScope::Global,
            });
        }
        self.compile_statements(statements)?;

        assert_eq!(self.output_stack.len(), size_before);

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
        symbol_table: SymbolTable,
    ) -> Result<(), CompileError> {
        self.symbol_table_stack.push(symbol_table);

        let mut emitted_return = false;

        for (i, statement) in program.statements.iter().enumerate() {
            let is_last = i == program.statements.len() - 1;

            if let ast::StatementType::Expression { ref expression } = statement.node {
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
        statements: &[ast::Statement],
        symbol_table: SymbolTable,
    ) -> Result<(), CompileError> {
        self.symbol_table_stack.push(symbol_table);
        for statement in statements {
            if let ast::StatementType::Expression { ref expression } = statement.node {
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

    fn compile_statements(&mut self, statements: &[ast::Statement]) -> Result<(), CompileError> {
        for statement in statements {
            self.compile_statement(statement)?
        }
        Ok(())
    }

    fn scope_for_name(&self, name: &str) -> bytecode::NameScope {
        let symbol = self.lookup_name(name);
        match symbol.scope {
            SymbolScope::Global => bytecode::NameScope::Global,
            SymbolScope::Nonlocal => bytecode::NameScope::NonLocal,
            SymbolScope::Unknown => bytecode::NameScope::Free,
            SymbolScope::Local => bytecode::NameScope::Free,
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

    fn compile_statement(&mut self, statement: &ast::Statement) -> Result<(), CompileError> {
        trace!("Compiling {:?}", statement);
        self.set_source_location(&statement.location);
        use ast::StatementType::*;

        match &statement.node {
            Import { names } => {
                // import a, b, c as d
                for name in names {
                    self.emit(Instruction::Import {
                        name: Some(name.symbol.clone()),
                        symbols: vec![],
                        level: 0,
                    });
                    if let Some(alias) = &name.alias {
                        for part in name.symbol.split('.').skip(1) {
                            self.emit(Instruction::LoadAttr {
                                name: part.to_owned(),
                            });
                        }
                        self.store_name(alias);
                    } else {
                        self.store_name(name.symbol.split('.').next().unwrap());
                    }
                }
            }
            ImportFrom {
                level,
                module,
                names,
            } => {
                let import_star = names.iter().any(|n| n.symbol == "*");

                if import_star {
                    // from .... import *
                    self.emit(Instruction::Import {
                        name: module.clone(),
                        symbols: vec!["*".to_owned()],
                        level: *level,
                    });
                    self.emit(Instruction::ImportStar);
                } else {
                    // from mod import a, b as c
                    // First, determine the fromlist (for import lib):
                    let from_list = names.iter().map(|n| n.symbol.clone()).collect();

                    // Load module once:
                    self.emit(Instruction::Import {
                        name: module.clone(),
                        symbols: from_list,
                        level: *level,
                    });

                    for name in names {
                        // import symbol from module:
                        self.emit(Instruction::ImportFrom {
                            name: name.symbol.to_string(),
                        });

                        // Store module under proper name:
                        if let Some(alias) = &name.alias {
                            self.store_name(alias);
                        } else {
                            self.store_name(&name.symbol);
                        }
                    }

                    // Pop module from stack:
                    self.emit(Instruction::Pop);
                }
            }
            Expression { expression } => {
                self.compile_expression(expression)?;

                // Pop result of stack, since we not use it:
                self.emit(Instruction::Pop);
            }
            Global { .. } | Nonlocal { .. } => {
                // Handled during symbol table construction.
            }
            If { test, body, orelse } => {
                let end_label = self.new_label();
                match orelse {
                    None => {
                        // Only if:
                        self.compile_jump_if(test, false, end_label)?;
                        self.compile_statements(body)?;
                        self.set_label(end_label);
                    }
                    Some(statements) => {
                        // if - else:
                        let else_label = self.new_label();
                        self.compile_jump_if(test, false, else_label)?;
                        self.compile_statements(body)?;
                        self.emit(Instruction::Jump { target: end_label });

                        // else:
                        self.set_label(else_label);
                        self.compile_statements(statements)?;
                    }
                }
                self.set_label(end_label);
            }
            While { test, body, orelse } => self.compile_while(test, body, orelse)?,
            With {
                is_async,
                items,
                body,
            } => {
                if *is_async {
                    unimplemented!("async with");
                } else {
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
            }
            For {
                is_async,
                target,
                iter,
                body,
                orelse,
            } => {
                if *is_async {
                    unimplemented!("async for");
                } else {
                    self.compile_for(target, iter, body, orelse)?
                }
            }
            Raise { exception, cause } => match exception {
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
            Try {
                body,
                handlers,
                orelse,
                finalbody,
            } => self.compile_try_statement(body, handlers, orelse, finalbody)?,
            FunctionDef {
                is_async,
                name,
                args,
                body,
                decorator_list,
                returns,
            } => {
                if *is_async {
                    unimplemented!("async def");
                } else {
                    self.compile_function_def(name, args, body, decorator_list, returns)?
                }
            }
            ClassDef {
                name,
                body,
                bases,
                keywords,
                decorator_list,
            } => self.compile_class_def(name, body, bases, keywords, decorator_list)?,
            Assert { test, msg } => {
                // if some flag, ignore all assert statements!
                if self.optimize == 0 {
                    let end_label = self.new_label();
                    self.compile_jump_if(test, true, end_label)?;
                    self.emit(Instruction::LoadName {
                        name: String::from("AssertionError"),
                        scope: bytecode::NameScope::Global,
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
            }
            Break => {
                if !self.in_loop {
                    return Err(CompileError {
                        error: CompileErrorType::InvalidBreak,
                        location: statement.location.clone(),
                    });
                }
                self.emit(Instruction::Break);
            }
            Continue => {
                if !self.in_loop {
                    return Err(CompileError {
                        error: CompileErrorType::InvalidContinue,
                        location: statement.location.clone(),
                    });
                }
                self.emit(Instruction::Continue);
            }
            Return { value } => {
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
            Assign { targets, value } => {
                self.compile_expression(value)?;

                for (i, target) in targets.iter().enumerate() {
                    if i + 1 != targets.len() {
                        self.emit(Instruction::Duplicate);
                    }
                    self.compile_store(target)?;
                }
            }
            AugAssign { target, op, value } => {
                self.compile_expression(target)?;
                self.compile_expression(value)?;

                // Perform operation:
                self.compile_op(op, true);
                self.compile_store(target)?;
            }
            AnnAssign {
                target,
                annotation,
                value,
            } => self.compile_annotated_assign(target, annotation, value)?,
            Delete { targets } => {
                for target in targets {
                    self.compile_delete(target)?;
                }
            }
            Pass => {
                // No need to emit any code here :)
            }
        }
        Ok(())
    }

    fn compile_delete(&mut self, expression: &ast::Expression) -> Result<(), CompileError> {
        match &expression.node {
            ast::ExpressionType::Identifier { name } => {
                self.emit(Instruction::DeleteName {
                    name: name.to_string(),
                });
            }
            ast::ExpressionType::Attribute { value, name } => {
                self.compile_expression(value)?;
                self.emit(Instruction::DeleteAttr {
                    name: name.to_string(),
                });
            }
            ast::ExpressionType::Subscript { a, b } => {
                self.compile_expression(a)?;
                self.compile_expression(b)?;
                self.emit(Instruction::DeleteSubscript);
            }
            ast::ExpressionType::Tuple { elements } => {
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
        self.push_output(CodeObject::new(
            args.args.iter().map(|a| a.arg.clone()).collect(),
            compile_varargs(&args.vararg),
            args.kwonlyargs.iter().map(|a| a.arg.clone()).collect(),
            compile_varargs(&args.kwarg),
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
        body: &[ast::Statement],
        handlers: &[ast::ExceptHandler],
        orelse: &Option<ast::Suite>,
        finalbody: &Option<ast::Suite>,
    ) -> Result<(), CompileError> {
        let mut handler_label = self.new_label();
        let finally_handler_label = self.new_label();
        let else_label = self.new_label();

        // Setup a finally block if we have a finally statement.
        if finalbody.is_some() {
            self.emit(Instruction::SetupFinally {
                handler: finally_handler_label,
            });
        }

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
                    scope: bytecode::NameScope::Global,
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

            if finalbody.is_some() {
                self.emit(Instruction::PopBlock); // pop finally block
                                                  // We enter the finally block, without exception.
                self.emit(Instruction::EnterFinally);
            }

            self.emit(Instruction::Jump {
                target: finally_handler_label,
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
        // raise the exception again!
        self.emit(Instruction::Raise { argc: 0 });

        // We successfully ran the try block:
        // else:
        self.set_label(else_label);
        if let Some(statements) = orelse {
            self.compile_statements(statements)?;
        }

        if finalbody.is_some() {
            self.emit(Instruction::PopBlock); // pop finally block

            // We enter the finally block, without return / exception.
            self.emit(Instruction::EnterFinally);
        }

        // finally:
        self.set_label(finally_handler_label);
        if let Some(statements) = finalbody {
            self.compile_statements(statements)?;
            self.emit(Instruction::EndFinally);
        }

        Ok(())
    }

    fn compile_function_def(
        &mut self,
        name: &str,
        args: &ast::Parameters,
        body: &[ast::Statement],
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
        body: &[ast::Statement],
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
        self.push_output(CodeObject::new(
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
            scope: bytecode::NameScope::Free,
        });
        self.emit(Instruction::StoreName {
            name: "__module__".to_string(),
            scope: bytecode::NameScope::Free,
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

    fn compile_while(
        &mut self,
        test: &ast::Expression,
        body: &[ast::Statement],
        orelse: &Option<Vec<ast::Statement>>,
    ) -> Result<(), CompileError> {
        let start_label = self.new_label();
        let else_label = self.new_label();
        let end_label = self.new_label();
        self.emit(Instruction::SetupLoop {
            start: start_label,
            end: end_label,
        });

        self.set_label(start_label);

        self.compile_jump_if(test, false, else_label)?;

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

    fn compile_for(
        &mut self,
        target: &ast::Expression,
        iter: &ast::Expression,
        body: &[ast::Statement],
        orelse: &Option<Vec<ast::Statement>>,
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
            self.emit(Instruction::JumpIfFalseOrPop {
                target: break_label,
            });
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

    fn compile_annotated_assign(
        &mut self,
        target: &ast::Expression,
        annotation: &ast::Expression,
        value: &Option<ast::Expression>,
    ) -> Result<(), CompileError> {
        if let Some(value) = value {
            self.compile_expression(value)?;
            self.compile_store(target)?;
        }

        // Compile annotation:
        self.compile_expression(annotation)?;

        if let ast::ExpressionType::Identifier { name } = &target.node {
            // Store as dict entry in __annotations__ dict:
            self.emit(Instruction::LoadName {
                name: String::from("__annotations__"),
                scope: bytecode::NameScope::Local,
            });
            self.emit(Instruction::LoadConst {
                value: bytecode::Constant::String {
                    value: name.to_string(),
                },
            });
            self.emit(Instruction::StoreSubscript);
        } else {
            // Drop annotation if not assigned to simple identifier.
            self.emit(Instruction::Pop);
        }
        Ok(())
    }

    fn compile_store(&mut self, target: &ast::Expression) -> Result<(), CompileError> {
        match &target.node {
            ast::ExpressionType::Identifier { name } => {
                self.store_name(name);
            }
            ast::ExpressionType::Subscript { a, b } => {
                self.compile_expression(a)?;
                self.compile_expression(b)?;
                self.emit(Instruction::StoreSubscript);
            }
            ast::ExpressionType::Attribute { value, name } => {
                self.compile_expression(value)?;
                self.emit(Instruction::StoreAttr {
                    name: name.to_string(),
                });
            }
            ast::ExpressionType::List { elements } | ast::ExpressionType::Tuple { elements } => {
                let mut seen_star = false;

                // Scan for star args:
                for (i, element) in elements.iter().enumerate() {
                    if let ast::ExpressionType::Starred { .. } = &element.node {
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
                    if let ast::ExpressionType::Starred { value } = &element.node {
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

    /// Implement boolean short circuit evaluation logic.
    /// https://en.wikipedia.org/wiki/Short-circuit_evaluation
    ///
    /// This means, in a boolean statement 'x and y' the variable y will
    /// not be evaluated when x is false.
    ///
    /// The idea is to jump to a label if the expression is either true or false
    /// (indicated by the condition parameter).
    fn compile_jump_if(
        &mut self,
        expression: &ast::Expression,
        condition: bool,
        target_label: Label,
    ) -> Result<(), CompileError> {
        // Compile expression for test, and jump to label if false
        match &expression.node {
            ast::ExpressionType::BoolOp { op, values } => {
                match op {
                    ast::BooleanOperator::And => {
                        if condition {
                            // If all values are true.
                            let end_label = self.new_label();
                            let (last_value, values) = values.split_last().unwrap();

                            // If any of the values is false, we can short-circuit.
                            for value in values {
                                self.compile_jump_if(value, false, end_label)?;
                            }

                            // It depends upon the last value now: will it be true?
                            self.compile_jump_if(last_value, true, target_label)?;
                            self.set_label(end_label);
                        } else {
                            // If any value is false, the whole condition is false.
                            for value in values {
                                self.compile_jump_if(value, false, target_label)?;
                            }
                        }
                    }
                    ast::BooleanOperator::Or => {
                        if condition {
                            // If any of the values is true.
                            for value in values {
                                self.compile_jump_if(value, true, target_label)?;
                            }
                        } else {
                            // If all of the values are false.
                            let end_label = self.new_label();
                            let (last_value, values) = values.split_last().unwrap();

                            // If any value is true, we can short-circuit:
                            for value in values {
                                self.compile_jump_if(value, true, end_label)?;
                            }

                            // It all depends upon the last value now!
                            self.compile_jump_if(last_value, false, target_label)?;
                            self.set_label(end_label);
                        }
                    }
                }
            }
            ast::ExpressionType::Unop {
                op: ast::UnaryOperator::Not,
                a,
            } => {
                self.compile_jump_if(a, !condition, target_label)?;
            }
            _ => {
                // Fall back case which always will work!
                self.compile_expression(expression)?;
                if condition {
                    self.emit(Instruction::JumpIfTrue {
                        target: target_label,
                    });
                } else {
                    self.emit(Instruction::JumpIfFalse {
                        target: target_label,
                    });
                }
            }
        }
        Ok(())
    }

    /// Compile a boolean operation as an expression.
    /// This means, that the last value remains on the stack.
    fn compile_bool_op(
        &mut self,
        op: &ast::BooleanOperator,
        values: &[ast::Expression],
    ) -> Result<(), CompileError> {
        let end_label = self.new_label();

        let (last_value, values) = values.split_last().unwrap();
        for value in values {
            self.compile_expression(value)?;

            match op {
                ast::BooleanOperator::And => {
                    self.emit(Instruction::JumpIfFalseOrPop { target: end_label });
                }
                ast::BooleanOperator::Or => {
                    self.emit(Instruction::JumpIfTrueOrPop { target: end_label });
                }
            }
        }

        // If all values did not qualify, take the value of the last value:
        self.compile_expression(last_value)?;
        self.set_label(end_label);
        Ok(())
    }

    fn compile_expression(&mut self, expression: &ast::Expression) -> Result<(), CompileError> {
        trace!("Compiling {:?}", expression);
        self.set_source_location(&expression.location);

        use ast::ExpressionType::*;
        match &expression.node {
            Call {
                function,
                args,
                keywords,
            } => self.compile_call(function, args, keywords)?,
            BoolOp { op, values } => self.compile_bool_op(op, values)?,
            Binop { a, op, b } => {
                self.compile_expression(a)?;
                self.compile_expression(b)?;

                // Perform operation:
                self.compile_op(op, false);
            }
            Subscript { a, b } => {
                self.compile_expression(a)?;
                self.compile_expression(b)?;
                self.emit(Instruction::Subscript);
            }
            Unop { op, a } => {
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
            Attribute { value, name } => {
                self.compile_expression(value)?;
                self.emit(Instruction::LoadAttr {
                    name: name.to_string(),
                });
            }
            Compare { vals, ops } => {
                self.compile_chained_comparison(vals, ops)?;
            }
            Number { value } => {
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
            List { elements } => {
                let size = elements.len();
                let must_unpack = self.gather_elements(elements)?;
                self.emit(Instruction::BuildList {
                    size,
                    unpack: must_unpack,
                });
            }
            Tuple { elements } => {
                let size = elements.len();
                let must_unpack = self.gather_elements(elements)?;
                self.emit(Instruction::BuildTuple {
                    size,
                    unpack: must_unpack,
                });
            }
            Set { elements } => {
                let size = elements.len();
                let must_unpack = self.gather_elements(elements)?;
                self.emit(Instruction::BuildSet {
                    size,
                    unpack: must_unpack,
                });
            }
            Dict { elements } => {
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
            Slice { elements } => {
                let size = elements.len();
                for element in elements {
                    self.compile_expression(element)?;
                }
                self.emit(Instruction::BuildSlice { size });
            }
            Yield { value } => {
                if !self.in_function_def {
                    return Err(CompileError {
                        error: CompileErrorType::InvalidYield,
                        location: self.current_source_location.clone(),
                    });
                }
                self.mark_generator();
                match value {
                    Some(expression) => self.compile_expression(expression)?,
                    Option::None => self.emit(Instruction::LoadConst {
                        value: bytecode::Constant::None,
                    }),
                };
                self.emit(Instruction::YieldValue);
            }
            Await { .. } => {
                unimplemented!("await");
            }
            YieldFrom { value } => {
                self.mark_generator();
                self.compile_expression(value)?;
                self.emit(Instruction::GetIter);
                self.emit(Instruction::LoadConst {
                    value: bytecode::Constant::None,
                });
                self.emit(Instruction::YieldFrom);
            }
            True => {
                self.emit(Instruction::LoadConst {
                    value: bytecode::Constant::Boolean { value: true },
                });
            }
            False => {
                self.emit(Instruction::LoadConst {
                    value: bytecode::Constant::Boolean { value: false },
                });
            }
            None => {
                self.emit(Instruction::LoadConst {
                    value: bytecode::Constant::None,
                });
            }
            Ellipsis => {
                self.emit(Instruction::LoadConst {
                    value: bytecode::Constant::Ellipsis,
                });
            }
            String { value } => {
                self.compile_string(value)?;
            }
            Bytes { value } => {
                self.emit(Instruction::LoadConst {
                    value: bytecode::Constant::Bytes {
                        value: value.clone(),
                    },
                });
            }
            Identifier { name } => {
                self.load_name(name);
            }
            Lambda { args, body } => {
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
            Comprehension { kind, generators } => {
                self.compile_comprehension(kind, generators)?;
            }
            Starred { .. } => {
                use std::string::String;
                return Err(CompileError {
                    error: CompileErrorType::SyntaxError(String::from(
                        "Invalid starred expression",
                    )),
                    location: self.current_source_location.clone(),
                });
            }
            IfExpression { test, body, orelse } => {
                let no_label = self.new_label();
                let end_label = self.new_label();
                self.compile_jump_if(test, false, no_label)?;
                // True case
                self.compile_expression(body)?;
                self.emit(Instruction::Jump { target: end_label });
                // False case
                self.set_label(no_label);
                self.compile_expression(orelse)?;
                // End
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
            if let ast::ExpressionType::Starred { .. } = &e.node {
                true
            } else {
                false
            }
        });

        for element in elements {
            if let ast::ExpressionType::Starred { value } = &element.node {
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
        self.push_output(CodeObject::new(
            vec![".0".to_string()],
            Varargs::None,
            vec![],
            Varargs::None,
            self.source_path.clone().unwrap(),
            line_number,
            name.clone(),
        ));
        self.enter_scope();

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
            if generator.is_async {
                unimplemented!("async for comprehensions");
            }

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
                self.compile_jump_if(if_condition, false, start_label)?
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

        // Pop scope
        self.leave_scope();

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
        if let Some(value) = try_get_constant_string(string) {
            self.emit(Instruction::LoadConst {
                value: bytecode::Constant::String { value },
            });
        } else {
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
                        conversion: conversion.map(compile_conversion_flag),
                        spec: spec.clone(),
                    });
                }
            }
        }
        Ok(())
    }

    // Scope helpers:
    fn enter_scope(&mut self) {
        // println!("Enter scope {:?}", self.symbol_table_stack);
        // Enter first subscope!
        let table = self
            .symbol_table_stack
            .last_mut()
            .unwrap()
            .sub_tables
            .remove(0);
        self.symbol_table_stack.push(table);
    }

    fn leave_scope(&mut self) {
        // println!("Leave scope {:?}", self.symbol_table_stack);
        let table = self.symbol_table_stack.pop().unwrap();
        assert!(table.sub_tables.is_empty());
    }

    fn lookup_name(&self, name: &str) -> &Symbol {
        // println!("Looking up {:?}", name);
        let symbol_table = self.symbol_table_stack.last().unwrap();
        symbol_table.lookup(name).expect(
            "The symbol must be present in the symbol table, even when it is undefined in python.",
        )
    }

    // Low level helper functions:
    fn emit(&mut self, instruction: Instruction) {
        let location = compile_location(&self.current_source_location);
        // TODO: insert source filename
        self.current_output().emit(instruction, location);
    }

    fn current_output(&mut self) -> &mut O {
        self.output_stack
            .last_mut()
            .expect("No OutputStream on stack")
    }

    // Generate a new label
    fn new_label(&mut self) -> Label {
        let l = Label::new(self.nxt_label);
        self.nxt_label += 1;
        l
    }

    // Assign current position the given label
    fn set_label(&mut self, label: Label) {
        self.current_output().set_label(label)
    }

    fn set_source_location(&mut self, location: &ast::Location) {
        self.current_source_location = location.clone();
    }

    fn get_source_line_number(&mut self) -> usize {
        self.current_source_location.row()
    }

    fn create_qualified_name(&self, name: &str, suffix: &str) -> String {
        if let Some(ref qualified_path) = self.current_qualified_path {
            format!("{}.{}{}", qualified_path, name, suffix)
        } else {
            format!("{}{}", name, suffix)
        }
    }

    fn mark_generator(&mut self) {
        self.current_output().mark_generator();
    }
}

fn get_doc(body: &[ast::Statement]) -> (&[ast::Statement], Option<String>) {
    if let Some((val, body_rest)) = body.split_first() {
        if let ast::StatementType::Expression { ref expression } = val.node {
            if let ast::ExpressionType::String { value } = &expression.node {
                if let Some(value) = try_get_constant_string(value) {
                    return (body_rest, Some(value.to_string()));
                }
            }
        }
    }
    (body, None)
}

fn try_get_constant_string(string: &ast::StringGroup) -> Option<String> {
    fn get_constant_string_inner(out_string: &mut String, string: &ast::StringGroup) -> bool {
        match string {
            ast::StringGroup::Constant { value } => {
                out_string.push_str(&value);
                true
            }
            ast::StringGroup::Joined { values } => values
                .iter()
                .all(|value| get_constant_string_inner(out_string, value)),
            ast::StringGroup::FormattedValue { .. } => false,
        }
    }
    let mut out_string = String::new();
    if get_constant_string_inner(&mut out_string, string) {
        Some(out_string)
    } else {
        None
    }
}

fn compile_location(location: &ast::Location) -> bytecode::Location {
    bytecode::Location::new(location.row(), location.column())
}

fn compile_varargs(varargs: &ast::Varargs) -> bytecode::Varargs {
    match varargs {
        ast::Varargs::None => bytecode::Varargs::None,
        ast::Varargs::Unnamed => bytecode::Varargs::Unnamed,
        ast::Varargs::Named(param) => bytecode::Varargs::Named(param.arg.clone()),
    }
}

fn compile_conversion_flag(conversion_flag: ast::ConversionFlag) -> bytecode::ConversionFlag {
    match conversion_flag {
        ast::ConversionFlag::Ascii => bytecode::ConversionFlag::Ascii,
        ast::ConversionFlag::Repr => bytecode::ConversionFlag::Repr,
        ast::ConversionFlag::Str => bytecode::ConversionFlag::Str,
    }
}

#[cfg(test)]
mod tests {
    use super::Compiler;
    use crate::symboltable::make_symbol_table;
    use rustpython_bytecode::bytecode::Constant::*;
    use rustpython_bytecode::bytecode::Instruction::*;
    use rustpython_bytecode::bytecode::{CodeObject, Label};
    use rustpython_parser::parser;

    fn compile_exec(source: &str) -> CodeObject {
        let mut compiler: Compiler = Default::default();
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
                JumpIfTrue {
                    target: Label::new(1)
                },
                LoadConst {
                    value: Boolean { value: false }
                },
                JumpIfTrue {
                    target: Label::new(1)
                },
                LoadConst {
                    value: Boolean { value: false }
                },
                JumpIfFalse {
                    target: Label::new(0)
                },
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
                JumpIfFalse {
                    target: Label::new(0)
                },
                LoadConst {
                    value: Boolean { value: false }
                },
                JumpIfFalse {
                    target: Label::new(0)
                },
                LoadConst {
                    value: Boolean { value: false }
                },
                JumpIfFalse {
                    target: Label::new(0)
                },
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
                JumpIfFalse {
                    target: Label::new(2)
                },
                LoadConst {
                    value: Boolean { value: false }
                },
                JumpIfTrue {
                    target: Label::new(1)
                },
                LoadConst {
                    value: Boolean { value: false }
                },
                JumpIfFalse {
                    target: Label::new(0)
                },
                LoadConst {
                    value: Boolean { value: true }
                },
                JumpIfFalse {
                    target: Label::new(0)
                },
                LoadConst { value: None },
                ReturnValue
            ],
            code.instructions
        );
    }

    #[test]
    fn test_constant_optimization() {
        let code = compile_exec("1 + 2 + 3 + 4\n1.5 * 2.5");
        assert_eq!(
            code.instructions,
            vec![
                LoadConst {
                    value: Integer { value: 10.into() }
                },
                Pop,
                LoadConst {
                    value: Float { value: 3.75 }
                },
                Pop,
                LoadConst { value: None },
                ReturnValue,
            ]
        );
    }
}
