//!
//! Take an AST and transform it into bytecode
//!
//! Inspirational code:
//!   <https://github.com/python/cpython/blob/main/Python/compile.c>
//!   <https://github.com/micropython/micropython/blob/master/py/compile.c>

#![deny(clippy::cast_possible_truncation)]

use crate::{
    error::{CodegenError, CodegenErrorType},
    ir,
    symboltable::{self, SymbolFlags, SymbolScope, SymbolTable},
    IndexSet,
};
use itertools::Itertools;
use num_complex::Complex64;
use num_traits::ToPrimitive;
use rustpython_ast::located::{self as located_ast, Located};
use rustpython_compiler_core::{
    bytecode::{self, Arg as OpArgMarker, CodeObject, ConstantData, Instruction, OpArg, OpArgType},
    Mode,
};
use rustpython_parser_core::source_code::{LineNumber, SourceLocation};
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
struct Compiler {
    code_stack: Vec<ir::CodeInfo>,
    symbol_table_stack: Vec<SymbolTable>,
    source_path: String,
    current_source_location: SourceLocation,
    qualified_path: Vec<String>,
    done_with_future_stmts: bool,
    future_annotations: bool,
    ctx: CompileContext,
    class_name: Option<String>,
    opts: CompileOpts,
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

/// Compile an located_ast::Mod produced from rustpython_parser::parse()
pub fn compile_top(
    ast: &located_ast::Mod,
    source_path: String,
    mode: Mode,
    opts: CompileOpts,
) -> CompileResult<CodeObject> {
    match ast {
        located_ast::Mod::Module(located_ast::ModModule { body, .. }) => {
            compile_program(body, source_path, opts)
        }
        located_ast::Mod::Interactive(located_ast::ModInteractive { body, .. }) => match mode {
            Mode::Single => compile_program_single(body, source_path, opts),
            Mode::BlockExpr => compile_block_expression(body, source_path, opts),
            _ => unreachable!("only Single and BlockExpr parsed to Interactive"),
        },
        located_ast::Mod::Expression(located_ast::ModExpression { body, .. }) => {
            compile_expression(body, source_path, opts)
        }
        located_ast::Mod::FunctionType(_) => panic!("can't compile a FunctionType"),
    }
}

/// A helper function for the shared code of the different compile functions
fn compile_impl<Ast: ?Sized>(
    ast: &Ast,
    source_path: String,
    opts: CompileOpts,
    make_symbol_table: impl FnOnce(&Ast) -> Result<SymbolTable, symboltable::SymbolTableError>,
    compile: impl FnOnce(&mut Compiler, &Ast, SymbolTable) -> CompileResult<()>,
) -> CompileResult<CodeObject> {
    let symbol_table = match make_symbol_table(ast) {
        Ok(x) => x,
        Err(e) => return Err(e.into_codegen_error(source_path)),
    };

    let mut compiler = Compiler::new(opts, source_path, "<module>".to_owned());
    compile(&mut compiler, ast, symbol_table)?;
    let code = compiler.pop_code_object();
    trace!("Compilation completed: {:?}", code);
    Ok(code)
}

/// Compile a standard Python program to bytecode
pub fn compile_program(
    ast: &[located_ast::Stmt],
    source_path: String,
    opts: CompileOpts,
) -> CompileResult<CodeObject> {
    compile_impl(
        ast,
        source_path,
        opts,
        SymbolTable::scan_program,
        Compiler::compile_program,
    )
}

/// Compile a Python program to bytecode for the context of a REPL
pub fn compile_program_single(
    ast: &[located_ast::Stmt],
    source_path: String,
    opts: CompileOpts,
) -> CompileResult<CodeObject> {
    compile_impl(
        ast,
        source_path,
        opts,
        SymbolTable::scan_program,
        Compiler::compile_program_single,
    )
}

pub fn compile_block_expression(
    ast: &[located_ast::Stmt],
    source_path: String,
    opts: CompileOpts,
) -> CompileResult<CodeObject> {
    compile_impl(
        ast,
        source_path,
        opts,
        SymbolTable::scan_program,
        Compiler::compile_block_expr,
    )
}

pub fn compile_expression(
    ast: &located_ast::Expr,
    source_path: String,
    opts: CompileOpts,
) -> CompileResult<CodeObject> {
    compile_impl(
        ast,
        source_path,
        opts,
        SymbolTable::scan_expr,
        Compiler::compile_eval,
    )
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

impl Compiler {
    fn new(opts: CompileOpts, source_path: String, code_name: String) -> Self {
        let module_code = ir::CodeInfo {
            flags: bytecode::CodeFlags::NEW_LOCALS,
            posonlyarg_count: 0,
            arg_count: 0,
            kwonlyarg_count: 0,
            source_path: source_path.clone(),
            first_line_number: LineNumber::MIN,
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
            source_path,
            current_source_location: SourceLocation::default(),
            qualified_path: Vec::new(),
            done_with_future_stmts: false,
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

    fn error(&mut self, error: CodegenErrorType) -> CodegenError {
        self.error_loc(error, self.current_source_location)
    }
    fn error_loc(&mut self, error: CodegenErrorType, location: SourceLocation) -> CodegenError {
        CodegenError {
            error,
            location: Some(location),
            source_path: self.source_path.clone(),
        }
    }

    fn push_output(
        &mut self,
        flags: bytecode::CodeFlags,
        posonlyarg_count: u32,
        arg_count: u32,
        kwonlyarg_count: u32,
        obj_name: String,
    ) {
        let source_path = self.source_path.clone();
        let first_line_number = self.get_source_line_number();

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
            .filter(|(_, s)| {
                s.scope == SymbolScope::Free || s.flags.contains(SymbolFlags::FREE_CLASS)
            })
            .map(|(var, _)| var.clone())
            .collect();

        self.symbol_table_stack.push(table);

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
        let table = self.symbol_table_stack.pop().unwrap();
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
        body: &[located_ast::Stmt],
        symbol_table: SymbolTable,
    ) -> CompileResult<()> {
        let size_before = self.code_stack.len();
        self.symbol_table_stack.push(symbol_table);

        let (doc, statements) = split_doc(body);
        if let Some(value) = doc {
            self.emit_constant(ConstantData::Str { value });
            let doc = self.name("__doc__");
            emit!(self, Instruction::StoreGlobal(doc))
        }

        if Self::find_ann(statements) {
            emit!(self, Instruction::SetupAnnotation);
        }

        self.compile_statements(statements)?;

        assert_eq!(self.code_stack.len(), size_before);

        // Emit None at end:
        self.emit_constant(ConstantData::None);
        emit!(self, Instruction::ReturnValue);
        Ok(())
    }

    fn compile_program_single(
        &mut self,
        body: &[located_ast::Stmt],
        symbol_table: SymbolTable,
    ) -> CompileResult<()> {
        self.symbol_table_stack.push(symbol_table);

        if let Some((last, body)) = body.split_last() {
            for statement in body {
                if let located_ast::Stmt::Expr(located_ast::StmtExpr { value, .. }) = &statement {
                    self.compile_expression(value)?;
                    emit!(self, Instruction::PrintExpr);
                } else {
                    self.compile_statement(statement)?;
                }
            }

            if let located_ast::Stmt::Expr(located_ast::StmtExpr { value, .. }) = &last {
                self.compile_expression(value)?;
                emit!(self, Instruction::Duplicate);
                emit!(self, Instruction::PrintExpr);
            } else {
                self.compile_statement(last)?;
                self.emit_constant(ConstantData::None);
            }
        } else {
            self.emit_constant(ConstantData::None);
        };

        emit!(self, Instruction::ReturnValue);
        Ok(())
    }

    fn compile_block_expr(
        &mut self,
        body: &[located_ast::Stmt],
        symbol_table: SymbolTable,
    ) -> CompileResult<()> {
        self.symbol_table_stack.push(symbol_table);

        self.compile_statements(body)?;

        if let Some(last_statement) = body.last() {
            match last_statement {
                located_ast::Stmt::Expr(_) => {
                    self.current_block().instructions.pop(); // pop Instruction::Pop
                }
                located_ast::Stmt::FunctionDef(_)
                | located_ast::Stmt::AsyncFunctionDef(_)
                | located_ast::Stmt::ClassDef(_) => {
                    let store_inst = self.current_block().instructions.pop().unwrap(); // pop Instruction::Store
                    emit!(self, Instruction::Duplicate);
                    self.current_block().instructions.push(store_inst);
                }
                _ => self.emit_constant(ConstantData::None),
            }
        }
        emit!(self, Instruction::ReturnValue);

        Ok(())
    }

    // Compile statement in eval mode:
    fn compile_eval(
        &mut self,
        expression: &located_ast::Expr,
        symbol_table: SymbolTable,
    ) -> CompileResult<()> {
        self.symbol_table_stack.push(symbol_table);
        self.compile_expression(expression)?;
        emit!(self, Instruction::ReturnValue);
        Ok(())
    }

    fn compile_statements(&mut self, statements: &[located_ast::Stmt]) -> CompileResult<()> {
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
        let symbol = symbol_table.lookup(name.as_ref()).expect(
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

        if NameUsage::Load == usage && name == "__debug__" {
            self.emit_constant(ConstantData::Boolean {
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

    fn compile_statement(&mut self, statement: &located_ast::Stmt) -> CompileResult<()> {
        use located_ast::*;

        trace!("Compiling {:?}", statement);
        self.set_source_location(statement.location());

        match &statement {
            // we do this here because `from __future__` still executes that `from` statement at runtime,
            // we still need to compile the ImportFrom down below
            Stmt::ImportFrom(located_ast::StmtImportFrom { module, names, .. })
                if module.as_ref().map(|id| id.as_str()) == Some("__future__") =>
            {
                self.compile_future_features(names)?
            }
            // if we find any other statement, stop accepting future statements
            _ => self.done_with_future_stmts = true,
        }

        match &statement {
            Stmt::Import(StmtImport { names, .. }) => {
                // import a, b, c as d
                for name in names {
                    let name = &name;
                    self.emit_constant(ConstantData::Integer {
                        value: num_traits::Zero::zero(),
                    });
                    self.emit_constant(ConstantData::None);
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
                        return Err(self.error_loc(
                            CodegenErrorType::FunctionImportStar,
                            statement.location(),
                        ));
                    }
                    vec![ConstantData::Str {
                        value: "*".to_owned(),
                    }]
                } else {
                    names
                        .iter()
                        .map(|n| ConstantData::Str {
                            value: n.name.to_string(),
                        })
                        .collect()
                };

                let module_idx = module.as_ref().map(|s| self.name(s.as_str()));

                // from .... import (*fromlist)
                self.emit_constant(ConstantData::Integer {
                    value: level.as_ref().map_or(0, |level| level.to_u32()).into(),
                });
                self.emit_constant(ConstantData::Tuple {
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
                test, body, orelse, ..
            }) => {
                let after_block = self.new_block();
                if orelse.is_empty() {
                    // Only if:
                    self.compile_jump_if(test, false, after_block)?;
                    self.compile_statements(body)?;
                } else {
                    // if - else:
                    let else_block = self.new_block();
                    self.compile_jump_if(test, false, else_block)?;
                    self.compile_statements(body)?;
                    emit!(
                        self,
                        Instruction::Jump {
                            target: after_block,
                        }
                    );

                    // else:
                    self.switch_to_block(else_block);
                    self.compile_statements(orelse)?;
                }
                self.switch_to_block(after_block);
            }
            Stmt::While(StmtWhile {
                test, body, orelse, ..
            }) => self.compile_while(test, body, orelse)?,
            Stmt::With(StmtWith { items, body, .. }) => self.compile_with(items, body, false)?,
            Stmt::AsyncWith(StmtAsyncWith { items, body, .. }) => {
                self.compile_with(items, body, true)?
            }
            Stmt::For(StmtFor {
                target,
                iter,
                body,
                orelse,
                ..
            }) => self.compile_for(target, iter, body, orelse, false)?,
            Stmt::AsyncFor(StmtAsyncFor {
                target,
                iter,
                body,
                orelse,
                ..
            }) => self.compile_for(target, iter, body, orelse, true)?,
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
                ..
            }) => self.compile_try_statement(body, handlers, orelse, finalbody)?,
            Stmt::TryStar(StmtTryStar {
                body,
                handlers,
                orelse,
                finalbody,
                ..
            }) => self.compile_try_star_statement(body, handlers, orelse, finalbody)?,
            Stmt::FunctionDef(StmtFunctionDef {
                name,
                args,
                body,
                decorator_list,
                returns,
                ..
            }) => self.compile_function_def(
                name.as_str(),
                args,
                body,
                decorator_list,
                returns.as_deref(),
                false,
            )?,
            Stmt::AsyncFunctionDef(StmtAsyncFunctionDef {
                name,
                args,
                body,
                decorator_list,
                returns,
                ..
            }) => self.compile_function_def(
                name.as_str(),
                args,
                body,
                decorator_list,
                returns.as_deref(),
                true,
            )?,
            Stmt::ClassDef(StmtClassDef {
                name,
                body,
                bases,
                keywords,
                decorator_list,
                ..
            }) => self.compile_class_def(name.as_str(), body, bases, keywords, decorator_list)?,
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
                        self.error_loc(CodegenErrorType::InvalidBreak, statement.location())
                    );
                }
            },
            Stmt::Continue(_) => match self.ctx.loop_data {
                Some((start, _)) => {
                    emit!(self, Instruction::Continue { target: start });
                }
                None => {
                    return Err(
                        self.error_loc(CodegenErrorType::InvalidContinue, statement.location())
                    );
                }
            },
            Stmt::Return(StmtReturn { value, .. }) => {
                if !self.ctx.in_func() {
                    return Err(
                        self.error_loc(CodegenErrorType::InvalidReturn, statement.location())
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
                            return Err(self.error_loc(
                                CodegenErrorType::AsyncReturnValue,
                                statement.location(),
                            ));
                        }
                        self.compile_expression(v)?;
                    }
                    None => {
                        self.emit_constant(ConstantData::None);
                    }
                }

                emit!(self, Instruction::ReturnValue);
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
        }
        Ok(())
    }

    fn compile_delete(&mut self, expression: &located_ast::Expr) -> CompileResult<()> {
        match &expression {
            located_ast::Expr::Name(located_ast::ExprName { id, .. }) => {
                self.compile_name(id.as_str(), NameUsage::Delete)?
            }
            located_ast::Expr::Attribute(located_ast::ExprAttribute { value, attr, .. }) => {
                self.check_forbidden_name(attr.as_str(), NameUsage::Delete)?;
                self.compile_expression(value)?;
                let idx = self.name(attr.as_str());
                emit!(self, Instruction::DeleteAttr { idx });
            }
            located_ast::Expr::Subscript(located_ast::ExprSubscript { value, slice, .. }) => {
                self.compile_expression(value)?;
                self.compile_expression(slice)?;
                emit!(self, Instruction::DeleteSubscript);
            }
            located_ast::Expr::Tuple(located_ast::ExprTuple { elts, .. })
            | located_ast::Expr::List(located_ast::ExprList { elts, .. }) => {
                for element in elts {
                    self.compile_delete(element)?;
                }
            }
            located_ast::Expr::BinOp(_) | located_ast::Expr::UnaryOp(_) => {
                return Err(self.error(CodegenErrorType::Delete("expression")))
            }
            _ => return Err(self.error(CodegenErrorType::Delete(expression.python_name()))),
        }
        Ok(())
    }

    fn enter_function(
        &mut self,
        name: &str,
        args: &located_ast::Arguments,
    ) -> CompileResult<bytecode::MakeFunctionFlags> {
        let have_defaults = !args.defaults.is_empty();
        if have_defaults {
            // Construct a tuple:
            let size = args.defaults.len().to_u32();
            for element in &args.defaults {
                self.compile_expression(element)?;
            }
            emit!(self, Instruction::BuildTuple { size });
        }

        if !args.kw_defaults.is_empty() {
            let required_kw_count = args.kwonlyargs.len().saturating_sub(args.kw_defaults.len());
            for (kw, default) in args.kwonlyargs[required_kw_count..]
                .iter()
                .zip(&args.kw_defaults)
            {
                self.emit_constant(ConstantData::Str {
                    value: kw.arg.to_string(),
                });
                self.compile_expression(default)?;
            }
            emit!(
                self,
                Instruction::BuildMap {
                    size: args.kw_defaults.len().to_u32(),
                }
            );
        }

        let mut func_flags = bytecode::MakeFunctionFlags::empty();
        if have_defaults {
            func_flags |= bytecode::MakeFunctionFlags::DEFAULTS;
        }
        if !args.kw_defaults.is_empty() {
            func_flags |= bytecode::MakeFunctionFlags::KW_ONLY_DEFAULTS;
        }

        self.push_output(
            bytecode::CodeFlags::NEW_LOCALS | bytecode::CodeFlags::IS_OPTIMIZED,
            args.posonlyargs.len().to_u32(),
            (args.posonlyargs.len() + args.args.len()).to_u32(),
            args.kwonlyargs.len().to_u32(),
            name.to_owned(),
        );

        let args_iter = std::iter::empty()
            .chain(&args.posonlyargs)
            .chain(&args.args)
            .chain(&args.kwonlyargs);
        for name in args_iter {
            self.varname(name.arg.as_str())?;
        }

        if let Some(name) = args.vararg.as_deref() {
            self.current_code_info().flags |= bytecode::CodeFlags::HAS_VARARGS;
            self.varname(name.arg.as_str())?;
        }
        if let Some(name) = args.kwarg.as_deref() {
            self.current_code_info().flags |= bytecode::CodeFlags::HAS_VARKEYWORDS;
            self.varname(name.arg.as_str())?;
        }

        Ok(func_flags)
    }

    fn prepare_decorators(&mut self, decorator_list: &[located_ast::Expr]) -> CompileResult<()> {
        for decorator in decorator_list {
            self.compile_expression(decorator)?;
        }
        Ok(())
    }

    fn apply_decorators(&mut self, decorator_list: &[located_ast::Expr]) {
        // Apply decorators:
        for _ in decorator_list {
            emit!(self, Instruction::CallFunctionPositional { nargs: 1 });
        }
    }

    fn compile_try_statement(
        &mut self,
        body: &[located_ast::Stmt],
        handlers: &[located_ast::Excepthandler],
        orelse: &[located_ast::Stmt],
        finalbody: &[located_ast::Stmt],
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
            let located_ast::Excepthandler::ExceptHandler(
                located_ast::ExcepthandlerExceptHandler {
                    type_, name, body, ..
                },
            ) = &handler;
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
        _body: &[located_ast::Stmt],
        _handlers: &[located_ast::Excepthandler],
        _orelse: &[located_ast::Stmt],
        _finalbody: &[located_ast::Stmt],
    ) -> CompileResult<()> {
        Err(self.error(CodegenErrorType::NotImplementedYet))
    }

    fn is_forbidden_arg_name(name: &str) -> bool {
        is_forbidden_name(name)
    }

    fn compile_function_def(
        &mut self,
        name: &str,
        args: &located_ast::Arguments,
        body: &[located_ast::Stmt],
        decorator_list: &[located_ast::Expr],
        returns: Option<&located_ast::Expr>, // TODO: use type hint somehow..
        is_async: bool,
    ) -> CompileResult<()> {
        // Create bytecode for this function:

        self.prepare_decorators(decorator_list)?;
        let mut func_flags = self.enter_function(name, args)?;
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

        let (doc_str, body) = split_doc(body);

        self.current_code_info()
            .constants
            .insert_full(ConstantData::None);

        self.compile_statements(body)?;

        // Emit None at end:
        match body.last() {
            Some(located_ast::Stmt::Return(_)) => {
                // the last instruction is a ReturnValue already, we don't need to emit it
            }
            _ => {
                self.emit_constant(ConstantData::None);
                emit!(self, Instruction::ReturnValue);
            }
        }

        let code = self.pop_code_object();
        self.qualified_path.pop();
        self.qualified_path.pop();
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
            self.compile_annotation(annotation)?;
            num_annotations += 1;
        }

        let args_iter = std::iter::empty()
            .chain(&args.posonlyargs)
            .chain(&args.args)
            .chain(&args.kwonlyargs)
            .chain(args.vararg.as_deref())
            .chain(args.kwarg.as_deref());
        for arg in args_iter {
            if let Some(annotation) = &arg.annotation {
                self.emit_constant(ConstantData::Str {
                    value: self.mangle(arg.arg.as_str()).into_owned(),
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

        self.emit_constant(ConstantData::Code {
            code: Box::new(code),
        });
        self.emit_constant(ConstantData::Str {
            value: qualified_name,
        });

        // Turn code object into function object:
        emit!(self, Instruction::MakeFunction(func_flags));

        emit!(self, Instruction::Duplicate);
        self.load_docstring(doc_str);
        emit!(self, Instruction::Rotate2);
        let doc = self.name("__doc__");
        emit!(self, Instruction::StoreAttr { idx: doc });

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
                    var, code.obj_name, self.source_path
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
    fn find_ann(body: &[located_ast::Stmt]) -> bool {
        use located_ast::*;

        for statement in body {
            let res = match &statement {
                Stmt::AnnAssign(_) => true,
                Stmt::For(StmtFor { body, orelse, .. }) => {
                    Self::find_ann(body) || Self::find_ann(orelse)
                }
                Stmt::If(StmtIf { body, orelse, .. }) => {
                    Self::find_ann(body) || Self::find_ann(orelse)
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
        body: &[located_ast::Stmt],
        bases: &[located_ast::Expr],
        keywords: &[located_ast::Keyword],
        decorator_list: &[located_ast::Expr],
    ) -> CompileResult<()> {
        self.prepare_decorators(decorator_list)?;

        emit!(self, Instruction::LoadBuildClass);

        let prev_ctx = self.ctx;
        self.ctx = CompileContext {
            func: FunctionContext::NoFunction,
            in_class: true,
            loop_data: None,
        };

        let prev_class_name = std::mem::replace(&mut self.class_name, Some(name.to_owned()));

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

        self.push_output(bytecode::CodeFlags::empty(), 0, 0, 0, name.to_owned());

        let (doc_str, body) = split_doc(body);

        let dunder_name = self.name("__name__");
        emit!(self, Instruction::LoadGlobal(dunder_name));
        let dunder_module = self.name("__module__");
        emit!(self, Instruction::StoreLocal(dunder_module));
        self.emit_constant(ConstantData::Str {
            value: qualified_name,
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
            self.emit_constant(ConstantData::None);
        }

        emit!(self, Instruction::ReturnValue);

        let code = self.pop_code_object();

        self.class_name = prev_class_name;
        self.qualified_path.pop();
        self.qualified_path.append(global_path_prefix.as_mut());
        self.ctx = prev_ctx;

        let mut func_flags = bytecode::MakeFunctionFlags::empty();

        if self.build_closure(&code) {
            func_flags |= bytecode::MakeFunctionFlags::CLOSURE;
        }

        self.emit_constant(ConstantData::Code {
            code: Box::new(code),
        });
        self.emit_constant(ConstantData::Str {
            value: name.to_owned(),
        });

        // Turn code object into function object:
        emit!(self, Instruction::MakeFunction(func_flags));

        self.emit_constant(ConstantData::Str {
            value: name.to_owned(),
        });

        let call = self.compile_call_inner(2, bases, keywords)?;
        self.compile_normal_call(call);

        self.apply_decorators(decorator_list);

        self.store_name(name)
    }

    fn load_docstring(&mut self, doc_str: Option<String>) {
        // TODO: __doc__ must be default None and no bytecode unless it is Some
        // Duplicate top of stack (the function or class object)

        // Doc string value:
        self.emit_constant(match doc_str {
            Some(doc) => ConstantData::Str { value: doc },
            None => ConstantData::None, // set docstring None if not declared
        });
    }

    fn compile_while(
        &mut self,
        test: &located_ast::Expr,
        body: &[located_ast::Stmt],
        orelse: &[located_ast::Stmt],
    ) -> CompileResult<()> {
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
        items: &[located_ast::Withitem],
        body: &[located_ast::Stmt],
        is_async: bool,
    ) -> CompileResult<()> {
        let with_location = self.current_source_location;

        let Some((item, items)) = items.split_first() else {
            return Err(self.error(CodegenErrorType::EmptyWithItems));
        };

        let final_block = {
            let final_block = self.new_block();
            self.compile_expression(&item.context_expr)?;

            self.set_source_location(with_location);
            if is_async {
                emit!(self, Instruction::BeforeAsyncWith);
                emit!(self, Instruction::GetAwaitable);
                self.emit_constant(ConstantData::None);
                emit!(self, Instruction::YieldFrom);
                emit!(self, Instruction::SetupAsyncWith { end: final_block });
            } else {
                emit!(self, Instruction::SetupWith { end: final_block });
            }

            match &item.optional_vars {
                Some(var) => {
                    self.set_source_location(var.location());
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
            self.set_source_location(with_location);
            self.compile_with(items, body, is_async)?;
        }

        // sort of "stack up" the layers of with blocks:
        // with a, b: body -> start_with(a) start_with(b) body() end_with(b) end_with(a)
        self.set_source_location(with_location);
        emit!(self, Instruction::PopBlock);

        emit!(self, Instruction::EnterFinally);

        self.switch_to_block(final_block);
        emit!(self, Instruction::WithCleanupStart);

        if is_async {
            emit!(self, Instruction::GetAwaitable);
            self.emit_constant(ConstantData::None);
            emit!(self, Instruction::YieldFrom);
        }

        emit!(self, Instruction::WithCleanupFinish);

        Ok(())
    }

    fn compile_for(
        &mut self,
        target: &located_ast::Expr,
        iter: &located_ast::Expr,
        body: &[located_ast::Stmt],
        orelse: &[located_ast::Stmt],
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
            self.emit_constant(ConstantData::None);
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

    fn compile_match(
        &mut self,
        subject: &located_ast::Expr,
        cases: &[located_ast::MatchCase],
    ) -> CompileResult<()> {
        eprintln!("match subject: {subject:?}");
        eprintln!("match cases: {cases:?}");
        Err(self.error(CodegenErrorType::NotImplementedYet))
    }

    fn compile_chained_comparison(
        &mut self,
        left: &located_ast::Expr,
        ops: &[located_ast::Cmpop],
        exprs: &[located_ast::Expr],
    ) -> CompileResult<()> {
        assert!(!ops.is_empty());
        assert_eq!(exprs.len(), ops.len());
        let (last_op, mid_ops) = ops.split_last().unwrap();
        let (last_val, mid_exprs) = exprs.split_last().unwrap();

        use bytecode::ComparisonOperator::*;
        use bytecode::TestOperator::*;
        let compile_cmpop = |c: &mut Self, op: &located_ast::Cmpop| match op {
            located_ast::Cmpop::Eq => emit!(c, Instruction::CompareOperation { op: Equal }),
            located_ast::Cmpop::NotEq => emit!(c, Instruction::CompareOperation { op: NotEqual }),
            located_ast::Cmpop::Lt => emit!(c, Instruction::CompareOperation { op: Less }),
            located_ast::Cmpop::LtE => emit!(c, Instruction::CompareOperation { op: LessOrEqual }),
            located_ast::Cmpop::Gt => emit!(c, Instruction::CompareOperation { op: Greater }),
            located_ast::Cmpop::GtE => {
                emit!(c, Instruction::CompareOperation { op: GreaterOrEqual })
            }
            located_ast::Cmpop::In => emit!(c, Instruction::TestOperation { op: In }),
            located_ast::Cmpop::NotIn => emit!(c, Instruction::TestOperation { op: NotIn }),
            located_ast::Cmpop::Is => emit!(c, Instruction::TestOperation { op: Is }),
            located_ast::Cmpop::IsNot => emit!(c, Instruction::TestOperation { op: IsNot }),
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

    fn compile_annotation(&mut self, annotation: &located_ast::Expr) -> CompileResult<()> {
        if self.future_annotations {
            self.emit_constant(ConstantData::Str {
                value: annotation.to_string(),
            });
        } else {
            self.compile_expression(annotation)?;
        }
        Ok(())
    }

    fn compile_annotated_assign(
        &mut self,
        target: &located_ast::Expr,
        annotation: &located_ast::Expr,
        value: Option<&located_ast::Expr>,
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

        if let located_ast::Expr::Name(located_ast::ExprName { id, .. }) = &target {
            // Store as dict entry in __annotations__ dict:
            let annotations = self.name("__annotations__");
            emit!(self, Instruction::LoadNameAny(annotations));
            self.emit_constant(ConstantData::Str {
                value: self.mangle(id.as_str()).into_owned(),
            });
            emit!(self, Instruction::StoreSubscript);
        } else {
            // Drop annotation if not assigned to simple identifier.
            emit!(self, Instruction::Pop);
        }

        Ok(())
    }

    fn compile_store(&mut self, target: &located_ast::Expr) -> CompileResult<()> {
        match &target {
            located_ast::Expr::Name(located_ast::ExprName { id, .. }) => {
                self.store_name(id.as_str())?
            }
            located_ast::Expr::Subscript(located_ast::ExprSubscript { value, slice, .. }) => {
                self.compile_expression(value)?;
                self.compile_expression(slice)?;
                emit!(self, Instruction::StoreSubscript);
            }
            located_ast::Expr::Attribute(located_ast::ExprAttribute { value, attr, .. }) => {
                self.check_forbidden_name(attr.as_str(), NameUsage::Store)?;
                self.compile_expression(value)?;
                let idx = self.name(attr.as_str());
                emit!(self, Instruction::StoreAttr { idx });
            }
            located_ast::Expr::List(located_ast::ExprList { elts, .. })
            | located_ast::Expr::Tuple(located_ast::ExprTuple { elts, .. }) => {
                let mut seen_star = false;

                // Scan for star args:
                for (i, element) in elts.iter().enumerate() {
                    if let located_ast::Expr::Starred(_) = &element {
                        if seen_star {
                            return Err(self.error(CodegenErrorType::MultipleStarArgs));
                        } else {
                            seen_star = true;
                            let before = i;
                            let after = elts.len() - i - 1;
                            let (before, after) = (|| Some((before.to_u8()?, after.to_u8()?)))()
                                .ok_or_else(|| {
                                    self.error_loc(
                                        CodegenErrorType::TooManyStarUnpack,
                                        target.location(),
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
                    if let located_ast::Expr::Starred(located_ast::ExprStarred { value, .. }) =
                        &element
                    {
                        self.compile_store(value)?;
                    } else {
                        self.compile_store(element)?;
                    }
                }
            }
            _ => {
                return Err(self.error(match target {
                    located_ast::Expr::Starred(_) => CodegenErrorType::SyntaxError(
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
        target: &located_ast::Expr,
        op: &located_ast::Operator,
        value: &located_ast::Expr,
    ) -> CompileResult<()> {
        enum AugAssignKind<'a> {
            Name { id: &'a str },
            Subscript,
            Attr { idx: bytecode::NameIdx },
        }

        let kind = match &target {
            located_ast::Expr::Name(located_ast::ExprName { id, .. }) => {
                let id = id.as_str();
                self.compile_name(id, NameUsage::Load)?;
                AugAssignKind::Name { id }
            }
            located_ast::Expr::Subscript(located_ast::ExprSubscript { value, slice, .. }) => {
                self.compile_expression(value)?;
                self.compile_expression(slice)?;
                emit!(self, Instruction::Duplicate2);
                emit!(self, Instruction::Subscript);
                AugAssignKind::Subscript
            }
            located_ast::Expr::Attribute(located_ast::ExprAttribute { value, attr, .. }) => {
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

    fn compile_op(&mut self, op: &located_ast::Operator, inplace: bool) {
        let op = match op {
            located_ast::Operator::Add => bytecode::BinaryOperator::Add,
            located_ast::Operator::Sub => bytecode::BinaryOperator::Subtract,
            located_ast::Operator::Mult => bytecode::BinaryOperator::Multiply,
            located_ast::Operator::MatMult => bytecode::BinaryOperator::MatrixMultiply,
            located_ast::Operator::Div => bytecode::BinaryOperator::Divide,
            located_ast::Operator::FloorDiv => bytecode::BinaryOperator::FloorDivide,
            located_ast::Operator::Mod => bytecode::BinaryOperator::Modulo,
            located_ast::Operator::Pow => bytecode::BinaryOperator::Power,
            located_ast::Operator::LShift => bytecode::BinaryOperator::Lshift,
            located_ast::Operator::RShift => bytecode::BinaryOperator::Rshift,
            located_ast::Operator::BitOr => bytecode::BinaryOperator::Or,
            located_ast::Operator::BitXor => bytecode::BinaryOperator::Xor,
            located_ast::Operator::BitAnd => bytecode::BinaryOperator::And,
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
        expression: &located_ast::Expr,
        condition: bool,
        target_block: ir::BlockIdx,
    ) -> CompileResult<()> {
        // Compile expression for test, and jump to label if false
        match &expression {
            located_ast::Expr::BoolOp(located_ast::ExprBoolOp { op, values, .. }) => {
                match op {
                    located_ast::Boolop::And => {
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
                    located_ast::Boolop::Or => {
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
            located_ast::Expr::UnaryOp(located_ast::ExprUnaryOp {
                op: located_ast::Unaryop::Not,
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
    fn compile_bool_op(
        &mut self,
        op: &located_ast::Boolop,
        values: &[located_ast::Expr],
    ) -> CompileResult<()> {
        let after_block = self.new_block();

        let (last_value, values) = values.split_last().unwrap();
        for value in values {
            self.compile_expression(value)?;

            match op {
                located_ast::Boolop::And => {
                    emit!(
                        self,
                        Instruction::JumpIfFalseOrPop {
                            target: after_block,
                        }
                    );
                }
                located_ast::Boolop::Or => {
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

    fn compile_dict(
        &mut self,
        keys: &[Option<located_ast::Expr>],
        values: &[located_ast::Expr],
    ) -> CompileResult<()> {
        let mut size = 0;
        let (packed, unpacked): (Vec<_>, Vec<_>) = keys
            .iter()
            .zip(values.iter())
            .partition(|(k, _)| k.is_some());
        for (key, value) in packed {
            self.compile_expression(key.as_ref().unwrap())?;
            self.compile_expression(value)?;
            size += 1;
        }
        emit!(self, Instruction::BuildMap { size });

        for (_, value) in unpacked {
            self.compile_expression(value)?;
            emit!(self, Instruction::DictUpdate);
        }

        Ok(())
    }

    fn compile_expression(&mut self, expression: &located_ast::Expr) -> CompileResult<()> {
        use located_ast::*;
        trace!("Compiling {:?}", expression);
        let location = expression.location();
        self.set_source_location(location);

        match &expression {
            Expr::Call(ExprCall {
                func,
                args,
                keywords,
                ..
            }) => self.compile_call(func, args, keywords)?,
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
                    Unaryop::UAdd => bytecode::UnaryOperator::Plus,
                    Unaryop::USub => bytecode::UnaryOperator::Minus,
                    Unaryop::Not => bytecode::UnaryOperator::Not,
                    Unaryop::Invert => bytecode::UnaryOperator::Invert,
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
            Expr::Constant(ExprConstant { value, .. }) => {
                self.emit_constant(compile_constant(value));
            }
            Expr::List(ExprList { elts, .. }) => {
                let (size, unpack) = self.gather_elements(0, elts)?;
                if unpack {
                    emit!(self, Instruction::BuildListUnpack { size });
                } else {
                    emit!(self, Instruction::BuildList { size });
                }
            }
            Expr::Tuple(ExprTuple { elts, .. }) => {
                let (size, unpack) = self.gather_elements(0, elts)?;
                if unpack {
                    emit!(self, Instruction::BuildTupleUnpack { size });
                } else {
                    emit!(self, Instruction::BuildTuple { size });
                }
            }
            Expr::Set(ExprSet { elts, .. }) => {
                let (size, unpack) = self.gather_elements(0, elts)?;
                if unpack {
                    emit!(self, Instruction::BuildSetUnpack { size });
                } else {
                    emit!(self, Instruction::BuildSet { size });
                }
            }
            Expr::Dict(ExprDict { keys, values, .. }) => {
                self.compile_dict(keys, values)?;
            }
            Expr::Slice(ExprSlice {
                lower, upper, step, ..
            }) => {
                let mut compile_bound = |bound: Option<&located_ast::Expr>| match bound {
                    Some(exp) => self.compile_expression(exp),
                    None => {
                        self.emit_constant(ConstantData::None);
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
                    Option::None => self.emit_constant(ConstantData::None),
                };
                emit!(self, Instruction::YieldValue);
            }
            Expr::Await(ExprAwait { value, .. }) => {
                if self.ctx.func != FunctionContext::AsyncFunction {
                    return Err(self.error(CodegenErrorType::InvalidAwait));
                }
                self.compile_expression(value)?;
                emit!(self, Instruction::GetAwaitable);
                self.emit_constant(ConstantData::None);
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
                self.emit_constant(ConstantData::None);
                emit!(self, Instruction::YieldFrom);
            }
            Expr::JoinedStr(ExprJoinedStr { values, .. }) => {
                if let Some(value) = try_get_constant_string(values) {
                    self.emit_constant(ConstantData::Str { value })
                } else {
                    for value in values {
                        self.compile_expression(value)?;
                    }
                    emit!(
                        self,
                        Instruction::BuildString {
                            size: values.len().to_u32(),
                        }
                    )
                }
            }
            Expr::FormattedValue(ExprFormattedValue {
                value,
                conversion,
                format_spec,
                ..
            }) => {
                match format_spec {
                    Some(spec) => self.compile_expression(spec)?,
                    None => self.emit_constant(ConstantData::Str {
                        value: String::new(),
                    }),
                };
                self.compile_expression(value)?;
                emit!(
                    self,
                    Instruction::FormatValue {
                        conversion: *conversion,
                    },
                );
            }
            Expr::Name(located_ast::ExprName { id, .. }) => self.load_name(id.as_str())?,
            Expr::Lambda(located_ast::ExprLambda { args, body, .. }) => {
                let prev_ctx = self.ctx;

                let name = "<lambda>".to_owned();
                let mut func_flags = self.enter_function(&name, args)?;

                self.ctx = CompileContext {
                    loop_data: Option::None,
                    in_class: prev_ctx.in_class,
                    func: FunctionContext::Function,
                };

                self.current_code_info()
                    .constants
                    .insert_full(ConstantData::None);

                self.compile_expression(body)?;
                emit!(self, Instruction::ReturnValue);
                let code = self.pop_code_object();
                if self.build_closure(&code) {
                    func_flags |= bytecode::MakeFunctionFlags::CLOSURE;
                }
                self.emit_constant(ConstantData::Code {
                    code: Box::new(code),
                });
                self.emit_constant(ConstantData::Str { value: name });
                // Turn code object into function object:
                emit!(self, Instruction::MakeFunction(func_flags));

                self.ctx = prev_ctx;
            }
            Expr::ListComp(located_ast::ExprListComp {
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
                )?;
            }
            Expr::SetComp(located_ast::ExprSetComp {
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
                )?;
            }
            Expr::DictComp(located_ast::ExprDictComp {
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
                )?;
            }
            Expr::GeneratorExp(located_ast::ExprGeneratorExp {
                elt, generators, ..
            }) => {
                self.compile_comprehension("<genexpr>", None, generators, &|compiler| {
                    compiler.compile_comprehension_element(elt)?;
                    compiler.mark_generator();
                    emit!(compiler, Instruction::YieldValue);
                    emit!(compiler, Instruction::Pop);

                    Ok(())
                })?;
            }
            Expr::Starred(_) => {
                return Err(self.error(CodegenErrorType::InvalidStarExpr));
            }
            Expr::IfExp(located_ast::ExprIfExp {
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

            Expr::NamedExpr(located_ast::ExprNamedExpr {
                target,
                value,
                range: _,
            }) => {
                self.compile_expression(value)?;
                emit!(self, Instruction::Duplicate);
                self.compile_store(target)?;
            }
        }
        Ok(())
    }

    fn compile_keywords(&mut self, keywords: &[located_ast::Keyword]) -> CompileResult<()> {
        let mut size = 0;
        let groupby = keywords.iter().group_by(|e| e.arg.is_none());
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
                        self.emit_constant(ConstantData::Str {
                            value: name.to_string(),
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

    fn compile_call(
        &mut self,
        func: &located_ast::Expr,
        args: &[located_ast::Expr],
        keywords: &[located_ast::Keyword],
    ) -> CompileResult<()> {
        let method =
            if let located_ast::Expr::Attribute(located_ast::ExprAttribute {
                value, attr, ..
            }) = &func
            {
                self.compile_expression(value)?;
                let idx = self.name(attr.as_str());
                emit!(self, Instruction::LoadMethod { idx });
                true
            } else {
                self.compile_expression(func)?;
                false
            };
        let call = self.compile_call_inner(0, args, keywords)?;
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
        args: &[located_ast::Expr],
        keywords: &[located_ast::Keyword],
    ) -> CompileResult<CallType> {
        let count = (args.len() + keywords.len()).to_u32() + additional_positional;

        // Normal arguments:
        let (size, unpack) = self.gather_elements(additional_positional, args)?;
        let has_double_star = keywords.iter().any(|k| k.arg.is_none());

        for keyword in keywords {
            if let Some(name) = &keyword.arg {
                self.check_forbidden_name(name.as_str(), NameUsage::Store)?;
            }
        }

        let call = if unpack || has_double_star {
            // Create a tuple with positional args:
            if unpack {
                emit!(self, Instruction::BuildTupleUnpack { size });
            } else {
                emit!(self, Instruction::BuildTuple { size });
            }

            // Create an optional map with kw-args:
            let has_kwargs = !keywords.is_empty();
            if has_kwargs {
                self.compile_keywords(keywords)?;
            }
            CallType::Ex { has_kwargs }
        } else if !keywords.is_empty() {
            let mut kwarg_names = vec![];
            for keyword in keywords {
                if let Some(name) = &keyword.arg {
                    kwarg_names.push(ConstantData::Str {
                        value: name.to_string(),
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
            CallType::Keyword { nargs: count }
        } else {
            CallType::Positional { nargs: count }
        };

        Ok(call)
    }

    // Given a vector of expr / star expr generate code which gives either
    // a list of expressions on the stack, or a list of tuples.
    fn gather_elements(
        &mut self,
        before: u32,
        elements: &[located_ast::Expr],
    ) -> CompileResult<(u32, bool)> {
        // First determine if we have starred elements:
        let has_stars = elements
            .iter()
            .any(|e| matches!(e, located_ast::Expr::Starred(_)));

        let size = if has_stars {
            let mut size = 0;

            if before > 0 {
                emit!(self, Instruction::BuildTuple { size: before });
                size += 1;
            }

            let groups = elements
                .iter()
                .map(|element| {
                    if let located_ast::Expr::Starred(located_ast::ExprStarred { value, .. }) =
                        &element
                    {
                        (true, value.as_ref())
                    } else {
                        (false, element)
                    }
                })
                .group_by(|(starred, _)| *starred);

            for (starred, run) in &groups {
                let mut run_size = 0;
                for (_, value) in run {
                    self.compile_expression(value)?;
                    run_size += 1
                }
                if starred {
                    size += run_size
                } else {
                    emit!(self, Instruction::BuildTuple { size: run_size });
                    size += 1
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

    fn compile_comprehension_element(&mut self, element: &located_ast::Expr) -> CompileResult<()> {
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
        generators: &[located_ast::Comprehension],
        compile_element: &dyn Fn(&mut Self) -> CompileResult<()>,
    ) -> CompileResult<()> {
        let prev_ctx = self.ctx;

        self.ctx = CompileContext {
            loop_data: None,
            in_class: prev_ctx.in_class,
            func: FunctionContext::Function,
        };

        // We must have at least one generator:
        assert!(!generators.is_empty());

        // Create magnificent function <listcomp>:
        self.push_output(
            bytecode::CodeFlags::NEW_LOCALS | bytecode::CodeFlags::IS_OPTIMIZED,
            1,
            1,
            0,
            name.to_owned(),
        );
        let arg0 = self.varname(".0")?;

        let return_none = init_collection.is_none();
        // Create empty object of proper type:
        if let Some(init_collection) = init_collection {
            self._emit(init_collection, OpArg(0), ir::BlockIdx::NULL)
        }

        let mut loop_labels = vec![];
        for generator in generators {
            if generator.is_async {
                unimplemented!("async for comprehensions");
            }

            let loop_block = self.new_block();
            let after_block = self.new_block();

            if loop_labels.is_empty() {
                // Load iterator onto stack (passed as first argument):
                emit!(self, Instruction::LoadFast(arg0));
            } else {
                // Evaluate iterated item:
                self.compile_expression(&generator.iter)?;

                // Get iterator / turn item into an iterator
                emit!(self, Instruction::GetIter);
            }

            loop_labels.push((loop_block, after_block));

            self.switch_to_block(loop_block);
            emit!(
                self,
                Instruction::ForIter {
                    target: after_block,
                }
            );

            self.compile_store(&generator.target)?;

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
        }

        if return_none {
            self.emit_constant(ConstantData::None)
        }

        // Return freshly filled list:
        emit!(self, Instruction::ReturnValue);

        // Fetch code for listcomp function:
        let code = self.pop_code_object();

        self.ctx = prev_ctx;

        let mut func_flags = bytecode::MakeFunctionFlags::empty();
        if self.build_closure(&code) {
            func_flags |= bytecode::MakeFunctionFlags::CLOSURE;
        }

        // List comprehension code:
        self.emit_constant(ConstantData::Code {
            code: Box::new(code),
        });

        // List comprehension function name:
        self.emit_constant(ConstantData::Str {
            value: name.to_owned(),
        });

        // Turn code object into function object:
        emit!(self, Instruction::MakeFunction(func_flags));

        // Evaluate iterated item:
        self.compile_expression(&generators[0].iter)?;

        // Get iterator / turn item into an iterator
        emit!(self, Instruction::GetIter);

        // Call just created <listcomp> function:
        emit!(self, Instruction::CallFunctionPositional { nargs: 1 });
        Ok(())
    }

    fn compile_future_features(
        &mut self,
        features: &[located_ast::Alias],
    ) -> Result<(), CodegenError> {
        if self.done_with_future_stmts {
            return Err(self.error(CodegenErrorType::InvalidFuturePlacement));
        }
        for feature in features {
            match feature.name.as_str() {
                // Python 3 features; we've already implemented them by default
                "nested_scopes" | "generators" | "division" | "absolute_import"
                | "with_statement" | "print_function" | "unicode_literals" => {}
                // "generator_stop" => {}
                "annotations" => self.future_annotations = true,
                other => {
                    return Err(self.error(CodegenErrorType::InvalidFutureFeature(other.to_owned())))
                }
            }
        }
        Ok(())
    }

    // Low level helper functions:
    fn _emit(&mut self, instr: Instruction, arg: OpArg, target: ir::BlockIdx) {
        let location = self.current_source_location;
        // TODO: insert source filename
        self.current_block().instructions.push(ir::InstructionInfo {
            instr,
            arg,
            target,
            location,
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

    fn emit_constant(&mut self, constant: ConstantData) {
        let info = self.current_code_info();
        let idx = info.constants.insert_full(constant).0.to_u32();
        self.emit_arg(idx, |idx| Instruction::LoadConst { idx })
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
        assert_eq!(
            code.blocks[block].next,
            ir::BlockIdx::NULL,
            "switching to completed block"
        );
        let prev_block = &mut code.blocks[prev.0 as usize];
        assert_eq!(
            prev_block.next.0,
            u32::MAX,
            "switching from block that's already got a next"
        );
        prev_block.next = block;
        code.current_block = block;
    }

    fn set_source_location(&mut self, location: SourceLocation) {
        self.current_source_location = location;
    }

    fn get_source_line_number(&mut self) -> LineNumber {
        let location = self.current_source_location;
        location.row
    }

    fn push_qualified_path(&mut self, name: &str) {
        self.qualified_path.push(name.to_owned());
    }

    fn mark_generator(&mut self) {
        self.current_code_info().flags |= bytecode::CodeFlags::IS_GENERATOR
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

fn split_doc(body: &[located_ast::Stmt]) -> (Option<String>, &[located_ast::Stmt]) {
    if let Some((located_ast::Stmt::Expr(expr), body_rest)) = body.split_first() {
        if let Some(doc) = try_get_constant_string(std::slice::from_ref(&expr.value)) {
            return (Some(doc), body_rest);
        }
    }
    (None, body)
}

fn try_get_constant_string(values: &[located_ast::Expr]) -> Option<String> {
    fn get_constant_string_inner(out_string: &mut String, value: &located_ast::Expr) -> bool {
        match value {
            located_ast::Expr::Constant(located_ast::ExprConstant {
                value: located_ast::Constant::Str(s),
                ..
            }) => {
                out_string.push_str(s);
                true
            }
            located_ast::Expr::JoinedStr(located_ast::ExprJoinedStr { values, .. }) => values
                .iter()
                .all(|value| get_constant_string_inner(out_string, value)),
            _ => false,
        }
    }
    let mut out_string = String::new();
    if values
        .iter()
        .all(|v| get_constant_string_inner(&mut out_string, v))
    {
        Some(out_string)
    } else {
        None
    }
}

fn compile_constant(value: &located_ast::Constant) -> ConstantData {
    match value {
        located_ast::Constant::None => ConstantData::None,
        located_ast::Constant::Bool(b) => ConstantData::Boolean { value: *b },
        located_ast::Constant::Str(s) => ConstantData::Str { value: s.clone() },
        located_ast::Constant::Bytes(b) => ConstantData::Bytes { value: b.clone() },
        located_ast::Constant::Int(i) => ConstantData::Integer { value: i.clone() },
        located_ast::Constant::Tuple(t) => ConstantData::Tuple {
            elements: t.iter().map(compile_constant).collect(),
        },
        located_ast::Constant::Float(f) => ConstantData::Float { value: *f },
        located_ast::Constant::Complex { real, imag } => ConstantData::Complex {
            value: Complex64::new(*real, *imag),
        },
        located_ast::Constant::Ellipsis => ConstantData::Ellipsis,
    }
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
mod tests {
    use super::*;
    use rustpython_parser as parser;
    use rustpython_parser_core::source_code::SourceLocator;

    fn compile_exec(source: &str) -> CodeObject {
        let mut locator: SourceLocator = SourceLocator::new(source);
        use rustpython_parser::ast::fold::Fold;
        let mut compiler: Compiler = Compiler::new(
            CompileOpts::default(),
            "source_path".to_owned(),
            "<module>".to_owned(),
        );
        let ast = parser::parse_program(source, "<test>").unwrap();
        let ast = locator.fold(ast).unwrap();
        let symbol_scope = SymbolTable::scan_program(&ast).unwrap();
        compiler.compile_program(&ast, symbol_scope).unwrap();
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
}
