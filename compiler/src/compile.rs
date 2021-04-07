//!
//! Take an AST and transform it into bytecode
//!
//! Inspirational code:
//!   https://github.com/python/cpython/blob/master/Python/compile.c
//!   https://github.com/micropython/micropython/blob/master/py/compile.c

use crate::ir::{self, CodeInfo};
pub use crate::mode::Mode;
use crate::symboltable::{make_symbol_table, make_symbol_table_expr, SymbolScope, SymbolTable};
use crate::IndexSet;
use crate::{
    error::{CompileError, CompileErrorType},
    symboltable,
};
use itertools::Itertools;
use num_complex::Complex64;
use num_traits::ToPrimitive;
use rustpython_ast as ast;
use rustpython_bytecode::{self as bytecode, CodeObject, ConstantData, Instruction};
use std::borrow::Cow;

type CompileResult<T> = Result<T, CompileError>;

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
impl CallType {
    fn normal_call(self) -> Instruction {
        match self {
            CallType::Positional { nargs } => Instruction::CallFunctionPositional { nargs },
            CallType::Keyword { nargs } => Instruction::CallFunctionKeyword { nargs },
            CallType::Ex { has_kwargs } => Instruction::CallFunctionEx { has_kwargs },
        }
    }
    fn method_call(self) -> Instruction {
        match self {
            CallType::Positional { nargs } => Instruction::CallMethodPositional { nargs },
            CallType::Keyword { nargs } => Instruction::CallMethodKeyword { nargs },
            CallType::Ex { has_kwargs } => Instruction::CallMethodEx { has_kwargs },
        }
    }
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
    class_name: Option<String>,
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

/// Compile an ast::Mod produced from rustpython_parser::parser::parse()
pub fn compile_top(
    ast: &ast::Mod,
    source_path: String,
    opts: CompileOpts,
) -> CompileResult<CodeObject> {
    match ast {
        ast::Mod::Module { body, .. } => compile_program(body, source_path, opts),
        ast::Mod::Interactive { body } => compile_program_single(body, source_path, opts),
        ast::Mod::Expression { body } => compile_expression(body, source_path, opts),
        ast::Mod::FunctionType { .. } => panic!("can't compile a FunctionType"),
    }
}

macro_rules! compile_impl {
    ($ast:expr, $source_path:expr, $opts:expr, $st:ident, $compile:ident) => {{
        let symbol_table = match $st($ast) {
            Ok(x) => x,
            Err(e) => return Err(e.into_compile_error($source_path)),
        };
        with_compiler($source_path, $opts, |compiler| {
            compiler.$compile($ast, symbol_table)
        })
    }};
}

/// Compile a standard Python program to bytecode
pub fn compile_program(
    ast: &[ast::Stmt],
    source_path: String,
    opts: CompileOpts,
) -> CompileResult<CodeObject> {
    compile_impl!(ast, source_path, opts, make_symbol_table, compile_program)
}

/// Compile a Python program to bytecode for the context of a REPL
pub fn compile_program_single(
    ast: &[ast::Stmt],
    source_path: String,
    opts: CompileOpts,
) -> CompileResult<CodeObject> {
    compile_impl!(
        ast,
        source_path,
        opts,
        make_symbol_table,
        compile_program_single
    )
}

pub fn compile_expression(
    ast: &ast::Expr,
    source_path: String,
    opts: CompileOpts,
) -> CompileResult<CodeObject> {
    compile_impl!(ast, source_path, opts, make_symbol_table_expr, compile_eval)
}

impl Compiler {
    fn new(opts: CompileOpts, source_path: String, code_name: String) -> Self {
        let module_code = CodeInfo {
            flags: bytecode::CodeFlags::NEW_LOCALS,
            posonlyarg_count: 0,
            arg_count: 0,
            kwonlyarg_count: 0,
            source_path: source_path.clone(),
            first_line_number: 0,
            obj_name: code_name,

            blocks: vec![ir::Block::default()],
            current_block: bytecode::Label(0),
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
            current_source_location: ast::Location::default(),
            current_qualified_path: None,
            done_with_future_stmts: false,
            ctx: CompileContext {
                loop_data: None,
                in_class: false,
                func: FunctionContext::NoFunction,
            },
            class_name: None,
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

    fn push_output(
        &mut self,
        flags: bytecode::CodeFlags,
        posonlyarg_count: usize,
        arg_count: usize,
        kwonlyarg_count: usize,
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
            .filter(|(_, s)| s.scope == SymbolScope::Free || s.is_free_class)
            .map(|(var, _)| var.clone())
            .collect();

        self.symbol_table_stack.push(table);

        let info = CodeInfo {
            flags,
            posonlyarg_count,
            arg_count,
            kwonlyarg_count,
            source_path,
            first_line_number,
            obj_name,

            blocks: vec![ir::Block::default()],
            current_block: bytecode::Label(0),
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
    fn varname(&mut self, name: &str) -> bytecode::NameIdx {
        self._name_inner(name, |i| &mut i.varname_cache)
    }
    fn _name_inner(
        &mut self,
        name: &str,
        cache: impl FnOnce(&mut CodeInfo) -> &mut IndexSet<String>,
    ) -> bytecode::NameIdx {
        let name = self.mangle(name);
        let cache = cache(self.current_codeinfo());
        cache
            .get_index_of(name.as_ref())
            .unwrap_or_else(|| cache.insert_full(name.into_owned()).0) as u32
    }

    fn compile_program(
        &mut self,
        body: &[ast::Stmt],
        symbol_table: SymbolTable,
    ) -> CompileResult<()> {
        let size_before = self.code_stack.len();
        self.symbol_table_stack.push(symbol_table);

        let (statements, doc) = get_doc(body);
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
        body: &[ast::Stmt],
        symbol_table: SymbolTable,
    ) -> CompileResult<()> {
        self.symbol_table_stack.push(symbol_table);

        let mut emitted_return = false;

        for (i, statement) in body.iter().enumerate() {
            let is_last = i == body.len() - 1;

            if let ast::StmtKind::Expr { value } = &statement.node {
                self.compile_expression(value)?;

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
    fn compile_eval(
        &mut self,
        expression: &ast::Expr,
        symbol_table: SymbolTable,
    ) -> CompileResult<()> {
        self.symbol_table_stack.push(symbol_table);
        self.compile_expression(expression)?;
        self.emit(Instruction::ReturnValue);
        Ok(())
    }

    fn compile_statements(&mut self, statements: &[ast::Stmt]) -> CompileResult<()> {
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

    fn mangle<'a>(&self, name: &'a str) -> Cow<'a, str> {
        symboltable::mangle_name(self.class_name.as_deref(), name)
    }

    fn compile_name(&mut self, name: &str, usage: NameUsage) {
        let name = self.mangle(name);
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
        self.emit(op(idx as u32));
    }

    fn compile_statement(&mut self, statement: &ast::Stmt) -> CompileResult<()> {
        trace!("Compiling {:?}", statement);
        self.set_source_location(statement.location);
        use ast::StmtKind::*;

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
                    let idx = self.name(&name.name);
                    self.emit(Instruction::ImportName { idx });
                    if let Some(alias) = &name.asname {
                        for part in name.name.split('.').skip(1) {
                            let idx = self.name(part);
                            self.emit(Instruction::LoadAttr { idx });
                        }
                        self.store_name(alias);
                    } else {
                        self.store_name(name.name.split('.').next().unwrap());
                    }
                }
            }
            ImportFrom {
                level,
                module,
                names,
            } => {
                let import_star = names.iter().any(|n| n.name == "*");

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
                            value: n.name.to_owned(),
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
                        let idx = self.name(&name.name);
                        // import symbol from module:
                        self.emit(Instruction::ImportFrom { idx });

                        // Store module under proper name:
                        if let Some(alias) = &name.asname {
                            self.store_name(alias);
                        } else {
                            self.store_name(&name.name);
                        }
                    }

                    // Pop module from stack:
                    self.emit(Instruction::Pop);
                }
            }
            Expr { value } => {
                self.compile_expression(value)?;

                // Pop result of stack, since we not use it:
                self.emit(Instruction::Pop);
            }
            Global { .. } | Nonlocal { .. } => {
                // Handled during symbol table construction.
            }
            If { test, body, orelse } => {
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
                    self.emit(Instruction::Jump {
                        target: after_block,
                    });

                    // else:
                    self.switch_to_block(else_block);
                    self.compile_statements(orelse)?;
                }
                self.switch_to_block(after_block);
            }
            While { test, body, orelse } => self.compile_while(test, body, orelse)?,
            With { items, body, .. } => self.compile_with(items, body, false)?,
            AsyncWith { items, body, .. } => self.compile_with(items, body, true)?,
            For {
                target,
                iter,
                body,
                orelse,
                ..
            } => self.compile_for(target, iter, body, orelse, false)?,
            AsyncFor {
                target,
                iter,
                body,
                orelse,
                ..
            } => self.compile_for(target, iter, body, orelse, true)?,
            Raise { exc, cause } => {
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
                self.emit(Instruction::Raise { kind });
            }
            Try {
                body,
                handlers,
                orelse,
                finalbody,
            } => self.compile_try_statement(body, handlers, orelse, finalbody)?,
            FunctionDef {
                name,
                args,
                body,
                decorator_list,
                returns,
                ..
            } => self.compile_function_def(
                name,
                args,
                body,
                decorator_list,
                returns.as_deref(),
                false,
            )?,
            AsyncFunctionDef {
                name,
                args,
                body,
                decorator_list,
                returns,
                ..
            } => self.compile_function_def(
                name,
                args,
                body,
                decorator_list,
                returns.as_deref(),
                true,
            )?,
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
                    let after_block = self.new_block();
                    self.compile_jump_if(test, true, after_block)?;

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
                    self.emit(Instruction::Raise {
                        kind: bytecode::RaiseKind::Raise,
                    });

                    self.switch_to_block(after_block);
                }
            }
            Break => {
                if self.ctx.loop_data.is_some() {
                    self.emit(Instruction::Break);
                } else {
                    return Err(self.error_loc(CompileErrorType::InvalidBreak, statement.location));
                }
            }
            Continue => match self.ctx.loop_data {
                Some((start, _)) => {
                    self.emit(Instruction::Continue { target: start });
                }
                None => {
                    return Err(
                        self.error_loc(CompileErrorType::InvalidContinue, statement.location)
                    );
                }
            },
            Return { value } => {
                if !self.ctx.in_func() {
                    return Err(self.error_loc(CompileErrorType::InvalidReturn, statement.location));
                }
                match value {
                    Some(v) => {
                        if self.ctx.func == FunctionContext::AsyncFunction
                            && self
                                .current_codeinfo()
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
            Assign { targets, value, .. } => {
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
                ..
            } => self.compile_annotated_assign(target, annotation, value.as_deref())?,
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

    fn compile_delete(&mut self, expression: &ast::Expr) -> CompileResult<()> {
        match &expression.node {
            ast::ExprKind::Name { id, .. } => {
                self.compile_name(id, NameUsage::Delete);
            }
            ast::ExprKind::Attribute { value, attr, .. } => {
                self.compile_expression(value)?;
                let idx = self.name(attr);
                self.emit(Instruction::DeleteAttr { idx });
            }
            ast::ExprKind::Subscript { value, slice, .. } => {
                self.compile_expression(value)?;
                self.compile_expression(slice)?;
                self.emit(Instruction::DeleteSubscript);
            }
            ast::ExprKind::Tuple { elts, .. } => {
                for element in elts {
                    self.compile_delete(element)?;
                }
            }
            _ => return Err(self.error(CompileErrorType::Delete(expression.node.name()))),
        }
        Ok(())
    }

    fn enter_function(
        &mut self,
        name: &str,
        args: &ast::Arguments,
    ) -> CompileResult<bytecode::MakeFunctionFlags> {
        let have_defaults = !args.defaults.is_empty();
        if have_defaults {
            // Construct a tuple:
            let size = args.defaults.len() as u32;
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
                    value: kw.node.arg.clone(),
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

        let mut funcflags = bytecode::MakeFunctionFlags::empty();
        if have_defaults {
            funcflags |= bytecode::MakeFunctionFlags::DEFAULTS;
        }
        if num_kw_only_defaults > 0 {
            funcflags |= bytecode::MakeFunctionFlags::KW_ONLY_DEFAULTS;
        }

        self.push_output(
            bytecode::CodeFlags::NEW_LOCALS | bytecode::CodeFlags::IS_OPTIMIZED,
            args.posonlyargs.len(),
            args.posonlyargs.len() + args.args.len(),
            args.kwonlyargs.len(),
            name.to_owned(),
        );

        let args_iter = std::iter::empty()
            .chain(&args.posonlyargs)
            .chain(&args.args)
            .chain(&args.kwonlyargs);
        for name in args_iter {
            self.varname(&name.node.arg);
        }

        let mut compile_varargs = |va: Option<&ast::Arg>, flag| {
            if let Some(name) = va {
                self.current_codeinfo().flags |= flag;
                self.varname(&name.node.arg);
            }
        };

        compile_varargs(args.vararg.as_deref(), bytecode::CodeFlags::HAS_VARARGS);
        compile_varargs(args.kwarg.as_deref(), bytecode::CodeFlags::HAS_VARKEYWORDS);

        Ok(funcflags)
    }

    fn prepare_decorators(&mut self, decorator_list: &[ast::Expr]) -> CompileResult<()> {
        for decorator in decorator_list {
            self.compile_expression(decorator)?;
        }
        Ok(())
    }

    fn apply_decorators(&mut self, decorator_list: &[ast::Expr]) {
        // Apply decorators:
        for _ in decorator_list {
            self.emit(Instruction::CallFunctionPositional { nargs: 1 });
        }
    }

    fn compile_try_statement(
        &mut self,
        body: &[ast::Stmt],
        handlers: &[ast::Excepthandler],
        orelse: &[ast::Stmt],
        finalbody: &[ast::Stmt],
    ) -> CompileResult<()> {
        let handler_block = self.new_block();
        let finally_block = self.new_block();

        // Setup a finally block if we have a finally statement.
        if !finalbody.is_empty() {
            self.emit(Instruction::SetupFinally {
                handler: finally_block,
            });
        }

        let else_block = self.new_block();

        // try:
        self.emit(Instruction::SetupExcept {
            handler: handler_block,
        });
        self.compile_statements(body)?;
        self.emit(Instruction::PopBlock);
        self.emit(Instruction::Jump { target: else_block });

        // except handlers:
        self.switch_to_block(handler_block);
        // Exception is on top of stack now
        for handler in handlers {
            let ast::ExcepthandlerKind::ExceptHandler { type_, name, body } = &handler.node;
            let next_handler = self.new_block();

            // If we gave a typ,
            // check if this handler can handle the exception:
            if let Some(exc_type) = type_ {
                // Duplicate exception for test:
                self.emit(Instruction::Duplicate);

                // Check exception type:
                self.compile_expression(exc_type)?;
                self.emit(Instruction::CompareOperation {
                    op: bytecode::ComparisonOperator::ExceptionMatch,
                });

                // We cannot handle this exception type:
                self.emit(Instruction::JumpIfFalse {
                    target: next_handler,
                });

                // We have a match, store in name (except x as y)
                if let Some(alias) = name {
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
            self.compile_statements(body)?;
            self.emit(Instruction::PopException);

            if !finalbody.is_empty() {
                self.emit(Instruction::PopBlock); // pop excepthandler block
                                                  // We enter the finally block, without exception.
                self.emit(Instruction::EnterFinally);
            }

            self.emit(Instruction::Jump {
                target: finally_block,
            });

            // Emit a new label for the next handler
            self.switch_to_block(next_handler);
        }

        // If code flows here, we have an unhandled exception,
        // raise the exception again!
        self.emit(Instruction::Raise {
            kind: bytecode::RaiseKind::Reraise,
        });

        // We successfully ran the try block:
        // else:
        self.switch_to_block(else_block);
        self.compile_statements(orelse)?;

        if !finalbody.is_empty() {
            self.emit(Instruction::PopBlock); // pop finally block

            // We enter the finallyhandler block, without return / exception.
            self.emit(Instruction::EnterFinally);
        }

        // finally:
        self.switch_to_block(finally_block);
        if !finalbody.is_empty() {
            self.compile_statements(finalbody)?;
            self.emit(Instruction::EndFinally);
        }

        Ok(())
    }

    fn compile_function_def(
        &mut self,
        name: &str,
        args: &ast::Arguments,
        body: &[ast::Stmt],
        decorator_list: &[ast::Expr],
        returns: Option<&ast::Expr>, // TODO: use type hint somehow..
        is_async: bool,
    ) -> CompileResult<()> {
        // Create bytecode for this function:

        self.prepare_decorators(decorator_list)?;
        let mut funcflags = self.enter_function(name, args)?;
        self.current_codeinfo()
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

        let qualified_name = self.create_qualified_name(name, "");
        let old_qualified_path = self.current_qualified_path.take();
        self.current_qualified_path = Some(self.create_qualified_name(name, ".<locals>"));

        let (body, doc_str) = get_doc(body);

        self.compile_statements(body)?;

        // Emit None at end:
        match body.last().map(|s| &s.node) {
            Some(ast::StmtKind::Return { .. }) => {
                // the last instruction is a ReturnValue already, we don't need to emit it
            }
            _ => {
                self.emit_constant(ConstantData::None);
                self.emit(Instruction::ReturnValue);
            }
        }

        let code = self.pop_code_object();
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

        let args_iter = std::iter::empty()
            .chain(&args.posonlyargs)
            .chain(&args.args)
            .chain(&args.kwonlyargs)
            .chain(args.vararg.as_deref())
            .chain(args.kwarg.as_deref());
        for arg in args_iter {
            if let Some(annotation) = &arg.node.annotation {
                self.emit_constant(ConstantData::Str {
                    value: self.mangle(&arg.node.arg).into_owned(),
                });
                self.compile_expression(&annotation)?;
                num_annotations += 1;
            }
        }

        if num_annotations > 0 {
            funcflags |= bytecode::MakeFunctionFlags::ANNOTATIONS;
            self.emit(Instruction::BuildMap {
                size: num_annotations,
                unpack: false,
                for_call: false,
            });
        }

        if self.build_closure(&code) {
            funcflags |= bytecode::MakeFunctionFlags::CLOSURE;
        }

        self.emit_constant(ConstantData::Code {
            code: Box::new(code),
        });
        self.emit_constant(ConstantData::Str {
            value: qualified_name,
        });

        // Turn code object into function object:
        self.emit(Instruction::MakeFunction(funcflags));

        self.emit(Instruction::Duplicate);
        self.load_docstring(doc_str);
        self.emit(Instruction::Rotate { amount: 2 });
        let doc = self.name("__doc__");
        self.emit(Instruction::StoreAttr { idx: doc });

        self.apply_decorators(decorator_list);

        self.store_name(name);

        Ok(())
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
            self.emit(Instruction::LoadClosure(idx as u32))
        }
        self.emit(Instruction::BuildTuple {
            size: code.freevars.len() as u32,
            unpack: false,
        });
        true
    }

    fn find_ann(&self, body: &[ast::Stmt]) -> bool {
        use ast::StmtKind::*;

        for statement in body {
            let res = match &statement.node {
                AnnAssign { .. } => true,
                For { body, orelse, .. } => self.find_ann(body) || self.find_ann(orelse),
                If { body, orelse, .. } => self.find_ann(body) || self.find_ann(orelse),
                While { body, orelse, .. } => self.find_ann(body) || self.find_ann(orelse),
                With { body, .. } => self.find_ann(body),
                Try {
                    body,
                    orelse,
                    finalbody,
                    ..
                } => self.find_ann(&body) || self.find_ann(orelse) || self.find_ann(finalbody),
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
        body: &[ast::Stmt],
        bases: &[ast::Expr],
        keywords: &[ast::Keyword],
        decorator_list: &[ast::Expr],
    ) -> CompileResult<()> {
        self.prepare_decorators(decorator_list)?;

        self.emit(Instruction::LoadBuildClass);

        let prev_ctx = self.ctx;
        self.ctx = CompileContext {
            func: FunctionContext::NoFunction,
            in_class: true,
            loop_data: None,
        };

        let prev_class_name = std::mem::replace(&mut self.class_name, Some(name.to_owned()));

        let qualified_name = self.create_qualified_name(name, "");
        let old_qualified_path = std::mem::replace(
            &mut self.current_qualified_path,
            Some(qualified_name.clone()),
        );

        self.push_output(bytecode::CodeFlags::empty(), 0, 0, 0, name.to_owned());

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
            self.emit(Instruction::LoadClosure(classcell_idx as u32));
            self.emit(Instruction::Duplicate);
            let classcell = self.name("__classcell__");
            self.emit(Instruction::StoreLocal(classcell));
        } else {
            self.emit_constant(ConstantData::None);
        }

        self.emit(Instruction::ReturnValue);

        let code = self.pop_code_object();

        self.class_name = prev_class_name;
        self.current_qualified_path = old_qualified_path;
        self.ctx = prev_ctx;

        let mut funcflags = bytecode::MakeFunctionFlags::empty();

        if self.build_closure(&code) {
            funcflags |= bytecode::MakeFunctionFlags::CLOSURE;
        }

        self.emit_constant(ConstantData::Code {
            code: Box::new(code),
        });
        self.emit_constant(ConstantData::Str {
            value: name.to_owned(),
        });

        // Turn code object into function object:
        self.emit(Instruction::MakeFunction(funcflags));

        self.emit_constant(ConstantData::Str {
            value: qualified_name,
        });

        let call = self.compile_call_inner(2, bases, keywords)?;
        self.emit(call.normal_call());

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
        test: &ast::Expr,
        body: &[ast::Stmt],
        orelse: &[ast::Stmt],
    ) -> CompileResult<()> {
        let while_block = self.new_block();
        let else_block = self.new_block();
        let after_block = self.new_block();

        self.emit(Instruction::SetupLoop {
            break_target: after_block,
        });
        self.switch_to_block(while_block);

        self.compile_jump_if(test, false, else_block)?;

        let was_in_loop = self.ctx.loop_data;
        self.ctx.loop_data = Some((while_block, after_block));
        self.compile_statements(body)?;
        self.ctx.loop_data = was_in_loop;
        self.emit(Instruction::Jump {
            target: while_block,
        });
        self.switch_to_block(else_block);
        self.emit(Instruction::PopBlock);
        self.compile_statements(orelse)?;
        self.switch_to_block(after_block);
        Ok(())
    }

    fn compile_with(
        &mut self,
        items: &[ast::Withitem],
        body: &[ast::Stmt],
        is_async: bool,
    ) -> CompileResult<()> {
        let end_blocks = items
            .iter()
            .map(|item| {
                let end_block = self.new_block();
                self.compile_expression(&item.context_expr)?;

                if is_async {
                    self.emit(Instruction::BeforeAsyncWith);
                    self.emit(Instruction::GetAwaitable);
                    self.emit_constant(ConstantData::None);
                    self.emit(Instruction::YieldFrom);
                    self.emit(Instruction::SetupAsyncWith { end: end_block });
                } else {
                    self.emit(Instruction::SetupWith { end: end_block });
                }

                match &item.optional_vars {
                    Some(var) => {
                        self.compile_store(var)?;
                    }
                    None => {
                        self.emit(Instruction::Pop);
                    }
                }
                Ok(end_block)
            })
            .collect::<CompileResult<Vec<_>>>()?;

        self.compile_statements(body)?;

        // sort of "stack up" the layers of with blocks:
        // with a, b: body -> start_with(a) start_with(b) body() end_with(b) end_with(a)
        for end_block in end_blocks.into_iter().rev() {
            self.emit(Instruction::PopBlock);
            self.emit(Instruction::EnterFinally);

            self.switch_to_block(end_block);
            self.emit(Instruction::WithCleanupStart);

            if is_async {
                self.emit(Instruction::GetAwaitable);
                self.emit_constant(ConstantData::None);
                self.emit(Instruction::YieldFrom);
            }

            self.emit(Instruction::WithCleanupFinish);
        }

        Ok(())
    }

    fn compile_for(
        &mut self,
        target: &ast::Expr,
        iter: &ast::Expr,
        body: &[ast::Stmt],
        orelse: &[ast::Stmt],
        is_async: bool,
    ) -> CompileResult<()> {
        // Start loop
        let for_block = self.new_block();
        let else_block = self.new_block();
        let after_block = self.new_block();

        self.emit(Instruction::SetupLoop {
            break_target: after_block,
        });

        // The thing iterated:
        self.compile_expression(iter)?;

        if is_async {
            self.emit(Instruction::GetAIter);

            self.switch_to_block(for_block);
            self.emit(Instruction::SetupExcept {
                handler: else_block,
            });
            self.emit(Instruction::GetANext);
            self.emit_constant(ConstantData::None);
            self.emit(Instruction::YieldFrom);
            self.compile_store(target)?;
            self.emit(Instruction::PopBlock);
        } else {
            // Retrieve Iterator
            self.emit(Instruction::GetIter);

            self.switch_to_block(for_block);
            self.emit(Instruction::ForIter { target: else_block });

            // Start of loop iteration, set targets:
            self.compile_store(target)?;
        };

        let was_in_loop = self.ctx.loop_data;
        self.ctx.loop_data = Some((for_block, after_block));
        self.compile_statements(body)?;
        self.ctx.loop_data = was_in_loop;
        self.emit(Instruction::Jump { target: for_block });

        self.switch_to_block(else_block);
        if is_async {
            self.emit(Instruction::EndAsyncFor);
        }
        self.emit(Instruction::PopBlock);
        self.compile_statements(orelse)?;

        self.switch_to_block(after_block);

        Ok(())
    }

    fn compile_chained_comparison(
        &mut self,
        left: &ast::Expr,
        ops: &[ast::Cmpop],
        vals: &[ast::Expr],
    ) -> CompileResult<()> {
        assert!(!ops.is_empty());
        assert_eq!(vals.len(), ops.len());
        let (last_op, mid_ops) = ops.split_last().unwrap();
        let (last_val, mid_vals) = vals.split_last().unwrap();

        let compile_cmpop = |op: &ast::Cmpop| match op {
            ast::Cmpop::Eq => bytecode::ComparisonOperator::Equal,
            ast::Cmpop::NotEq => bytecode::ComparisonOperator::NotEqual,
            ast::Cmpop::Lt => bytecode::ComparisonOperator::Less,
            ast::Cmpop::LtE => bytecode::ComparisonOperator::LessOrEqual,
            ast::Cmpop::Gt => bytecode::ComparisonOperator::Greater,
            ast::Cmpop::GtE => bytecode::ComparisonOperator::GreaterOrEqual,
            ast::Cmpop::In => bytecode::ComparisonOperator::In,
            ast::Cmpop::NotIn => bytecode::ComparisonOperator::NotIn,
            ast::Cmpop::Is => bytecode::ComparisonOperator::Is,
            ast::Cmpop::IsNot => bytecode::ComparisonOperator::IsNot,
        };

        // a == b == c == d
        // compile into (pseudocode):
        // result = a == b
        // if result:
        //   result = b == c
        //   if result:
        //     result = c == d

        // initialize lhs outside of loop
        self.compile_expression(left)?;

        let end_blocks = if mid_vals.is_empty() {
            None
        } else {
            let break_block = self.new_block();
            let after_block = self.new_block();
            Some((break_block, after_block))
        };

        // for all comparisons except the last (as the last one doesn't need a conditional jump)
        for (op, val) in mid_ops.iter().zip(mid_vals) {
            self.compile_expression(val)?;
            // store rhs for the next comparison in chain
            self.emit(Instruction::Duplicate);
            self.emit(Instruction::Rotate { amount: 3 });

            self.emit(Instruction::CompareOperation {
                op: compile_cmpop(op),
            });

            // if comparison result is false, we break with this value; if true, try the next one.
            if let Some((break_block, _)) = end_blocks {
                self.emit(Instruction::JumpIfFalseOrPop {
                    target: break_block,
                });
            }
        }

        // handle the last comparison
        self.compile_expression(last_val)?;
        self.emit(Instruction::CompareOperation {
            op: compile_cmpop(last_op),
        });

        if let Some((break_block, after_block)) = end_blocks {
            self.emit(Instruction::Jump {
                target: after_block,
            });

            // early exit left us with stack: `rhs, comparison_result`. We need to clean up rhs.
            self.switch_to_block(break_block);
            self.emit(Instruction::Rotate { amount: 2 });
            self.emit(Instruction::Pop);

            self.switch_to_block(after_block);
        }

        Ok(())
    }

    fn compile_annotated_assign(
        &mut self,
        target: &ast::Expr,
        annotation: &ast::Expr,
        value: Option<&ast::Expr>,
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

        if let ast::ExprKind::Name { id, .. } = &target.node {
            // Store as dict entry in __annotations__ dict:
            let annotations = self.name("__annotations__");
            self.emit(Instruction::LoadNameAny(annotations));
            self.emit_constant(ConstantData::Str {
                value: self.mangle(id).into_owned(),
            });
            self.emit(Instruction::StoreSubscript);
        } else {
            // Drop annotation if not assigned to simple identifier.
            self.emit(Instruction::Pop);
        }

        Ok(())
    }

    fn compile_store(&mut self, target: &ast::Expr) -> CompileResult<()> {
        match &target.node {
            ast::ExprKind::Name { id, .. } => {
                self.store_name(id);
            }
            ast::ExprKind::Subscript { value, slice, .. } => {
                self.compile_expression(value)?;
                self.compile_expression(slice)?;
                self.emit(Instruction::StoreSubscript);
            }
            ast::ExprKind::Attribute { value, attr, .. } => {
                self.compile_expression(value)?;
                let idx = self.name(attr);
                self.emit(Instruction::StoreAttr { idx });
            }
            ast::ExprKind::List { elts, .. } | ast::ExprKind::Tuple { elts, .. } => {
                let mut seen_star = false;

                // Scan for star args:
                for (i, element) in elts.iter().enumerate() {
                    if let ast::ExprKind::Starred { .. } = &element.node {
                        if seen_star {
                            return Err(self.error(CompileErrorType::MultipleStarArgs));
                        } else {
                            seen_star = true;
                            let before = i;
                            let after = elts.len() - i - 1;
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
                        size: elts.len() as u32,
                    });
                }

                for element in elts {
                    if let ast::ExprKind::Starred { value, .. } = &element.node {
                        self.compile_store(value)?;
                    } else {
                        self.compile_store(element)?;
                    }
                }
            }
            _ => {
                return Err(self.error(match target.node {
                    ast::ExprKind::Starred { .. } => CompileErrorType::SyntaxError(
                        "starred assignment target must be in a list or tuple".to_owned(),
                    ),
                    _ => CompileErrorType::Assign(target.node.name()),
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
        expression: &ast::Expr,
        condition: bool,
        target_block: ir::BlockIdx,
    ) -> CompileResult<()> {
        // Compile expression for test, and jump to label if false
        match &expression.node {
            ast::ExprKind::BoolOp { op, values } => {
                match op {
                    ast::Boolop::And => {
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
                    ast::Boolop::Or => {
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
            ast::ExprKind::UnaryOp {
                op: ast::Unaryop::Not,
                operand,
            } => {
                self.compile_jump_if(operand, !condition, target_block)?;
            }
            _ => {
                // Fall back case which always will work!
                self.compile_expression(expression)?;
                if condition {
                    self.emit(Instruction::JumpIfTrue {
                        target: target_block,
                    });
                } else {
                    self.emit(Instruction::JumpIfFalse {
                        target: target_block,
                    });
                }
            }
        }
        Ok(())
    }

    /// Compile a boolean operation as an expression.
    /// This means, that the last value remains on the stack.
    fn compile_bool_op(&mut self, op: &ast::Boolop, values: &[ast::Expr]) -> CompileResult<()> {
        let after_block = self.new_block();

        let (last_value, values) = values.split_last().unwrap();
        for value in values {
            self.compile_expression(value)?;

            match op {
                ast::Boolop::And => {
                    self.emit(Instruction::JumpIfFalseOrPop {
                        target: after_block,
                    });
                }
                ast::Boolop::Or => {
                    self.emit(Instruction::JumpIfTrueOrPop {
                        target: after_block,
                    });
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
        keys: &[Option<Box<ast::Expr>>],
        values: &[ast::Expr],
    ) -> CompileResult<()> {
        let mut size = 0;
        let mut has_unpacking = false;
        for (is_unpacking, subpairs) in &keys.iter().zip(values).group_by(|e| e.0.is_none()) {
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

    fn compile_expression(&mut self, expression: &ast::Expr) -> CompileResult<()> {
        trace!("Compiling {:?}", expression);
        self.set_source_location(expression.location);

        use ast::ExprKind::*;
        match &expression.node {
            Call {
                func,
                args,
                keywords,
            } => self.compile_call(func, args, keywords)?,
            BoolOp { op, values } => self.compile_bool_op(op, values)?,
            BinOp { left, op, right } => {
                self.compile_expression(left)?;
                self.compile_expression(right)?;

                // Perform operation:
                self.compile_op(op, false);
            }
            Subscript { value, slice, .. } => {
                self.compile_expression(value)?;
                self.compile_expression(slice)?;
                self.emit(Instruction::Subscript);
            }
            UnaryOp { op, operand } => {
                self.compile_expression(operand)?;

                // Perform operation:
                let i = match op {
                    ast::Unaryop::UAdd => bytecode::UnaryOperator::Plus,
                    ast::Unaryop::USub => bytecode::UnaryOperator::Minus,
                    ast::Unaryop::Not => bytecode::UnaryOperator::Not,
                    ast::Unaryop::Invert => bytecode::UnaryOperator::Invert,
                };
                let i = Instruction::UnaryOperation { op: i };
                self.emit(i);
            }
            Attribute { value, attr, .. } => {
                self.compile_expression(value)?;
                let idx = self.name(attr);
                self.emit(Instruction::LoadAttr { idx });
            }
            Compare {
                left,
                ops,
                comparators,
            } => {
                self.compile_chained_comparison(left, ops, comparators)?;
            }
            Constant { value, .. } => {
                self.emit_constant(compile_constant(value));
            }
            List { elts, .. } => {
                let (size, unpack) = self.gather_elements(0, elts)?;
                self.emit(Instruction::BuildList { size, unpack });
            }
            Tuple { elts, .. } => {
                let (size, unpack) = self.gather_elements(0, elts)?;
                self.emit(Instruction::BuildTuple { size, unpack });
            }
            Set { elts, .. } => {
                let (size, unpack) = self.gather_elements(0, elts)?;
                self.emit(Instruction::BuildSet { size, unpack });
            }
            Dict { keys, values } => {
                self.compile_dict(keys, values)?;
            }
            Slice { lower, upper, step } => {
                let mut compile_bound = |bound: Option<&ast::Expr>| match bound {
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
                self.emit(Instruction::BuildSlice { step });
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
            ast::ExprKind::JoinedStr { values } => {
                if let Some(value) = try_get_constant_string(values) {
                    self.emit_constant(ConstantData::Str { value })
                } else {
                    for value in values {
                        self.compile_expression(value)?;
                    }
                    self.emit(Instruction::BuildString {
                        size: values.len() as u32,
                    })
                }
            }
            ast::ExprKind::FormattedValue {
                value,
                conversion,
                format_spec,
            } => {
                match format_spec {
                    Some(spec) => self.compile_expression(spec)?,
                    None => self.emit_constant(ConstantData::Str {
                        value: String::new(),
                    }),
                };
                self.compile_expression(value)?;
                self.emit(Instruction::FormatValue {
                    conversion: compile_conversion_flag(*conversion),
                });
            }
            Name { id, .. } => {
                self.load_name(id);
            }
            Lambda { args, body } => {
                let prev_ctx = self.ctx;
                self.ctx = CompileContext {
                    loop_data: Option::None,
                    in_class: prev_ctx.in_class,
                    func: FunctionContext::Function,
                };

                let name = "<lambda>".to_owned();
                let mut funcflags = self.enter_function(&name, args)?;
                self.compile_expression(body)?;
                self.emit(Instruction::ReturnValue);
                let code = self.pop_code_object();
                if self.build_closure(&code) {
                    funcflags |= bytecode::MakeFunctionFlags::CLOSURE;
                }
                self.emit_constant(ConstantData::Code {
                    code: Box::new(code),
                });
                self.emit_constant(ConstantData::Str { value: name });
                // Turn code object into function object:
                self.emit(Instruction::MakeFunction(funcflags));

                self.ctx = prev_ctx;
            }
            ListComp { elt, generators } => {
                self.compile_comprehension(
                    "<listcomp>",
                    Some(Instruction::BuildList {
                        size: 0,
                        unpack: false,
                    }),
                    generators,
                    &|compiler| {
                        compiler.compile_comprehension_element(elt)?;
                        compiler.emit(Instruction::ListAppend {
                            i: (1 + generators.len()) as u32,
                        });
                        Ok(())
                    },
                )?;
            }
            SetComp { elt, generators } => {
                self.compile_comprehension(
                    "<setcomp>",
                    Some(Instruction::BuildSet {
                        size: 0,
                        unpack: false,
                    }),
                    generators,
                    &|compiler| {
                        compiler.compile_comprehension_element(elt)?;
                        compiler.emit(Instruction::SetAdd {
                            i: (1 + generators.len()) as u32,
                        });
                        Ok(())
                    },
                )?;
            }
            DictComp {
                key,
                value,
                generators,
            } => {
                self.compile_comprehension(
                    "<dictcomp>",
                    Some(Instruction::BuildMap {
                        size: 0,
                        for_call: false,
                        unpack: false,
                    }),
                    generators,
                    &|compiler| {
                        // changed evaluation order for Py38 named expression PEP 572
                        compiler.compile_expression(key)?;
                        compiler.compile_expression(value)?;

                        compiler.emit(Instruction::MapAddRev {
                            i: (1 + generators.len()) as u32,
                        });

                        Ok(())
                    },
                )?;
            }
            GeneratorExp { elt, generators } => {
                self.compile_comprehension("<genexpr>", None, generators, &|compiler| {
                    compiler.compile_comprehension_element(elt)?;
                    compiler.mark_generator();
                    compiler.emit(Instruction::YieldValue);
                    compiler.emit(Instruction::Pop);

                    Ok(())
                })?;
            }
            Starred { .. } => {
                return Err(self.error(CompileErrorType::InvalidStarExpr));
            }
            IfExp { test, body, orelse } => {
                let else_block = self.new_block();
                let after_block = self.new_block();
                self.compile_jump_if(test, false, else_block)?;

                // True case
                self.compile_expression(body)?;
                self.emit(Instruction::Jump {
                    target: after_block,
                });

                // False case
                self.switch_to_block(else_block);
                self.compile_expression(orelse)?;

                // End
                self.switch_to_block(after_block);
            }

            NamedExpr { target, value } => {
                self.compile_expression(value)?;
                self.emit(Instruction::Duplicate);
                self.compile_store(target)?;
            }
        }
        Ok(())
    }

    fn compile_keywords(&mut self, keywords: &[ast::Keyword]) -> CompileResult<()> {
        let mut size = 0;
        let groupby = keywords.iter().group_by(|e| e.node.arg.is_none());
        for (is_unpacking, subkeywords) in &groupby {
            if is_unpacking {
                for keyword in subkeywords {
                    self.compile_expression(&keyword.node.value)?;
                    size += 1;
                }
            } else {
                let mut subsize = 0;
                for keyword in subkeywords {
                    if let Some(name) = &keyword.node.arg {
                        self.emit_constant(ConstantData::Str {
                            value: name.to_owned(),
                        });
                        self.compile_expression(&keyword.node.value)?;
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
        func: &ast::Expr,
        args: &[ast::Expr],
        keywords: &[ast::Keyword],
    ) -> CompileResult<()> {
        let method = if let ast::ExprKind::Attribute { value, attr, .. } = &func.node {
            self.compile_expression(value)?;
            let idx = self.name(attr);
            self.emit(Instruction::LoadMethod { idx });
            true
        } else {
            self.compile_expression(func)?;
            false
        };
        let call = self.compile_call_inner(0, args, keywords)?;
        self.emit(if method {
            call.method_call()
        } else {
            call.normal_call()
        });
        Ok(())
    }

    fn compile_call_inner(
        &mut self,
        additional_positional: u32,
        args: &[ast::Expr],
        keywords: &[ast::Keyword],
    ) -> CompileResult<CallType> {
        let count = (args.len() + keywords.len()) as u32 + additional_positional;

        // Normal arguments:
        let (size, unpack) = self.gather_elements(additional_positional, args)?;
        let has_double_star = keywords.iter().any(|k| k.node.arg.is_none());

        let call = if unpack || has_double_star {
            // Create a tuple with positional args:
            self.emit(Instruction::BuildTuple { size, unpack });

            // Create an optional map with kw-args:
            let has_kwargs = !keywords.is_empty();
            if has_kwargs {
                self.compile_keywords(keywords)?;
            }
            CallType::Ex { has_kwargs }
        } else if !keywords.is_empty() {
            let mut kwarg_names = vec![];
            for keyword in keywords {
                if let Some(name) = &keyword.node.arg {
                    kwarg_names.push(ConstantData::Str {
                        value: name.to_owned(),
                    });
                } else {
                    // This means **kwargs!
                    panic!("name must be set");
                }
                self.compile_expression(&keyword.node.value)?;
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
        elements: &[ast::Expr],
    ) -> CompileResult<(u32, bool)> {
        // First determine if we have starred elements:
        let has_stars = elements
            .iter()
            .any(|e| matches!(e.node, ast::ExprKind::Starred { .. }));

        let size = if has_stars {
            let mut size = 0;

            if before > 0 {
                self.emit(Instruction::BuildTuple {
                    size: before,
                    unpack: false,
                });
                size += 1;
            }

            let groups = elements
                .iter()
                .map(|element| {
                    if let ast::ExprKind::Starred { value, .. } = &element.node {
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
                    self.emit(Instruction::BuildTuple {
                        size: run_size,
                        unpack: false,
                    });
                    size += 1
                }
            }

            size
        } else {
            for element in elements {
                self.compile_expression(element)?;
            }
            before + elements.len() as u32
        };

        Ok((size, has_stars))
    }

    fn compile_comprehension_element(&mut self, element: &ast::Expr) -> CompileResult<()> {
        self.compile_expression(element).map_err(|e| {
            if let CompileErrorType::InvalidStarExpr = e.error {
                self.error(CompileErrorType::SyntaxError(
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
        generators: &[ast::Comprehension],
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
        let arg0 = self.varname(".0");

        let return_none = init_collection.is_none();
        // Create empty object of proper type:
        if let Some(init_collection) = init_collection {
            self.emit(init_collection)
        }

        let mut loop_labels = vec![];
        for generator in generators {
            if generator.is_async {
                unimplemented!("async for comprehensions");
            }

            let loop_block = self.new_block();
            let after_block = self.new_block();

            // Setup for loop:
            self.emit(Instruction::SetupLoop {
                break_target: after_block,
            });

            if loop_labels.is_empty() {
                // Load iterator onto stack (passed as first argument):
                self.emit(Instruction::LoadFast(arg0));
            } else {
                // Evaluate iterated item:
                self.compile_expression(&generator.iter)?;

                // Get iterator / turn item into an iterator
                self.emit(Instruction::GetIter);
            }

            loop_labels.push((loop_block, after_block));

            self.switch_to_block(loop_block);
            self.emit(Instruction::ForIter {
                target: after_block,
            });

            self.compile_store(&generator.target)?;

            // Now evaluate the ifs:
            for if_condition in &generator.ifs {
                self.compile_jump_if(if_condition, false, loop_block)?
            }
        }

        compile_element(self)?;

        for (loop_block, after_block) in loop_labels.iter().rev().copied() {
            // Repeat:
            self.emit(Instruction::Jump { target: loop_block });

            // End of for loop:
            self.switch_to_block(after_block);
            self.emit(Instruction::PopBlock);
        }

        if return_none {
            self.emit_constant(ConstantData::None)
        }

        // Return freshly filled list:
        self.emit(Instruction::ReturnValue);

        // Fetch code for listcomp function:
        let code = self.pop_code_object();

        self.ctx = prev_ctx;

        let mut funcflags = bytecode::MakeFunctionFlags::empty();
        if self.build_closure(&code) {
            funcflags |= bytecode::MakeFunctionFlags::CLOSURE;
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
        self.emit(Instruction::MakeFunction(funcflags));

        // Evaluate iterated item:
        self.compile_expression(&generators[0].iter)?;

        // Get iterator / turn item into an iterator
        self.emit(Instruction::GetIter);

        // Call just created <listcomp> function:
        self.emit(Instruction::CallFunctionPositional { nargs: 1 });
        Ok(())
    }

    fn compile_future_features(&mut self, features: &[ast::Alias]) -> Result<(), CompileError> {
        if self.done_with_future_stmts {
            return Err(self.error(CompileErrorType::InvalidFuturePlacement));
        }
        for feature in features {
            match &*feature.name {
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
    fn emit(&mut self, instr: Instruction) {
        let location = compile_location(&self.current_source_location);
        // TODO: insert source filename
        self.current_block()
            .instructions
            .push(ir::InstructionInfo { instr, location });
    }

    // fn block_done()

    fn emit_constant(&mut self, constant: ConstantData) {
        let info = self.current_codeinfo();
        let idx = info.constants.insert_full(constant).0 as u32;
        self.emit(Instruction::LoadConst { idx })
    }

    fn current_codeinfo(&mut self) -> &mut CodeInfo {
        self.code_stack.last_mut().expect("no code on stack")
    }

    fn current_block(&mut self) -> &mut ir::Block {
        let info = self.current_codeinfo();
        &mut info.blocks[info.current_block.0 as usize]
    }

    fn new_block(&mut self) -> ir::BlockIdx {
        let code = self.current_codeinfo();
        let idx = bytecode::Label(code.blocks.len() as u32);
        code.blocks.push(ir::Block::default());
        idx
    }

    fn switch_to_block(&mut self, block: ir::BlockIdx) {
        let code = self.current_codeinfo();
        let prev = code.current_block;
        assert_eq!(
            code.blocks[block.0 as usize].next.0,
            u32::MAX,
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
        self.current_codeinfo().flags |= bytecode::CodeFlags::IS_GENERATOR
    }
}

fn get_doc(body: &[ast::Stmt]) -> (&[ast::Stmt], Option<String>) {
    if let Some((val, body_rest)) = body.split_first() {
        if let ast::StmtKind::Expr { value } = &val.node {
            if let Some(doc) = try_get_constant_string(std::slice::from_ref(value)) {
                return (body_rest, Some(doc));
            }
        }
    }
    (body, None)
}

fn try_get_constant_string(values: &[ast::Expr]) -> Option<String> {
    fn get_constant_string_inner(out_string: &mut String, value: &ast::Expr) -> bool {
        match &value.node {
            ast::ExprKind::Constant {
                value: ast::Constant::Str(s),
                ..
            } => {
                out_string.push_str(s);
                true
            }
            ast::ExprKind::JoinedStr { values } => values
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

fn compile_location(location: &ast::Location) -> bytecode::Location {
    bytecode::Location::new(location.row(), location.column())
}

fn compile_conversion_flag(
    conversion_flag: Option<ast::ConversionFlag>,
) -> bytecode::ConversionFlag {
    match conversion_flag {
        None => bytecode::ConversionFlag::None,
        Some(ast::ConversionFlag::Ascii) => bytecode::ConversionFlag::Ascii,
        Some(ast::ConversionFlag::Repr) => bytecode::ConversionFlag::Repr,
        Some(ast::ConversionFlag::Str) => bytecode::ConversionFlag::Str,
    }
}

fn compile_constant(value: &ast::Constant) -> ConstantData {
    match value {
        ast::Constant::None => ConstantData::None,
        ast::Constant::Bool(b) => ConstantData::Boolean { value: *b },
        ast::Constant::Str(s) => ConstantData::Str { value: s.clone() },
        ast::Constant::Bytes(b) => ConstantData::Bytes { value: b.clone() },
        ast::Constant::Int(i) => ConstantData::Integer { value: i.clone() },
        ast::Constant::Tuple(t) => ConstantData::Tuple {
            elements: t.iter().map(compile_constant).collect(),
        },
        ast::Constant::Float(f) => ConstantData::Float { value: *f },
        ast::Constant::Complex { real, imag } => ConstantData::Complex {
            value: Complex64::new(*real, *imag),
        },
        ast::Constant::Ellipsis => ConstantData::Ellipsis,
    }
}

#[cfg(test)]
mod tests {
    use super::{CompileOpts, Compiler};
    use crate::symboltable::make_symbol_table;
    use rustpython_bytecode::CodeObject;
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

    #[test]
    fn test_nested_double_async_with() {
        assert_dis_snapshot!(compile_exec(
            "\
for stop_exc in (StopIteration('spam'), StopAsyncIteration('ham')):
    with self.subTest(type=type(stop_exc)):
        try:
            async with woohoo():
                raise stop_exc
        except Exception as ex:
            self.assertIs(ex, stop_exc)
        else:
            self.fail(f'{stop_exc} was suppressed')
"
        ));
    }
}
