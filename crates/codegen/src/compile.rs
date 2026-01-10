//!
//! Take an AST and transform it into bytecode
//!
//! Inspirational code:
//!   <https://github.com/python/cpython/blob/main/Python/compile.c>
//!   <https://github.com/micropython/micropython/blob/master/py/compile.c>

// spell-checker:ignore starunpack subscripter

#![deny(clippy::cast_possible_truncation)]

use crate::{
    IndexMap, IndexSet, ToPythonName,
    error::{CodegenError, CodegenErrorType, InternalError, PatternUnreachableReason},
    ir::{self, BlockIdx},
    symboltable::{self, CompilerScope, SymbolFlags, SymbolScope, SymbolTable},
    unparse::UnparseExpr,
};
use alloc::borrow::Cow;
use itertools::Itertools;
use malachite_bigint::BigInt;
use num_complex::Complex;
use num_traits::{Num, ToPrimitive};
use ruff_python_ast::{
    Alias, Arguments, BoolOp, CmpOp, Comprehension, ConversionFlag, DebugText, Decorator, DictItem,
    ExceptHandler, ExceptHandlerExceptHandler, Expr, ExprAttribute, ExprBoolOp, ExprContext,
    ExprFString, ExprList, ExprName, ExprSlice, ExprStarred, ExprSubscript, ExprTuple, ExprUnaryOp,
    FString, FStringFlags, FStringPart, Identifier, Int, InterpolatedStringElement,
    InterpolatedStringElements, Keyword, MatchCase, ModExpression, ModModule, Operator, Parameters,
    Pattern, PatternMatchAs, PatternMatchClass, PatternMatchMapping, PatternMatchOr,
    PatternMatchSequence, PatternMatchSingleton, PatternMatchStar, PatternMatchValue, Singleton,
    Stmt, StmtExpr, TypeParam, TypeParamParamSpec, TypeParamTypeVar, TypeParamTypeVarTuple,
    TypeParams, UnaryOp, WithItem,
    visitor::{Visitor, walk_expr},
};
use ruff_text_size::{Ranged, TextRange};
use rustpython_compiler_core::{
    Mode, OneIndexed, PositionEncoding, SourceFile, SourceLocation,
    bytecode::{
        self, Arg as OpArgMarker, BinaryOperator, BuildSliceArgCount, CodeObject,
        ComparisonOperator, ConstantData, ConvertValueOparg, Instruction, Invert, OpArg, OpArgType,
        UnpackExArgs,
    },
};
use rustpython_wtf8::Wtf8Buf;
use std::collections::HashSet;

const MAXBLOCKS: usize = 20;

#[derive(Debug, Clone, Copy)]
pub enum FBlockType {
    WhileLoop,
    ForLoop,
    TryExcept,
    FinallyTry,
    FinallyEnd,
    With,
    AsyncWith,
    HandlerCleanup,
    PopValue,
    ExceptionHandler,
    ExceptionGroupHandler,
    AsyncComprehensionGenerator,
    StopIteration,
}

/// Stores additional data for fblock unwinding
// fb_datum
#[derive(Debug, Clone)]
pub enum FBlockDatum {
    None,
    /// For FinallyTry: stores the finally body statements to compile during unwind
    FinallyBody(Vec<Stmt>),
    /// For HandlerCleanup: stores the exception variable name (e.g., "e" in "except X as e")
    ExceptionName(String),
}

#[derive(Debug, Clone)]
pub struct FBlockInfo {
    pub fb_type: FBlockType,
    pub fb_block: BlockIdx,
    pub fb_exit: BlockIdx,
    // For Python 3.11+ exception table generation
    pub fb_handler: Option<BlockIdx>, // Exception handler block
    pub fb_stack_depth: u32,          // Stack depth at block entry
    pub fb_preserve_lasti: bool,      // Whether to preserve lasti (for SETUP_CLEANUP)
    // additional data for fblock unwinding
    pub fb_datum: FBlockDatum,
}

pub(crate) type InternalResult<T> = Result<T, InternalError>;
type CompileResult<T> = Result<T, CodegenError>;

#[derive(PartialEq, Eq, Clone, Copy)]
enum NameUsage {
    Load,
    Store,
    Delete,
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
    source_file: SourceFile,
    // current_source_location: SourceLocation,
    current_source_range: TextRange,
    done_with_future_stmts: DoneWithFuture,
    future_annotations: bool,
    ctx: CompileContext,
    opts: CompileOpts,
    in_annotation: bool,
}

enum DoneWithFuture {
    No,
    DoneWithDoc,
    Yes,
}

#[derive(Debug, Clone)]
pub struct CompileOpts {
    /// How optimized the bytecode output should be; any optimize > 0 does
    /// not emit assert statements
    pub optimize: u8,
    /// Include column info in bytecode (-X no_debug_ranges disables)
    pub debug_ranges: bool,
}

