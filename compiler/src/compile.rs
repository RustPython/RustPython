//!
//! Take an AST and transform it into bytecode
//!
//! Inspirational code:
//!   https://github.com/python/cpython/blob/master/Python/compile.c
//!   https://github.com/micropython/micropython/blob/master/py/compile.c

use crate::error::{CompileError, CompileErrorType};
pub use crate::mode::Mode;
use crate::symboltable::{make_symbol_table, statements_to_symbol_table, SymbolScope, SymbolTable};
use indexmap::IndexSet;
use itertools::Itertools;
use num_complex::Complex64;
use num_traits::ToPrimitive;
use rustpython_ast as ast;
use rustpython_bytecode::bytecode::{self, CodeObject, ConstantData, Instruction, Label};

type CompileResult<T> = Result<T, CompileError>;

struct CodeInfo {
    code: CodeObject,
    instructions: Vec<Instruction>,
    locations: Vec<bytecode::Location>,
    constants: Vec<ConstantData>,
    name_cache: IndexSet<String>,
    varname_cache: IndexSet<String>,
    cellvar_cache: IndexSet<String>,
    freevar_cache: IndexSet<String>,
    label_map: Vec<Option<Label>>,
}
impl CodeInfo {
    fn finalize_code(self) -> CodeObject {
        let CodeInfo {
            mut code,
            instructions,
            locations,
            constants,
            name_cache,
            varname_cache,
            cellvar_cache,
            freevar_cache,
            label_map,
        } = self;

        code.instructions = instructions.into();
        code.locations = locations.into();
        code.constants = constants.into();
        code.names = name_cache.into_iter().collect();
        code.varnames = varname_cache.into_iter().collect();
        code.cellvars = cellvar_cache.into_iter().collect();
        code.freevars = freevar_cache.into_iter().collect();

        if !code.cellvars.is_empty() {
            let total_args = code.arg_count
                + code.kwonlyarg_count
                + code.flags.contains(bytecode::CodeFlags::HAS_VARARGS) as usize
                + code.flags.contains(bytecode::CodeFlags::HAS_VARKEYWORDS) as usize;
            let all_args = &code.varnames[..total_args];
            let mut found_cellarg = false;
            let cell2arg = code
                .cellvars
                .iter()
                .map(|var| {
                    all_args.iter().position(|arg| var == arg).map_or(-1, |i| {
                        found_cellarg = true;
                        i as isize
                    })
                })
                .collect::<Box<[_]>>();
            if found_cellarg {
                code.cell2arg = Some(cell2arg);
            }
        }

        for instruction in &mut *code.instructions {
            use Instruction::*;
            // this is a little bit hacky, as until now the data stored inside Labels in
            // Instructions is just bookkeeping, but I think it's the best way to do this
            // XXX: any new instruction that uses a label has to be added here
            match instruction {
                Jump { target: l }
                | JumpIfTrue { target: l }
                | JumpIfFalse { target: l }
                | JumpIfTrueOrPop { target: l }
                | JumpIfFalseOrPop { target: l }
                | ForIter { target: l }
                | SetupFinally { handler: l }
                | SetupExcept { handler: l }
                | SetupWith { end: l }
                | SetupAsyncWith { end: l }
                | SetupLoop { end: l } => {
                    *l = label_map[l.0].expect("label never set");
                }

                _ => {}
            }
        }
        code
    }
}

enum NameUsage {
    Load,
    Store,
    Delete,
}

/// Main structure holding the state of compilation.
struct Compiler {
    code_stack: Vec<CodeInfo>,
    symbol_table_stack: Vec<SymbolTable>,
    source_path: String,
    current_source_location: ast::Location,
    current_qualified_path: Option<String>,
    done_with_future_stmts: bool,
    ctx: CompileContext,
    opts: CompileOpts,
}

#[derive(Debug, Clone)]
pub struct CompileOpts {
    /// How optimized the bytecode output should be; any optimize > 0 does
    /// not emit assert statements
    pub optimize: u8,
}
impl Default for CompileOpts {
    fn default() -> Self {
        CompileOpts { optimize: 0 }
    }
}

#[derive(Debug, Clone, Copy)]
struct CompileContext {
    in_loop: bool,
    in_class: bool,
    func: FunctionContext,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum FunctionContext {
    NoFunction,
    Function,
    AsyncFunction,
}

impl CompileContext {
    fn in_func(self) -> bool {
        self.func != FunctionContext::NoFunction
    }
}

/// A helper function for the shared code of the different compile functions
fn with_compiler(
    source_path: String,
    opts: CompileOpts,
    f: impl FnOnce(&mut Compiler) -> CompileResult<()>,
) -> CompileResult<CodeObject> {
    let mut compiler = Compiler::new(opts, source_path, "<module>".to_owned());
    f(&mut compiler)?;
    let code = compiler.pop_code_object();
    trace!("Compilation completed: {:?}", code);
    Ok(code)
}

/// Compile a standard Python program to bytecode
pub fn compile_program(
    ast: ast::Program,
    source_path: String,
    opts: CompileOpts,
) -> CompileResult<CodeObject> {
    let symbol_table = match make_symbol_table(&ast) {
        Ok(x) => x,
        Err(e) => return Err(e.into_compile_error(source_path)),
    };
    with_compiler(source_path, opts, |compiler| {
        compiler.compile_program(&ast, symbol_table)
    })
}

/// Compile a single Python expression to bytecode
pub fn compile_statement_eval(
    statement: Vec<ast::Statement>,
    source_path: String,
    opts: CompileOpts,
) -> CompileResult<CodeObject> {
    let symbol_table = match statements_to_symbol_table(&statement) {
        Ok(x) => x,
        Err(e) => return Err(e.into_compile_error(source_path)),
    };
    with_compiler(source_path, opts, |compiler| {
        compiler.compile_statement_eval(&statement, symbol_table)
    })
}

/// Compile a Python program to bytecode for the context of a REPL
pub fn compile_program_single(
    ast: ast::Program,
    source_path: String,
    opts: CompileOpts,
) -> CompileResult<CodeObject> {
    let symbol_table = match make_symbol_table(&ast) {
        Ok(x) => x,
        Err(e) => return Err(e.into_compile_error(source_path)),
    };
    with_compiler(source_path, opts, |compiler| {
        compiler.compile_program_single(&ast, symbol_table)
    })
}

impl Compiler {
    fn new(opts: CompileOpts, source_path: String, code_name: String) -> Self {
        let module_code = CodeInfo {
            code: CodeObject::new(
                bytecode::CodeFlags::NEW_LOCALS,
                0,
                0,
                0,
                source_path.clone(),
                0,
                code_name,
            ),
            instructions: Vec::new(),
            locations: Vec::new(),
            constants: Vec::new(),
            name_cache: IndexSet::new(),
            varname_cache: IndexSet::new(),
            cellvar_cache: IndexSet::new(),
            freevar_cache: IndexSet::new(),
            label_map: Vec::new(),
        };
        Compiler {
            code_stack: vec![module_code],
            symbol_table_stack: Vec::new(),
            source_path,
            current_source_location: ast::Location::default(),
            current_qualified_path: None,
            done_with_future_stmts: false,
            ctx: CompileContext {
                in_loop: false,
                in_class: false,
                func: FunctionContext::NoFunction,
            },
            opts,
        }
    }

