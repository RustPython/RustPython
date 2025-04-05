//!
//! Take an AST and transform it into bytecode
//!
//! Inspirational code:
//!   <https://github.com/python/cpython/blob/main/Python/compile.c>
//!   <https://github.com/micropython/micropython/blob/master/py/compile.c>

#![deny(clippy::cast_possible_truncation)]

use crate::{
    IndexSet, ToPythonName,
    error::{CodegenError, CodegenErrorType, PatternUnreachableReason},
    ir::{self, BlockIdx},
    symboltable::{self, SymbolFlags, SymbolScope, SymbolTable},
    unparse::unparse_expr,
};
use itertools::Itertools;
use malachite_bigint::BigInt;
use num_complex::Complex;
use num_traits::{Num, ToPrimitive};
use ruff_python_ast::{
    Alias, Arguments, BoolOp, CmpOp, Comprehension, ConversionFlag, DebugText, Decorator, DictItem,
    ExceptHandler, ExceptHandlerExceptHandler, Expr, ExprAttribute, ExprBoolOp, ExprFString,
    ExprList, ExprName, ExprStarred, ExprSubscript, ExprTuple, ExprUnaryOp, FString,
    FStringElement, FStringElements, FStringFlags, FStringPart, Identifier, Int, Keyword,
    MatchCase, ModExpression, ModModule, Operator, Parameters, Pattern, PatternMatchAs,
    PatternMatchClass, PatternMatchOr, PatternMatchSequence, PatternMatchSingleton,
    PatternMatchStar, PatternMatchValue, Singleton, Stmt, StmtExpr, TypeParam, TypeParamParamSpec,
    TypeParamTypeVar, TypeParamTypeVarTuple, TypeParams, UnaryOp, WithItem,
};
use ruff_source_file::OneIndexed;
use ruff_text_size::{Ranged, TextRange};
use rustpython_wtf8::Wtf8Buf;
// use rustpython_ast::located::{self as located_ast, Located};
use rustpython_compiler_core::{
    Mode,
    bytecode::{
        self, Arg as OpArgMarker, BinaryOperator, CodeObject, ComparisonOperator, ConstantData,
        Instruction, OpArg, OpArgType, UnpackExArgs,
    },
};
use rustpython_compiler_source::SourceCode;
// use rustpython_parser_core::source_code::{LineNumber, SourceLocation};
use std::borrow::Cow;

type CompileResult<T> = Result<T, CodegenError>;

#[derive(PartialEq, Eq, Clone, Copy)]
enum NameUsage {
    Load,
    Store,
    Delete,
}

enum CallType {
    Positional { nargs: u32 },
    Keyword { nargs: u32 },
    Ex { has_kwargs: bool },
}

fn is_forbidden_name(name: &str) -> bool {
    // See https://docs.python.org/3/library/constants.html#built-in-constants
    const BUILTIN_CONSTANTS: &[&str] = &["__debug__"];

    BUILTIN_CONSTANTS.contains(&name)
}

/// Main structure holding the state of compilation.
struct Compiler<'src> {
    code_stack: Vec<ir::CodeInfo>,
    symbol_table_stack: Vec<SymbolTable>,
    source_code: SourceCode<'src>,
    // current_source_location: SourceLocation,
    current_source_range: TextRange,
    qualified_path: Vec<String>,
    done_with_future_stmts: DoneWithFuture,
    future_annotations: bool,
    ctx: CompileContext,
    class_name: Option<String>,
    opts: CompileOpts,
}

enum DoneWithFuture {
    No,
    DoneWithDoc,
    Yes,
}

#[derive(Debug, Clone, Default)]
pub struct CompileOpts {
    /// How optimized the bytecode output should be; any optimize > 0 does
    /// not emit assert statements
    pub optimize: u8,
}

#[derive(Debug, Clone, Copy)]
struct CompileContext {
    loop_data: Option<(ir::BlockIdx, ir::BlockIdx)>,
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

#[derive(Debug, Clone, Copy, PartialEq)]
enum ComprehensionType {
    Generator,
    List,
    Set,
    Dict,
}

/// Compile an Mod produced from ruff parser
pub fn compile_top(
    ast: ruff_python_ast::Mod,
    source_code: SourceCode<'_>,
    mode: Mode,
    opts: CompileOpts,
) -> CompileResult<CodeObject> {
    match ast {
        ruff_python_ast::Mod::Module(module) => match mode {
            Mode::Exec | Mode::Eval => compile_program(&module, source_code, opts),
            Mode::Single => compile_program_single(&module, source_code, opts),
            Mode::BlockExpr => compile_block_expression(&module, source_code, opts),
        },
        ruff_python_ast::Mod::Expression(expr) => compile_expression(&expr, source_code, opts),
    }
}

/// Compile a standard Python program to bytecode
pub fn compile_program(
    ast: &ModModule,
    source_code: SourceCode<'_>,
    opts: CompileOpts,
) -> CompileResult<CodeObject> {
    let symbol_table = SymbolTable::scan_program(ast, source_code.clone())
        .map_err(|e| e.into_codegen_error(source_code.path.to_owned()))?;
    let mut compiler = Compiler::new(opts, source_code, "<module>".to_owned());
    compiler.compile_program(ast, symbol_table)?;
    let code = compiler.pop_code_object();
    trace!("Compilation completed: {:?}", code);
    Ok(code)
}

/// Compile a Python program to bytecode for the context of a REPL
pub fn compile_program_single(
    ast: &ModModule,
    source_code: SourceCode<'_>,
    opts: CompileOpts,
) -> CompileResult<CodeObject> {
    let symbol_table = SymbolTable::scan_program(ast, source_code.clone())
        .map_err(|e| e.into_codegen_error(source_code.path.to_owned()))?;
    let mut compiler = Compiler::new(opts, source_code, "<module>".to_owned());
    compiler.compile_program_single(&ast.body, symbol_table)?;
    let code = compiler.pop_code_object();
    trace!("Compilation completed: {:?}", code);
    Ok(code)
}

pub fn compile_block_expression(
    ast: &ModModule,
    source_code: SourceCode<'_>,
    opts: CompileOpts,
) -> CompileResult<CodeObject> {
    let symbol_table = SymbolTable::scan_program(ast, source_code.clone())
        .map_err(|e| e.into_codegen_error(source_code.path.to_owned()))?;
    let mut compiler = Compiler::new(opts, source_code, "<module>".to_owned());
    compiler.compile_block_expr(&ast.body, symbol_table)?;
    let code = compiler.pop_code_object();
    trace!("Compilation completed: {:?}", code);
    Ok(code)
}

pub fn compile_expression(
    ast: &ModExpression,
    source_code: SourceCode<'_>,
    opts: CompileOpts,
) -> CompileResult<CodeObject> {
    let symbol_table = SymbolTable::scan_expr(ast, source_code.clone())
        .map_err(|e| e.into_codegen_error(source_code.path.to_owned()))?;
    let mut compiler = Compiler::new(opts, source_code, "<module>".to_owned());
    compiler.compile_eval(ast, symbol_table)?;
    let code = compiler.pop_code_object();
    Ok(code)
}

macro_rules! emit {
    ($c:expr, Instruction::$op:ident { $arg:ident$(,)? }$(,)?) => {
        $c.emit_arg($arg, |x| Instruction::$op { $arg: x })
    };
    ($c:expr, Instruction::$op:ident { $arg:ident : $arg_val:expr $(,)? }$(,)?) => {
        $c.emit_arg($arg_val, |x| Instruction::$op { $arg: x })
    };
    ($c:expr, Instruction::$op:ident( $arg_val:expr $(,)? )$(,)?) => {
        $c.emit_arg($arg_val, Instruction::$op)
    };
    ($c:expr, Instruction::$op:ident$(,)?) => {
        $c.emit_no_arg(Instruction::$op)
    };
}

/// The pattern context holds information about captured names and jump targets.
#[derive(Clone)]
pub struct PatternContext {
    /// A list of names captured by the pattern.
    pub stores: Vec<String>,
    /// If false, then any name captures against our subject will raise.
    pub allow_irrefutable: bool,
    /// A list of jump target labels used on pattern failure.
    pub fail_pop: Vec<BlockIdx>,
    /// The number of items on top of the stack that should remain.
    pub on_top: usize,
}

impl Default for PatternContext {
    fn default() -> Self {
        Self::new()
    }
}

impl PatternContext {
    pub fn new() -> Self {
        PatternContext {
            stores: Vec::new(),
            allow_irrefutable: false,
            fail_pop: Vec::new(),
            on_top: 0,
        }
    }

    pub fn fail_pop_size(&self) -> usize {
        self.fail_pop.len()
    }
}

enum JumpOp {
    Jump,
    PopJumpIfFalse,
}

impl<'src> Compiler<'src> {
    fn new(opts: CompileOpts, source_code: SourceCode<'src>, code_name: String) -> Self {
        let module_code = ir::CodeInfo {
            flags: bytecode::CodeFlags::NEW_LOCALS,
            posonlyarg_count: 0,
            arg_count: 0,
            kwonlyarg_count: 0,
            source_path: source_code.path.to_owned(),
            first_line_number: OneIndexed::MIN,
            obj_name: code_name,

            blocks: vec![ir::Block::default()],
            current_block: ir::BlockIdx(0),
            constants: IndexSet::default(),
            name_cache: IndexSet::default(),
            varname_cache: IndexSet::default(),
            cellvar_cache: IndexSet::default(),
            freevar_cache: IndexSet::default(),
        };
        Compiler {
            code_stack: vec![module_code],
            symbol_table_stack: Vec::new(),
            source_code,
            // current_source_location: SourceLocation::default(),
            current_source_range: TextRange::default(),
            qualified_path: Vec::new(),
            done_with_future_stmts: DoneWithFuture::No,
            future_annotations: false,
            ctx: CompileContext {
                loop_data: None,
                in_class: false,
                func: FunctionContext::NoFunction,
            },
            class_name: None,
            opts,
        }
    }
}

impl Compiler<'_> {
    fn error(&mut self, error: CodegenErrorType) -> CodegenError {
        self.error_ranged(error, self.current_source_range)
    }
    fn error_ranged(&mut self, error: CodegenErrorType, range: TextRange) -> CodegenError {
        let location = self.source_code.source_location(range.start());
        CodegenError {
            error,
            location: Some(location),
            source_path: self.source_code.path.to_owned(),
        }
    }

    /// Push the next symbol table on to the stack
    fn push_symbol_table(&mut self) -> &SymbolTable {
        // Look up the next table contained in the scope of the current table
        let table = self
            .symbol_table_stack
            .last_mut()
            .expect("no next symbol table")
            .sub_tables
            .remove(0);
        // Push the next table onto the stack
        let last_idx = self.symbol_table_stack.len();
        self.symbol_table_stack.push(table);
        &self.symbol_table_stack[last_idx]
    }

    /// Pop the current symbol table off the stack
    fn pop_symbol_table(&mut self) -> SymbolTable {
        self.symbol_table_stack.pop().expect("compiler bug")
    }

    fn push_output(
        &mut self,
        flags: bytecode::CodeFlags,
        posonlyarg_count: u32,
        arg_count: u32,
        kwonlyarg_count: u32,
        obj_name: String,
    ) {
        let source_path = self.source_code.path.to_owned();
        let first_line_number = self.get_source_line_number();

        let table = self.push_symbol_table();

        let cellvar_cache = table
            .symbols
            .iter()
            .filter(|(_, s)| s.scope == SymbolScope::Cell)
            .map(|(var, _)| var.clone())
            .collect();
        let freevar_cache = table
            .symbols
            .iter()
            .filter(|(_, s)| {
                s.scope == SymbolScope::Free || s.flags.contains(SymbolFlags::FREE_CLASS)
            })
            .map(|(var, _)| var.clone())
            .collect();

        let info = ir::CodeInfo {
            flags,
            posonlyarg_count,
            arg_count,
            kwonlyarg_count,
            source_path,
            first_line_number,
            obj_name,

            blocks: vec![ir::Block::default()],
            current_block: ir::BlockIdx(0),
            constants: IndexSet::default(),
            name_cache: IndexSet::default(),
            varname_cache: IndexSet::default(),
            cellvar_cache,
            freevar_cache,
        };
        self.code_stack.push(info);
    }

    fn pop_code_object(&mut self) -> CodeObject {
        let table = self.pop_symbol_table();
        assert!(table.sub_tables.is_empty());
        self.code_stack
            .pop()
            .unwrap()
            .finalize_code(self.opts.optimize)
    }

    // could take impl Into<Cow<str>>, but everything is borrowed from ast structs; we never
    // actually have a `String` to pass
    fn name(&mut self, name: &str) -> bytecode::NameIdx {
        self._name_inner(name, |i| &mut i.name_cache)
    }
    fn varname(&mut self, name: &str) -> CompileResult<bytecode::NameIdx> {
        if Compiler::is_forbidden_arg_name(name) {
            return Err(self.error(CodegenErrorType::SyntaxError(format!(
                "cannot assign to {name}",
            ))));
        }
        Ok(self._name_inner(name, |i| &mut i.varname_cache))
    }
    fn _name_inner(
        &mut self,
        name: &str,
        cache: impl FnOnce(&mut ir::CodeInfo) -> &mut IndexSet<String>,
    ) -> bytecode::NameIdx {
        let name = self.mangle(name);
        let cache = cache(self.current_code_info());
        cache
            .get_index_of(name.as_ref())
            .unwrap_or_else(|| cache.insert_full(name.into_owned()).0)
            .to_u32()
    }

    fn compile_program(
        &mut self,
        body: &ModModule,
        symbol_table: SymbolTable,
    ) -> CompileResult<()> {
        let size_before = self.code_stack.len();
        self.symbol_table_stack.push(symbol_table);

        let (doc, statements) = split_doc(&body.body, &self.opts);
        if let Some(value) = doc {
            self.emit_load_const(ConstantData::Str {
                value: value.into(),
            });
            let doc = self.name("__doc__");
            emit!(self, Instruction::StoreGlobal(doc))
        }

        if Self::find_ann(statements) {
            emit!(self, Instruction::SetupAnnotation);
        }

        self.compile_statements(statements)?;

        assert_eq!(self.code_stack.len(), size_before);

        // Emit None at end:
        self.emit_return_const(ConstantData::None);
        Ok(())
    }

    fn compile_program_single(
        &mut self,
        body: &[Stmt],
        symbol_table: SymbolTable,
    ) -> CompileResult<()> {
        self.symbol_table_stack.push(symbol_table);

        if let Some((last, body)) = body.split_last() {
            for statement in body {
                if let Stmt::Expr(StmtExpr { value, .. }) = &statement {
                    self.compile_expression(value)?;
                    emit!(self, Instruction::PrintExpr);
                } else {
                    self.compile_statement(statement)?;
                }
            }

            if let Stmt::Expr(StmtExpr { value, .. }) = &last {
                self.compile_expression(value)?;
                emit!(self, Instruction::Duplicate);
                emit!(self, Instruction::PrintExpr);
            } else {
                self.compile_statement(last)?;
                self.emit_load_const(ConstantData::None);
            }
        } else {
            self.emit_load_const(ConstantData::None);
        };

        self.emit_return_value();
        Ok(())
    }

    fn compile_block_expr(
        &mut self,
        body: &[Stmt],
        symbol_table: SymbolTable,
    ) -> CompileResult<()> {
        self.symbol_table_stack.push(symbol_table);

        self.compile_statements(body)?;

        if let Some(last_statement) = body.last() {
            match last_statement {
                Stmt::Expr(_) => {
                    self.current_block().instructions.pop(); // pop Instruction::Pop
                }
                Stmt::FunctionDef(_) | Stmt::ClassDef(_) => {
                    let store_inst = self.current_block().instructions.pop().unwrap(); // pop Instruction::Store
                    emit!(self, Instruction::Duplicate);
                    self.current_block().instructions.push(store_inst);
                }
                _ => self.emit_load_const(ConstantData::None),
            }
        }
        self.emit_return_value();

        Ok(())
    }

    // Compile statement in eval mode:
    fn compile_eval(
        &mut self,
        expression: &ModExpression,
        symbol_table: SymbolTable,
    ) -> CompileResult<()> {
        self.symbol_table_stack.push(symbol_table);
        self.compile_expression(&expression.body)?;
        self.emit_return_value();
        Ok(())
    }

    fn compile_statements(&mut self, statements: &[Stmt]) -> CompileResult<()> {
        for statement in statements {
            self.compile_statement(statement)?
        }
        Ok(())
    }

    fn load_name(&mut self, name: &str) -> CompileResult<()> {
        self.compile_name(name, NameUsage::Load)
    }

    fn store_name(&mut self, name: &str) -> CompileResult<()> {
        self.compile_name(name, NameUsage::Store)
    }