impl Default for CompileOpts {
    fn default() -> Self {
        Self {
            optimize: 0,
            debug_ranges: true,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct CompileContext {
    loop_data: Option<(BlockIdx, BlockIdx)>,
    in_class: bool,
    func: FunctionContext,
    /// True if we're anywhere inside an async function (even inside nested comprehensions)
    in_async_scope: bool,
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

fn validate_duplicate_params(params: &Parameters) -> Result<(), CodegenErrorType> {
    let mut seen_params = HashSet::new();
    for param in params {
        let param_name = param.name().as_str();
        if !seen_params.insert(param_name) {
            return Err(CodegenErrorType::SyntaxError(format!(
                r#"Duplicate parameter "{param_name}""#
            )));
        }
    }

    Ok(())
}

/// Compile an Mod produced from ruff parser
pub fn compile_top(
    ast: ruff_python_ast::Mod,
    source_file: SourceFile,
    mode: Mode,
    opts: CompileOpts,
) -> CompileResult<CodeObject> {
    match ast {
        ruff_python_ast::Mod::Module(module) => match mode {
            Mode::Exec | Mode::Eval => compile_program(&module, source_file, opts),
            Mode::Single => compile_program_single(&module, source_file, opts),
            Mode::BlockExpr => compile_block_expression(&module, source_file, opts),
        },
        ruff_python_ast::Mod::Expression(expr) => compile_expression(&expr, source_file, opts),
    }
}

/// Compile a standard Python program to bytecode
pub fn compile_program(
    ast: &ModModule,
    source_file: SourceFile,
    opts: CompileOpts,
) -> CompileResult<CodeObject> {
    let symbol_table = SymbolTable::scan_program(ast, source_file.clone())
        .map_err(|e| e.into_codegen_error(source_file.name().to_owned()))?;
    let mut compiler = Compiler::new(opts, source_file, "<module>".to_owned());
    compiler.compile_program(ast, symbol_table)?;
    let code = compiler.exit_scope();
    trace!("Compilation completed: {code:?}");
    Ok(code)
}

/// Compile a Python program to bytecode for the context of a REPL
pub fn compile_program_single(
    ast: &ModModule,
    source_file: SourceFile,
    opts: CompileOpts,
) -> CompileResult<CodeObject> {
    let symbol_table = SymbolTable::scan_program(ast, source_file.clone())
        .map_err(|e| e.into_codegen_error(source_file.name().to_owned()))?;
    let mut compiler = Compiler::new(opts, source_file, "<module>".to_owned());
    compiler.compile_program_single(&ast.body, symbol_table)?;
    let code = compiler.exit_scope();
    trace!("Compilation completed: {code:?}");
    Ok(code)
}

pub fn compile_block_expression(
    ast: &ModModule,
    source_file: SourceFile,
    opts: CompileOpts,
) -> CompileResult<CodeObject> {
    let symbol_table = SymbolTable::scan_program(ast, source_file.clone())
        .map_err(|e| e.into_codegen_error(source_file.name().to_owned()))?;
    let mut compiler = Compiler::new(opts, source_file, "<module>".to_owned());
    compiler.compile_block_expr(&ast.body, symbol_table)?;
    let code = compiler.exit_scope();
    trace!("Compilation completed: {code:?}");
    Ok(code)
}

pub fn compile_expression(
    ast: &ModExpression,
    source_file: SourceFile,
    opts: CompileOpts,
) -> CompileResult<CodeObject> {
    let symbol_table = SymbolTable::scan_expr(ast, source_file.clone())
        .map_err(|e| e.into_codegen_error(source_file.name().to_owned()))?;
    let mut compiler = Compiler::new(opts, source_file, "<module>".to_owned());
    compiler.compile_eval(ast, symbol_table)?;
    let code = compiler.exit_scope();
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

fn eprint_location(zelf: &Compiler) {
    let start = zelf
        .source_file
        .to_source_code()
        .source_location(zelf.current_source_range.start(), PositionEncoding::Utf8);
    let end = zelf
        .source_file
        .to_source_code()
        .source_location(zelf.current_source_range.end(), PositionEncoding::Utf8);
    eprintln!(
        "LOCATION: {} from {}:{} to {}:{}",
        zelf.source_file.name(),
        start.line,
        start.character_offset,
        end.line,
        end.character_offset
    );
}

/// Better traceback for internal error
#[track_caller]
fn unwrap_internal<T>(zelf: &Compiler, r: InternalResult<T>) -> T {
    if let Err(ref r_err) = r {
        eprintln!("=== CODEGEN PANIC INFO ===");
        eprintln!("This IS an internal error: {r_err}");
        eprint_location(zelf);
        eprintln!("=== END PANIC INFO ===");
    }
    r.unwrap()
}

fn compiler_unwrap_option<T>(zelf: &Compiler, o: Option<T>) -> T {
    if o.is_none() {
        eprintln!("=== CODEGEN PANIC INFO ===");
        eprintln!("This IS an internal error, an option was unwrapped during codegen");
        eprint_location(zelf);
        eprintln!("=== END PANIC INFO ===");
    }
    o.unwrap()
}

// fn compiler_result_unwrap<T, E: core::fmt::Debug>(zelf: &Compiler, result: Result<T, E>) -> T {
//     if result.is_err() {
//         eprintln!("=== CODEGEN PANIC INFO ===");
//         eprintln!("This IS an internal error, an result was unwrapped during codegen");
//         eprint_location(zelf);
//         eprintln!("=== END PANIC INFO ===");
//     }
//     result.unwrap()
// }

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
    pub const fn new() -> Self {
        Self {
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

/// Type of collection to build in starunpack_helper
#[derive(Debug, Clone, Copy, PartialEq)]
enum CollectionType {
    Tuple,
    List,
    Set,
}

impl Compiler {
    fn new(opts: CompileOpts, source_file: SourceFile, code_name: String) -> Self {
        let module_code = ir::CodeInfo {
            flags: bytecode::CodeFlags::NEWLOCALS,
            source_path: source_file.name().to_owned(),
            private: None,
            blocks: vec![ir::Block::default()],
            current_block: BlockIdx::new(0),
            metadata: ir::CodeUnitMetadata {
                name: code_name.clone(),
                qualname: Some(code_name),
                consts: IndexSet::default(),
                names: IndexSet::default(),
                varnames: IndexSet::default(),
                cellvars: IndexSet::default(),
                freevars: IndexSet::default(),
                fast_hidden: IndexMap::default(),
                argcount: 0,
                posonlyargcount: 0,
                kwonlyargcount: 0,
                firstlineno: OneIndexed::MIN,
            },
            static_attributes: None,
            in_inlined_comp: false,
            fblock: Vec::with_capacity(MAXBLOCKS),
            symbol_table_index: 0, // Module is always the first symbol table
        };
        Self {
            code_stack: vec![module_code],
            symbol_table_stack: Vec::new(),
            source_file,
            // current_source_location: SourceLocation::default(),
            current_source_range: TextRange::default(),
            done_with_future_stmts: DoneWithFuture::No,
            future_annotations: false,
            ctx: CompileContext {
                loop_data: None,
                in_class: false,
                func: FunctionContext::NoFunction,
                in_async_scope: false,
            },
            opts,
            in_annotation: false,
        }
    }

    /// Check if the slice is a two-element slice (no step)
    // = is_two_element_slice
    const fn is_two_element_slice(slice: &Expr) -> bool {
        matches!(slice, Expr::Slice(s) if s.step.is_none())
    }

    /// Compile a slice expression
    // = compiler_slice
    fn compile_slice(&mut self, s: &ExprSlice) -> CompileResult<BuildSliceArgCount> {
        // Compile lower
        if let Some(lower) = &s.lower {
            self.compile_expression(lower)?;
        } else {
            self.emit_load_const(ConstantData::None);
        }

        // Compile upper
        if let Some(upper) = &s.upper {
            self.compile_expression(upper)?;
        } else {
            self.emit_load_const(ConstantData::None);
        }

        Ok(match &s.step {
            Some(step) => {
                // Compile step if present
                self.compile_expression(step)?;
                BuildSliceArgCount::Three
            }
            None => BuildSliceArgCount::Two,
        })
    }

    /// Compile a subscript expression
    // = compiler_subscript
    fn compile_subscript(
        &mut self,
        value: &Expr,
        slice: &Expr,
        ctx: ExprContext,
    ) -> CompileResult<()> {
        // 1. Check subscripter and index for Load context
        // 2. VISIT value
        // 3. Handle two-element slice specially
        // 4. Otherwise VISIT slice and emit appropriate instruction

        // For Load context, some checks are skipped for now
        // if ctx == ExprContext::Load {
        //     check_subscripter(value);
        //     check_index(value, slice);
        // }

        // VISIT(c, expr, e->v.Subscript.value)
        self.compile_expression(value)?;

        // Handle two-element slice (for Load/Store, not Del)
        if Self::is_two_element_slice(slice) && !matches!(ctx, ExprContext::Del) {
            let argc = match slice {
                Expr::Slice(s) => self.compile_slice(s)?,
                _ => unreachable!("is_two_element_slice should only return true for Expr::Slice"),
            };
            match ctx {
                ExprContext::Load => {
                    emit!(self, Instruction::BuildSlice { argc });
                    emit!(self, Instruction::Subscript);
                }
                ExprContext::Store => {
                    emit!(self, Instruction::BuildSlice { argc });
                    emit!(self, Instruction::StoreSubscr);
                }
                _ => unreachable!(),
            }
        } else {
            // VISIT(c, expr, e->v.Subscript.slice)
            self.compile_expression(slice)?;

            // Emit appropriate instruction based on context
            match ctx {
                ExprContext::Load => emit!(self, Instruction::Subscript),
                ExprContext::Store => emit!(self, Instruction::StoreSubscr),
                ExprContext::Del => emit!(self, Instruction::DeleteSubscr),
                ExprContext::Invalid => {
                    return Err(self.error(CodegenErrorType::SyntaxError(
                        "Invalid expression context".to_owned(),
                    )));
                }
            }
        }

        Ok(())
    }

    /// Helper function for compiling tuples/lists/sets with starred expressions
    ///
    /// Parameters:
    /// - elts: The elements to compile
    /// - pushed: Number of items already on the stack
    /// - collection_type: What type of collection to build (tuple, list, set)
    ///
    // = starunpack_helper in compile.c
    fn starunpack_helper(
        &mut self,
        elts: &[Expr],
        pushed: u32,
        collection_type: CollectionType,
    ) -> CompileResult<()> {
        // Use RustPython's existing approach with BuildXFromTuples
        let (size, unpack) = self.gather_elements(pushed, elts)?;

        if unpack {
            // Has starred elements
            match collection_type {
                CollectionType::Tuple => {
                    if size > 1 || pushed > 0 {
                        emit!(self, Instruction::BuildTupleFromTuples { size });
                    }
                    // If size == 1 and pushed == 0, the single tuple is already on the stack
                }
                CollectionType::List => {
                    emit!(self, Instruction::BuildListFromTuples { size });
                }
                CollectionType::Set => {
                    emit!(self, Instruction::BuildSetFromTuples { size });
                }
            }
        } else {
            // No starred elements
            match collection_type {
                CollectionType::Tuple => {
                    emit!(self, Instruction::BuildTuple { size });
                }
                CollectionType::List => {
                    emit!(self, Instruction::BuildList { size });
                }
                CollectionType::Set => {
                    emit!(self, Instruction::BuildSet { size });
                }
            }
        }

        Ok(())
    }

    fn error(&mut self, error: CodegenErrorType) -> CodegenError {
        self.error_ranged(error, self.current_source_range)
    }

    fn error_ranged(&mut self, error: CodegenErrorType, range: TextRange) -> CodegenError {
        let location = self
            .source_file
            .to_source_code()
            .source_location(range.start(), PositionEncoding::Utf8);
        CodegenError {
            error,
            location: Some(location),
            source_path: self.source_file.name().to_owned(),
        }
    }

    /// Get the SymbolTable for the current scope.
    fn current_symbol_table(&self) -> &SymbolTable {
        self.symbol_table_stack
            .last()
            .expect("symbol_table_stack is empty! This is a compiler bug.")
    }

    /// Get the index of a free variable.
    fn get_free_var_index(&mut self, name: &str) -> CompileResult<u32> {
        let info = self.code_stack.last_mut().unwrap();
        let idx = info
            .metadata
            .freevars
            .get_index_of(name)
            .unwrap_or_else(|| info.metadata.freevars.insert_full(name.to_owned()).0);
        Ok((idx + info.metadata.cellvars.len()).to_u32())
    }

    /// Get the index of a cell variable.
    fn get_cell_var_index(&mut self, name: &str) -> CompileResult<u32> {
        let info = self.code_stack.last_mut().unwrap();
        let idx = info
            .metadata
            .cellvars
            .get_index_of(name)
            .unwrap_or_else(|| info.metadata.cellvars.insert_full(name.to_owned()).0);
        Ok(idx.to_u32())
    }

    /// Get the index of a local variable.
    fn get_local_var_index(&mut self, name: &str) -> CompileResult<u32> {
        let info = self.code_stack.last_mut().unwrap();
        let idx = info
            .metadata
            .varnames
            .get_index_of(name)
            .unwrap_or_else(|| info.metadata.varnames.insert_full(name.to_owned()).0);
        Ok(idx.to_u32())
    }

    /// Get the index of a global name.
    fn get_global_name_index(&mut self, name: &str) -> u32 {
        let info = self.code_stack.last_mut().unwrap();
        let idx = info
            .metadata
            .names
            .get_index_of(name)
            .unwrap_or_else(|| info.metadata.names.insert_full(name.to_owned()).0);
        idx.to_u32()
    }

    /// Push the next symbol table on to the stack
    fn push_symbol_table(&mut self) -> CompileResult<&SymbolTable> {
        // Look up the next table contained in the scope of the current table
        let current_table = self
            .symbol_table_stack
            .last_mut()
            .expect("no current symbol table");

        if current_table.next_sub_table >= current_table.sub_tables.len() {
            let name = current_table.name.clone();
            let typ = current_table.typ;
            return Err(self.error(CodegenErrorType::SyntaxError(format!(
                "no symbol table available in {} (type: {:?})",
                name, typ
            ))));
        }

        let idx = current_table.next_sub_table;
        current_table.next_sub_table += 1;
        let table = current_table.sub_tables[idx].clone();

        // Push the next table onto the stack
        self.symbol_table_stack.push(table);
        Ok(self.current_symbol_table())
    }

    /// Pop the current symbol table off the stack
    fn pop_symbol_table(&mut self) -> SymbolTable {
        self.symbol_table_stack.pop().expect("compiler bug")
    }

    /// Check if this is an inlined comprehension context (PEP 709)
    /// Currently disabled - always returns false to avoid stack issues
    fn is_inlined_comprehension_context(&self, _comprehension_type: ComprehensionType) -> bool {
        // TODO: Implement PEP 709 inlined comprehensions properly
        // For now, disabled to avoid stack underflow issues
        false
    }

    /// Enter a new scope
    // = compiler_enter_scope
    fn enter_scope(
        &mut self,
        name: &str,
        scope_type: CompilerScope,
        key: usize, // In RustPython, we use the index in symbol_table_stack as key
        lineno: u32,
    ) -> CompileResult<()> {
        // Create location
        let location = SourceLocation {
            line: OneIndexed::new(lineno as usize).unwrap_or(OneIndexed::MIN),
            character_offset: OneIndexed::MIN,
        };

        // Allocate a new compiler unit

        // In Rust, we'll create the structure directly
        let source_path = self.source_file.name().to_owned();

        // Lookup symbol table entry using key (_PySymtable_Lookup)
        let ste = match self.symbol_table_stack.get(key) {
            Some(v) => v,
            None => {
                return Err(self.error(CodegenErrorType::SyntaxError(
                    "unknown symbol table entry".to_owned(),
                )));
            }
        };

        // Use varnames from symbol table (already collected in definition order)
        let varname_cache: IndexSet<String> = ste.varnames.iter().cloned().collect();

        // Build cellvars using dictbytype (CELL scope, sorted)
        let mut cellvar_cache = IndexSet::default();
        let mut cell_names: Vec<_> = ste
            .symbols
            .iter()
            .filter(|(_, s)| s.scope == SymbolScope::Cell)
            .map(|(name, _)| name.clone())
            .collect();
        cell_names.sort();
        for name in cell_names {
            cellvar_cache.insert(name);
        }

        // Handle implicit __class__ cell if needed
        if ste.needs_class_closure {
            // Cook up an implicit __class__ cell
            debug_assert_eq!(scope_type, CompilerScope::Class);
            cellvar_cache.insert("__class__".to_string());
        }

        // Handle implicit __classdict__ cell if needed
        if ste.needs_classdict {
            // Cook up an implicit __classdict__ cell
            debug_assert_eq!(scope_type, CompilerScope::Class);
            cellvar_cache.insert("__classdict__".to_string());
        }

        // Build freevars using dictbytype (FREE scope, offset by cellvars size)
        let mut freevar_cache = IndexSet::default();
        let mut free_names: Vec<_> = ste
            .symbols
            .iter()
            .filter(|(_, s)| {
                s.scope == SymbolScope::Free || s.flags.contains(SymbolFlags::FREE_CLASS)
            })
            .map(|(name, _)| name.clone())
            .collect();
        free_names.sort();
        for name in free_names {
            freevar_cache.insert(name);
        }

        // Initialize u_metadata fields
        let (flags, posonlyarg_count, arg_count, kwonlyarg_count) = match scope_type {
            CompilerScope::Module => (bytecode::CodeFlags::empty(), 0, 0, 0),
            CompilerScope::Class => (bytecode::CodeFlags::empty(), 0, 0, 0),
            CompilerScope::Function | CompilerScope::AsyncFunction | CompilerScope::Lambda => (
                bytecode::CodeFlags::NEWLOCALS | bytecode::CodeFlags::OPTIMIZED,
                0, // Will be set later in enter_function
                0, // Will be set later in enter_function
                0, // Will be set later in enter_function
            ),
            CompilerScope::Comprehension => (
                bytecode::CodeFlags::NEWLOCALS | bytecode::CodeFlags::OPTIMIZED,
                0,
                1, // comprehensions take one argument (.0)
                0,
            ),
            CompilerScope::TypeParams => (
                bytecode::CodeFlags::NEWLOCALS | bytecode::CodeFlags::OPTIMIZED,
                0,
                0,
                0,
            ),
        };

        // Get private name from parent scope
        let private = if !self.code_stack.is_empty() {
            self.code_stack.last().unwrap().private.clone()
        } else {
            None
        };

        // Create the new compilation unit
        let code_info = ir::CodeInfo {
            flags,
            source_path: source_path.clone(),
            private,
            blocks: vec![ir::Block::default()],
            current_block: BlockIdx::new(0),
            metadata: ir::CodeUnitMetadata {
                name: name.to_owned(),
                qualname: None, // Will be set below
                consts: IndexSet::default(),
                names: IndexSet::default(),
                varnames: varname_cache,
                cellvars: cellvar_cache,
                freevars: freevar_cache,
                fast_hidden: IndexMap::default(),
                argcount: arg_count,
                posonlyargcount: posonlyarg_count,
                kwonlyargcount: kwonlyarg_count,
                firstlineno: OneIndexed::new(lineno as usize).unwrap_or(OneIndexed::MIN),
            },
            static_attributes: if scope_type == CompilerScope::Class {
                Some(IndexSet::default())
            } else {
                None
            },
            in_inlined_comp: false,
            fblock: Vec::with_capacity(MAXBLOCKS),
            symbol_table_index: key,
        };

        // Push the old compiler unit on the stack (like PyCapsule)
        // This happens before setting qualname
        self.code_stack.push(code_info);

        // Set qualname after pushing (uses compiler_set_qualname logic)
        if scope_type != CompilerScope::Module {
            self.set_qualname();
        }

        // Emit RESUME instruction
        let _resume_loc = if scope_type == CompilerScope::Module {
            // Module scope starts with lineno 0
            SourceLocation {
                line: OneIndexed::MIN,
                character_offset: OneIndexed::MIN,
            }
        } else {
            location
        };

        // Set the source range for the RESUME instruction
        // For now, just use an empty range at the beginning
        self.current_source_range = TextRange::default();
        emit!(
            self,
            Instruction::Resume {
                arg: bytecode::ResumeType::AtFuncStart as u32
            }
        );

        if scope_type == CompilerScope::Module {
            // This would be loc.lineno = -1 in CPython
            // We handle this differently in RustPython
        }

        Ok(())
    }

    fn push_output(
        &mut self,
        flags: bytecode::CodeFlags,
        posonlyarg_count: u32,
        arg_count: u32,
        kwonlyarg_count: u32,
        obj_name: String,
    ) -> CompileResult<()> {
        // First push the symbol table
        let table = self.push_symbol_table()?;
        let scope_type = table.typ;

        // The key is the current position in the symbol table stack
        let key = self.symbol_table_stack.len() - 1;

        // Get the line number
        let lineno = self.get_source_line_number().get();

        // Call enter_scope which does most of the work
        self.enter_scope(&obj_name, scope_type, key, lineno.to_u32())?;

        // Override the values that push_output sets explicitly
        // enter_scope sets default values based on scope_type, but push_output
        // allows callers to specify exact values
        if let Some(info) = self.code_stack.last_mut() {
            info.flags = flags;
            info.metadata.argcount = arg_count;
            info.metadata.posonlyargcount = posonlyarg_count;
            info.metadata.kwonlyargcount = kwonlyarg_count;
        }
        Ok(())
    }

    // compiler_exit_scope
    fn exit_scope(&mut self) -> CodeObject {
        let _table = self.pop_symbol_table();

        // Various scopes can have sub_tables:
        // - TypeParams scope can have sub_tables (the function body's symbol table)
        // - Module scope can have sub_tables (for TypeAlias scopes, nested functions, classes)
        // - Function scope can have sub_tables (for nested functions, classes)
        // - Class scope can have sub_tables (for nested classes, methods)

        let pop = self.code_stack.pop();
        let stack_top = compiler_unwrap_option(self, pop);
        // No parent scope stack to maintain
        unwrap_internal(self, stack_top.finalize_code(&self.opts))
    }

    /// Push a new fblock
    // = compiler_push_fblock
    fn push_fblock(
        &mut self,
        fb_type: FBlockType,
        fb_block: BlockIdx,
        fb_exit: BlockIdx,
    ) -> CompileResult<()> {
        self.push_fblock_full(
            fb_type,
            fb_block,
            fb_exit,
            None,
            0,
            false,
            FBlockDatum::None,
        )
    }

    /// Push an fblock with exception handler info
    fn push_fblock_with_handler(
        &mut self,
        fb_type: FBlockType,
        fb_block: BlockIdx,
        fb_exit: BlockIdx,
        fb_handler: Option<BlockIdx>,
        fb_stack_depth: u32,
        fb_preserve_lasti: bool,
    ) -> CompileResult<()> {
        self.push_fblock_full(
            fb_type,
            fb_block,
            fb_exit,
            fb_handler,
            fb_stack_depth,
            fb_preserve_lasti,
            FBlockDatum::None,
        )
    }

    /// Push an fblock with all parameters including fb_datum
    #[allow(clippy::too_many_arguments)]
    fn push_fblock_full(
        &mut self,
        fb_type: FBlockType,
        fb_block: BlockIdx,
        fb_exit: BlockIdx,
        fb_handler: Option<BlockIdx>,
        fb_stack_depth: u32,
        fb_preserve_lasti: bool,
        fb_datum: FBlockDatum,
    ) -> CompileResult<()> {
        let code = self.current_code_info();
        if code.fblock.len() >= MAXBLOCKS {
            return Err(self.error(CodegenErrorType::SyntaxError(
                "too many statically nested blocks".to_owned(),
            )));
        }
        code.fblock.push(FBlockInfo {
            fb_type,
            fb_block,
            fb_exit,
            fb_handler,
            fb_stack_depth,
            fb_preserve_lasti,
            fb_datum,
        });
        Ok(())
    }

    /// Pop an fblock
    // = compiler_pop_fblock
    fn pop_fblock(&mut self, _expected_type: FBlockType) -> FBlockInfo {
        let code = self.current_code_info();
        // TODO: Add assertion to check expected type matches
        // assert!(matches!(fblock.fb_type, expected_type));
        code.fblock.pop().expect("fblock stack underflow")
    }

    /// Unwind a single fblock, emitting cleanup code
    /// preserve_tos: if true, preserve the top of stack (e.g., return value)
    fn unwind_fblock(&mut self, info: &FBlockInfo, preserve_tos: bool) -> CompileResult<()> {
        match info.fb_type {
            FBlockType::WhileLoop
            | FBlockType::ExceptionHandler
            | FBlockType::ExceptionGroupHandler
            | FBlockType::AsyncComprehensionGenerator
            | FBlockType::StopIteration => {
                // No cleanup needed
            }

            FBlockType::ForLoop => {
                // Pop the iterator
                if preserve_tos {
                    emit!(self, Instruction::Swap { index: 2 });
                }
                emit!(self, Instruction::PopTop);
            }

            FBlockType::TryExcept => {
                // No POP_BLOCK with exception table, just pop fblock
            }

            FBlockType::FinallyTry => {
                // FinallyTry is now handled specially in unwind_fblock_stack
                // to avoid infinite recursion when the finally body contains return/break/continue.
                // This branch should not be reached.
                unreachable!("FinallyTry should be handled by unwind_fblock_stack");
            }

            FBlockType::FinallyEnd => {
                // Stack when in FinallyEnd: [..., prev_exc, exc] or
                // [..., prev_exc, exc, return_value] if preserve_tos
                // Note: No lasti here - it's only pushed for cleanup handler exceptions
                // We need to pop: exc, prev_exc (via PopExcept)
                if preserve_tos {
                    emit!(self, Instruction::Swap { index: 2 });
                }
                emit!(self, Instruction::PopTop); // exc
                if preserve_tos {
                    emit!(self, Instruction::Swap { index: 2 });
                }
                emit!(self, Instruction::PopExcept); // prev_exc is restored
            }

            FBlockType::With | FBlockType::AsyncWith => {
                // Stack when entering: [..., __exit__, return_value (if preserve_tos)]
                // Need to call __exit__(None, None, None)

                emit!(self, Instruction::PopBlock);

                // If preserving return value, swap it below __exit__
                if preserve_tos {
                    emit!(self, Instruction::Swap { index: 2 });
                }
                // Stack after swap: [..., return_value, __exit__] or [..., __exit__]

                // Call __exit__(None, None, None)
                // Call protocol: [callable, self_or_null, arg1, arg2, arg3]
                emit!(self, Instruction::PushNull);
                // Stack: [..., __exit__, NULL]
                self.emit_load_const(ConstantData::None);
                self.emit_load_const(ConstantData::None);
                self.emit_load_const(ConstantData::None);
                // Stack: [..., __exit__, NULL, None, None, None]
                emit!(self, Instruction::Call { nargs: 3 });

                // For async with, await the result
                if matches!(info.fb_type, FBlockType::AsyncWith) {
                    emit!(self, Instruction::GetAwaitable);
                    self.emit_load_const(ConstantData::None);
                    self.compile_yield_from_sequence(true)?;
                }

                // Pop the __exit__ result
                emit!(self, Instruction::PopTop);
            }

            FBlockType::HandlerCleanup => {
                if preserve_tos {
                    emit!(self, Instruction::Swap { index: 2 });
                }
                emit!(self, Instruction::PopExcept);

                // If there's an exception name, clean it up
                if let FBlockDatum::ExceptionName(ref name) = info.fb_datum {
                    self.emit_load_const(ConstantData::None);
                    self.store_name(name)?;
                    self.compile_name(name, NameUsage::Delete)?;
                }
            }

            FBlockType::PopValue => {
                if preserve_tos {
                    emit!(self, Instruction::Swap { index: 2 });
                }
                emit!(self, Instruction::PopTop);
            }
        }
        Ok(())
    }

    /// Unwind the fblock stack, emitting cleanup code for each block
    /// preserve_tos: if true, preserve the top of stack (e.g., return value)
    /// stop_at_loop: if true, stop when encountering a loop (for break/continue)
    fn unwind_fblock_stack(&mut self, preserve_tos: bool, stop_at_loop: bool) -> CompileResult<()> {
        // Collect the info we need, with indices for FinallyTry blocks
        #[derive(Clone)]
        enum UnwindInfo {
            Normal(FBlockInfo),
            FinallyTry {
                body: Vec<ruff_python_ast::Stmt>,
                fblock_idx: usize,
            },
        }
        let mut unwind_infos = Vec::new();

        {
            let code = self.current_code_info();
            for i in (0..code.fblock.len()).rev() {
                // Check for exception group handler (forbidden)
                if matches!(code.fblock[i].fb_type, FBlockType::ExceptionGroupHandler) {
                    return Err(self.error(CodegenErrorType::BreakContinueReturnInExceptStar));
                }

                // Stop at loop if requested
                if stop_at_loop
                    && matches!(
                        code.fblock[i].fb_type,
                        FBlockType::WhileLoop | FBlockType::ForLoop
                    )
                {
                    break;
                }

                if matches!(code.fblock[i].fb_type, FBlockType::FinallyTry) {
                    if let FBlockDatum::FinallyBody(ref body) = code.fblock[i].fb_datum {
                        unwind_infos.push(UnwindInfo::FinallyTry {
                            body: body.clone(),
                            fblock_idx: i,
                        });
                    }
                } else {
                    unwind_infos.push(UnwindInfo::Normal(code.fblock[i].clone()));
                }
            }
        }

        // Process each fblock
        for info in unwind_infos {
            match info {
                UnwindInfo::Normal(fblock_info) => {
                    self.unwind_fblock(&fblock_info, preserve_tos)?;
                }
                UnwindInfo::FinallyTry { body, fblock_idx } => {
                    // Temporarily remove the FinallyTry fblock so nested return/break/continue
                    // in the finally body won't see it again
                    let code = self.current_code_info();
                    let saved_fblock = code.fblock.remove(fblock_idx);

                    // Push PopValue fblock if preserving tos
                    // IMPORTANT: When preserving TOS (return value), we need to update the
                    // exception handler's stack_depth to account for the return value on stack.
                    // Otherwise, if an exception occurs during the finally body, the stack
                    // will be unwound to the wrong depth and the return value will be lost.
                    if preserve_tos {
                        // Get the handler info from the saved fblock (or current handler)
                        // and create a new handler with stack_depth + 1
                        let (handler, stack_depth, preserve_lasti) =
                            if let Some(handler) = saved_fblock.fb_handler {
                                (
                                    Some(handler),
                                    saved_fblock.fb_stack_depth + 1, // +1 for return value
                                    saved_fblock.fb_preserve_lasti,
                                )
                            } else {
                                // No handler in saved_fblock, check current handler
                                if let Some(current_handler) = self.current_except_handler() {
                                    (
                                        Some(current_handler.handler_block),
                                        current_handler.stack_depth + 1, // +1 for return value
                                        current_handler.preserve_lasti,
                                    )
                                } else {
                                    (None, 1, false) // No handler, but still track the return value
                                }
                            };

                        self.push_fblock_with_handler(
                            FBlockType::PopValue,
                            saved_fblock.fb_block,
                            saved_fblock.fb_block,
                            handler,
                            stack_depth,
                            preserve_lasti,
                        )?;
                    }

                    self.compile_statements(&body)?;

                    if preserve_tos {
                        self.pop_fblock(FBlockType::PopValue);
                    }

                    // Restore the fblock
                    let code = self.current_code_info();
                    code.fblock.insert(fblock_idx, saved_fblock);
                }
            }
        }

        Ok(())
    }

    /// Get the current exception handler from fblock stack
    fn current_except_handler(&self) -> Option<ir::ExceptHandlerInfo> {
        let code = self.code_stack.last()?;
        // Walk fblock stack from top to find the nearest exception handler
        for fblock in code.fblock.iter().rev() {
            if let Some(handler) = fblock.fb_handler {
                return Some(ir::ExceptHandlerInfo {
                    handler_block: handler,
                    stack_depth: fblock.fb_stack_depth,
                    preserve_lasti: fblock.fb_preserve_lasti,
                });
            }
        }
        None
    }

    // could take impl Into<Cow<str>>, but everything is borrowed from ast structs; we never
    // actually have a `String` to pass
    fn name(&mut self, name: &str) -> bytecode::NameIdx {
        self._name_inner(name, |i| &mut i.metadata.names)
    }
    fn varname(&mut self, name: &str) -> CompileResult<bytecode::NameIdx> {
        if Self::is_forbidden_arg_name(name) {
            return Err(self.error(CodegenErrorType::SyntaxError(format!(
                "cannot assign to {name}",
            ))));
        }
        Ok(self._name_inner(name, |i| &mut i.metadata.varnames))
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

    /// Set the qualified name for the current code object
    // = compiler_set_qualname
    fn set_qualname(&mut self) -> String {
        let qualname = self.make_qualname();
        self.current_code_info().metadata.qualname = Some(qualname.clone());
        qualname
    }
    fn make_qualname(&mut self) -> String {
        let stack_size = self.code_stack.len();
        assert!(stack_size >= 1);

        let current_obj_name = self.current_code_info().metadata.name.clone();

        // If we're at the module level (stack_size == 1), qualname is just the name
        if stack_size <= 1 {
            return current_obj_name;
        }

        // Check parent scope
        let mut parent_idx = stack_size - 2;
        let mut parent = &self.code_stack[parent_idx];

        // If parent is TypeParams scope, look at grandparent
        // Check if parent is a type params scope by name pattern
        if parent.metadata.name.starts_with("<generic parameters of ") {
            if stack_size == 2 {
                // If we're immediately within the module, qualname is just the name
                return current_obj_name;
            }
            // Use grandparent
            parent_idx = stack_size - 3;
            parent = &self.code_stack[parent_idx];
        }

        // Check if this is a global class/function
        let mut force_global = false;
        if stack_size > self.symbol_table_stack.len() {
            // We might be in a situation where symbol table isn't pushed yet
            // In this case, check the parent symbol table
            if let Some(parent_table) = self.symbol_table_stack.last()
                && let Some(symbol) = parent_table.lookup(&current_obj_name)
                && symbol.scope == SymbolScope::GlobalExplicit
            {
                force_global = true;
            }
        } else if let Some(_current_table) = self.symbol_table_stack.last() {
            // Mangle the name if necessary (for private names in classes)
            let mangled_name = self.mangle(&current_obj_name);

            // Look up in parent symbol table to check scope
            if self.symbol_table_stack.len() >= 2 {
                let parent_table = &self.symbol_table_stack[self.symbol_table_stack.len() - 2];
                if let Some(symbol) = parent_table.lookup(&mangled_name)
                    && symbol.scope == SymbolScope::GlobalExplicit
                {
                    force_global = true;
                }
            }
        }

        // Build the qualified name
        if force_global {
            // For global symbols, qualname is just the name
            current_obj_name
        } else {
            // Check parent scope type
            let parent_obj_name = &parent.metadata.name;

            // Determine if parent is a function-like scope
            let is_function_parent = parent.flags.contains(bytecode::CodeFlags::OPTIMIZED)
                && !parent_obj_name.starts_with("<") // Not a special scope like <lambda>, <listcomp>, etc.
                && parent_obj_name != "<module>"; // Not the module scope

            if is_function_parent {
                // For functions, append .<locals> to parent qualname
                // Use parent's qualname if available, otherwise use parent_obj_name
                let parent_qualname = parent.metadata.qualname.as_ref().unwrap_or(parent_obj_name);
                format!("{parent_qualname}.<locals>.{current_obj_name}")
            } else {
                // For classes and other scopes, use parent's qualname directly
                // Use parent's qualname if available, otherwise use parent_obj_name
                let parent_qualname = parent.metadata.qualname.as_ref().unwrap_or(parent_obj_name);
                if parent_qualname == "<module>" {
                    // Module level, just use the name
                    current_obj_name
                } else {
                    // Concatenate parent qualname with current name
                    format!("{parent_qualname}.{current_obj_name}")
                }
            }
        }
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
            emit!(self, Instruction::SetupAnnotations);
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

        if Self::find_ann(body) {
            emit!(self, Instruction::SetupAnnotations);
        }

        if let Some((last, body)) = body.split_last() {
            for statement in body {
                if let Stmt::Expr(StmtExpr { value, .. }) = &statement {
                    self.compile_expression(value)?;
                    emit!(
                        self,
                        Instruction::CallIntrinsic1 {
                            func: bytecode::IntrinsicFunction1::Print
                        }
                    );

                    emit!(self, Instruction::PopTop);
                } else {
                    self.compile_statement(statement)?;
                }
            }

            if let Stmt::Expr(StmtExpr { value, .. }) = &last {
                self.compile_expression(value)?;
                emit!(self, Instruction::CopyItem { index: 1_u32 });
                emit!(
                    self,
                    Instruction::CallIntrinsic1 {
                        func: bytecode::IntrinsicFunction1::Print
                    }
                );

                emit!(self, Instruction::PopTop);
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
                    self.current_block().instructions.pop(); // pop Instruction::PopTop
                }
                Stmt::FunctionDef(_) | Stmt::ClassDef(_) => {
                    let pop_instructions = self.current_block().instructions.pop();
                    let store_inst = compiler_unwrap_option(self, pop_instructions); // pop Instruction::Store
                    emit!(self, Instruction::CopyItem { index: 1_u32 });
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
        // Use private from current code unit for name mangling
        let private = self
            .code_stack
            .last()
            .and_then(|info| info.private.as_deref());
        symboltable::mangle_name(private, name)
    }

    fn check_forbidden_name(&mut self, name: &str, usage: NameUsage) -> CompileResult<()> {
        let msg = match usage {
            NameUsage::Store if is_forbidden_name(name) => "cannot assign to",
            NameUsage::Delete if is_forbidden_name(name) => "cannot delete",
            _ => return Ok(()),
        };
        Err(self.error(CodegenErrorType::SyntaxError(format!("{msg} {name}"))))
    }

    // = compiler_nameop
    fn compile_name(&mut self, name: &str, usage: NameUsage) -> CompileResult<()> {
        enum NameOp {
            Fast,
            Global,
            Deref,
            Name,
        }

        let name = self.mangle(name);
        self.check_forbidden_name(&name, usage)?;

        // Special handling for __debug__
        if NameUsage::Load == usage && name == "__debug__" {
            self.emit_load_const(ConstantData::Boolean {
                value: self.opts.optimize == 0,
            });
            return Ok(());
        }

        // Determine the operation type based on symbol scope
        let is_function_like = self.ctx.in_func();

        // Look up the symbol, handling TypeParams scope specially
        let (symbol_scope, _is_typeparams) = {
            let current_table = self.current_symbol_table();
            let is_typeparams = current_table.typ == CompilerScope::TypeParams;

            // First try to find in current table
            let symbol = current_table.lookup(name.as_ref());

            // If not found and we're in TypeParams scope, try parent scope
            let symbol = if symbol.is_none() && is_typeparams {
                self.symbol_table_stack
                    .get(self.symbol_table_stack.len() - 2) // Try to get parent index
                    .expect("Symbol has no parent! This is a compiler bug.")
                    .lookup(name.as_ref())
            } else {
                symbol
            };

            (symbol.map(|s| s.scope), is_typeparams)
        };

        let actual_scope = symbol_scope.ok_or_else(|| {
            self.error(CodegenErrorType::SyntaxError(format!(
                "The symbol '{name}' must be present in the symbol table"
            )))
        })?;

        // Determine operation type based on scope
        let op_type = match actual_scope {
            SymbolScope::Free => NameOp::Deref,
            SymbolScope::Cell => NameOp::Deref,
            SymbolScope::Local => {
                if is_function_like {
                    NameOp::Fast
                } else {
                    NameOp::Name
                }
            }
            SymbolScope::GlobalImplicit => {
                if is_function_like {
                    NameOp::Global
                } else {
                    NameOp::Name
                }
            }
            SymbolScope::GlobalExplicit => NameOp::Global,
            SymbolScope::Unknown => NameOp::Name,
        };

        // Generate appropriate instructions based on operation type
        match op_type {
            NameOp::Deref => {
                let idx = match actual_scope {
                    SymbolScope::Free => self.get_free_var_index(&name)?,
                    SymbolScope::Cell => self.get_cell_var_index(&name)?,
                    _ => unreachable!("Invalid scope for Deref operation"),
                };

                let op = match usage {
                    NameUsage::Load => {
                        // Special case for class scope
                        if self.ctx.in_class && !self.ctx.in_func() {
                            Instruction::LoadFromDictOrDeref
                        } else {
                            Instruction::LoadDeref
                        }
                    }
                    NameUsage::Store => Instruction::StoreDeref,
                    NameUsage::Delete => Instruction::DeleteDeref,
                };
                self.emit_arg(idx, op);
            }
            NameOp::Fast => {
                let idx = self.get_local_var_index(&name)?;
                let op = match usage {
                    NameUsage::Load => Instruction::LoadFast,
                    NameUsage::Store => Instruction::StoreFast,
                    NameUsage::Delete => Instruction::DeleteFast,
                };
                self.emit_arg(idx, op);
            }
            NameOp::Global => {
                let idx = self.get_global_name_index(&name);
                let op = match usage {
                    NameUsage::Load => Instruction::LoadGlobal,
                    NameUsage::Store => Instruction::StoreGlobal,
                    NameUsage::Delete => Instruction::DeleteGlobal,
                };
                self.emit_arg(idx, op);
            }
            NameOp::Name => {
                let idx = self.get_global_name_index(&name);
                let op = match usage {
                    NameUsage::Load => Instruction::LoadName,
                    NameUsage::Store => Instruction::StoreName,
                    NameUsage::Delete => Instruction::DeleteName,
                };
                self.emit_arg(idx, op);
            }
        }

        Ok(())
    }

    fn compile_statement(&mut self, statement: &Stmt) -> CompileResult<()> {
        use ruff_python_ast::*;
        trace!("Compiling {statement:?}");
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

                // from .... import (*fromlist)
                self.emit_load_const(ConstantData::Integer {
                    value: (*level).into(),
                });
                self.emit_load_const(ConstantData::Tuple {
                    elements: from_list,
                });

                let module_name = module.as_ref().map_or("", |s| s.as_str());
                let module_idx = self.name(module_name);
                emit!(self, Instruction::ImportName { idx: module_idx });

                if import_star {
                    // from .... import *
                    emit!(
                        self,
                        Instruction::CallIntrinsic1 {
                            func: bytecode::IntrinsicFunction1::ImportStar
                        }
                    );
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
                    emit!(self, Instruction::PopTop);
                }
            }
            Stmt::Expr(StmtExpr { value, .. }) => {
                self.compile_expression(value)?;

                // Pop result of stack, since we not use it:
                emit!(self, Instruction::PopTop);
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
            Stmt::Raise(StmtRaise {
                exc, cause, range, ..
            }) => {
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
                    None => bytecode::RaiseKind::BareRaise,
                };
                self.set_source_range(*range);
                emit!(self, Instruction::RaiseVarargs { kind });
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
                    self.compile_try_star_except(body, handlers, orelse, finalbody)?
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
            }) => {
                validate_duplicate_params(parameters).map_err(|e| self.error(e))?;

                self.compile_function_def(
                    name.as_str(),
                    parameters,
                    body,
                    decorator_list,
                    returns.as_deref(),
                    *is_async,
                    type_params.as_deref(),
                )?
            }
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
                    emit!(self, Instruction::PushNull);
                    match msg {
                        Some(e) => {
                            self.compile_expression(e)?;
                            emit!(self, Instruction::Call { nargs: 1 });
                        }
                        None => {
                            emit!(self, Instruction::Call { nargs: 0 });
                        }
                    }
                    emit!(
                        self,
                        Instruction::RaiseVarargs {
                            kind: bytecode::RaiseKind::Raise,
                        }
                    );

                    self.switch_to_block(after_block);
                }
            }
            Stmt::Break(_) => {
                // Unwind fblock stack until we find a loop, emitting cleanup for each fblock
                self.compile_break_continue(statement.range(), true)?;
            }
            Stmt::Continue(_) => {
                // Unwind fblock stack until we find a loop, emitting cleanup for each fblock
                self.compile_break_continue(statement.range(), false)?;
            }
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
                                .contains(bytecode::CodeFlags::GENERATOR)
                        {
                            return Err(self.error_ranged(
                                CodegenErrorType::AsyncReturnValue,
                                statement.range(),
                            ));
                        }
                        self.compile_expression(v)?;
                        // Unwind fblock stack with preserve_tos=true (preserve return value)
                        self.unwind_fblock_stack(true, false)?;
                        self.emit_return_value();
                    }
                    None => {
                        // Unwind fblock stack with preserve_tos=false (no value to preserve)
                        self.unwind_fblock_stack(false, false)?;
                        self.emit_return_const(ConstantData::None);
                    }
                }
            }
            Stmt::Assign(StmtAssign { targets, value, .. }) => {
                self.compile_expression(value)?;

                for (i, target) in targets.iter().enumerate() {
                    if i + 1 != targets.len() {
                        emit!(self, Instruction::CopyItem { index: 1_u32 });
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

                // For PEP 695 syntax, we need to compile type_params first
                // so that they're available when compiling the value expression
                // Push name first
                self.emit_load_const(ConstantData::Str {
                    value: name_string.clone().into(),
                });

                if let Some(type_params) = type_params {
                    // For TypeAlias, we need to use push_symbol_table to properly handle the TypeAlias scope
                    self.push_symbol_table()?;

                    // Compile type params and push to stack
                    self.compile_type_params(type_params)?;
                    // Stack now has [name, type_params_tuple]

                    // Compile value expression (can now see T1, T2)
                    self.compile_expression(value)?;
                    // Stack: [name, type_params_tuple, value]

                    // Pop the TypeAlias scope
                    self.pop_symbol_table();
                } else {
                    // Push None for type_params
                    self.emit_load_const(ConstantData::None);
                    // Stack: [name, None]

                    // Compile value expression
                    self.compile_expression(value)?;
                    // Stack: [name, None, value]
                }

                // Build tuple of 3 elements and call intrinsic
                emit!(self, Instruction::BuildTuple { size: 3 });
                emit!(
                    self,
                    Instruction::CallIntrinsic1 {
                        func: bytecode::IntrinsicFunction1::TypeAlias
                    }
                );
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
            Expr::Subscript(ExprSubscript {
                value, slice, ctx, ..
            }) => {
                self.compile_subscript(value, slice, *ctx)?;
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

    fn enter_function(&mut self, name: &str, parameters: &Parameters) -> CompileResult<()> {
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

        self.push_output(
            bytecode::CodeFlags::NEWLOCALS | bytecode::CodeFlags::OPTIMIZED,
            parameters.posonlyargs.len().to_u32(),
            (parameters.posonlyargs.len() + parameters.args.len()).to_u32(),
            parameters.kwonlyargs.len().to_u32(),
            name.to_owned(),
        )?;

        let args_iter = core::iter::empty()
            .chain(&parameters.posonlyargs)
            .chain(&parameters.args)
            .map(|arg| &arg.parameter)
            .chain(kw_without_defaults)
            .chain(kw_with_defaults.into_iter().map(|(arg, _)| arg));
        for name in args_iter {
            self.varname(name.name.as_str())?;
        }

        if let Some(name) = parameters.vararg.as_deref() {
            self.current_code_info().flags |= bytecode::CodeFlags::VARARGS;
            self.varname(name.name.as_str())?;
        }
        if let Some(name) = parameters.kwarg.as_deref() {
            self.current_code_info().flags |= bytecode::CodeFlags::VARKEYWORDS;
            self.varname(name.name.as_str())?;
        }

        Ok(())
    }

    /// Push decorators onto the stack in source order.
    /// For @dec1 @dec2 def foo(): stack becomes [dec1, NULL, dec2, NULL]
    fn prepare_decorators(&mut self, decorator_list: &[Decorator]) -> CompileResult<()> {
        for decorator in decorator_list {
            self.compile_expression(&decorator.expression)?;
            emit!(self, Instruction::PushNull);
        }
        Ok(())
    }

    /// Apply decorators in reverse order (LIFO from stack).
    /// Stack [dec1, NULL, dec2, NULL, func] -> dec2(func) -> dec1(dec2(func))
    /// The forward loop works because each Call pops from TOS, naturally
    /// applying decorators bottom-up (innermost first).
    fn apply_decorators(&mut self, decorator_list: &[Decorator]) {
        for _ in decorator_list {
            emit!(self, Instruction::Call { nargs: 1 });
        }
    }

    /// Compile type parameter bound or default in a separate scope and return closure
    fn compile_type_param_bound_or_default(
        &mut self,
        expr: &Expr,
        name: &str,
        allow_starred: bool,
    ) -> CompileResult<()> {
        // Push the next symbol table onto the stack
        self.push_symbol_table()?;

        // Get the current symbol table
        let key = self.symbol_table_stack.len() - 1;
        let lineno = expr.range().start().to_u32();

        // Enter scope with the type parameter name
        self.enter_scope(name, CompilerScope::TypeParams, key, lineno)?;

        // Compile the expression
        if allow_starred && matches!(expr, Expr::Starred(_)) {
            if let Expr::Starred(starred) = expr {
                self.compile_expression(&starred.value)?;
                emit!(self, Instruction::UnpackSequence { size: 1 });
            }
        } else {
            self.compile_expression(expr)?;
        }

        // Return value
        emit!(self, Instruction::ReturnValue);

        // Exit scope and create closure
        let code = self.exit_scope();
        // Note: exit_scope already calls pop_symbol_table, so we don't need to call it again

        // Create type params function with closure
        self.make_closure(code, bytecode::MakeFunctionFlags::empty())?;
        emit!(self, Instruction::PushNull);

        // Call the function immediately
        emit!(self, Instruction::Call { nargs: 0 });

        Ok(())
    }

    /// Store each type parameter so it is accessible to the current scope, and leave a tuple of
    /// all the type parameters on the stack. Handles default values per PEP 695.
    fn compile_type_params(&mut self, type_params: &TypeParams) -> CompileResult<()> {
        // First, compile each type parameter and store it
        for type_param in &type_params.type_params {
            match type_param {
                TypeParam::TypeVar(TypeParamTypeVar {
                    name,
                    bound,
                    default,
                    ..
                }) => {
                    self.emit_load_const(ConstantData::Str {
                        value: name.as_str().into(),
                    });

                    if let Some(expr) = &bound {
                        let scope_name = if expr.is_tuple_expr() {
                            format!("<TypeVar constraint of {name}>")
                        } else {
                            format!("<TypeVar bound of {name}>")
                        };
                        self.compile_type_param_bound_or_default(expr, &scope_name, false)?;

                        let intrinsic = if expr.is_tuple_expr() {
                            bytecode::IntrinsicFunction2::TypeVarWithConstraint
                        } else {
                            bytecode::IntrinsicFunction2::TypeVarWithBound
                        };
                        emit!(self, Instruction::CallIntrinsic2 { func: intrinsic });
                    } else {
                        emit!(
                            self,
                            Instruction::CallIntrinsic1 {
                                func: bytecode::IntrinsicFunction1::TypeVar
                            }
                        );
                    }

                    if let Some(default_expr) = default {
                        let scope_name = format!("<TypeVar default of {name}>");
                        self.compile_type_param_bound_or_default(default_expr, &scope_name, false)?;
                        emit!(
                            self,
                            Instruction::CallIntrinsic2 {
                                func: bytecode::IntrinsicFunction2::SetTypeparamDefault
                            }
                        );
                    }

                    emit!(self, Instruction::CopyItem { index: 1_u32 });
                    self.store_name(name.as_ref())?;
                }
                TypeParam::ParamSpec(TypeParamParamSpec { name, default, .. }) => {
                    self.emit_load_const(ConstantData::Str {
                        value: name.as_str().into(),
                    });
                    emit!(
                        self,
                        Instruction::CallIntrinsic1 {
                            func: bytecode::IntrinsicFunction1::ParamSpec
                        }
                    );

                    if let Some(default_expr) = default {
                        let scope_name = format!("<ParamSpec default of {name}>");
                        self.compile_type_param_bound_or_default(default_expr, &scope_name, false)?;
                        emit!(
                            self,
                            Instruction::CallIntrinsic2 {
                                func: bytecode::IntrinsicFunction2::SetTypeparamDefault
                            }
                        );
                    }

                    emit!(self, Instruction::CopyItem { index: 1_u32 });
                    self.store_name(name.as_ref())?;
                }
                TypeParam::TypeVarTuple(TypeParamTypeVarTuple { name, default, .. }) => {
                    self.emit_load_const(ConstantData::Str {
                        value: name.as_str().into(),
                    });
                    emit!(
                        self,
                        Instruction::CallIntrinsic1 {
                            func: bytecode::IntrinsicFunction1::TypeVarTuple
                        }
                    );

                    if let Some(default_expr) = default {
                        // TypeVarTuple allows starred expressions
                        let scope_name = format!("<TypeVarTuple default of {name}>");
                        self.compile_type_param_bound_or_default(default_expr, &scope_name, true)?;
                        emit!(
                            self,
                            Instruction::CallIntrinsic2 {
                                func: bytecode::IntrinsicFunction2::SetTypeparamDefault
                            }
                        );
                    }

                    emit!(self, Instruction::CopyItem { index: 1_u32 });
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

        // finally needs TWO blocks:
        // - finally_block: normal path (no exception active)
        // - finally_except_block: exception path (PUSH_EXC_INFO -> body -> RERAISE)
        let finally_except_block = if !finalbody.is_empty() {
            Some(self.new_block())
        } else {
            None
        };
        let finally_cleanup_block = if finally_except_block.is_some() {
            Some(self.new_block())
        } else {
            None
        };
        // End block - continuation point after try-finally
        // Normal path jumps here to skip exception path blocks
        let end_block = self.new_block();

        // Calculate the stack depth at this point (for exception table)
        // SETUP_FINALLY captures current stack depth
        let current_depth = self.handler_stack_depth();

        // Setup a finally block if we have a finally statement.
        // Push fblock with handler info for exception table generation
        // IMPORTANT: handler goes to finally_except_block (exception path), not finally_block
        if !finalbody.is_empty() {
            // No SetupFinally emit - exception table handles this
            // Store finally body in fb_datum for unwind_fblock to compile inline
            // SETUP_FINALLY doesn't push lasti for try body handler
            // Exception table: L1 to L2 -> L4 [1] (no lasti)
            self.push_fblock_full(
                FBlockType::FinallyTry,
                finally_block,
                finally_block,
                finally_except_block, // Exception path goes to finally_except_block
                current_depth,
                false, // No lasti for first finally handler
                FBlockDatum::FinallyBody(finalbody.to_vec()), // Clone finally body for unwind
            )?;
        }

        let else_block = self.new_block();

        // if handlers is empty, compile body directly
        // without wrapping in TryExcept (only FinallyTry is needed)
        if handlers.is_empty() {
            // Just compile body with FinallyTry fblock active (if finalbody exists)
            self.compile_statements(body)?;

            // Pop FinallyTry fblock BEFORE compiling orelse/finally (normal path)
            // This prevents exception table from covering the normal path
            if !finalbody.is_empty() {
                self.pop_fblock(FBlockType::FinallyTry);
            }

            // Compile orelse (usually empty for try-finally without except)
            self.compile_statements(orelse)?;

            // Snapshot sub_tables before first finally compilation
            // This allows us to restore them for the second compilation (exception path)
            let sub_table_cursor = if !finalbody.is_empty() && finally_except_block.is_some() {
                self.symbol_table_stack.last().map(|t| t.next_sub_table)
            } else {
                None
            };

            // Compile finally body inline for normal path
            if !finalbody.is_empty() {
                self.compile_statements(finalbody)?;
            }

            // Jump to end (skip exception path blocks)
            emit!(self, Instruction::Jump { target: end_block });

            if let Some(finally_except) = finally_except_block {
                // Restore sub_tables for exception path compilation
                if let Some(cursor) = sub_table_cursor
                    && let Some(current_table) = self.symbol_table_stack.last_mut()
                {
                    current_table.next_sub_table = cursor;
                }

                self.switch_to_block(finally_except);
                // PUSH_EXC_INFO first, THEN push FinallyEnd fblock
                // Stack after unwind (no lasti): [exc] (depth = current_depth + 1)
                // Stack after PUSH_EXC_INFO: [prev_exc, exc] (depth = current_depth + 2)
                emit!(self, Instruction::PushExcInfo);
                if let Some(cleanup) = finally_cleanup_block {
                    // FinallyEnd fblock must be pushed AFTER PUSH_EXC_INFO
                    // Depth = current_depth + 1 (only prev_exc remains after RERAISE pops exc)
                    // Exception table: L4 to L5 -> L6 [2] lasti (cleanup handler DOES push lasti)
                    self.push_fblock_with_handler(
                        FBlockType::FinallyEnd,
                        cleanup,
                        cleanup,
                        Some(cleanup),
                        current_depth + 1,
                        true, // Cleanup handler pushes lasti
                    )?;
                }
                self.compile_statements(finalbody)?;
                // RERAISE 0 is emitted BEFORE pop_fblock
                // This ensures RERAISE goes to cleanup block (FinallyEnd handler)
                // which then properly restores prev_exc before going to outer handler
                emit!(
                    self,
                    Instruction::RaiseVarargs {
                        kind: bytecode::RaiseKind::ReraiseFromStack
                    }
                );
                if finally_cleanup_block.is_some() {
                    self.pop_fblock(FBlockType::FinallyEnd);
                }
            }

            if let Some(cleanup) = finally_cleanup_block {
                self.switch_to_block(cleanup);
                emit!(self, Instruction::CopyItem { index: 3_u32 });
                emit!(self, Instruction::PopExcept);
                emit!(
                    self,
                    Instruction::RaiseVarargs {
                        kind: bytecode::RaiseKind::ReraiseFromStack
                    }
                );
            }

            self.switch_to_block(end_block);
            return Ok(());
        }

        // try:
        // Push fblock with handler info for exception table generation
        // No SetupExcept emit - exception table handles this
        self.push_fblock_with_handler(
            FBlockType::TryExcept,
            handler_block,
            handler_block,
            Some(handler_block),
            current_depth, // stack depth for exception handler
            false,         // no lasti for except
        )?;
        self.compile_statements(body)?;
        self.pop_fblock(FBlockType::TryExcept);
        // No PopBlock emit - exception table handles this
        emit!(self, Instruction::Jump { target: else_block });

        // except handlers:
        self.switch_to_block(handler_block);

        // SETUP_CLEANUP(cleanup) for except block
        // This handles exceptions during exception matching
        // Exception table: L2 to L3 -> L5 [1] lasti
        // After PUSH_EXC_INFO, stack is [prev_exc, exc]
        // depth=1 means keep prev_exc on stack when routing to cleanup
        let cleanup_block = self.new_block();
        self.push_fblock_with_handler(
            FBlockType::ExceptionHandler,
            cleanup_block,
            cleanup_block,
            Some(cleanup_block),
            current_depth + 1, // After PUSH_EXC_INFO: [prev_exc] stays on stack
            true,              // preserve_lasti for cleanup
        )?;

        // Exception is on top of stack now, pushed by unwind_blocks
        // PUSH_EXC_INFO transforms [exc] -> [prev_exc, exc] for PopExcept
        emit!(self, Instruction::PushExcInfo);
        for handler in handlers {
            let ExceptHandler::ExceptHandler(ExceptHandlerExceptHandler {
                type_, name, body, ..
            }) = &handler;
            let next_handler = self.new_block();

            // If we gave a typ,
            // check if this handler can handle the exception:
            if let Some(exc_type) = type_ {
                // Duplicate exception for test:
                emit!(self, Instruction::CopyItem { index: 1_u32 });

                // Check exception type:
                self.compile_expression(exc_type)?;
                emit!(self, Instruction::JumpIfNotExcMatch(next_handler));

                // We have a match, store in name (except x as y)
                if let Some(alias) = name {
                    self.store_name(alias.as_str())?
                } else {
                    // Drop exception from top of stack:
                    emit!(self, Instruction::PopTop);
                }
            } else {
                // Catch all!
                // Drop exception from top of stack:
                emit!(self, Instruction::PopTop);
            }

            // If name is bound, we need a cleanup handler for RERAISE
            let handler_cleanup_block = if name.is_some() {
                // SETUP_CLEANUP(cleanup_end) for named handler
                let cleanup_end = self.new_block();
                // Stack at handler entry: [prev_exc, exc]
                // depth = 1 (prev_exc on stack after exception is popped)
                let handler_depth = current_depth + 1;
                self.push_fblock_with_handler(
                    FBlockType::HandlerCleanup,
                    cleanup_end,
                    cleanup_end,
                    Some(cleanup_end),
                    handler_depth,
                    true, // preserve_lasti for RERAISE
                )?;
                Some(cleanup_end)
            } else {
                // no SETUP_CLEANUP for unnamed handler
                self.push_fblock(FBlockType::HandlerCleanup, finally_block, finally_block)?;
                None
            };

            // Handler code:
            self.compile_statements(body)?;

            self.pop_fblock(FBlockType::HandlerCleanup);

            // Create a block for normal path continuation (after handler body succeeds)
            let handler_normal_exit = self.new_block();
            emit!(
                self,
                Instruction::Jump {
                    target: handler_normal_exit,
                }
            );

            // cleanup_end block for named handler
            // IMPORTANT: In CPython, cleanup_end is within outer SETUP_CLEANUP scope.
            // so when RERAISE is executed, it goes to the cleanup block which does POP_EXCEPT.
            // We MUST compile cleanup_end BEFORE popping ExceptionHandler so RERAISE routes to cleanup_block.
            if let Some(cleanup_end) = handler_cleanup_block {
                self.switch_to_block(cleanup_end);
                if let Some(alias) = name {
                    // name = None; del name; before RERAISE
                    self.emit_load_const(ConstantData::None);
                    self.store_name(alias.as_str())?;
                    self.compile_name(alias.as_str(), NameUsage::Delete)?;
                }
                // RERAISE 1 (with lasti) - exception is on stack from exception table routing
                // Stack at entry: [prev_exc (at handler_depth), lasti, exc]
                // This RERAISE is within ExceptionHandler scope, so it routes to cleanup_block
                // which does COPY 3; POP_EXCEPT; RERAISE
                emit!(
                    self,
                    Instruction::RaiseVarargs {
                        kind: bytecode::RaiseKind::ReraiseFromStack,
                    }
                );
            }

            // Switch to normal exit block - this is where handler body success continues
            self.switch_to_block(handler_normal_exit);

            // Now pop ExceptionHandler - the normal path continues from here
            // POP_BLOCK (HandlerCleanup) then POP_BLOCK (SETUP_CLEANUP)
            // followed by POP_EXCEPT
            self.pop_fblock(FBlockType::ExceptionHandler);
            emit!(self, Instruction::PopExcept);

            // Delete the exception variable if it was bound (normal path)
            if let Some(alias) = name {
                // Set the variable to None before deleting
                self.emit_load_const(ConstantData::None);
                self.store_name(alias.as_str())?;
                self.compile_name(alias.as_str(), NameUsage::Delete)?;
            }

            // Jump to finally block
            emit!(
                self,
                Instruction::Jump {
                    target: finally_block,
                }
            );

            // Re-push ExceptionHandler for next handler in the loop
            // This will be popped at the end of handlers loop or when matched
            self.push_fblock_with_handler(
                FBlockType::ExceptionHandler,
                cleanup_block,
                cleanup_block,
                Some(cleanup_block),
                current_depth + 1, // After PUSH_EXC_INFO: [prev_exc] stays on stack
                true,              // preserve_lasti for cleanup
            )?;

            // Emit a new label for the next handler
            self.switch_to_block(next_handler);
        }

        // If code flows here, we have an unhandled exception,
        // raise the exception again!
        // RERAISE 0
        // Stack: [prev_exc, exc] - exception is on stack from PUSH_EXC_INFO
        // NOTE: We emit RERAISE 0 BEFORE popping fblock so it is within cleanup handler scope
        emit!(
            self,
            Instruction::RaiseVarargs {
                kind: bytecode::RaiseKind::ReraiseFromStack,
            }
        );

        // Pop EXCEPTION_HANDLER fblock
        // Pop after RERAISE so the instruction has the correct exception handler
        self.pop_fblock(FBlockType::ExceptionHandler);

        // cleanup block (POP_EXCEPT_AND_RERAISE)
        // Stack at entry: [prev_exc, lasti, exc] (depth=1 + lasti + exc pushed)
        // COPY 3: copy prev_exc to top -> [prev_exc, lasti, exc, prev_exc]
        // POP_EXCEPT: pop prev_exc from stack and restore -> [prev_exc, lasti, exc]
        // RERAISE 1: reraise with lasti
        self.switch_to_block(cleanup_block);
        emit!(self, Instruction::CopyItem { index: 3_u32 });
        emit!(self, Instruction::PopExcept);
        emit!(
            self,
            Instruction::RaiseVarargs {
                kind: bytecode::RaiseKind::ReraiseFromStack,
            }
        );

        // We successfully ran the try block:
        // else:
        self.switch_to_block(else_block);
        self.compile_statements(orelse)?;

        // Pop the FinallyTry fblock before jumping to finally
        if !finalbody.is_empty() {
            // No PopBlock/EnterFinally emit - exception table handles this
            self.pop_fblock(FBlockType::FinallyTry);
        }

        // Snapshot sub_tables before first finally compilation (for double compilation issue)
        let sub_table_cursor = if !finalbody.is_empty() && finally_except_block.is_some() {
            self.symbol_table_stack.last().map(|t| t.next_sub_table)
        } else {
            None
        };

        // finally (normal path):
        self.switch_to_block(finally_block);
        if !finalbody.is_empty() {
            self.compile_statements(finalbody)?;
            // Jump to end_block to skip exception path blocks
            // This prevents fall-through to finally_except_block
            emit!(self, Instruction::Jump { target: end_block });
        }

        // finally (exception path)
        // This is where exceptions go to run finally before reraise
        // Stack at entry: [lasti, exc] (from exception table with preserve_lasti=true)
        if let Some(finally_except) = finally_except_block {
            // Restore sub_tables for exception path compilation
            if let Some(cursor) = sub_table_cursor
                && let Some(current_table) = self.symbol_table_stack.last_mut()
            {
                current_table.next_sub_table = cursor;
            }

            self.switch_to_block(finally_except);

            // SETUP_CLEANUP for finally body
            // Exceptions during finally body need to go to cleanup block
            // Stack at entry: [lasti, exc] (lasti from exception table, exc pushed)
            // After PUSH_EXC_INFO: [lasti, prev_exc, exc]
            // So depth should account for lasti being on stack
            if let Some(cleanup) = finally_cleanup_block {
                self.push_fblock_with_handler(
                    FBlockType::FinallyEnd,
                    cleanup,
                    cleanup,
                    Some(cleanup),
                    current_depth + 1, // [lasti] on stack before PUSH_EXC_INFO
                    true,
                )?;
            }

            // PUSH_EXC_INFO: [lasti, exc] -> [lasti, prev_exc, exc]
            // Sets exc as current VM exception, saves prev_exc for restoration
            emit!(self, Instruction::PushExcInfo);

            // Run finally body
            self.compile_statements(finalbody)?;

            // RERAISE 0 is emitted BEFORE pop_fblock
            // This ensures RERAISE goes to cleanup block (FinallyEnd handler)
            // which then properly restores prev_exc before going to outer handler
            // RERAISE 0: reraise the exception on TOS
            // Stack: [lasti, prev_exc, exc] - exception is on top
            emit!(
                self,
                Instruction::RaiseVarargs {
                    kind: bytecode::RaiseKind::ReraiseFromStack,
                }
            );

            if finally_cleanup_block.is_some() {
                self.pop_fblock(FBlockType::FinallyEnd);
            }
        }

        // finally cleanup block
        // This handles exceptions that occur during the finally body itself
        // Stack at entry: [lasti, prev_exc, lasti2, exc2] after exception table routing
        if let Some(cleanup) = finally_cleanup_block {
            self.switch_to_block(cleanup);
            // COPY 3: copy the exception from position 3
            emit!(self, Instruction::CopyItem { index: 3_u32 });
            // POP_EXCEPT: restore prev_exc as current exception
            emit!(self, Instruction::PopExcept);
            // RERAISE 1: reraise with lasti from stack
            emit!(
                self,
                Instruction::RaiseVarargs {
                    kind: bytecode::RaiseKind::ReraiseFromStack,
                }
            );
        }

        // End block - continuation point after try-finally
        // Normal execution continues here after the finally block
        self.switch_to_block(end_block);

        Ok(())
    }

    fn compile_try_star_except(
        &mut self,
        body: &[Stmt],
        handlers: &[ExceptHandler],
        orelse: &[Stmt],
        finalbody: &[Stmt],
    ) -> CompileResult<()> {
        // compiler_try_star_except
        // Stack layout during handler processing: [prev_exc, orig, list, rest]
        let handler_block = self.new_block();
        let finally_block = self.new_block();
        let else_block = self.new_block();
        let end_block = self.new_block();
        let reraise_star_block = self.new_block();
        let reraise_block = self.new_block();
        let _cleanup_block = self.new_block();

        // Calculate the stack depth at this point (for exception table)
        let current_depth = self.handler_stack_depth();

        // Push fblock with handler info for exception table generation
        if !finalbody.is_empty() {
            // No SetupFinally emit - exception table handles this
            self.push_fblock_with_handler(
                FBlockType::FinallyTry,
                finally_block,
                finally_block,
                Some(finally_block),
                current_depth, // stack depth for exception handler
                true,          // preserve lasti for finally
            )?;
        }

        // SETUP_FINALLY for try body
        // Push fblock with handler info for exception table generation
        self.push_fblock_with_handler(
            FBlockType::TryExcept,
            handler_block,
            handler_block,
            Some(handler_block),
            current_depth, // stack depth for exception handler
            false,         // no lasti for except
        )?;
        self.compile_statements(body)?;
        self.pop_fblock(FBlockType::TryExcept);
        emit!(self, Instruction::Jump { target: else_block });

        // Exception handler entry
        self.switch_to_block(handler_block);
        // Stack: [exc] (from exception table)

        // PUSH_EXC_INFO
        emit!(self, Instruction::PushExcInfo);
        // Stack: [prev_exc, exc]

        // Push EXCEPTION_GROUP_HANDLER fblock
        let eg_dummy1 = self.new_block();
        let eg_dummy2 = self.new_block();
        self.push_fblock(FBlockType::ExceptionGroupHandler, eg_dummy1, eg_dummy2)?;

        let n = handlers.len();
        for (i, handler) in handlers.iter().enumerate() {
            let ExceptHandler::ExceptHandler(ExceptHandlerExceptHandler {
                type_, name, body, ..
            }) = handler;

            let no_match_block = self.new_block();
            let next_block = self.new_block();

            // first handler creates list and copies exc
            if i == 0 {
                // ADDOP_I(c, loc, BUILD_LIST, 0);
                emit!(self, Instruction::BuildList { size: 0 });
                // Stack: [prev_exc, exc, []]
                // ADDOP_I(c, loc, COPY, 2);
                emit!(self, Instruction::CopyItem { index: 2 });
                // Stack: [prev_exc, exc, [], exc_copy]
                // Now stack is: [prev_exc, orig, list, rest]
            }

            // Compile exception type
            if let Some(exc_type) = type_ {
                // Check for unparenthesized tuple
                if let Expr::Tuple(ExprTuple { elts, range, .. }) = exc_type.as_ref()
                    && let Some(first) = elts.first()
                    && range.start().to_u32() == first.range().start().to_u32()
                {
                    return Err(self.error(CodegenErrorType::SyntaxError(
                        "multiple exception types must be parenthesized".to_owned(),
                    )));
                }
                self.compile_expression(exc_type)?;
            } else {
                return Err(self.error(CodegenErrorType::SyntaxError(
                    "except* must specify an exception type".to_owned(),
                )));
            }
            // Stack: [prev_exc, orig, list, rest, type]

            // ADDOP(c, loc, CHECK_EG_MATCH);
            emit!(self, Instruction::CheckEgMatch);
            // Stack: [prev_exc, orig, list, new_rest, match]

            // ADDOP_I(c, loc, COPY, 1);
            // ADDOP_JUMP(c, loc, POP_JUMP_IF_NONE, no_match);
            emit!(self, Instruction::CopyItem { index: 1 });
            self.emit_load_const(ConstantData::None);
            emit!(self, Instruction::IsOp(bytecode::Invert::No)); // is None?
            emit!(
                self,
                Instruction::PopJumpIfTrue {
                    target: no_match_block
                }
            );

            // Handler matched
            // Stack: [prev_exc, orig, list, new_rest, match]
            let handler_except_block = self.new_block();

            // Set matched exception as current exception (for __context__ in handler body)
            // This ensures that exceptions raised in the handler get the matched part
            // as their __context__, not the original full exception group
            emit!(self, Instruction::SetExcInfo);

            // Store match to name or pop
            if let Some(alias) = name {
                self.store_name(alias.as_str())?;
            } else {
                emit!(self, Instruction::PopTop); // pop match
            }
            // Stack: [prev_exc, orig, list, new_rest]

            // HANDLER_CLEANUP fblock for handler body
            // Stack depth: prev_exc(1) + orig(1) + list(1) + new_rest(1) = 4
            let eg_handler_depth = self.handler_stack_depth() + 4;
            self.push_fblock_with_handler(
                FBlockType::HandlerCleanup,
                next_block,
                end_block,
                Some(handler_except_block),
                eg_handler_depth,
                true, // preserve lasti
            )?;

            // Execute handler body
            self.compile_statements(body)?;

            // Handler body completed normally
            self.pop_fblock(FBlockType::HandlerCleanup);

            // Cleanup name binding
            if let Some(alias) = name {
                self.emit_load_const(ConstantData::None);
                self.store_name(alias.as_str())?;
                self.compile_name(alias.as_str(), NameUsage::Delete)?;
            }

            // Jump to next handler
            emit!(self, Instruction::Jump { target: next_block });

            // Handler raised an exception (cleanup_end label)
            self.switch_to_block(handler_except_block);
            // Stack: [prev_exc, orig, list, new_rest, lasti, raised_exc]
            // (lasti is pushed because push_lasti=true in HANDLER_CLEANUP fblock)

            // Cleanup name binding
            if let Some(alias) = name {
                self.emit_load_const(ConstantData::None);
                self.store_name(alias.as_str())?;
                self.compile_name(alias.as_str(), NameUsage::Delete)?;
            }

            // LIST_APPEND(3) - append raised_exc to list
            // Stack: [prev_exc, orig, list, new_rest, lasti, raised_exc]
            // After pop: [prev_exc, orig, list, new_rest, lasti] (len=5)
            // nth_value(i) = stack[len - i - 1], we need stack[2] = list
            // stack[5 - i - 1] = 2 -> i = 2
            emit!(self, Instruction::ListAppend { i: 2 });
            // Stack: [prev_exc, orig, list, new_rest, lasti]

            // POP_TOP - pop lasti
            emit!(self, Instruction::PopTop);
            // Stack: [prev_exc, orig, list, new_rest]

            // JUMP except_with_error
            // We directly JUMP to next_block since no_match_block falls through to it
            emit!(self, Instruction::Jump { target: next_block });

            // No match - pop match (None)
            self.switch_to_block(no_match_block);
            emit!(self, Instruction::PopTop); // pop match (None)
            // Stack: [prev_exc, orig, list, new_rest]
            // Falls through to next_block

            // except_with_error label
            // All paths merge here at next_block
            self.switch_to_block(next_block);
            // Stack: [prev_exc, orig, list, rest]

            // After last handler, append rest to list
            if i == n - 1 {
                // Stack: [prev_exc, orig, list, rest]
                // ADDOP_I(c, NO_LOCATION, LIST_APPEND, 1);
                // PEEK(1) = stack[len-1] after pop
                // RustPython nth_value(i) = stack[len-i-1] after pop
                // For LIST_APPEND 1: stack[len-1] = stack[len-i-1] -> i = 0
                emit!(self, Instruction::ListAppend { i: 0 });
                // Stack: [prev_exc, orig, list]
                emit!(
                    self,
                    Instruction::Jump {
                        target: reraise_star_block
                    }
                );
            }
        }

        // Pop EXCEPTION_GROUP_HANDLER fblock
        self.pop_fblock(FBlockType::ExceptionGroupHandler);

        // Reraise star block
        self.switch_to_block(reraise_star_block);
        // Stack: [prev_exc, orig, list]

        // CALL_INTRINSIC_2 PREP_RERAISE_STAR
        // Takes 2 args (orig, list) and produces result
        emit!(
            self,
            Instruction::CallIntrinsic2 {
                func: bytecode::IntrinsicFunction2::PrepReraiseStar
            }
        );
        // Stack: [prev_exc, result]

        // COPY 1
        emit!(self, Instruction::CopyItem { index: 1 });
        // Stack: [prev_exc, result, result]

        // POP_JUMP_IF_NOT_NONE reraise
        self.emit_load_const(ConstantData::None);
        emit!(self, Instruction::IsOp(bytecode::Invert::Yes)); // is not None?
        emit!(
            self,
            Instruction::PopJumpIfTrue {
                target: reraise_block
            }
        );
        // Stack: [prev_exc, result]

        // Nothing to reraise
        // POP_TOP - pop result (None)
        emit!(self, Instruction::PopTop);
        // Stack: [prev_exc]

        // POP_BLOCK - no-op for us with exception tables (fblocks handle this)
        // POP_EXCEPT - restore previous exception context
        emit!(self, Instruction::PopExcept);
        // Stack: []

        if !finalbody.is_empty() {
            self.pop_fblock(FBlockType::FinallyTry);
        }

        emit!(self, Instruction::Jump { target: end_block });

        // Reraise the result
        self.switch_to_block(reraise_block);
        // Stack: [prev_exc, result]

        // POP_BLOCK - no-op for us
        // SWAP 2
        emit!(self, Instruction::Swap { index: 2 });
        // Stack: [result, prev_exc]

        // POP_EXCEPT
        emit!(self, Instruction::PopExcept);
        // Stack: [result]

        // RERAISE 0
        emit!(self, Instruction::Reraise { depth: 0 });

        // try-else path
        // NOTE: When we reach here in compilation, the nothing-to-reraise path above
        // has already popped FinallyTry. But else_block is a different execution path
        // that branches from try body success (where FinallyTry is still active).
        // We need to re-push FinallyTry to reflect the correct fblock state for else path.
        if !finalbody.is_empty() {
            self.push_fblock_with_handler(
                FBlockType::FinallyTry,
                finally_block,
                finally_block,
                Some(finally_block),
                current_depth,
                true,
            )?;
        }
        self.switch_to_block(else_block);
        self.compile_statements(orelse)?;

        if !finalbody.is_empty() {
            // Pop the FinallyTry fblock we just pushed for the else path
            self.pop_fblock(FBlockType::FinallyTry);
        }

        emit!(self, Instruction::Jump { target: end_block });

        self.switch_to_block(end_block);
        if !finalbody.is_empty() {
            self.switch_to_block(finally_block);
            self.compile_statements(finalbody)?;
            // No EndFinally emit - exception table handles this
        }

        Ok(())
    }

    fn is_forbidden_arg_name(name: &str) -> bool {
        is_forbidden_name(name)
    }

    /// Compile default arguments
    // = compiler_default_arguments
    fn compile_default_arguments(
        &mut self,
        parameters: &Parameters,
    ) -> CompileResult<bytecode::MakeFunctionFlags> {
        let mut funcflags = bytecode::MakeFunctionFlags::empty();

        // Handle positional defaults
        let defaults: Vec<_> = core::iter::empty()
            .chain(&parameters.posonlyargs)
            .chain(&parameters.args)
            .filter_map(|x| x.default.as_deref())
            .collect();

        if !defaults.is_empty() {
            // Compile defaults and build tuple
            for default in &defaults {
                self.compile_expression(default)?;
            }
            emit!(
                self,
                Instruction::BuildTuple {
                    size: defaults.len().to_u32()
                }
            );
            funcflags |= bytecode::MakeFunctionFlags::DEFAULTS;
        }

        // Handle keyword-only defaults
        let mut kw_with_defaults = vec![];
        for kwonlyarg in &parameters.kwonlyargs {
            if let Some(default) = &kwonlyarg.default {
                kw_with_defaults.push((&kwonlyarg.parameter, default));
            }
        }

        if !kw_with_defaults.is_empty() {
            // Compile kwdefaults and build dict
            for (arg, default) in &kw_with_defaults {
                self.emit_load_const(ConstantData::Str {
                    value: arg.name.as_str().into(),
                });
                self.compile_expression(default)?;
            }
            emit!(
                self,
                Instruction::BuildMap {
                    size: kw_with_defaults.len().to_u32(),
                }
            );
            funcflags |= bytecode::MakeFunctionFlags::KW_ONLY_DEFAULTS;
        }

        Ok(funcflags)
    }

    /// Compile function body and create function object
    // = compiler_function_body
    fn compile_function_body(
        &mut self,
        name: &str,
        parameters: &Parameters,
        body: &[Stmt],
        is_async: bool,
        funcflags: bytecode::MakeFunctionFlags,
    ) -> CompileResult<()> {
        // Always enter function scope
        self.enter_function(name, parameters)?;
        self.current_code_info()
            .flags
            .set(bytecode::CodeFlags::COROUTINE, is_async);

        // Set up context
        let prev_ctx = self.ctx;
        self.ctx = CompileContext {
            loop_data: None,
            in_class: prev_ctx.in_class,
            func: if is_async {
                FunctionContext::AsyncFunction
            } else {
                FunctionContext::Function
            },
            // A function starts a new async scope only if it's async
            in_async_scope: is_async,
        };

        // Set qualname
        self.set_qualname();

        // Handle docstring
        let (doc_str, body) = split_doc(body, &self.opts);
        self.current_code_info()
            .metadata
            .consts
            .insert_full(ConstantData::None);

        // Compile body statements
        self.compile_statements(body)?;

        // Emit None at end if needed
        match body.last() {
            Some(Stmt::Return(_)) => {}
            _ => {
                self.emit_return_const(ConstantData::None);
            }
        }

        // Exit scope and create function object
        let code = self.exit_scope();
        self.ctx = prev_ctx;

        // Create function object with closure
        self.make_closure(code, funcflags)?;

        // Handle docstring if present
        if let Some(doc) = doc_str {
            emit!(self, Instruction::CopyItem { index: 1_u32 });
            self.emit_load_const(ConstantData::Str {
                value: doc.to_string().into(),
            });
            emit!(self, Instruction::Swap { index: 2 });
            let doc_attr = self.name("__doc__");
            emit!(self, Instruction::StoreAttr { idx: doc_attr });
        }

        Ok(())
    }

    /// Compile function annotations
    // = compiler_visit_annotations
    fn visit_annotations(
        &mut self,
        parameters: &Parameters,
        returns: Option<&Expr>,
    ) -> CompileResult<u32> {
        let mut num_annotations = 0;

        // Handle parameter annotations
        let parameters_iter = core::iter::empty()
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

        // Handle return annotation last
        if let Some(annotation) = returns {
            self.emit_load_const(ConstantData::Str {
                value: "return".into(),
            });
            self.compile_annotation(annotation)?;
            num_annotations += 1;
        }

        Ok(num_annotations)
    }

    // = compiler_function
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

        // compile defaults and return funcflags
        let funcflags = self.compile_default_arguments(parameters)?;

        let is_generic = type_params.is_some();
        let mut num_typeparam_args = 0;

        if is_generic {
            // Count args to pass to type params scope
            if funcflags.contains(bytecode::MakeFunctionFlags::DEFAULTS) {
                num_typeparam_args += 1;
            }
            if funcflags.contains(bytecode::MakeFunctionFlags::KW_ONLY_DEFAULTS) {
                num_typeparam_args += 1;
            }

            // Enter type params scope
            let type_params_name = format!("<generic parameters of {name}>");
            self.push_output(
                bytecode::CodeFlags::OPTIMIZED | bytecode::CodeFlags::NEWLOCALS,
                0,
                num_typeparam_args as u32,
                0,
                type_params_name,
            )?;

            // Add parameter names to varnames for the type params scope
            // These will be passed as arguments when the closure is called
            let current_info = self.current_code_info();
            if funcflags.contains(bytecode::MakeFunctionFlags::DEFAULTS) {
                current_info
                    .metadata
                    .varnames
                    .insert(".defaults".to_owned());
            }
            if funcflags.contains(bytecode::MakeFunctionFlags::KW_ONLY_DEFAULTS) {
                current_info
                    .metadata
                    .varnames
                    .insert(".kwdefaults".to_owned());
            }

            // Compile type parameters
            self.compile_type_params(type_params.unwrap())?;

            // Load defaults/kwdefaults with LOAD_FAST
            for i in 0..num_typeparam_args {
                emit!(self, Instruction::LoadFast(i as u32));
            }
        }

        // Compile annotations
        let mut annotations_flag = bytecode::MakeFunctionFlags::empty();
        let num_annotations = self.visit_annotations(parameters, returns)?;
        if num_annotations > 0 {
            annotations_flag = bytecode::MakeFunctionFlags::ANNOTATIONS;
            emit!(
                self,
                Instruction::BuildMap {
                    size: num_annotations,
                }
            );
        }

        // Compile function body
        let final_funcflags = funcflags | annotations_flag;
        self.compile_function_body(name, parameters, body, is_async, final_funcflags)?;

        // Handle type params if present
        if is_generic {
            // SWAP to get function on top
            // Stack: [type_params_tuple, function] -> [function, type_params_tuple]
            emit!(self, Instruction::Swap { index: 2 });

            // Call INTRINSIC_SET_FUNCTION_TYPE_PARAMS
            emit!(
                self,
                Instruction::CallIntrinsic2 {
                    func: bytecode::IntrinsicFunction2::SetFunctionTypeParams,
                }
            );

            // Return the function object from type params scope
            emit!(self, Instruction::ReturnValue);

            // Set argcount for type params scope
            self.current_code_info().metadata.argcount = num_typeparam_args as u32;

            // Exit type params scope and create closure
            let type_params_code = self.exit_scope();

            // Make closure for type params code
            self.make_closure(type_params_code, bytecode::MakeFunctionFlags::empty())?;

            // Call the type params closure with defaults/kwdefaults as arguments.
            // Call protocol: [callable, self_or_null, arg1, ..., argN]
            // We need to reorder: [args..., closure] -> [closure, NULL, args...]
            // Using Swap operations to move closure down and insert NULL.
            // Note: num_typeparam_args is at most 2 (defaults tuple, kwdefaults dict).
            if num_typeparam_args > 0 {
                match num_typeparam_args {
                    1 => {
                        // Stack: [arg1, closure]
                        emit!(self, Instruction::Swap { index: 2 }); // [closure, arg1]
                        emit!(self, Instruction::PushNull); // [closure, arg1, NULL]
                        emit!(self, Instruction::Swap { index: 2 }); // [closure, NULL, arg1]
                    }
                    2 => {
                        // Stack: [arg1, arg2, closure]
                        emit!(self, Instruction::Swap { index: 3 }); // [closure, arg2, arg1]
                        emit!(self, Instruction::Swap { index: 2 }); // [closure, arg1, arg2]
                        emit!(self, Instruction::PushNull); // [closure, arg1, arg2, NULL]
                        emit!(self, Instruction::Swap { index: 3 }); // [closure, NULL, arg2, arg1]
                        emit!(self, Instruction::Swap { index: 2 }); // [closure, NULL, arg1, arg2]
                    }
                    _ => unreachable!("only defaults and kwdefaults are supported"),
                }
                emit!(
                    self,
                    Instruction::Call {
                        nargs: num_typeparam_args as u32
                    }
                );
            } else {
                // Stack: [closure]
                emit!(self, Instruction::PushNull);
                // Stack: [closure, NULL]
                emit!(self, Instruction::Call { nargs: 0 });
            }
        }

        // Apply decorators
        self.apply_decorators(decorator_list);

        // Store the function
        self.store_name(name)?;

        Ok(())
    }

    /// Determines if a variable should be CELL or FREE type
    // = get_ref_type
    fn get_ref_type(&self, name: &str) -> Result<SymbolScope, CodegenErrorType> {
        // Special handling for __class__ and __classdict__ in class scope
        if self.ctx.in_class && (name == "__class__" || name == "__classdict__") {
            return Ok(SymbolScope::Cell);
        }

        let table = self.symbol_table_stack.last().unwrap();
        match table.lookup(name) {
            Some(symbol) => match symbol.scope {
                SymbolScope::Cell => Ok(SymbolScope::Cell),
                SymbolScope::Free => Ok(SymbolScope::Free),
                _ if symbol.flags.contains(SymbolFlags::FREE_CLASS) => Ok(SymbolScope::Free),
                _ => Err(CodegenErrorType::SyntaxError(format!(
                    "get_ref_type: invalid scope for '{name}'"
                ))),
            },
            None => Err(CodegenErrorType::SyntaxError(format!(
                "get_ref_type: cannot find symbol '{name}'"
            ))),
        }
    }

    /// Loads closure variables if needed and creates a function object
    // = compiler_make_closure
    fn make_closure(
        &mut self,
        code: CodeObject,
        flags: bytecode::MakeFunctionFlags,
    ) -> CompileResult<()> {
        // Handle free variables (closure)
        let has_freevars = !code.freevars.is_empty();
        if has_freevars {
            // Build closure tuple by loading free variables

            for var in &code.freevars {
                // Special case: If a class contains a method with a
                // free variable that has the same name as a method,
                // the name will be considered free *and* local in the
                // class. It should be handled by the closure, as
                // well as by the normal name lookup logic.

                // Get reference type using our get_ref_type function
                let ref_type = self.get_ref_type(var).map_err(|e| self.error(e))?;

                // Get parent code info
                let parent_code = self.code_stack.last().unwrap();
                let cellvars_len = parent_code.metadata.cellvars.len();

                // Look up the variable index based on reference type
                let idx = match ref_type {
                    SymbolScope::Cell => parent_code
                        .metadata
                        .cellvars
                        .get_index_of(var)
                        .or_else(|| {
                            parent_code
                                .metadata
                                .freevars
                                .get_index_of(var)
                                .map(|i| i + cellvars_len)
                        })
                        .ok_or_else(|| {
                            self.error(CodegenErrorType::SyntaxError(format!(
                                "compiler_make_closure: cannot find '{var}' in parent vars",
                            )))
                        })?,
                    SymbolScope::Free => parent_code
                        .metadata
                        .freevars
                        .get_index_of(var)
                        .map(|i| i + cellvars_len)
                        .or_else(|| parent_code.metadata.cellvars.get_index_of(var))
                        .ok_or_else(|| {
                            self.error(CodegenErrorType::SyntaxError(format!(
                                "compiler_make_closure: cannot find '{var}' in parent vars",
                            )))
                        })?,
                    _ => {
                        return Err(self.error(CodegenErrorType::SyntaxError(format!(
                            "compiler_make_closure: unexpected ref_type {ref_type:?} for '{var}'",
                        ))));
                    }
                };

                emit!(self, Instruction::LoadClosure(idx.to_u32()));
            }

            // Build tuple of closure variables
            emit!(
                self,
                Instruction::BuildTuple {
                    size: code.freevars.len().to_u32(),
                }
            );
        }

        // load code object and create function
        self.emit_load_const(ConstantData::Code {
            code: Box::new(code),
        });

        // Create function with no flags
        emit!(self, Instruction::MakeFunction);

        // Now set attributes one by one using SET_FUNCTION_ATTRIBUTE
        // Note: The order matters! Values must be on stack before calling SET_FUNCTION_ATTRIBUTE

        // Set closure if needed
        if has_freevars {
            emit!(
                self,
                Instruction::SetFunctionAttribute {
                    attr: bytecode::MakeFunctionFlags::CLOSURE
                }
            );
        }

        // Set annotations if present
        if flags.contains(bytecode::MakeFunctionFlags::ANNOTATIONS) {
            emit!(
                self,
                Instruction::SetFunctionAttribute {
                    attr: bytecode::MakeFunctionFlags::ANNOTATIONS
                }
            );
        }

        // Set kwdefaults if present
        if flags.contains(bytecode::MakeFunctionFlags::KW_ONLY_DEFAULTS) {
            emit!(
                self,
                Instruction::SetFunctionAttribute {
                    attr: bytecode::MakeFunctionFlags::KW_ONLY_DEFAULTS
                }
            );
        }

        // Set defaults if present
        if flags.contains(bytecode::MakeFunctionFlags::DEFAULTS) {
            emit!(
                self,
                Instruction::SetFunctionAttribute {
                    attr: bytecode::MakeFunctionFlags::DEFAULTS
                }
            );
        }

        // Set type_params if present
        if flags.contains(bytecode::MakeFunctionFlags::TYPE_PARAMS) {
            emit!(
                self,
                Instruction::SetFunctionAttribute {
                    attr: bytecode::MakeFunctionFlags::TYPE_PARAMS
                }
            );
        }

        Ok(())
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

    /// Compile the class body into a code object
    // = compiler_class_body
    fn compile_class_body(
        &mut self,
        name: &str,
        body: &[Stmt],
        type_params: Option<&TypeParams>,
        firstlineno: u32,
    ) -> CompileResult<CodeObject> {
        // 1. Enter class scope
        let key = self.symbol_table_stack.len();
        self.push_symbol_table()?;
        self.enter_scope(name, CompilerScope::Class, key, firstlineno)?;

        // Set qualname using the new method
        let qualname = self.set_qualname();

        // For class scopes, set u_private to the class name for name mangling
        self.code_stack.last_mut().unwrap().private = Some(name.to_owned());

        // 2. Set up class namespace
        let (doc_str, body) = split_doc(body, &self.opts);

        // Load (global) __name__ and store as __module__
        let dunder_name = self.name("__name__");
        emit!(self, Instruction::LoadGlobal(dunder_name));
        let dunder_module = self.name("__module__");
        emit!(self, Instruction::StoreName(dunder_module));

        // Store __qualname__
        self.emit_load_const(ConstantData::Str {
            value: qualname.into(),
        });
        let qualname_name = self.name("__qualname__");
        emit!(self, Instruction::StoreName(qualname_name));

        // Store __doc__ only if there's an explicit docstring
        if let Some(doc) = doc_str {
            self.emit_load_const(ConstantData::Str { value: doc.into() });
            let doc_name = self.name("__doc__");
            emit!(self, Instruction::StoreName(doc_name));
        }

        // Store __firstlineno__ (new in Python 3.12+)
        self.emit_load_const(ConstantData::Integer {
            value: BigInt::from(firstlineno),
        });
        let firstlineno_name = self.name("__firstlineno__");
        emit!(self, Instruction::StoreName(firstlineno_name));

        // Set __type_params__ if we have type parameters
        if type_params.is_some() {
            // Load .type_params from enclosing scope
            let dot_type_params = self.name(".type_params");
            emit!(self, Instruction::LoadName(dot_type_params));

            // Store as __type_params__
            let dunder_type_params = self.name("__type_params__");
            emit!(self, Instruction::StoreName(dunder_type_params));
        }

        // Setup annotations if needed
        if Self::find_ann(body) {
            emit!(self, Instruction::SetupAnnotations);
        }

        // 3. Compile the class body
        self.compile_statements(body)?;

        // 4. Handle __classcell__ if needed
        let classcell_idx = self
            .code_stack
            .last_mut()
            .unwrap()
            .metadata
            .cellvars
            .iter()
            .position(|var| *var == "__class__");

        if let Some(classcell_idx) = classcell_idx {
            emit!(self, Instruction::LoadClosure(classcell_idx.to_u32()));
            emit!(self, Instruction::CopyItem { index: 1_u32 });
            let classcell = self.name("__classcell__");
            emit!(self, Instruction::StoreName(classcell));
        } else {
            self.emit_load_const(ConstantData::None);
        }

        // Return the class namespace
        self.emit_return_value();

        // Exit scope and return the code object
        Ok(self.exit_scope())
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

        let is_generic = type_params.is_some();
        let firstlineno = self.get_source_line_number().get().to_u32();

        // Step 1: If generic, enter type params scope and compile type params
        if is_generic {
            let type_params_name = format!("<generic parameters of {name}>");
            self.push_output(
                bytecode::CodeFlags::OPTIMIZED | bytecode::CodeFlags::NEWLOCALS,
                0,
                0,
                0,
                type_params_name,
            )?;

            // Set private name for name mangling
            self.code_stack.last_mut().unwrap().private = Some(name.to_owned());

            // Compile type parameters and store as .type_params
            self.compile_type_params(type_params.unwrap())?;
            let dot_type_params = self.name(".type_params");
            emit!(self, Instruction::StoreName(dot_type_params));
        }

        // Step 2: Compile class body (always done, whether generic or not)
        let prev_ctx = self.ctx;
        self.ctx = CompileContext {
            func: FunctionContext::NoFunction,
            in_class: true,
            loop_data: None,
            in_async_scope: false,
        };
        let class_code = self.compile_class_body(name, body, type_params, firstlineno)?;
        self.ctx = prev_ctx;

        // Step 3: Generate the rest of the code for the call
        if is_generic {
            // Still in type params scope
            let dot_type_params = self.name(".type_params");
            let dot_generic_base = self.name(".generic_base");

            // Create .generic_base
            emit!(self, Instruction::LoadName(dot_type_params));
            emit!(
                self,
                Instruction::CallIntrinsic1 {
                    func: bytecode::IntrinsicFunction1::SubscriptGeneric
                }
            );
            emit!(self, Instruction::StoreName(dot_generic_base));

            // Generate class creation code
            emit!(self, Instruction::LoadBuildClass);
            emit!(self, Instruction::PushNull);

            // Set up the class function with type params
            let mut func_flags = bytecode::MakeFunctionFlags::empty();
            emit!(self, Instruction::LoadName(dot_type_params));
            func_flags |= bytecode::MakeFunctionFlags::TYPE_PARAMS;

            // Create class function with closure
            self.make_closure(class_code, func_flags)?;
            self.emit_load_const(ConstantData::Str { value: name.into() });

            // Compile bases and call __build_class__
            // Check for starred bases or **kwargs
            let has_starred = arguments
                .is_some_and(|args| args.args.iter().any(|arg| matches!(arg, Expr::Starred(_))));
            let has_double_star =
                arguments.is_some_and(|args| args.keywords.iter().any(|kw| kw.arg.is_none()));

            if has_starred || has_double_star {
                // Use CallFunctionEx for *bases or **kwargs
                // Stack has: [__build_class__, NULL, class_func, name]
                // Need to build: args tuple = (class_func, name, *bases, .generic_base)

                // Compile bases with gather_elements (handles starred)
                let (size, unpack) = if let Some(arguments) = arguments {
                    self.gather_elements(2, &arguments.args)? // 2 = class_func + name already on stack
                } else {
                    // Just class_func and name (no bases)
                    (2, false)
                };

                // Add .generic_base as final base
                emit!(self, Instruction::LoadName(dot_generic_base));

                // Build args tuple
                if unpack {
                    // Starred: gather_elements produced tuples on stack
                    emit!(self, Instruction::BuildTuple { size: 1 }); // (.generic_base,)
                    emit!(self, Instruction::BuildTupleFromTuples { size: size + 1 });
                } else {
                    // No starred: individual elements on stack
                    // size includes class_func + name + bases count, +1 for .generic_base
                    emit!(self, Instruction::BuildTuple { size: size + 1 });
                }

                // Build kwargs if needed
                let has_kwargs = arguments.is_some_and(|args| !args.keywords.is_empty());
                if has_kwargs {
                    self.compile_keywords(&arguments.unwrap().keywords)?;
                }
                emit!(self, Instruction::CallFunctionEx { has_kwargs });
            } else {
                // Simple case: no starred bases, no **kwargs
                // Compile bases normally
                let base_count = if let Some(arguments) = arguments {
                    for arg in &arguments.args {
                        self.compile_expression(arg)?;
                    }
                    arguments.args.len()
                } else {
                    0
                };

                // Load .generic_base as the last base
                emit!(self, Instruction::LoadName(dot_generic_base));

                let nargs = 2 + u32::try_from(base_count).expect("too many base classes") + 1;

                // Handle keyword arguments (no **kwargs here)
                if let Some(arguments) = arguments
                    && !arguments.keywords.is_empty()
                {
                    let mut kwarg_names = vec![];
                    for keyword in &arguments.keywords {
                        let name = keyword.arg.as_ref().expect(
                            "keyword argument name must be set (no **kwargs in this branch)",
                        );
                        kwarg_names.push(ConstantData::Str {
                            value: name.as_str().into(),
                        });
                        self.compile_expression(&keyword.value)?;
                    }
                    self.emit_load_const(ConstantData::Tuple {
                        elements: kwarg_names,
                    });
                    emit!(
                        self,
                        Instruction::CallKw {
                            nargs: nargs
                                + u32::try_from(arguments.keywords.len())
                                    .expect("too many keyword arguments")
                        }
                    );
                } else {
                    emit!(self, Instruction::Call { nargs });
                }
            }

            // Return the created class
            self.emit_return_value();

            // Exit type params scope and wrap in function
            let type_params_code = self.exit_scope();

            // Execute the type params function
            self.make_closure(type_params_code, bytecode::MakeFunctionFlags::empty())?;
            emit!(self, Instruction::PushNull);
            emit!(self, Instruction::Call { nargs: 0 });
        } else {
            // Non-generic class: standard path
            emit!(self, Instruction::LoadBuildClass);
            emit!(self, Instruction::PushNull);

            // Create class function with closure
            self.make_closure(class_code, bytecode::MakeFunctionFlags::empty())?;
            self.emit_load_const(ConstantData::Str { value: name.into() });

            if let Some(arguments) = arguments {
                self.compile_call_helper(2, arguments)?;
            } else {
                emit!(self, Instruction::Call { nargs: 2 });
            }
        }

        // Step 4: Apply decorators and store (common to both paths)
        self.apply_decorators(decorator_list);
        self.store_name(name)
    }

    fn compile_while(&mut self, test: &Expr, body: &[Stmt], orelse: &[Stmt]) -> CompileResult<()> {
        let while_block = self.new_block();
        let else_block = self.new_block();
        let after_block = self.new_block();

        // Note: SetupLoop is no longer emitted (break/continue use direct jumps)
        self.switch_to_block(while_block);

        // Push fblock for while loop
        self.push_fblock(FBlockType::WhileLoop, while_block, after_block)?;

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

        // Pop fblock
        self.pop_fblock(FBlockType::WhileLoop);
        // Note: PopBlock is no longer emitted for loops
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
        // Python 3.12+ style with statement:
        //
        // BEFORE_WITH          # TOS: ctx_mgr -> [__exit__, __enter__ result]
        // L1: STORE_NAME f     # exception table: L1 to L2 -> L3 [1] lasti
        // L2: ... body ...
        //     LOAD_CONST None  # normal exit
        //     LOAD_CONST None
        //     LOAD_CONST None
        //     CALL 2           # __exit__(None, None, None)
        //     POP_TOP
        //     JUMP after
        // L3: PUSH_EXC_INFO    # exception handler
        //     WITH_EXCEPT_START # call __exit__(type, value, tb), push result
        //     TO_BOOL
        //     POP_JUMP_IF_TRUE suppress
        //     RERAISE 2
        // suppress:
        //     POP_TOP          # pop exit result
        // L5: POP_EXCEPT
        //     POP_TOP          # pop __exit__
        //     POP_TOP          # pop prev_exc (or lasti depending on layout)
        //     JUMP after
        // L6: COPY 3           # cleanup handler for reraise
        //     POP_EXCEPT
        //     RERAISE 1
        // after: ...

        let with_range = self.current_source_range;

        let Some((item, items)) = items.split_first() else {
            return Err(self.error(CodegenErrorType::EmptyWithItems));
        };

        let exc_handler_block = self.new_block();
        let after_block = self.new_block();

        // Compile context expression and BEFORE_WITH
        self.compile_expression(&item.context_expr)?;
        self.set_source_range(with_range);

        if is_async {
            if self.ctx.func != FunctionContext::AsyncFunction {
                return Err(self.error(CodegenErrorType::InvalidAsyncWith));
            }
            emit!(self, Instruction::BeforeAsyncWith);
            emit!(self, Instruction::GetAwaitable);
            self.emit_load_const(ConstantData::None);
            self.compile_yield_from_sequence(true)?;
        } else {
            emit!(self, Instruction::BeforeWith);
        }

        // Stack: [..., __exit__, enter_result]
        // Push fblock for exception table - handler goes to exc_handler_block
        // preserve_lasti=true for with statements
        // Use handler_stack_depth() to include all items on stack (for loops, etc.)
        let with_depth = self.handler_stack_depth() + 1; // +1 for current __exit__
        self.push_fblock_with_handler(
            if is_async {
                FBlockType::AsyncWith
            } else {
                FBlockType::With
            },
            exc_handler_block, // block start (will become exit target after store)
            after_block,
            Some(exc_handler_block),
            with_depth,
            true, // preserve_lasti=true
        )?;

        // Store or pop the enter result
        match &item.optional_vars {
            Some(var) => {
                self.set_source_range(var.range());
                self.compile_store(var)?;
            }
            None => {
                emit!(self, Instruction::PopTop);
            }
        }
        // Stack: [..., __exit__]

        // Compile body or nested with
        if items.is_empty() {
            if body.is_empty() {
                return Err(self.error(CodegenErrorType::EmptyWithBody));
            }
            self.compile_statements(body)?;
        } else {
            self.set_source_range(with_range);
            self.compile_with(items, body, is_async)?;
        }

        // Pop fblock before normal exit
        self.pop_fblock(if is_async {
            FBlockType::AsyncWith
        } else {
            FBlockType::With
        });

        // ===== Normal exit path =====
        // Stack: [..., __exit__]
        // Call __exit__(None, None, None)
        self.set_source_range(with_range);
        emit!(self, Instruction::PushNull);
        self.emit_load_const(ConstantData::None);
        self.emit_load_const(ConstantData::None);
        self.emit_load_const(ConstantData::None);
        emit!(self, Instruction::Call { nargs: 3 });
        if is_async {
            emit!(self, Instruction::GetAwaitable);
            self.emit_load_const(ConstantData::None);
            self.compile_yield_from_sequence(true)?;
        }
        emit!(self, Instruction::PopTop); // Pop __exit__ result
        emit!(
            self,
            Instruction::Jump {
                target: after_block
            }
        );

        // ===== Exception handler path =====
        // Stack at entry (after unwind): [..., __exit__, lasti, exc]
        // PUSH_EXC_INFO -> [..., __exit__, lasti, prev_exc, exc]
        self.switch_to_block(exc_handler_block);

        // Create blocks for exception handling
        let cleanup_block = self.new_block();
        let suppress_block = self.new_block();

        // Push nested fblock for cleanup handler
        // Stack at exc_handler_block entry: [..., __exit__, lasti, exc]
        // After PUSH_EXC_INFO: [..., __exit__, lasti, prev_exc, exc]
        // If exception in __exit__, cleanup handler entry: [..., __exit__, lasti, prev_exc, lasti2, exc2]
        // cleanup_depth should be: with_depth + 2 (lasti + prev_exc)
        let cleanup_depth = with_depth + 2;
        self.push_fblock_with_handler(
            FBlockType::ExceptionHandler,
            exc_handler_block,
            after_block,
            Some(cleanup_block),
            cleanup_depth,
            true, // preserve_lasti=true
        )?;

        // PUSH_EXC_INFO: [exc] -> [prev_exc, exc]
        emit!(self, Instruction::PushExcInfo);

        // WITH_EXCEPT_START: call __exit__(type, value, tb)
        // Stack: [..., __exit__, lasti, prev_exc, exc]
        // __exit__ is at TOS-3, call with exception info
        emit!(self, Instruction::WithExceptStart);

        if is_async {
            emit!(self, Instruction::GetAwaitable);
            self.emit_load_const(ConstantData::None);
            self.compile_yield_from_sequence(true)?;
        }

        // TO_BOOL + POP_JUMP_IF_TRUE: check if exception is suppressed
        emit!(self, Instruction::ToBool);
        emit!(
            self,
            Instruction::PopJumpIfTrue {
                target: suppress_block
            }
        );

        // Pop the nested fblock BEFORE RERAISE so that RERAISE's exception
        // handler points to the outer handler (try-except), not cleanup_block.
        // This is critical: when RERAISE propagates the exception, the exception
        // table should route it to the outer try-except, not back to cleanup.
        self.pop_fblock(FBlockType::ExceptionHandler);

        // Not suppressed: RERAISE 2
        emit!(self, Instruction::Reraise { depth: 2 });

        // ===== Suppress block =====
        // Exception was suppressed, clean up stack
        // Stack: [..., __exit__, lasti, prev_exc, exc, True]
        // Need to pop: True, exc, prev_exc, __exit__
        self.switch_to_block(suppress_block);
        emit!(self, Instruction::PopTop); // pop True (TO_BOOL result)
        emit!(self, Instruction::PopExcept); // pop exc and restore prev_exc
        emit!(self, Instruction::PopTop); // pop __exit__
        emit!(self, Instruction::PopTop); // pop lasti
        emit!(
            self,
            Instruction::Jump {
                target: after_block
            }
        );

        // ===== Cleanup block (for nested exception during __exit__) =====
        // Stack: [..., __exit__, lasti, prev_exc, lasti2, exc2]
        // COPY 3: copy prev_exc to TOS
        // POP_EXCEPT: restore exception state
        // RERAISE 1: re-raise with lasti
        //
        // NOTE: We DON'T clear the fblock stack here because we want
        // outer exception handlers (e.g., try-except wrapping this with statement)
        // to be in the exception table for these instructions.
        // If we cleared fblock, exceptions here would propagate uncaught.
        self.switch_to_block(cleanup_block);
        emit!(self, Instruction::CopyItem { index: 3 });
        emit!(self, Instruction::PopExcept);
        emit!(self, Instruction::Reraise { depth: 1 });

        // ===== After block =====
        self.switch_to_block(after_block);

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

        // The thing iterated:
        self.compile_expression(iter)?;

        if is_async {
            if self.ctx.func != FunctionContext::AsyncFunction {
                return Err(self.error(CodegenErrorType::InvalidAsyncFor));
            }
            emit!(self, Instruction::GetAIter);

            self.switch_to_block(for_block);

            // Push fblock for async for loop with exception handler info
            // Note: SetupExcept is no longer emitted (exception table handles StopAsyncIteration)
            // Stack at this point: [..., async_iterator]
            // We need handler_stack_depth() + 1 to keep parent items + async_iterator on stack when exception occurs
            let async_for_depth = self.handler_stack_depth() + 1;
            self.push_fblock_with_handler(
                FBlockType::ForLoop,
                for_block,
                after_block,
                Some(else_block), // Handler for StopAsyncIteration
                async_for_depth,  // stack depth: keep async_iterator and parent items
                false,            // no lasti needed
            )?;

            emit!(self, Instruction::GetANext);
            self.emit_load_const(ConstantData::None);
            self.compile_yield_from_sequence(true)?;
            self.compile_store(target)?;
            // Note: PopBlock is no longer emitted (exception table handles this)
        } else {
            // Retrieve Iterator
            emit!(self, Instruction::GetIter);

            self.switch_to_block(for_block);

            // Push fblock for for loop
            self.push_fblock(FBlockType::ForLoop, for_block, after_block)?;

            emit!(self, Instruction::ForIter { target: else_block });

            // Start of loop iteration, set targets:
            self.compile_store(target)?;
        };

        let was_in_loop = self.ctx.loop_data.replace((for_block, after_block));
        self.compile_statements(body)?;
        self.ctx.loop_data = was_in_loop;
        emit!(self, Instruction::Jump { target: for_block });

        self.switch_to_block(else_block);

        // Pop fblock
        self.pop_fblock(FBlockType::ForLoop);

        if is_async {
            emit!(self, Instruction::EndAsyncFor);
        }
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
        panic!("Failing due to forbidden name {name:?}");
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
                    Instruction::PopJumpIfFalse {
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
            emit!(self, Instruction::PopTop);
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
        // Rotate TOS (top of stack) to position `count` down
        // This is done by a series of swaps
        // For count=1, no rotation needed (already at top)
        // For count=2, swap TOS with item 1 position down
        // For count=3, swap TOS with item 2 positions down, then with item 1 position down
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
        match n {
            // If no name is provided, simply pop the top of the stack.
            None => {
                emit!(self, Instruction::PopTop);
                Ok(())
            }
            Some(name) => {
                // Check if the name is forbidden for storing.
                if self.forbidden_name(name.as_str(), NameUsage::Store)? {
                    return Err(self.compile_error_forbidden_name(name.as_str()));
                }

                // Ensure we don't store the same name twice.
                // TODO: maybe pc.stores should be a set?
                if pc.stores.contains(&name.to_string()) {
                    return Err(
                        self.error(CodegenErrorType::DuplicateStore(name.as_str().to_string()))
                    );
                }

                // Calculate how many items to rotate:
                let rotations = pc.on_top + pc.stores.len() + 1;
                self.pattern_helper_rotate(rotations)?;

                // Append the name to the captured stores.
                pc.stores.push(name.to_string());
                Ok(())
            }
        }
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
                    value: (patterns.len() - i).into(),
                });
                // Subtract to compute the correct index.
                emit!(
                    self,
                    Instruction::BinaryOp {
                        op: BinaryOperator::Subtract
                    }
                );
            }
            // Use BINARY_OP/NB_SUBSCR to extract the element.
            emit!(self, Instruction::BinarySubscr);
            // Compile the subpattern in irrefutable mode.
            self.compile_pattern_subpattern(pattern, pc)?;
        }
        // Pop the subject off the stack.
        pc.on_top -= 1;
        emit!(self, Instruction::PopTop);
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
        let n_attrs = attrs.len();
        for i in 0..n_attrs {
            let attr = attrs[i].as_str();
            // Check if the attribute name is forbidden in a Store context.
            if self.forbidden_name(attr, NameUsage::Store)? {
                // Return an error if the name is forbidden.
                return Err(self.compile_error_forbidden_name(attr));
            }
            // Check for duplicates: compare with every subsequent attribute.
            for ident in attrs.iter().take(n_attrs).skip(i + 1) {
                let other = ident.as_str();
                if attr == other {
                    return Err(self.error(CodegenErrorType::RepeatedAttributePattern));
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
        let n_attrs = kwd_attrs.len();

        // Check for too many sub-patterns.
        if nargs > u32::MAX as usize || (nargs + n_attrs).saturating_sub(1) > i32::MAX as usize {
            let msg = format!(
                "too many sub-patterns in class pattern {:?}",
                match_class.cls
            );
            panic!("{}", msg);
            // return self.compiler_error(&msg);
        }

        // Validate keyword attributes if any.
        if n_attrs != 0 {
            self.validate_kwd_attrs(&kwd_attrs, &kwd_patterns)?;
        }

        // Compile the class expression.
        self.compile_expression(&match_class.cls)?;

        // Create a new tuple of attribute names.
        let mut attr_names = vec![];
        for name in &kwd_attrs {
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
        emit!(self, Instruction::IsOp(Invert::Yes));

        // At this point the TOS is a tuple of (nargs + n_attrs) attributes (or None).
        pc.on_top += 1;
        self.jump_to_fail_pop(pc, JumpOp::PopJumpIfFalse)?;

        // Unpack the tuple into (nargs + n_attrs) items.
        let total = nargs + n_attrs;
        emit!(
            self,
            Instruction::UnpackSequence {
                size: u32::try_from(total).unwrap()
            }
        );
        pc.on_top += total;
        pc.on_top -= 1;

        // Process each sub-pattern.
        for subpattern in patterns.iter().chain(kwd_patterns.iter()) {
            // Check if this is a true wildcard (underscore pattern without name binding)
            let is_true_wildcard = match subpattern {
                Pattern::MatchAs(match_as) => {
                    // Only consider it wildcard if both pattern and name are None (i.e., "_")
                    match_as.pattern.is_none() && match_as.name.is_none()
                }
                _ => subpattern.is_wildcard(),
            };

            // Decrement the on_top counter for each sub-pattern
            pc.on_top -= 1;

            if is_true_wildcard {
                emit!(self, Instruction::PopTop);
                continue; // Don't compile wildcard patterns
            }

            // Compile the subpattern without irrefutability checks.
            self.compile_pattern_subpattern(subpattern, pc)?;
        }
        Ok(())
    }

    fn compile_pattern_mapping(
        &mut self,
        p: &PatternMatchMapping,
        pc: &mut PatternContext,
    ) -> CompileResult<()> {
        let mapping = p;
        let keys = &mapping.keys;
        let patterns = &mapping.patterns;
        let size = keys.len();
        let star_target = &mapping.rest;

        // Validate pattern count matches key count
        if keys.len() != patterns.len() {
            return Err(self.error(CodegenErrorType::SyntaxError(format!(
                "keys ({}) / patterns ({}) length mismatch in mapping pattern",
                keys.len(),
                patterns.len()
            ))));
        }

        // Validate rest pattern: '_' cannot be used as a rest target
        if let Some(rest) = star_target
            && rest.as_str() == "_"
        {
            return Err(self.error(CodegenErrorType::SyntaxError("invalid syntax".to_string())));
        }

        // Step 1: Check if subject is a mapping
        // Stack: [subject]
        pc.on_top += 1;

        emit!(self, Instruction::MatchMapping);
        // Stack: [subject, is_mapping]

        self.jump_to_fail_pop(pc, JumpOp::PopJumpIfFalse)?;
        // Stack: [subject]

        // Special case: empty pattern {} with no rest
        if size == 0 && star_target.is_none() {
            // If the pattern is just "{}", we're done! Pop the subject
            pc.on_top -= 1;
            emit!(self, Instruction::PopTop);
            return Ok(());
        }

        // Length check for patterns with keys
        if size > 0 {
            // Check if the mapping has at least 'size' keys
            emit!(self, Instruction::GetLen);
            self.emit_load_const(ConstantData::Integer { value: size.into() });
            // Stack: [subject, len, size]
            emit!(
                self,
                Instruction::CompareOp {
                    op: ComparisonOperator::GreaterOrEqual
                }
            );
            self.jump_to_fail_pop(pc, JumpOp::PopJumpIfFalse)?;
            // Stack: [subject]
        }

        // Check for overflow (INT_MAX < size - 1)
        if size > (i32::MAX as usize + 1) {
            return Err(self.error(CodegenErrorType::SyntaxError(
                "too many sub-patterns in mapping pattern".to_string(),
            )));
        }
        #[allow(clippy::cast_possible_truncation)]
        let size = size as u32; // checked right before

        // Step 2: If we have keys to match
        if size > 0 {
            // Validate and compile keys
            let mut seen = HashSet::new();
            for key in keys {
                let is_attribute = matches!(key, Expr::Attribute(_));
                let is_literal = matches!(
                    key,
                    Expr::NumberLiteral(_)
                        | Expr::StringLiteral(_)
                        | Expr::BytesLiteral(_)
                        | Expr::BooleanLiteral(_)
                        | Expr::NoneLiteral(_)
                );
                let key_repr = if is_literal {
                    UnparseExpr::new(key, &self.source_file).to_string()
                } else if is_attribute {
                    String::new()
                } else {
                    return Err(self.error(CodegenErrorType::SyntaxError(
                        "mapping pattern keys may only match literals and attribute lookups"
                            .to_string(),
                    )));
                };

                if !key_repr.is_empty() && seen.contains(&key_repr) {
                    return Err(self.error(CodegenErrorType::SyntaxError(format!(
                        "mapping pattern checks duplicate key ({key_repr})"
                    ))));
                }
                if !key_repr.is_empty() {
                    seen.insert(key_repr);
                }

                self.compile_expression(key)?;
            }
        }
        // Stack: [subject, key1, key2, ..., key_n]

        // Build tuple of keys (empty tuple if size==0)
        emit!(self, Instruction::BuildTuple { size });
        // Stack: [subject, keys_tuple]

        // Match keys
        emit!(self, Instruction::MatchKeys);
        // Stack: [subject, keys_tuple, values_or_none]
        pc.on_top += 2; // subject and keys_tuple are underneath

        // Check if match succeeded
        emit!(self, Instruction::CopyItem { index: 1_u32 });
        // Stack: [subject, keys_tuple, values_tuple, values_tuple_copy]

        // Check if copy is None (consumes the copy like POP_JUMP_IF_NONE)
        self.emit_load_const(ConstantData::None);
        emit!(self, Instruction::IsOp(Invert::Yes));

        // Stack: [subject, keys_tuple, values_tuple, bool]
        self.jump_to_fail_pop(pc, JumpOp::PopJumpIfFalse)?;
        // Stack: [subject, keys_tuple, values_tuple]

        // Unpack values (the original values_tuple)
        emit!(self, Instruction::UnpackSequence { size });
        // Stack after unpack: [subject, keys_tuple, ...unpacked values...]
        pc.on_top += size as usize; // Unpacked size values, tuple replaced by values
        pc.on_top -= 1;

        // Step 3: Process matched values
        for i in 0..size {
            pc.on_top -= 1;
            self.compile_pattern_subpattern(&patterns[i as usize], pc)?;
        }

        // After processing subpatterns, adjust on_top
        // "Whatever happens next should consume the tuple of keys and the subject"
        // Stack currently: [subject, keys_tuple, ...any captured values...]
        pc.on_top -= 2;

        // Step 4: Handle rest pattern or cleanup
        if let Some(rest_name) = star_target {
            // Build rest dict for **rest pattern
            // Stack: [subject, keys_tuple]

            // Build rest dict exactly
            emit!(self, Instruction::BuildMap { size: 0 });
            // Stack: [subject, keys_tuple, {}]
            emit!(self, Instruction::Swap { index: 3 });
            // Stack: [{}, keys_tuple, subject]
            emit!(self, Instruction::DictUpdate { index: 2 });
            // Stack after DICT_UPDATE: [rest_dict, keys_tuple]
            // DICT_UPDATE consumes source (subject) and leaves dict in place

            // Unpack keys and delete from rest_dict
            emit!(self, Instruction::UnpackSequence { size });
            // Stack: [rest_dict, k1, k2, ..., kn] (if size==0, nothing pushed)

            // Delete each key from rest_dict (skipped when size==0)
            // while (size) { COPY(1 + size--); SWAP(2); DELETE_SUBSCR }
            let mut remaining = size;
            while remaining > 0 {
                // Copy rest_dict which is at position (1 + remaining) from TOS
                emit!(
                    self,
                    Instruction::CopyItem {
                        index: 1 + remaining
                    }
                );
                // Stack: [rest_dict, k1, ..., kn, rest_dict]
                emit!(self, Instruction::Swap { index: 2 });
                // Stack: [rest_dict, k1, ..., kn-1, rest_dict, kn]
                emit!(self, Instruction::DeleteSubscr);
                // Stack: [rest_dict, k1, ..., kn-1] (removed kn from rest_dict)
                remaining -= 1;
            }
            // Stack: [rest_dict] (plus any previously stored values)
            // pattern_helper_store_name will handle the rotation correctly

            // Store the rest dict
            self.pattern_helper_store_name(Some(rest_name), pc)?;

            // After storing all values, pc.on_top should be 0
            // The values are rotated to the bottom for later storage
            pc.on_top = 0;
        } else {
            // Non-rest pattern: just clean up the stack

            // Pop them as we're not using them
            emit!(self, Instruction::PopTop); // Pop keys_tuple
            emit!(self, Instruction::PopTop); // Pop subject
        }

        Ok(())
    }

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

            let n_stores = pc.stores.len();
            if i == 0 {
                // Save the captured names from the first alternative.
                control = Some(pc.stores.clone());
            } else {
                let control_vec = control.as_ref().unwrap();
                if n_stores != control_vec.len() {
                    return Err(self.error(CodegenErrorType::ConflictingNameBindPattern));
                } else if n_stores > 0 {
                    // Check that the names occur in the same order.
                    for i_control in (0..n_stores).rev() {
                        let name = &control_vec[i_control];
                        // Find the index of `name` in the current stores.
                        let i_stores =
                            pc.stores.iter().position(|n| n == name).ok_or_else(|| {
                                self.error(CodegenErrorType::ConflictingNameBindPattern)
                            })?;
                        if i_control != i_stores {
                            // The orders differ; we must reorder.
                            assert!(i_stores < i_control, "expected i_stores < i_control");
                            let rotations = i_stores + 1;
                            // Rotate pc.stores: take a slice of the first `rotations` items...
                            let rotated = pc.stores[0..rotations].to_vec();
                            // Remove those elements.
                            for _ in 0..rotations {
                                pc.stores.remove(0);
                            }
                            // Insert the rotated slice at the appropriate index.
                            let insert_pos = i_control - i_stores;
                            for (j, elem) in rotated.into_iter().enumerate() {
                                pc.stores.insert(insert_pos + j, elem);
                            }
                            // Also perform the same rotation on the evaluation stack.
                            for _ in 0..=i_stores {
                                self.pattern_helper_rotate(i_control + 1)?;
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
        emit!(self, Instruction::PopTop);
        self.jump_to_fail_pop(pc, JumpOp::Jump)?;

        // Use the label "end".
        self.switch_to_block(end);

        // Adjust the final captures.
        let n_stores = control.as_ref().unwrap().len();
        let n_rots = n_stores + 1 + pc.on_top + pc.stores.len();
        for i in 0..n_stores {
            // Rotate the capture to its proper place.
            self.pattern_helper_rotate(n_rots)?;
            let name = &control.as_ref().unwrap()[i];
            // Check for duplicate binding.
            if pc.stores.contains(name) {
                return Err(self.error(CodegenErrorType::DuplicateStore(name.to_string())));
            }
            pc.stores.push(name.clone());
        }

        // Old context and control will be dropped automatically.
        // Finally, pop the copy of the subject.
        emit!(self, Instruction::PopTop);
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
                Instruction::CompareOp {
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
                Instruction::CompareOp {
                    op: ComparisonOperator::GreaterOrEqual
                }
            );
            self.jump_to_fail_pop(pc, JumpOp::PopJumpIfFalse)?;
        }

        // Whatever comes next should consume the subject.
        pc.on_top -= 1;
        if only_wildcard {
            // Patterns like: [] / [_] / [_, _] / [*_] / [_, *_] / [_, _, *_] / etc.
            emit!(self, Instruction::PopTop);
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
            Instruction::CompareOp {
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
        emit!(self, Instruction::IsOp(Invert::No));
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
            Pattern::MatchMapping(pattern_type) => {
                self.compile_pattern_mapping(pattern_type, pattern_context)
            }
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

            if let Some(ref guard) = m.guard {
                self.ensure_fail_pop(pattern_context, 0)?;
                // Compile the guard expression
                self.compile_expression(guard)?;
                emit!(self, Instruction::ToBool);
                emit!(
                    self,
                    Instruction::PopJumpIfFalse {
                        target: pattern_context.fail_pop[0]
                    }
                );
            }

            if i != case_count - 1 {
                emit!(self, Instruction::PopTop);
            }

            self.compile_statements(&m.body)?;
            emit!(self, Instruction::Jump { target: end });
            self.emit_and_reset_fail_pop(pattern_context)?;
        }

        if has_default {
            let m = &cases[num_cases - 1];
            if num_cases == 1 {
                emit!(self, Instruction::PopTop);
            } else {
                emit!(self, Instruction::Nop);
            }
            if let Some(ref guard) = m.guard {
                // Compile guard and jump to end if false
                self.compile_expression(guard)?;
                emit!(self, Instruction::CopyItem { index: 1_u32 });
                emit!(self, Instruction::PopJumpIfFalse { target: end });
                emit!(self, Instruction::PopTop);
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

    /// [CPython `compiler_addcompare`](https://github.com/python/cpython/blob/627894459a84be3488a1789919679c997056a03c/Python/compile.c#L2880-L2924)
    fn compile_addcompare(&mut self, op: &CmpOp) {
        use bytecode::ComparisonOperator::*;
        match op {
            CmpOp::Eq => emit!(self, Instruction::CompareOp { op: Equal }),
            CmpOp::NotEq => emit!(self, Instruction::CompareOp { op: NotEqual }),
            CmpOp::Lt => emit!(self, Instruction::CompareOp { op: Less }),
            CmpOp::LtE => emit!(self, Instruction::CompareOp { op: LessOrEqual }),
            CmpOp::Gt => emit!(self, Instruction::CompareOp { op: Greater }),
            CmpOp::GtE => {
                emit!(self, Instruction::CompareOp { op: GreaterOrEqual })
            }
            CmpOp::In => emit!(self, Instruction::ContainsOp(Invert::No)),
            CmpOp::NotIn => emit!(self, Instruction::ContainsOp(Invert::Yes)),
            CmpOp::Is => emit!(self, Instruction::IsOp(Invert::No)),
            CmpOp::IsNot => emit!(self, Instruction::IsOp(Invert::Yes)),
        }
    }

    /// Compile a chained comparison.
    ///
    /// ```py
    /// a == b == c == d
    /// ```
    ///
    /// Will compile into (pseudo code):
    ///
    /// ```py
    /// result = a == b
    /// if result:
    ///   result = b == c
    ///   if result:
    ///     result = c == d
    /// ```
    ///
    /// # See Also
    /// - [CPython `compiler_compare`](https://github.com/python/cpython/blob/627894459a84be3488a1789919679c997056a03c/Python/compile.c#L4678-L4717)
    fn compile_compare(
        &mut self,
        left: &Expr,
        ops: &[CmpOp],
        comparators: &[Expr],
    ) -> CompileResult<()> {
        let (last_op, mid_ops) = ops.split_last().unwrap();
        let (last_comparator, mid_comparators) = comparators.split_last().unwrap();

        // initialize lhs outside of loop
        self.compile_expression(left)?;

        if mid_comparators.is_empty() {
            self.compile_expression(last_comparator)?;
            self.compile_addcompare(last_op);

            return Ok(());
        }

        let cleanup = self.new_block();

        // for all comparisons except the last (as the last one doesn't need a conditional jump)
        for (op, comparator) in mid_ops.iter().zip(mid_comparators) {
            self.compile_expression(comparator)?;

            // store rhs for the next comparison in chain
            emit!(self, Instruction::Swap { index: 2 });
            emit!(self, Instruction::CopyItem { index: 2 });

            self.compile_addcompare(op);

            // if comparison result is false, we break with this value; if true, try the next one.
            /*
            emit!(self, Instruction::CopyItem { index: 1 });
            // emit!(self, Instruction::ToBool); // TODO: Uncomment this
            emit!(self, Instruction::PopJumpIfFalse { target: cleanup });
            emit!(self, Instruction::PopTop);
            */

            emit!(self, Instruction::JumpIfFalseOrPop { target: cleanup });
        }

        self.compile_expression(last_comparator)?;
        self.compile_addcompare(last_op);

        let end = self.new_block();
        emit!(self, Instruction::Jump { target: end });

        // early exit left us with stack: `rhs, comparison_result`. We need to clean up rhs.
        self.switch_to_block(cleanup);
        emit!(self, Instruction::Swap { index: 2 });
        emit!(self, Instruction::PopTop);

        self.switch_to_block(end);
        Ok(())
    }

    fn compile_annotation(&mut self, annotation: &Expr) -> CompileResult<()> {
        if self.future_annotations {
            self.emit_load_const(ConstantData::Str {
                value: UnparseExpr::new(annotation, &self.source_file)
                    .to_string()
                    .into(),
            });
        } else {
            let was_in_annotation = self.in_annotation;
            self.in_annotation = true;

            // Special handling for starred annotations (*Ts -> Unpack[Ts])
            let result = match annotation {
                Expr::Starred(ExprStarred { value, .. }) => {
                    // *args: *Ts (where Ts is a TypeVarTuple).
                    // Do [annotation_value] = [*Ts].
                    self.compile_expression(value)?;
                    emit!(self, Instruction::UnpackSequence { size: 1 });
                    Ok(())
                }
                _ => self.compile_expression(annotation),
            };

            self.in_annotation = was_in_annotation;
            result?;
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
            emit!(self, Instruction::LoadName(annotations));
            self.emit_load_const(ConstantData::Str {
                value: self.mangle(id.as_str()).into_owned().into(),
            });
            emit!(self, Instruction::StoreSubscr);
        } else {
            // Drop annotation if not assigned to simple identifier.
            emit!(self, Instruction::PopTop);
        }

        Ok(())
    }

    fn compile_store(&mut self, target: &Expr) -> CompileResult<()> {
        match &target {
            Expr::Name(ExprName { id, .. }) => self.store_name(id.as_str())?,
            Expr::Subscript(ExprSubscript {
                value, slice, ctx, ..
            }) => {
                self.compile_subscript(value, slice, *ctx)?;
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
            Expr::Subscript(ExprSubscript {
                value,
                slice,
                ctx: _,
                ..
            }) => {
                // For augmented assignment, we need to load the value first
                // But we can't use compile_subscript directly because we need DUP_TOP2
                self.compile_expression(value)?;
                self.compile_expression(slice)?;
                emit!(self, Instruction::CopyItem { index: 2_u32 });
                emit!(self, Instruction::CopyItem { index: 2_u32 });
                emit!(self, Instruction::Subscript);
                AugAssignKind::Subscript
            }
            Expr::Attribute(ExprAttribute { value, attr, .. }) => {
                let attr = attr.as_str();
                self.check_forbidden_name(attr, NameUsage::Store)?;
                self.compile_expression(value)?;
                emit!(self, Instruction::CopyItem { index: 1_u32 });
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
                emit!(self, Instruction::Swap { index: 3 });
                emit!(self, Instruction::Swap { index: 2 });
                emit!(self, Instruction::StoreSubscr);
            }
            AugAssignKind::Attr { idx } => {
                // stack: CONTAINER RESULT
                emit!(self, Instruction::Swap { index: 2 });
                emit!(self, Instruction::StoreAttr { idx });
            }
        }

        Ok(())
    }

    fn compile_op(&mut self, op: &Operator, inplace: bool) {
        let bin_op = match op {
            Operator::Add => BinaryOperator::Add,
            Operator::Sub => BinaryOperator::Subtract,
            Operator::Mult => BinaryOperator::Multiply,
            Operator::MatMult => BinaryOperator::MatrixMultiply,
            Operator::Div => BinaryOperator::TrueDivide,
            Operator::FloorDiv => BinaryOperator::FloorDivide,
            Operator::Mod => BinaryOperator::Remainder,
            Operator::Pow => BinaryOperator::Power,
            Operator::LShift => BinaryOperator::Lshift,
            Operator::RShift => BinaryOperator::Rshift,
            Operator::BitOr => BinaryOperator::Or,
            Operator::BitXor => BinaryOperator::Xor,
            Operator::BitAnd => BinaryOperator::And,
        };

        let op = if inplace { bin_op.as_inplace() } else { bin_op };
        emit!(self, Instruction::BinaryOp { op })
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
        target_block: BlockIdx,
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
                        Instruction::PopJumpIfTrue {
                            target: target_block,
                        }
                    );
                } else {
                    emit!(
                        self,
                        Instruction::PopJumpIfFalse {
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

            emit!(self, Instruction::CopyItem { index: 1_u32 });
            match op {
                BoolOp::And => {
                    emit!(
                        self,
                        Instruction::PopJumpIfFalse {
                            target: after_block,
                        }
                    );
                }
                BoolOp::Or => {
                    emit!(
                        self,
                        Instruction::PopJumpIfTrue {
                            target: after_block,
                        }
                    );
                }
            }

            emit!(self, Instruction::PopTop);
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
            emit!(self, Instruction::DictUpdate { index: 1 });
        }

        Ok(())
    }

    /// Compile the yield-from/await sequence using SEND/END_SEND/CLEANUP_THROW.
    /// compiler_add_yield_from
    /// This generates:
    ///   send:
    ///     SEND exit
    ///     SETUP_FINALLY fail (via exception table)
    ///     YIELD_VALUE 1
    ///     POP_BLOCK (implicit)
    ///     RESUME
    ///     JUMP send
    ///   fail:
    ///     CLEANUP_THROW
    ///   exit:
    ///     END_SEND
    fn compile_yield_from_sequence(&mut self, is_await: bool) -> CompileResult<()> {
        let send_block = self.new_block();
        let fail_block = self.new_block();
        let exit_block = self.new_block();

        // send:
        self.switch_to_block(send_block);
        emit!(self, Instruction::Send { target: exit_block });

        // SETUP_FINALLY fail - set up exception handler for YIELD_VALUE
        // Stack at this point: [receiver, yielded_value]
        // handler_depth = base + 2 (receiver + yielded_value)
        let handler_depth = self.handler_stack_depth() + 2;
        self.push_fblock_with_handler(
            FBlockType::TryExcept, // Use TryExcept for exception handler
            send_block,
            exit_block,
            Some(fail_block),
            handler_depth,
            false, // no lasti needed
        )?;

        // YIELD_VALUE with arg=1 (yield-from/await mode - not wrapped for async gen)
        emit!(self, Instruction::YieldValue { arg: 1 });

        // POP_BLOCK (implicit - pop fblock before RESUME)
        self.pop_fblock(FBlockType::TryExcept);

        // RESUME
        emit!(
            self,
            Instruction::Resume {
                arg: if is_await {
                    bytecode::ResumeType::AfterAwait as u32
                } else {
                    bytecode::ResumeType::AfterYieldFrom as u32
                }
            }
        );

        // JUMP_NO_INTERRUPT send (regular JUMP in RustPython)
        emit!(self, Instruction::Jump { target: send_block });

        // fail: CLEANUP_THROW
        // Stack when exception: [receiver, yielded_value, exc]
        // CLEANUP_THROW: [sub_iter, last_sent_val, exc] -> [None, value]
        // After: stack is [None, value], fall through to exit
        self.switch_to_block(fail_block);
        emit!(self, Instruction::CleanupThrow);
        // Fall through to exit block

        // exit: END_SEND
        // Stack: [receiver, value] (from SEND) or [None, value] (from CLEANUP_THROW)
        // END_SEND: [receiver/None, value] -> [value]
        self.switch_to_block(exit_block);
        emit!(self, Instruction::EndSend);

        Ok(())
    }

    fn compile_expression(&mut self, expression: &Expr) -> CompileResult<()> {
        use ruff_python_ast::*;
        trace!("Compiling {expression:?}");
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
            Expr::Subscript(ExprSubscript {
                value, slice, ctx, ..
            }) => {
                self.compile_subscript(value, slice, *ctx)?;
            }
            Expr::UnaryOp(ExprUnaryOp { op, operand, .. }) => {
                self.compile_expression(operand)?;

                // Perform operation:
                match op {
                    UnaryOp::UAdd => emit!(
                        self,
                        Instruction::CallIntrinsic1 {
                            func: bytecode::IntrinsicFunction1::UnaryPositive
                        }
                    ),
                    UnaryOp::USub => emit!(self, Instruction::UnaryNegative),
                    UnaryOp::Not => {
                        emit!(self, Instruction::ToBool);
                        emit!(self, Instruction::UnaryNot);
                    }
                    UnaryOp::Invert => emit!(self, Instruction::UnaryInvert),
                };
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
                self.compile_compare(left, ops, comparators)?;
            }
            // Expr::Constant(ExprConstant { value, .. }) => {
            //     self.emit_load_const(compile_constant(value));
            // }
            Expr::List(ExprList { elts, .. }) => {
                self.starunpack_helper(elts, 0, CollectionType::List)?;
            }
            Expr::Tuple(ExprTuple { elts, .. }) => {
                self.starunpack_helper(elts, 0, CollectionType::Tuple)?;
            }
            Expr::Set(ExprSet { elts, .. }) => {
                self.starunpack_helper(elts, 0, CollectionType::Set)?;
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
                let argc = match step {
                    Some(_) => BuildSliceArgCount::Three,
                    None => BuildSliceArgCount::Two,
                };
                emit!(self, Instruction::BuildSlice { argc });
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
                // arg=0: direct yield (wrapped for async generators)
                emit!(self, Instruction::YieldValue { arg: 0 });
                emit!(
                    self,
                    Instruction::Resume {
                        arg: bytecode::ResumeType::AfterYield as u32
                    }
                );
            }
            Expr::Await(ExprAwait { value, .. }) => {
                if self.ctx.func != FunctionContext::AsyncFunction {
                    return Err(self.error(CodegenErrorType::InvalidAwait));
                }
                self.compile_expression(value)?;
                emit!(self, Instruction::GetAwaitable);
                self.emit_load_const(ConstantData::None);
                self.compile_yield_from_sequence(true)?;
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
                emit!(self, Instruction::GetYieldFromIter);
                self.emit_load_const(ConstantData::None);
                self.compile_yield_from_sequence(false)?;
            }
            Expr::Name(ExprName { id, .. }) => self.load_name(id.as_str())?,
            Expr::Lambda(ExprLambda {
                parameters, body, ..
            }) => {
                let default_params = Parameters::default();
                let params = parameters.as_deref().unwrap_or(&default_params);
                validate_duplicate_params(params).map_err(|e| self.error(e))?;

                let prev_ctx = self.ctx;
                let name = "<lambda>".to_owned();

                // Prepare defaults before entering function
                let defaults: Vec<_> = core::iter::empty()
                    .chain(&params.posonlyargs)
                    .chain(&params.args)
                    .filter_map(|x| x.default.as_deref())
                    .collect();
                let have_defaults = !defaults.is_empty();

                if have_defaults {
                    let size = defaults.len().to_u32();
                    for element in &defaults {
                        self.compile_expression(element)?;
                    }
                    emit!(self, Instruction::BuildTuple { size });
                }

                // Prepare keyword-only defaults
                let mut kw_with_defaults = vec![];
                for kwonlyarg in &params.kwonlyargs {
                    if let Some(default) = &kwonlyarg.default {
                        kw_with_defaults.push((&kwonlyarg.parameter, default));
                    }
                }

                let have_kwdefaults = !kw_with_defaults.is_empty();
                if have_kwdefaults {
                    let default_kw_count = kw_with_defaults.len();
                    for (arg, default) in &kw_with_defaults {
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

                self.enter_function(&name, params)?;
                let mut func_flags = bytecode::MakeFunctionFlags::empty();
                if have_defaults {
                    func_flags |= bytecode::MakeFunctionFlags::DEFAULTS;
                }
                if have_kwdefaults {
                    func_flags |= bytecode::MakeFunctionFlags::KW_ONLY_DEFAULTS;
                }

                // Set qualname for lambda
                self.set_qualname();

                self.ctx = CompileContext {
                    loop_data: Option::None,
                    in_class: prev_ctx.in_class,
                    func: FunctionContext::Function,
                    // Lambda is never async, so new scope is not async
                    in_async_scope: false,
                };

                self.current_code_info()
                    .metadata
                    .consts
                    .insert_full(ConstantData::None);

                self.compile_expression(body)?;
                self.emit_return_value();
                let code = self.exit_scope();

                // Create lambda function with closure
                self.make_closure(code, func_flags)?;

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
                    Self::contains_await(elt) || Self::generators_contain_await(generators),
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
                    Self::contains_await(elt) || Self::generators_contain_await(generators),
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
                    Self::contains_await(key)
                        || Self::contains_await(value)
                        || Self::generators_contain_await(generators),
                )?;
            }
            Expr::Generator(ExprGenerator {
                elt, generators, ..
            }) => {
                // Check if element or generators contain async content
                // This makes the generator expression into an async generator
                let element_contains_await =
                    Self::contains_await(elt) || Self::generators_contain_await(generators);
                self.compile_comprehension(
                    "<genexpr>",
                    None,
                    generators,
                    &|compiler| {
                        // Compile the element expression
                        // Note: if element is an async comprehension, compile_expression
                        // already handles awaiting it, so we don't need to await again here
                        compiler.compile_comprehension_element(elt)?;

                        compiler.mark_generator();
                        // arg=0: direct yield (wrapped for async generators)
                        emit!(compiler, Instruction::YieldValue { arg: 0 });
                        emit!(
                            compiler,
                            Instruction::Resume {
                                arg: bytecode::ResumeType::AfterYield as u32
                            }
                        );
                        emit!(compiler, Instruction::PopTop);

                        Ok(())
                    },
                    ComprehensionType::Generator,
                    element_contains_await,
                )?;
            }
            Expr::Starred(ExprStarred { value, .. }) => {
                if self.in_annotation {
                    // In annotation context, starred expressions are allowed (PEP 646)
                    // For now, just compile the inner value without wrapping with Unpack
                    // This is a temporary solution until we figure out how to properly import typing
                    self.compile_expression(value)?;
                } else {
                    return Err(self.error(CodegenErrorType::InvalidStarExpr));
                }
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
                node_index: _,
                range: _,
            }) => {
                self.compile_expression(value)?;
                emit!(self, Instruction::CopyItem { index: 1_u32 });
                self.compile_store(target)?;
            }
            Expr::FString(fstring) => {
                self.compile_expr_fstring(fstring)?;
            }
            Expr::TString(_) => {
                return Err(self.error(CodegenErrorType::NotImplementedYet));
            }
            Expr::StringLiteral(string) => {
                let value = string.value.to_str();
                if value.contains(char::REPLACEMENT_CHARACTER) {
                    let value = string
                        .value
                        .iter()
                        .map(|lit| {
                            let source = self.source_file.slice(lit.range);
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
        // Method call: obj → LOAD_ATTR_METHOD → [method, self_or_null] → args → CALL
        // Regular call: func → PUSH_NULL → args → CALL
        if let Expr::Attribute(ExprAttribute { value, attr, .. }) = &func {
            // Method call: compile object, then LOAD_ATTR_METHOD
            // LOAD_ATTR_METHOD pushes [method, self_or_null] on stack
            self.compile_expression(value)?;
            let idx = self.name(attr.as_str());
            emit!(self, Instruction::LoadAttrMethod { idx });
            self.compile_call_helper(0, args)?;
        } else {
            // Regular call: push func, then NULL for self_or_null slot
            // Stack layout: [func, NULL, args...] - same as method call [func, self, args...]
            self.compile_expression(func)?;
            emit!(self, Instruction::PushNull);
            self.compile_call_helper(0, args)?;
        }
        Ok(())
    }

    /// Compile call arguments and emit the appropriate CALL instruction.
    /// This is shared between compiler_call and compiler_class.
    fn compile_call_helper(
        &mut self,
        additional_positional: u32,
        arguments: &Arguments,
    ) -> CompileResult<()> {
        let args_count = u32::try_from(arguments.len()).expect("too many arguments");
        let count = args_count
            .checked_add(additional_positional)
            .expect("too many arguments");

        // Normal arguments:
        let (size, unpack) = self.gather_elements(additional_positional, &arguments.args)?;
        let has_double_star = arguments.keywords.iter().any(|k| k.arg.is_none());

        for keyword in &arguments.keywords {
            if let Some(name) = &keyword.arg {
                self.check_forbidden_name(name.as_str(), NameUsage::Store)?;
            }
        }

        if unpack || has_double_star {
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
            emit!(self, Instruction::CallFunctionEx { has_kwargs });
        } else if !arguments.keywords.is_empty() {
            // No **kwargs in this branch (has_double_star is false),
            // so all keywords have arg.is_some()
            let mut kwarg_names = Vec::with_capacity(arguments.keywords.len());
            for keyword in &arguments.keywords {
                let name = keyword
                    .arg
                    .as_ref()
                    .expect("has_double_star is false, so arg must be Some");
                kwarg_names.push(ConstantData::Str {
                    value: name.as_str().into(),
                });
                self.compile_expression(&keyword.value)?;
            }

            self.emit_load_const(ConstantData::Tuple {
                elements: kwarg_names,
            });
            // nargs = positional args + keyword args
            let positional = additional_positional
                .checked_add(u32::try_from(arguments.args.len()).expect("too many positional args"))
                .expect("too many positional args");
            let keyword_count =
                u32::try_from(arguments.keywords.len()).expect("too many keyword args");
            let nargs = positional
                .checked_add(keyword_count)
                .expect("too many arguments");
            emit!(self, Instruction::CallKw { nargs });
        } else {
            emit!(self, Instruction::Call { nargs: count });
        }

        Ok(())
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

        // Check for async comprehension outside async function (list/set/dict only, not generator expressions)
        // Use in_async_scope to allow nested async comprehensions inside an async function
        if comprehension_type != ComprehensionType::Generator
            && (has_an_async_gen || element_contains_await)
            && !prev_ctx.in_async_scope
        {
            return Err(self.error(CodegenErrorType::InvalidAsyncComprehension));
        }

        // Check if this comprehension should be inlined (PEP 709)
        let is_inlined = self.is_inlined_comprehension_context(comprehension_type);

        // async comprehensions are allowed in various contexts:
        // - list/set/dict comprehensions in async functions (or nested within)
        // - always for generator expressions
        let is_async_list_set_dict_comprehension = comprehension_type
            != ComprehensionType::Generator
            && (has_an_async_gen || element_contains_await)
            && prev_ctx.in_async_scope;

        let is_async_generator_comprehension = comprehension_type == ComprehensionType::Generator
            && (has_an_async_gen || element_contains_await);

        debug_assert!(!(is_async_list_set_dict_comprehension && is_async_generator_comprehension));

        let is_async = is_async_list_set_dict_comprehension || is_async_generator_comprehension;

        // We must have at least one generator:
        assert!(!generators.is_empty());

        if is_inlined {
            // PEP 709: Inlined comprehension - compile inline without new scope
            return self.compile_inlined_comprehension(
                init_collection,
                generators,
                compile_element,
                has_an_async_gen,
            );
        }

        // Non-inlined path: create a new code object (generator expressions, etc.)
        self.ctx = CompileContext {
            loop_data: None,
            in_class: prev_ctx.in_class,
            func: if is_async {
                FunctionContext::AsyncFunction
            } else {
                FunctionContext::Function
            },
            // Inherit in_async_scope from parent - nested async comprehensions are allowed
            // if we're anywhere inside an async function
            in_async_scope: prev_ctx.in_async_scope || is_async,
        };

        let flags = bytecode::CodeFlags::NEWLOCALS | bytecode::CodeFlags::OPTIMIZED;
        let flags = if is_async {
            flags | bytecode::CodeFlags::COROUTINE
        } else {
            flags
        };

        // Create magnificent function <listcomp>:
        self.push_output(flags, 1, 1, 0, name.to_owned())?;

        // Mark that we're in an inlined comprehension
        self.current_code_info().in_inlined_comp = true;

        // Set qualname for comprehension
        self.set_qualname();

        let arg0 = self.varname(".0")?;

        let return_none = init_collection.is_none();
        // Create empty object of proper type:
        if let Some(init_collection) = init_collection {
            self._emit(init_collection, OpArg(0), BlockIdx::NULL)
        }

        let mut loop_labels = vec![];
        for generator in generators {
            let loop_block = self.new_block();
            let after_block = self.new_block();

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

            loop_labels.push((loop_block, after_block, generator.is_async));
            self.switch_to_block(loop_block);
            if generator.is_async {
                emit!(self, Instruction::GetANext);

                let current_depth = (init_collection.is_some() as u32)
                    + u32::try_from(loop_labels.len()).unwrap()
                    + 1;
                self.push_fblock_with_handler(
                    FBlockType::AsyncComprehensionGenerator,
                    loop_block,
                    after_block,
                    Some(after_block),
                    current_depth,
                    false,
                )?;
                self.emit_load_const(ConstantData::None);
                self.compile_yield_from_sequence(true)?;
                self.compile_store(&generator.target)?;
                self.pop_fblock(FBlockType::AsyncComprehensionGenerator);
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

        for (loop_block, after_block, is_async) in loop_labels.iter().rev().copied() {
            emit!(self, Instruction::Jump { target: loop_block });

            self.switch_to_block(after_block);
            if is_async {
                emit!(self, Instruction::EndAsyncFor);
                emit!(self, Instruction::PopTop);
            }
        }

        if return_none {
            self.emit_load_const(ConstantData::None)
        }

        self.emit_return_value();

        let code = self.exit_scope();

        self.ctx = prev_ctx;

        // Create comprehension function with closure
        self.make_closure(code, bytecode::MakeFunctionFlags::empty())?;
        emit!(self, Instruction::PushNull);

        // Evaluate iterated item:
        self.compile_expression(&generators[0].iter)?;

        // Get iterator / turn item into an iterator
        // Use is_async from the first generator, not has_an_async_gen which covers ALL generators
        if generators[0].is_async {
            emit!(self, Instruction::GetAIter);
        } else {
            emit!(self, Instruction::GetIter);
        };

        // Call just created <listcomp> function:
        emit!(self, Instruction::Call { nargs: 1 });
        if is_async_list_set_dict_comprehension {
            emit!(self, Instruction::GetAwaitable);
            self.emit_load_const(ConstantData::None);
            self.compile_yield_from_sequence(true)?;
        }

        Ok(())
    }

    /// Collect variable names from an assignment target expression
    fn collect_target_names(&self, target: &Expr, names: &mut Vec<String>) {
        match target {
            Expr::Name(name) => {
                let name_str = name.id.to_string();
                if !names.contains(&name_str) {
                    names.push(name_str);
                }
            }
            Expr::Tuple(tuple) => {
                for elt in &tuple.elts {
                    self.collect_target_names(elt, names);
                }
            }
            Expr::List(list) => {
                for elt in &list.elts {
                    self.collect_target_names(elt, names);
                }
            }
            Expr::Starred(starred) => {
                self.collect_target_names(&starred.value, names);
            }
            _ => {
                // Other targets (attribute, subscript) don't bind local names
            }
        }
    }

    /// Compile an inlined comprehension (PEP 709)
    /// This generates bytecode inline without creating a new code object
    fn compile_inlined_comprehension(
        &mut self,
        init_collection: Option<Instruction>,
        generators: &[Comprehension],
        compile_element: &dyn Fn(&mut Self) -> CompileResult<()>,
        _has_an_async_gen: bool,
    ) -> CompileResult<()> {
        // PEP 709: Consume the comprehension's sub_table (but we won't use it as a separate scope)
        // We need to consume it to keep sub_tables in sync with AST traversal order.
        // The symbols are already merged into parent scope by analyze_symbol_table.
        let _comp_table = self
            .symbol_table_stack
            .last_mut()
            .expect("no current symbol table")
            .sub_tables
            .remove(0);

        // Collect local variables that need to be saved/restored
        // These are variables bound in the comprehension (iteration vars from targets)
        let mut pushed_locals: Vec<String> = Vec::new();
        for generator in generators {
            self.collect_target_names(&generator.target, &mut pushed_locals);
        }

        // Step 1: Compile the outermost iterator
        self.compile_expression(&generators[0].iter)?;
        // Use is_async from the first generator, not has_an_async_gen which covers ALL generators
        if generators[0].is_async {
            emit!(self, Instruction::GetAIter);
        } else {
            emit!(self, Instruction::GetIter);
        }

        // Step 2: Save local variables that will be shadowed by the comprehension
        for name in &pushed_locals {
            let idx = self.varname(name)?;
            emit!(self, Instruction::LoadFastAndClear(idx));
        }

        // Step 3: SWAP iterator to TOS (above saved locals)
        if !pushed_locals.is_empty() {
            emit!(
                self,
                Instruction::Swap {
                    index: u32::try_from(pushed_locals.len() + 1).unwrap()
                }
            );
        }

        // Step 4: Create the collection (list/set/dict)
        // For generator expressions, init_collection is None
        if let Some(init_collection) = init_collection {
            self._emit(init_collection, OpArg(0), BlockIdx::NULL);
            // SWAP to get iterator on top
            emit!(self, Instruction::Swap { index: 2 });
        }

        // Set up exception handler for cleanup on exception
        let cleanup_block = self.new_block();
        let end_block = self.new_block();

        if !pushed_locals.is_empty() {
            // Calculate stack depth for exception handler
            // Stack: [saved_locals..., collection?, iterator]
            let depth = self.handler_stack_depth()
                + u32::try_from(pushed_locals.len()).unwrap()
                + init_collection.is_some() as u32
                + 1;
            self.push_fblock_with_handler(
                FBlockType::TryExcept,
                cleanup_block,
                end_block,
                Some(cleanup_block),
                depth,
                false,
            )?;
        }

        // Step 5: Compile the comprehension loop(s)
        let mut loop_labels = vec![];
        for (i, generator) in generators.iter().enumerate() {
            let loop_block = self.new_block();
            let after_block = self.new_block();

            if i > 0 {
                // For nested loops, compile the iterator expression
                self.compile_expression(&generator.iter)?;
                if generator.is_async {
                    emit!(self, Instruction::GetAIter);
                } else {
                    emit!(self, Instruction::GetIter);
                }
            }

            loop_labels.push((loop_block, after_block, generator.is_async));
            self.switch_to_block(loop_block);

            if generator.is_async {
                emit!(self, Instruction::GetANext);
                self.emit_load_const(ConstantData::None);
                self.compile_yield_from_sequence(true)?;
                self.compile_store(&generator.target)?;
            } else {
                emit!(
                    self,
                    Instruction::ForIter {
                        target: after_block,
                    }
                );
                self.compile_store(&generator.target)?;
            }

            // Evaluate the if conditions
            for if_condition in &generator.ifs {
                self.compile_jump_if(if_condition, false, loop_block)?;
            }
        }

        // Step 6: Compile the element expression and append to collection
        compile_element(self)?;

        // Step 7: Close all loops
        for (loop_block, after_block, is_async) in loop_labels.iter().rev().copied() {
            emit!(self, Instruction::Jump { target: loop_block });
            self.switch_to_block(after_block);
            if is_async {
                emit!(self, Instruction::EndAsyncFor);
            }
            // Pop the iterator
            emit!(self, Instruction::PopTop);
        }

        // Step 8: Clean up - restore saved locals
        if !pushed_locals.is_empty() {
            self.pop_fblock(FBlockType::TryExcept);

            // Normal path: jump past cleanup
            emit!(self, Instruction::Jump { target: end_block });

            // Exception cleanup path
            self.switch_to_block(cleanup_block);
            // Stack: [saved_locals..., collection, exception]
            // Swap to get collection out from under exception
            emit!(self, Instruction::Swap { index: 2 });
            emit!(self, Instruction::PopTop); // Pop incomplete collection

            // Restore locals
            emit!(
                self,
                Instruction::Swap {
                    index: u32::try_from(pushed_locals.len() + 1).unwrap()
                }
            );
            for name in pushed_locals.iter().rev() {
                let idx = self.varname(name)?;
                emit!(self, Instruction::StoreFast(idx));
            }
            // Re-raise the exception
            emit!(
                self,
                Instruction::RaiseVarargs {
                    kind: bytecode::RaiseKind::ReraiseFromStack
                }
            );

            // Normal end path
            self.switch_to_block(end_block);
        }

        // SWAP result to TOS (above saved locals)
        if !pushed_locals.is_empty() {
            emit!(
                self,
                Instruction::Swap {
                    index: u32::try_from(pushed_locals.len() + 1).unwrap()
                }
            );
        }

        // Restore saved locals
        for name in pushed_locals.iter().rev() {
            let idx = self.varname(name)?;
            emit!(self, Instruction::StoreFast(idx));
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
    fn _emit(&mut self, instr: Instruction, arg: OpArg, target: BlockIdx) {
        let range = self.current_source_range;
        let source = self.source_file.to_source_code();
        let location = source.source_location(range.start(), PositionEncoding::Utf8);
        let end_location = source.source_location(range.end(), PositionEncoding::Utf8);
        let except_handler = self.current_except_handler();
        self.current_block().instructions.push(ir::InstructionInfo {
            instr,
            arg,
            target,
            location,
            end_location,
            except_handler,
        });
    }

    fn emit_no_arg(&mut self, ins: Instruction) {
        self._emit(ins, OpArg::null(), BlockIdx::NULL)
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
        info.metadata.consts.insert_full(constant).0.to_u32()
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
        if let Some(inst) = self.current_block().instructions.last_mut()
            && let Instruction::LoadConst { idx } = inst.instr
        {
            inst.instr = Instruction::ReturnConst { idx };
            return;
        }
        emit!(self, Instruction::ReturnValue)
    }

    fn current_code_info(&mut self) -> &mut ir::CodeInfo {
        self.code_stack.last_mut().expect("no code on stack")
    }

    /// Compile break or continue statement with proper fblock cleanup.
    /// compiler_break, compiler_continue
    /// This handles unwinding through With blocks and exception handlers.
    fn compile_break_continue(
        &mut self,
        range: ruff_text_size::TextRange,
        is_break: bool,
    ) -> CompileResult<()> {
        // unwind_fblock_stack
        // We need to unwind fblocks and compile cleanup code. For FinallyTry blocks,
        // we need to compile the finally body inline, but we must temporarily pop
        // the fblock so that nested break/continue in the finally body don't see it.

        // First, find the loop
        let code = self.current_code_info();
        let mut loop_idx = None;
        let mut is_for_loop = false;

        for i in (0..code.fblock.len()).rev() {
            match code.fblock[i].fb_type {
                FBlockType::WhileLoop => {
                    loop_idx = Some(i);
                    is_for_loop = false;
                    break;
                }
                FBlockType::ForLoop => {
                    loop_idx = Some(i);
                    is_for_loop = true;
                    break;
                }
                FBlockType::ExceptionGroupHandler => {
                    return Err(
                        self.error_ranged(CodegenErrorType::BreakContinueReturnInExceptStar, range)
                    );
                }
                _ => {}
            }
        }

        let Some(loop_idx) = loop_idx else {
            if is_break {
                return Err(self.error_ranged(CodegenErrorType::InvalidBreak, range));
            } else {
                return Err(self.error_ranged(CodegenErrorType::InvalidContinue, range));
            }
        };

        let loop_block = code.fblock[loop_idx].fb_block;
        let exit_block = code.fblock[loop_idx].fb_exit;

        // Collect the fblocks we need to unwind through, from top down to (but not including) the loop
        #[derive(Clone)]
        enum UnwindAction {
            With {
                is_async: bool,
            },
            HandlerCleanup,
            FinallyTry {
                body: Vec<ruff_python_ast::Stmt>,
                fblock_idx: usize,
            },
            FinallyEnd,
            PopValue, // Pop return value when continue/break cancels a return
        }
        let mut unwind_actions = Vec::new();

        {
            let code = self.current_code_info();
            for i in (loop_idx + 1..code.fblock.len()).rev() {
                match code.fblock[i].fb_type {
                    FBlockType::With => {
                        unwind_actions.push(UnwindAction::With { is_async: false });
                    }
                    FBlockType::AsyncWith => {
                        unwind_actions.push(UnwindAction::With { is_async: true });
                    }
                    FBlockType::HandlerCleanup => {
                        unwind_actions.push(UnwindAction::HandlerCleanup);
                    }
                    FBlockType::FinallyTry => {
                        // Need to execute finally body before break/continue
                        if let FBlockDatum::FinallyBody(ref body) = code.fblock[i].fb_datum {
                            unwind_actions.push(UnwindAction::FinallyTry {
                                body: body.clone(),
                                fblock_idx: i,
                            });
                        }
                    }
                    FBlockType::FinallyEnd => {
                        // Inside finally block reached via exception - need to pop exception
                        unwind_actions.push(UnwindAction::FinallyEnd);
                    }
                    FBlockType::PopValue => {
                        // Pop the return value that was saved on stack
                        unwind_actions.push(UnwindAction::PopValue);
                    }
                    _ => {}
                }
            }
        }

        // Emit cleanup for each fblock
        for action in unwind_actions {
            match action {
                UnwindAction::With { is_async } => {
                    // compiler_call_exit_with_nones
                    emit!(self, Instruction::PushNull);
                    self.emit_load_const(ConstantData::None);
                    self.emit_load_const(ConstantData::None);
                    self.emit_load_const(ConstantData::None);
                    emit!(self, Instruction::Call { nargs: 3 });

                    if is_async {
                        emit!(self, Instruction::GetAwaitable);
                        self.emit_load_const(ConstantData::None);
                        self.compile_yield_from_sequence(true)?;
                    }

                    emit!(self, Instruction::PopTop);
                }
                UnwindAction::HandlerCleanup => {
                    emit!(self, Instruction::PopExcept);
                }
                UnwindAction::FinallyTry { body, fblock_idx } => {
                    // compile finally body inline
                    // Temporarily pop the FinallyTry fblock so nested break/continue
                    // in the finally body won't see it again.
                    let code = self.current_code_info();
                    let saved_fblock = code.fblock.remove(fblock_idx);

                    self.compile_statements(&body)?;

                    // Restore the fblock (though this break/continue will jump away,
                    // this keeps the fblock stack consistent for error checking)
                    let code = self.current_code_info();
                    code.fblock.insert(fblock_idx, saved_fblock);
                }
                UnwindAction::FinallyEnd => {
                    // Stack when in FinallyEnd: [..., prev_exc, exc]
                    // Note: No lasti here - it's only pushed for cleanup handler exceptions
                    // We need to pop: exc, prev_exc (via PopExcept)
                    emit!(self, Instruction::PopTop); // exc
                    emit!(self, Instruction::PopExcept); // prev_exc is restored
                }
                UnwindAction::PopValue => {
                    // Pop the return value - continue/break cancels the pending return
                    emit!(self, Instruction::PopTop);
                }
            }
        }

        // For break in a for loop, pop the iterator
        if is_break && is_for_loop {
            emit!(self, Instruction::PopTop);
        }

        // Jump to target
        if is_break {
            emit!(self, Instruction::Break { target: exit_block });
        } else {
            emit!(self, Instruction::Continue { target: loop_block });
        }

        Ok(())
    }

    /// Calculate the current exception handler stack depth.
    /// CPython calculates this based on the SETUP_FINALLY/SETUP_CLEANUP stack depth.
    fn handler_stack_depth(&self) -> u32 {
        let code = match self.code_stack.last() {
            Some(c) => c,
            None => return 0,
        };
        let mut depth = 0u32;
        for fblock in &code.fblock {
            match fblock.fb_type {
                FBlockType::ForLoop => depth += 1,
                FBlockType::With | FBlockType::AsyncWith => depth += 1,
                // HandlerCleanup does NOT add to stack depth - it only tracks
                // cleanup code for named exception handlers. The stack item
                // (prev_exc) is already counted by ExceptionHandler.
                // FBlockType::HandlerCleanup => depth += 1,
                // inside exception handler, prev_exc is on stack
                FBlockType::ExceptionHandler => depth += 1,
                // ExceptionGroupHandler: inside except* handler path
                // Stack has [prev_exc, orig, list, rest] - add 4 for these
                FBlockType::ExceptionGroupHandler => depth += 4,
                // FinallyEnd: inside finally exception path
                // Stack has [prev_exc, exc] - add 2 for these (no lasti at this level)
                FBlockType::FinallyEnd => depth += 2,
                // PopValue: preserving a return value on stack during inline finally
                // The return value adds 1 to the stack depth
                FBlockType::PopValue => depth += 1,
                _ => {}
            }
        }
        depth
    }

    fn current_block(&mut self) -> &mut ir::Block {
        let info = self.current_code_info();
        &mut info.blocks[info.current_block]
    }

    fn new_block(&mut self) -> BlockIdx {
        let code = self.current_code_info();
        let idx = BlockIdx::new(code.blocks.len().to_u32());
        code.blocks.push(ir::Block::default());
        idx
    }

    fn switch_to_block(&mut self, block: BlockIdx) {
        let code = self.current_code_info();
        let prev = code.current_block;
        assert_ne!(prev, block, "recursive switching {prev:?} -> {block:?}");
        assert_eq!(
            code.blocks[block].next,
            BlockIdx::NULL,
            "switching {prev:?} -> {block:?} to completed block"
        );
        let prev_block = &mut code.blocks[prev.idx()];
        assert_eq!(
            u32::from(prev_block.next),
            u32::MAX,
            "switching {prev:?} -> {block:?} from block that's already got a next"
        );
        prev_block.next = block;
        code.current_block = block;
    }

    const fn set_source_range(&mut self, range: TextRange) {
        self.current_source_range = range;
    }

    fn get_source_line_number(&mut self) -> OneIndexed {
        self.source_file
            .to_source_code()
            .line_index(self.current_source_range.start())
    }

    fn mark_generator(&mut self) {
        self.current_code_info().flags |= bytecode::CodeFlags::GENERATOR
    }

    /// Whether the expression contains an await expression and
    /// thus requires the function to be async.
    ///
    /// Both:
    /// ```py
    /// async with: ...
    /// async for: ...
    /// ```
    /// are statements, so we won't check for them here
    fn contains_await(expression: &Expr) -> bool {
        #[derive(Default)]
        struct AwaitVisitor {
            found: bool,
        }

        impl Visitor<'_> for AwaitVisitor {
            fn visit_expr(&mut self, expr: &Expr) {
                if self.found {
                    return;
                }

                match expr {
                    Expr::Await(_) => self.found = true,
                    // Note: We do NOT check for async comprehensions here.
                    // Async list/set/dict comprehensions are handled by compile_comprehension
                    // which already awaits the result. A generator expression containing
                    // an async comprehension as its element does NOT become an async generator,
                    // because the async comprehension is awaited when evaluating the element.
                    _ => walk_expr(self, expr),
                }
            }
        }

        let mut visitor = AwaitVisitor::default();
        visitor.visit_expr(expression);
        visitor.found
    }

    /// Check if any of the generators (except the first one's iter) contains an await expression.
    /// The first generator's iter is evaluated outside the comprehension scope.
    fn generators_contain_await(generators: &[Comprehension]) -> bool {
        for (i, generator) in generators.iter().enumerate() {
            // First generator's iter is evaluated outside the comprehension
            if i > 0 && Self::contains_await(&generator.iter) {
                return true;
            }
            // Check ifs in all generators
            for if_expr in &generator.ifs {
                if Self::contains_await(if_expr) {
                    return true;
                }
            }
        }
        false
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
                    let source = self.source_file.slice(string.range);
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
        fstring_elements: &InterpolatedStringElements,
    ) -> CompileResult<()> {
        let mut element_count = 0;
        for element in fstring_elements {
            element_count += 1;
            match element {
                InterpolatedStringElement::Literal(string) => {
                    if string.value.contains(char::REPLACEMENT_CHARACTER) {
                        // might have a surrogate literal; should reparse to be sure
                        let source = self.source_file.slice(string.range);
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
                InterpolatedStringElement::Interpolation(fstring_expr) => {
                    let mut conversion = match fstring_expr.conversion {
                        ConversionFlag::None => ConvertValueOparg::None,
                        ConversionFlag::Str => ConvertValueOparg::Str,
                        ConversionFlag::Repr => ConvertValueOparg::Repr,
                        ConversionFlag::Ascii => ConvertValueOparg::Ascii,
                    };

                    if let Some(DebugText { leading, trailing }) = &fstring_expr.debug_text {
                        let range = fstring_expr.expression.range();
                        let source = self.source_file.slice(range);
                        let text = [leading, source, trailing].concat();

                        self.emit_load_const(ConstantData::Str { value: text.into() });
                        element_count += 1;

                        // If debug text is present, apply repr conversion when no `format_spec` specified.
                        // See action_helpers.c: fstring_find_expr_replacement
                        if matches!(
                            (conversion, &fstring_expr.format_spec),
                            (ConvertValueOparg::None, None)
                        ) {
                            conversion = ConvertValueOparg::Repr;
                        }
                    }

                    self.compile_expression(&fstring_expr.expression)?;

                    match conversion {
                        ConvertValueOparg::None => {}
                        ConvertValueOparg::Str
                        | ConvertValueOparg::Repr
                        | ConvertValueOparg::Ascii => {
                            emit!(self, Instruction::ConvertValue { oparg: conversion })
                        }
                    }

                    match &fstring_expr.format_spec {
                        Some(format_spec) => {
                            self.compile_fstring_elements(flags, &format_spec.elements)?;

                            emit!(self, Instruction::FormatWithSpec);
                        }
                        None => {
                            emit!(self, Instruction::FormatSimple);
                        }
                    }
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
    ) -> (Instruction, OpArg, BlockIdx);
}
impl<T: OpArgType> EmitArg<T> for T {
    fn emit(self, f: impl FnOnce(OpArgMarker<T>) -> Instruction) -> (Instruction, OpArg, BlockIdx) {
        let (marker, arg) = OpArgMarker::new(self);
        (f(marker), arg, BlockIdx::NULL)
    }
}
impl EmitArg<bytecode::Label> for BlockIdx {
    fn emit(
        self,
        f: impl FnOnce(OpArgMarker<bytecode::Label>) -> Instruction,
    ) -> (Instruction, OpArg, BlockIdx) {
        (f(OpArgMarker::marker()), OpArg::null(), self)
    }
}

/// Strips leading whitespace from a docstring.
///
/// `inspect.cleandoc` is a good reference, but has a few incompatibilities.
// = _PyCompile_CleanDoc
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
    let s = format!("{int}");
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
            node_index: AtomicNodeIndex::NONE,
            range,
            id: Name::new("x"),
            ctx: ExprContext::Load,
        });
        let not_present = &Expr::FString(ExprFString {
            node_index: AtomicNodeIndex::NONE,
            range,
            value: FStringValue::single(FString {
                node_index: AtomicNodeIndex::NONE,
                range,
                elements: vec![InterpolatedStringElement::Interpolation(
                    InterpolatedElement {
                        node_index: AtomicNodeIndex::NONE,
                        range,
                        expression: Box::new(expr_x),
                        debug_text: None,
                        conversion: ConversionFlag::None,
                        format_spec: None,
                    },
                )]
                .into(),
                flags,
            }),
        });
        assert!(!Compiler::contains_await(not_present));

        // f'{await x}'
        let expr_await_x = Expr::Await(ExprAwait {
            node_index: AtomicNodeIndex::NONE,
            range,
            value: Box::new(Expr::Name(ExprName {
                node_index: AtomicNodeIndex::NONE,
                range,
                id: Name::new("x"),
                ctx: ExprContext::Load,
            })),
        });
        let present = &Expr::FString(ExprFString {
            node_index: AtomicNodeIndex::NONE,
            range,
            value: FStringValue::single(FString {
                node_index: AtomicNodeIndex::NONE,
                range,
                elements: vec![InterpolatedStringElement::Interpolation(
                    InterpolatedElement {
                        node_index: AtomicNodeIndex::NONE,
                        range,
                        expression: Box::new(expr_await_x),
                        debug_text: None,
                        conversion: ConversionFlag::None,
                        format_spec: None,
                    },
                )]
                .into(),
                flags,
            }),
        });
        assert!(Compiler::contains_await(present));

        // f'{x:{await y}}'
        let expr_x = Expr::Name(ExprName {
            node_index: AtomicNodeIndex::NONE,
            range,
            id: Name::new("x"),
            ctx: ExprContext::Load,
        });
        let expr_await_y = Expr::Await(ExprAwait {
            node_index: AtomicNodeIndex::NONE,
            range,
            value: Box::new(Expr::Name(ExprName {
                node_index: AtomicNodeIndex::NONE,
                range,
                id: Name::new("y"),
                ctx: ExprContext::Load,
            })),
        });
        let present = &Expr::FString(ExprFString {
            node_index: AtomicNodeIndex::NONE,
            range,
            value: FStringValue::single(FString {
                node_index: AtomicNodeIndex::NONE,
                range,
                elements: vec![InterpolatedStringElement::Interpolation(
                    InterpolatedElement {
                        node_index: AtomicNodeIndex::NONE,
                        range,
                        expression: Box::new(expr_x),
                        debug_text: None,
                        conversion: ConversionFlag::None,
                        format_spec: Some(Box::new(InterpolatedStringFormatSpec {
                            node_index: AtomicNodeIndex::NONE,
                            range,
                            elements: vec![InterpolatedStringElement::Interpolation(
                                InterpolatedElement {
                                    node_index: AtomicNodeIndex::NONE,
                                    range,
                                    expression: Box::new(expr_await_y),
                                    debug_text: None,
                                    conversion: ConversionFlag::None,
                                    format_spec: None,
                                },
                            )]
                            .into(),
                        })),
                    },
                )]
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
    use rustpython_compiler_core::SourceFileBuilder;

    fn compile_exec(source: &str) -> CodeObject {
        let opts = CompileOpts::default();
        let source_file = SourceFileBuilder::new("source_path", source).finish();
        let parsed = ruff_python_parser::parse(
            source_file.source_text(),
            ruff_python_parser::Mode::Module.into(),
        )
        .unwrap();
        let ast = parsed.into_syntax();
        let ast = match ast {
            ruff_python_ast::Mod::Module(stmts) => stmts,
            _ => unreachable!(),
        };
        let symbol_table = SymbolTable::scan_program(&ast, source_file.clone())
            .map_err(|e| e.into_codegen_error(source_file.name().to_owned()))
            .unwrap();
        let mut compiler = Compiler::new(opts, source_file, "<module>".to_owned());
        compiler.compile_program(&ast, symbol_table).unwrap();
        compiler.exit_scope()
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
async def test():
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