    fn error(&self, error: CompileErrorType) -> CompileError {
        self.error_loc(error, self.current_source_location)
    }
    fn error_loc(&self, error: CompileErrorType, location: ast::Location) -> CompileError {
        CompileError {
            error,
            location,
            source_path: self.source_path.clone(),
        }
    }

    fn push_output(&mut self, code: CodeObject) {
        let table = self
            .symbol_table_stack
            .last_mut()
            .unwrap()
            .sub_tables
            .remove(0);

        let cellvar_cache = table
            .symbols
            .iter()
            .filter(|(_, s)| s.scope == SymbolScope::Cell)
            .map(|(var, _)| var.clone())
            .collect();
        let freevar_cache = table
            .symbols
            .iter()
            .filter(|(_, s)| s.scope == SymbolScope::Free || s.is_free_class)
            .map(|(var, _)| var.clone())
            .collect();

        self.symbol_table_stack.push(table);

        let info = CodeInfo {
            code,
            instructions: Vec::new(),
            locations: Vec::new(),
            constants: Vec::new(),
            name_cache: IndexSet::new(),
            varname_cache: IndexSet::new(),
            cellvar_cache,
            freevar_cache,
            label_map: Vec::new(),
        };
        self.code_stack.push(info);
    }

    fn pop_code_object(&mut self) -> CodeObject {
        let table = self.symbol_table_stack.pop().unwrap();
        assert!(table.sub_tables.is_empty());
        self.code_stack.pop().unwrap().finalize_code()
    }

    // could take impl Into<Cow<str>>, but everything is borrowed from ast structs; we never
    // actually have a `String` to pass
    fn name(&mut self, name: &str) -> bytecode::NameIdx {
        self._name_inner(name, |i| &mut i.name_cache)
    }
    fn varname(&mut self, name: &str) -> bytecode::NameIdx {
        self._name_inner(name, |i| &mut i.varname_cache)
    }
    fn _name_inner(
        &mut self,
        name: &str,
        cache: impl FnOnce(&mut CodeInfo) -> &mut IndexSet<String>,
    ) -> bytecode::NameIdx {
        let cache = cache(self.current_codeinfo());
        cache
            .get_index_of(name)
            .unwrap_or_else(|| cache.insert_full(name.to_owned()).0)
    }

    fn compile_program(
        &mut self,
        program: &ast::Program,
        symbol_table: SymbolTable,
    ) -> CompileResult<()> {
        let size_before = self.code_stack.len();
        self.symbol_table_stack.push(symbol_table);

        let (statements, doc) = get_doc(&program.statements);
        if let Some(value) = doc {
            self.emit_constant(ConstantData::Str { value });
            let doc = self.name("__doc__");
            self.emit(Instruction::StoreGlobal(doc))
        }

        if self.find_ann(statements) {
            self.emit(Instruction::SetupAnnotation);
        }

        self.compile_statements(statements)?;

        assert_eq!(self.code_stack.len(), size_before);

        // Emit None at end:
        self.emit_constant(ConstantData::None);
        self.emit(Instruction::ReturnValue);
        Ok(())
    }

    fn compile_program_single(
        &mut self,
        program: &ast::Program,
        symbol_table: SymbolTable,
    ) -> CompileResult<()> {
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
            self.emit_constant(ConstantData::None);
            self.emit(Instruction::ReturnValue);
        }

        Ok(())
    }

    // Compile statement in eval mode:
    fn compile_statement_eval(
        &mut self,
        statements: &[ast::Statement],
        symbol_table: SymbolTable,
    ) -> CompileResult<()> {
        self.symbol_table_stack.push(symbol_table);
        for statement in statements {
            if let ast::StatementType::Expression { ref expression } = statement.node {
                self.compile_expression(expression)?;
            } else {
                return Err(self.error_loc(CompileErrorType::ExpectExpr, statement.location));
            }
        }
        self.emit(Instruction::ReturnValue);
        Ok(())
    }

    fn compile_statements(&mut self, statements: &[ast::Statement]) -> CompileResult<()> {
        for statement in statements {
            self.compile_statement(statement)?
        }
        Ok(())
    }

    fn load_name(&mut self, name: &str) {
        self.compile_name(name, NameUsage::Load)
    }

    fn store_name(&mut self, name: &str) {
        self.compile_name(name, NameUsage::Store)
    }

    fn compile_name(&mut self, name: &str, usage: NameUsage) {
        let symbol_table = self.symbol_table_stack.last().unwrap();
        let symbol = symbol_table.lookup(name).expect(
            "The symbol must be present in the symbol table, even when it is undefined in python.",
        );
        let info = self.code_stack.last_mut().unwrap();
        let mut cache = &mut info.name_cache;
        enum NameOpType {
            Fast,
            Global,
            Deref,
            Local,
        }
        let op_typ = match symbol.scope {
            SymbolScope::Local if self.ctx.in_func() => {
                cache = &mut info.varname_cache;
                NameOpType::Fast
            }
            SymbolScope::GlobalExplicit => NameOpType::Global,
            SymbolScope::GlobalImplicit | SymbolScope::Unknown if self.ctx.in_func() => {
                NameOpType::Global
            }
            SymbolScope::GlobalImplicit | SymbolScope::Unknown => NameOpType::Local,
            SymbolScope::Local => NameOpType::Local,
            SymbolScope::Free => {
                cache = &mut info.freevar_cache;
                NameOpType::Deref
            }
            SymbolScope::Cell => {
                cache = &mut info.cellvar_cache;
                NameOpType::Deref
            }
            // // TODO: is this right?
            // SymbolScope::Unknown => NameOpType::Global,
        };
        let mut idx = cache
            .get_index_of(name)
            .unwrap_or_else(|| cache.insert_full(name.to_owned()).0);
        if let SymbolScope::Free = symbol.scope {
            idx += info.cellvar_cache.len();
        }
        let op = match op_typ {
            NameOpType::Fast => match usage {
                NameUsage::Load => Instruction::LoadFast,
                NameUsage::Store => Instruction::StoreFast,
                NameUsage::Delete => Instruction::DeleteFast,
            },
            NameOpType::Global => match usage {
                NameUsage::Load => Instruction::LoadGlobal,
                NameUsage::Store => Instruction::StoreGlobal,
                NameUsage::Delete => Instruction::DeleteGlobal,
            },
            NameOpType::Deref => match usage {
                NameUsage::Load if !self.ctx.in_func() && self.ctx.in_class => {
                    Instruction::LoadClassDeref
                }
                NameUsage::Load => Instruction::LoadDeref,
                NameUsage::Store => Instruction::StoreDeref,
                NameUsage::Delete => Instruction::DeleteDeref,
            },
            NameOpType::Local => match usage {
                NameUsage::Load => Instruction::LoadNameAny,
                NameUsage::Store => Instruction::StoreLocal,
                NameUsage::Delete => Instruction::DeleteLocal,
            },
        };
        self.emit(op(idx));
    }