    fn mangle<'a>(&self, name: &'a str) -> Cow<'a, str> {
        symboltable::mangle_name(self.class_name.as_deref(), name)
    }

    fn check_forbidden_name(&mut self, name: &str, usage: NameUsage) -> CompileResult<()> {
        let msg = match usage {
            NameUsage::Store if is_forbidden_name(name) => "cannot assign to",
            NameUsage::Delete if is_forbidden_name(name) => "cannot delete",
            _ => return Ok(()),
        };
        Err(self.error(CodegenErrorType::SyntaxError(format!("{msg} {name}"))))
    }

    fn compile_name(&mut self, name: &str, usage: NameUsage) -> CompileResult<()> {
        let name = self.mangle(name);

        self.check_forbidden_name(&name, usage)?;

        let symbol_table = self.symbol_table_stack.last().unwrap();
        let symbol = symbol_table.lookup(name.as_ref()).unwrap_or_else(||
            unreachable!("the symbol '{name}' should be present in the symbol table, even when it is undefined in python."),
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
            } // TODO: is this right?
              // SymbolScope::Unknown => NameOpType::Global,
        };

        if NameUsage::Load == usage && name == "__debug__" {
            self.emit_load_const(ConstantData::Boolean {
                value: self.opts.optimize == 0,
            });
            return Ok(());
        }

        let mut idx = cache
            .get_index_of(name.as_ref())
            .unwrap_or_else(|| cache.insert_full(name.into_owned()).0);
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
        self.emit_arg(idx.to_u32(), op);

        Ok(())
    }

    fn compile_statement(&mut self, statement: &Stmt) -> CompileResult<()> {
        use ruff_python_ast::*;
        trace!("Compiling {:?}", statement);
        self.set_source_range(statement.range());

        match &statement {
            // we do this here because `from __future__` still executes that `from` statement at runtime,
            // we still need to compile the ImportFrom down below
            Stmt::ImportFrom(StmtImportFrom { module, names, .. })
                if module.as_ref().map(|id| id.as_str()) == Some("__future__") =>
            {
                self.compile_future_features(names)?
            }
            // ignore module-level doc comments
            Stmt::Expr(StmtExpr { value, .. })
                if matches!(&**value, Expr::StringLiteral(..))
                    && matches!(self.done_with_future_stmts, DoneWithFuture::No) =>
            {
                self.done_with_future_stmts = DoneWithFuture::DoneWithDoc
            }
            // if we find any other statement, stop accepting future statements
            _ => self.done_with_future_stmts = DoneWithFuture::Yes,
        }

        match &statement {
            Stmt::Import(StmtImport { names, .. }) => {
                // import a, b, c as d
                for name in names {
                    let name = &name;
                    self.emit_load_const(ConstantData::Integer {
                        value: num_traits::Zero::zero(),
                    });
                    self.emit_load_const(ConstantData::None);
                    let idx = self.name(&name.name);
                    emit!(self, Instruction::ImportName { idx });
                    if let Some(alias) = &name.asname {
                        for part in name.name.split('.').skip(1) {
                            let idx = self.name(part);
                            emit!(self, Instruction::LoadAttr { idx });
                        }
                        self.store_name(alias.as_str())?
                    } else {
                        self.store_name(name.name.split('.').next().unwrap())?
                    }
                }
            }
            Stmt::ImportFrom(StmtImportFrom {
                level,
                module,
                names,
                ..
            }) => {
                let import_star = names.iter().any(|n| &n.name == "*");

                let from_list = if import_star {
                    if self.ctx.in_func() {
                        return Err(self.error_ranged(
                            CodegenErrorType::FunctionImportStar,
                            statement.range(),
                        ));
                    }
                    vec![ConstantData::Str { value: "*".into() }]
                } else {
                    names
                        .iter()
                        .map(|n| ConstantData::Str {
                            value: n.name.as_str().into(),
                        })
                        .collect()
                };

                let module_idx = module.as_ref().map(|s| self.name(s.as_str()));

                // from .... import (*fromlist)
                self.emit_load_const(ConstantData::Integer {
                    value: (*level).into(),
                });
                self.emit_load_const(ConstantData::Tuple {
                    elements: from_list,
                });
                if let Some(idx) = module_idx {
                    emit!(self, Instruction::ImportName { idx });
                } else {
                    emit!(self, Instruction::ImportNameless);
                }

                if import_star {
                    // from .... import *
                    emit!(self, Instruction::ImportStar);
                } else {
                    // from mod import a, b as c

                    for name in names {
                        let name = &name;
                        let idx = self.name(name.name.as_str());
                        // import symbol from module:
                        emit!(self, Instruction::ImportFrom { idx });

                        // Store module under proper name:
                        if let Some(alias) = &name.asname {
                            self.store_name(alias.as_str())?
                        } else {
                            self.store_name(name.name.as_str())?
                        }
                    }

                    // Pop module from stack:
                    emit!(self, Instruction::Pop);
                }
            }
            Stmt::Expr(StmtExpr { value, .. }) => {
                self.compile_expression(value)?;

                // Pop result of stack, since we not use it:
                emit!(self, Instruction::Pop);
            }
            Stmt::Global(_) | Stmt::Nonlocal(_) => {
                // Handled during symbol table construction.
            }
            Stmt::If(StmtIf {
                test,
                body,
                elif_else_clauses,
                ..
            }) => {
                match elif_else_clauses.as_slice() {
                    // Only if
                    [] => {
                        let after_block = self.new_block();
                        self.compile_jump_if(test, false, after_block)?;
                        self.compile_statements(body)?;
                        self.switch_to_block(after_block);
                    }
                    // If, elif*, elif/else
                    [rest @ .., tail] => {
                        let after_block = self.new_block();
                        let mut next_block = self.new_block();

                        self.compile_jump_if(test, false, next_block)?;
                        self.compile_statements(body)?;
                        emit!(
                            self,
                            Instruction::Jump {
                                target: after_block
                            }
                        );

                        for clause in rest {
                            self.switch_to_block(next_block);
                            next_block = self.new_block();
                            if let Some(test) = &clause.test {
                                self.compile_jump_if(test, false, next_block)?;
                            } else {
                                unreachable!() // must be elif
                            }
                            self.compile_statements(&clause.body)?;
                            emit!(
                                self,
                                Instruction::Jump {
                                    target: after_block
                                }
                            );
                        }

                        self.switch_to_block(next_block);
                        if let Some(test) = &tail.test {
                            self.compile_jump_if(test, false, after_block)?;
                        }
                        self.compile_statements(&tail.body)?;
                        self.switch_to_block(after_block);
                    }
                }
            }
            Stmt::While(StmtWhile {
                test, body, orelse, ..
            }) => self.compile_while(test, body, orelse)?,
            Stmt::With(StmtWith {
                items,
                body,
                is_async,
                ..
            }) => self.compile_with(items, body, *is_async)?,
            Stmt::For(StmtFor {
                target,
                iter,
                body,
                orelse,
                is_async,
                ..
            }) => self.compile_for(target, iter, body, orelse, *is_async)?,
            Stmt::Match(StmtMatch { subject, cases, .. }) => self.compile_match(subject, cases)?,
            Stmt::Raise(StmtRaise { exc, cause, .. }) => {
                let kind = match exc {
                    Some(value) => {
                        self.compile_expression(value)?;
                        match cause {
                            Some(cause) => {
                                self.compile_expression(cause)?;
                                bytecode::RaiseKind::RaiseCause
                            }
                            None => bytecode::RaiseKind::Raise,
                        }
                    }
                    None => bytecode::RaiseKind::Reraise,
                };
                emit!(self, Instruction::Raise { kind });
            }
            Stmt::Try(StmtTry {
                body,
                handlers,
                orelse,
                finalbody,
                is_star,
                ..
            }) => {
                if *is_star {
                    self.compile_try_star_statement(body, handlers, orelse, finalbody)?
                } else {
                    self.compile_try_statement(body, handlers, orelse, finalbody)?
                }
            }
            Stmt::FunctionDef(StmtFunctionDef {
                name,
                parameters,
                body,
                decorator_list,
                returns,
                type_params,
                is_async,
                ..
            }) => self.compile_function_def(
                name.as_str(),
                parameters,
                body,
                decorator_list,
                returns.as_deref(),
                *is_async,
                type_params.as_deref(),
            )?,
            Stmt::ClassDef(StmtClassDef {
                name,
                body,
                decorator_list,
                type_params,
                arguments,
                ..
            }) => self.compile_class_def(
                name.as_str(),
                body,
                decorator_list,
                type_params.as_deref(),
                arguments.as_deref(),
            )?,
            Stmt::Assert(StmtAssert { test, msg, .. }) => {
                // if some flag, ignore all assert statements!
                if self.opts.optimize == 0 {
                    let after_block = self.new_block();
                    self.compile_jump_if(test, true, after_block)?;

                    let assertion_error = self.name("AssertionError");
                    emit!(self, Instruction::LoadGlobal(assertion_error));
                    match msg {
                        Some(e) => {
                            self.compile_expression(e)?;
                            emit!(self, Instruction::CallFunctionPositional { nargs: 1 });
                        }
                        None => {
                            emit!(self, Instruction::CallFunctionPositional { nargs: 0 });
                        }
                    }
                    emit!(
                        self,
                        Instruction::Raise {
                            kind: bytecode::RaiseKind::Raise,
                        }
                    );

                    self.switch_to_block(after_block);
                }
            }
            Stmt::Break(_) => match self.ctx.loop_data {
                Some((_, end)) => {
                    emit!(self, Instruction::Break { target: end });
                }
                None => {
                    return Err(
                        self.error_ranged(CodegenErrorType::InvalidBreak, statement.range())
                    );
                }
            },
            Stmt::Continue(_) => match self.ctx.loop_data {
                Some((start, _)) => {
                    emit!(self, Instruction::Continue { target: start });
                }
                None => {
                    return Err(
                        self.error_ranged(CodegenErrorType::InvalidContinue, statement.range())
                    );
                }
            },
            Stmt::Return(StmtReturn { value, .. }) => {
                if !self.ctx.in_func() {
                    return Err(
                        self.error_ranged(CodegenErrorType::InvalidReturn, statement.range())
                    );
                }
                match value {
                    Some(v) => {
                        if self.ctx.func == FunctionContext::AsyncFunction
                            && self
                                .current_code_info()
                                .flags
                                .contains(bytecode::CodeFlags::IS_GENERATOR)
                        {
                            return Err(self.error_ranged(
                                CodegenErrorType::AsyncReturnValue,
                                statement.range(),
                            ));
                        }
                        self.compile_expression(v)?;
                        self.emit_return_value();
                    }
                    None => {
                        self.emit_return_const(ConstantData::None);
                    }
                }
            }
            Stmt::Assign(StmtAssign { targets, value, .. }) => {
                self.compile_expression(value)?;

                for (i, target) in targets.iter().enumerate() {
                    if i + 1 != targets.len() {
                        emit!(self, Instruction::Duplicate);
                    }
                    self.compile_store(target)?;
                }
            }
            Stmt::AugAssign(StmtAugAssign {
                target, op, value, ..
            }) => self.compile_augassign(target, op, value)?,
            Stmt::AnnAssign(StmtAnnAssign {
                target,
                annotation,
                value,
                ..
            }) => self.compile_annotated_assign(target, annotation, value.as_deref())?,
            Stmt::Delete(StmtDelete { targets, .. }) => {
                for target in targets {
                    self.compile_delete(target)?;
                }
            }
            Stmt::Pass(_) => {
                // No need to emit any code here :)
            }
            Stmt::TypeAlias(StmtTypeAlias {
                name,
                type_params,
                value,
                ..
            }) => {
                // let name_string = name.to_string();
                let Some(name) = name.as_name_expr() else {
                    // FIXME: is error here?
                    return Err(self.error(CodegenErrorType::SyntaxError(
                        "type alias expect name".to_owned(),
                    )));
                };
                let name_string = name.id.to_string();
                if type_params.is_some() {
                    self.push_symbol_table();
                }
                self.compile_expression(value)?;
                if let Some(type_params) = type_params {
                    self.compile_type_params(type_params)?;
                    self.pop_symbol_table();
                }
                self.emit_load_const(ConstantData::Str {
                    value: name_string.clone().into(),
                });
                emit!(self, Instruction::TypeAlias);
                self.store_name(&name_string)?;
            }
            Stmt::IpyEscapeCommand(_) => todo!(),
        }
        Ok(())
    }

    fn compile_delete(&mut self, expression: &Expr) -> CompileResult<()> {
        use ruff_python_ast::*;
        match &expression {
            Expr::Name(ExprName { id, .. }) => self.compile_name(id.as_str(), NameUsage::Delete)?,
            Expr::Attribute(ExprAttribute { value, attr, .. }) => {
                self.check_forbidden_name(attr.as_str(), NameUsage::Delete)?;
                self.compile_expression(value)?;
                let idx = self.name(attr.as_str());
                emit!(self, Instruction::DeleteAttr { idx });
            }
            Expr::Subscript(ExprSubscript { value, slice, .. }) => {
                self.compile_expression(value)?;
                self.compile_expression(slice)?;
                emit!(self, Instruction::DeleteSubscript);
            }
            Expr::Tuple(ExprTuple { elts, .. }) | Expr::List(ExprList { elts, .. }) => {
                for element in elts {
                    self.compile_delete(element)?;
                }
            }
            Expr::BinOp(_) | Expr::UnaryOp(_) => {
                return Err(self.error(CodegenErrorType::Delete("expression")));
            }
            _ => return Err(self.error(CodegenErrorType::Delete(expression.python_name()))),
        }
        Ok(())
    }

    fn enter_function(
        &mut self,
        name: &str,
        parameters: &Parameters,
    ) -> CompileResult<bytecode::MakeFunctionFlags> {
        let defaults: Vec<_> = std::iter::empty()
            .chain(&parameters.posonlyargs)
            .chain(&parameters.args)
            .filter_map(|x| x.default.as_deref())
            .collect();
        let have_defaults = !defaults.is_empty();
        if have_defaults {
            // Construct a tuple:
            let size = defaults.len().to_u32();
            for element in &defaults {
                self.compile_expression(element)?;
            }
            emit!(self, Instruction::BuildTuple { size });
        }

        // TODO: partition_in_place
        let mut kw_without_defaults = vec![];
        let mut kw_with_defaults = vec![];
        for kwonlyarg in &parameters.kwonlyargs {
            if let Some(default) = &kwonlyarg.default {
                kw_with_defaults.push((&kwonlyarg.parameter, default));
            } else {
                kw_without_defaults.push(&kwonlyarg.parameter);
            }
        }

        // let (kw_without_defaults, kw_with_defaults) = args.split_kwonlyargs();
        if !kw_with_defaults.is_empty() {
            let default_kw_count = kw_with_defaults.len();
            for (arg, default) in kw_with_defaults.iter() {
                self.emit_load_const(ConstantData::Str {
                    value: arg.name.as_str().into(),
                });
                self.compile_expression(default)?;
            }
            emit!(
                self,
                Instruction::BuildMap {
                    size: default_kw_count.to_u32(),
                }
            );
        }

        let mut func_flags = bytecode::MakeFunctionFlags::empty();
        if have_defaults {
            func_flags |= bytecode::MakeFunctionFlags::DEFAULTS;
        }
        if !kw_with_defaults.is_empty() {
            func_flags |= bytecode::MakeFunctionFlags::KW_ONLY_DEFAULTS;
        }

        self.push_output(
            bytecode::CodeFlags::NEW_LOCALS | bytecode::CodeFlags::IS_OPTIMIZED,
            parameters.posonlyargs.len().to_u32(),
            (parameters.posonlyargs.len() + parameters.args.len()).to_u32(),
            parameters.kwonlyargs.len().to_u32(),
            name.to_owned(),
        );

        let args_iter = std::iter::empty()
            .chain(&parameters.posonlyargs)
            .chain(&parameters.args)
            .map(|arg| &arg.parameter)
            .chain(kw_without_defaults)
            .chain(kw_with_defaults.into_iter().map(|(arg, _)| arg));
        for name in args_iter {
            self.varname(name.name.as_str())?;
        }

        if let Some(name) = parameters.vararg.as_deref() {
            self.current_code_info().flags |= bytecode::CodeFlags::HAS_VARARGS;
            self.varname(name.name.as_str())?;
        }
        if let Some(name) = parameters.kwarg.as_deref() {
            self.current_code_info().flags |= bytecode::CodeFlags::HAS_VARKEYWORDS;
            self.varname(name.name.as_str())?;
        }

        Ok(func_flags)
    }

    fn prepare_decorators(&mut self, decorator_list: &[Decorator]) -> CompileResult<()> {
        for decorator in decorator_list {
            self.compile_expression(&decorator.expression)?;
        }
        Ok(())
    }

    fn apply_decorators(&mut self, decorator_list: &[Decorator]) {
        // Apply decorators:
        for _ in decorator_list {
            emit!(self, Instruction::CallFunctionPositional { nargs: 1 });
        }
    }

    /// Store each type parameter so it is accessible to the current scope, and leave a tuple of
    /// all the type parameters on the stack.
    fn compile_type_params(&mut self, type_params: &TypeParams) -> CompileResult<()> {
        for type_param in &type_params.type_params {
            match type_param {
                TypeParam::TypeVar(TypeParamTypeVar { name, bound, .. }) => {
                    if let Some(expr) = &bound {
                        self.compile_expression(expr)?;
                        self.emit_load_const(ConstantData::Str {
                            value: name.as_str().into(),
                        });
                        emit!(self, Instruction::TypeVarWithBound);
                        emit!(self, Instruction::Duplicate);
                        self.store_name(name.as_ref())?;
                    } else {
                        // self.store_name(type_name.as_str())?;
                        self.emit_load_const(ConstantData::Str {
                            value: name.as_str().into(),
                        });
                        emit!(self, Instruction::TypeVar);
                        emit!(self, Instruction::Duplicate);
                        self.store_name(name.as_ref())?;
                    }
                }
                TypeParam::ParamSpec(TypeParamParamSpec { name, .. }) => {
                    self.emit_load_const(ConstantData::Str {
                        value: name.as_str().into(),
                    });
                    emit!(self, Instruction::ParamSpec);
                    emit!(self, Instruction::Duplicate);
                    self.store_name(name.as_ref())?;
                }
                TypeParam::TypeVarTuple(TypeParamTypeVarTuple { name, .. }) => {
                    self.emit_load_const(ConstantData::Str {
                        value: name.as_str().into(),
                    });
                    emit!(self, Instruction::TypeVarTuple);
                    emit!(self, Instruction::Duplicate);
                    self.store_name(name.as_ref())?;
                }
            };
        }
        emit!(
            self,
            Instruction::BuildTuple {
                size: u32::try_from(type_params.len()).unwrap(),
            }
        );
        Ok(())
    }

    fn compile_try_statement(
        &mut self,
        body: &[Stmt],
        handlers: &[ExceptHandler],
        orelse: &[Stmt],
        finalbody: &[Stmt],
    ) -> CompileResult<()> {
        let handler_block = self.new_block();
        let finally_block = self.new_block();

        // Setup a finally block if we have a finally statement.
        if !finalbody.is_empty() {
            emit!(
                self,
                Instruction::SetupFinally {
                    handler: finally_block,
                }
            );
        }

        let else_block = self.new_block();

        // try:
        emit!(
            self,
            Instruction::SetupExcept {
                handler: handler_block,
            }
        );
        self.compile_statements(body)?;
        emit!(self, Instruction::PopBlock);
        emit!(self, Instruction::Jump { target: else_block });

        // except handlers:
        self.switch_to_block(handler_block);
        // Exception is on top of stack now
        for handler in handlers {
            let ExceptHandler::ExceptHandler(ExceptHandlerExceptHandler {
                type_, name, body, ..
            }) = &handler;
            let next_handler = self.new_block();

            // If we gave a typ,
            // check if this handler can handle the exception:
            if let Some(exc_type) = type_ {
                // Duplicate exception for test:
                emit!(self, Instruction::Duplicate);

                // Check exception type:
                self.compile_expression(exc_type)?;
                emit!(
                    self,
                    Instruction::TestOperation {
                        op: bytecode::TestOperator::ExceptionMatch,
                    }
                );

                // We cannot handle this exception type:
                emit!(
                    self,
                    Instruction::JumpIfFalse {
                        target: next_handler,
                    }
                );

                // We have a match, store in name (except x as y)
                if let Some(alias) = name {
                    self.store_name(alias.as_str())?
                } else {
                    // Drop exception from top of stack:
                    emit!(self, Instruction::Pop);
                }
            } else {
                // Catch all!
                // Drop exception from top of stack:
                emit!(self, Instruction::Pop);
            }

            // Handler code:
            self.compile_statements(body)?;
            emit!(self, Instruction::PopException);

            if !finalbody.is_empty() {
                emit!(self, Instruction::PopBlock); // pop excepthandler block
                // We enter the finally block, without exception.
                emit!(self, Instruction::EnterFinally);
            }

            emit!(
                self,
                Instruction::Jump {
                    target: finally_block,
                }
            );

            // Emit a new label for the next handler
            self.switch_to_block(next_handler);
        }

        // If code flows here, we have an unhandled exception,
        // raise the exception again!
        emit!(
            self,
            Instruction::Raise {
                kind: bytecode::RaiseKind::Reraise,
            }
        );

        // We successfully ran the try block:
        // else:
        self.switch_to_block(else_block);
        self.compile_statements(orelse)?;

        if !finalbody.is_empty() {
            emit!(self, Instruction::PopBlock); // pop finally block

            // We enter the finallyhandler block, without return / exception.
            emit!(self, Instruction::EnterFinally);
        }

        // finally:
        self.switch_to_block(finally_block);
        if !finalbody.is_empty() {
            self.compile_statements(finalbody)?;
            emit!(self, Instruction::EndFinally);
        }

        Ok(())
    }

    fn compile_try_star_statement(
        &mut self,
        _body: &[Stmt],
        _handlers: &[ExceptHandler],
        _orelse: &[Stmt],
        _finalbody: &[Stmt],
    ) -> CompileResult<()> {
        Err(self.error(CodegenErrorType::NotImplementedYet))
    }

    fn is_forbidden_arg_name(name: &str) -> bool {
        is_forbidden_name(name)
    }

    #[allow(clippy::too_many_arguments)]
    fn compile_function_def(
        &mut self,
        name: &str,
        parameters: &Parameters,
        body: &[Stmt],
        decorator_list: &[Decorator],
        returns: Option<&Expr>, // TODO: use type hint somehow..
        is_async: bool,
        type_params: Option<&TypeParams>,
    ) -> CompileResult<()> {
        self.prepare_decorators(decorator_list)?;

        // If there are type params, we need to push a special symbol table just for them
        if type_params.is_some() {
            self.push_symbol_table();
        }

        let mut func_flags = self.enter_function(name, parameters)?;
        self.current_code_info()
            .flags
            .set(bytecode::CodeFlags::IS_COROUTINE, is_async);

        // remember to restore self.ctx.in_loop to the original after the function is compiled
        let prev_ctx = self.ctx;

        self.ctx = CompileContext {
            loop_data: None,
            in_class: prev_ctx.in_class,
            func: if is_async {
                FunctionContext::AsyncFunction
            } else {
                FunctionContext::Function
            },
        };

        self.push_qualified_path(name);
        let qualified_name = self.qualified_path.join(".");
        self.push_qualified_path("<locals>");

        let (doc_str, body) = split_doc(body, &self.opts);

        self.current_code_info()
            .constants
            .insert_full(ConstantData::None);

        self.compile_statements(body)?;

        // Emit None at end:
        match body.last() {
            Some(Stmt::Return(_)) => {
                // the last instruction is a ReturnValue already, we don't need to emit it
            }
            _ => {
                self.emit_return_const(ConstantData::None);
            }
        }

        let code = self.pop_code_object();
        self.qualified_path.pop();
        self.qualified_path.pop();
        self.ctx = prev_ctx;

        // Prepare generic type parameters:
        if let Some(type_params) = type_params {
            self.compile_type_params(type_params)?;
            func_flags |= bytecode::MakeFunctionFlags::TYPE_PARAMS;
        }

        // Prepare type annotations:
        let mut num_annotations = 0;

        // Return annotation:
        if let Some(annotation) = returns {
            // key:
            self.emit_load_const(ConstantData::Str {
                value: "return".into(),
            });
            // value:
            self.compile_annotation(annotation)?;
            num_annotations += 1;
        }

        let parameters_iter = std::iter::empty()
            .chain(&parameters.posonlyargs)
            .chain(&parameters.args)
            .chain(&parameters.kwonlyargs)
            .map(|x| &x.parameter)
            .chain(parameters.vararg.as_deref())
            .chain(parameters.kwarg.as_deref());
        for param in parameters_iter {
            if let Some(annotation) = &param.annotation {
                self.emit_load_const(ConstantData::Str {
                    value: self.mangle(param.name.as_str()).into_owned().into(),
                });
                self.compile_annotation(annotation)?;
                num_annotations += 1;
            }
        }

        if num_annotations > 0 {
            func_flags |= bytecode::MakeFunctionFlags::ANNOTATIONS;
            emit!(
                self,
                Instruction::BuildMap {
                    size: num_annotations,
                }
            );
        }

        if self.build_closure(&code) {
            func_flags |= bytecode::MakeFunctionFlags::CLOSURE;
        }

        // Pop the special type params symbol table
        if type_params.is_some() {
            self.pop_symbol_table();
        }

        self.emit_load_const(ConstantData::Code {
            code: Box::new(code),
        });
        self.emit_load_const(ConstantData::Str {
            value: qualified_name.into(),
        });

        // Turn code object into function object:
        emit!(self, Instruction::MakeFunction(func_flags));

        if let Some(value) = doc_str {
            emit!(self, Instruction::Duplicate);
            self.emit_load_const(ConstantData::Str {
                value: value.into(),
            });
            emit!(self, Instruction::Rotate2);
            let doc = self.name("__doc__");
            emit!(self, Instruction::StoreAttr { idx: doc });
        }

        self.apply_decorators(decorator_list);

        self.store_name(name)
    }

    fn build_closure(&mut self, code: &CodeObject) -> bool {
        if code.freevars.is_empty() {
            return false;
        }
        for var in &*code.freevars {
            let table = self.symbol_table_stack.last().unwrap();
            let symbol = table.lookup(var).unwrap_or_else(|| {
                panic!(
                    "couldn't look up var {} in {} in {}",
                    var, code.obj_name, self.source_code.path
                )
            });
            let parent_code = self.code_stack.last().unwrap();
            let vars = match symbol.scope {
                SymbolScope::Free => &parent_code.freevar_cache,
                SymbolScope::Cell => &parent_code.cellvar_cache,
                _ if symbol.flags.contains(SymbolFlags::FREE_CLASS) => &parent_code.freevar_cache,
                x => unreachable!(
                    "var {} in a {:?} should be free or cell but it's {:?}",
                    var, table.typ, x
                ),
            };
            let mut idx = vars.get_index_of(var).unwrap();
            if let SymbolScope::Free = symbol.scope {
                idx += parent_code.cellvar_cache.len();
            }
            emit!(self, Instruction::LoadClosure(idx.to_u32()))
        }
        emit!(
            self,
            Instruction::BuildTuple {
                size: code.freevars.len().to_u32(),
            }
        );
        true
    }

    // Python/compile.c find_ann
    fn find_ann(body: &[Stmt]) -> bool {
        use ruff_python_ast::*;
        for statement in body {
            let res = match &statement {
                Stmt::AnnAssign(_) => true,
                Stmt::For(StmtFor { body, orelse, .. }) => {
                    Self::find_ann(body) || Self::find_ann(orelse)
                }
                Stmt::If(StmtIf {
                    body,
                    elif_else_clauses,
                    ..
                }) => {
                    Self::find_ann(body)
                        || elif_else_clauses.iter().any(|x| Self::find_ann(&x.body))
                }
                Stmt::While(StmtWhile { body, orelse, .. }) => {
                    Self::find_ann(body) || Self::find_ann(orelse)
                }
                Stmt::With(StmtWith { body, .. }) => Self::find_ann(body),
                Stmt::Try(StmtTry {
                    body,
                    orelse,
                    finalbody,
                    ..
                }) => Self::find_ann(body) || Self::find_ann(orelse) || Self::find_ann(finalbody),
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
        body: &[Stmt],
        decorator_list: &[Decorator],
        type_params: Option<&TypeParams>,
        arguments: Option<&Arguments>,
    ) -> CompileResult<()> {
        self.prepare_decorators(decorator_list)?;

        let prev_ctx = self.ctx;
        self.ctx = CompileContext {
            func: FunctionContext::NoFunction,
            in_class: true,
            loop_data: None,
        };

        let prev_class_name = self.class_name.replace(name.to_owned());

        // Check if the class is declared global
        let symbol_table = self.symbol_table_stack.last().unwrap();
        let symbol = symbol_table.lookup(name.as_ref()).expect(
            "The symbol must be present in the symbol table, even when it is undefined in python.",
        );
        let mut global_path_prefix = Vec::new();
        if symbol.scope == SymbolScope::GlobalExplicit {
            global_path_prefix.append(&mut self.qualified_path);
        }
        self.push_qualified_path(name);
        let qualified_name = self.qualified_path.join(".");

        // If there are type params, we need to push a special symbol table just for them
        if type_params.is_some() {
            self.push_symbol_table();
        }

        self.push_output(bytecode::CodeFlags::empty(), 0, 0, 0, name.to_owned());

        let (doc_str, body) = split_doc(body, &self.opts);

        let dunder_name = self.name("__name__");
        emit!(self, Instruction::LoadGlobal(dunder_name));
        let dunder_module = self.name("__module__");
        emit!(self, Instruction::StoreLocal(dunder_module));
        self.emit_load_const(ConstantData::Str {
            value: qualified_name.into(),
        });
        let qualname = self.name("__qualname__");
        emit!(self, Instruction::StoreLocal(qualname));
        self.load_docstring(doc_str);
        let doc = self.name("__doc__");
        emit!(self, Instruction::StoreLocal(doc));
        // setup annotations
        if Self::find_ann(body) {
            emit!(self, Instruction::SetupAnnotation);
        }
        self.compile_statements(body)?;

        let classcell_idx = self
            .code_stack
            .last_mut()
            .unwrap()
            .cellvar_cache
            .iter()
            .position(|var| *var == "__class__");

        if let Some(classcell_idx) = classcell_idx {
            emit!(self, Instruction::LoadClosure(classcell_idx.to_u32()));
            emit!(self, Instruction::Duplicate);
            let classcell = self.name("__classcell__");
            emit!(self, Instruction::StoreLocal(classcell));
        } else {
            self.emit_load_const(ConstantData::None);
        }

        self.emit_return_value();

        let code = self.pop_code_object();

        self.class_name = prev_class_name;
        self.qualified_path.pop();
        self.qualified_path.append(global_path_prefix.as_mut());
        self.ctx = prev_ctx;

        emit!(self, Instruction::LoadBuildClass);

        let mut func_flags = bytecode::MakeFunctionFlags::empty();

        // Prepare generic type parameters:
        if let Some(type_params) = type_params {
            self.compile_type_params(type_params)?;
            func_flags |= bytecode::MakeFunctionFlags::TYPE_PARAMS;
        }

        if self.build_closure(&code) {
            func_flags |= bytecode::MakeFunctionFlags::CLOSURE;
        }

        // Pop the special type params symbol table
        if type_params.is_some() {
            self.pop_symbol_table();
        }

        self.emit_load_const(ConstantData::Code {
            code: Box::new(code),
        });
        self.emit_load_const(ConstantData::Str { value: name.into() });

        // Turn code object into function object:
        emit!(self, Instruction::MakeFunction(func_flags));

        self.emit_load_const(ConstantData::Str { value: name.into() });

        // Call the __build_class__ builtin
        let call = if let Some(arguments) = arguments {
            self.compile_call_inner(2, arguments)?
        } else {
            CallType::Positional { nargs: 2 }
        };
        self.compile_normal_call(call);

        self.apply_decorators(decorator_list);

        self.store_name(name)
    }

    fn load_docstring(&mut self, doc_str: Option<String>) {
        // TODO: __doc__ must be default None and no bytecode unless it is Some
        // Duplicate top of stack (the function or class object)

        // Doc string value:
        self.emit_load_const(match doc_str {
            Some(doc) => ConstantData::Str { value: doc.into() },
            None => ConstantData::None, // set docstring None if not declared
        });
    }

    fn compile_while(&mut self, test: &Expr, body: &[Stmt], orelse: &[Stmt]) -> CompileResult<()> {
        let while_block = self.new_block();
        let else_block = self.new_block();
        let after_block = self.new_block();

        emit!(self, Instruction::SetupLoop);
        self.switch_to_block(while_block);

        self.compile_jump_if(test, false, else_block)?;

        let was_in_loop = self.ctx.loop_data.replace((while_block, after_block));
        self.compile_statements(body)?;
        self.ctx.loop_data = was_in_loop;
        emit!(
            self,
            Instruction::Jump {
                target: while_block,
            }
        );
        self.switch_to_block(else_block);
        emit!(self, Instruction::PopBlock);
        self.compile_statements(orelse)?;
        self.switch_to_block(after_block);
        Ok(())
    }

    fn compile_with(
        &mut self,
        items: &[WithItem],
        body: &[Stmt],
        is_async: bool,
    ) -> CompileResult<()> {
        let with_range = self.current_source_range;

        let Some((item, items)) = items.split_first() else {
            return Err(self.error(CodegenErrorType::EmptyWithItems));
        };

        let final_block = {
            let final_block = self.new_block();
            self.compile_expression(&item.context_expr)?;

            self.set_source_range(with_range);
            if is_async {
                emit!(self, Instruction::BeforeAsyncWith);
                emit!(self, Instruction::GetAwaitable);
                self.emit_load_const(ConstantData::None);
                emit!(self, Instruction::YieldFrom);
                emit!(self, Instruction::SetupAsyncWith { end: final_block });
            } else {
                emit!(self, Instruction::SetupWith { end: final_block });
            }

            match &item.optional_vars {
                Some(var) => {
                    self.set_source_range(var.range());
                    self.compile_store(var)?;
                }
                None => {
                    emit!(self, Instruction::Pop);
                }
            }
            final_block
        };

        if items.is_empty() {
            if body.is_empty() {
                return Err(self.error(CodegenErrorType::EmptyWithBody));
            }
            self.compile_statements(body)?;
        } else {
            self.set_source_range(with_range);
            self.compile_with(items, body, is_async)?;
        }

        // sort of "stack up" the layers of with blocks:
        // with a, b: body -> start_with(a) start_with(b) body() end_with(b) end_with(a)
        self.set_source_range(with_range);
        emit!(self, Instruction::PopBlock);

        emit!(self, Instruction::EnterFinally);

        self.switch_to_block(final_block);
        emit!(self, Instruction::WithCleanupStart);

        if is_async {
            emit!(self, Instruction::GetAwaitable);
            self.emit_load_const(ConstantData::None);
            emit!(self, Instruction::YieldFrom);
        }

        emit!(self, Instruction::WithCleanupFinish);

        Ok(())
    }

    fn compile_for(
        &mut self,
        target: &Expr,
        iter: &Expr,
        body: &[Stmt],
        orelse: &[Stmt],
        is_async: bool,
    ) -> CompileResult<()> {
        // Start loop
        let for_block = self.new_block();
        let else_block = self.new_block();
        let after_block = self.new_block();

        emit!(self, Instruction::SetupLoop);

        // The thing iterated:
        self.compile_expression(iter)?;

        if is_async {
            emit!(self, Instruction::GetAIter);

            self.switch_to_block(for_block);
            emit!(
                self,
                Instruction::SetupExcept {
                    handler: else_block,
                }
            );
            emit!(self, Instruction::GetANext);
            self.emit_load_const(ConstantData::None);
            emit!(self, Instruction::YieldFrom);
            self.compile_store(target)?;
            emit!(self, Instruction::PopBlock);
        } else {
            // Retrieve Iterator
            emit!(self, Instruction::GetIter);

            self.switch_to_block(for_block);
            emit!(self, Instruction::ForIter { target: else_block });

            // Start of loop iteration, set targets:
            self.compile_store(target)?;
        };

        let was_in_loop = self.ctx.loop_data.replace((for_block, after_block));
        self.compile_statements(body)?;
        self.ctx.loop_data = was_in_loop;
        emit!(self, Instruction::Jump { target: for_block });

        self.switch_to_block(else_block);
        if is_async {
            emit!(self, Instruction::EndAsyncFor);
        }
        emit!(self, Instruction::PopBlock);
        self.compile_statements(orelse)?;

        self.switch_to_block(after_block);

        Ok(())
    }

    fn forbidden_name(&mut self, name: &str, ctx: NameUsage) -> CompileResult<bool> {
        if ctx == NameUsage::Store && name == "__debug__" {
            return Err(self.error(CodegenErrorType::Assign("__debug__")));
            // return Ok(true);
        }
        if ctx == NameUsage::Delete && name == "__debug__" {
            return Err(self.error(CodegenErrorType::Delete("__debug__")));
            // return Ok(true);
        }
        Ok(false)
    }

    fn compile_error_forbidden_name(&mut self, name: &str) -> CodegenError {
        // TODO: make into error (fine for now since it realistically errors out earlier)
        panic!("Failing due to forbidden name {:?}", name);
    }

    /// Ensures that `pc.fail_pop` has at least `n + 1` entries.
    /// If not, new labels are generated and pushed until the required size is reached.
    fn ensure_fail_pop(&mut self, pc: &mut PatternContext, n: usize) -> CompileResult<()> {
        let required_size = n + 1;
        if required_size <= pc.fail_pop.len() {
            return Ok(());
        }
        while pc.fail_pop.len() < required_size {
            let new_block = self.new_block();
            pc.fail_pop.push(new_block);
        }
        Ok(())
    }

    fn jump_to_fail_pop(&mut self, pc: &mut PatternContext, op: JumpOp) -> CompileResult<()> {
        // Compute the total number of items to pop:
        // items on top plus the captured objects.
        let pops = pc.on_top + pc.stores.len();
        // Ensure that the fail_pop vector has at least `pops + 1` elements.
        self.ensure_fail_pop(pc, pops)?;
        // Emit a jump using the jump target stored at index `pops`.
        match op {
            JumpOp::Jump => {
                emit!(
                    self,
                    Instruction::Jump {
                        target: pc.fail_pop[pops]
                    }
                );
            }
            JumpOp::PopJumpIfFalse => {
                emit!(
                    self,
                    Instruction::JumpIfFalse {
                        target: pc.fail_pop[pops]
                    }
                );
            }
        }
        Ok(())
    }

    /// Emits the necessary POP instructions for all failure targets in the pattern context,
    /// then resets the fail_pop vector.
    fn emit_and_reset_fail_pop(&mut self, pc: &mut PatternContext) -> CompileResult<()> {
        // If the fail_pop vector is empty, nothing needs to be done.
        if pc.fail_pop.is_empty() {
            debug_assert!(pc.fail_pop.is_empty());
            return Ok(());
        }
        // Iterate over the fail_pop vector in reverse order, skipping the first label.
        for &label in pc.fail_pop.iter().skip(1).rev() {
            self.switch_to_block(label);
            // Emit the POP instruction.
            emit!(self, Instruction::Pop);
        }
        // Finally, use the first label.
        self.switch_to_block(pc.fail_pop[0]);
        pc.fail_pop.clear();
        // Free the memory used by the vector.
        pc.fail_pop.shrink_to_fit();
        Ok(())
    }

    /// Duplicate the effect of Python 3.10's ROT_* instructions using SWAPs.
    fn pattern_helper_rotate(&mut self, mut count: usize) -> CompileResult<()> {
        while count > 1 {
            // Emit a SWAP instruction with the current count.
            emit!(
                self,
                Instruction::Swap {
                    index: u32::try_from(count).unwrap()
                }
            );
            count -= 1;
        }
        Ok(())
    }

    /// Helper to store a captured name for a star pattern.
    ///
    /// If `n` is `None`, it emits a POP_TOP instruction. Otherwise, it first
    /// checks that the name is allowed and not already stored. Then it rotates
    /// the object on the stack beneath any preserved items and appends the name
    /// to the list of captured names.
    fn pattern_helper_store_name(
        &mut self,
        n: Option<&Identifier>,
        pc: &mut PatternContext,
    ) -> CompileResult<()> {
        // If no name is provided, simply pop the top of the stack.
        if n.is_none() {
            emit!(self, Instruction::Pop);
            return Ok(());
        }
        let name = n.unwrap();

        // Check if the name is forbidden for storing.
        if self.forbidden_name(name.as_str(), NameUsage::Store)? {
            return Err(self.compile_error_forbidden_name(name.as_str()));
        }

        // Ensure we don't store the same name twice.
        if pc.stores.contains(&name.to_string()) {
            return Err(self.error(CodegenErrorType::DuplicateStore(name.as_str().to_string())));
        }

        // Calculate how many items to rotate:
        // the count is the number of items to preserve on top plus the current stored names,
        // plus one for the new value.
        let rotations = pc.on_top + pc.stores.len() + 1;
        self.pattern_helper_rotate(rotations)?;

        // Append the name to the captured stores.
        pc.stores.push(name.to_string());
        Ok(())
    }

    fn pattern_unpack_helper(&mut self, elts: &[Pattern]) -> CompileResult<()> {
        let n = elts.len();
        let mut seen_star = false;
        for (i, elt) in elts.iter().enumerate() {
            if elt.is_match_star() {
                if !seen_star {
                    if i >= (1 << 8) || (n - i - 1) >= ((i32::MAX as usize) >> 8) {
                        todo!();
                        // return self.compiler_error(loc, "too many expressions in star-unpacking sequence pattern");
                    }
                    let args = UnpackExArgs {
                        before: u8::try_from(i).unwrap(),
                        after: u8::try_from(n - i - 1).unwrap(),
                    };
                    emit!(self, Instruction::UnpackEx { args });
                    seen_star = true;
                } else {
                    // TODO: Fix error msg
                    return Err(self.error(CodegenErrorType::MultipleStarArgs));
                    // return self.compiler_error(loc, "multiple starred expressions in sequence pattern");
                }
            }
        }
        if !seen_star {
            emit!(
                self,
                Instruction::UnpackSequence {
                    size: u32::try_from(n).unwrap()
                }
            );
        }
        Ok(())
    }

    fn pattern_helper_sequence_unpack(
        &mut self,
        patterns: &[Pattern],
        _star: Option<usize>,
        pc: &mut PatternContext,
    ) -> CompileResult<()> {
        // Unpack the sequence into individual subjects.
        self.pattern_unpack_helper(patterns)?;
        let size = patterns.len();
        // Increase the on_top counter for the newly unpacked subjects.
        pc.on_top += size;
        // For each unpacked subject, compile its subpattern.
        for pattern in patterns {
            // Decrement on_top for each subject as it is consumed.
            pc.on_top -= 1;
            self.compile_pattern_subpattern(pattern, pc)?;
        }
        Ok(())
    }

    fn pattern_helper_sequence_subscr(
        &mut self,
        patterns: &[Pattern],
        star: usize,
        pc: &mut PatternContext,
    ) -> CompileResult<()> {
        // Keep the subject around for extracting elements.
        pc.on_top += 1;
        for (i, pattern) in patterns.iter().enumerate() {
            // if pattern.is_wildcard() {
            // continue;
            // }
            if i == star {
                // This must be a starred wildcard.
                // assert!(pattern.is_star_wildcard());
                continue;
            }
            // Duplicate the subject.
            emit!(self, Instruction::CopyItem { index: 1_u32 });
            if i < star {
                // For indices before the star, use a nonnegative index equal to i.
                self.emit_load_const(ConstantData::Integer { value: i.into() });
            } else {
                // For indices after the star, compute a nonnegative index:
                // index = len(subject) - (size - i)
                emit!(self, Instruction::GetLen);
                self.emit_load_const(ConstantData::Integer {
                    value: (patterns.len() - 1).into(),
                });
                // Subtract to compute the correct index.
                emit!(
                    self,
                    Instruction::BinaryOperation {
                        op: BinaryOperator::Subtract
                    }
                );
            }
            // Use BINARY_OP/NB_SUBSCR to extract the element.
            emit!(self, Instruction::BinarySubscript);
            // Compile the subpattern in irrefutable mode.
            self.compile_pattern_subpattern(pattern, pc)?;
        }
        // Pop the subject off the stack.
        pc.on_top -= 1;
        emit!(self, Instruction::Pop);
        Ok(())
    }

    fn compile_pattern_subpattern(
        &mut self,
        p: &Pattern,
        pc: &mut PatternContext,
    ) -> CompileResult<()> {
        // Save the current allow_irrefutable state.
        let old_allow_irrefutable = pc.allow_irrefutable;
        // Temporarily allow irrefutable patterns.
        pc.allow_irrefutable = true;
        // Compile the pattern.
        self.compile_pattern(p, pc)?;
        // Restore the original state.
        pc.allow_irrefutable = old_allow_irrefutable;
        Ok(())
    }

    fn compile_pattern_as(
        &mut self,
        p: &PatternMatchAs,
        pc: &mut PatternContext,
    ) -> CompileResult<()> {
        // If there is no sub-pattern, then it's an irrefutable match.
        if p.pattern.is_none() {
            if !pc.allow_irrefutable {
                if let Some(_name) = p.name.as_ref() {
                    // TODO: This error message does not match cpython exactly
                    // A name capture makes subsequent patterns unreachable.
                    return Err(self.error(CodegenErrorType::UnreachablePattern(
                        PatternUnreachableReason::NameCapture,
                    )));
                } else {
                    // A wildcard makes remaining patterns unreachable.
                    return Err(self.error(CodegenErrorType::UnreachablePattern(
                        PatternUnreachableReason::Wildcard,
                    )));
                }
            }
            // If irrefutable matches are allowed, store the name (if any).
            return self.pattern_helper_store_name(p.name.as_ref(), pc);
        }

        // Otherwise, there is a sub-pattern. Duplicate the object on top of the stack.
        pc.on_top += 1;
        emit!(self, Instruction::CopyItem { index: 1_u32 });
        // Compile the sub-pattern.
        self.compile_pattern(p.pattern.as_ref().unwrap(), pc)?;
        // After success, decrement the on_top counter.
        pc.on_top -= 1;
        // Store the captured name (if any).
        self.pattern_helper_store_name(p.name.as_ref(), pc)?;
        Ok(())
    }

    fn compile_pattern_star(
        &mut self,
        p: &PatternMatchStar,
        pc: &mut PatternContext,
    ) -> CompileResult<()> {
        self.pattern_helper_store_name(p.name.as_ref(), pc)?;
        Ok(())
    }

    /// Validates that keyword attributes in a class pattern are allowed
    /// and not duplicated.
    fn validate_kwd_attrs(
        &mut self,
        attrs: &[Identifier],
        _patterns: &[Pattern],
    ) -> CompileResult<()> {
        let nattrs = attrs.len();
        for i in 0..nattrs {
            let attr = attrs[i].as_str();
            // Check if the attribute name is forbidden in a Store context.
            if self.forbidden_name(attr, NameUsage::Store)? {
                // Return an error if the name is forbidden.
                return Err(self.compile_error_forbidden_name(attr));
            }
            // Check for duplicates: compare with every subsequent attribute.
            for ident in attrs.iter().take(nattrs).skip(i + 1) {
                let other = ident.as_str();
                if attr == other {
                    todo!();
                    // return Err(self.compiler_error(
                    //     &format!("attribute name repeated in class pattern: {}", attr),
                    // ));
                }
            }
        }
        Ok(())
    }

    fn compile_pattern_class(
        &mut self,
        p: &PatternMatchClass,
        pc: &mut PatternContext,
    ) -> CompileResult<()> {
        // Extract components from the MatchClass pattern.
        let match_class = p;
        let patterns = &match_class.arguments.patterns;

        // Extract keyword attributes and patterns.
        // Capacity is pre-allocated based on the number of keyword arguments.
        let mut kwd_attrs = Vec::with_capacity(match_class.arguments.keywords.len());
        let mut kwd_patterns = Vec::with_capacity(match_class.arguments.keywords.len());
        for kwd in &match_class.arguments.keywords {
            kwd_attrs.push(kwd.attr.clone());
            kwd_patterns.push(kwd.pattern.clone());
        }

        let nargs = patterns.len();
        let nattrs = kwd_attrs.len();
        let nkwd_patterns = kwd_patterns.len();

        // Validate that keyword attribute names and patterns match in length.
        if nattrs != nkwd_patterns {
            let msg = format!(
                "kwd_attrs ({}) / kwd_patterns ({}) length mismatch in class pattern",
                nattrs, nkwd_patterns
            );
            unreachable!("{}", msg);
        }

        // Check for too many sub-patterns.
        if nargs > u32::MAX as usize || (nargs + nattrs).saturating_sub(1) > i32::MAX as usize {
            let msg = format!(
                "too many sub-patterns in class pattern {:?}",
                match_class.cls
            );
            panic!("{}", msg);
            // return self.compiler_error(&msg);
        }

        // Validate keyword attributes if any.
        if nattrs != 0 {
            self.validate_kwd_attrs(&kwd_attrs, &kwd_patterns)?;
        }

        // Compile the class expression.
        self.compile_expression(&match_class.cls)?;

        // Create a new tuple of attribute names.
        let mut attr_names = vec![];
        for name in kwd_attrs.iter() {
            // Py_NewRef(name) is emulated by cloning the name into a PyObject.
            attr_names.push(ConstantData::Str {
                value: name.as_str().to_string().into(),
            });
        }

        // Emit instructions:
        // 1. Load the new tuple of attribute names.
        self.emit_load_const(ConstantData::Tuple {
            elements: attr_names,
        });
        // 2. Emit MATCH_CLASS with nargs.
        emit!(self, Instruction::MatchClass(u32::try_from(nargs).unwrap()));
        // 3. Duplicate the top of the stack.
        emit!(self, Instruction::CopyItem { index: 1_u32 });
        // 4. Load None.
        self.emit_load_const(ConstantData::None);
        // 5. Compare with IS_OP 1.
        emit!(self, Instruction::IsOperation(true));

        // At this point the TOS is a tuple of (nargs + nattrs) attributes (or None).
        pc.on_top += 1;
        self.jump_to_fail_pop(pc, JumpOp::PopJumpIfFalse)?;

        // Unpack the tuple into (nargs + nattrs) items.
        let total = nargs + nattrs;
        emit!(
            self,
            Instruction::UnpackSequence {
                size: u32::try_from(total).unwrap()
            }
        );
        pc.on_top += total;
        pc.on_top -= 1;

        // Process each sub-pattern.
        for i in 0..total {
            // Decrement the on_top counter as each sub-pattern is processed.
            pc.on_top -= 1;
            let subpattern = if i < nargs {
                // Positional sub-pattern.
                &patterns[i]
            } else {
                // Keyword sub-pattern.
                &kwd_patterns[i - nargs]
            };
            if subpattern.is_wildcard() {
                // For wildcard patterns, simply pop the top of the stack.
                emit!(self, Instruction::Pop);
                continue;
            }
            // Compile the subpattern without irrefutability checks.
            self.compile_pattern_subpattern(subpattern, pc)?;
        }
        Ok(())
    }

    // fn compile_pattern_mapping(&mut self, p: &PatternMatchMapping, pc: &mut PatternContext) -> CompileResult<()> {
    //     // Ensure the pattern is a mapping pattern.
    //     let mapping = p; // Extract MatchMapping-specific data.
    //     let keys = &mapping.keys;
    //     let patterns = &mapping.patterns;
    //     let size = keys.len();
    //     let npatterns = patterns.len();

    //     if size != npatterns {
    //         panic!("keys ({}) / patterns ({}) length mismatch in mapping pattern", size, npatterns);
    //         // return self.compiler_error(
    //             // &format!("keys ({}) / patterns ({}) length mismatch in mapping pattern", size, npatterns)
    //         // );
    //     }

    //     // A double-star target is present if `rest` is set.
    //     let star_target = mapping.rest;

    //     // Keep the subject on top during the mapping and length checks.
    //     pc.on_top += 1;
    //     emit!(self, Instruction::MatchMapping);
    //     self.jump_to_fail_pop(pc, JumpOp::PopJumpIfFalse)?;

    //     // If the pattern is just "{}" (empty mapping) and there's no star target,
    //     // we're done—pop the subject.
    //     if size == 0 && star_target.is_none() {
    //         pc.on_top -= 1;
    //         emit!(self, Instruction::Pop);
    //         return Ok(());
    //     }

    //     // If there are any keys, perform a length check.
    //     if size != 0 {
    //         emit!(self, Instruction::GetLen);
    //         self.emit_load_const(ConstantData::Integer { value: size.into() });
    //         emit!(self, Instruction::CompareOperation { op: ComparisonOperator::GreaterOrEqual });
    //         self.jump_to_fail_pop(pc, JumpOp::PopJumpIfFalse)?;
    //     }

    //     // Check that the number of subpatterns is not absurd.
    //     if size.saturating_sub(1) > (i32::MAX as usize) {
    //         panic!("too many sub-patterns in mapping pattern");
    //         // return self.compiler_error("too many sub-patterns in mapping pattern");
    //     }

    //     // Collect all keys into a set for duplicate checking.
    //     let mut seen = HashSet::new();

    //     // For each key, validate it and check for duplicates.
    //     for (i, key) in keys.iter().enumerate() {
    //         if let Some(key_val) = key.as_literal_expr() {
    //             let in_seen = seen.contains(&key_val);
    //             if in_seen {
    //                 panic!("mapping pattern checks duplicate key: {:?}", key_val);
    //                 // return self.compiler_error(format!("mapping pattern checks duplicate key: {:?}", key_val));
    //             }
    //             seen.insert(key_val);
    //         } else if !key.is_attribute_expr() {
    //             panic!("mapping pattern keys may only match literals and attribute lookups");
    //             // return self.compiler_error("mapping pattern keys may only match literals and attribute lookups");
    //         }

    //         // Visit the key expression.
    //         self.compile_expression(key)?;
    //     }
    //     // Drop the set (its resources will be freed automatically).

    //     // Build a tuple of keys and emit MATCH_KEYS.
    //     emit!(self, Instruction::BuildTuple { size: size as u32 });
    //     emit!(self, Instruction::MatchKeys);
    //     // Now, on top of the subject there are two new tuples: one of keys and one of values.
    //     pc.on_top += 2;

    //     // Prepare for matching the values.
    //     emit!(self, Instruction::CopyItem { index: 1_u32 });
    //     self.emit_load_const(ConstantData::None);
    //     // TODO: should be is
    //     emit!(self, Instruction::IsOperation(true));
    //     self.jump_to_fail_pop(pc, JumpOp::PopJumpIfFalse)?;

    //     // Unpack the tuple of values.
    //     emit!(self, Instruction::UnpackSequence { size: size as u32 });
    //     pc.on_top += size.saturating_sub(1);

    //     // Compile each subpattern in "subpattern" mode.
    //     for pattern in patterns {
    //         pc.on_top = pc.on_top.saturating_sub(1);
    //         self.compile_pattern_subpattern(pattern, pc)?;
    //     }

    //     // Consume the tuple of keys and the subject.
    //     pc.on_top = pc.on_top.saturating_sub(2);
    //     if let Some(star_target) = star_target {
    //         // If we have a starred name, bind a dict of remaining items to it.
    //         // This sequence of instructions performs:
    //         //   rest = dict(subject)
    //         //   for key in keys: del rest[key]
    //         emit!(self, Instruction::BuildMap { size: 0 });           // Build an empty dict.
    //         emit!(self, Instruction::Swap(3));                        // Rearrange stack: [empty, keys, subject]
    //         emit!(self, Instruction::DictUpdate { size: 2 });         // Update dict with subject.
    //         emit!(self, Instruction::UnpackSequence { size: size as u32 }); // Unpack keys.
    //         let mut remaining = size;
    //         while remaining > 0 {
    //             emit!(self, Instruction::CopyItem { index: 1 + remaining as u32 }); // Duplicate subject copy.
    //             emit!(self, Instruction::Swap { index: 2_u32 });                    // Bring key to top.
    //             emit!(self, Instruction::DeleteSubscript);              // Delete key from dict.
    //             remaining -= 1;
    //         }
    //         // Bind the dict to the starred target.
    //         self.pattern_helper_store_name(Some(&star_target), pc)?;
    //     } else {
    //         // No starred target: just pop the tuple of keys and the subject.
    //         emit!(self, Instruction::Pop);
    //         emit!(self, Instruction::Pop);
    //     }
    //     Ok(())
    // }

    fn compile_pattern_or(
        &mut self,
        p: &PatternMatchOr,
        pc: &mut PatternContext,
    ) -> CompileResult<()> {
        // Ensure the pattern is a MatchOr.
        let end = self.new_block(); // Create a new jump target label.
        let size = p.patterns.len();
        assert!(size > 1, "MatchOr must have more than one alternative");

        // Save the current pattern context.
        let old_pc = pc.clone();
        // Simulate Py_INCREF on pc.stores by cloning it.
        pc.stores = pc.stores.clone();
        let mut control: Option<Vec<String>> = None; // Will hold the capture list of the first alternative.

        // Process each alternative.
        for (i, alt) in p.patterns.iter().enumerate() {
            // Create a fresh empty store for this alternative.
            pc.stores = Vec::new();
            // An irrefutable subpattern must be last (if allowed).
            pc.allow_irrefutable = (i == size - 1) && old_pc.allow_irrefutable;
            // Reset failure targets and the on_top counter.
            pc.fail_pop.clear();
            pc.on_top = 0;
            // Emit a COPY(1) instruction before compiling the alternative.
            emit!(self, Instruction::CopyItem { index: 1_u32 });
            self.compile_pattern(alt, pc)?;

            let nstores = pc.stores.len();
            if i == 0 {
                // Save the captured names from the first alternative.
                control = Some(pc.stores.clone());
            } else {
                let control_vec = control.as_ref().unwrap();
                if nstores != control_vec.len() {
                    todo!();
                    // return self.compiler_error("alternative patterns bind different names");
                } else if nstores > 0 {
                    // Check that the names occur in the same order.
                    for icontrol in (0..nstores).rev() {
                        let name = &control_vec[icontrol];
                        // Find the index of `name` in the current stores.
                        let istores = pc.stores.iter().position(|n| n == name).unwrap();
                        // .ok_or_else(|| self.compiler_error("alternative patterns bind different names"))?;
                        if icontrol != istores {
                            // The orders differ; we must reorder.
                            assert!(istores < icontrol, "expected istores < icontrol");
                            let rotations = istores + 1;
                            // Rotate pc.stores: take a slice of the first `rotations` items...
                            let rotated = pc.stores[0..rotations].to_vec();
                            // Remove those elements.
                            for _ in 0..rotations {
                                pc.stores.remove(0);
                            }
                            // Insert the rotated slice at the appropriate index.
                            let insert_pos = icontrol - istores;
                            for (j, elem) in rotated.into_iter().enumerate() {
                                pc.stores.insert(insert_pos + j, elem);
                            }
                            // Also perform the same rotation on the evaluation stack.
                            for _ in 0..(istores + 1) {
                                self.pattern_helper_rotate(icontrol + 1)?;
                            }
                        }
                    }
                }
            }
            // Emit a jump to the common end label and reset any failure jump targets.
            emit!(self, Instruction::Jump { target: end });
            self.emit_and_reset_fail_pop(pc)?;
        }

        // Restore the original pattern context.
        *pc = old_pc.clone();
        // Simulate Py_INCREF on pc.stores.
        pc.stores = pc.stores.clone();
        // In C, old_pc.fail_pop is set to NULL to avoid freeing it later.
        // In Rust, old_pc is a local clone, so we need not worry about that.

        // No alternative matched: pop the subject and fail.
        emit!(self, Instruction::Pop);
        self.jump_to_fail_pop(pc, JumpOp::Jump)?;

        // Use the label "end".
        self.switch_to_block(end);

        // Adjust the final captures.
        let nstores = control.as_ref().unwrap().len();
        let nrots = nstores + 1 + pc.on_top + pc.stores.len();
        for i in 0..nstores {
            // Rotate the capture to its proper place.
            self.pattern_helper_rotate(nrots)?;
            let name = &control.as_ref().unwrap()[i];
            // Check for duplicate binding.
            if pc.stores.iter().any(|n| n == name) {
                return Err(self.error(CodegenErrorType::DuplicateStore(name.to_string())));
            }
            pc.stores.push(name.clone());
        }

        // Old context and control will be dropped automatically.
        // Finally, pop the copy of the subject.
        emit!(self, Instruction::Pop);
        Ok(())
    }

    fn compile_pattern_sequence(
        &mut self,
        p: &PatternMatchSequence,
        pc: &mut PatternContext,
    ) -> CompileResult<()> {
        // Ensure the pattern is a MatchSequence.
        let patterns = &p.patterns; // a slice of Pattern
        let size = patterns.len();
        let mut star: Option<usize> = None;
        let mut only_wildcard = true;
        let mut star_wildcard = false;

        // Find a starred pattern, if it exists. There may be at most one.
        for (i, pattern) in patterns.iter().enumerate() {
            if pattern.is_match_star() {
                if star.is_some() {
                    // TODO: Fix error msg
                    return Err(self.error(CodegenErrorType::MultipleStarArgs));
                }
                // star wildcard check
                star_wildcard = pattern
                    .as_match_star()
                    .map(|m| m.name.is_none())
                    .unwrap_or(false);
                only_wildcard &= star_wildcard;
                star = Some(i);
                continue;
            }
            // wildcard check
            only_wildcard &= pattern
                .as_match_as()
                .map(|m| m.name.is_none())
                .unwrap_or(false);
        }

        // Keep the subject on top during the sequence and length checks.
        pc.on_top += 1;
        emit!(self, Instruction::MatchSequence);
        self.jump_to_fail_pop(pc, JumpOp::PopJumpIfFalse)?;

        if star.is_none() {
            // No star: len(subject) == size
            emit!(self, Instruction::GetLen);
            self.emit_load_const(ConstantData::Integer { value: size.into() });
            emit!(
                self,
                Instruction::CompareOperation {
                    op: ComparisonOperator::Equal
                }
            );
            self.jump_to_fail_pop(pc, JumpOp::PopJumpIfFalse)?;
        } else if size > 1 {
            // Star exists: len(subject) >= size - 1
            emit!(self, Instruction::GetLen);
            self.emit_load_const(ConstantData::Integer {
                value: (size - 1).into(),
            });
            emit!(
                self,
                Instruction::CompareOperation {
                    op: ComparisonOperator::GreaterOrEqual
                }
            );
            self.jump_to_fail_pop(pc, JumpOp::PopJumpIfFalse)?;
        }

        // Whatever comes next should consume the subject.
        pc.on_top -= 1;
        if only_wildcard {
            // Patterns like: [] / [_] / [_, _] / [*_] / [_, *_] / [_, _, *_] / etc.
            emit!(self, Instruction::Pop);
        } else if star_wildcard {
            self.pattern_helper_sequence_subscr(patterns, star.unwrap(), pc)?;
        } else {
            self.pattern_helper_sequence_unpack(patterns, star, pc)?;
        }
        Ok(())
    }

    fn compile_pattern_value(
        &mut self,
        p: &PatternMatchValue,
        pc: &mut PatternContext,
    ) -> CompileResult<()> {
        // TODO: ensure literal or attribute lookup
        self.compile_expression(&p.value)?;
        emit!(
            self,
            Instruction::CompareOperation {
                op: bytecode::ComparisonOperator::Equal
            }
        );
        // emit!(self, Instruction::ToBool);
        self.jump_to_fail_pop(pc, JumpOp::PopJumpIfFalse)?;
        Ok(())
    }

    fn compile_pattern_singleton(
        &mut self,
        p: &PatternMatchSingleton,
        pc: &mut PatternContext,
    ) -> CompileResult<()> {
        // Load the singleton constant value.
        self.emit_load_const(match p.value {
            Singleton::None => ConstantData::None,
            Singleton::False => ConstantData::Boolean { value: false },
            Singleton::True => ConstantData::Boolean { value: true },
        });
        // Compare using the "Is" operator.
        emit!(
            self,
            Instruction::CompareOperation {
                op: bytecode::ComparisonOperator::Equal
            }
        );
        // Jump to the failure label if the comparison is false.
        self.jump_to_fail_pop(pc, JumpOp::PopJumpIfFalse)?;
        Ok(())
    }

    fn compile_pattern(
        &mut self,
        pattern_type: &Pattern,
        pattern_context: &mut PatternContext,
    ) -> CompileResult<()> {
        match &pattern_type {
            Pattern::MatchValue(pattern_type) => {
                self.compile_pattern_value(pattern_type, pattern_context)
            }
            Pattern::MatchSingleton(pattern_type) => {
                self.compile_pattern_singleton(pattern_type, pattern_context)
            }
            Pattern::MatchSequence(pattern_type) => {
                self.compile_pattern_sequence(pattern_type, pattern_context)
            }
            // Pattern::MatchMapping(pattern_type) => self.compile_pattern_mapping(pattern_type, pattern_context),
            Pattern::MatchClass(pattern_type) => {
                self.compile_pattern_class(pattern_type, pattern_context)
            }
            Pattern::MatchStar(pattern_type) => {
                self.compile_pattern_star(pattern_type, pattern_context)
            }
            Pattern::MatchAs(pattern_type) => {
                self.compile_pattern_as(pattern_type, pattern_context)
            }
            Pattern::MatchOr(pattern_type) => {
                self.compile_pattern_or(pattern_type, pattern_context)
            }
            _ => {
                // The eprintln gives context as to which pattern type is not implemented.
                eprintln!("not implemented pattern type: {pattern_type:?}");
                Err(self.error(CodegenErrorType::NotImplementedYet))
            }
        }
    }

    fn compile_match_inner(
        &mut self,
        subject: &Expr,
        cases: &[MatchCase],
        pattern_context: &mut PatternContext,
    ) -> CompileResult<()> {
        self.compile_expression(subject)?;
        let end = self.new_block();

        let num_cases = cases.len();
        assert!(num_cases > 0);
        let has_default = cases.iter().last().unwrap().pattern.is_match_star() && num_cases > 1;

        let case_count = num_cases - if has_default { 1 } else { 0 };
        for (i, m) in cases.iter().enumerate().take(case_count) {
            // Only copy the subject if not on the last case
            if i != case_count - 1 {
                emit!(self, Instruction::CopyItem { index: 1_u32 });
            }

            pattern_context.stores = Vec::with_capacity(1);
            pattern_context.allow_irrefutable = m.guard.is_some() || i == case_count - 1;
            pattern_context.fail_pop.clear();
            pattern_context.on_top = 0;

            self.compile_pattern(&m.pattern, pattern_context)?;
            assert_eq!(pattern_context.on_top, 0);

            for name in &pattern_context.stores {
                self.compile_name(name, NameUsage::Store)?;
            }

            if let Some(ref _guard) = m.guard {
                self.ensure_fail_pop(pattern_context, 0)?;
                // TODO: Fix compile jump if call
                return Err(self.error(CodegenErrorType::NotImplementedYet));
                // Jump if the guard fails. We assume that patter_context.fail_pop[0] is the jump target.
                // self.compile_jump_if(&m.pattern, &guard, pattern_context.fail_pop[0])?;
            }

            if i != case_count - 1 {
                emit!(self, Instruction::Pop);
            }

            self.compile_statements(&m.body)?;
            emit!(self, Instruction::Jump { target: end });
            self.emit_and_reset_fail_pop(pattern_context)?;
        }

        if has_default {
            let m = &cases[num_cases - 1];
            if num_cases == 1 {
                emit!(self, Instruction::Pop);
            } else {
                emit!(self, Instruction::Nop);
            }
            if let Some(ref _guard) = m.guard {
                // TODO: Fix compile jump if call
                return Err(self.error(CodegenErrorType::NotImplementedYet));
            }
            self.compile_statements(&m.body)?;
        }
        self.switch_to_block(end);
        Ok(())
    }

    fn compile_match(&mut self, subject: &Expr, cases: &[MatchCase]) -> CompileResult<()> {
        let mut pattern_context = PatternContext::new();
        self.compile_match_inner(subject, cases, &mut pattern_context)?;
        Ok(())
    }

    fn compile_chained_comparison(
        &mut self,
        left: &Expr,
        ops: &[CmpOp],
        exprs: &[Expr],
    ) -> CompileResult<()> {
        assert!(!ops.is_empty());
        assert_eq!(exprs.len(), ops.len());
        let (last_op, mid_ops) = ops.split_last().unwrap();
        let (last_val, mid_exprs) = exprs.split_last().unwrap();

        use bytecode::ComparisonOperator::*;
        use bytecode::TestOperator::*;
        let compile_cmpop = |c: &mut Self, op: &CmpOp| match op {
            CmpOp::Eq => emit!(c, Instruction::CompareOperation { op: Equal }),
            CmpOp::NotEq => emit!(c, Instruction::CompareOperation { op: NotEqual }),
            CmpOp::Lt => emit!(c, Instruction::CompareOperation { op: Less }),
            CmpOp::LtE => emit!(c, Instruction::CompareOperation { op: LessOrEqual }),
            CmpOp::Gt => emit!(c, Instruction::CompareOperation { op: Greater }),
            CmpOp::GtE => {
                emit!(c, Instruction::CompareOperation { op: GreaterOrEqual })
            }
            CmpOp::In => emit!(c, Instruction::TestOperation { op: In }),
            CmpOp::NotIn => emit!(c, Instruction::TestOperation { op: NotIn }),
            CmpOp::Is => emit!(c, Instruction::TestOperation { op: Is }),
            CmpOp::IsNot => emit!(c, Instruction::TestOperation { op: IsNot }),
        };

        // a == b == c == d
        // compile into (pseudo code):
        // result = a == b
        // if result:
        //   result = b == c
        //   if result:
        //     result = c == d

        // initialize lhs outside of loop
        self.compile_expression(left)?;

        let end_blocks = if mid_exprs.is_empty() {
            None
        } else {
            let break_block = self.new_block();
            let after_block = self.new_block();
            Some((break_block, after_block))
        };

        // for all comparisons except the last (as the last one doesn't need a conditional jump)
        for (op, val) in mid_ops.iter().zip(mid_exprs) {
            self.compile_expression(val)?;
            // store rhs for the next comparison in chain
            emit!(self, Instruction::Duplicate);
            emit!(self, Instruction::Rotate3);

            compile_cmpop(self, op);

            // if comparison result is false, we break with this value; if true, try the next one.
            if let Some((break_block, _)) = end_blocks {
                emit!(
                    self,
                    Instruction::JumpIfFalseOrPop {
                        target: break_block,
                    }
                );
            }
        }

        // handle the last comparison
        self.compile_expression(last_val)?;
        compile_cmpop(self, last_op);

        if let Some((break_block, after_block)) = end_blocks {
            emit!(
                self,
                Instruction::Jump {
                    target: after_block,
                }
            );

            // early exit left us with stack: `rhs, comparison_result`. We need to clean up rhs.
            self.switch_to_block(break_block);
            emit!(self, Instruction::Rotate2);
            emit!(self, Instruction::Pop);

            self.switch_to_block(after_block);
        }

        Ok(())
    }

    fn compile_annotation(&mut self, annotation: &Expr) -> CompileResult<()> {
        if self.future_annotations {
            self.emit_load_const(ConstantData::Str {
                value: unparse_expr(annotation, &self.source_code)
                    .to_string()
                    .into(),
            });
        } else {
            self.compile_expression(annotation)?;
        }
        Ok(())
    }

    fn compile_annotated_assign(
        &mut self,
        target: &Expr,
        annotation: &Expr,
        value: Option<&Expr>,
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
        self.compile_annotation(annotation)?;

        if let Expr::Name(ExprName { id, .. }) = &target {
            // Store as dict entry in __annotations__ dict:
            let annotations = self.name("__annotations__");
            emit!(self, Instruction::LoadNameAny(annotations));
            self.emit_load_const(ConstantData::Str {
                value: self.mangle(id.as_str()).into_owned().into(),
            });
            emit!(self, Instruction::StoreSubscript);
        } else {
            // Drop annotation if not assigned to simple identifier.
            emit!(self, Instruction::Pop);
        }

        Ok(())
    }

    fn compile_store(&mut self, target: &Expr) -> CompileResult<()> {
        match &target {
            Expr::Name(ExprName { id, .. }) => self.store_name(id.as_str())?,
            Expr::Subscript(ExprSubscript { value, slice, .. }) => {
                self.compile_expression(value)?;
                self.compile_expression(slice)?;
                emit!(self, Instruction::StoreSubscript);
            }
            Expr::Attribute(ExprAttribute { value, attr, .. }) => {
                self.check_forbidden_name(attr.as_str(), NameUsage::Store)?;
                self.compile_expression(value)?;
                let idx = self.name(attr.as_str());
                emit!(self, Instruction::StoreAttr { idx });
            }
            Expr::List(ExprList { elts, .. }) | Expr::Tuple(ExprTuple { elts, .. }) => {
                let mut seen_star = false;

                // Scan for star args:
                for (i, element) in elts.iter().enumerate() {
                    if let Expr::Starred(_) = &element {
                        if seen_star {
                            return Err(self.error(CodegenErrorType::MultipleStarArgs));
                        } else {
                            seen_star = true;
                            let before = i;
                            let after = elts.len() - i - 1;
                            let (before, after) = (|| Some((before.to_u8()?, after.to_u8()?)))()
                                .ok_or_else(|| {
                                    self.error_ranged(
                                        CodegenErrorType::TooManyStarUnpack,
                                        target.range(),
                                    )
                                })?;
                            let args = bytecode::UnpackExArgs { before, after };
                            emit!(self, Instruction::UnpackEx { args });
                        }
                    }
                }

                if !seen_star {
                    emit!(
                        self,
                        Instruction::UnpackSequence {
                            size: elts.len().to_u32(),
                        }
                    );
                }

                for element in elts {
                    if let Expr::Starred(ExprStarred { value, .. }) = &element {
                        self.compile_store(value)?;
                    } else {
                        self.compile_store(element)?;
                    }
                }
            }
            _ => {
                return Err(self.error(match target {
                    Expr::Starred(_) => CodegenErrorType::SyntaxError(
                        "starred assignment target must be in a list or tuple".to_owned(),
                    ),
                    _ => CodegenErrorType::Assign(target.python_name()),
                }));
            }
        }

        Ok(())
    }

    fn compile_augassign(
        &mut self,
        target: &Expr,
        op: &Operator,
        value: &Expr,
    ) -> CompileResult<()> {
        enum AugAssignKind<'a> {
            Name { id: &'a str },
            Subscript,
            Attr { idx: bytecode::NameIdx },
        }

        let kind = match &target {
            Expr::Name(ExprName { id, .. }) => {
                let id = id.as_str();
                self.compile_name(id, NameUsage::Load)?;
                AugAssignKind::Name { id }
            }
            Expr::Subscript(ExprSubscript { value, slice, .. }) => {
                self.compile_expression(value)?;
                self.compile_expression(slice)?;
                emit!(self, Instruction::Duplicate2);
                emit!(self, Instruction::Subscript);
                AugAssignKind::Subscript
            }
            Expr::Attribute(ExprAttribute { value, attr, .. }) => {
                let attr = attr.as_str();
                self.check_forbidden_name(attr, NameUsage::Store)?;
                self.compile_expression(value)?;
                emit!(self, Instruction::Duplicate);
                let idx = self.name(attr);
                emit!(self, Instruction::LoadAttr { idx });
                AugAssignKind::Attr { idx }
            }
            _ => {
                return Err(self.error(CodegenErrorType::Assign(target.python_name())));
            }
        };

        self.compile_expression(value)?;
        self.compile_op(op, true);

        match kind {
            AugAssignKind::Name { id } => {
                // stack: RESULT
                self.compile_name(id, NameUsage::Store)?;
            }
            AugAssignKind::Subscript => {
                // stack: CONTAINER SLICE RESULT
                emit!(self, Instruction::Rotate3);
                emit!(self, Instruction::StoreSubscript);
            }
            AugAssignKind::Attr { idx } => {
                // stack: CONTAINER RESULT
                emit!(self, Instruction::Rotate2);
                emit!(self, Instruction::StoreAttr { idx });
            }
        }

        Ok(())
    }

    fn compile_op(&mut self, op: &Operator, inplace: bool) {
        let op = match op {
            Operator::Add => bytecode::BinaryOperator::Add,
            Operator::Sub => bytecode::BinaryOperator::Subtract,
            Operator::Mult => bytecode::BinaryOperator::Multiply,
            Operator::MatMult => bytecode::BinaryOperator::MatrixMultiply,
            Operator::Div => bytecode::BinaryOperator::Divide,
            Operator::FloorDiv => bytecode::BinaryOperator::FloorDivide,
            Operator::Mod => bytecode::BinaryOperator::Modulo,
            Operator::Pow => bytecode::BinaryOperator::Power,
            Operator::LShift => bytecode::BinaryOperator::Lshift,
            Operator::RShift => bytecode::BinaryOperator::Rshift,
            Operator::BitOr => bytecode::BinaryOperator::Or,
            Operator::BitXor => bytecode::BinaryOperator::Xor,
            Operator::BitAnd => bytecode::BinaryOperator::And,
        };
        if inplace {
            emit!(self, Instruction::BinaryOperationInplace { op })
        } else {
            emit!(self, Instruction::BinaryOperation { op })
        }
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
        expression: &Expr,
        condition: bool,
        target_block: ir::BlockIdx,
    ) -> CompileResult<()> {
        // Compile expression for test, and jump to label if false
        match &expression {
            Expr::BoolOp(ExprBoolOp { op, values, .. }) => {
                match op {
                    BoolOp::And => {
                        if condition {
                            // If all values are true.
                            let end_block = self.new_block();
                            let (last_value, values) = values.split_last().unwrap();

                            // If any of the values is false, we can short-circuit.
                            for value in values {
                                self.compile_jump_if(value, false, end_block)?;
                            }

                            // It depends upon the last value now: will it be true?
                            self.compile_jump_if(last_value, true, target_block)?;
                            self.switch_to_block(end_block);
                        } else {
                            // If any value is false, the whole condition is false.
                            for value in values {
                                self.compile_jump_if(value, false, target_block)?;
                            }
                        }
                    }
                    BoolOp::Or => {
                        if condition {
                            // If any of the values is true.
                            for value in values {
                                self.compile_jump_if(value, true, target_block)?;
                            }
                        } else {
                            // If all of the values are false.
                            let end_block = self.new_block();
                            let (last_value, values) = values.split_last().unwrap();

                            // If any value is true, we can short-circuit:
                            for value in values {
                                self.compile_jump_if(value, true, end_block)?;
                            }

                            // It all depends upon the last value now!
                            self.compile_jump_if(last_value, false, target_block)?;
                            self.switch_to_block(end_block);
                        }
                    }
                }
            }
            Expr::UnaryOp(ExprUnaryOp {
                op: UnaryOp::Not,
                operand,
                ..
            }) => {
                self.compile_jump_if(operand, !condition, target_block)?;
            }
            _ => {
                // Fall back case which always will work!
                self.compile_expression(expression)?;
                if condition {
                    emit!(
                        self,
                        Instruction::JumpIfTrue {
                            target: target_block,
                        }
                    );
                } else {
                    emit!(
                        self,
                        Instruction::JumpIfFalse {
                            target: target_block,
                        }
                    );
                }
            }
        }
        Ok(())
    }

    /// Compile a boolean operation as an expression.
    /// This means, that the last value remains on the stack.
    fn compile_bool_op(&mut self, op: &BoolOp, values: &[Expr]) -> CompileResult<()> {
        let after_block = self.new_block();

        let (last_value, values) = values.split_last().unwrap();
        for value in values {
            self.compile_expression(value)?;

            match op {
                BoolOp::And => {
                    emit!(
                        self,
                        Instruction::JumpIfFalseOrPop {
                            target: after_block,
                        }
                    );
                }
                BoolOp::Or => {
                    emit!(
                        self,
                        Instruction::JumpIfTrueOrPop {
                            target: after_block,
                        }
                    );
                }
            }
        }

        // If all values did not qualify, take the value of the last value:
        self.compile_expression(last_value)?;
        self.switch_to_block(after_block);
        Ok(())
    }

    fn compile_dict(&mut self, items: &[DictItem]) -> CompileResult<()> {
        // FIXME: correct order to build map, etc d = {**a, 'key': 2} should override
        // 'key' in dict a
        let mut size = 0;
        let (packed, unpacked): (Vec<_>, Vec<_>) = items.iter().partition(|x| x.key.is_some());
        for item in packed {
            self.compile_expression(item.key.as_ref().unwrap())?;
            self.compile_expression(&item.value)?;
            size += 1;
        }
        emit!(self, Instruction::BuildMap { size });

        for item in unpacked {
            self.compile_expression(&item.value)?;
            emit!(self, Instruction::DictUpdate);
        }

        Ok(())
    }

    fn compile_expression(&mut self, expression: &Expr) -> CompileResult<()> {
        use ruff_python_ast::*;
        trace!("Compiling {:?}", expression);
        let range = expression.range();
        self.set_source_range(range);

        match &expression {
            Expr::Call(ExprCall {
                func, arguments, ..
            }) => self.compile_call(func, arguments)?,
            Expr::BoolOp(ExprBoolOp { op, values, .. }) => self.compile_bool_op(op, values)?,
            Expr::BinOp(ExprBinOp {
                left, op, right, ..
            }) => {
                self.compile_expression(left)?;
                self.compile_expression(right)?;

                // Perform operation:
                self.compile_op(op, false);
            }
            Expr::Subscript(ExprSubscript { value, slice, .. }) => {
                self.compile_expression(value)?;
                self.compile_expression(slice)?;
                emit!(self, Instruction::Subscript);
            }
            Expr::UnaryOp(ExprUnaryOp { op, operand, .. }) => {
                self.compile_expression(operand)?;

                // Perform operation:
                let op = match op {
                    UnaryOp::UAdd => bytecode::UnaryOperator::Plus,
                    UnaryOp::USub => bytecode::UnaryOperator::Minus,
                    UnaryOp::Not => bytecode::UnaryOperator::Not,
                    UnaryOp::Invert => bytecode::UnaryOperator::Invert,
                };
                emit!(self, Instruction::UnaryOperation { op });
            }
            Expr::Attribute(ExprAttribute { value, attr, .. }) => {
                self.compile_expression(value)?;
                let idx = self.name(attr.as_str());
                emit!(self, Instruction::LoadAttr { idx });
            }
            Expr::Compare(ExprCompare {
                left,
                ops,
                comparators,
                ..
            }) => {
                self.compile_chained_comparison(left, ops, comparators)?;
            }
            // Expr::Constant(ExprConstant { value, .. }) => {
            //     self.emit_load_const(compile_constant(value));
            // }
            Expr::List(ExprList { elts, .. }) => {
                let (size, unpack) = self.gather_elements(0, elts)?;
                if unpack {
                    emit!(self, Instruction::BuildListFromTuples { size });
                } else {
                    emit!(self, Instruction::BuildList { size });
                }
            }
            Expr::Tuple(ExprTuple { elts, .. }) => {
                let (size, unpack) = self.gather_elements(0, elts)?;
                if unpack {
                    if size > 1 {
                        emit!(self, Instruction::BuildTupleFromTuples { size });
                    }
                } else {
                    emit!(self, Instruction::BuildTuple { size });
                }
            }
            Expr::Set(ExprSet { elts, .. }) => {
                let (size, unpack) = self.gather_elements(0, elts)?;
                if unpack {
                    emit!(self, Instruction::BuildSetFromTuples { size });
                } else {
                    emit!(self, Instruction::BuildSet { size });
                }
            }
            Expr::Dict(ExprDict { items, .. }) => {
                self.compile_dict(items)?;
            }
            Expr::Slice(ExprSlice {
                lower, upper, step, ..
            }) => {
                let mut compile_bound = |bound: Option<&Expr>| match bound {
                    Some(exp) => self.compile_expression(exp),
                    None => {
                        self.emit_load_const(ConstantData::None);
                        Ok(())
                    }
                };
                compile_bound(lower.as_deref())?;
                compile_bound(upper.as_deref())?;
                if let Some(step) = step {
                    self.compile_expression(step)?;
                }
                let step = step.is_some();
                emit!(self, Instruction::BuildSlice { step });
            }
            Expr::Yield(ExprYield { value, .. }) => {
                if !self.ctx.in_func() {
                    return Err(self.error(CodegenErrorType::InvalidYield));
                }
                self.mark_generator();
                match value {
                    Some(expression) => self.compile_expression(expression)?,
                    Option::None => self.emit_load_const(ConstantData::None),
                };
                emit!(self, Instruction::YieldValue);
            }
            Expr::Await(ExprAwait { value, .. }) => {
                if self.ctx.func != FunctionContext::AsyncFunction {
                    return Err(self.error(CodegenErrorType::InvalidAwait));
                }
                self.compile_expression(value)?;
                emit!(self, Instruction::GetAwaitable);
                self.emit_load_const(ConstantData::None);
                emit!(self, Instruction::YieldFrom);
            }
            Expr::YieldFrom(ExprYieldFrom { value, .. }) => {
                match self.ctx.func {
                    FunctionContext::NoFunction => {
                        return Err(self.error(CodegenErrorType::InvalidYieldFrom));
                    }
                    FunctionContext::AsyncFunction => {
                        return Err(self.error(CodegenErrorType::AsyncYieldFrom));
                    }
                    FunctionContext::Function => {}
                }
                self.mark_generator();
                self.compile_expression(value)?;
                emit!(self, Instruction::GetIter);
                self.emit_load_const(ConstantData::None);
                emit!(self, Instruction::YieldFrom);
            }
            Expr::Name(ExprName { id, .. }) => self.load_name(id.as_str())?,
            Expr::Lambda(ExprLambda {
                parameters, body, ..
            }) => {
                let prev_ctx = self.ctx;

                let name = "<lambda>".to_owned();
                let mut func_flags = self
                    .enter_function(&name, parameters.as_deref().unwrap_or(&Default::default()))?;

                self.ctx = CompileContext {
                    loop_data: Option::None,
                    in_class: prev_ctx.in_class,
                    func: FunctionContext::Function,
                };

                self.current_code_info()
                    .constants
                    .insert_full(ConstantData::None);

                self.compile_expression(body)?;
                self.emit_return_value();
                let code = self.pop_code_object();
                if self.build_closure(&code) {
                    func_flags |= bytecode::MakeFunctionFlags::CLOSURE;
                }
                self.emit_load_const(ConstantData::Code {
                    code: Box::new(code),
                });
                self.emit_load_const(ConstantData::Str { value: name.into() });
                // Turn code object into function object:
                emit!(self, Instruction::MakeFunction(func_flags));

                self.ctx = prev_ctx;
            }
            Expr::ListComp(ExprListComp {
                elt, generators, ..
            }) => {
                self.compile_comprehension(
                    "<listcomp>",
                    Some(Instruction::BuildList {
                        size: OpArgMarker::marker(),
                    }),
                    generators,
                    &|compiler| {
                        compiler.compile_comprehension_element(elt)?;
                        emit!(
                            compiler,
                            Instruction::ListAppend {
                                i: generators.len().to_u32(),
                            }
                        );
                        Ok(())
                    },
                    ComprehensionType::List,
                    Self::contains_await(elt),
                )?;
            }
            Expr::SetComp(ExprSetComp {
                elt, generators, ..
            }) => {
                self.compile_comprehension(
                    "<setcomp>",
                    Some(Instruction::BuildSet {
                        size: OpArgMarker::marker(),
                    }),
                    generators,
                    &|compiler| {
                        compiler.compile_comprehension_element(elt)?;
                        emit!(
                            compiler,
                            Instruction::SetAdd {
                                i: generators.len().to_u32(),
                            }
                        );
                        Ok(())
                    },
                    ComprehensionType::Set,
                    Self::contains_await(elt),
                )?;
            }
            Expr::DictComp(ExprDictComp {
                key,
                value,
                generators,
                ..
            }) => {
                self.compile_comprehension(
                    "<dictcomp>",
                    Some(Instruction::BuildMap {
                        size: OpArgMarker::marker(),
                    }),
                    generators,
                    &|compiler| {
                        // changed evaluation order for Py38 named expression PEP 572
                        compiler.compile_expression(key)?;
                        compiler.compile_expression(value)?;

                        emit!(
                            compiler,
                            Instruction::MapAdd {
                                i: generators.len().to_u32(),
                            }
                        );

                        Ok(())
                    },
                    ComprehensionType::Dict,
                    Self::contains_await(key) || Self::contains_await(value),
                )?;
            }
            Expr::Generator(ExprGenerator {
                elt, generators, ..
            }) => {
                self.compile_comprehension(
                    "<genexpr>",
                    None,
                    generators,
                    &|compiler| {
                        compiler.compile_comprehension_element(elt)?;
                        compiler.mark_generator();
                        emit!(compiler, Instruction::YieldValue);
                        emit!(compiler, Instruction::Pop);

                        Ok(())
                    },
                    ComprehensionType::Generator,
                    Self::contains_await(elt),
                )?;
            }
            Expr::Starred(_) => {
                return Err(self.error(CodegenErrorType::InvalidStarExpr));
            }
            Expr::If(ExprIf {
                test, body, orelse, ..
            }) => {
                let else_block = self.new_block();
                let after_block = self.new_block();
                self.compile_jump_if(test, false, else_block)?;

                // True case
                self.compile_expression(body)?;
                emit!(
                    self,
                    Instruction::Jump {
                        target: after_block,
                    }
                );

                // False case
                self.switch_to_block(else_block);
                self.compile_expression(orelse)?;

                // End
                self.switch_to_block(after_block);
            }

            Expr::Named(ExprNamed {
                target,
                value,
                range: _,
            }) => {
                self.compile_expression(value)?;
                emit!(self, Instruction::Duplicate);
                self.compile_store(target)?;
            }
            Expr::FString(fstring) => {
                self.compile_expr_fstring(fstring)?;
            }
            Expr::StringLiteral(string) => {
                let value = string.value.to_str();
                if value.contains(char::REPLACEMENT_CHARACTER) {
                    let value = string
                        .value
                        .iter()
                        .map(|lit| {
                            let source = self.source_code.get_range(lit.range);
                            crate::string_parser::parse_string_literal(source, lit.flags.into())
                        })
                        .collect();
                    // might have a surrogate literal; should reparse to be sure
                    self.emit_load_const(ConstantData::Str { value });
                } else {
                    self.emit_load_const(ConstantData::Str {
                        value: value.into(),
                    });
                }
            }
            Expr::BytesLiteral(bytes) => {
                let iter = bytes.value.iter().flat_map(|x| x.iter().copied());
                let v: Vec<u8> = iter.collect();
                self.emit_load_const(ConstantData::Bytes { value: v });
            }
            Expr::NumberLiteral(number) => match &number.value {
                Number::Int(int) => {
                    let value = ruff_int_to_bigint(int).map_err(|e| self.error(e))?;
                    self.emit_load_const(ConstantData::Integer { value });
                }
                Number::Float(float) => {
                    self.emit_load_const(ConstantData::Float { value: *float });
                }
                Number::Complex { real, imag } => {
                    self.emit_load_const(ConstantData::Complex {
                        value: Complex::new(*real, *imag),
                    });
                }
            },
            Expr::BooleanLiteral(b) => {
                self.emit_load_const(ConstantData::Boolean { value: b.value });
            }
            Expr::NoneLiteral(_) => {
                self.emit_load_const(ConstantData::None);
            }
            Expr::EllipsisLiteral(_) => {
                self.emit_load_const(ConstantData::Ellipsis);
            }
            Expr::IpyEscapeCommand(_) => {
                panic!("unexpected ipy escape command");
            }
        }
        Ok(())
    }

    fn compile_keywords(&mut self, keywords: &[Keyword]) -> CompileResult<()> {
        let mut size = 0;
        let groupby = keywords.iter().chunk_by(|e| e.arg.is_none());
        for (is_unpacking, sub_keywords) in &groupby {
            if is_unpacking {
                for keyword in sub_keywords {
                    self.compile_expression(&keyword.value)?;
                    size += 1;
                }
            } else {
                let mut sub_size = 0;
                for keyword in sub_keywords {
                    if let Some(name) = &keyword.arg {
                        self.emit_load_const(ConstantData::Str {
                            value: name.as_str().into(),
                        });
                        self.compile_expression(&keyword.value)?;
                        sub_size += 1;
                    }
                }
                emit!(self, Instruction::BuildMap { size: sub_size });
                size += 1;
            }
        }
        if size > 1 {
            emit!(self, Instruction::BuildMapForCall { size });
        }
        Ok(())
    }

    fn compile_call(&mut self, func: &Expr, args: &Arguments) -> CompileResult<()> {
        let method = if let Expr::Attribute(ExprAttribute { value, attr, .. }) = &func {
            self.compile_expression(value)?;
            let idx = self.name(attr.as_str());
            emit!(self, Instruction::LoadMethod { idx });
            true
        } else {
            self.compile_expression(func)?;
            false
        };
        let call = self.compile_call_inner(0, args)?;
        if method {
            self.compile_method_call(call)
        } else {
            self.compile_normal_call(call)
        }
        Ok(())
    }

    fn compile_normal_call(&mut self, ty: CallType) {
        match ty {
            CallType::Positional { nargs } => {
                emit!(self, Instruction::CallFunctionPositional { nargs })
            }
            CallType::Keyword { nargs } => emit!(self, Instruction::CallFunctionKeyword { nargs }),
            CallType::Ex { has_kwargs } => emit!(self, Instruction::CallFunctionEx { has_kwargs }),
        }
    }
    fn compile_method_call(&mut self, ty: CallType) {
        match ty {
            CallType::Positional { nargs } => {
                emit!(self, Instruction::CallMethodPositional { nargs })
            }
            CallType::Keyword { nargs } => emit!(self, Instruction::CallMethodKeyword { nargs }),
            CallType::Ex { has_kwargs } => emit!(self, Instruction::CallMethodEx { has_kwargs }),
        }
    }

    fn compile_call_inner(
        &mut self,
        additional_positional: u32,
        arguments: &Arguments,
    ) -> CompileResult<CallType> {
        let count = u32::try_from(arguments.len()).unwrap() + additional_positional;

        // Normal arguments:
        let (size, unpack) = self.gather_elements(additional_positional, &arguments.args)?;
        let has_double_star = arguments.keywords.iter().any(|k| k.arg.is_none());

        for keyword in &arguments.keywords {
            if let Some(name) = &keyword.arg {
                self.check_forbidden_name(name.as_str(), NameUsage::Store)?;
            }
        }

        let call = if unpack || has_double_star {
            // Create a tuple with positional args:
            if unpack {
                emit!(self, Instruction::BuildTupleFromTuples { size });
            } else {
                emit!(self, Instruction::BuildTuple { size });
            }

            // Create an optional map with kw-args:
            let has_kwargs = !arguments.keywords.is_empty();
            if has_kwargs {
                self.compile_keywords(&arguments.keywords)?;
            }
            CallType::Ex { has_kwargs }
        } else if !arguments.keywords.is_empty() {
            let mut kwarg_names = vec![];
            for keyword in &arguments.keywords {
                if let Some(name) = &keyword.arg {
                    kwarg_names.push(ConstantData::Str {
                        value: name.as_str().into(),
                    });
                } else {
                    // This means **kwargs!
                    panic!("name must be set");
                }
                self.compile_expression(&keyword.value)?;
            }

            self.emit_load_const(ConstantData::Tuple {
                elements: kwarg_names,
            });
            CallType::Keyword { nargs: count }
        } else {
            CallType::Positional { nargs: count }
        };

        Ok(call)
    }

    // Given a vector of expr / star expr generate code which gives either
    // a list of expressions on the stack, or a list of tuples.
    fn gather_elements(&mut self, before: u32, elements: &[Expr]) -> CompileResult<(u32, bool)> {
        // First determine if we have starred elements:
        let has_stars = elements.iter().any(|e| matches!(e, Expr::Starred(_)));

        let size = if has_stars {
            let mut size = 0;
            let mut iter = elements.iter().peekable();
            let mut run_size = before;

            loop {
                if iter.peek().is_none_or(|e| matches!(e, Expr::Starred(_))) {
                    emit!(self, Instruction::BuildTuple { size: run_size });
                    run_size = 0;
                    size += 1;
                }

                match iter.next() {
                    Some(Expr::Starred(ExprStarred { value, .. })) => {
                        self.compile_expression(value)?;
                        // We need to collect each unpacked element into a
                        // tuple, since any side-effects during the conversion
                        // should be made visible before evaluating remaining
                        // expressions.
                        emit!(self, Instruction::BuildTupleFromIter);
                        size += 1;
                    }
                    Some(element) => {
                        self.compile_expression(element)?;
                        run_size += 1;
                    }
                    None => break,
                }
            }

            size
        } else {
            for element in elements {
                self.compile_expression(element)?;
            }
            before + elements.len().to_u32()
        };

        Ok((size, has_stars))
    }

    fn compile_comprehension_element(&mut self, element: &Expr) -> CompileResult<()> {
        self.compile_expression(element).map_err(|e| {
            if let CodegenErrorType::InvalidStarExpr = e.error {
                self.error(CodegenErrorType::SyntaxError(
                    "iterable unpacking cannot be used in comprehension".to_owned(),
                ))
            } else {
                e
            }
        })
    }

    fn compile_comprehension(
        &mut self,
        name: &str,
        init_collection: Option<Instruction>,
        generators: &[Comprehension],
        compile_element: &dyn Fn(&mut Self) -> CompileResult<()>,
        comprehension_type: ComprehensionType,
        element_contains_await: bool,
    ) -> CompileResult<()> {
        let prev_ctx = self.ctx;
        let has_an_async_gen = generators.iter().any(|g| g.is_async);

        // async comprehensions are allowed in various contexts:
        // - list/set/dict comprehensions in async functions
        // - always for generator expressions
        // Note: generators have to be treated specially since their async version is a fundamentally
        // different type (aiter vs iter) instead of just an awaitable.

        // for if it actually is async, we check if any generator is async or if the element contains await

        // if the element expression contains await, but the context doesn't allow for async,
        // then we continue on here with is_async=false and will produce a syntax once the await is hit

        let is_async_list_set_dict_comprehension = comprehension_type
            != ComprehensionType::Generator
            && (has_an_async_gen || element_contains_await) // does it have to be async? (uses await or async for)
            && prev_ctx.func == FunctionContext::AsyncFunction; // is it allowed to be async? (in an async function)

        let is_async_generator_comprehension = comprehension_type == ComprehensionType::Generator
            && (has_an_async_gen || element_contains_await);

        // since one is for generators, and one for not generators, they should never both be true
        debug_assert!(!(is_async_list_set_dict_comprehension && is_async_generator_comprehension));

        let is_async = is_async_list_set_dict_comprehension || is_async_generator_comprehension;

        self.ctx = CompileContext {
            loop_data: None,
            in_class: prev_ctx.in_class,
            func: if is_async {
                FunctionContext::AsyncFunction
            } else {
                FunctionContext::Function
            },
        };

        // We must have at least one generator:
        assert!(!generators.is_empty());

        let flags = bytecode::CodeFlags::NEW_LOCALS | bytecode::CodeFlags::IS_OPTIMIZED;
        let flags = if is_async {
            flags | bytecode::CodeFlags::IS_COROUTINE
        } else {
            flags
        };

        // Create magnificent function <listcomp>:
        self.push_output(flags, 1, 1, 0, name.to_owned());
        let arg0 = self.varname(".0")?;

        let return_none = init_collection.is_none();
        // Create empty object of proper type:
        if let Some(init_collection) = init_collection {
            self._emit(init_collection, OpArg(0), ir::BlockIdx::NULL)
        }

        let mut loop_labels = vec![];
        for generator in generators {
            let loop_block = self.new_block();
            let after_block = self.new_block();

            // emit!(self, Instruction::SetupLoop);

            if loop_labels.is_empty() {
                // Load iterator onto stack (passed as first argument):
                emit!(self, Instruction::LoadFast(arg0));
            } else {
                // Evaluate iterated item:
                self.compile_expression(&generator.iter)?;

                // Get iterator / turn item into an iterator
                if generator.is_async {
                    emit!(self, Instruction::GetAIter);
                } else {
                    emit!(self, Instruction::GetIter);
                }
            }

            loop_labels.push((loop_block, after_block));
            self.switch_to_block(loop_block);
            if generator.is_async {
                emit!(
                    self,
                    Instruction::SetupExcept {
                        handler: after_block,
                    }
                );
                emit!(self, Instruction::GetANext);
                self.emit_load_const(ConstantData::None);
                emit!(self, Instruction::YieldFrom);
                self.compile_store(&generator.target)?;
                emit!(self, Instruction::PopBlock);
            } else {
                emit!(
                    self,
                    Instruction::ForIter {
                        target: after_block,
                    }
                );
                self.compile_store(&generator.target)?;
            }

            // Now evaluate the ifs:
            for if_condition in &generator.ifs {
                self.compile_jump_if(if_condition, false, loop_block)?
            }
        }

        compile_element(self)?;

        for (loop_block, after_block) in loop_labels.iter().rev().copied() {
            // Repeat:
            emit!(self, Instruction::Jump { target: loop_block });

            // End of for loop:
            self.switch_to_block(after_block);
            if has_an_async_gen {
                emit!(self, Instruction::EndAsyncFor);
            }
        }

        if return_none {
            self.emit_load_const(ConstantData::None)
        }

        // Return freshly filled list:
        self.emit_return_value();

        // Fetch code for listcomp function:
        let code = self.pop_code_object();

        self.ctx = prev_ctx;

        let mut func_flags = bytecode::MakeFunctionFlags::empty();
        if self.build_closure(&code) {
            func_flags |= bytecode::MakeFunctionFlags::CLOSURE;
        }

        // List comprehension code:
        self.emit_load_const(ConstantData::Code {
            code: Box::new(code),
        });

        // List comprehension function name:
        self.emit_load_const(ConstantData::Str { value: name.into() });

        // Turn code object into function object:
        emit!(self, Instruction::MakeFunction(func_flags));

        // Evaluate iterated item:
        self.compile_expression(&generators[0].iter)?;

        // Get iterator / turn item into an iterator
        if has_an_async_gen {
            emit!(self, Instruction::GetAIter);
        } else {
            emit!(self, Instruction::GetIter);
        };

        // Call just created <listcomp> function:
        emit!(self, Instruction::CallFunctionPositional { nargs: 1 });
        if is_async_list_set_dict_comprehension {
            // async, but not a generator and not an async for
            // in this case, we end up with an awaitable
            // that evaluates to the list/set/dict, so here we add an await
            emit!(self, Instruction::GetAwaitable);
            self.emit_load_const(ConstantData::None);
            emit!(self, Instruction::YieldFrom);
        }

        Ok(())
    }

    fn compile_future_features(&mut self, features: &[Alias]) -> Result<(), CodegenError> {
        if let DoneWithFuture::Yes = self.done_with_future_stmts {
            return Err(self.error(CodegenErrorType::InvalidFuturePlacement));
        }
        self.done_with_future_stmts = DoneWithFuture::DoneWithDoc;
        for feature in features {
            match feature.name.as_str() {
                // Python 3 features; we've already implemented them by default
                "nested_scopes" | "generators" | "division" | "absolute_import"
                | "with_statement" | "print_function" | "unicode_literals" | "generator_stop" => {}
                "annotations" => self.future_annotations = true,
                other => {
                    return Err(
                        self.error(CodegenErrorType::InvalidFutureFeature(other.to_owned()))
                    );
                }
            }
        }
        Ok(())
    }

    // Low level helper functions:
    fn _emit(&mut self, instr: Instruction, arg: OpArg, target: ir::BlockIdx) {
        let range = self.current_source_range;
        let location = self.source_code.source_location(range.start());
        // TODO: insert source filename
        self.current_block().instructions.push(ir::InstructionInfo {
            instr,
            arg,
            target,
            location,
            // range,
        });
    }

    fn emit_no_arg(&mut self, ins: Instruction) {
        self._emit(ins, OpArg::null(), ir::BlockIdx::NULL)
    }

    fn emit_arg<A: OpArgType, T: EmitArg<A>>(
        &mut self,
        arg: T,
        f: impl FnOnce(OpArgMarker<A>) -> Instruction,
    ) {
        let (op, arg, target) = arg.emit(f);
        self._emit(op, arg, target)
    }

    // fn block_done()

    fn arg_constant(&mut self, constant: ConstantData) -> u32 {
        let info = self.current_code_info();
        info.constants.insert_full(constant).0.to_u32()
    }

    fn emit_load_const(&mut self, constant: ConstantData) {
        let idx = self.arg_constant(constant);
        self.emit_arg(idx, |idx| Instruction::LoadConst { idx })
    }

    fn emit_return_const(&mut self, constant: ConstantData) {
        let idx = self.arg_constant(constant);
        self.emit_arg(idx, |idx| Instruction::ReturnConst { idx })
    }

    fn emit_return_value(&mut self) {
        if let Some(inst) = self.current_block().instructions.last_mut() {
            if let Instruction::LoadConst { idx } = inst.instr {
                inst.instr = Instruction::ReturnConst { idx };
                return;
            }
        }
        emit!(self, Instruction::ReturnValue)
    }

    fn current_code_info(&mut self) -> &mut ir::CodeInfo {
        self.code_stack.last_mut().expect("no code on stack")
    }

    fn current_block(&mut self) -> &mut ir::Block {
        let info = self.current_code_info();
        &mut info.blocks[info.current_block]
    }

    fn new_block(&mut self) -> ir::BlockIdx {
        let code = self.current_code_info();
        let idx = ir::BlockIdx(code.blocks.len().to_u32());
        code.blocks.push(ir::Block::default());
        idx
    }

    fn switch_to_block(&mut self, block: ir::BlockIdx) {
        let code = self.current_code_info();
        let prev = code.current_block;
        assert_ne!(prev, block, "recursive switching {prev:?} -> {block:?}");
        assert_eq!(
            code.blocks[block].next,
            ir::BlockIdx::NULL,
            "switching {prev:?} -> {block:?} to completed block"
        );
        let prev_block = &mut code.blocks[prev.0 as usize];
        assert_eq!(
            prev_block.next.0,
            u32::MAX,
            "switching {prev:?} -> {block:?} from block that's already got a next"
        );
        prev_block.next = block;
        code.current_block = block;
    }

    fn set_source_range(&mut self, range: TextRange) {
        self.current_source_range = range;
    }

    fn get_source_line_number(&mut self) -> OneIndexed {
        self.source_code
            .line_index(self.current_source_range.start())
    }

    fn push_qualified_path(&mut self, name: &str) {
        self.qualified_path.push(name.to_owned());
    }

    fn mark_generator(&mut self) {
        self.current_code_info().flags |= bytecode::CodeFlags::IS_GENERATOR
    }

    /// Whether the expression contains an await expression and
    /// thus requires the function to be async.
    /// Async with and async for are statements, so I won't check for them here
    fn contains_await(expression: &Expr) -> bool {
        use ruff_python_ast::*;

        match &expression {
            Expr::Call(ExprCall {
                func, arguments, ..
            }) => {
                Self::contains_await(func)
                    || arguments.args.iter().any(Self::contains_await)
                    || arguments
                        .keywords
                        .iter()
                        .any(|kw| Self::contains_await(&kw.value))
            }
            Expr::BoolOp(ExprBoolOp { values, .. }) => values.iter().any(Self::contains_await),
            Expr::BinOp(ExprBinOp { left, right, .. }) => {
                Self::contains_await(left) || Self::contains_await(right)
            }
            Expr::Subscript(ExprSubscript { value, slice, .. }) => {
                Self::contains_await(value) || Self::contains_await(slice)
            }
            Expr::UnaryOp(ExprUnaryOp { operand, .. }) => Self::contains_await(operand),
            Expr::Attribute(ExprAttribute { value, .. }) => Self::contains_await(value),
            Expr::Compare(ExprCompare {
                left, comparators, ..
            }) => Self::contains_await(left) || comparators.iter().any(Self::contains_await),
            Expr::List(ExprList { elts, .. }) => elts.iter().any(Self::contains_await),
            Expr::Tuple(ExprTuple { elts, .. }) => elts.iter().any(Self::contains_await),
            Expr::Set(ExprSet { elts, .. }) => elts.iter().any(Self::contains_await),
            Expr::Dict(ExprDict { items, .. }) => items
                .iter()
                .flat_map(|item| &item.key)
                .any(Self::contains_await),
            Expr::Slice(ExprSlice {
                lower, upper, step, ..
            }) => {
                lower.as_deref().is_some_and(Self::contains_await)
                    || upper.as_deref().is_some_and(Self::contains_await)
                    || step.as_deref().is_some_and(Self::contains_await)
            }
            Expr::Yield(ExprYield { value, .. }) => {
                value.as_deref().is_some_and(Self::contains_await)
            }
            Expr::Await(ExprAwait { .. }) => true,
            Expr::YieldFrom(ExprYieldFrom { value, .. }) => Self::contains_await(value),
            Expr::Name(ExprName { .. }) => false,
            Expr::Lambda(ExprLambda { body, .. }) => Self::contains_await(body),
            Expr::ListComp(ExprListComp {
                elt, generators, ..
            }) => {
                Self::contains_await(elt)
                    || generators.iter().any(|jen| Self::contains_await(&jen.iter))
            }
            Expr::SetComp(ExprSetComp {
                elt, generators, ..
            }) => {
                Self::contains_await(elt)
                    || generators.iter().any(|jen| Self::contains_await(&jen.iter))
            }
            Expr::DictComp(ExprDictComp {
                key,
                value,
                generators,
                ..
            }) => {
                Self::contains_await(key)
                    || Self::contains_await(value)
                    || generators.iter().any(|jen| Self::contains_await(&jen.iter))
            }
            Expr::Generator(ExprGenerator {
                elt, generators, ..
            }) => {
                Self::contains_await(elt)
                    || generators.iter().any(|jen| Self::contains_await(&jen.iter))
            }
            Expr::Starred(expr) => Self::contains_await(&expr.value),
            Expr::If(ExprIf {
                test, body, orelse, ..
            }) => {
                Self::contains_await(test)
                    || Self::contains_await(body)
                    || Self::contains_await(orelse)
            }

            Expr::Named(ExprNamed {
                target,
                value,
                range: _,
            }) => Self::contains_await(target) || Self::contains_await(value),
            Expr::FString(ExprFString { value, range: _ }) => {
                fn expr_element_contains_await<F: Copy + Fn(&Expr) -> bool>(
                    expr_element: &FStringExpressionElement,
                    contains_await: F,
                ) -> bool {
                    contains_await(&expr_element.expression)
                        || expr_element
                            .format_spec
                            .iter()
                            .flat_map(|spec| spec.elements.expressions())
                            .any(|element| expr_element_contains_await(element, contains_await))
                }

                value.elements().any(|element| match element {
                    FStringElement::Expression(expr_element) => {
                        expr_element_contains_await(expr_element, Self::contains_await)
                    }
                    FStringElement::Literal(_) => false,
                })
            }
            Expr::StringLiteral(_)
            | Expr::BytesLiteral(_)
            | Expr::NumberLiteral(_)
            | Expr::BooleanLiteral(_)
            | Expr::NoneLiteral(_)
            | Expr::EllipsisLiteral(_)
            | Expr::IpyEscapeCommand(_) => false,
        }
    }

    fn compile_expr_fstring(&mut self, fstring: &ExprFString) -> CompileResult<()> {
        let fstring = &fstring.value;
        for part in fstring {
            self.compile_fstring_part(part)?;
        }
        let part_count: u32 = fstring
            .iter()
            .len()
            .try_into()
            .expect("BuildString size overflowed");
        if part_count > 1 {
            emit!(self, Instruction::BuildString { size: part_count });
        }

        Ok(())
    }

    fn compile_fstring_part(&mut self, part: &FStringPart) -> CompileResult<()> {
        match part {
            FStringPart::Literal(string) => {
                if string.value.contains(char::REPLACEMENT_CHARACTER) {
                    // might have a surrogate literal; should reparse to be sure
                    let source = self.source_code.get_range(string.range);
                    let value =
                        crate::string_parser::parse_string_literal(source, string.flags.into());
                    self.emit_load_const(ConstantData::Str {
                        value: value.into(),
                    });
                } else {
                    self.emit_load_const(ConstantData::Str {
                        value: string.value.to_string().into(),
                    });
                }
                Ok(())
            }
            FStringPart::FString(fstring) => self.compile_fstring(fstring),
        }
    }

    fn compile_fstring(&mut self, fstring: &FString) -> CompileResult<()> {
        self.compile_fstring_elements(fstring.flags, &fstring.elements)
    }

    fn compile_fstring_elements(
        &mut self,
        flags: FStringFlags,
        fstring_elements: &FStringElements,
    ) -> CompileResult<()> {
        let mut element_count = 0;
        for element in fstring_elements {
            element_count += 1;
            match element {
                FStringElement::Literal(string) => {
                    if string.value.contains(char::REPLACEMENT_CHARACTER) {
                        // might have a surrogate literal; should reparse to be sure
                        let source = self.source_code.get_range(string.range);
                        let value = crate::string_parser::parse_fstring_literal_element(
                            source.into(),
                            flags.into(),
                        );
                        self.emit_load_const(ConstantData::Str {
                            value: value.into(),
                        });
                    } else {
                        self.emit_load_const(ConstantData::Str {
                            value: string.value.to_string().into(),
                        });
                    }
                }
                FStringElement::Expression(fstring_expr) => {
                    let mut conversion = fstring_expr.conversion;

                    if let Some(DebugText { leading, trailing }) = &fstring_expr.debug_text {
                        let range = fstring_expr.expression.range();
                        let source = self.source_code.get_range(range);
                        let text = [leading, source, trailing].concat();

                        self.emit_load_const(ConstantData::Str { value: text.into() });
                        element_count += 1;
                    }

                    match &fstring_expr.format_spec {
                        None => {
                            self.emit_load_const(ConstantData::Str {
                                value: Wtf8Buf::new(),
                            });
                            // Match CPython behavior: If debug text is present, apply repr conversion.
                            // See: https://github.com/python/cpython/blob/f61afca262d3a0aa6a8a501db0b1936c60858e35/Parser/action_helpers.c#L1456
                            if conversion == ConversionFlag::None
                                && fstring_expr.debug_text.is_some()
                            {
                                conversion = ConversionFlag::Repr;
                            }
                        }
                        Some(format_spec) => {
                            self.compile_fstring_elements(flags, &format_spec.elements)?;
                        }
                    }

                    self.compile_expression(&fstring_expr.expression)?;

                    let conversion = match conversion {
                        ConversionFlag::None => bytecode::ConversionFlag::None,
                        ConversionFlag::Str => bytecode::ConversionFlag::Str,
                        ConversionFlag::Ascii => bytecode::ConversionFlag::Ascii,
                        ConversionFlag::Repr => bytecode::ConversionFlag::Repr,
                    };
                    emit!(self, Instruction::FormatValue { conversion });
                }
            }
        }

        if element_count == 0 {
            // ensure to put an empty string on the stack if there aren't any fstring elements
            self.emit_load_const(ConstantData::Str {
                value: Wtf8Buf::new(),
            });
        } else if element_count > 1 {
            emit!(
                self,
                Instruction::BuildString {
                    size: element_count
                }
            );
        }

        Ok(())
    }
}

trait EmitArg<Arg: OpArgType> {
    fn emit(
        self,
        f: impl FnOnce(OpArgMarker<Arg>) -> Instruction,
    ) -> (Instruction, OpArg, ir::BlockIdx);
}
impl<T: OpArgType> EmitArg<T> for T {
    fn emit(
        self,
        f: impl FnOnce(OpArgMarker<T>) -> Instruction,
    ) -> (Instruction, OpArg, ir::BlockIdx) {
        let (marker, arg) = OpArgMarker::new(self);
        (f(marker), arg, ir::BlockIdx::NULL)
    }
}
impl EmitArg<bytecode::Label> for ir::BlockIdx {
    fn emit(
        self,
        f: impl FnOnce(OpArgMarker<bytecode::Label>) -> Instruction,
    ) -> (Instruction, OpArg, ir::BlockIdx) {
        (f(OpArgMarker::marker()), OpArg::null(), self)
    }
}

/// Strips leading whitespace from a docstring.
///
/// The code has been ported from `_PyCompile_CleanDoc` in cpython.
/// `inspect.cleandoc` is also a good reference, but has a few incompatibilities.
fn clean_doc(doc: &str) -> String {
    let doc = expandtabs(doc, 8);
    // First pass: find minimum indentation of any non-blank lines
    // after first line.
    let margin = doc
        .lines()
        // Find the non-blank lines
        .filter(|line| !line.trim().is_empty())
        // get the one with the least indentation
        .map(|line| line.chars().take_while(|c| c == &' ').count())
        .min();
    if let Some(margin) = margin {
        let mut cleaned = String::with_capacity(doc.len());
        // copy first line without leading whitespace
        if let Some(first_line) = doc.lines().next() {
            cleaned.push_str(first_line.trim_start());
        }
        // copy subsequent lines without margin.
        for line in doc.split('\n').skip(1) {
            cleaned.push('\n');
            let cleaned_line = line.chars().skip(margin).collect::<String>();
            cleaned.push_str(&cleaned_line);
        }

        cleaned
    } else {
        doc.to_owned()
    }
}

// copied from rustpython_common::str, so we don't have to depend on it just for this function
fn expandtabs(input: &str, tab_size: usize) -> String {
    let tab_stop = tab_size;
    let mut expanded_str = String::with_capacity(input.len());
    let mut tab_size = tab_stop;
    let mut col_count = 0usize;
    for ch in input.chars() {
        match ch {
            '\t' => {
                let num_spaces = tab_size - col_count;
                col_count += num_spaces;
                let expand = " ".repeat(num_spaces);
                expanded_str.push_str(&expand);
            }
            '\r' | '\n' => {
                expanded_str.push(ch);
                col_count = 0;
                tab_size = 0;
            }
            _ => {
                expanded_str.push(ch);
                col_count += 1;
            }
        }
        if col_count >= tab_size {
            tab_size += tab_stop;
        }
    }
    expanded_str
}

fn split_doc<'a>(body: &'a [Stmt], opts: &CompileOpts) -> (Option<String>, &'a [Stmt]) {
    if let Some((Stmt::Expr(expr), body_rest)) = body.split_first() {
        let doc_comment = match &*expr.value {
            Expr::StringLiteral(value) => Some(&value.value),
            // f-strings are not allowed in Python doc comments.
            Expr::FString(_) => None,
            _ => None,
        };
        if let Some(doc) = doc_comment {
            return if opts.optimize < 2 {
                (Some(clean_doc(doc.to_str())), body_rest)
            } else {
                (None, body_rest)
            };
        }
    }
    (None, body)
}

pub fn ruff_int_to_bigint(int: &Int) -> Result<BigInt, CodegenErrorType> {
    if let Some(small) = int.as_u64() {
        Ok(BigInt::from(small))
    } else {
        parse_big_integer(int)
    }
}

/// Converts a `ruff` ast integer into a `BigInt`.
/// Unlike small integers, big integers may be stored in one of four possible radix representations.
fn parse_big_integer(int: &Int) -> Result<BigInt, CodegenErrorType> {
    // TODO: Improve ruff API
    // Can we avoid this copy?
    let s = format!("{}", int);
    let mut s = s.as_str();
    // See: https://peps.python.org/pep-0515/#literal-grammar
    let radix = match s.get(0..2) {
        Some("0b" | "0B") => {
            s = s.get(2..).unwrap_or(s);
            2
        }
        Some("0o" | "0O") => {
            s = s.get(2..).unwrap_or(s);
            8
        }
        Some("0x" | "0X") => {
            s = s.get(2..).unwrap_or(s);
            16
        }
        _ => 10,
    };

    BigInt::from_str_radix(s, radix).map_err(|e| {
        CodegenErrorType::SyntaxError(format!(
            "unparsed integer literal (radix {radix}): {s} ({e})"
        ))
    })
}

// Note: Not a good practice in general. Keep this trait private only for compiler
trait ToU32 {
    fn to_u32(self) -> u32;
}