    fn compile_statement(&mut self, statement: &ast::Statement) -> CompileResult<()> {
        trace!("Compiling {:?}", statement);
        self.set_source_location(statement.location);
        use ast::StatementType::*;

        match &statement.node {
            // we do this here because `from __future__` still executes that `from` statement at runtime,
            // we still need to compile the ImportFrom down below
            ImportFrom { module, names, .. } if module.as_deref() == Some("__future__") => {
                self.compile_future_features(&names)?
            }
            // if we find any other statement, stop accepting future statements
            _ => self.done_with_future_stmts = true,
        }

        match &statement.node {
            Import { names } => {
                // import a, b, c as d
                for name in names {
                    self.emit_constant(ConstantData::Integer {
                        value: num_traits::Zero::zero(),
                    });
                    self.emit_constant(ConstantData::None);
                    let idx = self.name(&name.symbol);
                    self.emit(Instruction::ImportName { idx });
                    if let Some(alias) = &name.alias {
                        for part in name.symbol.split('.').skip(1) {
                            let idx = self.name(part);
                            self.emit(Instruction::LoadAttr { idx });
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

                let from_list = if import_star {
                    if self.ctx.in_func() {
                        return Err(self
                            .error_loc(CompileErrorType::FunctionImportStar, statement.location));
                    }
                    vec![ConstantData::Str {
                        value: "*".to_owned(),
                    }]
                } else {
                    names
                        .iter()
                        .map(|n| ConstantData::Str {
                            value: n.symbol.to_owned(),
                        })
                        .collect()
                };

                let module_idx = module.as_ref().map(|s| self.name(s));

                // from .... import (*fromlist)
                self.emit_constant(ConstantData::Integer {
                    value: (*level).into(),
                });
                self.emit_constant(ConstantData::Tuple {
                    elements: from_list,
                });
                if let Some(idx) = module_idx {
                    self.emit(Instruction::ImportName { idx });
                } else {
                    self.emit(Instruction::ImportNameless);
                }

                if import_star {
                    // from .... import *
                    self.emit(Instruction::ImportStar);
                } else {
                    // from mod import a, b as c

                    for name in names {
                        let idx = self.name(&name.symbol);
                        // import symbol from module:
                        self.emit(Instruction::ImportFrom { idx });

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
                let is_async = *is_async;

                let end_labels = items
                    .iter()
                    .map(|item| {
                        let end_label = self.new_label();
                        self.compile_expression(&item.context_expr)?;

                        if is_async {
                            self.emit(Instruction::BeforeAsyncWith);
                            self.emit(Instruction::GetAwaitable);
                            self.emit_constant(ConstantData::None);
                            self.emit(Instruction::YieldFrom);
                            self.emit(Instruction::SetupAsyncWith { end: end_label });
                        } else {
                            self.emit(Instruction::SetupWith { end: end_label });
                        }

                        match &item.optional_vars {
                            Some(var) => {
                                self.compile_store(var)?;
                            }
                            None => {
                                self.emit(Instruction::Pop);
                            }
                        }
                        Ok(end_label)
                    })
                    .collect::<CompileResult<Vec<_>>>()?;

                self.compile_statements(body)?;

                // sort of "stack up" the layers of with blocks:
                // with a, b: body -> start_with(a) start_with(b) body() end_with(b) end_with(a)
                for end_label in end_labels.into_iter().rev() {
                    self.emit(Instruction::PopBlock);
                    self.emit(Instruction::EnterFinally);
                    self.set_label(end_label);
                    self.emit(Instruction::WithCleanupStart);

                    if is_async {
                        self.emit(Instruction::GetAwaitable);
                        self.emit_constant(ConstantData::None);
                        self.emit(Instruction::YieldFrom);
                    }

                    self.emit(Instruction::WithCleanupFinish);
                }
            }
            For {
                is_async,
                target,
                iter,
                body,
                orelse,
            } => self.compile_for(target, iter, body, orelse, *is_async)?,
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
                self.compile_function_def(name, args, body, decorator_list, returns, *is_async)?;
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
                if self.opts.optimize == 0 {
                    let end_label = self.new_label();
                    self.compile_jump_if(test, true, end_label)?;
                    let assertion_error = self.name("AssertionError");
                    self.emit(Instruction::LoadGlobal(assertion_error));
                    match msg {
                        Some(e) => {
                            self.compile_expression(e)?;
                            self.emit(Instruction::CallFunctionPositional { nargs: 1 });
                        }
                        None => {
                            self.emit(Instruction::CallFunctionPositional { nargs: 0 });
                        }
                    }
                    self.emit(Instruction::Raise { argc: 1 });
                    self.set_label(end_label);
                }
            }
            Break => {
                if !self.ctx.in_loop {
                    return Err(self.error_loc(CompileErrorType::InvalidBreak, statement.location));
                }
                self.emit(Instruction::Break);
            }
            Continue => {
                if !self.ctx.in_loop {
                    return Err(
                        self.error_loc(CompileErrorType::InvalidContinue, statement.location)
                    );
                }
                self.emit(Instruction::Continue);
            }
            Return { value } => {
                if !self.ctx.in_func() {
                    return Err(self.error_loc(CompileErrorType::InvalidReturn, statement.location));
                }
                match value {
                    Some(v) => {
                        if self.ctx.func == FunctionContext::AsyncFunction
                            && self
                                .current_code()
                                .flags
                                .contains(bytecode::CodeFlags::IS_GENERATOR)
                        {
                            return Err(self.error_loc(
                                CompileErrorType::AsyncReturnValue,
                                statement.location,
                            ));
                        }
                        self.compile_expression(v)?;
                    }
                    None => {
                        self.emit_constant(ConstantData::None);
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

    fn compile_delete(&mut self, expression: &ast::Expression) -> CompileResult<()> {
        match &expression.node {
            ast::ExpressionType::Identifier { name } => {
                self.compile_name(name, NameUsage::Delete);
            }
            ast::ExpressionType::Attribute { value, name } => {
                self.compile_expression(value)?;
                let idx = self.name(name);
                self.emit(Instruction::DeleteAttr { idx });
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
            _ => return Err(self.error(CompileErrorType::Delete(expression.name()))),
        }
        Ok(())
    }

    fn enter_function(&mut self, name: &str, args: &ast::Parameters) -> CompileResult<()> {
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
                self.emit_constant(ConstantData::Str {
                    value: kw.arg.clone(),
                });
                self.compile_expression(default)?;
                num_kw_only_defaults += 1;
            }
        }
        if num_kw_only_defaults > 0 {
            self.emit(Instruction::BuildMap {
                size: num_kw_only_defaults,
                unpack: false,
                for_call: false,
            });
        }

        let mut flags = bytecode::CodeFlags::NEW_LOCALS | bytecode::CodeFlags::IS_OPTIMIZED;
        if have_defaults {
            flags |= bytecode::CodeFlags::HAS_DEFAULTS;
        }
        if num_kw_only_defaults > 0 {
            flags |= bytecode::CodeFlags::HAS_KW_ONLY_DEFAULTS;
        }

        let line_number = self.get_source_line_number();
        self.push_output(CodeObject::new(
            flags,
            args.posonlyargs_count,
            args.args.len(),
            args.kwonlyargs.len(),
            self.source_path.clone(),
            line_number,
            name.to_owned(),
        ));

        for name in &args.args {
            self.varname(&name.arg);
        }
        for name in &args.kwonlyargs {
            self.varname(&name.arg);
        }

        let mut compile_varargs = |va: &ast::Varargs, flag| match va {
            ast::Varargs::None | ast::Varargs::Unnamed => {}
            ast::Varargs::Named(name) => {
                self.current_code().flags |= flag;
                self.varname(&name.arg);
            }
        };

        compile_varargs(&args.vararg, bytecode::CodeFlags::HAS_VARARGS);
        compile_varargs(&args.kwarg, bytecode::CodeFlags::HAS_VARKEYWORDS);

        Ok(())
    }

    fn prepare_decorators(&mut self, decorator_list: &[ast::Expression]) -> CompileResult<()> {
        for decorator in decorator_list {
            self.compile_expression(decorator)?;
        }
        Ok(())
    }

    fn apply_decorators(&mut self, decorator_list: &[ast::Expression]) {
        // Apply decorators:
        for _ in decorator_list {
            self.emit(Instruction::CallFunctionPositional { nargs: 1 });
        }
    }

    fn compile_try_statement(
        &mut self,
        body: &[ast::Statement],
        handlers: &[ast::ExceptHandler],
        orelse: &Option<ast::Suite>,
        finalbody: &Option<ast::Suite>,
    ) -> CompileResult<()> {
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
                self.compile_expression(exc_type)?;
                self.emit(Instruction::CompareOperation {
                    op: bytecode::ComparisonOperator::ExceptionMatch,
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
        is_async: bool,
    ) -> CompileResult<()> {
        // Create bytecode for this function:

        self.prepare_decorators(decorator_list)?;
        self.enter_function(name, args)?;

        // remember to restore self.ctx.in_loop to the original after the function is compiled
        let prev_ctx = self.ctx;

        self.ctx = CompileContext {
            in_loop: false,
            in_class: prev_ctx.in_class,
            func: if is_async {
                FunctionContext::AsyncFunction
            } else {
                FunctionContext::Function
            },
        };

        let qualified_name = self.create_qualified_name(name, "");
        let old_qualified_path = self.current_qualified_path.take();
        self.current_qualified_path = Some(self.create_qualified_name(name, ".<locals>"));

        let (body, doc_str) = get_doc(body);

        self.compile_statements(body)?;

        // Emit None at end:
        match body.last().map(|s| &s.node) {
            Some(ast::StatementType::Return { .. }) => {
                // the last instruction is a ReturnValue already, we don't need to emit it
            }
            _ => {
                self.emit_constant(ConstantData::None);
                self.emit(Instruction::ReturnValue);
            }
        }

        let mut code = self.pop_code_object();
        self.current_qualified_path = old_qualified_path;
        self.ctx = prev_ctx;

        // Prepare type annotations:
        let mut num_annotations = 0;

        // Return annotation:
        if let Some(annotation) = returns {
            // key:
            self.emit_constant(ConstantData::Str {
                value: "return".to_owned(),
            });
            // value:
            self.compile_expression(annotation)?;
            num_annotations += 1;
        }

        let mut visit_arg_annotation = |arg: &ast::Parameter| -> CompileResult<()> {
            if let Some(annotation) = &arg.annotation {
                self.emit_constant(ConstantData::Str {
                    value: arg.arg.to_owned(),
                });
                self.compile_expression(&annotation)?;
                num_annotations += 1;
            }
            Ok(())
        };

        for arg in args.args.iter().chain(args.kwonlyargs.iter()) {
            visit_arg_annotation(arg)?;
        }

        if let ast::Varargs::Named(arg) = &args.vararg {
            visit_arg_annotation(arg)?;
        }

        if let ast::Varargs::Named(arg) = &args.kwarg {
            visit_arg_annotation(arg)?;
        }

        if num_annotations > 0 {
            code.flags |= bytecode::CodeFlags::HAS_ANNOTATIONS;
            self.emit(Instruction::BuildMap {
                size: num_annotations,
                unpack: false,
                for_call: false,
            });
        }

        if is_async {
            code.flags |= bytecode::CodeFlags::IS_COROUTINE;
        }

        self.build_closure(&code);

        self.emit_constant(ConstantData::Code {
            code: Box::new(code),
        });
        self.emit_constant(ConstantData::Str {
            value: qualified_name,
        });

        // Turn code object into function object:
        self.emit(Instruction::MakeFunction);

        self.emit(Instruction::Duplicate);
        self.load_docstring(doc_str);
        self.emit(Instruction::Rotate { amount: 2 });
        let doc = self.name("__doc__");
        self.emit(Instruction::StoreAttr { idx: doc });

        self.apply_decorators(decorator_list);

        self.store_name(name);

        Ok(())
    }

    fn build_closure(&mut self, code: &CodeObject) {
        if !code.freevars.is_empty() {
            for var in &*code.freevars {
                let table = self.symbol_table_stack.last().unwrap();
                let symbol = table.lookup(var).unwrap();
                let parent_code = self.code_stack.last().unwrap();
                let vars = match symbol.scope {
                    SymbolScope::Free => &parent_code.freevar_cache,
                    SymbolScope::Cell => &parent_code.cellvar_cache,
                    _ if symbol.is_free_class => &parent_code.freevar_cache,
                    x => unreachable!(
                        "var {} in a {:?} should be free or cell but it's {:?}",
                        var, table.typ, x
                    ),
                };
                let mut idx = vars.get_index_of(var).unwrap();
                if let SymbolScope::Free = symbol.scope {
                    idx += parent_code.cellvar_cache.len();
                }
                self.emit(Instruction::LoadClosure(idx))
            }
            self.emit(Instruction::BuildTuple {
                size: code.freevars.len(),
                unpack: false,
            })
        }
    }

    fn find_ann(&self, body: &[ast::Statement]) -> bool {
        use ast::StatementType::*;
        let option_stmt_to_bool = |suit: &Option<ast::Suite>| -> bool {
            match suit {
                Some(stmts) => self.find_ann(stmts),
                None => false,
            }
        };

        for statement in body {
            let res = match &statement.node {
                AnnAssign {
                    target: _,
                    annotation: _,
                    value: _,
                } => true,
                For {
                    is_async: _,
                    target: _,
                    iter: _,
                    body,
                    orelse,
                } => self.find_ann(body) || option_stmt_to_bool(orelse),
                If {
                    test: _,
                    body,
                    orelse,
                } => self.find_ann(body) || option_stmt_to_bool(orelse),
                While {
                    test: _,
                    body,
                    orelse,
                } => self.find_ann(body) || option_stmt_to_bool(orelse),
                With {
                    is_async: _,
                    items: _,
                    body,
                } => self.find_ann(body),
                Try {
                    body,
                    handlers: _,
                    orelse,
                    finalbody,
                } => {
                    self.find_ann(&body)
                        || option_stmt_to_bool(orelse)
                        || option_stmt_to_bool(finalbody)
                }
                _ => false,
            };
            if res {
                return true;
            }
        }
        false
    }

    fn compile_class_def(
        &mut self,
        name: &str,
        body: &[ast::Statement],
        bases: &[ast::Expression],
        keywords: &[ast::Keyword],
        decorator_list: &[ast::Expression],
    ) -> CompileResult<()> {
        self.prepare_decorators(decorator_list)?;

        let prev_ctx = self.ctx;
        self.ctx = CompileContext {
            func: FunctionContext::NoFunction,
            in_class: true,
            in_loop: false,
        };

        let qualified_name = self.create_qualified_name(name, "");
        let old_qualified_path = self.current_qualified_path.take();
        self.current_qualified_path = Some(qualified_name.clone());

        self.emit(Instruction::LoadBuildClass);
        let line_number = self.get_source_line_number();
        self.push_output(CodeObject::new(
            bytecode::CodeFlags::empty(),
            0,
            0,
            0,
            self.source_path.clone(),
            line_number,
            name.to_owned(),
        ));

        let (new_body, doc_str) = get_doc(body);

        let dunder_name = self.name("__name__");
        self.emit(Instruction::LoadGlobal(dunder_name));
        let dunder_module = self.name("__module__");
        self.emit(Instruction::StoreLocal(dunder_module));
        self.emit_constant(ConstantData::Str {
            value: qualified_name.clone(),
        });
        let qualname = self.name("__qualname__");
        self.emit(Instruction::StoreLocal(qualname));
        self.load_docstring(doc_str);
        let doc = self.name("__doc__");
        self.emit(Instruction::StoreLocal(doc));
        // setup annotations
        if self.find_ann(body) {
            self.emit(Instruction::SetupAnnotation);
        }
        self.compile_statements(new_body)?;

        let classcell_idx = self
            .code_stack
            .last_mut()
            .unwrap()
            .cellvar_cache
            .iter()
            .position(|var| *var == "__class__");

        if let Some(classcell_idx) = classcell_idx {
            self.emit(Instruction::LoadClosure(classcell_idx));
            self.emit(Instruction::Duplicate);
            let classcell = self.name("__classcell__");
            self.emit(Instruction::StoreLocal(classcell));
        } else {
            self.emit_constant(ConstantData::None);
        }

        self.emit(Instruction::ReturnValue);

        let code = self.pop_code_object();

        self.current_qualified_path = old_qualified_path;
        self.ctx = prev_ctx;

        self.build_closure(&code);

        self.emit_constant(ConstantData::Code {
            code: Box::new(code),
        });
        self.emit_constant(ConstantData::Str {
            value: name.to_owned(),
        });

        // Turn code object into function object:
        self.emit(Instruction::MakeFunction);

        self.emit_constant(ConstantData::Str {
            value: qualified_name,
        });

        for base in bases {
            self.compile_expression(base)?;
        }

        if !keywords.is_empty() {
            let mut kwarg_names = vec![];
            for keyword in keywords {
                if let Some(name) = &keyword.name {
                    kwarg_names.push(ConstantData::Str {
                        value: name.to_owned(),
                    });
                } else {
                    // This means **kwargs!
                    panic!("name must be set");
                }
                self.compile_expression(&keyword.value)?;
            }

            self.emit_constant(ConstantData::Tuple {
                elements: kwarg_names,
            });
            self.emit(Instruction::CallFunctionKeyword {
                nargs: 2 + keywords.len() + bases.len(),
            });
        } else {
            self.emit(Instruction::CallFunctionPositional {
                nargs: 2 + bases.len(),
            });
        }

        self.apply_decorators(decorator_list);

        self.store_name(name);
        Ok(())
    }

    fn load_docstring(&mut self, doc_str: Option<String>) {
        // TODO: __doc__ must be default None and no bytecodes unless it is Some
        // Duplicate top of stack (the function or class object)

        // Doc string value:
        self.emit_constant(match doc_str {
            Some(doc) => ConstantData::Str { value: doc },
            None => ConstantData::None, // set docstring None if not declared
        });
    }

    fn compile_while(
        &mut self,
        test: &ast::Expression,
        body: &[ast::Statement],
        orelse: &Option<Vec<ast::Statement>>,
    ) -> CompileResult<()> {
        let start_label = self.new_label();
        let else_label = self.new_label();
        let end_label = self.new_label();

        self.emit(Instruction::SetupLoop { end: end_label });
        self.set_label(start_label);

        self.compile_jump_if(test, false, else_label)?;

        let was_in_loop = self.ctx.in_loop;
        self.ctx.in_loop = true;
        self.compile_statements(body)?;
        self.ctx.in_loop = was_in_loop;
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
        is_async: bool,
    ) -> CompileResult<()> {
        // Start loop
        let start_label = self.new_label();
        let else_label = self.new_label();
        let end_label = self.new_label();

        // The thing iterated:
        self.compile_expression(iter)?;

        if is_async {
            let check_asynciter_label = self.new_label();
            let body_label = self.new_label();

            self.emit(Instruction::GetAIter);

            self.emit(Instruction::SetupLoop { end: end_label });
            self.set_label(start_label);
            self.emit(Instruction::SetupExcept {
                handler: check_asynciter_label,
            });
            self.emit(Instruction::GetANext);
            self.emit_constant(ConstantData::None);
            self.emit(Instruction::YieldFrom);
            self.compile_store(target)?;
            self.emit(Instruction::PopBlock);
            self.emit(Instruction::Jump { target: body_label });

            self.set_label(check_asynciter_label);
            self.emit(Instruction::Duplicate);
            let stopasynciter = self.name("StopAsyncIteration");
            self.emit(Instruction::LoadGlobal(stopasynciter));
            self.emit(Instruction::CompareOperation {
                op: bytecode::ComparisonOperator::ExceptionMatch,
            });
            self.emit(Instruction::JumpIfTrue { target: else_label });
            self.emit(Instruction::Raise { argc: 0 });

            let was_in_loop = self.ctx.in_loop;
            self.ctx.in_loop = true;
            self.set_label(body_label);
            self.compile_statements(body)?;
            self.ctx.in_loop = was_in_loop;
        } else {
            // Retrieve Iterator
            self.emit(Instruction::GetIter);

            self.emit(Instruction::SetupLoop { end: end_label });
            self.set_label(start_label);
            self.emit(Instruction::ForIter { target: else_label });

            // Start of loop iteration, set targets:
            self.compile_store(target)?;

            let was_in_loop = self.ctx.in_loop;
            self.ctx.in_loop = true;
            self.compile_statements(body)?;
            self.ctx.in_loop = was_in_loop;
        }

        self.emit(Instruction::Jump {
            target: start_label,
        });
        self.set_label(else_label);
        self.emit(Instruction::PopBlock);
        if let Some(orelse) = orelse {
            self.compile_statements(orelse)?;
        }
        self.set_label(end_label);
        if is_async {
            self.emit(Instruction::Pop);
        }
        Ok(())
    }

    fn compile_chained_comparison(
        &mut self,
        vals: &[ast::Expression],
        ops: &[ast::Comparison],
    ) -> CompileResult<()> {
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

        if vals.len() > 2 {
            self.emit(Instruction::Jump { target: last_label });

            // early exit left us with stack: `rhs, comparison_result`. We need to clean up rhs.
            self.set_label(break_label);
            self.emit(Instruction::Rotate { amount: 2 });
            self.emit(Instruction::Pop);

            self.set_label(last_label);
        }

        Ok(())
    }

    fn compile_annotated_assign(
        &mut self,
        target: &ast::Expression,
        annotation: &ast::Expression,
        value: &Option<ast::Expression>,
    ) -> CompileResult<()> {
        if let Some(value) = value {
            self.compile_expression(value)?;
            self.compile_store(target)?;
        }

        // Annotations are only evaluated in a module or class.
        if self.ctx.in_func() {
            return Ok(());
        }

        // Compile annotation:
        self.compile_expression(annotation)?;

        if let ast::ExpressionType::Identifier { name } = &target.node {
            // Store as dict entry in __annotations__ dict:
            let annotations = self.name("__annotations__");
            self.emit(Instruction::LoadNameAny(annotations));
            self.emit_constant(ConstantData::Str {
                value: name.to_owned(),
            });
            self.emit(Instruction::StoreSubscript);
        } else {
            // Drop annotation if not assigned to simple identifier.
            self.emit(Instruction::Pop);
        }

        Ok(())
    }

    fn compile_store(&mut self, target: &ast::Expression) -> CompileResult<()> {
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
                let idx = self.name(name);
                self.emit(Instruction::StoreAttr { idx });
            }
            ast::ExpressionType::List { elements } | ast::ExpressionType::Tuple { elements } => {
                let mut seen_star = false;

                // Scan for star args:
                for (i, element) in elements.iter().enumerate() {
                    if let ast::ExpressionType::Starred { .. } = &element.node {
                        if seen_star {
                            return Err(self.error(CompileErrorType::MultipleStarArgs));
                        } else {
                            seen_star = true;
                            let before = i;
                            let after = elements.len() - i - 1;
                            let (before, after) = (|| Some((before.to_u8()?, after.to_u8()?)))()
                                .ok_or_else(|| {
                                    self.error_loc(
                                        CompileErrorType::TooManyStarUnpack,
                                        target.location,
                                    )
                                })?;
                            self.emit(Instruction::UnpackEx { before, after });
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
                return Err(self.error(match target.node {
                    ast::ExpressionType::Starred { .. } => CompileErrorType::SyntaxError(
                        "starred assignment target must be in a list or tuple".to_owned(),
                    ),
                    _ => CompileErrorType::Assign(target.name()),
                }))
            }
        }

        Ok(())
    }

    fn compile_op(&mut self, op: &ast::Operator, inplace: bool) {
        let op = match op {
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
        let ins = if inplace {
            Instruction::BinaryOperationInplace { op }
        } else {
            Instruction::BinaryOperation { op }
        };
        self.emit(ins);
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
    ) -> CompileResult<()> {
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
    ) -> CompileResult<()> {
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

    fn compile_dict(
        &mut self,
        pairs: &[(Option<ast::Expression>, ast::Expression)],
    ) -> CompileResult<()> {
        let mut size = 0;
        let mut has_unpacking = false;
        for (is_unpacking, subpairs) in &pairs.iter().group_by(|e| e.0.is_none()) {
            if is_unpacking {
                for (_, value) in subpairs {
                    self.compile_expression(value)?;
                    size += 1;
                }
                has_unpacking = true;
            } else {
                let mut subsize = 0;
                for (key, value) in subpairs {
                    if let Some(key) = key {
                        self.compile_expression(key)?;
                        self.compile_expression(value)?;
                        subsize += 1;
                    }
                }
                self.emit(Instruction::BuildMap {
                    size: subsize,
                    unpack: false,
                    for_call: false,
                });
                size += 1;
            }
        }
        if size == 0 {
            self.emit(Instruction::BuildMap {
                size,
                unpack: false,
                for_call: false,
            });
        }
        if size > 1 || has_unpacking {
            self.emit(Instruction::BuildMap {
                size,
                unpack: true,
                for_call: false,
            });
        }
        Ok(())
    }

    fn compile_expression(&mut self, expression: &ast::Expression) -> CompileResult<()> {
        trace!("Compiling {:?}", expression);
        self.set_source_location(expression.location);

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
                let idx = self.name(name);
                self.emit(Instruction::LoadAttr { idx });
            }
            Compare { vals, ops } => {
                self.compile_chained_comparison(vals, ops)?;
            }
            Number { value } => {
                let const_value = match value {
                    ast::Number::Integer { value } => ConstantData::Integer {
                        value: value.clone(),
                    },
                    ast::Number::Float { value } => ConstantData::Float { value: *value },
                    ast::Number::Complex { real, imag } => ConstantData::Complex {
                        value: Complex64::new(*real, *imag),
                    },
                };
                self.emit_constant(const_value);
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
                self.compile_dict(elements)?;
            }
            Slice { elements } => {
                let size = elements.len();
                for element in elements {
                    self.compile_expression(element)?;
                }
                self.emit(Instruction::BuildSlice { size });
            }
            Yield { value } => {
                if !self.ctx.in_func() {
                    return Err(self.error(CompileErrorType::InvalidYield));
                }
                self.mark_generator();
                match value {
                    Some(expression) => self.compile_expression(expression)?,
                    Option::None => self.emit_constant(ConstantData::None),
                };
                self.emit(Instruction::YieldValue);
            }
            Await { value } => {
                if self.ctx.func != FunctionContext::AsyncFunction {
                    return Err(self.error(CompileErrorType::InvalidAwait));
                }
                self.compile_expression(value)?;
                self.emit(Instruction::GetAwaitable);
                self.emit_constant(ConstantData::None);
                self.emit(Instruction::YieldFrom);
            }
            YieldFrom { value } => {
                match self.ctx.func {
                    FunctionContext::NoFunction => {
                        return Err(self.error(CompileErrorType::InvalidYieldFrom))
                    }
                    FunctionContext::AsyncFunction => {
                        return Err(self.error(CompileErrorType::AsyncYieldFrom))
                    }
                    FunctionContext::Function => {}
                }
                self.mark_generator();
                self.compile_expression(value)?;
                self.emit(Instruction::GetIter);
                self.emit_constant(ConstantData::None);
                self.emit(Instruction::YieldFrom);
            }
            True => {
                self.emit_constant(ConstantData::Boolean { value: true });
            }
            False => {
                self.emit_constant(ConstantData::Boolean { value: false });
            }
            ast::ExpressionType::None => {
                self.emit_constant(ConstantData::None);
            }
            Ellipsis => {
                self.emit_constant(ConstantData::Ellipsis);
            }
            ast::ExpressionType::String { value } => {
                self.compile_string(value)?;
            }
            Bytes { value } => {
                self.emit_constant(ConstantData::Bytes {
                    value: value.clone(),
                });
            }
            Identifier { name } => {
                self.load_name(name);
            }
            Lambda { args, body } => {
                let prev_ctx = self.ctx;
                self.ctx = CompileContext {
                    in_loop: false,
                    in_class: prev_ctx.in_class,
                    func: FunctionContext::Function,
                };

                let name = "<lambda>".to_owned();
                self.enter_function(&name, args)?;
                self.compile_expression(body)?;
                self.emit(Instruction::ReturnValue);
                let code = self.pop_code_object();
                self.build_closure(&code);
                self.emit_constant(ConstantData::Code {
                    code: Box::new(code),
                });
                self.emit_constant(ConstantData::Str { value: name });
                // Turn code object into function object:
                self.emit(Instruction::MakeFunction);

                self.ctx = prev_ctx;
            }
            Comprehension { kind, generators } => {
                self.compile_comprehension(kind, generators)?;
            }
            Starred { .. } => {
                return Err(self.error(CompileErrorType::InvalidStarExpr));
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

            NamedExpression { left, right } => {
                self.compile_expression(right)?;
                self.emit(Instruction::Duplicate);
                self.compile_store(left)?;
            }
        }
        Ok(())
    }

    fn compile_keywords(&mut self, keywords: &[ast::Keyword]) -> CompileResult<()> {
        let mut size = 0;
        for (is_unpacking, subkeywords) in &keywords.iter().group_by(|e| e.name.is_none()) {
            if is_unpacking {
                for keyword in subkeywords {
                    self.compile_expression(&keyword.value)?;
                    size += 1;
                }
            } else {
                let mut subsize = 0;
                for keyword in subkeywords {
                    if let Some(name) = &keyword.name {
                        self.emit_constant(ConstantData::Str {
                            value: name.to_owned(),
                        });
                        self.compile_expression(&keyword.value)?;
                        subsize += 1;
                    }
                }
                self.emit(Instruction::BuildMap {
                    size: subsize,
                    unpack: false,
                    for_call: false,
                });
                size += 1;
            }
        }
        if size > 1 {
            self.emit(Instruction::BuildMap {
                size,
                unpack: true,
                for_call: true,
            });
        }
        Ok(())
    }

    fn compile_call(
        &mut self,
        function: &ast::Expression,
        args: &[ast::Expression],
        keywords: &[ast::Keyword],
    ) -> CompileResult<()> {
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
                self.compile_keywords(keywords)?;
                self.emit(Instruction::CallFunctionEx { has_kwargs: true });
            } else {
                self.emit(Instruction::CallFunctionEx { has_kwargs: false });
            }
        } else {
            // Keyword arguments:
            if !keywords.is_empty() {
                let mut kwarg_names = vec![];
                for keyword in keywords {
                    if let Some(name) = &keyword.name {
                        kwarg_names.push(ConstantData::Str {
                            value: name.to_owned(),
                        });
                    } else {
                        // This means **kwargs!
                        panic!("name must be set");
                    }
                    self.compile_expression(&keyword.value)?;
                }

                self.emit_constant(ConstantData::Tuple {
                    elements: kwarg_names,
                });
                self.emit(Instruction::CallFunctionKeyword { nargs: count });
            } else {
                self.emit(Instruction::CallFunctionPositional { nargs: count });
            }
        }
        Ok(())
    }

    // Given a vector of expr / star expr generate code which gives either
    // a list of expressions on the stack, or a list of tuples.
    fn gather_elements(&mut self, elements: &[ast::Expression]) -> CompileResult<bool> {
        // First determine if we have starred elements:
        let has_stars = elements
            .iter()
            .any(|e| matches!(e.node, ast::ExpressionType::Starred { .. }));

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
    ) -> CompileResult<()> {
        let prev_ctx = self.ctx;

        self.ctx = CompileContext {
            in_loop: false,
            in_class: prev_ctx.in_class,
            func: FunctionContext::Function,
        };

        // We must have at least one generator:
        assert!(!generators.is_empty());

        let name = match kind {
            ast::ComprehensionKind::GeneratorExpression { .. } => "<genexpr>",
            ast::ComprehensionKind::List { .. } => "<listcomp>",
            ast::ComprehensionKind::Set { .. } => "<setcomp>",
            ast::ComprehensionKind::Dict { .. } => "<dictcomp>",
        }
        .to_owned();

        let line_number = self.get_source_line_number();
        // Create magnificent function <listcomp>:
        self.push_output(CodeObject::new(
            bytecode::CodeFlags::NEW_LOCALS | bytecode::CodeFlags::IS_OPTIMIZED,
            1,
            1,
            0,
            self.source_path.clone(),
            line_number,
            name.clone(),
        ));
        let arg0 = self.varname(".0");

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
                    for_call: false,
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
                self.emit(Instruction::LoadFast(arg0));
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
            self.emit(Instruction::SetupLoop { end: end_label });
            self.set_label(start_label);
            self.emit(Instruction::ForIter { target: end_label });

            self.compile_store(&generator.target)?;

            // Now evaluate the ifs:
            for if_condition in &generator.ifs {
                self.compile_jump_if(if_condition, false, start_label)?
            }
        }

        let mut compile_element = |element| {
            self.compile_expression(element).map_err(|e| {
                if let CompileErrorType::InvalidStarExpr = e.error {
                    self.error(CompileErrorType::SyntaxError(
                        "iterable unpacking cannot be used in comprehension".to_owned(),
                    ))
                } else {
                    e
                }
            })
        };

        match kind {
            ast::ComprehensionKind::GeneratorExpression { element } => {
                compile_element(element)?;
                self.mark_generator();
                self.emit(Instruction::YieldValue);
                self.emit(Instruction::Pop);
            }
            ast::ComprehensionKind::List { element } => {
                compile_element(element)?;
                self.emit(Instruction::ListAppend {
                    i: 1 + generators.len(),
                });
            }
            ast::ComprehensionKind::Set { element } => {
                compile_element(element)?;
                self.emit(Instruction::SetAdd {
                    i: 1 + generators.len(),
                });
            }
            ast::ComprehensionKind::Dict { key, value } => {
                // changed evaluation order for Py38 named expression PEP 572
                self.compile_expression(key)?;
                self.compile_expression(value)?;

                self.emit(Instruction::MapAddRev {
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

        self.ctx = prev_ctx;

        self.build_closure(&code);

        // List comprehension code:
        self.emit_constant(ConstantData::Code {
            code: Box::new(code),
        });

        // List comprehension function name:
        self.emit_constant(ConstantData::Str { value: name });

        // Turn code object into function object:
        self.emit(Instruction::MakeFunction);

        // Evaluate iterated item:
        self.compile_expression(&generators[0].iter)?;

        // Get iterator / turn item into an iterator
        self.emit(Instruction::GetIter);

        // Call just created <listcomp> function:
        self.emit(Instruction::CallFunctionPositional { nargs: 1 });
        Ok(())
    }

    fn compile_string(&mut self, string: &ast::StringGroup) -> CompileResult<()> {
        if let Some(value) = try_get_constant_string(string) {
            self.emit_constant(ConstantData::Str { value });
        } else {
            match string {
                ast::StringGroup::Joined { values } => {
                    for value in values {
                        self.compile_string(value)?;
                    }
                    self.emit(Instruction::BuildString { size: values.len() })
                }
                ast::StringGroup::Constant { value } => {
                    self.emit_constant(ConstantData::Str {
                        value: value.to_owned(),
                    });
                }
                ast::StringGroup::FormattedValue {
                    value,
                    conversion,
                    spec,
                } => {
                    match spec {
                        Some(spec) => self.compile_string(spec)?,
                        None => self.emit_constant(ConstantData::Str {
                            value: String::new(),
                        }),
                    };
                    self.compile_expression(value)?;
                    self.emit(Instruction::FormatValue {
                        conversion: conversion.map(compile_conversion_flag),
                    });
                }
            }
        }
        Ok(())
    }

    fn compile_future_features(
        &mut self,
        features: &[ast::ImportSymbol],
    ) -> Result<(), CompileError> {
        if self.done_with_future_stmts {
            return Err(self.error(CompileErrorType::InvalidFuturePlacement));
        }
        for feature in features {
            match &*feature.symbol {
                // Python 3 features; we've already implemented them by default
                "nested_scopes" | "generators" | "division" | "absolute_import"
                | "with_statement" | "print_function" | "unicode_literals" => {}
                // "generator_stop" => {}
                // "annotations" => {}
                other => {
                    return Err(self.error(CompileErrorType::InvalidFutureFeature(other.to_owned())))
                }
            }
        }
        Ok(())
    }

    // Low level helper functions:
    fn emit(&mut self, instruction: Instruction) {
        let location = compile_location(&self.current_source_location);
        // TODO: insert source filename
        let info = self.current_codeinfo();
        info.instructions.push(instruction);
        info.locations.push(location);
    }

    fn emit_constant(&mut self, constant: ConstantData) {
        let info = self.current_codeinfo();
        let idx = info.constants.len();
        info.constants.push(constant);
        self.emit(Instruction::LoadConst { idx })
    }

    fn current_code(&mut self) -> &mut CodeObject {
        &mut self.current_codeinfo().code
    }

    fn current_codeinfo(&mut self) -> &mut CodeInfo {
        self.code_stack.last_mut().expect("no code on stack")
    }

    // Generate a new label
    fn new_label(&mut self) -> Label {
        let label_map = &mut self.current_codeinfo().label_map;
        let label = Label(label_map.len());
        label_map.push(None);
        label
    }

    // Assign current position the given label
    fn set_label(&mut self, label: Label) {
        let CodeInfo {
            instructions,
            label_map,
            ..
        } = self.current_codeinfo();
        let actual_label = Label(instructions.len());
        let prev_val = std::mem::replace(&mut label_map[label.0], Some(actual_label));
        debug_assert!(
            prev_val.map_or(true, |x| x == actual_label),
            "double-set a label"
        );
    }

    fn set_source_location(&mut self, location: ast::Location) {
        self.current_source_location = location;
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
        self.current_code().flags |= bytecode::CodeFlags::IS_GENERATOR
    }
}

fn get_doc(body: &[ast::Statement]) -> (&[ast::Statement], Option<String>) {
    if let Some((val, body_rest)) = body.split_first() {
        if let ast::StatementType::Expression { ref expression } = val.node {
            if let ast::ExpressionType::String { value } = &expression.node {
                if let Some(value) = try_get_constant_string(value) {
                    return (body_rest, Some(value));
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

fn compile_conversion_flag(conversion_flag: ast::ConversionFlag) -> bytecode::ConversionFlag {
    match conversion_flag {
        ast::ConversionFlag::Ascii => bytecode::ConversionFlag::Ascii,
        ast::ConversionFlag::Repr => bytecode::ConversionFlag::Repr,
        ast::ConversionFlag::Str => bytecode::ConversionFlag::Str,
    }
}

#[cfg(test)]
mod tests {
    use super::{CompileOpts, Compiler};
    use crate::symboltable::make_symbol_table;
    use rustpython_bytecode::bytecode::CodeObject;
    use rustpython_parser::parser;

    fn compile_exec(source: &str) -> CodeObject {
        let mut compiler: Compiler = Compiler::new(
            CompileOpts::default(),
            "source_path".to_owned(),
            "<module>".to_owned(),
        );
        let ast = parser::parse_program(source).unwrap();
        let symbol_scope = make_symbol_table(&ast).unwrap();
        compiler.compile_program(&ast, symbol_scope).unwrap();
        compiler.pop_code_object()
    }

    macro_rules! assert_dis_snapshot {
        ($value:expr) => {
            insta::assert_snapshot!(
                insta::internals::AutoName,
                $value.display_expand_codeobjects().to_string(),
                stringify!($value)
            )
        };
    }

    #[test]
    fn test_if_ors() {
        assert_dis_snapshot!(compile_exec(
            "\
if True or False or False:
    pass
"
        ));
    }

    #[test]
    fn test_if_ands() {
        assert_dis_snapshot!(compile_exec(
            "\
if True and False and False:
    pass
"
        ));
    }

    #[test]
    fn test_if_mixed() {
        assert_dis_snapshot!(compile_exec(
            "\
if (True and False) or (False and True):
    pass
"
        ));
    }
}