impl ToU32 for usize {
    fn to_u32(self) -> u32 {
        self.try_into().unwrap()
    }
}

#[cfg(test)]
mod ruff_tests {
    use super::*;
    use ruff_python_ast::name::Name;
    use ruff_python_ast::*;

    /// Test if the compiler can correctly identify fstrings containing an `await` expression.
    #[test]
    fn test_fstring_contains_await() {
        let range = TextRange::default();
        let flags = FStringFlags::empty();

        // f'{x}'
        let expr_x = Expr::Name(ExprName {
            range,
            id: Name::new("x"),
            ctx: ExprContext::Load,
        });
        let not_present = &Expr::FString(ExprFString {
            range,
            value: FStringValue::single(FString {
                range,
                elements: vec![FStringElement::Expression(FStringExpressionElement {
                    range,
                    expression: Box::new(expr_x),
                    debug_text: None,
                    conversion: ConversionFlag::None,
                    format_spec: None,
                })]
                .into(),
                flags,
            }),
        });
        assert!(!Compiler::contains_await(not_present));

        // f'{await x}'
        let expr_await_x = Expr::Await(ExprAwait {
            range,
            value: Box::new(Expr::Name(ExprName {
                range,
                id: Name::new("x"),
                ctx: ExprContext::Load,
            })),
        });
        let present = &Expr::FString(ExprFString {
            range,
            value: FStringValue::single(FString {
                range,
                elements: vec![FStringElement::Expression(FStringExpressionElement {
                    range,
                    expression: Box::new(expr_await_x),
                    debug_text: None,
                    conversion: ConversionFlag::None,
                    format_spec: None,
                })]
                .into(),
                flags,
            }),
        });
        assert!(Compiler::contains_await(present));

        // f'{x:{await y}}'
        let expr_x = Expr::Name(ExprName {
            range,
            id: Name::new("x"),
            ctx: ExprContext::Load,
        });
        let expr_await_y = Expr::Await(ExprAwait {
            range,
            value: Box::new(Expr::Name(ExprName {
                range,
                id: Name::new("y"),
                ctx: ExprContext::Load,
            })),
        });
        let present = &Expr::FString(ExprFString {
            range,
            value: FStringValue::single(FString {
                range,
                elements: vec![FStringElement::Expression(FStringExpressionElement {
                    range,
                    expression: Box::new(expr_x),
                    debug_text: None,
                    conversion: ConversionFlag::None,
                    format_spec: Some(Box::new(FStringFormatSpec {
                        range,
                        elements: vec![FStringElement::Expression(FStringExpressionElement {
                            range,
                            expression: Box::new(expr_await_y),
                            debug_text: None,
                            conversion: ConversionFlag::None,
                            format_spec: None,
                        })]
                        .into(),
                    })),
                })]
                .into(),
                flags,
            }),
        });
        assert!(Compiler::contains_await(present));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compile_exec(source: &str) -> CodeObject {
        let opts = CompileOpts::default();
        let source_code = SourceCode::new("source_path", source);
        let parsed =
            ruff_python_parser::parse(source_code.text, ruff_python_parser::Mode::Module.into())
                .unwrap();
        let ast = parsed.into_syntax();
        let ast = match ast {
            ruff_python_ast::Mod::Module(stmts) => stmts,
            _ => unreachable!(),
        };
        let symbol_table = SymbolTable::scan_program(&ast, source_code.clone())
            .map_err(|e| e.into_codegen_error(source_code.path.to_owned()))
            .unwrap();
        let mut compiler = Compiler::new(opts, source_code, "<module>".to_owned());
        compiler.compile_program(&ast, symbol_table).unwrap();
        compiler.pop_code_object()
    }

    macro_rules! assert_dis_snapshot {
        ($value:expr) => {
            insta::assert_snapshot!(
                insta::internals::AutoName,
                $value.display_expand_code_objects().to_string(),
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

    #[test]
    fn test_nested_double_async_with() {
        assert_dis_snapshot!(compile_exec(
            "\
for stop_exc in (StopIteration('spam'), StopAsyncIteration('ham')):
    with self.subTest(type=type(stop_exc)):
        try:
            async with egg():
                raise stop_exc
        except Exception as ex:
            self.assertIs(ex, stop_exc)
        else:
            self.fail(f'{stop_exc} was suppressed')
"
        ));
    }

    #[test]
    fn test_match() {
        assert_dis_snapshot!(compile_exec(
            "\
class Test:
    pass

t = Test()
match t:
    case Test():
        assert True
    case _:
        assert False
"
        ));
    }
}
