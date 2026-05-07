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
    preprocess,
    symboltable::{self, CompilerScope, Symbol, SymbolFlags, SymbolScope, SymbolTable},
    unparse::UnparseExpr,
};
use alloc::borrow::Cow;
use core::mem;
use itertools::Itertools;
use malachite_bigint::BigInt;
use num_complex::Complex;
use num_traits::{Num, ToPrimitive, Zero};
use ruff_python_ast as ast;
use ruff_text_size::{Ranged, TextRange, TextSize};
use rustpython_compiler_core::{
    Mode, OneIndexed, PositionEncoding, SourceFile, SourceLocation,
    bytecode::{
        self, AnyInstruction, Arg as OpArgMarker, BinaryOperator, BuildSliceArgCount, CodeObject,
        ComparisonOperator, ConstantData, ConvertValueOparg, Instruction, InstructionMetadata,
        IntrinsicFunction1, Invert, LoadAttr, LoadSuperAttr, OpArg, OpArgType, PseudoInstruction,
        SpecialMethod, UnpackExArgs, oparg,
    },
};
use rustpython_wtf8::Wtf8Buf;

/// Extension trait for `ast::Expr` to add constant checking methods
trait ExprExt {
    /// Check if an expression is a constant literal
    fn is_constant(&self) -> bool;

    /// Check if a slice expression has all constant elements
    fn is_constant_slice(&self) -> bool;

    /// Check if we should use BINARY_SLICE/STORE_SLICE optimization
    fn should_use_slice_optimization(&self) -> bool;
}

impl ExprExt for ast::Expr {
    fn is_constant(&self) -> bool {
        matches!(
            self,
            ast::Expr::NumberLiteral(_)
                | ast::Expr::StringLiteral(_)
                | ast::Expr::BytesLiteral(_)
                | ast::Expr::NoneLiteral(_)
                | ast::Expr::BooleanLiteral(_)
                | ast::Expr::EllipsisLiteral(_)
        )
    }

    fn is_constant_slice(&self) -> bool {
        match self {
            ast::Expr::Slice(s) => {
                let lower_const =
                    s.lower.is_none() || s.lower.as_deref().is_some_and(|e| e.is_constant());
                let upper_const =
                    s.upper.is_none() || s.upper.as_deref().is_some_and(|e| e.is_constant());
                let step_const =
                    s.step.is_none() || s.step.as_deref().is_some_and(|e| e.is_constant());
                lower_const && upper_const && step_const
            }
            _ => false,
        }
    }

    fn should_use_slice_optimization(&self) -> bool {
        !self.is_constant_slice() && matches!(self, ast::Expr::Slice(s) if s.step.is_none())
    }
}

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
    FinallyBody(Vec<ast::Stmt>),
    /// For HandlerCleanup: stores the exception variable name (e.g., "e" in "except X as e")
    ExceptionName(String),
}

/// Type of super() call optimization detected by can_optimize_super_call()
#[derive(Debug, Clone)]
enum SuperCallType<'a> {
    /// super(class, self) - explicit 2-argument form
    TwoArg {
        class_arg: &'a ast::Expr,
        self_arg: &'a ast::Expr,
    },
    /// super() - implicit 0-argument form (uses __class__ cell)
    ZeroArg,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BuiltinGeneratorCallKind {
    Tuple,
    All,
    Any,
}

#[derive(Debug, Clone)]
pub struct FBlockInfo {
    pub fb_type: FBlockType,
    pub fb_block: BlockIdx,
    pub fb_exit: BlockIdx,
    pub fb_range: TextRange,
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
    /// True when compiling in "single" (interactive) mode.
    /// Expression statements at module scope emit CALL_INTRINSIC_1(Print).
    interactive: bool,
    /// Counter for dead-code elimination during constant folding.
    /// When > 0, the compiler walks AST (consuming sub_tables) but emits no bytecode.
    /// Mirrors CPython's `c_do_not_emit_bytecode`.
    do_not_emit_bytecode: u32,
    /// Disable constant BoolOp folding in contexts where CPython preserves
    /// short-circuit structure, such as starred unpack expressions.
    disable_const_boolop_folding: bool,
    /// Disable constant tuple/list/set collection folding in contexts where
    /// CPython keeps the builder form for later assignment lowering.
    disable_const_collection_folding: bool,
}

#[derive(Clone, Copy)]
enum DoneWithFuture {
    No,
    DoneWithDoc,
    Yes,
}

#[derive(Clone, Copy, Debug)]
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

#[derive(Debug, Clone, Copy)]
enum ComprehensionLoopControl {
    Iteration {
        loop_block: BlockIdx,
        if_cleanup_block: BlockIdx,
        after_block: BlockIdx,
        is_async: bool,
        end_async_for_target: BlockIdx,
    },
    IfCleanupOnly {
        if_cleanup_block: BlockIdx,
    },
}

fn validate_duplicate_params(params: &ast::Parameters) -> Result<(), CodegenErrorType> {
    let mut seen_params = IndexSet::default();
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
    mut ast: ruff_python_ast::Mod,
    source_file: SourceFile,
    mode: Mode,
    opts: CompileOpts,
) -> CompileResult<CodeObject> {
    preprocess::preprocess_mod(&mut ast);
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
    ast: &ast::ModModule,
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
    ast: &ast::ModModule,
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
    ast: &ast::ModModule,
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
    ast: &ast::ModExpression,
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
    // Struct variant with single identifier (e.g., Foo::A { arg })
    ($c:expr, $enum:ident :: $op:ident { $arg:ident $(,)? } $(,)?) => {
        $c.emit_arg($arg, |x| $enum::$op { $arg: x })
    };

    // Struct variant with explicit value (e.g., Foo::A { arg: 42 })
    ($c:expr, $enum:ident :: $op:ident { $arg:ident : $arg_val:expr $(,)? } $(,)?) => {
        $c.emit_arg($arg_val, |x| $enum::$op { $arg: x })
    };

    // Tuple variant (e.g., Foo::B(42)). Should never be reached, here for validation.
    ($c:expr, $enum:ident :: $op:ident($arg_val:expr $(,)? ) $(,)?) => {
        panic!("No instruction should be defined as `Instruction::Foo(value)` use `Instruction::Foo { x: value }` instead")
    };

    // No-arg variant (e.g., Foo::C)
    ($c:expr, $enum:ident :: $op:ident $(,)?) => {
        $c.emit_no_arg($enum::$op)
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
    #[must_use]
    pub const fn new() -> Self {
        Self {
            stores: Vec::new(),
            allow_irrefutable: false,
            fail_pop: Vec::new(),
            on_top: 0,
        }
    }

    #[must_use]
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

const STACK_USE_GUIDELINE: u32 = 30;

impl Compiler {
    fn constant_truthiness(constant: &ConstantData) -> bool {
        match constant {
            ConstantData::Tuple { elements } | ConstantData::Frozenset { elements } => {
                !elements.is_empty()
            }
            ConstantData::Integer { value } => !value.is_zero(),
            ConstantData::Float { value } => *value != 0.0,
            ConstantData::Complex { value } => value.re != 0.0 || value.im != 0.0,
            ConstantData::Boolean { value } => *value,
            ConstantData::Str { value } => !value.is_empty(),
            ConstantData::Bytes { value } => !value.is_empty(),
            ConstantData::Code { .. } | ConstantData::Slice { .. } | ConstantData::Ellipsis => true,
            ConstantData::None => false,
        }
    }

    fn boolop_fast_fold_literal(expr: &ast::Expr) -> bool {
        matches!(
            expr,
            ast::Expr::NumberLiteral(_)
                | ast::Expr::StringLiteral(_)
                | ast::Expr::BytesLiteral(_)
                | ast::Expr::BooleanLiteral(_)
                | ast::Expr::NoneLiteral(_)
                | ast::Expr::EllipsisLiteral(_)
        )
    }

    fn constant_expr_truthiness(&mut self, expr: &ast::Expr) -> CompileResult<Option<bool>> {
        Ok(self
            .try_fold_constant_expr(expr)?
            .map(|constant| Self::constant_truthiness(&constant)))
    }

    fn disable_load_fast_borrow_for_block(&mut self, block: BlockIdx) {
        if block != BlockIdx::NULL {
            self.current_code_info().blocks[block.idx()].disable_load_fast_borrow = true;
        }
    }

    fn new(opts: CompileOpts, source_file: SourceFile, code_name: String) -> Self {
        let module_code = ir::CodeInfo {
            flags: bytecode::CodeFlags::NEWLOCALS,
            source_path: source_file.name().to_owned(),
            private: None,
            blocks: vec![ir::Block::default()],
            current_block: BlockIdx::new(0),
            annotations_blocks: None,
            metadata: ir::CodeUnitMetadata {
                name: code_name.clone(),
                qualname: Some(code_name),
                consts: IndexSet::default(),
                names: IndexSet::default(),
                varnames: IndexSet::default(),
                cellvars: IndexSet::default(),
                freevars: IndexSet::default(),
                fast_hidden: IndexMap::default(),
                fast_hidden_final: IndexSet::default(),
                argcount: 0,
                posonlyargcount: 0,
                kwonlyargcount: 0,
                firstlineno: OneIndexed::MIN,
            },
            static_attributes: None,
            in_inlined_comp: false,
            fblock: Vec::with_capacity(MAXBLOCKS),
            symbol_table_index: 0, // Module is always the first symbol table
            in_conditional_block: 0,
            next_conditional_annotation_index: 0,
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
            interactive: false,
            do_not_emit_bytecode: 0,
            disable_const_boolop_folding: false,
            disable_const_collection_folding: false,
        }
    }

    fn compile_expression_without_const_boolop_folding(
        &mut self,
        expression: &ast::Expr,
    ) -> CompileResult<()> {
        let previous = self.disable_const_boolop_folding;
        self.disable_const_boolop_folding = true;
        let result = self.compile_expression(expression);
        self.disable_const_boolop_folding = previous;
        result.map(|_| ())
    }

    fn compile_expression_without_const_collection_folding(
        &mut self,
        expression: &ast::Expr,
    ) -> CompileResult<()> {
        let previous = self.disable_const_collection_folding;
        self.disable_const_collection_folding = true;
        let result = self.compile_expression(expression);
        self.disable_const_collection_folding = previous;
        result.map(|_| ())
    }

    fn is_unpack_assignment_target(target: &ast::Expr) -> bool {
        matches!(target, ast::Expr::List(_) | ast::Expr::Tuple(_))
    }

    fn statements_end_with_scope_exit(body: &[ast::Stmt]) -> bool {
        body.last()
            .is_some_and(Self::statement_ends_with_scope_exit)
    }

    fn statements_end_with_finally_entry_scope_exit(body: &[ast::Stmt]) -> bool {
        body.last()
            .is_some_and(Self::statement_ends_with_finally_entry_scope_exit)
    }

    fn statement_ends_with_finally_entry_scope_exit(stmt: &ast::Stmt) -> bool {
        match stmt {
            ast::Stmt::Return(_)
            | ast::Stmt::Raise(_)
            | ast::Stmt::Break(_)
            | ast::Stmt::Continue(_) => true,
            ast::Stmt::If(ast::StmtIf { body, .. }) => {
                Self::statements_end_with_finally_entry_scope_exit(body)
            }
            _ => false,
        }
    }

    fn statement_ends_with_scope_exit(stmt: &ast::Stmt) -> bool {
        match stmt {
            ast::Stmt::Return(_) | ast::Stmt::Raise(_) => true,
            ast::Stmt::If(ast::StmtIf {
                body,
                elif_else_clauses,
                ..
            }) => {
                let has_else = elif_else_clauses
                    .last()
                    .is_some_and(|clause| clause.test.is_none());
                has_else
                    && Self::statements_end_with_scope_exit(body)
                    && elif_else_clauses
                        .iter()
                        .all(|clause| Self::statements_end_with_scope_exit(&clause.body))
            }
            _ => false,
        }
    }

    fn statements_end_with_with_cleanup_scope_exit(body: &[ast::Stmt]) -> bool {
        body.last().is_some_and(|stmt| match stmt {
            ast::Stmt::With(ast::StmtWith { body, .. }) => {
                Self::statements_end_with_scope_exit(body)
                    || Self::statements_end_with_with_cleanup_scope_exit(body)
            }
            _ => false,
        })
    }

    fn preserves_finally_entry_nop(body: &[ast::Stmt]) -> bool {
        body.last().is_some_and(|stmt| match stmt {
            ast::Stmt::Try(ast::StmtTry {
                body,
                handlers,
                finalbody,
                ..
            }) => {
                !finalbody.is_empty()
                    || (!handlers.is_empty() && Self::statements_end_with_scope_exit(body))
            }
            ast::Stmt::If(ast::StmtIf {
                body,
                elif_else_clauses,
                ..
            }) => {
                elif_else_clauses.is_empty()
                    && Self::statements_end_with_finally_entry_scope_exit(body)
            }
            _ => false,
        })
    }

    fn compile_module_annotation_setup_sequence(
        &mut self,
        body: &[ast::Stmt],
    ) -> CompileResult<()> {
        let (saved_blocks, saved_current_block) = {
            let code = self.current_code_info();
            (
                mem::replace(&mut code.blocks, vec![ir::Block::default()]),
                mem::replace(&mut code.current_block, BlockIdx::new(0)),
            )
        };

        let result = self.compile_module_annotate(body);

        let annotations_blocks = {
            let code = self.current_code_info();
            let annotations_blocks = mem::replace(&mut code.blocks, saved_blocks);
            code.current_block = saved_current_block;
            annotations_blocks
        };
        self.current_code_info().annotations_blocks = Some(annotations_blocks);

        result.map(|_| ())
    }

    /// Compile just start and stop of a slice (for BINARY_SLICE/STORE_SLICE)
    // = codegen_slice_two_parts
    fn compile_slice_two_parts(&mut self, s: &ast::ExprSlice) -> CompileResult<()> {
        // Compile lower (or None)
        if let Some(lower) = &s.lower {
            self.compile_expression(lower)?;
        } else {
            self.emit_load_const(ConstantData::None);
        }

        // Compile upper (or None)
        if let Some(upper) = &s.upper {
            self.compile_expression(upper)?;
        } else {
            self.emit_load_const(ConstantData::None);
        }

        Ok(())
    }
    /// Compile a subscript expression
    // = compiler_subscript
    fn compile_subscript(
        &mut self,
        value: &ast::Expr,
        slice: &ast::Expr,
        ctx: ast::ExprContext,
    ) -> CompileResult<()> {
        // Save full subscript expression range (set by compile_expression before this call)
        let subscript_range = self.current_source_range;

        // VISIT(c, expr, e->v.Subscript.value)
        self.compile_expression(value)?;

        // Handle two-element non-constant slice with BINARY_SLICE/STORE_SLICE
        let use_slice_opt = matches!(ctx, ast::ExprContext::Load | ast::ExprContext::Store)
            && slice.should_use_slice_optimization();
        if use_slice_opt {
            match slice {
                ast::Expr::Slice(s) => self.compile_slice_two_parts(s)?,
                _ => unreachable!(
                    "should_use_slice_optimization should only return true for ast::Expr::Slice"
                ),
            };
        } else {
            // VISIT(c, expr, e->v.Subscript.slice)
            self.compile_expression(slice)?;
        }

        // Restore full subscript expression range before emitting
        self.set_source_range(subscript_range);

        match (use_slice_opt, ctx) {
            (true, ast::ExprContext::Load) => emit!(self, Instruction::BinarySlice),
            (true, ast::ExprContext::Store) => emit!(self, Instruction::StoreSlice),
            (true, _) => unreachable!(),
            (false, ast::ExprContext::Load) => emit!(
                self,
                Instruction::BinaryOp {
                    op: BinaryOperator::Subscr
                }
            ),
            (false, ast::ExprContext::Store) => emit!(self, Instruction::StoreSubscr),
            (false, ast::ExprContext::Del) => emit!(self, Instruction::DeleteSubscr),
            (false, ast::ExprContext::Invalid) => {
                return Err(self.error(CodegenErrorType::SyntaxError(
                    "Invalid expression context".to_owned(),
                )));
            }
        }

        Ok(())
    }

    /// Helper function for compiling tuples/lists/sets with starred expressions
    ///
    /// ast::Parameters:
    /// - elts: The elements to compile
    /// - pushed: Number of items already on the stack
    /// - collection_type: What type of collection to build (tuple, list, set)
    ///
    // = starunpack_helper in compile.c
    fn starunpack_helper(
        &mut self,
        elts: &[ast::Expr],
        pushed: u32,
        collection_type: CollectionType,
    ) -> CompileResult<()> {
        let collection_range = self.current_source_range;
        let n = elts.len().to_u32();
        let seen_star = elts.iter().any(|e| matches!(e, ast::Expr::Starred(_)));

        let big = n + pushed > STACK_USE_GUIDELINE;

        // Match CPython's constant ordering by letting the late flowgraph-style
        // folding passes introduce tuple-backed constants after their operands
        // have first been emitted as constants.
        let can_fold_const_collection = false;
        if !self.disable_const_collection_folding
            && !seen_star
            && pushed == 0
            && can_fold_const_collection
            && let Some(folded) = self.try_fold_constant_collection(elts, collection_type)?
        {
            match collection_type {
                CollectionType::Tuple => {
                    self.emit_load_const(folded);
                }
                CollectionType::List => {
                    self.set_source_range(collection_range);
                    emit!(self, Instruction::BuildList { count: 0 });
                    self.emit_load_const(folded);
                    self.set_source_range(collection_range);
                    emit!(self, Instruction::ListExtend { i: 1 });
                }
                CollectionType::Set => {
                    self.set_source_range(collection_range);
                    emit!(self, Instruction::BuildSet { count: 0 });
                    self.emit_load_const(folded);
                    self.set_source_range(collection_range);
                    emit!(self, Instruction::SetUpdate { i: 1 });
                }
            }
            return Ok(());
        }

        // If no stars and not too big, compile all elements and build once
        if !seen_star && !big {
            for elt in elts {
                self.compile_expression(elt)?;
            }
            let total_size = n + pushed;
            self.set_source_range(collection_range);
            match collection_type {
                CollectionType::List => {
                    emit!(self, Instruction::BuildList { count: total_size });
                }
                CollectionType::Set => {
                    emit!(self, Instruction::BuildSet { count: total_size });
                }
                CollectionType::Tuple => {
                    emit!(self, Instruction::BuildTuple { count: total_size });
                }
            }
            return Ok(());
        }

        // Has stars or too big: use streaming approach.
        let mut sequence_built = false;
        let mut i = 0u32;

        if big {
            match collection_type {
                CollectionType::List => {
                    emit!(self, Instruction::BuildList { count: pushed });
                    sequence_built = true;
                }
                CollectionType::Set => {
                    emit!(self, Instruction::BuildSet { count: pushed });
                    sequence_built = true;
                }
                CollectionType::Tuple => {
                    emit!(self, Instruction::BuildList { count: pushed });
                    sequence_built = true;
                }
            }
        }

        for elt in elts.iter() {
            if let ast::Expr::Starred(ast::ExprStarred { value, .. }) = elt {
                // When we hit first star, build sequence with elements so far
                if !sequence_built {
                    self.set_source_range(collection_range);
                    match collection_type {
                        CollectionType::List => {
                            emit!(self, Instruction::BuildList { count: i + pushed });
                        }
                        CollectionType::Set => {
                            emit!(self, Instruction::BuildSet { count: i + pushed });
                        }
                        CollectionType::Tuple => {
                            emit!(self, Instruction::BuildList { count: i + pushed });
                        }
                    }
                    sequence_built = true;
                }

                // Compile the starred expression and extend
                self.compile_expression_without_const_boolop_folding(value)?;
                self.set_source_range(collection_range);
                match collection_type {
                    CollectionType::List => {
                        emit!(self, Instruction::ListExtend { i: 1 });
                    }
                    CollectionType::Set => {
                        emit!(self, Instruction::SetUpdate { i: 1 });
                    }
                    CollectionType::Tuple => {
                        emit!(self, Instruction::ListExtend { i: 1 });
                    }
                }
            } else {
                // Non-starred element
                self.compile_expression(elt)?;

                if sequence_built {
                    // Sequence already exists, append to it
                    self.set_source_range(collection_range);
                    match collection_type {
                        CollectionType::List => {
                            emit!(self, Instruction::ListAppend { i: 1 });
                        }
                        CollectionType::Set => {
                            emit!(self, Instruction::SetAdd { i: 1 });
                        }
                        CollectionType::Tuple => {
                            emit!(self, Instruction::ListAppend { i: 1 });
                        }
                    }
                } else {
                    // Still collecting elements before first star
                    i += 1;
                }
            }
        }

        // If we never built sequence (all non-starred), build it now
        if !sequence_built {
            self.set_source_range(collection_range);
            match collection_type {
                CollectionType::List => {
                    emit!(self, Instruction::BuildList { count: i + pushed });
                }
                CollectionType::Set => {
                    emit!(self, Instruction::BuildSet { count: i + pushed });
                }
                CollectionType::Tuple => {
                    emit!(self, Instruction::BuildTuple { count: i + pushed });
                }
            }
        } else if collection_type == CollectionType::Tuple {
            // For tuples, convert the list to tuple
            self.set_source_range(collection_range);
            emit!(
                self,
                Instruction::CallIntrinsic1 {
                    func: IntrinsicFunction1::ListToTuple
                }
            );
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

    /// Match CPython's `is_import_originated()`: only imports recorded in the
    /// module-level symbol table suppress method-call optimization.
    fn is_name_imported(&self, name: &str) -> bool {
        self.symbol_table_stack
            .first()
            .and_then(|table| table.symbols.get(name))
            .is_some_and(|sym| sym.flags.contains(SymbolFlags::IMPORTED))
    }

    /// Get the cell-relative index of a free variable.
    /// Returns ncells + freevar_idx. Fixed up to localsplus index during finalize.
    fn get_free_var_index(&mut self, name: &str) -> CompileResult<oparg::VarNum> {
        let info = self.code_stack.last_mut().unwrap();
        let idx = info
            .metadata
            .freevars
            .get_index_of(name)
            .unwrap_or_else(|| info.metadata.freevars.insert_full(name.to_owned()).0);
        Ok((idx + info.metadata.cellvars.len()).to_u32().into())
    }

    /// Get the cell-relative index of a cell variable.
    /// Returns cellvar_idx. Fixed up to localsplus index during finalize.
    fn get_cell_var_index(&mut self, name: &str) -> CompileResult<oparg::VarNum> {
        let info = self.code_stack.last_mut().unwrap();
        let idx = info
            .metadata
            .cellvars
            .get_index_of(name)
            .unwrap_or_else(|| info.metadata.cellvars.insert_full(name.to_owned()).0);
        Ok(idx.to_u32().into())
    }

    /// Get the index of a local variable.
    fn get_local_var_index(&mut self, name: &str) -> CompileResult<oparg::VarNum> {
        let info = self.code_stack.last_mut().unwrap();
        let idx = info
            .metadata
            .varnames
            .get_index_of(name)
            .unwrap_or_else(|| info.metadata.varnames.insert_full(name.to_owned()).0);
        Ok(idx.to_u32().into())
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

    /// Push the annotation symbol table from the next sub_table's annotation_block
    /// The annotation_block is stored in the function's scope, which is the next sub_table
    /// Returns true if annotation_block exists, false otherwise
    fn push_annotation_symbol_table(&mut self) -> bool {
        let current_table = self
            .symbol_table_stack
            .last_mut()
            .expect("no current symbol table");

        // The annotation_block is in the next sub_table (function scope)
        let next_idx = current_table.next_sub_table;
        if next_idx >= current_table.sub_tables.len() {
            return false;
        }

        let next_table = &mut current_table.sub_tables[next_idx];
        if let Some(annotation_block) = next_table.annotation_block.take() {
            self.symbol_table_stack.push(*annotation_block);
            true
        } else {
            false
        }
    }

    /// Push the annotation symbol table for module/class level annotations
    /// This takes annotation_block from the current symbol table (not sub_tables)
    fn push_current_annotation_symbol_table(&mut self) -> bool {
        let current_table = self
            .symbol_table_stack
            .last_mut()
            .expect("no current symbol table");

        // For modules/classes, annotation_block is directly in the current table
        if let Some(annotation_block) = current_table.annotation_block.take() {
            self.symbol_table_stack.push(*annotation_block);
            true
        } else {
            false
        }
    }

    /// Pop the annotation symbol table and restore it to the function scope's annotation_block
    fn pop_annotation_symbol_table(&mut self) {
        let annotation_table = self.symbol_table_stack.pop().expect("compiler bug");
        let current_table = self
            .symbol_table_stack
            .last_mut()
            .expect("no current symbol table");

        // Restore to the next sub_table (function scope) where it came from
        let next_idx = current_table.next_sub_table;
        if next_idx < current_table.sub_tables.len() {
            current_table.sub_tables[next_idx].annotation_block = Some(Box::new(annotation_table));
        }
    }

    /// Pop the current symbol table off the stack
    fn pop_symbol_table(&mut self) -> SymbolTable {
        self.symbol_table_stack.pop().expect("compiler bug")
    }

    /// Check if a super() call can be optimized
    /// Returns Some(SuperCallType) if optimization is possible, None otherwise
    fn can_optimize_super_call<'a>(
        &self,
        value: &'a ast::Expr,
        attr: &str,
    ) -> Option<SuperCallType<'a>> {
        // 1. value must be a Call expression
        let ast::Expr::Call(ast::ExprCall {
            func, arguments, ..
        }) = value
        else {
            return None;
        };

        // 2. func must be Name("super")
        let ast::Expr::Name(ast::ExprName { id, .. }) = func.as_ref() else {
            return None;
        };
        if id.as_str() != "super" {
            return None;
        }

        // 3. attr must not be "__class__"
        if attr == "__class__" {
            return None;
        }

        // 4. No keyword arguments
        if !arguments.keywords.is_empty() {
            return None;
        }

        // 5. Must be inside a function (not at module level or class body)
        if !self.ctx.in_func() {
            return None;
        }

        // 6. "super" must be GlobalImplicit (not redefined locally or at module level)
        let table = self.current_symbol_table();
        if let Some(symbol) = table.lookup("super")
            && symbol.scope != SymbolScope::GlobalImplicit
        {
            return None;
        }
        // Also check top-level scope to detect module-level shadowing.
        // Only block if super is actually *bound* at module level (not just used).
        if let Some(top_table) = self.symbol_table_stack.first()
            && let Some(sym) = top_table.lookup("super")
            && sym.scope != SymbolScope::GlobalImplicit
        {
            return None;
        }

        // 7. Check argument pattern
        let args = &arguments.args;

        // No starred expressions allowed
        if args.iter().any(|arg| matches!(arg, ast::Expr::Starred(_))) {
            return None;
        }

        match args.len() {
            2 => {
                // 2-arg: super(class, self)
                Some(SuperCallType::TwoArg {
                    class_arg: &args[0],
                    self_arg: &args[1],
                })
            }
            0 => {
                // 0-arg: super() - need __class__ cell and first parameter
                // Enclosing function should have at least one positional argument
                let info = self.code_stack.last()?;
                if info.metadata.argcount == 0 && info.metadata.posonlyargcount == 0 {
                    return None;
                }

                // Check if __class__ is available as a cell/free variable
                // The scope must be Free (from enclosing class) or have FREE_CLASS flag
                if let Some(symbol) = table.lookup("__class__") {
                    if symbol.scope != SymbolScope::Free
                        && !symbol.flags.contains(SymbolFlags::FREE_CLASS)
                    {
                        return None;
                    }
                } else {
                    // __class__ not in symbol table, optimization not possible
                    return None;
                }

                Some(SuperCallType::ZeroArg)
            }
            _ => None, // 1 or 3+ args - not optimizable
        }
    }

    /// Load arguments for super() optimization onto the stack
    /// Stack result: [global_super, class, self]
    fn load_args_for_super(&mut self, super_type: &SuperCallType<'_>) -> CompileResult<()> {
        // 1. Load global super
        self.compile_name("super", NameUsage::Load)?;

        match super_type {
            SuperCallType::TwoArg {
                class_arg,
                self_arg,
            } => {
                // 2-arg: load provided arguments
                self.compile_expression(class_arg)?;
                self.compile_expression(self_arg)?;
            }
            SuperCallType::ZeroArg => {
                // 0-arg: load __class__ cell and first parameter
                // Load __class__ from cell/free variable
                let scope = self.get_ref_type("__class__").map_err(|e| self.error(e))?;
                let idx = match scope {
                    SymbolScope::Cell => self.get_cell_var_index("__class__")?,
                    SymbolScope::Free => self.get_free_var_index("__class__")?,
                    _ => {
                        return Err(self.error(CodegenErrorType::SyntaxError(
                            "super(): __class__ cell not found".to_owned(),
                        )));
                    }
                };
                emit!(self, Instruction::LoadDeref { i: idx });

                // Load first parameter (typically 'self').
                // Safety: can_optimize_super_call() ensures argcount > 0, and
                // parameters are always added to varnames first (see symboltable.rs).
                let first_param = {
                    let info = self.code_stack.last().unwrap();
                    info.metadata.varnames.first().cloned()
                };
                let first_param = first_param.ok_or_else(|| {
                    self.error(CodegenErrorType::SyntaxError(
                        "super(): no arguments and no first parameter".to_owned(),
                    ))
                })?;
                self.compile_name(&first_param, NameUsage::Load)?;
            }
        }
        Ok(())
    }

    /// Check if this is an inlined comprehension context (PEP 709).
    /// Generator expressions are never inlined.
    fn is_inlined_comprehension_context(
        &self,
        comprehension_type: ComprehensionType,
        comp_table: &SymbolTable,
    ) -> bool {
        if comprehension_type == ComprehensionType::Generator {
            return false;
        }
        comp_table.comp_inlined
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

        // Build cellvars using dictbytype (CELL scope or COMP_CELL flag, sorted)
        let mut cellvar_cache = IndexSet::default();
        // CPython ordering: parameter cells first (in parameter order),
        // then non-parameter cells (alphabetically sorted)
        let cell_symbols: Vec<_> = ste
            .symbols
            .iter()
            .filter(|(_, s)| {
                s.scope == SymbolScope::Cell || s.flags.contains(SymbolFlags::COMP_CELL)
            })
            .map(|(name, sym)| (name.clone(), sym.flags))
            .collect();
        let mut param_cells = Vec::new();
        let mut nonparam_cells = Vec::new();
        for (name, flags) in cell_symbols {
            if flags.contains(SymbolFlags::PARAMETER) {
                param_cells.push(name);
            } else {
                nonparam_cells.push(name);
            }
        }
        // param_cells are already in parameter order (from varname_cache insertion order)
        param_cells.sort_by_key(|n| varname_cache.get_index_of(n.as_str()).unwrap_or(usize::MAX));
        nonparam_cells.sort();
        for name in param_cells {
            cellvar_cache.insert(name);
        }
        for name in nonparam_cells {
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

        // Handle implicit __conditional_annotations__ cell if needed.
        if Self::scope_needs_conditional_annotations_cell(ste) {
            cellvar_cache.insert("__conditional_annotations__".to_string());
        }

        // Build freevars using dictbytype (FREE scope, offset by cellvars size)
        let mut freevar_cache = IndexSet::default();
        let annotation_free_names: IndexSet<String> = ste
            .annotation_block
            .as_ref()
            .map(|annotation| {
                annotation
                    .symbols
                    .iter()
                    .filter(|(_, s)| {
                        s.scope == SymbolScope::Free || s.flags.contains(SymbolFlags::FREE_CLASS)
                    })
                    .map(|(name, _)| name.clone())
                    .collect()
            })
            .unwrap_or_default();
        let mut free_names: Vec<_> = ste
            .symbols
            .iter()
            .filter(|(_, s)| {
                s.scope == SymbolScope::Free || s.flags.contains(SymbolFlags::FREE_CLASS)
            })
            .filter(|(name, symbol)| {
                if !matches!(
                    scope_type,
                    CompilerScope::Function | CompilerScope::AsyncFunction | CompilerScope::Lambda
                ) {
                    return true;
                }
                !(annotation_free_names.contains(*name) && symbol.flags.is_empty())
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
            CompilerScope::Annotation => (
                bytecode::CodeFlags::NEWLOCALS | bytecode::CodeFlags::OPTIMIZED,
                1, // format is positional-only
                1, // annotation scope takes one argument (format)
                0,
            ),
        };

        // Set CO_NESTED for scopes defined inside another function/class/etc.
        // (i.e., not at module level)
        let flags = if self.code_stack.len() > 1 {
            flags | bytecode::CodeFlags::NESTED
        } else {
            flags
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
            source_path,
            private,
            blocks: vec![ir::Block::default()],
            current_block: BlockIdx::new(0),
            annotations_blocks: None,
            metadata: ir::CodeUnitMetadata {
                name: name.to_owned(),
                qualname: None, // Will be set below
                consts: IndexSet::default(),
                names: IndexSet::default(),
                varnames: varname_cache,
                cellvars: cellvar_cache,
                freevars: freevar_cache,
                fast_hidden: IndexMap::default(),
                fast_hidden_final: IndexSet::default(),
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
            in_conditional_block: 0,
            next_conditional_annotation_index: 0,
        };

        // Push the old compiler unit on the stack (like PyCapsule)
        // This happens before setting qualname
        self.code_stack.push(code_info);

        // Set qualname after pushing (uses compiler_set_qualname logic)
        if scope_type != CompilerScope::Module {
            self.set_qualname();
        }

        self.emit_prefix_cell_setup();

        // Emit RESUME (handles async preamble and module lineno 0)
        // CPython: LOCATION(lineno, lineno, 0, 0), then loc.lineno = 0 for module
        self.emit_resume_for_scope(scope_type, lineno);

        Ok(())
    }

    /// Emit RESUME instruction with proper handling for async preamble and module lineno.
    /// codegen_enter_scope equivalent for RESUME emission.
    fn emit_resume_for_scope(&mut self, scope_type: CompilerScope, lineno: u32) {
        // For generators and async functions, emit RETURN_GENERATOR + POP_TOP before RESUME
        let is_gen =
            scope_type == CompilerScope::AsyncFunction || self.current_symbol_table().is_generator;
        if is_gen {
            emit!(self, Instruction::ReturnGenerator);
            emit!(self, Instruction::PopTop);
        }

        // CPython: LOCATION(lineno, lineno, 0, 0)
        // Module scope: loc.lineno = 0 (before the first line)
        let lineno_override = if scope_type == CompilerScope::Module {
            Some(0)
        } else {
            None
        };

        // Use lineno for location (col = 0 as in CPython)
        let location = SourceLocation {
            line: OneIndexed::new(lineno as usize).unwrap_or(OneIndexed::MIN),
            character_offset: OneIndexed::MIN, // col = 0
        };
        let end_location = location; // end_lineno = lineno, end_col = 0
        let except_handler = None;

        self.current_block().instructions.push(ir::InstructionInfo {
            instr: Instruction::Resume {
                context: OpArgMarker::marker(),
            }
            .into(),
            arg: OpArg::new(oparg::ResumeLocation::AtFuncStart.into()),
            target: BlockIdx::NULL,
            location,
            end_location,
            except_handler,
            folded_from_nonliteral_expr: false,
            lineno_override,
            cache_entries: 0,
            preserve_redundant_jump_as_nop: false,
            remove_no_location_nop: false,
            preserve_block_start_no_location_nop: false,
        });
    }

    fn emit_prefix_cell_setup(&mut self) {
        let metadata = &self.code_stack.last().unwrap().metadata;
        let varnames = metadata.varnames.clone();
        let cellvars = metadata.cellvars.clone();
        let freevars = metadata.freevars.clone();
        let ncells = cellvars.len();
        if ncells > 0 {
            let cellfixedoffsets = ir::build_cellfixedoffsets(&varnames, &cellvars, &freevars);
            let mut sorted = vec![None; varnames.len() + ncells];
            for (oldindex, fixed) in cellfixedoffsets.iter().copied().take(ncells).enumerate() {
                sorted[fixed as usize] = Some(oldindex);
            }
            for oldindex in sorted.into_iter().flatten() {
                let i_varnum: oparg::VarNum =
                    u32::try_from(oldindex).expect("too many cellvars").into();
                emit!(self, Instruction::MakeCell { i: i_varnum });
            }
        }

        let nfrees = freevars.len();
        if nfrees > 0 {
            emit!(
                self,
                Instruction::CopyFreeVars {
                    n: u32::try_from(nfrees).expect("too many freevars"),
                }
            );
        }
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
            // Preserve NESTED flag set by enter_scope
            info.flags = flags | (info.flags & bytecode::CodeFlags::NESTED);
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
        // - ast::TypeParams scope can have sub_tables (the function body's symbol table)
        // - Module scope can have sub_tables (for TypeAlias scopes, nested functions, classes)
        // - Function scope can have sub_tables (for nested functions, classes)
        // - Class scope can have sub_tables (for nested classes, methods)

        let pop = self.code_stack.pop();
        let stack_top = compiler_unwrap_option(self, pop);
        // No parent scope stack to maintain
        unwrap_internal(self, stack_top.finalize_code(&self.opts))
    }

    /// Exit annotation scope - similar to exit_scope but restores annotation_block to parent
    fn exit_annotation_scope(&mut self, saved_ctx: CompileContext) -> CodeObject {
        self.pop_annotation_symbol_table();
        self.ctx = saved_ctx;

        let pop = self.code_stack.pop();
        let stack_top = compiler_unwrap_option(self, pop);
        unwrap_internal(self, stack_top.finalize_code(&self.opts))
    }

    /// Enter annotation scope using the symbol table's annotation_block.
    /// Returns None if no annotation_block exists.
    /// On success, returns the saved CompileContext to pass to exit_annotation_scope.
    fn enter_annotation_scope(
        &mut self,
        _func_name: &str,
    ) -> CompileResult<Option<CompileContext>> {
        if !self.push_annotation_symbol_table() {
            return Ok(None);
        }

        // Annotation scopes are never async (even inside async functions)
        let saved_ctx = self.ctx;
        self.ctx = CompileContext {
            loop_data: None,
            in_class: saved_ctx.in_class,
            func: FunctionContext::Function,
            in_async_scope: false,
        };

        let key = self.symbol_table_stack.len() - 1;
        let lineno = self.get_source_line_number().get();
        self.enter_scope(
            "__annotate__",
            CompilerScope::Annotation,
            key,
            lineno.to_u32(),
        )?;

        // Override arg_count since enter_scope sets it to 1 but we need the varnames
        // setup to be correct too
        self.current_code_info()
            .metadata
            .varnames
            .insert("format".to_owned());

        // Emit format validation: if format > VALUE_WITH_FAKE_GLOBALS: raise NotImplementedError
        // VALUE_WITH_FAKE_GLOBALS = 2 (from annotationlib.Format)
        self.emit_format_validation()?;

        Ok(Some(saved_ctx))
    }

    /// Emit format parameter validation for annotation scope
    /// if format > VALUE_WITH_FAKE_GLOBALS (2): raise NotImplementedError
    fn emit_format_validation(&mut self) -> CompileResult<()> {
        // Load format parameter (first local variable, index 0)
        emit!(
            self,
            Instruction::LoadFast {
                var_num: oparg::VarNum::from_u32(0)
            }
        );

        // Load VALUE_WITH_FAKE_GLOBALS constant (2)
        self.emit_load_const(ConstantData::Integer { value: 2.into() });

        // Compare: format > 2
        emit!(
            self,
            Instruction::CompareOp {
                opname: ComparisonOperator::Greater
            }
        );

        // Jump to body if format <= 2 (comparison is false)
        let body_block = self.new_block();
        emit!(self, Instruction::PopJumpIfFalse { delta: body_block });

        // Raise NotImplementedError
        emit!(
            self,
            Instruction::LoadCommonConstant {
                idx: bytecode::CommonConstant::NotImplementedError
            }
        );
        emit!(
            self,
            Instruction::RaiseVarargs {
                argc: bytecode::RaiseKind::Raise
            }
        );

        // Body label - continue with annotation evaluation
        self.switch_to_block(body_block);

        Ok(())
    }

    /// Push a new fblock
    // = compiler_push_fblock
    fn push_fblock(
        &mut self,
        fb_type: FBlockType,
        fb_block: BlockIdx,
        fb_exit: BlockIdx,
    ) -> CompileResult<()> {
        self.push_fblock_full(fb_type, fb_block, fb_exit, FBlockDatum::None)
    }

    /// Push an fblock with all parameters including fb_datum
    fn push_fblock_full(
        &mut self,
        fb_type: FBlockType,
        fb_block: BlockIdx,
        fb_exit: BlockIdx,
        fb_datum: FBlockDatum,
    ) -> CompileResult<()> {
        let fb_range = self.current_source_range;
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
            fb_range,
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
                // When returning from a for-loop, CPython swaps the preserved
                // value with the iterator and uses POP_TOP for loop cleanup.
                if preserve_tos {
                    emit!(self, Instruction::Swap { i: 2 });
                }
                emit!(self, Instruction::PopTop);
            }

            FBlockType::TryExcept => {
                emit!(self, PseudoInstruction::PopBlock);
            }

            FBlockType::FinallyTry => {
                // FinallyTry is now handled specially in unwind_fblock_stack
                // to avoid infinite recursion when the finally body contains return/break/continue.
                // This branch should not be reached.
                unreachable!("FinallyTry should be handled by unwind_fblock_stack");
            }

            FBlockType::FinallyEnd => {
                // codegen_unwind_fblock(FINALLY_END)
                if preserve_tos {
                    emit!(self, Instruction::Swap { i: 2 });
                }
                emit!(self, Instruction::PopTop); // exc_value
                if preserve_tos {
                    emit!(self, Instruction::Swap { i: 2 });
                }
                emit!(self, PseudoInstruction::PopBlock);
                emit!(self, Instruction::PopExcept);
            }

            FBlockType::With | FBlockType::AsyncWith => {
                // Stack: [..., exit_func, self_exit, return_value (if preserve_tos)]
                self.set_source_range(info.fb_range);
                emit!(self, PseudoInstruction::PopBlock);

                if preserve_tos {
                    // Rotate return value below the exit pair
                    // [exit_func, self_exit, value] → [value, exit_func, self_exit]
                    emit!(self, Instruction::Swap { i: 3 }); // [value, self_exit, exit_func]
                    emit!(self, Instruction::Swap { i: 2 }); // [value, exit_func, self_exit]
                }

                // Call exit_func(self_exit, None, None, None)
                self.emit_load_const(ConstantData::None);
                self.emit_load_const(ConstantData::None);
                self.emit_load_const(ConstantData::None);
                emit!(self, Instruction::Call { argc: 3 });

                // For async with, await the result
                if matches!(info.fb_type, FBlockType::AsyncWith) {
                    emit!(self, Instruction::GetAwaitable { r#where: 2 });
                    self.emit_load_const(ConstantData::None);
                    let _ = self.compile_yield_from_sequence(true)?;
                }

                // Pop the __exit__ result
                emit!(self, Instruction::PopTop);
            }

            FBlockType::HandlerCleanup => {
                // codegen_unwind_fblock(HANDLER_CLEANUP)
                if let FBlockDatum::ExceptionName(_) = info.fb_datum {
                    // Named handler: PopBlock for inner SETUP_CLEANUP
                    emit!(self, PseudoInstruction::PopBlock);
                }
                if preserve_tos {
                    emit!(self, Instruction::Swap { i: 2 });
                }
                // PopBlock for outer SETUP_CLEANUP (ExceptionHandler)
                emit!(self, PseudoInstruction::PopBlock);
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
                    emit!(self, Instruction::Swap { i: 2 });
                }
                emit!(self, Instruction::PopTop);
            }
        }
        Ok(())
    }

    /// Unwind the fblock stack, emitting cleanup code for each block
    /// preserve_tos: if true, preserve the top of stack (e.g., return value)
    /// stop_at_loop: if true, stop when encountering a loop (for break/continue)
    fn unwind_fblock_stack(
        &mut self,
        preserve_tos: bool,
        stop_at_loop: bool,
    ) -> CompileResult<bool> {
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
        let mut unwound_finally = false;
        for info in unwind_infos {
            match info {
                UnwindInfo::Normal(fblock_info) => {
                    self.unwind_fblock(&fblock_info, preserve_tos)?;
                }
                UnwindInfo::FinallyTry { body, fblock_idx } => {
                    // codegen_unwind_fblock(FINALLY_TRY)
                    emit!(self, PseudoInstruction::PopBlock);

                    // Temporarily remove the FinallyTry fblock so nested return/break/continue
                    // in the finally body won't see it again
                    let code = self.current_code_info();
                    let saved_fblock = code.fblock.remove(fblock_idx);

                    // Push PopValue fblock if preserving tos
                    if preserve_tos {
                        self.push_fblock(
                            FBlockType::PopValue,
                            saved_fblock.fb_block,
                            saved_fblock.fb_block,
                        )?;
                    }

                    self.compile_statements(&body)?;
                    unwound_finally = true;

                    if preserve_tos {
                        self.pop_fblock(FBlockType::PopValue);
                    }

                    // Restore the fblock
                    let code = self.current_code_info();
                    code.fblock.insert(fblock_idx, saved_fblock);
                }
            }
        }

        Ok(unwound_finally)
    }

    // could take impl Into<Cow<str>>, but everything is borrowed from ast structs; we never
    // actually have a `String` to pass
    fn name(&mut self, name: &str) -> bytecode::NameIdx {
        self._name_inner(name, |i| &mut i.metadata.names)
    }

    fn varname(&mut self, name: &str) -> CompileResult<oparg::VarNum> {
        // Note: __debug__ checks are now handled in symboltable phase
        Ok(oparg::VarNum::from_u32(
            self._name_inner(name, |i| &mut i.metadata.varnames),
        ))
    }

    fn _name_inner(
        &mut self,
        name: &str,
        cache: impl FnOnce(&mut ir::CodeInfo) -> &mut IndexSet<String>,
    ) -> u32 {
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

        let parent_scope = self
            .symbol_table_stack
            .get(parent_idx)
            .map(|table| table.typ);

        // CPython skips both generic-parameter scopes and annotation scopes
        // when building qualnames for the contained function/class code object.
        if matches!(
            parent_scope,
            Some(CompilerScope::TypeParams | CompilerScope::Annotation)
        ) || parent.metadata.name.starts_with("<generic parameters of ")
        {
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
        body: &ast::ModModule,
        symbol_table: SymbolTable,
    ) -> CompileResult<()> {
        let size_before = self.code_stack.len();
        // Set future_annotations from symbol table (detected during symbol table scan)
        self.future_annotations = symbol_table.future_annotations;

        // Module-level __conditional_annotations__ cell
        let has_module_cond_ann = Self::scope_needs_conditional_annotations_cell(&symbol_table);
        if has_module_cond_ann {
            self.current_code_info()
                .metadata
                .cellvars
                .insert("__conditional_annotations__".to_string());
        }

        self.symbol_table_stack.push(symbol_table);

        // Match flowgraph.c insert_prefix_instructions() for module-level
        // synthetic cells before RESUME.
        if has_module_cond_ann {
            self.emit_prefix_cell_setup();
        }

        self.emit_resume_for_scope(CompilerScope::Module, 1);
        emit!(self, PseudoInstruction::AnnotationsPlaceholder);

        let (doc, statements) = split_doc(&body.body, &self.opts);
        if let Some(value) = doc {
            self.emit_load_const(ConstantData::Str {
                value: value.into(),
            });
            let doc = self.name("__doc__");
            emit!(self, Instruction::StoreName { namei: doc })
        }

        // Handle annotation bookkeeping in CPython order: initialize the
        // conditional annotation set first, then materialize __annotations__.
        if Self::find_ann(statements) {
            if Self::scope_needs_conditional_annotations_cell(self.current_symbol_table()) {
                emit!(self, Instruction::BuildSet { count: 0 });
                self.store_name("__conditional_annotations__")?;
            }

            if self.future_annotations {
                emit!(self, Instruction::SetupAnnotations);
            }
        }

        // Compile all statements
        self.compile_statements(statements)?;

        if Self::find_ann(statements) && !self.future_annotations {
            self.compile_module_annotation_setup_sequence(statements)?;
        }

        assert_eq!(self.code_stack.len(), size_before);

        // Match _PyCodegen_AddReturnAtEnd(): implicit scope epilogues start
        // without a source location and receive one later via CFG line
        // propagation.
        self.emit_return_const_no_location(ConstantData::None);
        Ok(())
    }

    fn compile_program_single(
        &mut self,
        body: &[ast::Stmt],
        symbol_table: SymbolTable,
    ) -> CompileResult<()> {
        self.interactive = true;
        // Set future_annotations from symbol table (detected during symbol table scan)
        self.future_annotations = symbol_table.future_annotations;
        self.symbol_table_stack.push(symbol_table);

        self.emit_resume_for_scope(CompilerScope::Module, 1);
        emit!(self, PseudoInstruction::AnnotationsPlaceholder);

        // Handle annotations based on future_annotations flag
        if Self::find_ann(body) {
            if self.future_annotations {
                // PEP 563: Initialize __annotations__ dict
                emit!(self, Instruction::SetupAnnotations);
            } else {
                // PEP 649: Initialize __conditional_annotations__ before the body.
                // CPython generates __annotate__ after the body in codegen_body().
                if self.current_symbol_table().has_conditional_annotations {
                    emit!(self, Instruction::BuildSet { count: 0 });
                    self.store_name("__conditional_annotations__")?;
                }
            }
        }

        if let Some((last, body)) = body.split_last() {
            for statement in body {
                if let ast::Stmt::Expr(ast::StmtExpr { value, .. }) = &statement {
                    self.compile_expression(value)?;
                    emit!(
                        self,
                        Instruction::CallIntrinsic1 {
                            func: bytecode::IntrinsicFunction1::Print
                        }
                    );

                    emit!(self, Instruction::PopTop);
                    self.set_no_location();
                } else {
                    self.compile_statement(statement)?;
                }
            }

            if let ast::Stmt::Expr(ast::StmtExpr { value, .. }) = &last {
                self.compile_expression(value)?;
                emit!(self, Instruction::Copy { i: 1 });
                emit!(
                    self,
                    Instruction::CallIntrinsic1 {
                        func: bytecode::IntrinsicFunction1::Print
                    }
                );

                emit!(self, Instruction::PopTop);
                self.set_no_location();
            } else {
                self.compile_statement(last)?;
                self.emit_load_const(ConstantData::None);
            }
        } else {
            self.emit_load_const(ConstantData::None);
        };

        if Self::find_ann(body) && !self.future_annotations {
            self.compile_module_annotation_setup_sequence(body)?;
        }

        self.emit_return_value();
        Ok(())
    }

    fn compile_block_expr(
        &mut self,
        body: &[ast::Stmt],
        symbol_table: SymbolTable,
    ) -> CompileResult<()> {
        self.symbol_table_stack.push(symbol_table);
        self.emit_resume_for_scope(CompilerScope::Module, 1);

        self.compile_statements(body)?;

        if let Some(last_statement) = body.last() {
            match last_statement {
                ast::Stmt::Expr(_) => {
                    self.current_block().instructions.pop(); // pop Instruction::PopTop
                }
                ast::Stmt::FunctionDef(_) | ast::Stmt::ClassDef(_) => {
                    let pop_instructions = self.current_block().instructions.pop();
                    let store_inst = compiler_unwrap_option(self, pop_instructions); // pop Instruction::Store
                    emit!(self, Instruction::Copy { i: 1 });
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
        expression: &ast::ModExpression,
        symbol_table: SymbolTable,
    ) -> CompileResult<()> {
        self.symbol_table_stack.push(symbol_table);
        self.emit_resume_for_scope(CompilerScope::Module, 1);

        self.compile_expression(&expression.body)?;
        self.emit_return_value();
        Ok(())
    }

    fn compile_statements(&mut self, statements: &[ast::Stmt]) -> CompileResult<()> {
        for statement in statements {
            self.compile_statement(statement)?
        }
        Ok(())
    }

    fn scope_needs_conditional_annotations_cell(symbol_table: &SymbolTable) -> bool {
        match symbol_table.typ {
            CompilerScope::Module => {
                symbol_table.has_conditional_annotations
                    || (symbol_table.future_annotations && symbol_table.annotation_block.is_some())
            }
            CompilerScope::Class => {
                symbol_table.has_conditional_annotations
                    || symbol_table.lookup("__conditional_annotations__").is_some()
            }
            _ => false,
        }
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
        let mangled_names = self.current_symbol_table().mangled_names.as_ref();
        symboltable::maybe_mangle_name(private, mangled_names, name)
    }

    fn module_name_declared_global_in_nested_scope(table: &SymbolTable, name: &str) -> bool {
        table.sub_tables.iter().any(|subtable| {
            (!subtable.comp_inlined
                && subtable
                    .lookup(name)
                    .is_some_and(|symbol| symbol.scope == SymbolScope::GlobalExplicit))
                || Self::module_name_declared_global_in_nested_scope(subtable, name)
        })
    }

    // = compiler_nameop
    fn compile_name(&mut self, name: &str, usage: NameUsage) -> CompileResult<()> {
        enum NameOp {
            Fast,
            Global,
            Deref,
            Name,
            DictOrGlobals, // PEP 649: can_see_class_scope
        }

        let name = self.mangle(name);

        // Special handling for __debug__
        if NameUsage::Load == usage && name == "__debug__" {
            self.emit_load_const(ConstantData::Boolean {
                value: self.opts.optimize == 0,
            });
            return Ok(());
        }

        // Determine the operation type based on symbol scope
        let is_function_like = self.ctx.in_func();

        // Look up the symbol, handling ast::TypeParams and Annotation scopes specially
        let (symbol_scope, can_see_class_scope, class_declared_global) = {
            let current_idx = self.symbol_table_stack.len() - 1;
            let current_table = &self.symbol_table_stack[current_idx];
            let is_typeparams = current_table.typ == CompilerScope::TypeParams;
            let is_annotation = current_table.typ == CompilerScope::Annotation;
            let can_see_class = current_table.can_see_class_scope;
            let parent_table = current_idx
                .checked_sub(1)
                .and_then(|idx| self.symbol_table_stack.get(idx));

            // First try to find in current table
            let symbol = current_table.lookup(name.as_ref());

            // If not found and we're in ast::TypeParams or Annotation scope, try parent scope
            let symbol = if symbol.is_none() && (is_typeparams || is_annotation) {
                self.symbol_table_stack
                    .get(self.symbol_table_stack.len() - 2) // Try to get parent index
                    .expect("Symbol has no parent! This is a compiler bug.")
                    .lookup(name.as_ref())
            } else {
                symbol
            };
            let class_declared_global = can_see_class
                && parent_table.is_some_and(|table| table.typ == CompilerScope::Class)
                && parent_table
                    .and_then(|table| table.lookup(name.as_ref()))
                    .is_some_and(|symbol| symbol.flags.contains(SymbolFlags::GLOBAL));

            (
                symbol.map(|s| s.scope),
                can_see_class,
                class_declared_global,
            )
        };

        // Special handling for class scope implicit cell variables
        // These are treated as Cell even if not explicitly marked in symbol table
        // __class__ and __classdict__: only LOAD uses Cell (stores go to class namespace)
        // __conditional_annotations__: both LOAD and STORE use Cell (it's a mutable set
        // that the annotation scope accesses through the closure)
        let symbol_scope = {
            let current_table = self.current_symbol_table();
            if current_table.typ == CompilerScope::Class
                && !self.current_code_info().in_inlined_comp
                && ((usage == NameUsage::Load
                    && (name == "__class__"
                        || name == "__classdict__"
                        || name == "__conditional_annotations__"))
                    || (name == "__conditional_annotations__" && usage == NameUsage::Store))
            {
                Some(SymbolScope::Cell)
            } else {
                symbol_scope
            }
        };

        // In annotation or type params scope, missing symbols are treated as global implicit
        // This allows referencing global names like Union, Optional, etc. that are imported
        // at module level but not explicitly bound in the function scope
        let actual_scope = match symbol_scope {
            Some(scope) => scope,
            None => {
                let current_table = self.current_symbol_table();
                if matches!(
                    current_table.typ,
                    CompilerScope::Annotation | CompilerScope::TypeParams
                ) {
                    SymbolScope::GlobalImplicit
                } else {
                    return Err(self.error(CodegenErrorType::SyntaxError(format!(
                        "the symbol '{name}' must be present in the symbol table"
                    ))));
                }
            }
        };

        let module_global_from_nested_scope = {
            let current_table = self.current_symbol_table();
            current_table.typ == CompilerScope::Module
                && Self::module_name_declared_global_in_nested_scope(current_table, name.as_ref())
        };

        // Determine operation type based on scope
        let op_type = match actual_scope {
            SymbolScope::Free => NameOp::Deref,
            SymbolScope::Cell => NameOp::Deref,
            SymbolScope::Local => {
                if module_global_from_nested_scope {
                    NameOp::Global
                } else if is_function_like
                    || self
                        .current_code_info()
                        .metadata
                        .fast_hidden
                        .get(name.as_ref())
                        .is_some_and(|&hidden| hidden)
                {
                    NameOp::Fast
                } else {
                    NameOp::Name
                }
            }
            SymbolScope::GlobalImplicit => {
                // PEP 649: In annotation scope with class visibility, use DictOrGlobals
                // to check classdict first before globals
                if class_declared_global {
                    NameOp::Global
                } else if can_see_class_scope {
                    NameOp::DictOrGlobals
                } else if is_function_like {
                    NameOp::Global
                } else {
                    NameOp::Name
                }
            }
            SymbolScope::GlobalExplicit => {
                // A global declared in the owning class body must bypass the
                // classdict, but an explicit global inherited from an outer
                // function still participates in DictOrGlobals lookup.
                if can_see_class_scope && !class_declared_global {
                    NameOp::DictOrGlobals
                } else {
                    NameOp::Global
                }
            }
            SymbolScope::Unknown => {
                if module_global_from_nested_scope {
                    NameOp::Global
                } else {
                    NameOp::Name
                }
            }
        };

        // Generate appropriate instructions based on operation type
        match op_type {
            NameOp::Deref => {
                let i = match actual_scope {
                    SymbolScope::Free => self.get_free_var_index(&name)?,
                    SymbolScope::Cell => self.get_cell_var_index(&name)?,
                    _ => unreachable!("Invalid scope for Deref operation"),
                };

                match usage {
                    NameUsage::Load => {
                        // ClassBlock (not inlined comp): LOAD_LOCALS first, then LOAD_FROM_DICT_OR_DEREF
                        if self.ctx.in_class
                            && !self.ctx.in_func()
                            && !self.current_code_info().in_inlined_comp
                        {
                            emit!(self, Instruction::LoadLocals);
                            emit!(self, Instruction::LoadFromDictOrDeref { i });
                        // can_see_class_scope: LOAD_DEREF(__classdict__) first
                        } else if can_see_class_scope {
                            let classdict_idx = self.get_free_var_index("__classdict__")?;
                            emit!(self, Instruction::LoadDeref { i: classdict_idx });
                            emit!(self, Instruction::LoadFromDictOrDeref { i });
                        } else {
                            emit!(self, Instruction::LoadDeref { i });
                        }
                    }
                    NameUsage::Store => emit!(self, Instruction::StoreDeref { i }),
                    NameUsage::Delete => emit!(self, Instruction::DeleteDeref { i }),
                };
            }
            NameOp::Fast => {
                let var_num = self.get_local_var_index(&name)?;
                match usage {
                    NameUsage::Load => emit!(self, Instruction::LoadFast { var_num }),
                    NameUsage::Store => emit!(self, Instruction::StoreFast { var_num }),
                    NameUsage::Delete => emit!(self, Instruction::DeleteFast { var_num }),
                };
            }
            NameOp::Global => {
                let namei = self.get_global_name_index(&name);
                match usage {
                    NameUsage::Load => {
                        self.emit_load_global(namei, false);
                        return Ok(());
                    }
                    NameUsage::Store => emit!(self, Instruction::StoreGlobal { namei }),
                    NameUsage::Delete => emit!(self, Instruction::DeleteGlobal { namei }),
                };
            }
            NameOp::Name => {
                let namei = self.get_global_name_index(&name);
                match usage {
                    NameUsage::Load => {
                        if self.current_symbol_table().typ == CompilerScope::Class
                            && self.current_code_info().in_inlined_comp
                        {
                            self.emit_load_global(namei, false);
                        } else {
                            emit!(self, Instruction::LoadName { namei });
                        }
                    }
                    NameUsage::Store => emit!(self, Instruction::StoreName { namei }),
                    NameUsage::Delete => emit!(self, Instruction::DeleteName { namei }),
                };
            }
            NameOp::DictOrGlobals => {
                // PEP 649: First check classdict (from __classdict__ freevar), then globals
                let idx = self.get_global_name_index(&name);
                match usage {
                    NameUsage::Load => {
                        // Load __classdict__ first (it's a free variable in annotation scope)
                        let classdict_idx = self.get_free_var_index("__classdict__")?;
                        emit!(self, Instruction::LoadDeref { i: classdict_idx });
                        emit!(self, Instruction::LoadFromDictOrGlobals { i: idx });
                    }
                    // Store/Delete in annotation scope should use Name ops
                    NameUsage::Store => {
                        emit!(self, Instruction::StoreName { namei: idx });
                    }
                    NameUsage::Delete => {
                        emit!(self, Instruction::DeleteName { namei: idx });
                    }
                }
            }
        }

        Ok(())
    }

    fn compile_statement(&mut self, statement: &ast::Stmt) -> CompileResult<()> {
        trace!("Compiling {statement:?}");
        let prev_source_range = self.current_source_range;
        self.set_source_range(statement.range());

        match &statement {
            // we do this here because `from __future__` still executes that `from` statement at runtime,
            // we still need to compile the ImportFrom down below
            ast::Stmt::ImportFrom(ast::StmtImportFrom { module, names, .. })
                if module.as_ref().map(|id| id.as_str()) == Some("__future__") =>
            {
                self.compile_future_features(names)?
            }
            // ignore module-level doc comments
            ast::Stmt::Expr(ast::StmtExpr { value, .. })
                if matches!(&**value, ast::Expr::StringLiteral(..))
                    && matches!(self.done_with_future_stmts, DoneWithFuture::No) =>
            {
                self.done_with_future_stmts = DoneWithFuture::DoneWithDoc
            }
            // if we find any other statement, stop accepting future statements
            _ => self.done_with_future_stmts = DoneWithFuture::Yes,
        }

        match &statement {
            ast::Stmt::Import(ast::StmtImport { names, .. }) => {
                // import a, b, c as d
                for name in names {
                    let name = &name;
                    self.emit_load_const(ConstantData::Integer {
                        value: num_traits::Zero::zero(),
                    });
                    self.emit_load_const(ConstantData::None);
                    let namei = self.name(&name.name);
                    emit!(self, Instruction::ImportName { namei });
                    if let Some(alias) = &name.asname {
                        let parts: Vec<&str> = name.name.split('.').skip(1).collect();
                        for (i, part) in parts.iter().enumerate() {
                            let namei = self.name(part);
                            emit!(self, Instruction::ImportFrom { namei });
                            if i < parts.len() - 1 {
                                emit!(self, Instruction::Swap { i: 2 });
                                emit!(self, Instruction::PopTop);
                            }
                        }
                        self.store_name(alias.as_str())?;
                        if !parts.is_empty() {
                            emit!(self, Instruction::PopTop);
                        }
                    } else {
                        self.store_name(name.name.split('.').next().unwrap())?
                    }
                }
            }
            ast::Stmt::ImportFrom(ast::StmtImportFrom {
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
                emit!(self, Instruction::ImportName { namei: module_idx });

                if import_star {
                    // from .... import *
                    emit!(
                        self,
                        Instruction::CallIntrinsic1 {
                            func: bytecode::IntrinsicFunction1::ImportStar
                        }
                    );
                    emit!(self, Instruction::PopTop);
                } else {
                    // from mod import a, b as c

                    for name in names {
                        let name = &name;
                        let idx = self.name(name.name.as_str());
                        // import symbol from module:
                        emit!(self, Instruction::ImportFrom { namei: idx });

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
            ast::Stmt::Expr(ast::StmtExpr { value, .. }) => {
                // Optimize away constant expressions with no side effects.
                // In interactive mode, always compile (to print the result).
                let dominated_by_interactive =
                    self.interactive && !self.ctx.in_func() && !self.ctx.in_class;
                if !dominated_by_interactive && Self::is_const_expression(value) {
                    emit!(self, Instruction::Nop);
                } else {
                    self.compile_expression(value)?;

                    if dominated_by_interactive {
                        emit!(
                            self,
                            Instruction::CallIntrinsic1 {
                                func: bytecode::IntrinsicFunction1::Print
                            }
                        );
                    }

                    emit!(self, Instruction::PopTop);
                    self.set_no_location();
                }
            }
            ast::Stmt::Global(_) | ast::Stmt::Nonlocal(_) => {
                // Handled during symbol table construction.
            }
            ast::Stmt::If(ast::StmtIf {
                test,
                body,
                elif_else_clauses,
                ..
            }) => {
                self.enter_conditional_block();
                self.compile_if(test, body, elif_else_clauses, test.range())?;
                self.leave_conditional_block();
                self.set_source_range(statement.range());
            }
            ast::Stmt::While(ast::StmtWhile {
                test, body, orelse, ..
            }) => self.compile_while(test, body, orelse)?,
            ast::Stmt::With(ast::StmtWith {
                items,
                body,
                is_async,
                ..
            }) => self.compile_with(items, body, *is_async)?,
            ast::Stmt::For(ast::StmtFor {
                target,
                iter,
                body,
                orelse,
                is_async,
                ..
            }) => self.compile_for(target, iter, body, orelse, *is_async)?,
            ast::Stmt::Match(ast::StmtMatch { subject, cases, .. }) => {
                self.compile_match(subject, cases)?
            }
            ast::Stmt::Raise(ast::StmtRaise {
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
                emit!(self, Instruction::RaiseVarargs { argc: kind });
                // Start a new block so dead code after raise doesn't
                // corrupt the except stack in label_exception_targets
                let dead = self.new_block();
                self.switch_to_block(dead);
            }
            ast::Stmt::Try(ast::StmtTry {
                body,
                handlers,
                orelse,
                finalbody,
                is_star,
                ..
            }) => {
                self.enter_conditional_block();
                if *is_star {
                    self.compile_try_star_except(body, handlers, orelse, finalbody)?
                } else {
                    self.compile_try_statement(body, handlers, orelse, finalbody)?
                }
                self.leave_conditional_block();
            }
            ast::Stmt::FunctionDef(ast::StmtFunctionDef {
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
            ast::Stmt::ClassDef(ast::StmtClassDef {
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
            ast::Stmt::Assert(ast::StmtAssert { test, msg, .. }) => {
                // if some flag, ignore all assert statements!
                if self.opts.optimize == 0 {
                    let after_block = self.new_block();
                    self.compile_jump_if(test, true, after_block)?;
                    emit!(
                        self,
                        Instruction::LoadCommonConstant {
                            idx: bytecode::CommonConstant::AssertionError
                        }
                    );
                    if let Some(e) = msg {
                        self.compile_expression(e)?;
                        emit!(self, Instruction::Call { argc: 0 });
                    }
                    emit!(
                        self,
                        Instruction::RaiseVarargs {
                            argc: bytecode::RaiseKind::Raise,
                        }
                    );
                    self.switch_to_block(after_block);
                } else {
                    // Optimized-out asserts still need to consume any nested
                    // scope symbol tables they contain so later nested scopes
                    // stay aligned with AST traversal order.
                    self.consume_skipped_nested_scopes_in_expr(test)?;
                    if let Some(expr) = msg {
                        self.consume_skipped_nested_scopes_in_expr(expr)?;
                    }
                }
            }
            ast::Stmt::Break(_) => {
                emit!(self, Instruction::Nop); // NOP for line tracing
                // Unwind fblock stack until we find a loop, emitting cleanup for each fblock
                self.compile_break_continue(statement.range(), true)?;
                let dead = self.new_block();
                self.switch_to_block(dead);
            }
            ast::Stmt::Continue(_) => {
                emit!(self, Instruction::Nop); // NOP for line tracing
                // Unwind fblock stack until we find a loop, emitting cleanup for each fblock
                self.compile_break_continue(statement.range(), false)?;
                let dead = self.new_block();
                self.switch_to_block(dead);
            }
            ast::Stmt::Return(ast::StmtReturn { value, .. }) => {
                if !self.ctx.in_func() {
                    return Err(
                        self.error_ranged(CodegenErrorType::InvalidReturn, statement.range())
                    );
                }

                let prev_source_range = self.current_source_range;
                let stmt_range = statement.range();
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
                        let folded_constant = if v.is_constant() {
                            self.try_fold_constant_expr(v)?
                        } else {
                            None
                        };
                        let preserve_tos = folded_constant.is_none();
                        if preserve_tos {
                            self.compile_expression(v)?;
                        } else {
                            self.set_source_range(v.range());
                            emit!(self, Instruction::Nop);
                        }

                        let source = self.source_file.to_source_code();
                        if source.line_index(v.range().start())
                            != source.line_index(stmt_range.start())
                        {
                            self.set_source_range(stmt_range);
                            emit!(self, Instruction::Nop);
                        }
                        self.set_source_range(stmt_range);
                        let unwound_finally = self.unwind_fblock_stack(preserve_tos, false)?;
                        if !unwound_finally {
                            self.set_source_range(stmt_range);
                        }
                        match folded_constant {
                            Some(constant) if unwound_finally => {
                                self.emit_return_const_no_location(constant);
                            }
                            Some(constant) => {
                                self.emit_load_const(constant);
                                self.emit_return_value();
                            }
                            None => {
                                self.emit_return_value();
                                if unwound_finally {
                                    self.set_no_location();
                                }
                            }
                        }
                    }
                    None => {
                        self.set_source_range(stmt_range);
                        emit!(self, Instruction::Nop);
                        // Unwind fblock stack with preserve_tos=false (no value to preserve)
                        let unwound_finally = self.unwind_fblock_stack(false, false)?;
                        if unwound_finally {
                            self.emit_return_const_no_location(ConstantData::None);
                        } else {
                            self.set_source_range(stmt_range);
                            self.emit_return_const(ConstantData::None);
                        }
                    }
                }
                self.set_source_range(prev_source_range);
                let dead = self.new_block();
                self.switch_to_block(dead);
            }
            ast::Stmt::Assign(ast::StmtAssign { targets, value, .. }) => {
                if targets.len() == 1 && Self::is_unpack_assignment_target(&targets[0]) {
                    self.compile_expression_without_const_collection_folding(value)?;
                } else {
                    self.compile_expression(value)?;
                }

                for (i, target) in targets.iter().enumerate() {
                    if i + 1 != targets.len() {
                        emit!(self, Instruction::Copy { i: 1 });
                    }
                    self.compile_store(target)?;
                }
            }
            ast::Stmt::AugAssign(ast::StmtAugAssign {
                target, op, value, ..
            }) => self.compile_augassign(target, op, value)?,
            ast::Stmt::AnnAssign(ast::StmtAnnAssign {
                target,
                annotation,
                value,
                simple,
                ..
            }) => {
                self.compile_annotated_assign(target, annotation, value.as_deref(), *simple)?;
                // Bare annotations in function scope emit no code; restore
                // source range so subsequent instructions keep the correct line.
                if value.is_none() && self.ctx.in_func() {
                    self.set_source_range(prev_source_range);
                }
            }
            ast::Stmt::Delete(ast::StmtDelete { targets, .. }) => {
                for target in targets {
                    self.compile_delete(target)?;
                }
            }
            ast::Stmt::Pass(_) => {
                emit!(self, Instruction::Nop); // NOP for line tracing
            }
            ast::Stmt::TypeAlias(ast::StmtTypeAlias {
                name,
                type_params,
                value,
                ..
            }) => {
                let Some(name) = name.as_name_expr() else {
                    return Err(self.error(CodegenErrorType::SyntaxError(
                        "type alias expect name".to_owned(),
                    )));
                };
                let name_string = name.id.to_string();

                if let Some(type_params) = type_params {
                    self.push_symbol_table()?;
                    let key = self.symbol_table_stack.len() - 1;
                    let lineno = self.get_source_line_number().get().to_u32();
                    let scope_name = format!("<generic parameters of {name_string}>");
                    self.enter_scope(&scope_name, CompilerScope::TypeParams, key, lineno)?;

                    // TypeParams scope is function-like
                    let prev_ctx = self.ctx;
                    self.ctx = CompileContext {
                        loop_data: None,
                        in_class: prev_ctx.in_class,
                        func: FunctionContext::Function,
                        in_async_scope: false,
                    };

                    self.emit_load_const(ConstantData::Str {
                        value: name_string.clone().into(),
                    });
                    self.compile_type_params(type_params)?;
                    self.compile_typealias_value_closure(&name_string, value)?;
                    emit!(self, Instruction::BuildTuple { count: 3 });
                    emit!(
                        self,
                        Instruction::CallIntrinsic1 {
                            func: bytecode::IntrinsicFunction1::TypeAlias
                        }
                    );
                    emit!(self, Instruction::ReturnValue);

                    let code = self.exit_scope();
                    self.ctx = prev_ctx;
                    self.make_closure(code, bytecode::MakeFunctionFlags::new())?;
                    emit!(self, Instruction::PushNull);
                    emit!(self, Instruction::Call { argc: 0 });
                } else {
                    self.emit_load_const(ConstantData::Str {
                        value: name_string.clone().into(),
                    });
                    self.emit_load_const(ConstantData::None);
                    self.compile_typealias_value_closure(&name_string, value)?;
                    emit!(self, Instruction::BuildTuple { count: 3 });
                    emit!(
                        self,
                        Instruction::CallIntrinsic1 {
                            func: bytecode::IntrinsicFunction1::TypeAlias
                        }
                    );
                }

                self.store_name(&name_string)?;
            }
            ast::Stmt::IpyEscapeCommand(_) => todo!(),
        }
        Ok(())
    }

    fn compile_delete(&mut self, expression: &ast::Expr) -> CompileResult<()> {
        match &expression {
            ast::Expr::Name(ast::ExprName { id, .. }) => {
                self.compile_name(id.as_str(), NameUsage::Delete)?
            }
            ast::Expr::Attribute(ast::ExprAttribute { value, attr, .. }) => {
                self.compile_expression(value)?;
                let namei = self.name(attr.as_str());
                emit!(self, Instruction::DeleteAttr { namei });
            }
            ast::Expr::Subscript(ast::ExprSubscript {
                value, slice, ctx, ..
            }) => {
                self.compile_subscript(value, slice, *ctx)?;
            }
            ast::Expr::Tuple(ast::ExprTuple { elts, .. })
            | ast::Expr::List(ast::ExprList { elts, .. }) => {
                for element in elts {
                    self.compile_delete(element)?;
                }
            }
            ast::Expr::BinOp(_) | ast::Expr::UnaryOp(_) => {
                return Err(self.error(CodegenErrorType::Delete("expression")));
            }
            _ => return Err(self.error(CodegenErrorType::Delete(expression.python_name()))),
        }
        Ok(())
    }

    fn enter_function(&mut self, name: &str, parameters: &ast::Parameters) -> CompileResult<()> {
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
    /// For @dec1 @dec2 def foo(): stack becomes [dec1, dec2]
    fn prepare_decorators(&mut self, decorator_list: &[ast::Decorator]) -> CompileResult<()> {
        for decorator in decorator_list {
            self.compile_expression(&decorator.expression)?;
        }
        Ok(())
    }

    /// Apply decorators: each decorator calls the function below it.
    /// Stack: [dec1, dec2, func] → CALL 0 → [dec1, dec2(func)] → CALL 0 → [dec1(dec2(func))]
    fn apply_decorators(&mut self, decorator_list: &[ast::Decorator]) {
        for _ in decorator_list {
            emit!(self, Instruction::Call { argc: 0 });
        }
    }

    /// Compile type parameter bound or default in a separate scope and return closure
    fn compile_type_param_bound_or_default(
        &mut self,
        expr: &ast::Expr,
        name: &str,
        allow_starred: bool,
    ) -> CompileResult<()> {
        self.emit_load_const(ConstantData::Tuple {
            elements: vec![ConstantData::Integer { value: 1.into() }],
        });

        // Push the next symbol table onto the stack
        self.push_symbol_table()?;

        // Get the current symbol table
        let key = self.symbol_table_stack.len() - 1;
        let lineno = self.get_source_line_number().get().to_u32();

        // Enter scope with the type parameter name
        self.enter_scope(name, CompilerScope::Annotation, key, lineno)?;

        self.current_code_info()
            .metadata
            .varnames
            .insert(".format".to_owned());

        self.emit_format_validation()?;

        // TypeParams scope is function-like
        let prev_ctx = self.ctx;
        self.ctx = CompileContext {
            loop_data: None,
            in_class: prev_ctx.in_class,
            func: FunctionContext::Function,
            in_async_scope: false,
        };

        // Compile the expression
        if allow_starred && matches!(expr, ast::Expr::Starred(_)) {
            if let ast::Expr::Starred(starred) = expr {
                self.compile_expression(&starred.value)?;
                emit!(self, Instruction::UnpackSequence { count: 1 });
            }
        } else {
            self.compile_expression(expr)?;
        }

        // Return value
        emit!(self, Instruction::ReturnValue);

        // Exit scope and create closure
        let code = self.exit_scope();
        self.ctx = prev_ctx;

        self.make_closure(
            code,
            bytecode::MakeFunctionFlags::from([bytecode::MakeFunctionFlag::Defaults]),
        )?;

        Ok(())
    }

    fn compile_typealias_value_closure(
        &mut self,
        alias_name: &str,
        value: &ast::Expr,
    ) -> CompileResult<()> {
        self.emit_load_const(ConstantData::Tuple {
            elements: vec![ConstantData::Integer { value: 1.into() }],
        });

        self.push_symbol_table()?;
        let key = self.symbol_table_stack.len() - 1;
        let lineno = self.get_source_line_number().get().to_u32();
        self.enter_scope(alias_name, CompilerScope::Annotation, key, lineno)?;
        self.current_code_info()
            .metadata
            .varnames
            .insert(".format".to_owned());
        self.emit_format_validation()?;

        let prev_ctx = self.ctx;
        self.ctx = CompileContext {
            loop_data: None,
            in_class: prev_ctx.in_class,
            func: FunctionContext::Function,
            in_async_scope: false,
        };

        self.compile_expression(value)?;
        emit!(self, Instruction::ReturnValue);

        let code = self.exit_scope();
        self.ctx = prev_ctx;
        self.make_closure(
            code,
            bytecode::MakeFunctionFlags::from([bytecode::MakeFunctionFlag::Defaults]),
        )?;

        Ok(())
    }

    /// Store each type parameter so it is accessible to the current scope, and leave a tuple of
    /// all the type parameters on the stack. Handles default values per PEP 695.
    fn compile_type_params(&mut self, type_params: &ast::TypeParams) -> CompileResult<()> {
        // First, compile each type parameter and store it
        for type_param in &type_params.type_params {
            match type_param {
                ast::TypeParam::TypeVar(ast::TypeParamTypeVar {
                    name,
                    bound,
                    default,
                    ..
                }) => {
                    self.emit_load_const(ConstantData::Str {
                        value: name.as_str().into(),
                    });

                    if let Some(expr) = &bound {
                        self.compile_type_param_bound_or_default(expr, name.as_str(), false)?;

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
                        self.compile_type_param_bound_or_default(
                            default_expr,
                            name.as_str(),
                            false,
                        )?;
                        emit!(
                            self,
                            Instruction::CallIntrinsic2 {
                                func: bytecode::IntrinsicFunction2::SetTypeparamDefault
                            }
                        );
                    }

                    emit!(self, Instruction::Copy { i: 1 });
                    self.store_name(name.as_ref())?;
                }
                ast::TypeParam::ParamSpec(ast::TypeParamParamSpec { name, default, .. }) => {
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
                        self.compile_type_param_bound_or_default(
                            default_expr,
                            name.as_str(),
                            false,
                        )?;
                        emit!(
                            self,
                            Instruction::CallIntrinsic2 {
                                func: bytecode::IntrinsicFunction2::SetTypeparamDefault
                            }
                        );
                    }

                    emit!(self, Instruction::Copy { i: 1 });
                    self.store_name(name.as_ref())?;
                }
                ast::TypeParam::TypeVarTuple(ast::TypeParamTypeVarTuple {
                    name, default, ..
                }) => {
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
                        self.compile_type_param_bound_or_default(
                            default_expr,
                            name.as_str(),
                            true,
                        )?;
                        emit!(
                            self,
                            Instruction::CallIntrinsic2 {
                                func: bytecode::IntrinsicFunction2::SetTypeparamDefault
                            }
                        );
                    }

                    emit!(self, Instruction::Copy { i: 1 });
                    self.store_name(name.as_ref())?;
                }
            };
        }
        emit!(
            self,
            Instruction::BuildTuple {
                count: u32::try_from(type_params.len()).unwrap(),
            }
        );
        Ok(())
    }

    fn compile_try_statement(
        &mut self,
        body: &[ast::Stmt],
        handlers: &[ast::ExceptHandler],
        orelse: &[ast::Stmt],
        finalbody: &[ast::Stmt],
    ) -> CompileResult<()> {
        if finalbody.is_empty() {
            return self.compile_try_except_no_finally(body, handlers, orelse);
        }

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
        let has_bare_except = handlers.iter().any(|handler| {
            matches!(
                handler,
                ast::ExceptHandler::ExceptHandler(ast::ExceptHandlerExceptHandler {
                    type_: None,
                    ..
                })
            )
        });
        if has_bare_except {
            self.disable_load_fast_borrow_for_block(end_block);
        }

        // Emit NOP at the try: line so LINE events fire for it
        emit!(self, Instruction::Nop);

        // Setup a finally block if we have a finally statement.
        // Push fblock with handler info for exception table generation
        // IMPORTANT: handler goes to finally_except_block (exception path), not finally_block
        if !finalbody.is_empty() {
            // SETUP_FINALLY doesn't push lasti for try body handler
            // Exception table: L1 to L2 -> L4 [1] (no lasti)
            let setup_target = finally_except_block.unwrap_or(finally_block);
            emit!(
                self,
                PseudoInstruction::SetupFinally {
                    delta: setup_target
                }
            );
            // Store finally body in fb_datum for unwind_fblock to compile inline
            self.push_fblock_full(
                FBlockType::FinallyTry,
                finally_block,
                finally_block,
                FBlockDatum::FinallyBody(finalbody.to_vec()), // Clone finally body for unwind
            )?;
        }

        // if handlers is empty, compile body directly
        // without wrapping in TryExcept (only FinallyTry is needed)
        if handlers.is_empty() {
            let preserve_finally_entry_nop = Self::preserves_finally_entry_nop(body);

            // Just compile body with FinallyTry fblock active (if finalbody exists)
            self.compile_statements(body)?;

            // Pop FinallyTry fblock BEFORE compiling orelse/finally (normal path)
            // This prevents exception table from covering the normal path
            if !finalbody.is_empty() {
                emit!(self, PseudoInstruction::PopBlock);
                if preserve_finally_entry_nop {
                    self.preserve_last_redundant_nop();
                } else {
                    self.set_no_location();
                    self.remove_last_no_location_nop();
                }
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
            emit!(
                self,
                PseudoInstruction::JumpNoInterrupt { delta: end_block }
            );
            self.set_no_location();
            self.preserve_last_redundant_jump_as_nop();

            if let Some(finally_except) = finally_except_block {
                // Restore sub_tables for exception path compilation
                if let Some(cursor) = sub_table_cursor
                    && let Some(current_table) = self.symbol_table_stack.last_mut()
                {
                    current_table.next_sub_table = cursor;
                }

                self.switch_to_block(finally_except);
                // SETUP_CLEANUP before PUSH_EXC_INFO
                if let Some(cleanup) = finally_cleanup_block {
                    emit!(self, PseudoInstruction::SetupCleanup { delta: cleanup });
                }
                emit!(self, Instruction::PushExcInfo);
                if let Some(cleanup) = finally_cleanup_block {
                    self.push_fblock(FBlockType::FinallyEnd, cleanup, cleanup)?;
                }
                self.compile_statements(finalbody)?;

                // RERAISE must be inside the cleanup handler's exception table
                // range. When RERAISE re-raises the exception, the cleanup
                // handler (COPY 3, POP_EXCEPT, RERAISE 1) runs POP_EXCEPT to
                // restore exc_info before the exception reaches the outer handler.
                emit!(self, Instruction::Reraise { depth: 0 });
                self.set_no_location();

                // PopBlock after RERAISE (dead code, but marks the exception
                // table range end so the cleanup covers RERAISE).
                if finally_cleanup_block.is_some() {
                    emit!(self, PseudoInstruction::PopBlock);
                    self.pop_fblock(FBlockType::FinallyEnd);
                }
            }

            if let Some(cleanup) = finally_cleanup_block {
                self.switch_to_block(cleanup);
                emit!(self, Instruction::Copy { i: 3 });
                emit!(self, Instruction::PopExcept);
                emit!(self, Instruction::Reraise { depth: 1 });
            }

            self.switch_to_block(end_block);
            return Ok(());
        }

        // try:
        emit!(
            self,
            PseudoInstruction::SetupFinally {
                delta: handler_block
            }
        );
        self.push_fblock(FBlockType::TryExcept, handler_block, handler_block)?;
        self.compile_statements(body)?;
        emit!(self, PseudoInstruction::PopBlock);
        self.set_no_location();
        self.pop_fblock(FBlockType::TryExcept);

        let cleanup_block = self.new_block();
        // We successfully ran the try block:
        // else:
        self.compile_statements(orelse)?;

        emit!(
            self,
            PseudoInstruction::JumpNoInterrupt {
                delta: finally_block,
            }
        );
        self.set_no_location();

        // except handlers:
        self.switch_to_block(handler_block);

        // SETUP_CLEANUP(cleanup) for except block
        // This handles exceptions during exception matching
        // Exception table: L2 to L3 -> L5 [1] lasti
        // After PUSH_EXC_INFO, stack is [prev_exc, exc]
        // depth=1 means keep prev_exc on stack when routing to cleanup
        emit!(
            self,
            PseudoInstruction::SetupCleanup {
                delta: cleanup_block
            }
        );
        self.set_no_location();
        self.push_fblock(FBlockType::ExceptionHandler, cleanup_block, cleanup_block)?;

        // Exception is on top of stack now, pushed by unwind_blocks
        // PUSH_EXC_INFO transforms [exc] -> [prev_exc, exc] for PopExcept
        emit!(self, Instruction::PushExcInfo);
        self.set_no_location();
        for handler in handlers {
            let ast::ExceptHandler::ExceptHandler(ast::ExceptHandlerExceptHandler {
                type_,
                name,
                body,
                range: handler_range,
                ..
            }) = &handler;
            self.set_source_range(*handler_range);
            let next_handler = self.new_block();

            if let Some(exc_type) = type_ {
                self.compile_expression(exc_type)?;
                emit!(self, Instruction::CheckExcMatch);
                emit!(
                    self,
                    Instruction::PopJumpIfFalse {
                        delta: next_handler
                    }
                );

                if let Some(alias) = name {
                    self.store_name(alias.as_str())?
                } else {
                    emit!(self, Instruction::PopTop);
                }
            } else {
                emit!(self, Instruction::PopTop);
            }

            let handler_cleanup_block = if name.is_some() {
                let cleanup_end = self.new_block();
                emit!(self, PseudoInstruction::SetupCleanup { delta: cleanup_end });
                self.push_fblock_full(
                    FBlockType::HandlerCleanup,
                    cleanup_end,
                    cleanup_end,
                    FBlockDatum::ExceptionName(name.as_ref().unwrap().as_str().to_owned()),
                )?;
                Some(cleanup_end)
            } else {
                self.push_fblock(FBlockType::HandlerCleanup, finally_block, finally_block)?;
                None
            };

            self.compile_statements(body)?;

            self.pop_fblock(FBlockType::HandlerCleanup);
            if handler_cleanup_block.is_some() {
                emit!(self, PseudoInstruction::PopBlock);
            }

            if let Some(cleanup_end) = handler_cleanup_block {
                let handler_normal_exit = self.new_block();
                emit!(
                    self,
                    PseudoInstruction::JumpNoInterrupt {
                        delta: handler_normal_exit,
                    }
                );

                self.switch_to_block(cleanup_end);
                if let Some(alias) = name {
                    self.emit_load_const(ConstantData::None);
                    self.store_name(alias.as_str())?;
                    self.compile_name(alias.as_str(), NameUsage::Delete)?;
                }
                emit!(self, Instruction::Reraise { depth: 1 });
                self.switch_to_block(handler_normal_exit);
            }

            emit!(self, PseudoInstruction::PopBlock);
            self.pop_fblock(FBlockType::ExceptionHandler);
            emit!(self, Instruction::PopExcept);

            if let Some(alias) = name {
                self.emit_load_const(ConstantData::None);
                self.store_name(alias.as_str())?;
                self.compile_name(alias.as_str(), NameUsage::Delete)?;
            }

            emit!(
                self,
                PseudoInstruction::JumpNoInterrupt {
                    delta: finally_block,
                }
            );
            self.set_no_location();

            self.push_fblock(FBlockType::ExceptionHandler, cleanup_block, cleanup_block)?;
            self.switch_to_block(next_handler);
        }

        emit!(self, Instruction::Reraise { depth: 0 });
        self.set_no_location();
        self.pop_fblock(FBlockType::ExceptionHandler);

        self.switch_to_block(cleanup_block);
        emit!(self, Instruction::Copy { i: 3 });
        self.set_no_location();
        emit!(self, Instruction::PopExcept);
        self.set_no_location();
        emit!(self, Instruction::Reraise { depth: 1 });
        self.set_no_location();

        // finally (normal path):
        // CPython's codegen_try_finally emits the wrapped try/except first and
        // places the outer finally body at the inner try/except end label.  Keep
        // the FinallyTry fblock active through exception-handler normal exits so
        // the CFG and exception-table ranges match that structure.
        self.switch_to_block(finally_block);
        if !finalbody.is_empty() {
            let preserve_finally_normal_pop_block_nop = orelse.is_empty()
                && !Self::statements_end_with_scope_exit(body)
                && handlers.iter().all(|handler| match handler {
                    ast::ExceptHandler::ExceptHandler(handler) => {
                        Self::statements_end_with_scope_exit(&handler.body)
                    }
                });
            if preserve_finally_normal_pop_block_nop && let Some(last_body_stmt) = body.last() {
                self.set_source_range(last_body_stmt.range());
            }
            emit!(self, PseudoInstruction::PopBlock);
            if preserve_finally_normal_pop_block_nop {
                self.preserve_last_redundant_nop();
            } else {
                self.set_no_location();
            }
            self.pop_fblock(FBlockType::FinallyTry);

            // Snapshot sub_tables before first finally compilation (for double compilation issue)
            let sub_table_cursor = if finally_except_block.is_some() {
                self.symbol_table_stack.last().map(|t| t.next_sub_table)
            } else {
                None
            };

            self.compile_statements(finalbody)?;
            // Jump to end_block to skip exception path blocks
            // This prevents fall-through to finally_except_block
            emit!(
                self,
                PseudoInstruction::JumpNoInterrupt { delta: end_block }
            );
            self.set_no_location();
            self.preserve_last_redundant_jump_as_nop();

            // finally (exception path)
            // This is where exceptions go to run finally before reraise
            // Stack at entry: [lasti, exc] (from exception table with preserve_lasti=true)
            let finally_except = finally_except_block.expect("finally except block");
            // Restore sub_tables for exception path compilation
            if let Some(cursor) = sub_table_cursor
                && let Some(current_table) = self.symbol_table_stack.last_mut()
            {
                current_table.next_sub_table = cursor;
            }

            self.switch_to_block(finally_except);

            // SETUP_CLEANUP for finally body
            // Exceptions during finally body need to go to cleanup block
            if let Some(cleanup) = finally_cleanup_block {
                emit!(self, PseudoInstruction::SetupCleanup { delta: cleanup });
                self.set_no_location();
            }
            emit!(self, Instruction::PushExcInfo);
            self.set_no_location();
            if let Some(cleanup) = finally_cleanup_block {
                self.push_fblock(FBlockType::FinallyEnd, cleanup, cleanup)?;
            }

            // Run finally body
            self.compile_statements(finalbody)?;

            // RERAISE must be inside the cleanup handler's exception table
            // range. The cleanup handler (COPY 3, POP_EXCEPT, RERAISE 1)
            // runs POP_EXCEPT to restore exc_info before re-raising to
            // the outer handler.
            emit!(self, Instruction::Reraise { depth: 0 });
            self.set_no_location();

            // PopBlock after RERAISE (dead code, but marks the exception
            // table range end so the cleanup covers RERAISE).
            if finally_cleanup_block.is_some() {
                emit!(self, PseudoInstruction::PopBlock);
                self.pop_fblock(FBlockType::FinallyEnd);
            }
        }

        // finally cleanup block
        // This handles exceptions that occur during the finally body itself
        // Stack at entry: [lasti, prev_exc, lasti2, exc2] after exception table routing
        if let Some(cleanup) = finally_cleanup_block {
            self.switch_to_block(cleanup);
            // COPY 3: copy the exception from position 3
            emit!(self, Instruction::Copy { i: 3 });
            // POP_EXCEPT: restore prev_exc as current exception
            emit!(self, Instruction::PopExcept);
            // RERAISE 1: reraise with lasti from stack
            emit!(self, Instruction::Reraise { depth: 1 });
        }

        // End block - continuation point after try-finally
        // Normal execution continues here after the finally block
        self.switch_to_block(end_block);

        Ok(())
    }

    fn compile_try_except_no_finally(
        &mut self,
        body: &[ast::Stmt],
        handlers: &[ast::ExceptHandler],
        orelse: &[ast::Stmt],
    ) -> CompileResult<()> {
        let normal_exit_range = orelse
            .last()
            .map(ast::Stmt::range)
            .or_else(|| body.last().map(ast::Stmt::range));
        let handler_block = self.new_block();
        let cleanup_block = self.new_block();
        let end_block = self.new_block();
        let has_bare_except = handlers.iter().any(|handler| {
            matches!(
                handler,
                ast::ExceptHandler::ExceptHandler(ast::ExceptHandlerExceptHandler {
                    type_: None,
                    ..
                })
            )
        });
        if has_bare_except {
            self.disable_load_fast_borrow_for_block(end_block);
        }

        emit!(
            self,
            PseudoInstruction::SetupFinally {
                delta: handler_block
            }
        );

        self.push_fblock(FBlockType::TryExcept, handler_block, handler_block)?;
        self.compile_statements(body)?;
        self.pop_fblock(FBlockType::TryExcept);
        emit!(self, PseudoInstruction::PopBlock);
        self.set_no_location();
        self.remove_last_no_location_nop();
        self.compile_statements(orelse)?;
        emit!(
            self,
            PseudoInstruction::JumpNoInterrupt { delta: end_block }
        );
        self.set_no_location();
        self.remove_last_no_location_nop();

        self.switch_to_block(handler_block);
        emit!(
            self,
            PseudoInstruction::SetupCleanup {
                delta: cleanup_block
            }
        );
        self.set_no_location();
        emit!(self, Instruction::PushExcInfo);
        self.set_no_location();
        self.push_fblock(FBlockType::ExceptionHandler, cleanup_block, cleanup_block)?;

        for handler in handlers {
            let ast::ExceptHandler::ExceptHandler(ast::ExceptHandlerExceptHandler {
                type_,
                name,
                body,
                range: handler_range,
                ..
            }) = handler;
            self.set_source_range(*handler_range);
            let next_handler = self.new_block();

            if let Some(exc_type) = type_ {
                self.compile_expression(exc_type)?;
                emit!(self, Instruction::CheckExcMatch);
                emit!(
                    self,
                    Instruction::PopJumpIfFalse {
                        delta: next_handler
                    }
                );
            }

            if let Some(alias) = name {
                self.store_name(alias.as_str())?;

                let cleanup_end = self.new_block();
                emit!(self, PseudoInstruction::SetupCleanup { delta: cleanup_end });
                self.push_fblock_full(
                    FBlockType::HandlerCleanup,
                    cleanup_end,
                    cleanup_end,
                    FBlockDatum::ExceptionName(alias.as_str().to_owned()),
                )?;

                self.compile_statements(body)?;

                self.pop_fblock(FBlockType::HandlerCleanup);
                emit!(self, PseudoInstruction::PopBlock);
                self.set_no_location();
                emit!(self, PseudoInstruction::PopBlock);
                self.set_no_location();
                self.pop_fblock(FBlockType::ExceptionHandler);
                emit!(self, Instruction::PopExcept);
                self.set_no_location();

                self.emit_load_const(ConstantData::None);
                self.set_no_location();
                self.store_name(alias.as_str())?;
                self.set_no_location();
                self.compile_name(alias.as_str(), NameUsage::Delete)?;
                self.set_no_location();

                emit!(
                    self,
                    PseudoInstruction::JumpNoInterrupt { delta: end_block }
                );
                self.set_no_location();

                self.switch_to_block(cleanup_end);
                self.emit_load_const(ConstantData::None);
                self.set_no_location();
                self.store_name(alias.as_str())?;
                self.set_no_location();
                self.compile_name(alias.as_str(), NameUsage::Delete)?;
                self.set_no_location();
                emit!(self, Instruction::Reraise { depth: 1 });
                self.set_no_location();
            } else {
                emit!(self, Instruction::PopTop);
                self.push_fblock(FBlockType::HandlerCleanup, end_block, end_block)?;

                self.compile_statements(body)?;

                self.pop_fblock(FBlockType::HandlerCleanup);
                emit!(self, PseudoInstruction::PopBlock);
                self.set_no_location();
                self.pop_fblock(FBlockType::ExceptionHandler);
                emit!(self, Instruction::PopExcept);
                self.set_no_location();
                emit!(
                    self,
                    PseudoInstruction::JumpNoInterrupt { delta: end_block }
                );
                self.set_no_location();
            }

            self.push_fblock(FBlockType::ExceptionHandler, cleanup_block, cleanup_block)?;
            self.switch_to_block(next_handler);
        }

        emit!(self, Instruction::Reraise { depth: 0 });
        self.set_no_location();
        self.pop_fblock(FBlockType::ExceptionHandler);

        self.switch_to_block(cleanup_block);
        emit!(self, Instruction::Copy { i: 3 });
        self.set_no_location();
        emit!(self, Instruction::PopExcept);
        self.set_no_location();
        emit!(self, Instruction::Reraise { depth: 1 });
        self.set_no_location();

        self.switch_to_block(end_block);
        if let Some(range) = normal_exit_range {
            self.set_source_range(range);
        }
        Ok(())
    }

    fn compile_try_star_except(
        &mut self,
        body: &[ast::Stmt],
        handlers: &[ast::ExceptHandler],
        orelse: &[ast::Stmt],
        finalbody: &[ast::Stmt],
    ) -> CompileResult<()> {
        // compiler_try_star_except
        // Stack layout during handler processing: [prev_exc, orig, list, rest]
        let handler_block = self.new_block();
        let finally_block = self.new_block();
        let cleanup_block = self.new_block();
        let end_block = self.new_block();
        let reraise_star_block = self.new_block();
        let reraise_block = self.new_block();
        let finally_cleanup_block = if !finalbody.is_empty() {
            Some(self.new_block())
        } else {
            None
        };
        let exit_block = self.new_block();
        let continuation_block = end_block;
        let else_block = if orelse.is_empty() && finalbody.is_empty() {
            continuation_block
        } else {
            self.new_block()
        };
        if !handlers.is_empty() {
            self.disable_load_fast_borrow_for_block(end_block);
            if !finalbody.is_empty() {
                self.disable_load_fast_borrow_for_block(exit_block);
            }
        }

        // Emit NOP at the try: line so LINE events fire for it
        emit!(self, Instruction::Nop);

        // Push fblock with handler info for exception table generation
        if !finalbody.is_empty() {
            emit!(
                self,
                PseudoInstruction::SetupFinally {
                    delta: finally_block
                }
            );
            self.push_fblock_full(
                FBlockType::FinallyTry,
                finally_block,
                finally_block,
                FBlockDatum::FinallyBody(finalbody.to_vec()),
            )?;
        }

        // SETUP_FINALLY for try body
        emit!(
            self,
            PseudoInstruction::SetupFinally {
                delta: handler_block
            }
        );
        self.push_fblock(FBlockType::TryExcept, handler_block, handler_block)?;
        self.compile_statements(body)?;
        emit!(self, PseudoInstruction::PopBlock);
        self.set_no_location();
        self.remove_last_no_location_nop();
        self.pop_fblock(FBlockType::TryExcept);
        emit!(
            self,
            PseudoInstruction::JumpNoInterrupt { delta: else_block }
        );
        self.set_no_location();
        self.remove_last_no_location_nop();

        // Exception handler entry
        self.switch_to_block(handler_block);
        // Stack: [exc] (from exception table)

        emit!(
            self,
            PseudoInstruction::SetupCleanup {
                delta: cleanup_block
            }
        );

        // PUSH_EXC_INFO
        emit!(self, Instruction::PushExcInfo);
        // Stack: [prev_exc, exc]

        // Push EXCEPTION_GROUP_HANDLER fblock
        self.push_fblock(
            FBlockType::ExceptionGroupHandler,
            cleanup_block,
            cleanup_block,
        )?;

        // Initialize handler stack before the loop
        // BUILD_LIST 0 + COPY 2 to set up [prev_exc, orig, list, rest]
        emit!(self, Instruction::BuildList { count: 0 });
        // Stack: [prev_exc, exc, []]
        emit!(self, Instruction::Copy { i: 2 });
        // Stack: [prev_exc, orig, list, rest]

        let n = handlers.len();
        if n == 0 {
            // Empty handlers (invalid AST) - append rest to list and proceed
            // Stack: [prev_exc, orig, list, rest]
            emit!(self, Instruction::ListAppend { i: 1 });
            // Stack: [prev_exc, orig, list]
            emit!(
                self,
                PseudoInstruction::JumpNoInterrupt {
                    delta: reraise_star_block
                }
            );
            self.set_no_location();
        }
        for (i, handler) in handlers.iter().enumerate() {
            let ast::ExceptHandler::ExceptHandler(ast::ExceptHandlerExceptHandler {
                type_,
                name,
                body,
                range: handler_range,
                ..
            }) = handler;
            let is_last_handler = i == n - 1;

            let no_match_block = self.new_block();
            let next_handler_block = if is_last_handler {
                reraise_star_block
            } else {
                self.new_block()
            };

            // Compile exception type
            if let Some(exc_type) = type_ {
                // Check for unparenthesized tuple
                if let ast::Expr::Tuple(ast::ExprTuple { elts, range, .. }) = exc_type.as_ref()
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
            emit!(self, Instruction::Copy { i: 1 });
            emit!(
                self,
                Instruction::PopJumpIfNone {
                    delta: no_match_block
                }
            );

            // Handler matched
            // Stack: [prev_exc, orig, list, new_rest, match]
            // Note: CheckEgMatch already sets the matched exception as current exception
            let handler_except_block = self.new_block();

            // Store match to name or pop
            if let Some(alias) = name {
                self.store_name(alias.as_str())?;
            } else {
                emit!(self, Instruction::PopTop); // pop match
            }
            // Stack: [prev_exc, orig, list, new_rest]

            // HANDLER_CLEANUP fblock for handler body
            emit!(
                self,
                PseudoInstruction::SetupCleanup {
                    delta: handler_except_block
                }
            );
            self.push_fblock_full(
                FBlockType::HandlerCleanup,
                next_handler_block,
                end_block,
                if let Some(alias) = name {
                    FBlockDatum::ExceptionName(alias.as_str().to_owned())
                } else {
                    FBlockDatum::None
                },
            )?;

            // Execute handler body
            self.compile_statements(body)?;

            // Handler body completed normally
            emit!(self, PseudoInstruction::PopBlock);
            self.set_no_location();
            self.pop_fblock(FBlockType::HandlerCleanup);

            // Cleanup name binding
            if let Some(alias) = name {
                self.emit_load_const(ConstantData::None);
                self.store_name(alias.as_str())?;
                self.compile_name(alias.as_str(), NameUsage::Delete)?;
            }

            if is_last_handler {
                emit!(self, Instruction::ListAppend { i: 1 });
            }
            emit!(
                self,
                PseudoInstruction::JumpNoInterrupt {
                    delta: next_handler_block
                }
            );

            // Handler raised an exception (cleanup_end label)
            self.switch_to_block(handler_except_block);
            // Stack: [prev_exc, orig, list, new_rest, lasti, raised_exc]
            // (lasti is pushed because push_lasti=true in HANDLER_CLEANUP fblock)

            // Cleanup name binding
            self.set_no_location();
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
            emit!(self, Instruction::ListAppend { i: 3 });
            // Stack: [prev_exc, orig, list, new_rest, lasti]

            // POP_TOP - pop lasti
            emit!(self, Instruction::PopTop);
            // Stack: [prev_exc, orig, list, new_rest]

            if is_last_handler {
                emit!(self, Instruction::ListAppend { i: 1 });
                emit!(
                    self,
                    PseudoInstruction::JumpNoInterrupt {
                        delta: reraise_star_block
                    }
                );
            } else {
                emit!(
                    self,
                    PseudoInstruction::JumpNoInterrupt {
                        delta: next_handler_block
                    }
                );
            }

            if is_last_handler {
                self.switch_to_block(no_match_block);
                self.set_source_range(*handler_range);
                emit!(self, Instruction::PopTop); // pop match (None)
                // Stack: [prev_exc, orig, list, new_rest]

                self.set_no_location();
                emit!(self, Instruction::ListAppend { i: 1 });
                emit!(
                    self,
                    PseudoInstruction::JumpNoInterrupt {
                        delta: reraise_star_block
                    }
                );
            } else {
                self.switch_to_block(no_match_block);
                self.set_source_range(*handler_range);
                emit!(self, Instruction::PopTop); // pop match (None)
                // Stack: [prev_exc, orig, list, new_rest]
                self.switch_to_block(next_handler_block);
            }
        }

        // Pop EXCEPTION_GROUP_HANDLER fblock
        self.pop_fblock(FBlockType::ExceptionGroupHandler);

        // Reraise star block
        self.switch_to_block(reraise_star_block);
        // Stack: [prev_exc, orig, list]
        self.set_no_location();

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
        emit!(self, Instruction::Copy { i: 1 });
        // Stack: [prev_exc, result, result]

        // POP_JUMP_IF_NOT_NONE reraise
        emit!(
            self,
            Instruction::PopJumpIfNotNone {
                delta: reraise_block
            }
        );
        // Stack: [prev_exc, result]

        // Nothing to reraise
        // POP_TOP - pop result (None)
        emit!(self, Instruction::PopTop);
        // Stack: [prev_exc]

        emit!(self, PseudoInstruction::PopBlock);
        self.set_no_location();
        // POP_EXCEPT - restore previous exception context
        emit!(self, Instruction::PopExcept);
        // Stack: []

        emit!(
            self,
            PseudoInstruction::JumpNoInterrupt {
                delta: continuation_block
            }
        );

        // Reraise the result
        self.switch_to_block(reraise_block);

        // Stack: [prev_exc, result]
        emit!(self, PseudoInstruction::PopBlock);
        self.set_no_location();
        emit!(self, Instruction::Swap { i: 2 });
        // Stack: [result, prev_exc]

        // POP_EXCEPT
        emit!(self, Instruction::PopExcept);
        // Stack: [result]

        // RERAISE 0
        emit!(self, Instruction::Reraise { depth: 0 });

        self.switch_to_block(cleanup_block);
        self.set_no_location();
        emit!(self, Instruction::Copy { i: 3 });
        emit!(self, Instruction::PopExcept);
        emit!(self, Instruction::Reraise { depth: 1 });

        // try-else path
        if else_block != continuation_block {
            self.switch_to_block(else_block);
            self.compile_statements(orelse)?;

            emit!(
                self,
                PseudoInstruction::JumpNoInterrupt {
                    delta: continuation_block
                }
            );
            self.set_no_location();
        }

        if !finalbody.is_empty() {
            self.switch_to_block(end_block);
            emit!(self, PseudoInstruction::PopBlock);
            self.set_no_location();
            self.remove_last_no_location_nop();
            self.pop_fblock(FBlockType::FinallyTry);

            // Snapshot sub_tables before first finally compilation
            let sub_table_cursor = self.symbol_table_stack.last().map(|t| t.next_sub_table);

            // Compile finally body inline for normal path
            self.compile_statements(finalbody)?;
            emit!(
                self,
                PseudoInstruction::JumpNoInterrupt { delta: exit_block }
            );

            // Restore sub_tables for exception path compilation
            if let Some(cursor) = sub_table_cursor
                && let Some(current_table) = self.symbol_table_stack.last_mut()
            {
                current_table.next_sub_table = cursor;
            }

            // Exception handler path
            self.switch_to_block(finally_block);
            emit!(self, Instruction::PushExcInfo);

            if let Some(cleanup) = finally_cleanup_block {
                emit!(self, PseudoInstruction::SetupCleanup { delta: cleanup });
                self.push_fblock(FBlockType::FinallyEnd, cleanup, cleanup)?;
            }

            self.compile_statements(finalbody)?;

            if finally_cleanup_block.is_some() {
                emit!(self, PseudoInstruction::PopBlock);
                self.pop_fblock(FBlockType::FinallyEnd);
            }

            emit!(self, Instruction::Reraise { depth: 0 });
            self.set_no_location();

            if let Some(cleanup) = finally_cleanup_block {
                self.switch_to_block(cleanup);
                emit!(self, Instruction::Copy { i: 3 });
                emit!(self, Instruction::PopExcept);
                emit!(self, Instruction::Reraise { depth: 1 });
            }
        }

        self.switch_to_block(if finalbody.is_empty() {
            end_block
        } else {
            exit_block
        });

        Ok(())
    }

    /// Compile default arguments
    // = compiler_default_arguments
    fn compile_default_arguments(
        &mut self,
        parameters: &ast::Parameters,
    ) -> CompileResult<bytecode::MakeFunctionFlags> {
        let mut funcflags = bytecode::MakeFunctionFlags::new();

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
                    count: defaults.len().to_u32()
                }
            );
            funcflags.insert(bytecode::MakeFunctionFlag::Defaults);
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
                    value: self.mangle(arg.name.as_str()).into_owned().into(),
                });
                self.compile_expression(default)?;
            }
            emit!(
                self,
                Instruction::BuildMap {
                    count: kw_with_defaults.len().to_u32(),
                }
            );
            funcflags.insert(bytecode::MakeFunctionFlag::KwOnlyDefaults);
        }

        Ok(funcflags)
    }

    /// Compile function body and create function object
    // = compiler_function_body
    fn compile_function_body(
        &mut self,
        name: &str,
        parameters: &ast::Parameters,
        body: &[ast::Stmt],
        is_async: bool,
        funcflags: bytecode::MakeFunctionFlags,
    ) -> CompileResult<()> {
        // Save source range so MAKE_FUNCTION gets the `def` line, not the body's last line
        let saved_range = self.current_source_range;

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

        // PEP 479: Wrap generator/coroutine body with StopIteration handler
        let is_gen = is_async || self.current_symbol_table().is_generator;
        let stop_iteration_block = if is_gen {
            let handler_block = self.new_block();
            emit!(
                self,
                PseudoInstruction::SetupCleanup {
                    delta: handler_block
                }
            );
            self.set_no_location();
            self.push_fblock(FBlockType::StopIteration, handler_block, handler_block)?;
            Some(handler_block)
        } else {
            None
        };

        // Handle docstring - store in co_consts[0] if present
        let (doc_str, body) = split_doc(body, &self.opts);
        if let Some(doc) = &doc_str {
            // Docstring present: store in co_consts[0] and set HAS_DOCSTRING flag
            self.current_code_info()
                .metadata
                .consts
                .insert_full(ConstantData::Str {
                    value: doc.to_string().into(),
                });
            self.current_code_info().flags |= bytecode::CodeFlags::HAS_DOCSTRING;
        }
        // Compile body statements
        self.compile_statements(body)?;

        // Emit implicit `return None` if the body doesn't end with return.
        // Also ensure None is in co_consts even when not emitting return
        // (matching CPython: functions without explicit constants always
        // have None in co_consts).
        match body.last() {
            Some(ast::Stmt::Return(_)) => {}
            _ => {
                self.emit_return_const_no_location(ConstantData::None);
            }
        }
        // Functions with no other constants should still have None in co_consts
        if self.current_code_info().metadata.consts.is_empty() {
            self.arg_constant(ConstantData::None);
        }

        // Close StopIteration handler and emit handler code
        if let Some(handler_block) = stop_iteration_block {
            emit!(self, PseudoInstruction::PopBlock);
            self.set_no_location();
            self.pop_fblock(FBlockType::StopIteration);
            self.switch_to_block(handler_block);
            emit!(
                self,
                Instruction::CallIntrinsic1 {
                    func: oparg::IntrinsicFunction1::StopIterationError
                }
            );
            self.set_no_location();
            emit!(self, Instruction::Reraise { depth: 1u32 });
            self.set_no_location();
        }

        // Exit scope and create function object
        let code = self.exit_scope();
        self.ctx = prev_ctx;

        self.set_source_range(saved_range);

        // Create function object with closure
        self.make_closure(code, funcflags)?;

        // Note: docstring is now retrieved from co_consts[0] by the VM
        // when HAS_DOCSTRING flag is set, so no runtime __doc__ assignment needed

        Ok(())
    }

    /// Compile function annotations as a closure (PEP 649)
    /// Returns true if an __annotate__ closure was created
    /// Uses symbol table's annotation_block for proper scoping.
    fn compile_annotations_closure(
        &mut self,
        func_name: &str,
        parameters: &ast::Parameters,
        returns: Option<&ast::Expr>,
    ) -> CompileResult<bool> {
        let has_signature_annotations = parameters
            .args
            .iter()
            .map(|x| &x.parameter)
            .chain(parameters.posonlyargs.iter().map(|x| &x.parameter))
            .chain(parameters.vararg.as_deref())
            .chain(parameters.kwonlyargs.iter().map(|x| &x.parameter))
            .chain(parameters.kwarg.as_deref())
            .any(|param| param.annotation.is_some())
            || returns.is_some();
        if !has_signature_annotations {
            return Ok(false);
        }

        // Try to enter annotation scope - returns None if no annotation_block exists
        let Some(saved_ctx) = self.enter_annotation_scope(func_name)? else {
            return Ok(false);
        };

        // Count annotations
        let parameters_iter = parameters
            .args
            .iter()
            .map(|x| &x.parameter)
            .chain(parameters.posonlyargs.iter().map(|x| &x.parameter))
            .chain(parameters.vararg.as_deref())
            .chain(parameters.kwonlyargs.iter().map(|x| &x.parameter))
            .chain(parameters.kwarg.as_deref());

        let num_annotations: u32 =
            u32::try_from(parameters_iter.filter(|p| p.annotation.is_some()).count())
                .expect("too many annotations")
                + if returns.is_some() { 1 } else { 0 };

        // Compile annotations inside the annotation scope
        let parameters_iter = parameters
            .args
            .iter()
            .map(|x| &x.parameter)
            .chain(parameters.posonlyargs.iter().map(|x| &x.parameter))
            .chain(parameters.vararg.as_deref())
            .chain(parameters.kwonlyargs.iter().map(|x| &x.parameter))
            .chain(parameters.kwarg.as_deref());

        for param in parameters_iter {
            if let Some(annotation) = &param.annotation {
                self.emit_load_const(ConstantData::Str {
                    value: self.mangle(param.name.as_str()).into_owned().into(),
                });
                self.compile_annotation(annotation)?;
            }
        }

        // Handle return annotation
        if let Some(annotation) = returns {
            self.emit_load_const(ConstantData::Str {
                value: "return".into(),
            });
            self.compile_annotation(annotation)?;
        }

        // Build the map and return it
        emit!(
            self,
            Instruction::BuildMap {
                count: num_annotations,
            }
        );
        emit!(self, Instruction::ReturnValue);

        // Exit the annotation scope and get the code object
        let annotate_code = self.exit_annotation_scope(saved_ctx);

        // Make a closure from the code object
        self.make_closure(annotate_code, bytecode::MakeFunctionFlags::new())?;

        Ok(true)
    }

    /// Collect annotated assignments from module/class body in AST order
    /// (including nested conditional blocks). This preserves the same walk
    /// order as symbol-table construction so the annotation scope's
    /// `sub_tables` cursor stays aligned.
    fn collect_annotations(body: &[ast::Stmt]) -> Vec<&ast::StmtAnnAssign> {
        fn walk<'a>(stmts: &'a [ast::Stmt], out: &mut Vec<&'a ast::StmtAnnAssign>) {
            for stmt in stmts {
                match stmt {
                    ast::Stmt::AnnAssign(stmt) => out.push(stmt),
                    ast::Stmt::If(ast::StmtIf {
                        body,
                        elif_else_clauses,
                        ..
                    }) => {
                        walk(body, out);
                        for clause in elif_else_clauses {
                            walk(&clause.body, out);
                        }
                    }
                    ast::Stmt::For(ast::StmtFor { body, orelse, .. })
                    | ast::Stmt::While(ast::StmtWhile { body, orelse, .. }) => {
                        walk(body, out);
                        walk(orelse, out);
                    }
                    ast::Stmt::With(ast::StmtWith { body, .. }) => walk(body, out),
                    ast::Stmt::Try(ast::StmtTry {
                        body,
                        handlers,
                        orelse,
                        finalbody,
                        ..
                    }) => {
                        walk(body, out);
                        for handler in handlers {
                            let ast::ExceptHandler::ExceptHandler(
                                ast::ExceptHandlerExceptHandler { body, .. },
                            ) = handler;
                            walk(body, out);
                        }
                        walk(orelse, out);
                        walk(finalbody, out);
                    }
                    ast::Stmt::Match(ast::StmtMatch { cases, .. }) => {
                        for case in cases {
                            walk(&case.body, out);
                        }
                    }
                    _ => {}
                }
            }
        }
        let mut annotations = Vec::new();
        walk(body, &mut annotations);
        annotations
    }

    /// Compile module-level __annotate__ function (PEP 649)
    /// Returns true if __annotate__ was created and stored
    fn compile_module_annotate(&mut self, body: &[ast::Stmt]) -> CompileResult<bool> {
        let annotations = Self::collect_annotations(body);
        let simple_annotation_count = annotations
            .iter()
            .filter(|stmt| stmt.simple && matches!(stmt.target.as_ref(), ast::Expr::Name(_)))
            .count();

        if simple_annotation_count == 0 {
            return Ok(false);
        }

        // Check if we have conditional annotations
        let has_conditional = self.current_symbol_table().has_conditional_annotations;

        // Get parent scope type BEFORE pushing annotation symbol table
        let parent_scope_type = self.current_symbol_table().typ;
        // Try to push annotation symbol table from current scope
        if !self.push_current_annotation_symbol_table() {
            return Ok(false);
        }

        // Annotation scopes are never async (even inside async functions)
        let saved_ctx = self.ctx;
        self.ctx = CompileContext {
            loop_data: None,
            in_class: saved_ctx.in_class,
            func: FunctionContext::Function,
            in_async_scope: false,
        };

        // Enter annotation scope for code generation
        let key = self.symbol_table_stack.len() - 1;
        let lineno = self.get_source_line_number().get();
        self.enter_scope(
            "__annotate__",
            CompilerScope::Annotation,
            key,
            lineno.to_u32(),
        )?;

        // Add 'format' parameter to varnames
        self.current_code_info()
            .metadata
            .varnames
            .insert("format".to_owned());

        // Emit format validation: if format > VALUE_WITH_FAKE_GLOBALS: raise NotImplementedError
        self.emit_format_validation()?;

        emit!(self, Instruction::BuildMap { count: 0 });

        let mut simple_idx = 0usize;
        for stmt in annotations {
            let ast::StmtAnnAssign {
                target,
                annotation,
                simple,
                ..
            } = stmt;
            let simple_name = if *simple {
                match target.as_ref() {
                    ast::Expr::Name(ast::ExprName { id, .. }) => Some(id.as_str()),
                    _ => None,
                }
            } else {
                None
            };

            if simple_name.is_none() {
                if !self.future_annotations {
                    self.do_not_emit_bytecode += 1;
                    let result = self.compile_annotation(annotation);
                    self.do_not_emit_bytecode -= 1;
                    result?;
                }
                continue;
            }

            let not_set_block = has_conditional.then(|| self.new_block());
            let name = simple_name.expect("missing simple annotation name");

            if has_conditional {
                self.emit_load_const(ConstantData::Integer {
                    value: simple_idx.into(),
                });
                if parent_scope_type == CompilerScope::Class {
                    let idx = self.get_free_var_index("__conditional_annotations__")?;
                    emit!(self, Instruction::LoadDeref { i: idx });
                } else {
                    let cond_annotations_name = self.name("__conditional_annotations__");
                    self.emit_load_global(cond_annotations_name, false);
                }
                emit!(
                    self,
                    Instruction::ContainsOp {
                        invert: bytecode::Invert::No
                    }
                );
                emit!(
                    self,
                    Instruction::PopJumpIfFalse {
                        delta: not_set_block.expect("missing not_set block")
                    }
                );
            }

            self.compile_annotation(annotation)?;
            emit!(self, Instruction::Copy { i: 2 });
            self.emit_load_const(ConstantData::Str {
                value: self.mangle(name).into_owned().into(),
            });
            emit!(self, Instruction::StoreSubscr);
            simple_idx += 1;

            if let Some(not_set_block) = not_set_block {
                self.switch_to_block(not_set_block);
            }
        }

        emit!(self, Instruction::ReturnValue);

        // Exit annotation scope - pop symbol table, restore to parent's annotation_block, and get code
        let annotation_table = self.pop_symbol_table();
        // Restore annotation_block to module's symbol table
        self.symbol_table_stack
            .last_mut()
            .expect("no module symbol table")
            .annotation_block = Some(Box::new(annotation_table));
        // Restore context
        self.ctx = saved_ctx;
        // Exit code scope
        let pop = self.code_stack.pop();
        let annotate_code = unwrap_internal(
            self,
            compiler_unwrap_option(self, pop).finalize_code(&self.opts),
        );

        // Make a closure from the code object
        self.make_closure(annotate_code, bytecode::MakeFunctionFlags::new())?;

        // Store as __annotate_func__ for classes, __annotate__ for modules
        let name = if parent_scope_type == CompilerScope::Class {
            "__annotate_func__"
        } else {
            "__annotate__"
        };
        self.store_name(name)?;

        Ok(true)
    }

    // = compiler_function
    #[allow(clippy::too_many_arguments)]
    fn compile_function_def(
        &mut self,
        name: &str,
        parameters: &ast::Parameters,
        body: &[ast::Stmt],
        decorator_list: &[ast::Decorator],
        returns: Option<&ast::Expr>, // TODO: use type hint somehow..
        is_async: bool,
        type_params: Option<&ast::TypeParams>,
    ) -> CompileResult<()> {
        // Save the source range of the `def` line before compiling decorators/defaults,
        // so that the function code object gets the correct co_firstlineno.
        let def_source_range = self.current_source_range;

        self.prepare_decorators(decorator_list)?;

        // compile defaults and return funcflags
        let funcflags = self.compile_default_arguments(parameters)?;

        // Restore the `def` line range so that enter_function → push_output → get_source_line_number()
        // records the `def` keyword's line as co_firstlineno, not the last default-argument line.
        self.set_source_range(def_source_range);

        let is_generic = type_params.is_some();
        let mut num_typeparam_args = 0;

        // Save context before entering TypeParams scope
        let saved_ctx = self.ctx;

        if is_generic {
            // Count args to pass to type params scope
            if funcflags.contains(&bytecode::MakeFunctionFlag::Defaults) {
                num_typeparam_args += 1;
            }
            if funcflags.contains(&bytecode::MakeFunctionFlag::KwOnlyDefaults) {
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

            // TypeParams scope is function-like
            self.ctx = CompileContext {
                loop_data: None,
                in_class: saved_ctx.in_class,
                func: FunctionContext::Function,
                in_async_scope: false,
            };

            // Add parameter names to varnames for the type params scope
            // These will be passed as arguments when the closure is called
            let current_info = self.current_code_info();
            if funcflags.contains(&bytecode::MakeFunctionFlag::Defaults) {
                current_info
                    .metadata
                    .varnames
                    .insert(".defaults".to_owned());
            }
            if funcflags.contains(&bytecode::MakeFunctionFlag::KwOnlyDefaults) {
                current_info
                    .metadata
                    .varnames
                    .insert(".kwdefaults".to_owned());
            }

            // Compile type parameters
            self.compile_type_params(type_params.unwrap())?;

            // Load defaults/kwdefaults with LOAD_FAST
            for i in 0..num_typeparam_args {
                let var_num = oparg::VarNum::from(i as u32);
                emit!(self, Instruction::LoadFast { var_num });
            }
        }

        // Compile annotations as closure (PEP 649)
        let mut annotations_flag = bytecode::MakeFunctionFlags::new();
        if self.compile_annotations_closure(name, parameters, returns)? {
            annotations_flag.insert(bytecode::MakeFunctionFlag::Annotate);
        }

        // Compile function body
        let final_funcflags = funcflags | annotations_flag;
        self.compile_function_body(name, parameters, body, is_async, final_funcflags)?;

        // Handle type params if present
        if is_generic {
            // SWAP to get function on top
            // Stack: [type_params_tuple, function] -> [function, type_params_tuple]
            emit!(self, Instruction::Swap { i: 2 });

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
            self.ctx = saved_ctx;

            // Make closure for type params code
            self.make_closure(type_params_code, bytecode::MakeFunctionFlags::new())?;

            // Call the type params closure with defaults/kwdefaults as arguments.
            // Call protocol: [callable, self_or_null, arg1, ..., argN]
            // We need to reorder: [args..., closure] -> [closure, NULL, args...]
            // Using Swap operations to move closure down and insert NULL.
            // Note: num_typeparam_args is at most 2 (defaults tuple, kwdefaults dict).
            if num_typeparam_args > 0 {
                match num_typeparam_args {
                    1 => {
                        // Stack: [arg1, closure]
                        emit!(self, Instruction::Swap { i: 2 }); // [closure, arg1]
                        emit!(self, Instruction::PushNull); // [closure, arg1, NULL]
                        emit!(self, Instruction::Swap { i: 2 }); // [closure, NULL, arg1]
                    }
                    2 => {
                        // Stack: [arg1, arg2, closure]
                        emit!(self, Instruction::Swap { i: 3 }); // [closure, arg2, arg1]
                        emit!(self, Instruction::Swap { i: 2 }); // [closure, arg1, arg2]
                        emit!(self, Instruction::PushNull); // [closure, arg1, arg2, NULL]
                        emit!(self, Instruction::Swap { i: 3 }); // [closure, NULL, arg2, arg1]
                        emit!(self, Instruction::Swap { i: 2 }); // [closure, NULL, arg1, arg2]
                    }
                    _ => unreachable!("only defaults and kwdefaults are supported"),
                }
                emit!(
                    self,
                    Instruction::Call {
                        argc: num_typeparam_args as u32
                    }
                );
            } else {
                // Stack: [closure]
                emit!(self, Instruction::PushNull);
                // Stack: [closure, NULL]
                emit!(self, Instruction::Call { argc: 0 });
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
        let table = self.symbol_table_stack.last().unwrap();

        // Special handling for __class__, __classdict__, and __conditional_annotations__ in class scope
        // This should only apply when we're actually IN a class body,
        // not when we're in a method nested inside a class.
        if table.typ == CompilerScope::Class
            && (name == "__class__"
                || name == "__classdict__"
                || name == "__conditional_annotations__")
        {
            return Ok(SymbolScope::Cell);
        }
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

                emit!(self, PseudoInstruction::LoadClosure { i: idx.to_u32() });
            }

            // Build tuple of closure variables
            emit!(
                self,
                Instruction::BuildTuple {
                    count: code.freevars.len().to_u32(),
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
                    flag: bytecode::MakeFunctionFlag::Closure
                }
            );
        }

        // Set annotations if present
        if flags.contains(&bytecode::MakeFunctionFlag::Annotations) {
            emit!(
                self,
                Instruction::SetFunctionAttribute {
                    flag: bytecode::MakeFunctionFlag::Annotations
                }
            );
        }

        // Set __annotate__ closure if present (PEP 649)
        if flags.contains(&bytecode::MakeFunctionFlag::Annotate) {
            emit!(
                self,
                Instruction::SetFunctionAttribute {
                    flag: bytecode::MakeFunctionFlag::Annotate
                }
            );
        }

        // Set kwdefaults if present
        if flags.contains(&bytecode::MakeFunctionFlag::KwOnlyDefaults) {
            emit!(
                self,
                Instruction::SetFunctionAttribute {
                    flag: bytecode::MakeFunctionFlag::KwOnlyDefaults
                }
            );
        }

        // Set defaults if present
        if flags.contains(&bytecode::MakeFunctionFlag::Defaults) {
            emit!(
                self,
                Instruction::SetFunctionAttribute {
                    flag: bytecode::MakeFunctionFlag::Defaults
                }
            );
        }

        // Set type_params if present
        if flags.contains(&bytecode::MakeFunctionFlag::TypeParams) {
            emit!(
                self,
                Instruction::SetFunctionAttribute {
                    flag: bytecode::MakeFunctionFlag::TypeParams
                }
            );
        }

        Ok(())
    }

    // Python/compile.c _PyCompile_MaybeAddStaticAttributeToClass
    fn maybe_add_static_attribute_to_class(&mut self, value: &ast::Expr, attr: &str) {
        if !matches!(value, ast::Expr::Name(n) if n.id.as_str() == "self") {
            return;
        }
        if let Some(class_unit) = self
            .code_stack
            .iter_mut()
            .rev()
            .find(|unit| unit.static_attributes.is_some())
        {
            class_unit
                .static_attributes
                .as_mut()
                .unwrap()
                .insert(attr.to_owned());
        }
    }

    // Python/compile.c find_ann
    fn find_ann(body: &[ast::Stmt]) -> bool {
        for statement in body {
            let res = match &statement {
                ast::Stmt::AnnAssign(_) => true,
                ast::Stmt::For(ast::StmtFor { body, orelse, .. }) => {
                    Self::find_ann(body) || Self::find_ann(orelse)
                }
                ast::Stmt::If(ast::StmtIf {
                    body,
                    elif_else_clauses,
                    ..
                }) => {
                    Self::find_ann(body)
                        || elif_else_clauses.iter().any(|x| Self::find_ann(&x.body))
                }
                ast::Stmt::While(ast::StmtWhile { body, orelse, .. }) => {
                    Self::find_ann(body) || Self::find_ann(orelse)
                }
                ast::Stmt::With(ast::StmtWith { body, .. }) => Self::find_ann(body),
                ast::Stmt::Match(ast::StmtMatch { cases, .. }) => {
                    cases.iter().any(|case| Self::find_ann(&case.body))
                }
                ast::Stmt::Try(ast::StmtTry {
                    body,
                    handlers,
                    orelse,
                    finalbody,
                    ..
                }) => {
                    Self::find_ann(body)
                        || handlers.iter().any(|h| {
                            let ast::ExceptHandler::ExceptHandler(
                                ast::ExceptHandlerExceptHandler { body, .. },
                            ) = h;
                            Self::find_ann(body)
                        })
                        || Self::find_ann(orelse)
                        || Self::find_ann(finalbody)
                }
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
        body: &[ast::Stmt],
        type_params: Option<&ast::TypeParams>,
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

        // Load __name__ and store as __module__
        let dunder_name = self.name("__name__");
        emit!(self, Instruction::LoadName { namei: dunder_name });
        let dunder_module = self.name("__module__");
        emit!(
            self,
            Instruction::StoreName {
                namei: dunder_module
            }
        );

        // Store __qualname__
        self.emit_load_const(ConstantData::Str {
            value: qualname.into(),
        });
        let qualname_name = self.name("__qualname__");
        emit!(
            self,
            Instruction::StoreName {
                namei: qualname_name
            }
        );

        // Store __firstlineno__ before __doc__
        self.emit_load_const(ConstantData::Integer {
            value: BigInt::from(firstlineno),
        });
        let firstlineno_name = self.name("__firstlineno__");
        emit!(
            self,
            Instruction::StoreName {
                namei: firstlineno_name
            }
        );

        // Set __type_params__ from the enclosing type-params closure when
        // compiling a generic class body.
        if type_params.is_some() {
            self.load_name(".type_params")?;
            self.store_name("__type_params__")?;
        }

        // PEP 649: Initialize __classdict__ after synthetic generic-class
        // setup so nested generic classes match CPython's prologue order.
        if self.current_symbol_table().needs_classdict {
            emit!(self, Instruction::LoadLocals);
            let classdict_idx = self.get_cell_var_index("__classdict__")?;
            emit!(self, Instruction::StoreDeref { i: classdict_idx });
        }

        // Store __doc__ only if there's an explicit docstring.
        if let Some(doc) = doc_str {
            self.emit_load_const(ConstantData::Str { value: doc.into() });
            let doc_name = self.name("__doc__");
            emit!(self, Instruction::StoreName { namei: doc_name });
        }

        // Handle class annotation bookkeeping in CPython order.
        if Self::find_ann(body) {
            if Self::scope_needs_conditional_annotations_cell(self.current_symbol_table()) {
                emit!(self, Instruction::BuildSet { count: 0 });
                self.store_name("__conditional_annotations__")?;
            }

            if self.future_annotations {
                emit!(self, Instruction::SetupAnnotations);
            }
        }

        // 3. Compile the class body
        self.compile_statements(body)?;

        if Self::find_ann(body) && !self.future_annotations {
            self.compile_module_annotate(body)?;
        }

        // 4. Handle __classcell__ if needed
        let classcell_idx = self
            .code_stack
            .last_mut()
            .unwrap()
            .metadata
            .cellvars
            .iter()
            .position(|var| *var == "__class__");

        // Emit __static_attributes__ tuple
        {
            let mut attrs: Vec<String> = self
                .code_stack
                .last()
                .unwrap()
                .static_attributes
                .as_ref()
                .map(|s| s.iter().cloned().collect())
                .unwrap_or_default();
            attrs.sort();
            self.emit_load_const(ConstantData::Tuple {
                elements: attrs
                    .into_iter()
                    .map(|s| ConstantData::Str { value: s.into() })
                    .collect(),
            });
            self.set_no_location();
            let static_attrs_name = self.name("__static_attributes__");
            emit!(
                self,
                Instruction::StoreName {
                    namei: static_attrs_name
                }
            );
            self.set_no_location();
        }

        // Store __classdictcell__ if __classdict__ is a cell variable
        if self.current_symbol_table().needs_classdict {
            let classdict_idx = u32::from(self.get_cell_var_index("__classdict__")?);
            emit!(self, PseudoInstruction::LoadClosure { i: classdict_idx });
            self.set_no_location();
            let classdictcell = self.name("__classdictcell__");
            emit!(
                self,
                Instruction::StoreName {
                    namei: classdictcell
                }
            );
            self.set_no_location();
        }

        if let Some(classcell_idx) = classcell_idx {
            emit!(
                self,
                PseudoInstruction::LoadClosure {
                    i: classcell_idx.to_u32()
                }
            );
            self.set_no_location();
            emit!(self, Instruction::Copy { i: 1 });
            self.set_no_location();
            let classcell = self.name("__classcell__");
            emit!(self, Instruction::StoreName { namei: classcell });
            self.set_no_location();
        } else {
            self.emit_load_const(ConstantData::None);
            self.set_no_location();
        }

        // Return the class namespace
        self.emit_return_value();
        self.set_no_location();

        // Exit scope and return the code object
        Ok(self.exit_scope())
    }

    fn compile_class_def(
        &mut self,
        name: &str,
        body: &[ast::Stmt],
        decorator_list: &[ast::Decorator],
        type_params: Option<&ast::TypeParams>,
        arguments: Option<&ast::Arguments>,
    ) -> CompileResult<()> {
        self.prepare_decorators(decorator_list)?;

        let is_generic = type_params.is_some();
        let firstlineno = decorator_list
            .first()
            .map(|decorator| {
                self.source_file
                    .to_source_code()
                    .line_index(decorator.expression.range().start())
                    .get()
                    .to_u32()
            })
            .unwrap_or_else(|| self.get_source_line_number().get().to_u32());

        // Save context before entering any scopes
        let saved_ctx = self.ctx;

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

            // TypeParams scope is function-like
            self.ctx = CompileContext {
                loop_data: None,
                in_class: saved_ctx.in_class,
                func: FunctionContext::Function,
                in_async_scope: false,
            };

            // Compile type parameters and store them in the synthetic cell that
            // generic class bodies close over.
            self.compile_type_params(type_params.unwrap())?;
            self.store_name(".type_params")?;
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
            // Generate class creation code
            emit!(self, Instruction::LoadBuildClass);
            emit!(self, Instruction::PushNull);

            // Create the class body function with the .type_params closure
            // captured through the class code object's freevars.
            self.make_closure(class_code, bytecode::MakeFunctionFlags::new())?;
            self.emit_load_const(ConstantData::Str { value: name.into() });

            // Create .generic_base after the class function and name are on the
            // stack so the remaining call shape matches CPython's ordering.
            self.load_name(".type_params")?;
            emit!(
                self,
                Instruction::CallIntrinsic1 {
                    func: bytecode::IntrinsicFunction1::SubscriptGeneric
                }
            );
            self.store_name(".generic_base")?;

            // Compile bases and call __build_class__
            // Check for starred bases or **kwargs
            let has_starred = arguments.is_some_and(|args| {
                args.args
                    .iter()
                    .any(|arg| matches!(arg, ast::Expr::Starred(_)))
            });
            let has_double_star =
                arguments.is_some_and(|args| args.keywords.iter().any(|kw| kw.arg.is_none()));

            if has_starred || has_double_star {
                // Use CallFunctionEx for *bases or **kwargs
                // Stack has: [__build_class__, NULL, class_func, name]
                // Need to build: args tuple = (class_func, name, *bases, .generic_base)

                // Build a list starting with class_func and name (2 elements already on stack)
                emit!(self, Instruction::BuildList { count: 2 });

                // Add bases to the list
                if let Some(arguments) = arguments {
                    for arg in &arguments.args {
                        if let ast::Expr::Starred(ast::ExprStarred { value, .. }) = arg {
                            // Starred: compile and extend
                            self.compile_expression(value)?;
                            emit!(self, Instruction::ListExtend { i: 1 });
                        } else {
                            // Non-starred: compile and append
                            self.compile_expression(arg)?;
                            emit!(self, Instruction::ListAppend { i: 1 });
                        }
                    }
                }

                // Add .generic_base as final element
                self.load_name(".generic_base")?;
                emit!(self, Instruction::ListAppend { i: 1 });

                // Convert list to tuple
                emit!(
                    self,
                    Instruction::CallIntrinsic1 {
                        func: IntrinsicFunction1::ListToTuple
                    }
                );

                // Build kwargs if needed
                if arguments.is_some_and(|args| !args.keywords.is_empty()) {
                    self.compile_keywords(&arguments.unwrap().keywords)?;
                } else {
                    emit!(self, Instruction::PushNull);
                }
                emit!(self, Instruction::CallFunctionEx);
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
                self.load_name(".generic_base")?;

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
                            argc: nargs
                                + u32::try_from(arguments.keywords.len())
                                    .expect("too many keyword arguments")
                        }
                    );
                } else {
                    emit!(self, Instruction::Call { argc: nargs });
                }
            }

            // Return the created class
            self.emit_return_value();

            // Exit type params scope and wrap in function
            let type_params_code = self.exit_scope();
            self.ctx = saved_ctx;

            // Execute the type params function
            self.make_closure(type_params_code, bytecode::MakeFunctionFlags::new())?;
            emit!(self, Instruction::PushNull);
            emit!(self, Instruction::Call { argc: 0 });
        } else {
            // Non-generic class: standard path
            emit!(self, Instruction::LoadBuildClass);
            emit!(self, Instruction::PushNull);

            // Create class function with closure
            self.make_closure(class_code, bytecode::MakeFunctionFlags::new())?;
            self.emit_load_const(ConstantData::Str { value: name.into() });

            if let Some(arguments) = arguments {
                self.codegen_call_helper(2, arguments, self.current_source_range)?;
            } else {
                emit!(self, Instruction::Call { argc: 2 });
            }
        }

        // Step 4: Apply decorators and store (common to both paths)
        self.apply_decorators(decorator_list);
        self.store_name(name)
    }

    /// Compile an if statement with constant condition elimination.
    /// = compiler_if in CPython codegen.c
    fn compile_if(
        &mut self,
        test: &ast::Expr,
        body: &[ast::Stmt],
        elif_else_clauses: &[ast::ElifElseClause],
        _stmt_range: TextRange,
    ) -> CompileResult<()> {
        let end_block = self.new_block();
        let next_block = if elif_else_clauses.is_empty() {
            end_block
        } else {
            self.new_block()
        };

        if matches!(self.constant_expr_truthiness(test)?, Some(false)) {
            self.disable_load_fast_borrow_for_block(next_block);
        }
        self.compile_jump_if(test, false, next_block)?;
        self.compile_statements(body)?;

        let Some((clause, rest)) = elif_else_clauses.split_first() else {
            self.switch_to_block(end_block);
            return Ok(());
        };

        emit!(
            self,
            PseudoInstruction::JumpNoInterrupt { delta: end_block }
        );
        self.set_no_location();
        self.switch_to_block(next_block);

        if let Some(test) = &clause.test {
            self.compile_if(test, &clause.body, rest, test.range())?;
        } else {
            debug_assert!(rest.is_empty());
            self.compile_statements(&clause.body)?;
        }
        self.switch_to_block(end_block);
        Ok(())
    }

    fn compile_while(
        &mut self,
        test: &ast::Expr,
        body: &[ast::Stmt],
        orelse: &[ast::Stmt],
    ) -> CompileResult<()> {
        self.enter_conditional_block();

        let while_block = self.new_block();
        let else_block = self.new_block();
        let after_block = self.new_block();

        self.switch_to_block(while_block);
        self.push_fblock(FBlockType::WhileLoop, while_block, after_block)?;
        if matches!(self.constant_expr_truthiness(test)?, Some(false)) {
            self.disable_load_fast_borrow_for_block(else_block);
            self.disable_load_fast_borrow_for_block(after_block);
        }
        self.compile_jump_if(test, false, else_block)?;

        let was_in_loop = self.ctx.loop_data.replace((while_block, after_block));
        self.compile_statements(body)?;
        self.ctx.loop_data = was_in_loop;
        emit!(self, PseudoInstruction::Jump { delta: while_block });
        self.set_no_location();
        self.switch_to_block(else_block);

        self.pop_fblock(FBlockType::WhileLoop);
        self.compile_statements(orelse)?;
        self.switch_to_block(after_block);

        self.leave_conditional_block();
        Ok(())
    }

    fn compile_with(
        &mut self,
        items: &[ast::WithItem],
        body: &[ast::Stmt],
        is_async: bool,
    ) -> CompileResult<()> {
        self.enter_conditional_block();

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

        // Compile context expression and load __enter__/__exit__ methods
        self.compile_expression(&item.context_expr)?;
        self.set_source_range(with_range);

        // Stack: [cm]
        emit!(self, Instruction::Copy { i: 1 }); // [cm, cm]

        if is_async {
            if self.ctx.func != FunctionContext::AsyncFunction {
                return Err(self.error(CodegenErrorType::InvalidAsyncWith));
            }
            // Load __aexit__ and __aenter__, then call __aenter__
            emit!(
                self,
                Instruction::LoadSpecial {
                    method: SpecialMethod::AExit
                }
            ); // [cm, aexit_func, self_ae]
            emit!(self, Instruction::Swap { i: 2 }); // [cm, self_ae, aexit_func]
            emit!(self, Instruction::Swap { i: 3 }); // [aexit_func, self_ae, cm]
            emit!(
                self,
                Instruction::LoadSpecial {
                    method: SpecialMethod::AEnter
                }
            ); // [aexit_func, self_ae, aenter_func, self_an]
            emit!(self, Instruction::Call { argc: 0 }); // [aexit_func, self_ae, awaitable]
            emit!(self, Instruction::GetAwaitable { r#where: 1 });
            self.emit_load_const(ConstantData::None);
            let _ = self.compile_yield_from_sequence(true)?;
        } else {
            // Load __exit__ and __enter__, then call __enter__
            emit!(
                self,
                Instruction::LoadSpecial {
                    method: SpecialMethod::Exit
                }
            ); // [cm, exit_func, self_exit]
            emit!(self, Instruction::Swap { i: 2 }); // [cm, self_exit, exit_func]
            emit!(self, Instruction::Swap { i: 3 }); // [exit_func, self_exit, cm]
            emit!(
                self,
                Instruction::LoadSpecial {
                    method: SpecialMethod::Enter
                }
            ); // [exit_func, self_exit, enter_func, self_enter]
            emit!(self, Instruction::Call { argc: 0 }); // [exit_func, self_exit, result]
        }

        // Stack: [..., __exit__, enter_result]
        // Push fblock for exception table - handler goes to exc_handler_block
        // preserve_lasti=true for with statements
        emit!(
            self,
            PseudoInstruction::SetupWith {
                delta: exc_handler_block
            }
        );
        self.push_fblock(
            if is_async {
                FBlockType::AsyncWith
            } else {
                FBlockType::With
            },
            exc_handler_block, // block start (will become exit target after store)
            after_block,
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

        let preserve_outer_cleanup_target_nop = !is_async
            && (Self::statements_end_with_with_cleanup_scope_exit(body)
                || self.current_block_has_terminal_with_suppress_exit_predecessor());

        // Pop fblock before normal exit.  CPython emits this POP_BLOCK with
        // no location for sync with, but with the with-item location for
        // async with.
        if is_async {
            self.set_source_range(with_range);
        }
        emit!(self, PseudoInstruction::PopBlock);
        if !is_async {
            self.set_no_location();
            if preserve_outer_cleanup_target_nop {
                self.preserve_last_redundant_nop();
            } else {
                self.remove_last_no_location_nop();
            }
            self.set_source_range(with_range);
        }
        self.pop_fblock(if is_async {
            FBlockType::AsyncWith
        } else {
            FBlockType::With
        });

        // ===== Normal exit path =====
        // Stack: [..., exit_func, self_exit]
        // Call exit_func(self_exit, None, None, None)
        self.emit_load_const(ConstantData::None);
        self.emit_load_const(ConstantData::None);
        self.emit_load_const(ConstantData::None);
        emit!(self, Instruction::Call { argc: 3 });
        if is_async {
            emit!(self, Instruction::GetAwaitable { r#where: 2 });
            self.emit_load_const(ConstantData::None);
            let _ = self.compile_yield_from_sequence(true)?;
        }
        emit!(self, Instruction::PopTop); // Pop __exit__ result
        emit!(self, PseudoInstruction::Jump { delta: after_block });
        self.set_no_location();

        // ===== Exception handler path =====
        // Stack at entry: [..., exit_func, self_exit, lasti, exc]
        // PUSH_EXC_INFO -> [..., exit_func, self_exit, lasti, prev_exc, exc]
        self.switch_to_block(exc_handler_block);

        let cleanup_block = self.new_block();
        let suppress_block = self.new_block();

        emit!(
            self,
            PseudoInstruction::SetupCleanup {
                delta: cleanup_block
            }
        );
        self.push_fblock(FBlockType::ExceptionHandler, exc_handler_block, after_block)?;

        emit!(self, Instruction::PushExcInfo);

        // WITH_EXCEPT_START: call exit_func(self_exit, type, value, tb)
        // Stack: [..., exit_func, self_exit, lasti, prev_exc, exc]
        emit!(self, Instruction::WithExceptStart);

        if is_async {
            emit!(self, Instruction::GetAwaitable { r#where: 2 });
            self.emit_load_const(ConstantData::None);
            let _ = self.compile_yield_from_sequence(true)?;
        }

        emit!(self, Instruction::ToBool);
        emit!(
            self,
            Instruction::PopJumpIfTrue {
                delta: suppress_block
            }
        );

        emit!(self, Instruction::Reraise { depth: 2 });

        // ===== Suppress block =====
        // Stack: [..., exit_func, self_exit, lasti, prev_exc, exc, True]
        self.switch_to_block(suppress_block);
        emit!(self, Instruction::PopTop); // pop True
        emit!(self, PseudoInstruction::PopBlock);
        self.pop_fblock(FBlockType::ExceptionHandler);
        emit!(self, Instruction::PopExcept); // pop exc, restore prev_exc
        emit!(self, Instruction::PopTop); // pop lasti
        emit!(self, Instruction::PopTop); // pop self_exit
        emit!(self, Instruction::PopTop); // pop exit_func
        emit!(
            self,
            PseudoInstruction::JumpNoInterrupt { delta: after_block }
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
        emit!(self, Instruction::Copy { i: 3 });
        emit!(self, Instruction::PopExcept);
        emit!(self, Instruction::Reraise { depth: 1 });

        // ===== After block =====
        self.switch_to_block(after_block);

        self.leave_conditional_block();
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
        self.enter_conditional_block();

        // Start loop
        let for_block = self.new_block();
        let else_block = self.new_block();
        let after_block = self.new_block();
        let mut end_async_for_target = BlockIdx::NULL;

        // The thing iterated:
        self.compile_for_iterable_expression(iter, is_async)?;

        if is_async {
            if self.ctx.func != FunctionContext::AsyncFunction {
                return Err(self.error(CodegenErrorType::InvalidAsyncFor));
            }
            emit!(self, Instruction::GetAIter);

            self.switch_to_block(for_block);

            // codegen_async_for: push fblock BEFORE SETUP_FINALLY
            self.push_fblock(FBlockType::ForLoop, for_block, after_block)?;

            // SETUP_FINALLY to guard the __anext__ call
            emit!(self, PseudoInstruction::SetupFinally { delta: else_block });
            emit!(self, Instruction::GetANext);
            self.emit_load_const(ConstantData::None);
            end_async_for_target = self.compile_yield_from_sequence(true)?;
            // POP_BLOCK for SETUP_FINALLY - only GetANext/yield_from are protected
            emit!(self, PseudoInstruction::PopBlock);
            emit!(self, Instruction::NotTaken);

            // Success block for __anext__
            self.compile_store(target)?;
        } else {
            // Retrieve Iterator
            emit!(self, Instruction::GetIter);

            self.switch_to_block(for_block);

            // Push fblock for for loop
            self.push_fblock(FBlockType::ForLoop, for_block, after_block)?;

            emit!(self, Instruction::ForIter { delta: else_block });

            // Match CPython's line attribution by compiling the loop target on
            // the target range directly instead of leaving a synthetic anchor
            // NOP between FOR_ITER and the unpack/store sequence.
            let saved_range = self.current_source_range;
            self.set_source_range(target.range());
            self.compile_store(target)?;
            self.set_source_range(saved_range);
        };

        let was_in_loop = self.ctx.loop_data.replace((for_block, after_block));
        self.compile_statements(body)?;
        self.ctx.loop_data = was_in_loop;
        emit!(self, PseudoInstruction::Jump { delta: for_block });
        self.set_no_location();

        self.switch_to_block(else_block);

        // Except block for __anext__ / end of sync for
        // No PopBlock here - for async, POP_BLOCK is already in for_block
        self.pop_fblock(FBlockType::ForLoop);

        // End-of-loop instructions are on the `for` line, not the body's last line
        let saved_range = self.current_source_range;
        self.set_source_range(iter.range());
        if is_async {
            self.emit_end_async_for(end_async_for_target);
        } else {
            emit!(self, Instruction::EndFor);
            emit!(self, Instruction::PopIter);
        }
        self.set_source_range(saved_range);
        self.compile_statements(orelse)?;

        self.switch_to_block(after_block);

        // Implicit return after for-loop should be attributed to the `for` line
        self.set_source_range(iter.range());

        self.leave_conditional_block();
        Ok(())
    }

    fn compile_for_iterable_expression(
        &mut self,
        iter: &ast::Expr,
        is_async: bool,
    ) -> CompileResult<()> {
        // Match CPython's iterable lowering for `for`/comprehension fronts:
        // a non-starred list literal used only for iteration is emitted as a tuple.
        // Skip async-for/async comprehension iteration because GET_AITER expects
        // the original object semantics.
        if !is_async
            && let ast::Expr::List(ast::ExprList { elts, .. }) = iter
            && !elts.iter().any(|e| matches!(e, ast::Expr::Starred(_)))
        {
            if let Some(folded) = self.try_fold_constant_collection(elts, CollectionType::List)? {
                self.emit_load_const(folded);
            } else {
                for elt in elts {
                    self.compile_expression(elt)?;
                }
                emit!(
                    self,
                    Instruction::BuildTuple {
                        count: u32::try_from(elts.len()).expect("too many elements"),
                    }
                );
            }
            return Ok(());
        }

        self.compile_expression(iter)
    }

    fn singleton_comprehension_assignment_iter(iter: &ast::Expr) -> Option<&ast::Expr> {
        let elts = match iter {
            ast::Expr::List(ast::ExprList { elts, .. }) => elts,
            ast::Expr::Tuple(ast::ExprTuple { elts, .. }) => elts,
            _ => return None,
        };
        match elts.as_slice() {
            [elt] if !matches!(elt, ast::Expr::Starred(_)) => Some(elt),
            _ => None,
        }
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
        self.error(CodegenErrorType::SyntaxError(format!(
            "cannot use forbidden name '{name}' in pattern"
        )))
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
                    PseudoInstruction::Jump {
                        delta: pc.fail_pop[pops]
                    }
                );
            }
            JumpOp::PopJumpIfFalse => {
                emit!(
                    self,
                    Instruction::PopJumpIfFalse {
                        delta: pc.fail_pop[pops]
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
                    i: u32::try_from(count).unwrap()
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
        n: Option<&ast::Identifier>,
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

    fn pattern_unpack_helper(&mut self, elts: &[ast::Pattern]) -> CompileResult<()> {
        let n = elts.len();
        let mut seen_star = false;
        for (i, elt) in elts.iter().enumerate() {
            if elt.is_match_star() {
                if !seen_star {
                    if i >= (1 << 8) || (n - i - 1) >= ((i32::MAX as usize) >> 8) {
                        todo!();
                        // return self.compiler_error(loc, "too many expressions in star-unpacking sequence pattern");
                    }
                    let counts = UnpackExArgs {
                        before: u8::try_from(i).unwrap(),
                        after: u8::try_from(n - i - 1).unwrap(),
                    };
                    emit!(self, Instruction::UnpackEx { counts });
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
                    count: u32::try_from(n).unwrap()
                }
            );
        }
        Ok(())
    }

    fn pattern_helper_sequence_unpack(
        &mut self,
        patterns: &[ast::Pattern],
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
        patterns: &[ast::Pattern],
        star: usize,
        pc: &mut PatternContext,
    ) -> CompileResult<()> {
        // Keep the subject around for extracting elements.
        pc.on_top += 1;
        for (i, pattern) in patterns.iter().enumerate() {
            let is_true_wildcard = matches!(
                pattern,
                ast::Pattern::MatchAs(ast::PatternMatchAs {
                    pattern: None,
                    name: None,
                    ..
                })
            );
            if is_true_wildcard {
                continue;
            }
            if i == star {
                // This must be a starred wildcard.
                // assert!(pattern.is_star_wildcard());
                continue;
            }
            // Duplicate the subject.
            emit!(self, Instruction::Copy { i: 1 });
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
            emit!(
                self,
                Instruction::BinaryOp {
                    op: BinaryOperator::Subscr
                }
            );
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
        p: &ast::Pattern,
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
        p: &ast::PatternMatchAs,
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
                }
                // A wildcard makes remaining patterns unreachable.
                return Err(self.error(CodegenErrorType::UnreachablePattern(
                    PatternUnreachableReason::Wildcard,
                )));
            }
            // If irrefutable matches are allowed, store the name (if any).
            return self.pattern_helper_store_name(p.name.as_ref(), pc);
        }

        // Otherwise, there is a sub-pattern. Duplicate the object on top of the stack.
        pc.on_top += 1;
        emit!(self, Instruction::Copy { i: 1 });
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
        p: &ast::PatternMatchStar,
        pc: &mut PatternContext,
    ) -> CompileResult<()> {
        self.pattern_helper_store_name(p.name.as_ref(), pc)?;
        Ok(())
    }

    /// Validates that keyword attributes in a class pattern are allowed
    /// and not duplicated.
    fn validate_kwd_attrs(
        &mut self,
        attrs: &[ast::Identifier],
        _patterns: &[ast::Pattern],
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
        p: &ast::PatternMatchClass,
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
            return Err(self.error(CodegenErrorType::SyntaxError(
                "too many sub-patterns in class pattern".to_owned(),
            )));
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
        emit!(
            self,
            Instruction::MatchClass {
                count: u32::try_from(nargs).unwrap()
            }
        );
        // 3. Duplicate the top of the stack.
        emit!(self, Instruction::Copy { i: 1 });
        // 4. Load None.
        self.emit_load_const(ConstantData::None);
        // 5. Compare with IS_OP 1.
        emit!(
            self,
            Instruction::IsOp {
                invert: Invert::Yes
            }
        );

        // At this point the TOS is a tuple of (nargs + n_attrs) attributes (or None).
        pc.on_top += 1;
        self.jump_to_fail_pop(pc, JumpOp::PopJumpIfFalse)?;

        // Unpack the tuple into (nargs + n_attrs) items.
        let total = nargs + n_attrs;
        emit!(
            self,
            Instruction::UnpackSequence {
                count: u32::try_from(total).unwrap()
            }
        );
        pc.on_top += total;
        pc.on_top -= 1;

        // Process each sub-pattern.
        for subpattern in patterns.iter().chain(kwd_patterns.iter()) {
            // Check if this is a true wildcard (underscore pattern without name binding)
            let is_true_wildcard = match subpattern {
                ast::Pattern::MatchAs(match_as) => {
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
        p: &ast::PatternMatchMapping,
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
                    opname: ComparisonOperator::GreaterOrEqual
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
        #[allow(clippy::cast_possible_truncation, reason = "checked right before")]
        let size = size as u32;

        // Step 2: If we have keys to match
        if size > 0 {
            // Validate and compile keys
            let mut seen = IndexSet::default();
            for key in keys {
                let is_attribute = matches!(key, ast::Expr::Attribute(_));
                let is_literal = matches!(
                    key,
                    ast::Expr::NumberLiteral(_)
                        | ast::Expr::StringLiteral(_)
                        | ast::Expr::BytesLiteral(_)
                        | ast::Expr::BooleanLiteral(_)
                        | ast::Expr::NoneLiteral(_)
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
        emit!(self, Instruction::BuildTuple { count: size });
        // Stack: [subject, keys_tuple]

        // Match keys
        emit!(self, Instruction::MatchKeys);
        // Stack: [subject, keys_tuple, values_or_none]
        pc.on_top += 2; // subject and keys_tuple are underneath

        // Check if match succeeded
        emit!(self, Instruction::Copy { i: 1 });
        // Stack: [subject, keys_tuple, values_tuple, values_tuple_copy]

        // Check if copy is None (consumes the copy like POP_JUMP_IF_NONE)
        self.emit_load_const(ConstantData::None);
        emit!(
            self,
            Instruction::IsOp {
                invert: Invert::Yes
            }
        );

        // Stack: [subject, keys_tuple, values_tuple, bool]
        self.jump_to_fail_pop(pc, JumpOp::PopJumpIfFalse)?;
        // Stack: [subject, keys_tuple, values_tuple]

        // Unpack values (the original values_tuple)
        emit!(self, Instruction::UnpackSequence { count: size });
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
            emit!(self, Instruction::BuildMap { count: 0 });
            // Stack: [subject, keys_tuple, {}]
            emit!(self, Instruction::Swap { i: 3 });
            // Stack: [{}, keys_tuple, subject]
            emit!(self, Instruction::DictUpdate { i: 2 });
            // Stack after DICT_UPDATE: [rest_dict, keys_tuple]
            // DICT_UPDATE consumes source (subject) and leaves dict in place

            // Unpack keys and delete from rest_dict
            emit!(self, Instruction::UnpackSequence { count: size });
            // Stack: [rest_dict, k1, k2, ..., kn] (if size==0, nothing pushed)

            // Delete each key from rest_dict (skipped when size==0)
            // while (size) { COPY(1 + size--); SWAP(2); DELETE_SUBSCR }
            let mut remaining = size;
            while remaining > 0 {
                // Copy rest_dict which is at position (1 + remaining) from TOS
                emit!(self, Instruction::Copy { i: 1 + remaining });
                // Stack: [rest_dict, k1, ..., kn, rest_dict]
                emit!(self, Instruction::Swap { i: 2 });
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
        p: &ast::PatternMatchOr,
        pc: &mut PatternContext,
    ) -> CompileResult<()> {
        // Ensure the pattern is a MatchOr.
        let end = self.new_block(); // Create a new jump target label.
        let size = p.patterns.len();
        if size <= 1 {
            return Err(self.error(CodegenErrorType::SyntaxError(
                "MatchOr requires at least 2 patterns".to_owned(),
            )));
        }

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
            emit!(self, Instruction::Copy { i: 1 });
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
                            self.set_source_range(alt.range());
                            for _ in 0..=i_stores {
                                self.pattern_helper_rotate(i_control + 1)?;
                            }
                        }
                    }
                }
            }
            // Emit a jump to the common end label and reset any failure jump targets.
            self.set_source_range(alt.range());
            emit!(self, PseudoInstruction::Jump { delta: end });
            self.set_source_range(alt.range());
            self.emit_and_reset_fail_pop(pc)?;
        }

        // Restore the original pattern context.
        *pc = old_pc;
        // Simulate Py_INCREF on pc.stores.
        pc.stores = pc.stores.clone();
        // In C, old_pc.fail_pop is set to NULL to avoid freeing it later.
        // In Rust, old_pc is a local clone, so we need not worry about that.

        // No alternative matched: pop the subject and fail.
        self.set_source_range(p.range());
        emit!(self, Instruction::PopTop);
        self.jump_to_fail_pop(pc, JumpOp::Jump)?;

        // Use the label "end".
        self.switch_to_block(end);

        // Adjust the final captures.
        let n_stores = control.as_ref().unwrap().len();
        let n_rots = n_stores + 1 + pc.on_top + pc.stores.len();
        for i in 0..n_stores {
            // Rotate the capture to its proper place.
            self.set_source_range(p.range());
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
        self.set_source_range(p.range());
        emit!(self, Instruction::PopTop);
        Ok(())
    }

    fn compile_pattern_sequence(
        &mut self,
        p: &ast::PatternMatchSequence,
        pc: &mut PatternContext,
    ) -> CompileResult<()> {
        // Ensure the pattern is a MatchSequence.
        let patterns = &p.patterns; // a slice of ast::Pattern
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
                    opname: ComparisonOperator::Equal
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
                    opname: ComparisonOperator::GreaterOrEqual
                }
            );
            self.jump_to_fail_pop(pc, JumpOp::PopJumpIfFalse)?;
        }

        // Whatever comes next should consume the subject.
        pc.on_top -= 1;
        if only_wildcard {
            // ast::Patterns like: [] / [_] / [_, _] / [*_] / [_, *_] / [_, _, *_] / etc.
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
        p: &ast::PatternMatchValue,
        pc: &mut PatternContext,
    ) -> CompileResult<()> {
        // Match CPython codegen_pattern_value(): compare, then normalize to bool
        // before the fail jump. Late IR folding will collapse COMPARE_OP+TO_BOOL
        // into COMPARE_OP bool(...) when applicable.
        self.compile_expression(&p.value)?;
        emit!(
            self,
            Instruction::CompareOp {
                opname: bytecode::ComparisonOperator::Equal
            }
        );
        emit!(self, Instruction::ToBool);
        self.jump_to_fail_pop(pc, JumpOp::PopJumpIfFalse)?;
        Ok(())
    }

    fn compile_pattern_singleton(
        &mut self,
        p: &ast::PatternMatchSingleton,
        pc: &mut PatternContext,
    ) -> CompileResult<()> {
        // Load the singleton constant value.
        self.emit_load_const(match p.value {
            ast::Singleton::None => ConstantData::None,
            ast::Singleton::False => ConstantData::Boolean { value: false },
            ast::Singleton::True => ConstantData::Boolean { value: true },
        });
        // Compare using the "Is" operator.
        emit!(self, Instruction::IsOp { invert: Invert::No });
        // Jump to the failure label if the comparison is false.
        self.jump_to_fail_pop(pc, JumpOp::PopJumpIfFalse)?;
        Ok(())
    }

    fn compile_pattern(
        &mut self,
        pattern_type: &ast::Pattern,
        pattern_context: &mut PatternContext,
    ) -> CompileResult<()> {
        let prev_source_range = self.current_source_range;
        self.set_source_range(pattern_type.range());
        let result = match &pattern_type {
            ast::Pattern::MatchValue(pattern_type) => {
                self.compile_pattern_value(pattern_type, pattern_context)
            }
            ast::Pattern::MatchSingleton(pattern_type) => {
                self.compile_pattern_singleton(pattern_type, pattern_context)
            }
            ast::Pattern::MatchSequence(pattern_type) => {
                self.compile_pattern_sequence(pattern_type, pattern_context)
            }
            ast::Pattern::MatchMapping(pattern_type) => {
                self.compile_pattern_mapping(pattern_type, pattern_context)
            }
            ast::Pattern::MatchClass(pattern_type) => {
                self.compile_pattern_class(pattern_type, pattern_context)
            }
            ast::Pattern::MatchStar(pattern_type) => {
                self.compile_pattern_star(pattern_type, pattern_context)
            }
            ast::Pattern::MatchAs(pattern_type) => {
                self.compile_pattern_as(pattern_type, pattern_context)
            }
            ast::Pattern::MatchOr(pattern_type) => {
                self.compile_pattern_or(pattern_type, pattern_context)
            }
        };
        self.set_source_range(prev_source_range);
        result
    }

    fn compile_match_inner(
        &mut self,
        subject: &ast::Expr,
        cases: &[ast::MatchCase],
        pattern_context: &mut PatternContext,
    ) -> CompileResult<()> {
        fn is_trailing_wildcard_default(pattern: &ast::Pattern) -> bool {
            match pattern {
                ast::Pattern::MatchAs(match_as) => {
                    match_as.pattern.is_none() && match_as.name.is_none()
                }
                _ => false,
            }
        }

        self.compile_expression(subject)?;
        let end = self.new_block();

        let num_cases = cases.len();
        assert!(num_cases > 0);
        let has_default =
            num_cases > 1 && is_trailing_wildcard_default(&cases.last().unwrap().pattern);

        let case_count = num_cases - if has_default { 1 } else { 0 };
        for (i, m) in cases.iter().enumerate().take(case_count) {
            // Only copy the subject if not on the last case
            if i != case_count - 1 {
                emit!(self, Instruction::Copy { i: 1 });
            }

            pattern_context.stores = Vec::with_capacity(1);
            pattern_context.allow_irrefutable = m.guard.is_some() || i == num_cases - 1;
            pattern_context.fail_pop.clear();
            pattern_context.on_top = 0;

            self.compile_pattern(&m.pattern, pattern_context)?;
            assert_eq!(pattern_context.on_top, 0);

            self.set_source_range(m.pattern.range());
            for name in &pattern_context.stores {
                self.compile_name(name, NameUsage::Store)?;
            }

            if let Some(ref guard) = m.guard {
                self.ensure_fail_pop(pattern_context, 0)?;
                self.compile_jump_if_inner(
                    guard,
                    false,
                    pattern_context.fail_pop[0],
                    Some(m.pattern.range()),
                )?;
            }

            if i != case_count - 1 {
                if let Some(first_stmt) = m.body.first() {
                    self.set_source_range(first_stmt.range());
                }
                if matches!(m.pattern, ast::Pattern::MatchOr(_)) {
                    emit!(self, Instruction::Nop);
                }
                emit!(self, Instruction::PopTop);
            }

            self.compile_statements(&m.body)?;
            emit!(self, PseudoInstruction::JumpNoInterrupt { delta: end });
            self.set_source_range(m.pattern.range());
            self.emit_and_reset_fail_pop(pattern_context)?;
        }

        if has_default {
            let m = &cases[num_cases - 1];
            if num_cases == 1 {
                emit!(self, Instruction::PopTop);
            } else if m.guard.is_none() {
                emit!(self, Instruction::Nop);
            }
            if let Some(ref guard) = m.guard {
                self.compile_jump_if(guard, false, end)?;
            }
            self.compile_statements(&m.body)?;
        }
        self.switch_to_block(end);
        Ok(())
    }

    fn compile_match(
        &mut self,
        subject: &ast::Expr,
        cases: &[ast::MatchCase],
    ) -> CompileResult<()> {
        self.enter_conditional_block();
        let mut pattern_context = PatternContext::new();
        self.compile_match_inner(subject, cases, &mut pattern_context)?;
        self.leave_conditional_block();
        Ok(())
    }

    /// [CPython `compiler_addcompare`](https://github.com/python/cpython/blob/627894459a84be3488a1789919679c997056a03c/Python/compile.c#L2880-L2924)
    fn compile_addcompare(&mut self, op: &ast::CmpOp) {
        use bytecode::ComparisonOperator::*;
        match op {
            ast::CmpOp::Eq => emit!(self, Instruction::CompareOp { opname: Equal }),
            ast::CmpOp::NotEq => emit!(self, Instruction::CompareOp { opname: NotEqual }),
            ast::CmpOp::Lt => emit!(self, Instruction::CompareOp { opname: Less }),
            ast::CmpOp::LtE => emit!(
                self,
                Instruction::CompareOp {
                    opname: LessOrEqual
                }
            ),
            ast::CmpOp::Gt => emit!(self, Instruction::CompareOp { opname: Greater }),
            ast::CmpOp::GtE => {
                emit!(
                    self,
                    Instruction::CompareOp {
                        opname: GreaterOrEqual
                    }
                )
            }
            ast::CmpOp::In => emit!(self, Instruction::ContainsOp { invert: Invert::No }),
            ast::CmpOp::NotIn => emit!(
                self,
                Instruction::ContainsOp {
                    invert: Invert::Yes
                }
            ),
            ast::CmpOp::Is => emit!(self, Instruction::IsOp { invert: Invert::No }),
            ast::CmpOp::IsNot => emit!(
                self,
                Instruction::IsOp {
                    invert: Invert::Yes
                }
            ),
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
        left: &ast::Expr,
        ops: &[ast::CmpOp],
        comparators: &[ast::Expr],
    ) -> CompileResult<()> {
        // Save the full Compare expression range for COMPARE_OP positions
        let compare_range = self.current_source_range;
        let (last_op, mid_ops) = ops.split_last().unwrap();
        let (last_comparator, mid_comparators) = comparators.split_last().unwrap();

        // initialize lhs outside of loop
        self.compile_expression(left)?;

        if mid_comparators.is_empty() {
            self.compile_expression(last_comparator)?;
            self.set_source_range(compare_range);
            self.compile_addcompare(last_op);

            return Ok(());
        }

        let cleanup = self.new_block();

        // for all comparisons except the last (as the last one doesn't need a conditional jump)
        for (op, comparator) in mid_ops.iter().zip(mid_comparators) {
            self.compile_expression(comparator)?;

            // store rhs for the next comparison in chain
            self.set_source_range(compare_range);
            emit!(self, Instruction::Swap { i: 2 });
            emit!(self, Instruction::Copy { i: 2 });

            self.compile_addcompare(op);

            // if comparison result is false, we break with this value; if true, try the next one.
            emit!(self, Instruction::Copy { i: 1 });
            emit!(self, Instruction::ToBool);
            emit!(self, Instruction::PopJumpIfFalse { delta: cleanup });
            emit!(self, Instruction::PopTop);
        }

        self.compile_expression(last_comparator)?;
        self.set_source_range(compare_range);
        self.compile_addcompare(last_op);

        let end = self.new_block();
        emit!(self, PseudoInstruction::Jump { delta: end });

        // early exit left us with stack: `rhs, comparison_result`. We need to clean up rhs.
        self.switch_to_block(cleanup);
        emit!(self, Instruction::Swap { i: 2 });
        emit!(self, Instruction::PopTop);

        self.switch_to_block(end);
        Ok(())
    }

    fn compile_jump_if_compare(
        &mut self,
        left: &ast::Expr,
        ops: &[ast::CmpOp],
        comparators: &[ast::Expr],
        condition: bool,
        target_block: BlockIdx,
    ) -> CompileResult<()> {
        let compare_range = self.current_source_range;
        let (last_op, mid_ops) = ops.split_last().unwrap();
        let (last_comparator, mid_comparators) = comparators.split_last().unwrap();

        self.compile_expression(left)?;

        if mid_comparators.is_empty() {
            self.compile_expression(last_comparator)?;
            self.set_source_range(compare_range);
            self.compile_addcompare(last_op);
            self.emit_pop_jump_by_condition(condition, target_block);
            return Ok(());
        }

        let cleanup = self.new_block();
        let end = self.new_block();

        for (op, comparator) in mid_ops.iter().zip(mid_comparators) {
            self.compile_expression(comparator)?;
            self.set_source_range(compare_range);
            emit!(self, Instruction::Swap { i: 2 });
            emit!(self, Instruction::Copy { i: 2 });
            self.compile_addcompare(op);
            emit!(self, Instruction::ToBool);
            emit!(self, Instruction::PopJumpIfFalse { delta: cleanup });
        }

        self.compile_expression(last_comparator)?;
        self.set_source_range(compare_range);
        self.compile_addcompare(last_op);
        emit!(self, Instruction::ToBool);
        self.emit_pop_jump_by_condition(condition, target_block);
        emit!(self, PseudoInstruction::Jump { delta: end });

        self.switch_to_block(cleanup);
        emit!(self, Instruction::PopTop);
        if !condition {
            emit!(
                self,
                PseudoInstruction::Jump {
                    delta: target_block
                }
            );
        }

        self.switch_to_block(end);
        Ok(())
    }

    fn emit_pop_jump_by_condition(&mut self, condition: bool, target_block: BlockIdx) {
        if condition {
            emit!(
                self,
                Instruction::PopJumpIfTrue {
                    delta: target_block
                }
            );
        } else {
            emit!(
                self,
                Instruction::PopJumpIfFalse {
                    delta: target_block,
                }
            );
        }
    }

    fn compile_annotation(&mut self, annotation: &ast::Expr) -> CompileResult<()> {
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
                ast::Expr::Starred(ast::ExprStarred { value, .. }) => {
                    // *args: *Ts (where Ts is a TypeVarTuple).
                    // Do [annotation_value] = [*Ts].
                    self.compile_expression(value)?;
                    emit!(self, Instruction::UnpackSequence { count: 1 });
                    Ok(())
                }
                _ => self.compile_expression(annotation),
            };

            self.in_annotation = was_in_annotation;
            result?;
        }
        Ok(())
    }

    fn compile_check_annotation_expression(&mut self, expression: &ast::Expr) -> CompileResult<()> {
        self.compile_expression(expression)?;
        emit!(self, Instruction::PopTop);
        Ok(())
    }

    fn compile_check_annotation_subscript(&mut self, expression: &ast::Expr) -> CompileResult<()> {
        match expression {
            ast::Expr::Slice(ast::ExprSlice {
                lower, upper, step, ..
            }) => {
                if let Some(lower) = lower {
                    self.compile_check_annotation_expression(lower)?;
                }
                if let Some(upper) = upper {
                    self.compile_check_annotation_expression(upper)?;
                }
                if let Some(step) = step {
                    self.compile_check_annotation_expression(step)?;
                }
            }
            ast::Expr::Tuple(ast::ExprTuple { elts, .. }) => {
                for element in elts {
                    self.compile_check_annotation_subscript(element)?;
                }
            }
            _ => self.compile_check_annotation_expression(expression)?,
        }
        Ok(())
    }

    fn compile_annotated_assign(
        &mut self,
        target: &ast::Expr,
        annotation: &ast::Expr,
        value: Option<&ast::Expr>,
        simple: bool,
    ) -> CompileResult<()> {
        // Perform the actual assignment first
        if let Some(value) = value {
            self.compile_expression(value)?;
            self.compile_store(target)?;
        }

        // If we have a simple name in module or class scope, store annotation
        if simple
            && !self.ctx.in_func()
            && let ast::Expr::Name(ast::ExprName { id, .. }) = target
        {
            if self.future_annotations {
                // PEP 563: Store stringified annotation directly to __annotations__
                // Compile annotation as string
                self.compile_annotation(annotation)?;
                // Load __annotations__
                let annotations_name = self.name("__annotations__");
                emit!(
                    self,
                    Instruction::LoadName {
                        namei: annotations_name
                    }
                );
                // Load the variable name
                self.emit_load_const(ConstantData::Str {
                    value: self.mangle(id.as_str()).into_owned().into(),
                });
                // Store: __annotations__[name] = annotation
                emit!(self, Instruction::StoreSubscr);
            } else {
                // PEP 649: Handle conditional annotations
                if self.current_symbol_table().has_conditional_annotations {
                    // Allocate an index for every annotation when has_conditional_annotations
                    // This keeps indices aligned with compile_module_annotate's enumeration
                    let code_info = self.current_code_info();
                    let annotation_index = code_info.next_conditional_annotation_index;
                    code_info.next_conditional_annotation_index += 1;

                    // Determine if this annotation is conditional
                    // Module and Class scopes both need all annotations tracked
                    let scope_type = self.current_symbol_table().typ;
                    let in_conditional_block = self.current_code_info().in_conditional_block > 0;
                    let is_conditional =
                        matches!(scope_type, CompilerScope::Module | CompilerScope::Class)
                            || in_conditional_block;

                    // Only add to __conditional_annotations__ set if actually conditional
                    if is_conditional {
                        self.load_name("__conditional_annotations__")?;
                        self.emit_load_const(ConstantData::Integer {
                            value: annotation_index.into(),
                        });
                        emit!(self, Instruction::SetAdd { i: 1 });
                        emit!(self, Instruction::PopTop);
                    }
                }
            }
        }

        if value.is_none() {
            match target {
                ast::Expr::Attribute(ast::ExprAttribute { value, .. }) => {
                    self.compile_check_annotation_expression(value)?;
                }
                ast::Expr::Subscript(ast::ExprSubscript { value, slice, .. }) => {
                    self.compile_check_annotation_expression(value)?;
                    self.compile_check_annotation_subscript(slice)?;
                }
                _ => {}
            }
        }

        Ok(())
    }

    fn compile_store(&mut self, target: &ast::Expr) -> CompileResult<()> {
        let prev_source_range = self.current_source_range;
        self.set_source_range(target.range());
        let result = (|| -> CompileResult<()> {
            match &target {
                ast::Expr::Name(ast::ExprName { id, .. }) => self.store_name(id.as_str())?,
                ast::Expr::Subscript(ast::ExprSubscript {
                    value, slice, ctx, ..
                }) => {
                    self.compile_subscript(value, slice, *ctx)?;
                }
                ast::Expr::Attribute(ast::ExprAttribute { value, attr, .. }) => {
                    self.maybe_add_static_attribute_to_class(value, attr.as_str());
                    self.compile_expression(value)?;
                    let namei = self.name(attr.as_str());
                    emit!(self, Instruction::StoreAttr { namei });
                }
                ast::Expr::List(ast::ExprList { elts, .. })
                | ast::Expr::Tuple(ast::ExprTuple { elts, .. }) => {
                    let mut seen_star = false;

                    // Scan for star args:
                    for (i, element) in elts.iter().enumerate() {
                        if let ast::Expr::Starred(_) = &element {
                            if seen_star {
                                return Err(self.error(CodegenErrorType::MultipleStarArgs));
                            } else {
                                seen_star = true;
                                let before = i;
                                let after = elts.len() - i - 1;
                                let (before, after) = (|| Some((before.to_u8()?, after.to_u8()?)))(
                                )
                                .ok_or_else(|| {
                                    self.error_ranged(
                                        CodegenErrorType::TooManyStarUnpack,
                                        target.range(),
                                    )
                                })?;
                                let counts = bytecode::UnpackExArgs { before, after };
                                emit!(self, Instruction::UnpackEx { counts });
                            }
                        }
                    }

                    if !seen_star {
                        emit!(
                            self,
                            Instruction::UnpackSequence {
                                count: elts.len().to_u32(),
                            }
                        );
                    }

                    for element in elts {
                        if let ast::Expr::Starred(ast::ExprStarred { value, .. }) = &element {
                            self.compile_store(value)?;
                        } else {
                            self.compile_store(element)?;
                        }
                    }
                }
                _ => {
                    return Err(self.error(match target {
                        ast::Expr::Starred(_) => CodegenErrorType::SyntaxError(
                            "starred assignment target must be in a list or tuple".to_owned(),
                        ),
                        _ => CodegenErrorType::Assign(target.python_name()),
                    }));
                }
            }
            Ok(())
        })();

        self.set_source_range(prev_source_range);
        result
    }

    fn compile_augassign(
        &mut self,
        target: &ast::Expr,
        op: &ast::Operator,
        value: &ast::Expr,
    ) -> CompileResult<()> {
        enum AugAssignKind<'a> {
            Name { id: &'a str },
            Subscript { use_slice_opt: bool },
            Attr { idx: bytecode::NameIdx },
        }

        let kind = match &target {
            ast::Expr::Name(ast::ExprName { id, .. }) => {
                let id = id.as_str();
                self.compile_name(id, NameUsage::Load)?;
                AugAssignKind::Name { id }
            }
            ast::Expr::Subscript(ast::ExprSubscript {
                value,
                slice,
                ctx: _,
                ..
            }) => {
                let use_slice_opt = slice.should_use_slice_optimization();
                self.compile_expression(value)?;
                if use_slice_opt {
                    let ast::Expr::Slice(slice_expr) = slice.as_ref() else {
                        unreachable!(
                            "should_use_slice_optimization should only return true for ast::Expr::Slice"
                        );
                    };
                    self.compile_slice_two_parts(slice_expr)?;
                    emit!(self, Instruction::Copy { i: 3 });
                    emit!(self, Instruction::Copy { i: 3 });
                    emit!(self, Instruction::Copy { i: 3 });
                    emit!(self, Instruction::BinarySlice);
                } else {
                    self.compile_expression(slice)?;
                    emit!(self, Instruction::Copy { i: 2 });
                    emit!(self, Instruction::Copy { i: 2 });
                    emit!(
                        self,
                        Instruction::BinaryOp {
                            op: BinaryOperator::Subscr
                        }
                    );
                }
                AugAssignKind::Subscript { use_slice_opt }
            }
            ast::Expr::Attribute(ast::ExprAttribute { value, attr, .. }) => {
                let attr = attr.as_str();
                self.compile_expression(value)?;
                emit!(self, Instruction::Copy { i: 1 });
                let idx = self.name(attr);
                self.emit_load_attr(idx);
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
            AugAssignKind::Subscript { use_slice_opt } => {
                if use_slice_opt {
                    // stack: CONTAINER START STOP RESULT
                    emit!(self, Instruction::Swap { i: 4 });
                    emit!(self, Instruction::Swap { i: 3 });
                    emit!(self, Instruction::Swap { i: 2 });
                    emit!(self, Instruction::StoreSlice);
                } else {
                    // stack: CONTAINER SLICE RESULT
                    emit!(self, Instruction::Swap { i: 3 });
                    emit!(self, Instruction::Swap { i: 2 });
                    emit!(self, Instruction::StoreSubscr);
                }
            }
            AugAssignKind::Attr { idx } => {
                // stack: CONTAINER RESULT
                emit!(self, Instruction::Swap { i: 2 });
                emit!(self, Instruction::StoreAttr { namei: idx });
            }
        }

        Ok(())
    }

    fn compile_op(&mut self, op: &ast::Operator, inplace: bool) {
        let bin_op = match op {
            ast::Operator::Add => BinaryOperator::Add,
            ast::Operator::Sub => BinaryOperator::Subtract,
            ast::Operator::Mult => BinaryOperator::Multiply,
            ast::Operator::MatMult => BinaryOperator::MatrixMultiply,
            ast::Operator::Div => BinaryOperator::TrueDivide,
            ast::Operator::FloorDiv => BinaryOperator::FloorDivide,
            ast::Operator::Mod => BinaryOperator::Remainder,
            ast::Operator::Pow => BinaryOperator::Power,
            ast::Operator::LShift => BinaryOperator::Lshift,
            ast::Operator::RShift => BinaryOperator::Rshift,
            ast::Operator::BitOr => BinaryOperator::Or,
            ast::Operator::BitXor => BinaryOperator::Xor,
            ast::Operator::BitAnd => BinaryOperator::And,
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
    fn compile_jump_if_inner(
        &mut self,
        expression: &ast::Expr,
        condition: bool,
        target_block: BlockIdx,
        source_range: Option<TextRange>,
    ) -> CompileResult<()> {
        let prev_source_range = self.current_source_range;
        self.set_source_range(source_range.unwrap_or_else(|| expression.range()));

        // Compile expression for test, and jump to label if false
        let result = match &expression {
            ast::Expr::BoolOp(ast::ExprBoolOp { op, values, .. }) => {
                let (last_value, prefix_values) = values.split_last().unwrap();
                let cond2 = matches!(op, ast::BoolOp::Or);
                let next2 = if cond2 != condition {
                    self.new_block()
                } else {
                    target_block
                };

                for value in prefix_values {
                    self.compile_jump_if_inner(value, cond2, next2, source_range)?;
                }
                self.compile_jump_if_inner(last_value, condition, target_block, source_range)?;

                if next2 != target_block {
                    self.switch_to_block(next2);
                }
                Ok(())
            }
            ast::Expr::UnaryOp(ast::ExprUnaryOp {
                op: ast::UnaryOp::Not,
                operand,
                ..
            }) => self.compile_jump_if_inner(operand, !condition, target_block, source_range),
            ast::Expr::If(ast::ExprIf {
                test, body, orelse, ..
            }) => {
                let end = self.new_block();
                let next2 = self.new_block();
                self.compile_jump_if_inner(test, false, next2, source_range)?;
                self.compile_jump_if_inner(body, condition, target_block, source_range)?;
                emit!(self, PseudoInstruction::JumpNoInterrupt { delta: end });
                self.set_no_location();

                self.switch_to_block(next2);
                self.compile_jump_if_inner(orelse, condition, target_block, source_range)?;

                self.switch_to_block(end);
                Ok(())
            }
            ast::Expr::Compare(ast::ExprCompare {
                left,
                ops,
                comparators,
                ..
            }) if ops.len() > 1 => {
                self.compile_jump_if_compare(left, ops, comparators, condition, target_block)
            }
            // `x is None` / `x is not None` → POP_JUMP_IF_NONE / POP_JUMP_IF_NOT_NONE
            ast::Expr::Compare(ast::ExprCompare {
                left,
                ops,
                comparators,
                ..
            }) if ops.len() == 1
                && matches!(ops[0], ast::CmpOp::Is | ast::CmpOp::IsNot)
                && comparators.len() == 1
                && matches!(&comparators[0], ast::Expr::NoneLiteral(_)) =>
            {
                self.compile_expression(left)?;
                let source = self.source_file.to_source_code();
                let comparator_line = source.line_index(comparators[0].range().start());
                let left_line = source.line_index(left.range().start());
                if comparator_line != left_line {
                    self.set_source_range(comparators[0].range());
                    emit!(self, Instruction::Nop);
                    self.set_source_range(source_range.unwrap_or_else(|| expression.range()));
                }
                let is_not = matches!(ops[0], ast::CmpOp::IsNot);
                // is None + jump_if_false → POP_JUMP_IF_NOT_NONE
                // is None + jump_if_true → POP_JUMP_IF_NONE
                // is not None + jump_if_false → POP_JUMP_IF_NONE
                // is not None + jump_if_true → POP_JUMP_IF_NOT_NONE
                let jump_if_none = condition != is_not;
                if jump_if_none {
                    emit!(
                        self,
                        Instruction::PopJumpIfNone {
                            delta: target_block,
                        }
                    );
                } else {
                    emit!(
                        self,
                        Instruction::PopJumpIfNotNone {
                            delta: target_block,
                        }
                    );
                }
                Ok(())
            }
            _ => {
                // Fall back case which always will work!
                self.compile_expression(expression)?;
                emit!(self, Instruction::ToBool);
                if condition {
                    emit!(
                        self,
                        Instruction::PopJumpIfTrue {
                            delta: target_block,
                        }
                    );
                } else {
                    emit!(
                        self,
                        Instruction::PopJumpIfFalse {
                            delta: target_block,
                        }
                    );
                }
                Ok(())
            }
        };

        self.set_source_range(prev_source_range);
        result
    }

    fn compile_jump_if(
        &mut self,
        expression: &ast::Expr,
        condition: bool,
        target_block: BlockIdx,
    ) -> CompileResult<()> {
        self.compile_jump_if_inner(expression, condition, target_block, None)
    }

    /// Compile a boolean operation as an expression.
    /// This means, that the last value remains on the stack.
    fn compile_bool_op(&mut self, op: &ast::BoolOp, values: &[ast::Expr]) -> CompileResult<()> {
        fn flatten_same_boolop_values<'a>(
            op: &ast::BoolOp,
            value: &'a ast::Expr,
            out: &mut Vec<&'a ast::Expr>,
        ) {
            if let ast::Expr::BoolOp(ast::ExprBoolOp {
                op: inner_op,
                values,
                ..
            }) = value
                && inner_op == op
            {
                for value in values {
                    flatten_same_boolop_values(op, value, out);
                }
            } else {
                out.push(value);
            }
        }

        let mut flattened = Vec::with_capacity(values.len());
        for value in values {
            flatten_same_boolop_values(op, value, &mut flattened);
        }

        let after_block = self.new_block();
        let (last_value, prefix_values) = flattened.split_last().unwrap();

        for value in prefix_values {
            let continue_block = self.new_block();
            self.compile_expression(value)?;
            self.emit_short_circuit_test(op, after_block);
            self.switch_to_block(continue_block);
            emit!(self, Instruction::PopTop);
        }

        self.compile_expression(last_value)?;
        self.switch_to_block(after_block);
        Ok(())
    }

    fn compile_bool_op_with_head_constant(
        &mut self,
        op: &ast::BoolOp,
        head: ConstantData,
        tail: &[ast::Expr],
    ) -> CompileResult<()> {
        self.emit_load_const(head);
        self.mark_last_instruction_folded_from_nonliteral_expr();
        if tail.is_empty() {
            return Ok(());
        }

        let after_block = self.new_block();
        for value in tail {
            self.emit_short_circuit_test(op, after_block);
            emit!(self, Instruction::PopTop);
            self.compile_expression(value)?;
        }
        self.switch_to_block(after_block);
        Ok(())
    }

    /// Emit `Copy 1` + conditional jump for short-circuit evaluation.
    /// For `And`, emits `PopJumpIfFalse`; for `Or`, emits `PopJumpIfTrue`.
    fn emit_short_circuit_test(&mut self, op: &ast::BoolOp, target: BlockIdx) {
        emit!(self, Instruction::Copy { i: 1 });
        emit!(self, Instruction::ToBool);
        match op {
            ast::BoolOp::And => {
                emit!(self, Instruction::PopJumpIfFalse { delta: target });
            }
            ast::BoolOp::Or => {
                emit!(self, Instruction::PopJumpIfTrue { delta: target });
            }
        }
    }

    fn compile_dict(&mut self, items: &[ast::DictItem]) -> CompileResult<()> {
        let has_unpacking = items.iter().any(|item| item.key.is_none());

        if !has_unpacking {
            // Match CPython's compiler_subdict chunking strategy:
            // - n≤15: BUILD_MAP n (all pairs on stack)
            // - n>15: BUILD_MAP 0 + MAP_ADD chunks of 17, last chunk uses
            //   BUILD_MAP n (if ≤15) or BUILD_MAP 0 + MAP_ADD
            const STACK_LIMIT: usize = 15;
            const BIG_MAP_CHUNK: usize = 17;

            if items.len() <= STACK_LIMIT {
                for item in items {
                    self.compile_expression(item.key.as_ref().unwrap())?;
                    self.compile_expression(&item.value)?;
                }
                emit!(
                    self,
                    Instruction::BuildMap {
                        count: u32::try_from(items.len()).expect("too many dict items"),
                    }
                );
            } else {
                // Split: leading full chunks of BIG_MAP_CHUNK via MAP_ADD,
                // remainder via BUILD_MAP n or MAP_ADD depending on size
                let n = items.len();
                let remainder = n % BIG_MAP_CHUNK;
                let n_big_chunks = n / BIG_MAP_CHUNK;
                // If remainder fits on stack (≤15), use BUILD_MAP n for it.
                // Otherwise it becomes another MAP_ADD chunk.
                let (big_count, tail_count) = if remainder > 0 && remainder <= STACK_LIMIT {
                    (n_big_chunks, remainder)
                } else {
                    // remainder is 0 or >15: all chunks are MAP_ADD chunks
                    let total_map_add = if remainder == 0 {
                        n_big_chunks
                    } else {
                        n_big_chunks + 1
                    };
                    (total_map_add, 0usize)
                };

                emit!(self, Instruction::BuildMap { count: 0 });

                let mut idx = 0;
                for chunk_i in 0..big_count {
                    if chunk_i > 0 {
                        emit!(self, Instruction::BuildMap { count: 0 });
                    }
                    let chunk_size = if idx + BIG_MAP_CHUNK <= n - tail_count {
                        BIG_MAP_CHUNK
                    } else {
                        n - tail_count - idx
                    };
                    for item in &items[idx..idx + chunk_size] {
                        self.compile_expression(item.key.as_ref().unwrap())?;
                        self.compile_expression(&item.value)?;
                        emit!(self, Instruction::MapAdd { i: 1 });
                    }
                    if chunk_i > 0 {
                        emit!(self, Instruction::DictUpdate { i: 1 });
                    }
                    idx += chunk_size;
                }

                // Tail: remaining pairs via BUILD_MAP n + DICT_UPDATE
                if tail_count > 0 {
                    for item in &items[idx..idx + tail_count] {
                        self.compile_expression(item.key.as_ref().unwrap())?;
                        self.compile_expression(&item.value)?;
                    }
                    emit!(
                        self,
                        Instruction::BuildMap {
                            count: tail_count.to_u32(),
                        }
                    );
                    emit!(self, Instruction::DictUpdate { i: 1 });
                }
            }
            return Ok(());
        }

        // Complex case with ** unpacking: preserve insertion order.
        // Collect runs of regular k:v pairs and emit BUILD_MAP + DICT_UPDATE
        // for each run, and DICT_UPDATE for each ** entry.
        let mut have_dict = false;
        let mut elements: u32 = 0;

        // Flush pending regular pairs as a BUILD_MAP, merging into the
        // accumulator dict via DICT_UPDATE when one already exists.
        macro_rules! flush_pending {
            () => {
                #[allow(unused_assignments)]
                if elements > 0 {
                    emit!(self, Instruction::BuildMap { count: elements });
                    if have_dict {
                        emit!(self, Instruction::DictUpdate { i: 1 });
                    } else {
                        have_dict = true;
                    }
                    elements = 0;
                }
            };
        }

        for item in items {
            if let Some(key) = &item.key {
                // Regular key: value pair
                self.compile_expression(key)?;
                self.compile_expression(&item.value)?;
                elements += 1;
            } else {
                // ** unpacking entry
                flush_pending!();
                if !have_dict {
                    emit!(self, Instruction::BuildMap { count: 0 });
                    have_dict = true;
                }
                self.compile_expression(&item.value)?;
                emit!(self, Instruction::DictUpdate { i: 1 });
            }
        }

        flush_pending!();
        if !have_dict {
            emit!(self, Instruction::BuildMap { count: 0 });
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
    ///     JUMP exit
    ///   exit:
    ///     END_SEND
    fn compile_yield_from_sequence(&mut self, is_await: bool) -> CompileResult<BlockIdx> {
        let send_block = self.new_block();
        let fail_block = self.new_block();
        let exit_block = self.new_block();

        // send:
        self.switch_to_block(send_block);
        emit!(self, Instruction::Send { delta: exit_block });

        // SETUP_FINALLY fail - set up exception handler for YIELD_VALUE
        emit!(self, PseudoInstruction::SetupFinally { delta: fail_block });
        self.push_fblock(
            FBlockType::TryExcept, // Use TryExcept for exception handler
            send_block,
            exit_block,
        )?;

        // YIELD_VALUE with arg=1 (yield-from/await mode - not wrapped for async gen)
        emit!(self, Instruction::YieldValue { arg: 1 });

        // POP_BLOCK before RESUME
        emit!(self, PseudoInstruction::PopBlock);
        self.pop_fblock(FBlockType::TryExcept);

        // RESUME
        emit!(
            self,
            Instruction::Resume {
                context: if is_await {
                    oparg::ResumeContext::from(oparg::ResumeLocation::AfterAwait)
                } else {
                    oparg::ResumeContext::from(oparg::ResumeLocation::AfterYieldFrom)
                }
            }
        );

        // JUMP_BACKWARD_NO_INTERRUPT send
        emit!(
            self,
            PseudoInstruction::JumpNoInterrupt { delta: send_block }
        );

        // fail: CLEANUP_THROW
        // Stack when exception: [receiver, yielded_value, exc]
        // CLEANUP_THROW: [sub_iter, last_sent_val, exc] -> [None, value]
        // CPython lets this block fall through to END_SEND during codegen;
        // push_cold_blocks_to_end later inserts the no-interrupt jump after
        // moving the cold fail block behind the warm exit path.
        self.switch_to_block(fail_block);
        emit!(self, Instruction::CleanupThrow);

        // exit: END_SEND
        // Stack: [receiver, value] (from SEND) or [None, value] (from CLEANUP_THROW)
        // END_SEND: [receiver/None, value] -> [value]
        self.switch_to_block(exit_block);
        emit!(self, Instruction::EndSend);

        Ok(send_block)
    }

    /// Returns true if the expression is a constant with no side effects.
    fn is_const_expression(expr: &ast::Expr) -> bool {
        matches!(
            expr,
            ast::Expr::StringLiteral(_)
                | ast::Expr::BytesLiteral(_)
                | ast::Expr::NumberLiteral(_)
                | ast::Expr::BooleanLiteral(_)
                | ast::Expr::NoneLiteral(_)
                | ast::Expr::EllipsisLiteral(_)
        ) || matches!(expr, ast::Expr::FString(fstring) if Self::fstring_value_is_const(&fstring.value))
    }

    fn fstring_value_is_const(fstring: &ast::FStringValue) -> bool {
        for part in fstring {
            if !Self::fstring_part_is_const(part) {
                return false;
            }
        }
        true
    }

    fn fstring_part_is_const(part: &ast::FStringPart) -> bool {
        match part {
            ast::FStringPart::Literal(_) => true,
            ast::FStringPart::FString(fstring) => fstring
                .elements
                .iter()
                .all(|element| matches!(element, ast::InterpolatedStringElement::Literal(_))),
        }
    }

    fn compile_expression(&mut self, expression: &ast::Expr) -> CompileResult<()> {
        trace!("Compiling {expression:?}");
        let range = expression.range();
        self.set_source_range(range);

        if let ast::Expr::Subscript(ast::ExprSubscript {
            ctx: ast::ExprContext::Load,
            ..
        }) = expression
            && let Some(constant) = self.try_fold_constant_expr(expression)?
        {
            self.emit_load_const(constant);
            return Ok(());
        }

        if matches!(expression, ast::Expr::BinOp(_))
            && let Some(constant) = self.try_fold_constant_expr(expression)?
        {
            self.emit_load_const(constant);
            return Ok(());
        }

        if !self.disable_const_boolop_folding
            && let ast::Expr::BoolOp(ast::ExprBoolOp { op, values, .. }) = expression
        {
            let mut simplified_prefix = 0usize;
            let mut last_constant = None;
            let mut retained_head = None;
            for value in values {
                let Some(constant) = self.try_fold_constant_expr(value)? else {
                    break;
                };
                if !Self::boolop_fast_fold_literal(value) {
                    retained_head = Some(constant);
                    simplified_prefix += 1;
                    break;
                }
                let is_truthy = Self::constant_truthiness(&constant);
                last_constant = Some(constant);
                match op {
                    ast::BoolOp::Or if is_truthy => {
                        self.emit_load_const(last_constant.expect("missing boolop constant"));
                        self.mark_last_instruction_folded_from_nonliteral_expr();
                        return Ok(());
                    }
                    ast::BoolOp::And if !is_truthy => {
                        self.emit_load_const(last_constant.expect("missing boolop constant"));
                        self.mark_last_instruction_folded_from_nonliteral_expr();
                        return Ok(());
                    }
                    ast::BoolOp::Or | ast::BoolOp::And => {
                        simplified_prefix += 1;
                    }
                }
            }

            if let Some(head) = retained_head {
                self.compile_bool_op_with_head_constant(op, head, &values[simplified_prefix..])?;
                return Ok(());
            }
            if simplified_prefix == values.len() {
                self.emit_load_const(last_constant.expect("missing folded boolop constant"));
                self.mark_last_instruction_folded_from_nonliteral_expr();
                return Ok(());
            }
            if simplified_prefix > 0 {
                let tail = &values[simplified_prefix..];
                if let [value] = tail {
                    self.compile_expression(value)?;
                } else {
                    self.compile_bool_op(op, tail)?;
                }
                self.mark_last_instruction_folded_from_nonliteral_expr();
                return Ok(());
            }
        }

        match &expression {
            ast::Expr::Call(ast::ExprCall {
                func, arguments, ..
            }) => self.compile_call(func, arguments)?,
            ast::Expr::BoolOp(ast::ExprBoolOp { op, values, .. }) => {
                self.compile_bool_op(op, values)?
            }
            ast::Expr::BinOp(ast::ExprBinOp {
                left, op, right, ..
            }) => {
                self.compile_expression(left)?;
                self.compile_expression(right)?;

                // Restore full expression range before emitting the operation
                self.set_source_range(range);
                self.compile_op(op, false);
            }
            ast::Expr::Subscript(ast::ExprSubscript {
                value, slice, ctx, ..
            }) => {
                self.compile_subscript(value, slice, *ctx)?;
            }
            ast::Expr::UnaryOp(ast::ExprUnaryOp { op, operand, .. }) => {
                self.compile_expression(operand)?;

                // Restore full expression range before emitting the operation
                self.set_source_range(range);
                match op {
                    ast::UnaryOp::UAdd => emit!(
                        self,
                        Instruction::CallIntrinsic1 {
                            func: bytecode::IntrinsicFunction1::UnaryPositive
                        }
                    ),
                    ast::UnaryOp::USub => emit!(self, Instruction::UnaryNegative),
                    ast::UnaryOp::Not => {
                        emit!(self, Instruction::ToBool);
                        emit!(self, Instruction::UnaryNot);
                    }
                    ast::UnaryOp::Invert => emit!(self, Instruction::UnaryInvert),
                };
            }
            ast::Expr::Attribute(ast::ExprAttribute { value, attr, .. }) => {
                // Check for super() attribute access optimization
                if let Some(super_type) = self.can_optimize_super_call(value, attr.as_str()) {
                    // super().attr or super(cls, self).attr optimization
                    // Stack: [global_super, class, self] → LOAD_SUPER_ATTR → [attr]
                    // Set source range to super() call for arg-loading instructions
                    let super_range = value.range();
                    self.set_source_range(super_range);
                    self.load_args_for_super(&super_type)?;
                    self.set_source_range(super_range);
                    let idx = self.name(attr.as_str());
                    match super_type {
                        SuperCallType::TwoArg { .. } => {
                            self.emit_load_super_attr(idx);
                        }
                        SuperCallType::ZeroArg => {
                            self.emit_load_zero_super_attr(idx);
                        }
                    }
                } else {
                    // Normal attribute access
                    self.compile_expression(value)?;
                    let idx = self.name(attr.as_str());
                    self.emit_load_attr(idx);
                }
            }
            ast::Expr::Compare(ast::ExprCompare {
                left,
                ops,
                comparators,
                ..
            }) => {
                self.compile_compare(left, ops, comparators)?;
            }
            // ast::Expr::Constant(ExprConstant { value, .. }) => {
            //     self.emit_load_const(compile_constant(value));
            // }
            ast::Expr::List(ast::ExprList { elts, .. }) => {
                self.starunpack_helper(elts, 0, CollectionType::List)?;
            }
            ast::Expr::Tuple(ast::ExprTuple { elts, .. }) => {
                self.starunpack_helper(elts, 0, CollectionType::Tuple)?;
            }
            ast::Expr::Set(ast::ExprSet { elts, .. }) => {
                self.starunpack_helper(elts, 0, CollectionType::Set)?;
            }
            ast::Expr::Dict(ast::ExprDict { items, .. }) => {
                self.compile_dict(items)?;
            }
            ast::Expr::Slice(ast::ExprSlice {
                lower, upper, step, ..
            }) => {
                if let Some(folded_const) = self.try_fold_constant_slice(
                    lower.as_deref(),
                    upper.as_deref(),
                    step.as_deref(),
                )? {
                    self.emit_load_const(folded_const);
                    return Ok(());
                }
                let mut compile_bound = |bound: Option<&ast::Expr>| match bound {
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
            ast::Expr::Yield(ast::ExprYield { value, .. }) => {
                if !self.ctx.in_func() {
                    return Err(self.error(CodegenErrorType::InvalidYield));
                }
                self.mark_generator();
                match value {
                    Some(expression) => self.compile_expression(expression)?,
                    Option::None => self.emit_load_const(ConstantData::None),
                };
                if self.ctx.func == FunctionContext::AsyncFunction {
                    emit!(
                        self,
                        Instruction::CallIntrinsic1 {
                            func: bytecode::IntrinsicFunction1::AsyncGenWrap
                        }
                    );
                }
                // arg=0: direct yield (wrapped for async generators)
                emit!(self, Instruction::YieldValue { arg: 0 });
                emit!(
                    self,
                    Instruction::Resume {
                        context: oparg::ResumeContext::from(oparg::ResumeLocation::AfterYield)
                    }
                );
            }
            ast::Expr::Await(ast::ExprAwait { value, .. }) => {
                if self.ctx.func != FunctionContext::AsyncFunction {
                    return Err(self.error(CodegenErrorType::InvalidAwait));
                }
                self.compile_expression(value)?;
                emit!(self, Instruction::GetAwaitable { r#where: 0 });
                self.emit_load_const(ConstantData::None);
                let _ = self.compile_yield_from_sequence(true)?;
            }
            ast::Expr::YieldFrom(ast::ExprYieldFrom { value, .. }) => {
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
                let _ = self.compile_yield_from_sequence(false)?;
            }
            ast::Expr::Name(ast::ExprName { id, .. }) => self.load_name(id.as_str())?,
            ast::Expr::Lambda(ast::ExprLambda {
                parameters, body, ..
            }) => {
                let default_params = ast::Parameters::default();
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
                    emit!(self, Instruction::BuildTuple { count: size });
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
                            value: self.mangle(arg.name.as_str()).into_owned().into(),
                        });
                        self.compile_expression(default)?;
                    }
                    emit!(
                        self,
                        Instruction::BuildMap {
                            count: default_kw_count.to_u32(),
                        }
                    );
                }

                self.enter_function(&name, params)?;
                let mut func_flags = bytecode::MakeFunctionFlags::new();
                if have_defaults {
                    func_flags.insert(bytecode::MakeFunctionFlag::Defaults);
                }
                if have_kwdefaults {
                    func_flags.insert(bytecode::MakeFunctionFlag::KwOnlyDefaults);
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

                // Lambda cannot have docstrings, so no None is added to co_consts

                self.compile_expression(body)?;
                self.emit_return_value();
                let code = self.exit_scope();

                // Create lambda function with closure
                self.make_closure(code, func_flags)?;

                self.ctx = prev_ctx;
            }
            ast::Expr::ListComp(ast::ExprListComp {
                elt,
                generators,
                range,
                ..
            }) => {
                self.compile_comprehension(
                    "<listcomp>",
                    Some(
                        Instruction::BuildList {
                            count: OpArgMarker::marker(),
                        }
                        .into(),
                    ),
                    generators,
                    &|compiler, collection_add_i| {
                        compiler.compile_comprehension_element(elt)?;
                        emit!(
                            compiler,
                            Instruction::ListAppend {
                                i: collection_add_i.to_u32(),
                            }
                        );
                        Ok(())
                    },
                    ComprehensionType::List,
                    Self::contains_await(elt) || Self::generators_contain_await(generators),
                    *range,
                )?;
            }
            ast::Expr::SetComp(ast::ExprSetComp {
                elt,
                generators,
                range,
                ..
            }) => {
                self.compile_comprehension(
                    "<setcomp>",
                    Some(
                        Instruction::BuildSet {
                            count: OpArgMarker::marker(),
                        }
                        .into(),
                    ),
                    generators,
                    &|compiler, collection_add_i| {
                        compiler.compile_comprehension_element(elt)?;
                        emit!(
                            compiler,
                            Instruction::SetAdd {
                                i: collection_add_i.to_u32(),
                            }
                        );
                        Ok(())
                    },
                    ComprehensionType::Set,
                    Self::contains_await(elt) || Self::generators_contain_await(generators),
                    *range,
                )?;
            }
            ast::Expr::DictComp(ast::ExprDictComp {
                key,
                value,
                generators,
                range,
                ..
            }) => {
                self.compile_comprehension(
                    "<dictcomp>",
                    Some(
                        Instruction::BuildMap {
                            count: OpArgMarker::marker(),
                        }
                        .into(),
                    ),
                    generators,
                    &|compiler, collection_add_i| {
                        // changed evaluation order for Py38 named expression PEP 572
                        compiler.compile_expression(key)?;
                        compiler.compile_expression(value)?;

                        emit!(
                            compiler,
                            Instruction::MapAdd {
                                i: collection_add_i.to_u32(),
                            }
                        );

                        Ok(())
                    },
                    ComprehensionType::Dict,
                    Self::contains_await(key)
                        || Self::contains_await(value)
                        || Self::generators_contain_await(generators),
                    *range,
                )?;
            }
            ast::Expr::Generator(ast::ExprGenerator {
                elt,
                generators,
                range,
                ..
            }) => {
                // Check if element or generators contain async content
                // This makes the generator expression into an async generator
                let element_contains_await =
                    Self::contains_await(elt) || Self::generators_contain_await(generators);
                self.compile_comprehension(
                    "<genexpr>",
                    None,
                    generators,
                    &|compiler, _collection_add_i| {
                        // Compile the element expression
                        // Note: if element is an async comprehension, compile_expression
                        // already handles awaiting it, so we don't need to await again here
                        compiler.compile_comprehension_element(elt)?;

                        compiler.mark_generator();
                        if compiler.ctx.func == FunctionContext::AsyncFunction {
                            emit!(
                                compiler,
                                Instruction::CallIntrinsic1 {
                                    func: bytecode::IntrinsicFunction1::AsyncGenWrap
                                }
                            );
                        }
                        // arg=0: direct yield (wrapped for async generators)
                        emit!(compiler, Instruction::YieldValue { arg: 0 });
                        emit!(
                            compiler,
                            Instruction::Resume {
                                context: oparg::ResumeContext::from(
                                    oparg::ResumeLocation::AfterYield
                                )
                            }
                        );
                        emit!(compiler, Instruction::PopTop);

                        Ok(())
                    },
                    ComprehensionType::Generator,
                    element_contains_await,
                    *range,
                )?;
            }
            ast::Expr::Starred(ast::ExprStarred { value, .. }) => {
                if self.in_annotation {
                    // In annotation context, starred expressions are allowed (PEP 646)
                    // For now, just compile the inner value without wrapping with Unpack
                    // This is a temporary solution until we figure out how to properly import typing
                    self.compile_expression(value)?;
                } else {
                    return Err(self.error(CodegenErrorType::InvalidStarExpr));
                }
            }
            ast::Expr::If(ast::ExprIf {
                test, body, orelse, ..
            }) => {
                let folded_test_truthiness = self
                    .try_fold_constant_expr(test)?
                    .as_ref()
                    .map(Self::constant_truthiness);
                let else_block = self.new_block();
                let after_block = self.new_block();
                self.compile_jump_if(test, false, else_block)?;

                // True case
                self.compile_expression(body)?;
                if folded_test_truthiness == Some(true) {
                    self.mark_last_instruction_folded_from_nonliteral_expr();
                }
                emit!(
                    self,
                    PseudoInstruction::JumpNoInterrupt { delta: after_block }
                );
                self.set_no_location();

                // False case
                self.switch_to_block(else_block);
                self.compile_expression(orelse)?;
                if folded_test_truthiness == Some(false) {
                    self.mark_last_instruction_folded_from_nonliteral_expr();
                }

                // End
                self.switch_to_block(after_block);
            }

            ast::Expr::Named(ast::ExprNamed {
                target,
                value,
                node_index: _,
                range: _,
            }) => {
                // Walrus targets in inlined comps should NOT be hidden from locals()
                if self.current_code_info().in_inlined_comp
                    && let ast::Expr::Name(ast::ExprName { id, .. }) = target.as_ref()
                {
                    let name = self.mangle(id.as_str());
                    let info = self.code_stack.last_mut().unwrap();
                    info.metadata.fast_hidden.insert(name.to_string(), false);
                    info.metadata.fast_hidden_final.swap_remove(name.as_ref());
                }
                self.compile_expression(value)?;
                emit!(self, Instruction::Copy { i: 1 });
                self.compile_store(target)?;
            }
            ast::Expr::FString(fstring) => {
                self.compile_expr_fstring(fstring)?;
            }
            ast::Expr::TString(tstring) => {
                self.compile_expr_tstring(tstring)?;
            }
            ast::Expr::StringLiteral(string) => {
                let value = self.compile_string_value(string);
                self.emit_load_const(ConstantData::Str { value });
            }
            ast::Expr::BytesLiteral(bytes) => {
                let iter = bytes.value.iter().flat_map(|x| x.iter().copied());
                let v: Vec<u8> = iter.collect();
                self.emit_load_const(ConstantData::Bytes { value: v });
            }
            ast::Expr::NumberLiteral(number) => match &number.value {
                ast::Number::Int(int) => {
                    let value = ruff_int_to_bigint(int).map_err(|e| self.error(e))?;
                    self.emit_load_const(ConstantData::Integer { value });
                }
                ast::Number::Float(float) => {
                    self.emit_load_const(ConstantData::Float { value: *float });
                }
                ast::Number::Complex { real, imag } => {
                    self.emit_load_const(ConstantData::Complex {
                        value: Complex::new(*real, *imag),
                    });
                }
            },
            ast::Expr::BooleanLiteral(b) => {
                self.emit_load_const(ConstantData::Boolean { value: b.value });
            }
            ast::Expr::NoneLiteral(_) => {
                self.emit_load_const(ConstantData::None);
            }
            ast::Expr::EllipsisLiteral(_) => {
                self.emit_load_const(ConstantData::Ellipsis);
            }
            ast::Expr::IpyEscapeCommand(_) => {
                panic!("unexpected ipy escape command");
            }
        }
        Ok(())
    }

    fn compile_keywords(&mut self, keywords: &[ast::Keyword]) -> CompileResult<()> {
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
                emit!(self, Instruction::BuildMap { count: sub_size });
                size += 1;
            }
        }
        if size > 1 {
            // Merge all dicts: first dict is accumulator, merge rest into it
            for _ in 1..size {
                emit!(self, Instruction::DictMerge { i: 1 });
            }
        }
        Ok(())
    }

    fn detect_builtin_generator_call(
        &self,
        func: &ast::Expr,
        args: &ast::Arguments,
    ) -> Option<BuiltinGeneratorCallKind> {
        let ast::Expr::Name(ast::ExprName { id, .. }) = func else {
            return None;
        };
        if args.args.len() != 1
            || !args.keywords.is_empty()
            || !matches!(args.args[0], ast::Expr::Generator(_))
        {
            return None;
        }
        match id.as_str() {
            "tuple" => Some(BuiltinGeneratorCallKind::Tuple),
            "all" => Some(BuiltinGeneratorCallKind::All),
            "any" => Some(BuiltinGeneratorCallKind::Any),
            _ => None,
        }
    }

    /// Emit the optimized inline loop for builtin(genexpr) calls.
    ///
    /// Stack on entry: `[func]` where `func` is the builtin candidate.
    /// On return the compiler is positioned at the fallback block so the
    /// normal call path can compile the original generator argument again.
    fn optimize_builtin_generator_call(
        &mut self,
        kind: BuiltinGeneratorCallKind,
        generator_expr: &ast::Expr,
        end: BlockIdx,
    ) -> CompileResult<()> {
        let common_constant = match kind {
            BuiltinGeneratorCallKind::Tuple => bytecode::CommonConstant::BuiltinTuple,
            BuiltinGeneratorCallKind::All => bytecode::CommonConstant::BuiltinAll,
            BuiltinGeneratorCallKind::Any => bytecode::CommonConstant::BuiltinAny,
        };

        let fallback = self.new_block();
        let loop_block = self.new_block();
        let cleanup = self.new_block();

        // Stack: [func] — copy function for identity check
        emit!(self, Instruction::Copy { i: 1 });
        emit!(
            self,
            Instruction::LoadCommonConstant {
                idx: common_constant
            }
        );
        emit!(self, Instruction::IsOp { invert: Invert::No });
        emit!(self, Instruction::PopJumpIfFalse { delta: fallback });
        emit!(self, Instruction::PopTop);

        if matches!(kind, BuiltinGeneratorCallKind::Tuple) {
            emit!(self, Instruction::BuildList { count: 0 });
        }

        let sub_table_cursor = self.symbol_table_stack.last().map(|t| t.next_sub_table);
        self.compile_expression(generator_expr)?;
        if let Some(cursor) = sub_table_cursor
            && let Some(current_table) = self.symbol_table_stack.last_mut()
        {
            current_table.next_sub_table = cursor;
        }
        self.switch_to_block(loop_block);
        emit!(self, Instruction::ForIter { delta: cleanup });

        match kind {
            BuiltinGeneratorCallKind::Tuple => {
                emit!(self, Instruction::ListAppend { i: 2 });
                emit!(self, PseudoInstruction::Jump { delta: loop_block });
            }
            BuiltinGeneratorCallKind::All => {
                emit!(self, Instruction::ToBool);
                emit!(self, Instruction::PopJumpIfTrue { delta: loop_block });
                emit!(self, Instruction::PopIter);
                self.emit_load_const(ConstantData::Boolean { value: false });
                emit!(self, PseudoInstruction::Jump { delta: end });
            }
            BuiltinGeneratorCallKind::Any => {
                emit!(self, Instruction::ToBool);
                emit!(self, Instruction::PopJumpIfFalse { delta: loop_block });
                emit!(self, Instruction::PopIter);
                self.emit_load_const(ConstantData::Boolean { value: true });
                emit!(self, PseudoInstruction::Jump { delta: end });
            }
        }

        self.switch_to_block(cleanup);
        emit!(self, Instruction::EndFor);
        emit!(self, Instruction::PopIter);
        match kind {
            BuiltinGeneratorCallKind::Tuple => {
                emit!(
                    self,
                    Instruction::CallIntrinsic1 {
                        func: IntrinsicFunction1::ListToTuple
                    }
                );
            }
            BuiltinGeneratorCallKind::All => {
                self.emit_load_const(ConstantData::Boolean { value: true });
            }
            BuiltinGeneratorCallKind::Any => {
                self.emit_load_const(ConstantData::Boolean { value: false });
            }
        }
        emit!(self, PseudoInstruction::Jump { delta: end });

        self.switch_to_block(fallback);
        Ok(())
    }

    fn compile_call(&mut self, func: &ast::Expr, args: &ast::Arguments) -> CompileResult<()> {
        // Save the call expression's source range so CALL instructions use the
        // call start line, not the last argument's line.
        let call_range = self.current_source_range;
        let uses_ex_call = self.call_uses_ex_call(args);

        // Method call: obj → LOAD_ATTR_METHOD → [method, self_or_null] → args → CALL
        // Regular call: func → PUSH_NULL → args → CALL
        if let ast::Expr::Attribute(ast::ExprAttribute { value, attr, .. }) = &func {
            // Check for super() method call optimization
            if let Some(super_type) = self.can_optimize_super_call(value, attr.as_str()) {
                // super().method() or super(cls, self).method() optimization
                // CALL path: [global_super, class, self] → LOAD_SUPER_METHOD → [method, self]
                // CALL_FUNCTION_EX path: [global_super, class, self] → LOAD_SUPER_ATTR → [attr]
                // Set source range to the super() call for LOAD_GLOBAL/LOAD_DEREF/etc.
                let super_range = value.range();
                self.set_source_range(super_range);
                self.load_args_for_super(&super_type)?;
                self.set_source_range(super_range);
                let idx = self.name(attr.as_str());
                if uses_ex_call {
                    match super_type {
                        SuperCallType::TwoArg { .. } => {
                            self.emit_load_super_attr(idx);
                        }
                        SuperCallType::ZeroArg => {
                            self.emit_load_zero_super_attr(idx);
                        }
                    }
                    emit!(self, Instruction::PushNull);
                    self.codegen_call_helper(0, args, call_range)?;
                } else {
                    match super_type {
                        SuperCallType::TwoArg { .. } => {
                            self.emit_load_super_method(idx);
                        }
                        SuperCallType::ZeroArg => {
                            self.emit_load_zero_super_method(idx);
                        }
                    }
                    // NOP for line tracking at .method( line
                    self.set_source_range(attr.range());
                    emit!(self, Instruction::Nop);
                    // CALL at .method( line (not the full expression line)
                    self.codegen_call_helper(0, args, attr.range())?;
                }
            } else {
                self.compile_expression(value)?;
                let idx = self.name(attr.as_str());
                // Imported names and CALL_FUNCTION_EX-style calls use plain
                // LOAD_ATTR + PUSH_NULL; other names use method-call mode.
                // Check current scope and enclosing scopes for IMPORTED flag.
                let is_import = matches!(value.as_ref(), ast::Expr::Name(ast::ExprName { id, .. })
                    if self.is_name_imported(id.as_str()));
                if is_import || uses_ex_call {
                    self.emit_load_attr(idx);
                    emit!(self, Instruction::PushNull);
                } else {
                    self.emit_load_attr_method(idx);
                }
                self.codegen_call_helper(0, args, call_range)?;
            }
        } else if let Some(kind) = (!uses_ex_call)
            .then(|| self.detect_builtin_generator_call(func, args))
            .flatten()
        {
            let end = self.new_block();
            self.compile_expression(func)?;
            self.optimize_builtin_generator_call(kind, &args.args[0], end)?;
            self.set_source_range(call_range);
            emit!(self, Instruction::PushNull);
            self.codegen_call_helper(0, args, call_range)?;
            self.switch_to_block(end);
        } else {
            // Regular call: push func, then NULL for self_or_null slot
            // Stack layout: [func, NULL, args...] - same as method call [func, self, args...]
            self.compile_expression(func)?;
            emit!(self, Instruction::PushNull);
            self.codegen_call_helper(0, args, call_range)?;
        }
        Ok(())
    }

    fn call_uses_ex_call(&self, arguments: &ast::Arguments) -> bool {
        let has_starred = arguments
            .args
            .iter()
            .any(|arg| matches!(arg, ast::Expr::Starred(_)));
        let has_double_star = arguments.keywords.iter().any(|k| k.arg.is_none());
        let too_big =
            arguments.args.len() + arguments.keywords.len() * 2 > STACK_USE_GUIDELINE as usize;
        has_starred || has_double_star || too_big
    }

    /// Compile subkwargs: emit key-value pairs for BUILD_MAP
    fn codegen_subkwargs(
        &mut self,
        keywords: &[ast::Keyword],
        begin: usize,
        end: usize,
    ) -> CompileResult<()> {
        let n = end - begin;
        assert!(n > 0);

        // For large kwargs, use BUILD_MAP(0) + MAP_ADD to avoid stack overflow.
        let big = n * 2 > STACK_USE_GUIDELINE as usize;

        if big {
            emit!(self, Instruction::BuildMap { count: 0 });
        }

        for kw in &keywords[begin..end] {
            // Key first, then value - this is critical!
            self.emit_load_const(ConstantData::Str {
                value: kw.arg.as_ref().unwrap().as_str().into(),
            });
            self.compile_expression(&kw.value)?;

            if big {
                emit!(self, Instruction::MapAdd { i: 1 });
            }
        }

        if !big {
            emit!(self, Instruction::BuildMap { count: n.to_u32() });
        }

        Ok(())
    }

    /// Compile call arguments and emit the appropriate CALL instruction.
    /// `call_range` is the source range of the call expression, used to set
    /// the correct line number on the CALL instruction.
    fn codegen_call_helper(
        &mut self,
        additional_positional: u32,
        arguments: &ast::Arguments,
        call_range: TextRange,
    ) -> CompileResult<()> {
        let nelts = arguments.args.len();
        let nkwelts = arguments.keywords.len();

        // Check if we have starred args or **kwargs
        let has_starred = arguments
            .args
            .iter()
            .any(|arg| matches!(arg, ast::Expr::Starred(_)));
        let has_double_star = arguments.keywords.iter().any(|k| k.arg.is_none());

        // Check if exceeds CPython's stack-use guideline.
        // With CALL_KW, kwargs values go on stack but keys go in a const tuple,
        // so stack usage is: func + null + positional_args + kwarg_values + kwnames_tuple
        let too_big = nelts + nkwelts * 2 > STACK_USE_GUIDELINE as usize;

        if !has_starred && !has_double_star && !too_big {
            // Simple call path: no * or ** args
            for arg in &arguments.args {
                self.compile_expression(arg)?;
            }

            if nkwelts > 0 {
                // Compile keyword values and build kwnames tuple
                let mut kwarg_names = Vec::with_capacity(nkwelts);
                for keyword in &arguments.keywords {
                    kwarg_names.push(ConstantData::Str {
                        value: keyword.arg.as_ref().unwrap().as_str().into(),
                    });
                    self.compile_expression(&keyword.value)?;
                }

                // Restore call expression range for kwnames and CALL_KW
                self.set_source_range(call_range);
                self.emit_load_const(ConstantData::Tuple {
                    elements: kwarg_names,
                });

                let argc = additional_positional + nelts.to_u32() + nkwelts.to_u32();
                emit!(self, Instruction::CallKw { argc });
            } else {
                self.set_source_range(call_range);
                let argc = additional_positional + nelts.to_u32();
                emit!(self, Instruction::Call { argc });
            }
        } else {
            // ex_call path: has * or ** args

            // Compile positional arguments
            if additional_positional == 0
                && nelts == 1
                && matches!(arguments.args[0], ast::Expr::Starred(_))
            {
                // Single starred arg: pass value directly to CallFunctionEx.
                // Runtime will convert to tuple and validate with function name.
                if let ast::Expr::Starred(ast::ExprStarred { value, .. }) = &arguments.args[0] {
                    self.compile_expression_without_const_boolop_folding(value)?;
                }
            } else if !has_starred {
                for arg in &arguments.args {
                    self.compile_expression(arg)?;
                }
                self.set_source_range(call_range);
                let positional_count = additional_positional + nelts.to_u32();
                if positional_count == 0 {
                    self.emit_load_const(ConstantData::Tuple { elements: vec![] });
                } else {
                    emit!(
                        self,
                        Instruction::BuildTuple {
                            count: positional_count
                        }
                    );
                }
            } else {
                // Use starunpack_helper to build a list, then convert to tuple
                self.starunpack_helper(
                    &arguments.args,
                    additional_positional,
                    CollectionType::List,
                )?;
                emit!(
                    self,
                    Instruction::CallIntrinsic1 {
                        func: IntrinsicFunction1::ListToTuple
                    }
                );
            }

            // Compile keyword arguments
            if nkwelts > 0 {
                let mut have_dict = false;
                let mut nseen = 0usize;

                for (i, keyword) in arguments.keywords.iter().enumerate() {
                    if keyword.arg.is_none() {
                        // **kwargs unpacking
                        if nseen > 0 {
                            // Pack up preceding keywords using codegen_subkwargs
                            self.codegen_subkwargs(&arguments.keywords, i - nseen, i)?;
                            if have_dict {
                                emit!(self, Instruction::DictMerge { i: 1 });
                            }
                            have_dict = true;
                            nseen = 0;
                        }

                        if !have_dict {
                            emit!(self, Instruction::BuildMap { count: 0 });
                            have_dict = true;
                        }

                        self.compile_expression_without_const_boolop_folding(&keyword.value)?;
                        emit!(self, Instruction::DictMerge { i: 1 });
                    } else {
                        nseen += 1;
                    }
                }

                // Pack up any trailing keyword arguments
                if nseen > 0 {
                    self.codegen_subkwargs(&arguments.keywords, nkwelts - nseen, nkwelts)?;
                    if have_dict {
                        emit!(self, Instruction::DictMerge { i: 1 });
                    }
                    have_dict = true;
                }

                assert!(have_dict);
            } else {
                emit!(self, Instruction::PushNull);
            }

            self.set_source_range(call_range);
            emit!(self, Instruction::CallFunctionEx);
        }

        Ok(())
    }

    fn compile_comprehension_element(&mut self, element: &ast::Expr) -> CompileResult<()> {
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

    fn consume_next_sub_table(&mut self) -> CompileResult<()> {
        {
            let _ = self.push_symbol_table()?;
        }
        let _ = self.pop_symbol_table();
        Ok(())
    }

    fn consume_skipped_nested_scopes_in_expr(
        &mut self,
        expression: &ast::Expr,
    ) -> CompileResult<()> {
        use ast::visitor::Visitor;

        struct SkippedScopeVisitor<'a> {
            compiler: &'a mut Compiler,
            error: Option<CodegenError>,
        }

        impl SkippedScopeVisitor<'_> {
            fn consume_scope(&mut self) {
                if self.error.is_none() {
                    self.error = self.compiler.consume_next_sub_table().err();
                }
            }
        }

        impl ast::visitor::Visitor<'_> for SkippedScopeVisitor<'_> {
            fn visit_expr(&mut self, expr: &ast::Expr) {
                if self.error.is_some() {
                    return;
                }

                match expr {
                    ast::Expr::Lambda(ast::ExprLambda { parameters, .. }) => {
                        // Defaults are scanned before enter_scope in the
                        // symbol table builder, so their nested scopes
                        // precede the lambda scope in sub_tables.
                        if let Some(params) = parameters.as_deref() {
                            for default in params
                                .posonlyargs
                                .iter()
                                .chain(&params.args)
                                .chain(&params.kwonlyargs)
                                .filter_map(|p| p.default.as_deref())
                            {
                                self.visit_expr(default);
                            }
                        }
                        self.consume_scope();
                    }
                    ast::Expr::ListComp(ast::ExprListComp { generators, .. })
                    | ast::Expr::SetComp(ast::ExprSetComp { generators, .. })
                    | ast::Expr::Generator(ast::ExprGenerator { generators, .. }) => {
                        if let Some(first) = generators.first() {
                            self.visit_expr(&first.iter);
                        }
                        self.consume_scope();
                    }
                    ast::Expr::DictComp(ast::ExprDictComp { generators, .. }) => {
                        if let Some(first) = generators.first() {
                            self.visit_expr(&first.iter);
                        }
                        self.consume_scope();
                    }
                    _ => ast::visitor::walk_expr(self, expr),
                }
            }
        }

        let mut visitor = SkippedScopeVisitor {
            compiler: self,
            error: None,
        };
        visitor.visit_expr(expression);
        if let Some(err) = visitor.error {
            Err(err)
        } else {
            Ok(())
        }
    }

    fn peek_next_sub_table_after_skipped_nested_scopes_in_expr(
        &mut self,
        expression: &ast::Expr,
    ) -> CompileResult<SymbolTable> {
        let saved_cursor = self
            .symbol_table_stack
            .last()
            .expect("no current symbol table")
            .next_sub_table;
        let result = (|| {
            self.consume_skipped_nested_scopes_in_expr(expression)?;
            let current_table = self
                .symbol_table_stack
                .last()
                .expect("no current symbol table");
            if let Some(table) = current_table.sub_tables.get(current_table.next_sub_table) {
                Ok(table.clone())
            } else {
                let name = current_table.name.clone();
                let typ = current_table.typ;
                Err(self.error(CodegenErrorType::SyntaxError(format!(
                    "no symbol table available in {} (type: {:?})",
                    name, typ
                ))))
            }
        })();
        self.symbol_table_stack
            .last_mut()
            .expect("no current symbol table")
            .next_sub_table = saved_cursor;
        result
    }

    fn push_output_with_symbol_table(
        &mut self,
        table: SymbolTable,
        flags: bytecode::CodeFlags,
        posonlyarg_count: u32,
        arg_count: u32,
        kwonlyarg_count: u32,
        obj_name: String,
    ) -> CompileResult<()> {
        let scope_type = table.typ;
        self.symbol_table_stack.push(table);

        let key = self.symbol_table_stack.len() - 1;
        let lineno = self.get_source_line_number().get();
        self.enter_scope(&obj_name, scope_type, key, lineno.to_u32())?;

        if let Some(info) = self.code_stack.last_mut() {
            info.flags = flags | (info.flags & bytecode::CodeFlags::NESTED);
            info.metadata.argcount = arg_count;
            info.metadata.posonlyargcount = posonlyarg_count;
            info.metadata.kwonlyargcount = kwonlyarg_count;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn compile_comprehension(
        &mut self,
        name: &str,
        init_collection: Option<AnyInstruction>,
        generators: &[ast::Comprehension],
        compile_element: &dyn Fn(&mut Self, usize) -> CompileResult<()>,
        comprehension_type: ComprehensionType,
        element_contains_await: bool,
        comprehension_range: TextRange,
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
        let outermost = &generators[0];
        let comp_table =
            self.peek_next_sub_table_after_skipped_nested_scopes_in_expr(&outermost.iter)?;

        let is_inlined = self.is_inlined_comprehension_context(comprehension_type, &comp_table);

        if is_inlined {
            // CPython inlines every non-generator comprehension that the
            // symtable marked as comp_inlined, including async variants.
            // codegen_comprehension() only branches on ste_comp_inlined here
            // and relies on the inlined path itself to handle GET_AITER /
            // async-comprehension cleanup.
            return self.compile_inlined_comprehension(
                comp_table,
                init_collection,
                generators,
                compile_element,
                has_an_async_gen,
                comprehension_range,
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

        // The symbol table follows CPython's symtable walk: nested scopes
        // in the outermost iterator are recorded before the comprehension
        // scope itself. Peek past those nested scopes so we can enter the
        // correct comprehension table here, then let the real outermost
        // iterator compile consume its nested scopes later in parent scope.
        self.push_output_with_symbol_table(comp_table, flags, 1, 1, 0, name.to_owned())?;

        // Set qualname for comprehension
        self.set_qualname();

        let arg0 = self.varname(".0")?;

        let return_none = init_collection.is_none();

        // PEP 479: Wrap generator/coroutine body with StopIteration handler
        let is_gen_scope = self.current_symbol_table().is_generator || is_async;
        let stop_iteration_block = if is_gen_scope {
            let handler_block = self.new_block();
            emit!(
                self,
                PseudoInstruction::SetupCleanup {
                    delta: handler_block
                }
            );
            self.set_no_location();
            self.push_fblock(FBlockType::StopIteration, handler_block, handler_block)?;
            Some(handler_block)
        } else {
            None
        };

        // Create empty object of proper type:
        if let Some(init_collection) = init_collection {
            self._emit(init_collection, OpArg::new(0), BlockIdx::NULL)
        }

        let mut loop_labels = vec![];
        let mut real_loop_depth = 0;
        for (gen_index, generator) in generators.iter().enumerate() {
            if gen_index > 0
                && !generator.is_async
                && let Some(singleton_iter) =
                    Self::singleton_comprehension_assignment_iter(&generator.iter)
            {
                self.compile_expression(singleton_iter)?;
                self.compile_store(&generator.target)?;

                if !generator.ifs.is_empty() {
                    let if_cleanup_block = self.new_block();
                    for if_condition in &generator.ifs {
                        self.compile_jump_if(if_condition, false, if_cleanup_block)?;
                    }
                    let body_block = self.new_block();
                    self.switch_to_block(body_block);
                    loop_labels.push(ComprehensionLoopControl::IfCleanupOnly { if_cleanup_block });
                }
                continue;
            }

            let loop_block = self.new_block();
            let if_cleanup_block = self.new_block();
            let after_block = self.new_block();

            if gen_index == 0 {
                // Load iterator onto stack (passed as first argument):
                emit!(self, Instruction::LoadFast { var_num: arg0 });
            } else {
                // Evaluate iterated item:
                self.compile_for_iterable_expression(&generator.iter, generator.is_async)?;

                // Get iterator / turn item into an iterator
                if generator.is_async {
                    emit!(self, Instruction::GetAIter);
                } else {
                    emit!(self, Instruction::GetIter);
                }
            }

            self.switch_to_block(loop_block);
            let mut end_async_for_target = BlockIdx::NULL;
            if generator.is_async {
                emit!(self, PseudoInstruction::SetupFinally { delta: after_block });
                emit!(self, Instruction::GetANext);
                self.push_fblock(
                    FBlockType::AsyncComprehensionGenerator,
                    loop_block,
                    after_block,
                )?;
                self.emit_load_const(ConstantData::None);
                end_async_for_target = self.compile_yield_from_sequence(true)?;
                // POP_BLOCK before store: only __anext__/yield_from are
                // protected by SetupFinally targeting END_ASYNC_FOR.
                emit!(self, PseudoInstruction::PopBlock);
                self.pop_fblock(FBlockType::AsyncComprehensionGenerator);
                self.compile_store(&generator.target)?;
            } else {
                emit!(self, Instruction::ForIter { delta: after_block });
                self.compile_store(&generator.target)?;
            }
            real_loop_depth += 1;
            loop_labels.push(ComprehensionLoopControl::Iteration {
                loop_block,
                if_cleanup_block,
                after_block,
                is_async: generator.is_async,
                end_async_for_target,
            });

            // CPython always lowers comprehension guards through codegen_jump_if
            // and leaves constant-folding to later CFG optimization passes.
            for if_condition in &generator.ifs {
                self.compile_jump_if(if_condition, false, if_cleanup_block)?;
            }
            if !generator.ifs.is_empty() {
                let body_block = self.new_block();
                self.switch_to_block(body_block);
            }
        }

        compile_element(self, real_loop_depth + 1)?;

        for loop_control in loop_labels.iter().rev().copied() {
            match loop_control {
                ComprehensionLoopControl::Iteration {
                    loop_block,
                    if_cleanup_block,
                    after_block,
                    is_async,
                    end_async_for_target,
                } => {
                    emit!(self, PseudoInstruction::Jump { delta: loop_block });

                    self.switch_to_block(if_cleanup_block);
                    emit!(self, PseudoInstruction::Jump { delta: loop_block });

                    self.switch_to_block(after_block);
                    if is_async {
                        // EndAsyncFor pops both the exception and the aiter
                        // (handler depth is before GetANext, so aiter is at handler depth)
                        self.emit_end_async_for(end_async_for_target);
                    } else {
                        // END_FOR + POP_ITER pattern (CPython 3.14)
                        emit!(self, Instruction::EndFor);
                        emit!(self, Instruction::PopIter);
                    }
                }
                ComprehensionLoopControl::IfCleanupOnly { if_cleanup_block } => {
                    self.switch_to_block(if_cleanup_block);
                }
            }
        }

        if return_none {
            self.emit_load_const(ConstantData::None)
        }

        self.emit_return_value();

        // Close StopIteration handler and emit handler code
        if let Some(handler_block) = stop_iteration_block {
            emit!(self, PseudoInstruction::PopBlock);
            self.set_no_location();
            self.pop_fblock(FBlockType::StopIteration);
            self.switch_to_block(handler_block);
            emit!(
                self,
                Instruction::CallIntrinsic1 {
                    func: oparg::IntrinsicFunction1::StopIterationError
                }
            );
            self.set_no_location();
            emit!(self, Instruction::Reraise { depth: 1u32 });
            self.set_no_location();
        }

        let code = self.exit_scope();

        self.ctx = prev_ctx;

        // Create comprehension function with closure
        self.make_closure(code, bytecode::MakeFunctionFlags::new())?;

        // Evaluate iterated item:
        self.compile_for_iterable_expression(&outermost.iter, outermost.is_async)?;
        self.symbol_table_stack
            .last_mut()
            .expect("no current symbol table")
            .next_sub_table += 1;

        // Get iterator / turn item into an iterator
        // Use is_async from the first generator, not has_an_async_gen which covers ALL generators
        if outermost.is_async {
            emit!(self, Instruction::GetAIter);
        } else {
            emit!(self, Instruction::GetIter);
        };

        // Call just created <listcomp> function:
        emit!(self, Instruction::Call { argc: 0 });
        if is_async_list_set_dict_comprehension {
            emit!(self, Instruction::GetAwaitable { r#where: 0 });
            self.emit_load_const(ConstantData::None);
            let _ = self.compile_yield_from_sequence(true)?;
        }

        Ok(())
    }

    /// Compile an inlined comprehension (PEP 709)
    /// This generates bytecode inline without creating a new code object
    fn compile_inlined_comprehension(
        &mut self,
        comp_table: SymbolTable,
        init_collection: Option<AnyInstruction>,
        generators: &[ast::Comprehension],
        compile_element: &dyn Fn(&mut Self, usize) -> CompileResult<()>,
        has_async: bool,
        comprehension_range: TextRange,
    ) -> CompileResult<()> {
        fn collect_bound_names(target: &ast::Expr, out: &mut Vec<String>) {
            match target {
                ast::Expr::Name(ast::ExprName { id, .. }) => out.push(id.to_string()),
                ast::Expr::Tuple(ast::ExprTuple { elts, .. })
                | ast::Expr::List(ast::ExprList { elts, .. }) => {
                    for elt in elts {
                        collect_bound_names(elt, out);
                    }
                }
                ast::Expr::Starred(ast::ExprStarred { value, .. }) => {
                    collect_bound_names(value, out);
                }
                _ => {}
            }
        }

        // Compile the outermost iterator first. Its expression may reference
        // nested scopes (e.g. lambdas) whose sub_tables sit at the current
        // position in the parent's list. Those must be consumed before we
        // splice in the comprehension's own children.
        self.compile_for_iterable_expression(
            &generators[0].iter,
            has_async && generators[0].is_async,
        )?;
        self.symbol_table_stack
            .last_mut()
            .expect("no current symbol table")
            .next_sub_table += 1;

        let was_in_inlined_comp = self.current_code_info().in_inlined_comp;
        let saved_source_range = self.current_source_range;
        let in_class_block = {
            let ct = self.current_symbol_table();
            ct.typ == CompilerScope::Class && !was_in_inlined_comp
        };
        self.current_code_info().in_inlined_comp = true;

        let mut temp_symbols: IndexMap<String, Symbol> = IndexMap::default();
        let mut changed_fast_hidden = Vec::new();

        let result = (|| {
            // Splice the comprehension's children (e.g. nested inlined
            // comprehensions) into the parent so the compiler can find them.
            if !comp_table.sub_tables.is_empty() {
                let current_table = self
                    .symbol_table_stack
                    .last_mut()
                    .expect("no current symbol table");
                let insert_pos = current_table.next_sub_table;
                for (i, st) in comp_table.sub_tables.iter().enumerate() {
                    current_table.sub_tables.insert(insert_pos + i, st.clone());
                }
            }
            if has_async && generators[0].is_async {
                emit!(self, Instruction::GetAIter);
            } else {
                emit!(self, Instruction::GetIter);
            }

            let mut source_order_bound_names = Vec::new();
            for generator in generators {
                collect_bound_names(&generator.target, &mut source_order_bound_names);
            }

            let mut pushed_locals: Vec<String> = Vec::new();
            for name in source_order_bound_names
                .into_iter()
                .chain(comp_table.symbols.keys().cloned())
            {
                if pushed_locals.iter().any(|existing| existing == &name) {
                    continue;
                }
                if let Some(sym) = comp_table.symbols.get(&name) {
                    if sym.flags.contains(SymbolFlags::PARAMETER) {
                        continue; // skip .0
                    }
                    // Walrus operator targets (ASSIGNED_IN_COMPREHENSION without ITER)
                    // are not local to the comprehension; they leak to the outer scope.
                    let is_walrus = sym.flags.contains(SymbolFlags::ASSIGNED_IN_COMPREHENSION)
                        && !sym.flags.contains(SymbolFlags::ITER);
                    let is_local = sym
                        .flags
                        .intersects(SymbolFlags::ASSIGNED | SymbolFlags::ITER)
                        && !sym.flags.contains(SymbolFlags::NONLOCAL)
                        && !is_walrus;
                    if is_local {
                        pushed_locals.push(name);
                    }
                }
            }

            // TweakInlinedComprehensionScopes: temporarily override parent
            // symbols with comprehension scopes where they differ. For
            // module/class scopes, also enable temporary fast locals for
            // comprehension-bound names only.
            for (name, comp_sym) in &comp_table.symbols {
                if comp_sym.flags.contains(SymbolFlags::PARAMETER) {
                    continue; // skip .0
                }
                let comp_scope = comp_sym.scope;

                let current_table = self.symbol_table_stack.last().expect("no symbol table");
                if let Some(outer_sym) = current_table.symbols.get(name) {
                    let outer_scope = outer_sym.scope;
                    if (comp_scope != outer_scope
                        && comp_scope != SymbolScope::Free
                        && !(comp_scope == SymbolScope::Cell && outer_scope == SymbolScope::Free))
                        || in_class_block
                    {
                        temp_symbols.insert(name.clone(), outer_sym.clone());
                        let current_table =
                            self.symbol_table_stack.last_mut().expect("no symbol table");
                        current_table.symbols.insert(name.clone(), comp_sym.clone());
                    }
                }
            }
            if !self.ctx.in_func() {
                for name in &pushed_locals {
                    if self
                        .current_code_info()
                        .metadata
                        .fast_hidden
                        .get(name.as_str())
                        .is_none_or(|&hidden| !hidden)
                    {
                        self.current_code_info()
                            .metadata
                            .fast_hidden
                            .insert(name.clone(), true);
                        self.current_code_info()
                            .metadata
                            .fast_hidden_final
                            .insert(name.clone());
                        changed_fast_hidden.push(name.clone());
                    }
                }
            }

            // Step 2: Save local variables that will be shadowed by the comprehension.
            // For each variable, we push the fast local value via LoadFastAndClear.
            // For merged CELL variables, LoadFastAndClear saves the cell object from
            // the merged slot, and MAKE_CELL creates a new empty cell in-place.
            // MAKE_CELL has no stack effect (operates only on fastlocals).
            self.set_source_range(comprehension_range);
            let mut total_stack_items: usize = 0;
            for name in &pushed_locals {
                let var_num = self.varname(name)?;
                emit!(self, Instruction::LoadFastAndClear { var_num });
                total_stack_items += 1;
                // If the comp symbol is CELL, emit MAKE_CELL to create fresh cell
                if let Some(comp_sym) = comp_table.symbols.get(name)
                    && comp_sym.scope == SymbolScope::Cell
                {
                    let i = if self
                        .current_symbol_table()
                        .symbols
                        .get(name)
                        .is_some_and(|s| s.scope == SymbolScope::Free)
                    {
                        self.get_free_var_index(name)?
                    } else {
                        self.get_cell_var_index(name)?
                    };
                    emit!(self, Instruction::MakeCell { i });
                }
            }

            // Step 3: SWAP iterator to TOS (above saved locals + cell values)
            if total_stack_items > 0 {
                emit!(
                    self,
                    Instruction::Swap {
                        i: u32::try_from(total_stack_items + 1).unwrap()
                    }
                );
            }

            // Step 4: Create the collection (list/set/dict)
            if let Some(init_collection) = init_collection {
                self._emit(init_collection, OpArg::new(0), BlockIdx::NULL);
                // SWAP to get iterator on top
                emit!(self, Instruction::Swap { i: 2 });
            }

            // Set up exception handler for cleanup on exception
            let cleanup_block = self.new_block();
            let end_block = self.new_block();

            if !pushed_locals.is_empty() {
                emit!(
                    self,
                    PseudoInstruction::SetupFinally {
                        delta: cleanup_block
                    }
                );
                self.push_fblock(FBlockType::TryExcept, cleanup_block, end_block)?;
            }

            // Step 5: Compile the comprehension loop(s)
            let mut loop_labels: Vec<ComprehensionLoopControl> = vec![];
            let mut real_loop_depth = 0;
            for (i, generator) in generators.iter().enumerate() {
                if i > 0
                    && !generator.is_async
                    && let Some(singleton_iter) =
                        Self::singleton_comprehension_assignment_iter(&generator.iter)
                {
                    self.compile_expression(singleton_iter)?;
                    self.compile_store(&generator.target)?;

                    if !generator.ifs.is_empty() {
                        let if_cleanup_block = self.new_block();
                        for if_condition in &generator.ifs {
                            self.compile_jump_if(if_condition, false, if_cleanup_block)?;
                        }
                        let body_block = self.new_block();
                        self.switch_to_block(body_block);
                        loop_labels
                            .push(ComprehensionLoopControl::IfCleanupOnly { if_cleanup_block });
                    }
                    continue;
                }

                let loop_block = self.new_block();
                let if_cleanup_block = self.new_block();
                let after_block = self.new_block();

                if i > 0 {
                    self.compile_for_iterable_expression(&generator.iter, generator.is_async)?;
                    if generator.is_async {
                        emit!(self, Instruction::GetAIter);
                    } else {
                        emit!(self, Instruction::GetIter);
                    }
                }

                self.switch_to_block(loop_block);

                let mut end_async_for_target = BlockIdx::NULL;
                if generator.is_async {
                    emit!(self, PseudoInstruction::SetupFinally { delta: after_block });
                    emit!(self, Instruction::GetANext);
                    self.push_fblock(
                        FBlockType::AsyncComprehensionGenerator,
                        loop_block,
                        after_block,
                    )?;
                    self.emit_load_const(ConstantData::None);
                    end_async_for_target = self.compile_yield_from_sequence(true)?;
                    emit!(self, PseudoInstruction::PopBlock);
                    self.pop_fblock(FBlockType::AsyncComprehensionGenerator);
                    self.compile_store(&generator.target)?;
                } else {
                    let saved_range = self.current_source_range;
                    self.set_source_range(generator.iter.range());
                    emit!(self, Instruction::ForIter { delta: after_block });
                    self.set_source_range(saved_range);
                    self.compile_store(&generator.target)?;
                }

                real_loop_depth += 1;
                loop_labels.push(ComprehensionLoopControl::Iteration {
                    loop_block,
                    if_cleanup_block,
                    after_block,
                    is_async: generator.is_async,
                    end_async_for_target,
                });

                // CPython always lowers comprehension guards through codegen_jump_if
                // and leaves constant-folding to later CFG optimization passes.
                for if_condition in &generator.ifs {
                    self.compile_jump_if(if_condition, false, if_cleanup_block)?;
                }
            }

            // Step 6: Compile the element expression and append to collection
            compile_element(self, real_loop_depth + 1)?;

            // Step 7: Close all loops
            for loop_control in loop_labels.iter().rev().copied() {
                match loop_control {
                    ComprehensionLoopControl::Iteration {
                        loop_block,
                        if_cleanup_block,
                        after_block,
                        is_async,
                        end_async_for_target,
                    } => {
                        emit!(self, PseudoInstruction::Jump { delta: loop_block });

                        self.switch_to_block(if_cleanup_block);
                        emit!(self, PseudoInstruction::Jump { delta: loop_block });

                        self.switch_to_block(after_block);
                        if is_async {
                            self.emit_end_async_for(end_async_for_target);
                        } else {
                            emit!(self, Instruction::EndFor);
                            emit!(self, Instruction::PopIter);
                        }
                    }
                    ComprehensionLoopControl::IfCleanupOnly { if_cleanup_block } => {
                        self.switch_to_block(if_cleanup_block);
                    }
                }
            }

            // Step 8: Clean up - restore saved locals (and cell values)
            self.set_source_range(comprehension_range);
            if total_stack_items > 0 {
                emit!(self, PseudoInstruction::PopBlock);
                self.pop_fblock(FBlockType::TryExcept);

                // Match CPython codegen_pop_inlined_comprehension_locals():
                // the synthetic jump that skips the exception cleanup uses
                // JUMP_NO_INTERRUPT, which becomes JUMP_BACKWARD_NO_INTERRUPT
                // when the cleanup tail sits above the final restore block.
                emit!(
                    self,
                    PseudoInstruction::JumpNoInterrupt { delta: end_block }
                );

                // Exception cleanup path
                self.switch_to_block(cleanup_block);
                // Stack: [saved_values..., collection, exception]
                emit!(self, Instruction::Swap { i: 2 });
                emit!(self, Instruction::PopTop); // Pop incomplete collection

                // Restore locals and cell values
                emit!(
                    self,
                    Instruction::Swap {
                        i: u32::try_from(total_stack_items + 1).unwrap()
                    }
                );
                for name in pushed_locals.iter().rev() {
                    let var_num = self.varname(name)?.as_u32();
                    emit!(self, PseudoInstruction::StoreFastMaybeNull { var_num });
                }
                // Re-raise the exception
                emit!(self, Instruction::Reraise { depth: 0 });

                // Normal end path
                self.switch_to_block(end_block);
            }

            // SWAP result to TOS (above saved values)
            if total_stack_items > 0 {
                emit!(
                    self,
                    Instruction::Swap {
                        i: u32::try_from(total_stack_items + 1).unwrap()
                    }
                );
            }

            // Restore saved locals (StoreFast restores the saved cell object for merged cells)
            for name in pushed_locals.iter().rev() {
                let var_num = self.varname(name)?.as_u32();
                emit!(self, PseudoInstruction::StoreFastMaybeNull { var_num });
            }
            self.set_source_range(saved_source_range);

            Ok(())
        })();

        let current_table = self.symbol_table_stack.last_mut().expect("no symbol table");
        for (name, original_sym) in temp_symbols {
            current_table.symbols.insert(name, original_sym);
        }
        for name in changed_fast_hidden {
            self.current_code_info()
                .metadata
                .fast_hidden
                .insert(name, false);
        }
        self.current_code_info().in_inlined_comp = was_in_inlined_comp;

        result
    }

    fn compile_future_features(&mut self, features: &[ast::Alias]) -> Result<(), CodegenError> {
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
    fn _emit<I: Into<AnyInstruction>>(&mut self, instr: I, arg: OpArg, target: BlockIdx) {
        if self.do_not_emit_bytecode > 0 {
            return;
        }
        let range = self.current_source_range;
        let source = self.source_file.to_source_code();
        let location = source.source_location(range.start(), PositionEncoding::Utf8);
        let end_location = source.source_location(range.end(), PositionEncoding::Utf8);
        let except_handler = None;
        self.current_block().instructions.push(ir::InstructionInfo {
            instr: instr.into(),
            arg,
            target,
            location,
            end_location,
            except_handler,
            folded_from_nonliteral_expr: false,
            lineno_override: None,
            cache_entries: 0,
            preserve_redundant_jump_as_nop: false,
            remove_no_location_nop: false,
            preserve_block_start_no_location_nop: false,
        });
    }

    fn mark_last_instruction_folded_from_nonliteral_expr(&mut self) {
        if let Some(info) = self.current_block().instructions.last_mut() {
            info.folded_from_nonliteral_expr = true;
        }
    }

    fn preserve_last_redundant_jump_as_nop(&mut self) {
        if let Some(info) = self.current_block().instructions.last_mut() {
            info.preserve_redundant_jump_as_nop = true;
        }
    }

    fn preserve_last_redundant_nop(&mut self) {
        if let Some(info) = self.current_block().instructions.last_mut() {
            info.preserve_block_start_no_location_nop = true;
        }
    }

    fn current_block_has_terminal_with_suppress_exit_predecessor(&self) -> bool {
        let code = self.code_stack.last().expect("no code on stack");
        let target = code.current_block;
        let mut has_suppress_exit = false;
        let mut has_normal_exit = false;

        for block in &code.blocks {
            let Some((last, prefix)) = block.instructions.split_last() else {
                continue;
            };
            if last.target != target {
                continue;
            }
            match last.instr.pseudo() {
                Some(PseudoInstruction::JumpNoInterrupt { .. }) => {
                    let real_instrs: Vec<_> =
                        prefix.iter().filter_map(|info| info.instr.real()).collect();
                    has_suppress_exit |= matches!(
                        real_instrs.as_slice(),
                        [
                            Instruction::PopTop,
                            Instruction::PopExcept,
                            Instruction::PopTop,
                            Instruction::PopTop,
                            Instruction::PopTop,
                        ]
                    );
                }
                Some(PseudoInstruction::Jump { .. }) => {
                    has_normal_exit |= !prefix.iter().any(|info| info.instr.is_scope_exit());
                }
                _ => {}
            }
        }

        has_suppress_exit && !has_normal_exit
    }

    fn remove_last_no_location_nop(&mut self) {
        if let Some(info) = self.current_block().instructions.last_mut() {
            info.remove_no_location_nop = true;
        }
    }

    /// Mark the last emitted instruction as having no source location.
    /// Prevents it from triggering LINE events in sys.monitoring.
    fn set_no_location(&mut self) {
        if let Some(last) = self.current_block().instructions.last_mut() {
            last.lineno_override = Some(-1);
        }
    }

    fn emit_no_arg<I: Into<AnyInstruction>>(&mut self, ins: I) {
        self._emit(ins, OpArg::NULL, BlockIdx::NULL)
    }

    fn emit_arg<A: OpArgType, T: EmitArg<A>, I: Into<AnyInstruction>>(
        &mut self,
        arg: T,
        f: impl FnOnce(OpArgMarker<A>) -> I,
    ) {
        let (op, arg, target) = arg.emit(f);
        self._emit(op, arg, target)
    }

    // fn block_done()

    /// Convert a string literal AST node to Wtf8Buf, handling surrogate literals correctly.
    fn compile_string_value(&self, string: &ast::ExprStringLiteral) -> Wtf8Buf {
        let value = string.value.to_str();
        if value.contains(char::REPLACEMENT_CHARACTER) {
            // Might have a surrogate literal; reparse from source to preserve them.
            string
                .value
                .iter()
                .map(|lit| {
                    let source = self.source_file.slice(lit.range);
                    crate::string_parser::parse_string_literal(source, lit.flags.into())
                })
                .collect()
        } else {
            value.into()
        }
    }

    fn compile_fstring_literal_value(
        &self,
        string: &ast::InterpolatedStringLiteralElement,
        flags: ast::FStringFlags,
    ) -> Wtf8Buf {
        if string.value.contains(char::REPLACEMENT_CHARACTER) {
            let source = self.source_file.slice(string.range);
            crate::string_parser::parse_fstring_literal_element(source.into(), flags.into()).into()
        } else {
            string.value.to_string().into()
        }
    }

    fn compile_tstring_literal_value(
        &self,
        string: &ast::InterpolatedStringLiteralElement,
        flags: ast::TStringFlags,
    ) -> Wtf8Buf {
        if string.value.contains(char::REPLACEMENT_CHARACTER) {
            let source = self.source_file.slice(string.range);
            crate::string_parser::parse_fstring_literal_element(source.into(), flags.into()).into()
        } else {
            string.value.to_string().into()
        }
    }

    fn compile_fstring_part_literal_value(&self, string: &ast::StringLiteral) -> Wtf8Buf {
        if string.value.contains(char::REPLACEMENT_CHARACTER) {
            let source = self.source_file.slice(string.range);
            crate::string_parser::parse_string_literal(source, string.flags.into()).into()
        } else {
            string.value.to_string().into()
        }
    }

    fn arg_constant(&mut self, constant: ConstantData) -> oparg::ConstIdx {
        let info = self.current_code_info();
        if let ConstantData::Code { code } = &constant
            && let Some(idx) = info.metadata.consts.iter().position(|existing| {
                matches!(
                    existing,
                    ConstantData::Code {
                        code: existing_code
                    } if Self::code_objects_equivalent(existing_code, code)
                )
            })
        {
            return u32::try_from(idx)
                .expect("constant table index overflow")
                .into();
        }
        info.metadata.consts.insert_full(constant).0.to_u32().into()
    }

    fn constants_equivalent(lhs: &ConstantData, rhs: &ConstantData) -> bool {
        match (lhs, rhs) {
            (ConstantData::Code { code: lhs }, ConstantData::Code { code: rhs }) => {
                Self::code_objects_equivalent(lhs, rhs)
            }
            (ConstantData::Tuple { elements: lhs }, ConstantData::Tuple { elements: rhs })
            | (
                ConstantData::Frozenset { elements: lhs },
                ConstantData::Frozenset { elements: rhs },
            ) => {
                lhs.len() == rhs.len()
                    && lhs
                        .iter()
                        .zip(rhs.iter())
                        .all(|(lhs, rhs)| Self::constants_equivalent(lhs, rhs))
            }
            (ConstantData::Slice { elements: lhs }, ConstantData::Slice { elements: rhs }) => lhs
                .iter()
                .zip(rhs.iter())
                .all(|(lhs, rhs)| Self::constants_equivalent(lhs, rhs)),
            _ => lhs == rhs,
        }
    }

    fn code_objects_equivalent(lhs: &bytecode::CodeObject, rhs: &bytecode::CodeObject) -> bool {
        lhs.instructions.len() == rhs.instructions.len()
            && lhs
                .instructions
                .iter()
                .zip(rhs.instructions.iter())
                .all(|(lhs, rhs)| u8::from(lhs.op) == u8::from(rhs.op) && lhs.arg == rhs.arg)
            && lhs.locations == rhs.locations
            && lhs.flags.bits() == rhs.flags.bits()
            && lhs.posonlyarg_count == rhs.posonlyarg_count
            && lhs.arg_count == rhs.arg_count
            && lhs.kwonlyarg_count == rhs.kwonlyarg_count
            && lhs.source_path == rhs.source_path
            && lhs.first_line_number == rhs.first_line_number
            && lhs.max_stackdepth == rhs.max_stackdepth
            && lhs.obj_name == rhs.obj_name
            && lhs.qualname == rhs.qualname
            && lhs.constants.len() == rhs.constants.len()
            && lhs
                .constants
                .iter()
                .zip(rhs.constants.iter())
                .all(|(lhs, rhs)| Self::constants_equivalent(lhs, rhs))
            && lhs.names == rhs.names
            && lhs.varnames == rhs.varnames
            && lhs.cellvars == rhs.cellvars
            && lhs.freevars == rhs.freevars
            && lhs.localspluskinds == rhs.localspluskinds
            && lhs.linetable == rhs.linetable
            && lhs.exceptiontable == rhs.exceptiontable
    }

    /// Try to fold a collection of constant expressions into a single ConstantData::Tuple.
    /// Returns None if any element cannot be folded.
    fn try_fold_constant_collection(
        &mut self,
        elts: &[ast::Expr],
        collection_type: CollectionType,
    ) -> CompileResult<Option<ConstantData>> {
        let mut constants = Vec::with_capacity(elts.len());
        for elt in elts {
            let Some(constant) = self.try_fold_constant_expr(elt)? else {
                return Ok(None);
            };
            constants.push(constant);
        }
        let constant = match collection_type {
            CollectionType::Tuple | CollectionType::List => ConstantData::Tuple {
                elements: constants,
            },
            CollectionType::Set => ConstantData::Frozenset {
                elements: constants,
            },
        };
        Ok(Some(constant))
    }

    fn constant_as_fold_int(constant: &ConstantData) -> Option<(BigInt, bool)> {
        match constant {
            ConstantData::Boolean { value } => Some((BigInt::from(u8::from(*value)), true)),
            ConstantData::Integer { value } => Some((value.clone(), false)),
            _ => None,
        }
    }

    fn try_fold_constant_binop(
        op: ast::Operator,
        left: &ConstantData,
        right: &ConstantData,
    ) -> Option<ConstantData> {
        let (left_int, left_is_bool) = Self::constant_as_fold_int(left)?;
        let (right_int, right_is_bool) = Self::constant_as_fold_int(right)?;
        let zero = BigInt::from(0);

        if !left_is_bool && !right_is_bool {
            return None;
        }

        match op {
            ast::Operator::Add => Some(ConstantData::Integer {
                value: left_int + right_int,
            }),
            ast::Operator::Sub => Some(ConstantData::Integer {
                value: left_int - right_int,
            }),
            ast::Operator::Mult => Some(ConstantData::Integer {
                value: left_int * right_int,
            }),
            ast::Operator::Div => {
                if right_int.is_zero() {
                    return None;
                }
                Some(ConstantData::Float {
                    value: left_int.to_f64()? / right_int.to_f64()?,
                })
            }
            ast::Operator::FloorDiv => {
                if right_int.is_zero() || left_int < zero || right_int < zero {
                    return None;
                }
                Some(ConstantData::Integer {
                    value: left_int / right_int,
                })
            }
            ast::Operator::Mod => {
                if right_int.is_zero() || left_int < zero || right_int < zero {
                    return None;
                }
                Some(ConstantData::Integer {
                    value: left_int % right_int,
                })
            }
            ast::Operator::Pow => {
                let exponent = right_int.to_u32()?;
                if exponent > 128 {
                    return None;
                }
                Some(ConstantData::Integer {
                    value: left_int.pow(exponent),
                })
            }
            ast::Operator::BitAnd => {
                if left_is_bool && right_is_bool {
                    Some(ConstantData::Boolean {
                        value: !left_int.is_zero() & !right_int.is_zero(),
                    })
                } else {
                    Some(ConstantData::Integer {
                        value: left_int & right_int,
                    })
                }
            }
            ast::Operator::BitOr => {
                if left_is_bool && right_is_bool {
                    Some(ConstantData::Boolean {
                        value: !left_int.is_zero() | !right_int.is_zero(),
                    })
                } else {
                    Some(ConstantData::Integer {
                        value: left_int | right_int,
                    })
                }
            }
            ast::Operator::BitXor => {
                if left_is_bool && right_is_bool {
                    Some(ConstantData::Boolean {
                        value: !left_int.is_zero() ^ !right_int.is_zero(),
                    })
                } else {
                    Some(ConstantData::Integer {
                        value: left_int ^ right_int,
                    })
                }
            }
            ast::Operator::MatMult | ast::Operator::LShift | ast::Operator::RShift => None,
        }
    }

    fn try_fold_constant_expr(&mut self, expr: &ast::Expr) -> CompileResult<Option<ConstantData>> {
        Ok(Some(match expr {
            ast::Expr::NumberLiteral(num) => match &num.value {
                ast::Number::Int(int) => ConstantData::Integer {
                    value: ruff_int_to_bigint(int).map_err(|e| self.error(e))?,
                },
                ast::Number::Float(f) => ConstantData::Float { value: *f },
                ast::Number::Complex { real, imag } => ConstantData::Complex {
                    value: Complex::new(*real, *imag),
                },
            },
            ast::Expr::StringLiteral(s) => ConstantData::Str {
                value: self.compile_string_value(s),
            },
            ast::Expr::BytesLiteral(b) => ConstantData::Bytes {
                value: b.value.bytes().collect(),
            },
            ast::Expr::BooleanLiteral(b) => ConstantData::Boolean { value: b.value },
            ast::Expr::NoneLiteral(_) => ConstantData::None,
            ast::Expr::EllipsisLiteral(_) => ConstantData::Ellipsis,
            ast::Expr::Name(ast::ExprName { id, ctx, .. })
                if matches!(ctx, ast::ExprContext::Load) && id.as_str() == "__debug__" =>
            {
                ConstantData::Boolean {
                    value: self.opts.optimize == 0,
                }
            }
            ast::Expr::Tuple(ast::ExprTuple { elts, .. }) => {
                let mut elements = Vec::with_capacity(elts.len());
                for elt in elts {
                    let Some(constant) = self.try_fold_constant_expr(elt)? else {
                        return Ok(None);
                    };
                    elements.push(constant);
                }
                ConstantData::Tuple { elements }
            }
            ast::Expr::Subscript(ast::ExprSubscript { value, slice, .. }) => {
                let Some(container) = self.try_fold_constant_expr(value)? else {
                    return Ok(None);
                };
                let Some(index) = self.try_fold_constant_expr(slice)? else {
                    return Ok(None);
                };
                let ConstantData::Integer { value: index } = index else {
                    return Ok(None);
                };
                let Some(index): Option<i64> = index.try_into().ok() else {
                    return Ok(None);
                };

                match container {
                    ConstantData::Str { value } => {
                        let string = value.to_string();
                        if string.contains(char::REPLACEMENT_CHARACTER) {
                            return Ok(None);
                        }
                        let chars: Vec<_> = string.chars().collect();
                        let Some(len) = i64::try_from(chars.len()).ok() else {
                            return Ok(None);
                        };
                        let idx: i64 = if index < 0 { len + index } else { index };
                        let Some(idx) = usize::try_from(idx).ok() else {
                            return Ok(None);
                        };
                        let Some(ch) = chars.get(idx) else {
                            return Ok(None);
                        };
                        ConstantData::Str {
                            value: ch.to_string().into(),
                        }
                    }
                    ConstantData::Bytes { value } => {
                        let Some(len) = i64::try_from(value.len()).ok() else {
                            return Ok(None);
                        };
                        let idx: i64 = if index < 0 { len + index } else { index };
                        let Some(idx) = usize::try_from(idx).ok() else {
                            return Ok(None);
                        };
                        let Some(byte) = value.get(idx) else {
                            return Ok(None);
                        };
                        ConstantData::Integer {
                            value: BigInt::from(*byte),
                        }
                    }
                    ConstantData::Tuple { elements } => {
                        let Some(len) = i64::try_from(elements.len()).ok() else {
                            return Ok(None);
                        };
                        let idx: i64 = if index < 0 { len + index } else { index };
                        let Some(idx) = usize::try_from(idx).ok() else {
                            return Ok(None);
                        };
                        let Some(element) = elements.get(idx) else {
                            return Ok(None);
                        };
                        element.clone()
                    }
                    _ => return Ok(None),
                }
            }
            ast::Expr::BinOp(ast::ExprBinOp {
                left, op, right, ..
            }) => {
                let Some(left) = self.try_fold_constant_expr(left)? else {
                    return Ok(None);
                };
                let Some(right) = self.try_fold_constant_expr(right)? else {
                    return Ok(None);
                };
                let Some(constant) = Self::try_fold_constant_binop(*op, &left, &right) else {
                    return Ok(None);
                };
                constant
            }
            ast::Expr::UnaryOp(ast::ExprUnaryOp { op, operand, .. }) => {
                let Some(constant) = self.try_fold_constant_expr(operand)? else {
                    return Ok(None);
                };
                match (op, constant) {
                    (ast::UnaryOp::UAdd, value) => value,
                    (ast::UnaryOp::USub, ConstantData::Integer { value }) => {
                        ConstantData::Integer { value: -value }
                    }
                    (ast::UnaryOp::USub, ConstantData::Float { value }) => {
                        ConstantData::Float { value: -value }
                    }
                    (ast::UnaryOp::USub, ConstantData::Complex { value }) => {
                        ConstantData::Complex { value: -value }
                    }
                    (ast::UnaryOp::Invert, ConstantData::Integer { value }) => {
                        ConstantData::Integer { value: !value }
                    }
                    (ast::UnaryOp::Not, value) => ConstantData::Boolean {
                        value: !Self::constant_truthiness(&value),
                    },
                    _ => return Ok(None),
                }
            }
            ast::Expr::BoolOp(ast::ExprBoolOp { op, values, .. }) => {
                let mut constants = Vec::with_capacity(values.len());
                for value in values {
                    let Some(constant) = self.try_fold_constant_expr(value)? else {
                        return Ok(None);
                    };
                    constants.push(constant);
                }
                let mut iter = constants.into_iter();
                let Some(first) = iter.next() else {
                    return Ok(None);
                };
                let mut selected = first;
                match op {
                    ast::BoolOp::Or => {
                        if !Self::constant_truthiness(&selected) {
                            for constant in iter {
                                let is_truthy = Self::constant_truthiness(&constant);
                                selected = constant;
                                if is_truthy {
                                    break;
                                }
                            }
                        }
                    }
                    ast::BoolOp::And => {
                        if Self::constant_truthiness(&selected) {
                            for constant in iter {
                                let is_truthy = Self::constant_truthiness(&constant);
                                selected = constant;
                                if !is_truthy {
                                    break;
                                }
                            }
                        }
                    }
                }
                selected
            }
            _ => return Ok(None),
        }))
    }

    fn emit_load_const(&mut self, constant: ConstantData) {
        let idx = self.arg_constant(constant);
        self.emit_arg(idx, |consti| Instruction::LoadConst { consti })
    }

    fn try_fold_constant_slice(
        &mut self,
        lower: Option<&ast::Expr>,
        upper: Option<&ast::Expr>,
        step: Option<&ast::Expr>,
    ) -> CompileResult<Option<ConstantData>> {
        if [lower, upper, step]
            .into_iter()
            .flatten()
            .any(|expr| !expr.is_constant())
        {
            return Ok(None);
        }

        let start = match lower {
            Some(expr) => {
                let Some(constant) = self.try_fold_constant_expr(expr)? else {
                    return Ok(None);
                };
                constant
            }
            None => ConstantData::None,
        };
        let stop = match upper {
            Some(expr) => {
                let Some(constant) = self.try_fold_constant_expr(expr)? else {
                    return Ok(None);
                };
                constant
            }
            None => ConstantData::None,
        };
        let step = match step {
            Some(expr) => {
                let Some(constant) = self.try_fold_constant_expr(expr)? else {
                    return Ok(None);
                };
                constant
            }
            None => ConstantData::None,
        };

        Ok(Some(ConstantData::Slice {
            elements: Box::new([start, stop, step]),
        }))
    }

    fn emit_return_const(&mut self, constant: ConstantData) {
        self.emit_load_const(constant);
        emit!(self, Instruction::ReturnValue)
    }

    fn emit_return_const_no_location(&mut self, constant: ConstantData) {
        self.emit_load_const(constant);
        self.set_no_location();
        emit!(self, Instruction::ReturnValue);
        self.set_no_location();
    }

    fn emit_end_async_for(&mut self, send_target: BlockIdx) {
        self._emit(Instruction::EndAsyncFor, OpArg::NULL, send_target);
    }

    /// Emit LOAD_ATTR for attribute access (method=false).
    /// Encodes: (name_idx << 1) | 0
    fn emit_load_attr(&mut self, name_idx: u32) {
        let encoded = LoadAttr::new(name_idx, false);
        self.emit_arg(encoded, |namei| Instruction::LoadAttr { namei })
    }

    /// Emit LOAD_ATTR with method flag set (for method calls).
    /// Encodes: (name_idx << 1) | 1
    fn emit_load_attr_method(&mut self, name_idx: u32) {
        let encoded = LoadAttr::new(name_idx, true);
        self.emit_arg(encoded, |namei| Instruction::LoadAttr { namei })
    }

    /// Emit LOAD_GLOBAL.
    /// Encodes: (name_idx << 1) | push_null_bit
    fn emit_load_global(&mut self, name_idx: u32, push_null: bool) {
        let encoded = (name_idx << 1) | u32::from(push_null);
        self.emit_arg(encoded, |namei| Instruction::LoadGlobal { namei });
    }

    /// Emit LOAD_SUPER_ATTR for 2-arg super().attr access.
    /// Encodes: (name_idx << 2) | 0b10 (method=0, class=1)
    fn emit_load_super_attr(&mut self, name_idx: u32) {
        let encoded = LoadSuperAttr::new(name_idx, false, true);
        self.emit_arg(encoded, |namei| Instruction::LoadSuperAttr { namei })
    }

    /// Emit LOAD_SUPER_ATTR for 2-arg super().method() call.
    /// Encodes: (name_idx << 2) | 0b11 (method=1, class=1)
    fn emit_load_super_method(&mut self, name_idx: u32) {
        let encoded = LoadSuperAttr::new(name_idx, true, true);
        self.emit_arg(encoded, |namei| Instruction::LoadSuperAttr { namei })
    }

    /// Emit LOAD_SUPER_ATTR for 0-arg super().attr access.
    /// Encodes: (name_idx << 2) | 0b00 (method=0, class=0)
    fn emit_load_zero_super_attr(&mut self, name_idx: u32) {
        let encoded = LoadSuperAttr::new(name_idx, false, false);
        self.emit_arg(encoded, |namei| Instruction::LoadSuperAttr { namei })
    }

    /// Emit LOAD_SUPER_ATTR for 0-arg super().method() call.
    /// Encodes: (name_idx << 2) | 0b01 (method=1, class=0)
    fn emit_load_zero_super_method(&mut self, name_idx: u32) {
        let encoded = LoadSuperAttr::new(name_idx, true, false);
        self.emit_arg(encoded, |namei| Instruction::LoadSuperAttr { namei })
    }

    fn emit_return_value(&mut self) {
        emit!(self, Instruction::ReturnValue)
    }

    fn current_code_info(&mut self) -> &mut ir::CodeInfo {
        self.code_stack.last_mut().expect("no code on stack")
    }

    /// Enter a conditional block (if/for/while/match/try/with)
    /// PEP 649: Track conditional annotation context
    fn enter_conditional_block(&mut self) {
        self.current_code_info().in_conditional_block += 1;
    }

    /// Leave a conditional block
    fn leave_conditional_block(&mut self) {
        let code_info = self.current_code_info();
        debug_assert!(code_info.in_conditional_block > 0);
        code_info.in_conditional_block -= 1;
    }

    /// Compile break or continue statement with proper fblock cleanup.
    /// compiler_break, compiler_continue
    /// This handles unwinding through With blocks and exception handlers.
    fn compile_break_continue(
        &mut self,
        range: ruff_text_size::TextRange,
        is_break: bool,
    ) -> CompileResult<()> {
        if self.do_not_emit_bytecode > 0 {
            // Still validate that we're inside a loop even in dead code
            let code = self.current_code_info();
            let mut found_loop = false;
            for i in (0..code.fblock.len()).rev() {
                match code.fblock[i].fb_type {
                    FBlockType::WhileLoop | FBlockType::ForLoop => {
                        found_loop = true;
                        break;
                    }
                    FBlockType::ExceptionGroupHandler => {
                        return Err(self.error_ranged(
                            CodegenErrorType::BreakContinueReturnInExceptStar,
                            range,
                        ));
                    }
                    _ => {}
                }
            }
            if !found_loop {
                if is_break {
                    return Err(self.error_ranged(CodegenErrorType::InvalidBreak, range));
                }
                return Err(self.error_ranged(CodegenErrorType::InvalidContinue, range));
            }
            return Ok(());
        }

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
            }
            return Err(self.error_ranged(CodegenErrorType::InvalidContinue, range));
        };

        let loop_block = code.fblock[loop_idx].fb_block;
        let exit_block = code.fblock[loop_idx].fb_exit;

        let prev_source_range = self.current_source_range;
        self.set_source_range(range);
        emit!(self, Instruction::Nop);
        self.set_source_range(prev_source_range);

        // Collect the fblocks we need to unwind through, from top down to (but not including) the loop
        #[derive(Clone)]
        enum UnwindAction {
            With {
                is_async: bool,
                range: TextRange,
            },
            HandlerCleanup {
                name: Option<String>,
            },
            TryExcept,
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
                        unwind_actions.push(UnwindAction::With {
                            is_async: false,
                            range: code.fblock[i].fb_range,
                        });
                    }
                    FBlockType::AsyncWith => {
                        unwind_actions.push(UnwindAction::With {
                            is_async: true,
                            range: code.fblock[i].fb_range,
                        });
                    }
                    FBlockType::HandlerCleanup => {
                        let name = match &code.fblock[i].fb_datum {
                            FBlockDatum::ExceptionName(name) => Some(name.clone()),
                            _ => None,
                        };
                        unwind_actions.push(UnwindAction::HandlerCleanup { name });
                    }
                    FBlockType::TryExcept => {
                        unwind_actions.push(UnwindAction::TryExcept);
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
        let mut jump_no_location = false;
        for action in unwind_actions {
            match action {
                UnwindAction::With { is_async, range } => {
                    // Stack: [..., exit_func, self_exit]
                    let saved_range = self.current_source_range;
                    self.set_source_range(range);
                    emit!(self, PseudoInstruction::PopBlock);
                    self.emit_load_const(ConstantData::None);
                    self.emit_load_const(ConstantData::None);
                    self.emit_load_const(ConstantData::None);
                    emit!(self, Instruction::Call { argc: 3 });

                    if is_async {
                        emit!(self, Instruction::GetAwaitable { r#where: 2 });
                        self.emit_load_const(ConstantData::None);
                        let _ = self.compile_yield_from_sequence(true)?;
                    }

                    emit!(self, Instruction::PopTop);
                    self.set_source_range(saved_range);
                    jump_no_location = true;
                }
                UnwindAction::HandlerCleanup { ref name } => {
                    // codegen_unwind_fblock(HANDLER_CLEANUP)
                    if name.is_some() {
                        // Named handler: PopBlock for inner SETUP_CLEANUP
                        emit!(self, PseudoInstruction::PopBlock);
                    }
                    // PopBlock for outer SETUP_CLEANUP (ExceptionHandler)
                    emit!(self, PseudoInstruction::PopBlock);
                    emit!(self, Instruction::PopExcept);
                    if let Some(name) = name {
                        self.emit_load_const(ConstantData::None);
                        self.store_name(name)?;
                        self.compile_name(name, NameUsage::Delete)?;
                    }
                }
                UnwindAction::TryExcept => {
                    // codegen_unwind_fblock(TRY_EXCEPT)
                    emit!(self, PseudoInstruction::PopBlock);
                }
                UnwindAction::FinallyTry { body, fblock_idx } => {
                    // codegen_unwind_fblock(FINALLY_TRY)
                    emit!(self, PseudoInstruction::PopBlock);

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
                    jump_no_location = true;
                }
                UnwindAction::FinallyEnd => {
                    // codegen_unwind_fblock(FINALLY_END)
                    emit!(self, Instruction::PopTop); // exc_value
                    emit!(self, PseudoInstruction::PopBlock);
                    emit!(self, Instruction::PopExcept);
                }
                UnwindAction::PopValue => {
                    // Pop the return value - continue/break cancels the pending return
                    emit!(self, Instruction::PopTop);
                }
            }
        }

        // CPython unwinds a for-loop break with POP_TOP rather than POP_ITER.
        if is_break && is_for_loop {
            emit!(self, Instruction::PopTop);
        }

        // Jump to target
        let target = if is_break { exit_block } else { loop_block };
        emit!(self, PseudoInstruction::Jump { delta: target });
        if jump_no_location {
            self.set_no_location();
        }

        Ok(())
    }

    fn current_block(&mut self) -> &mut ir::Block {
        let info = self.current_code_info();
        &mut info.blocks[info.current_block]
    }

    fn new_block(&mut self) -> BlockIdx {
        let code = self.current_code_info();
        let idx = BlockIdx::new(code.blocks.len().to_u32());
        let inherited_disable_load_fast_borrow =
            code.blocks[code.current_block].disable_load_fast_borrow;
        let block = ir::Block {
            disable_load_fast_borrow: inherited_disable_load_fast_borrow,
            ..ir::Block::default()
        };
        code.blocks.push(block);
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
    fn contains_await(expression: &ast::Expr) -> bool {
        use ast::visitor::Visitor;

        #[derive(Default)]
        struct AwaitVisitor {
            found: bool,
        }

        impl ast::visitor::Visitor<'_> for AwaitVisitor {
            fn visit_expr(&mut self, expr: &ast::Expr) {
                if self.found {
                    return;
                }

                match expr {
                    ast::Expr::Await(_) => self.found = true,
                    ast::Expr::ListComp(ast::ExprListComp { generators, .. })
                    | ast::Expr::SetComp(ast::ExprSetComp { generators, .. })
                    | ast::Expr::DictComp(ast::ExprDictComp { generators, .. })
                        if generators.iter().any(|generator| generator.is_async) =>
                    {
                        self.found = true
                    }
                    _ => ast::visitor::walk_expr(self, expr),
                }
            }
        }

        let mut visitor = AwaitVisitor::default();
        visitor.visit_expr(expression);
        visitor.found
    }

    /// Check if any of the generators (except the first one's iter) contains an await expression.
    /// The first generator's iter is evaluated outside the comprehension scope.
    fn generators_contain_await(generators: &[ast::Comprehension]) -> bool {
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

    fn compile_expr_fstring(&mut self, fstring: &ast::ExprFString) -> CompileResult<()> {
        let fstring_range = fstring.range;
        let fstring = fstring.value.as_slice();
        if self.count_fstring_parts(fstring) > STACK_USE_GUIDELINE {
            return self.compile_fstring_parts_joined(fstring);
        }

        let mut element_count = 0;
        let mut pending_literal = None;
        let mut pending_literal_no_location = false;
        for part in fstring {
            self.compile_fstring_part_into(
                part,
                &mut pending_literal,
                &mut pending_literal_no_location,
                &mut element_count,
                false,
            )?;
        }
        self.set_source_range(fstring_range);
        self.finish_fstring(pending_literal, pending_literal_no_location, element_count)
    }

    fn compile_fstring_parts_joined(&mut self, fstring: &[ast::FStringPart]) -> CompileResult<()> {
        self.emit_load_const(ConstantData::Str {
            value: Wtf8Buf::new(),
        });
        let join_idx = self.get_global_name_index("join");
        self.emit_load_attr_method(join_idx);
        emit!(self, Instruction::BuildList { count: 0 });

        let mut element_count = 0;
        let mut pending_literal = None;
        let mut pending_literal_no_location = false;
        for part in fstring {
            self.compile_fstring_part_into(
                part,
                &mut pending_literal,
                &mut pending_literal_no_location,
                &mut element_count,
                true,
            )?;
        }
        self.finish_fstring_join(pending_literal, pending_literal_no_location, element_count);
        Ok(())
    }

    fn compile_fstring_part_into(
        &mut self,
        part: &ast::FStringPart,
        pending_literal: &mut Option<Wtf8Buf>,
        pending_literal_no_location: &mut bool,
        element_count: &mut u32,
        append_to_join_list: bool,
    ) -> CompileResult<()> {
        match part {
            ast::FStringPart::Literal(string) => {
                let value = self.compile_fstring_part_literal_value(string);
                if pending_literal.is_none() {
                    self.set_source_range(string.range);
                    *pending_literal_no_location = string.range == TextRange::default();
                    *pending_literal = Some(value);
                } else if let Some(pending) = pending_literal.as_mut() {
                    *pending_literal_no_location &= string.range == TextRange::default();
                    pending.push_wtf8(value.as_ref());
                }
                Ok(())
            }
            ast::FStringPart::FString(fstring) => self.compile_fstring_elements_into(
                fstring.flags,
                &fstring.elements,
                pending_literal,
                pending_literal_no_location,
                element_count,
                append_to_join_list,
            ),
        }
    }

    fn finish_fstring(
        &mut self,
        mut pending_literal: Option<Wtf8Buf>,
        mut pending_literal_no_location: bool,
        mut element_count: u32,
    ) -> CompileResult<()> {
        let keep_empty = element_count == 0;
        self.emit_pending_fstring_literal(
            &mut pending_literal,
            &mut pending_literal_no_location,
            &mut element_count,
            keep_empty,
            false,
        );

        if element_count == 0 {
            self.emit_load_const(ConstantData::Str {
                value: Wtf8Buf::new(),
            });
        } else if element_count > 1 {
            emit!(
                self,
                Instruction::BuildString {
                    count: element_count
                }
            );
        }

        Ok(())
    }

    fn finish_fstring_join(
        &mut self,
        mut pending_literal: Option<Wtf8Buf>,
        mut pending_literal_no_location: bool,
        mut element_count: u32,
    ) {
        let keep_empty = element_count == 0;
        self.emit_pending_fstring_literal(
            &mut pending_literal,
            &mut pending_literal_no_location,
            &mut element_count,
            keep_empty,
            true,
        );
        emit!(self, Instruction::Call { argc: 1 });
    }

    fn emit_pending_fstring_literal(
        &mut self,
        pending_literal: &mut Option<Wtf8Buf>,
        pending_literal_no_location: &mut bool,
        element_count: &mut u32,
        keep_empty: bool,
        append_to_join_list: bool,
    ) {
        let Some(value) = pending_literal.take() else {
            return;
        };
        let no_location = *pending_literal_no_location;
        *pending_literal_no_location = false;

        // CPython drops empty literal fragments when they are adjacent to
        // formatted values, but still emits an empty string for a fully-empty
        // f-string.
        if value.is_empty() && (!keep_empty || *element_count > 0) {
            return;
        }

        self.emit_load_const(ConstantData::Str { value });
        if no_location {
            self.set_no_location();
        }
        *element_count += 1;
        if append_to_join_list {
            emit!(self, Instruction::ListAppend { i: 1 });
        }
    }

    fn count_fstring_parts(&self, fstring: &[ast::FStringPart]) -> u32 {
        let mut element_count = 0;
        let mut pending_literal = None;
        for part in fstring {
            self.count_fstring_part_into(part, &mut pending_literal, &mut element_count);
        }
        let keep_empty = element_count == 0;
        Self::count_pending_fstring_literal(&mut pending_literal, &mut element_count, keep_empty);
        element_count
    }

    fn count_fstring_part_into(
        &self,
        part: &ast::FStringPart,
        pending_literal: &mut Option<Wtf8Buf>,
        element_count: &mut u32,
    ) {
        match part {
            ast::FStringPart::Literal(string) => {
                let value = self.compile_fstring_part_literal_value(string);
                if let Some(pending) = pending_literal.as_mut() {
                    pending.push_wtf8(value.as_ref());
                } else {
                    *pending_literal = Some(value);
                }
            }
            ast::FStringPart::FString(fstring) => self.count_fstring_elements_into(
                fstring.flags,
                &fstring.elements,
                pending_literal,
                element_count,
            ),
        }
    }

    fn count_pending_fstring_literal(
        pending_literal: &mut Option<Wtf8Buf>,
        element_count: &mut u32,
        keep_empty: bool,
    ) {
        let Some(value) = pending_literal.take() else {
            return;
        };

        if value.is_empty() && (!keep_empty || *element_count > 0) {
            return;
        }

        *element_count += 1;
    }

    fn compile_fstring_elements(
        &mut self,
        flags: ast::FStringFlags,
        fstring_elements: &ast::InterpolatedStringElements,
    ) -> CompileResult<()> {
        if self.count_fstring_elements(flags, fstring_elements) > STACK_USE_GUIDELINE {
            return self.compile_fstring_elements_joined(flags, fstring_elements);
        }

        let mut element_count = 0;
        let mut pending_literal: Option<Wtf8Buf> = None;
        let mut pending_literal_no_location = false;
        self.compile_fstring_elements_into(
            flags,
            fstring_elements,
            &mut pending_literal,
            &mut pending_literal_no_location,
            &mut element_count,
            false,
        )?;
        self.finish_fstring(pending_literal, pending_literal_no_location, element_count)
    }

    fn compile_fstring_elements_joined(
        &mut self,
        flags: ast::FStringFlags,
        fstring_elements: &ast::InterpolatedStringElements,
    ) -> CompileResult<()> {
        self.emit_load_const(ConstantData::Str {
            value: Wtf8Buf::new(),
        });
        let join_idx = self.get_global_name_index("join");
        self.emit_load_attr_method(join_idx);
        emit!(self, Instruction::BuildList { count: 0 });

        let mut element_count = 0;
        let mut pending_literal: Option<Wtf8Buf> = None;
        let mut pending_literal_no_location = false;
        self.compile_fstring_elements_into(
            flags,
            fstring_elements,
            &mut pending_literal,
            &mut pending_literal_no_location,
            &mut element_count,
            true,
        )?;
        self.finish_fstring_join(pending_literal, pending_literal_no_location, element_count);
        Ok(())
    }

    fn compile_fstring_elements_into(
        &mut self,
        flags: ast::FStringFlags,
        fstring_elements: &ast::InterpolatedStringElements,
        pending_literal: &mut Option<Wtf8Buf>,
        pending_literal_no_location: &mut bool,
        element_count: &mut u32,
        append_to_join_list: bool,
    ) -> CompileResult<()> {
        for element in fstring_elements {
            match element {
                ast::InterpolatedStringElement::Literal(string) => {
                    let value = self.compile_fstring_literal_value(string, flags);
                    if pending_literal.is_none() {
                        self.set_source_range(string.range);
                        *pending_literal_no_location = string.range == TextRange::default();
                        *pending_literal = Some(value);
                    } else if let Some(pending) = pending_literal.as_mut() {
                        *pending_literal_no_location &= string.range == TextRange::default();
                        pending.push_wtf8(value.as_ref());
                    }
                }
                ast::InterpolatedStringElement::Interpolation(fstring_expr) => {
                    let mut conversion = match fstring_expr.conversion {
                        ast::ConversionFlag::None => ConvertValueOparg::None,
                        ast::ConversionFlag::Str => ConvertValueOparg::Str,
                        ast::ConversionFlag::Repr => ConvertValueOparg::Repr,
                        ast::ConversionFlag::Ascii => ConvertValueOparg::Ascii,
                    };

                    if let Some(ast::DebugText { leading, trailing }) = &fstring_expr.debug_text {
                        let range = fstring_expr.expression.range();
                        let source = self.source_file.slice(range);
                        let text = [
                            strip_fstring_debug_comments(leading).as_str(),
                            source,
                            strip_fstring_debug_comments(trailing).as_str(),
                        ]
                        .concat();

                        let text: Wtf8Buf = text.into();
                        if pending_literal.is_none() {
                            *pending_literal_no_location = false;
                            *pending_literal = Some(Wtf8Buf::new());
                        }
                        pending_literal.as_mut().unwrap().push_wtf8(text.as_ref());

                        // If debug text is present, apply repr conversion when no `format_spec` specified.
                        // See action_helpers.c: fstring_find_expr_replacement
                        if matches!(
                            (conversion, &fstring_expr.format_spec),
                            (ConvertValueOparg::None, None)
                        ) {
                            conversion = ConvertValueOparg::Repr;
                        }
                    }

                    self.emit_pending_fstring_literal(
                        pending_literal,
                        pending_literal_no_location,
                        element_count,
                        false,
                        append_to_join_list,
                    );

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

                    *element_count += 1;
                    if append_to_join_list {
                        emit!(self, Instruction::ListAppend { i: 1 });
                    }
                }
            }
        }

        Ok(())
    }

    fn count_fstring_elements(
        &self,
        flags: ast::FStringFlags,
        fstring_elements: &ast::InterpolatedStringElements,
    ) -> u32 {
        let mut element_count = 0;
        let mut pending_literal = None;
        self.count_fstring_elements_into(
            flags,
            fstring_elements,
            &mut pending_literal,
            &mut element_count,
        );
        let keep_empty = element_count == 0;
        Self::count_pending_fstring_literal(&mut pending_literal, &mut element_count, keep_empty);
        element_count
    }

    fn count_fstring_elements_into(
        &self,
        flags: ast::FStringFlags,
        fstring_elements: &ast::InterpolatedStringElements,
        pending_literal: &mut Option<Wtf8Buf>,
        element_count: &mut u32,
    ) {
        for element in fstring_elements {
            match element {
                ast::InterpolatedStringElement::Literal(string) => {
                    let value = self.compile_fstring_literal_value(string, flags);
                    if let Some(pending) = pending_literal.as_mut() {
                        pending.push_wtf8(value.as_ref());
                    } else {
                        *pending_literal = Some(value);
                    }
                }
                ast::InterpolatedStringElement::Interpolation(fstring_expr) => {
                    if let Some(ast::DebugText { leading, trailing }) = &fstring_expr.debug_text {
                        let range = fstring_expr.expression.range();
                        let source = self.source_file.slice(range);
                        let text = [
                            strip_fstring_debug_comments(leading).as_str(),
                            source,
                            strip_fstring_debug_comments(trailing).as_str(),
                        ]
                        .concat();

                        let text: Wtf8Buf = text.into();
                        pending_literal
                            .get_or_insert_with(Wtf8Buf::new)
                            .push_wtf8(text.as_ref());
                    }

                    Self::count_pending_fstring_literal(pending_literal, element_count, false);
                    *element_count += 1;
                }
            }
        }
    }

    fn compile_expr_tstring(&mut self, expr_tstring: &ast::ExprTString) -> CompileResult<()> {
        // ast::TStringValue can contain multiple ast::TString parts (implicit
        // concatenation). Match CPython's stack order by materializing the
        // strings tuple first, then evaluating interpolations left-to-right.
        let tstring_value = &expr_tstring.value;

        let mut all_strings: Vec<Wtf8Buf> = Vec::new();
        let mut current_string = Wtf8Buf::new();
        let mut interp_count: u32 = 0;

        for tstring in tstring_value.iter() {
            self.collect_tstring_strings(
                tstring,
                &mut all_strings,
                &mut current_string,
                &mut interp_count,
            );
        }

        all_strings.push(core::mem::take(&mut current_string));

        let string_count: u32 = all_strings
            .len()
            .try_into()
            .expect("t-string string count overflowed");
        for s in &all_strings {
            self.emit_load_const(ConstantData::Str { value: s.clone() });
        }
        emit!(
            self,
            Instruction::BuildTuple {
                count: string_count
            }
        );

        for tstring in tstring_value.iter() {
            self.compile_tstring_interpolations(tstring)?;
        }

        emit!(
            self,
            Instruction::BuildTuple {
                count: interp_count
            }
        );
        emit!(self, Instruction::BuildTemplate);

        Ok(())
    }

    fn collect_tstring_strings(
        &self,
        tstring: &ast::TString,
        strings: &mut Vec<Wtf8Buf>,
        current_string: &mut Wtf8Buf,
        interp_count: &mut u32,
    ) {
        for element in &tstring.elements {
            match element {
                ast::InterpolatedStringElement::Literal(lit) => {
                    current_string
                        .push_wtf8(&self.compile_tstring_literal_value(lit, tstring.flags));
                }
                ast::InterpolatedStringElement::Interpolation(interp) => {
                    if let Some(ast::DebugText { leading, trailing }) = &interp.debug_text {
                        let range = interp.expression.range();
                        let source = self.source_file.slice(range);
                        let text = [
                            strip_fstring_debug_comments(leading).as_str(),
                            source,
                            strip_fstring_debug_comments(trailing).as_str(),
                        ]
                        .concat();
                        current_string.push_str(&text);
                    }
                    strings.push(core::mem::take(current_string));
                    *interp_count += 1;
                }
            }
        }
    }

    fn compile_tstring_interpolations(&mut self, tstring: &ast::TString) -> CompileResult<()> {
        for element in &tstring.elements {
            let ast::InterpolatedStringElement::Interpolation(interp) = element else {
                continue;
            };

            self.compile_expression(&interp.expression)?;

            let expr_range = interp.expression.range();
            let expr_source = if interp.range.start() < expr_range.start()
                && interp.range.end() >= expr_range.end()
            {
                let after_brace = interp.range.start() + TextSize::new(1);
                self.source_file
                    .slice(TextRange::new(after_brace, expr_range.end()))
            } else {
                self.source_file.slice(expr_range)
            };
            self.emit_load_const(ConstantData::Str {
                value: expr_source.to_string().into(),
            });

            let mut conversion: u32 = match interp.conversion {
                ast::ConversionFlag::None => 0,
                ast::ConversionFlag::Str => 1,
                ast::ConversionFlag::Repr => 2,
                ast::ConversionFlag::Ascii => 3,
            };

            if interp.debug_text.is_some() && conversion == 0 && interp.format_spec.is_none() {
                conversion = 2;
            }

            let has_format_spec = interp.format_spec.is_some();
            if let Some(format_spec) = &interp.format_spec {
                self.compile_fstring_elements(ast::FStringFlags::empty(), &format_spec.elements)?;
            }

            // CPython keeps bit 1 set in BUILD_INTERPOLATION's oparg and uses
            // bit 0 for the optional format spec.
            let format = 2 | (conversion << 2) | u32::from(has_format_spec);
            emit!(self, Instruction::BuildInterpolation { format });
        }

        Ok(())
    }
}

trait EmitArg<Arg: OpArgType> {
    fn emit<I: Into<AnyInstruction>>(
        self,
        f: impl FnOnce(OpArgMarker<Arg>) -> I,
    ) -> (AnyInstruction, OpArg, BlockIdx);
}

impl<T: OpArgType> EmitArg<T> for T {
    fn emit<I: Into<AnyInstruction>>(
        self,
        f: impl FnOnce(OpArgMarker<T>) -> I,
    ) -> (AnyInstruction, OpArg, BlockIdx) {
        let (marker, arg) = OpArgMarker::new(self);
        (f(marker).into(), arg, BlockIdx::NULL)
    }
}

impl EmitArg<bytecode::Label> for BlockIdx {
    fn emit<I: Into<AnyInstruction>>(
        self,
        f: impl FnOnce(OpArgMarker<bytecode::Label>) -> I,
    ) -> (AnyInstruction, OpArg, BlockIdx) {
        (f(OpArgMarker::marker()).into(), OpArg::NULL, self)
    }
}

/// Strips leading whitespace from a docstring.
///
/// `inspect.cleandoc` is a good reference, but has a few incompatibilities.
// = _PyCompile_CleanDoc
fn clean_doc(doc: &str) -> String {
    let doc = expandtabs(doc, 8);
    // First pass: find minimum indentation of non-blank lines AFTER the first line.
    // A "blank line" is one containing only spaces (or empty).
    let margin = doc
        .split('\n')
        .skip(1) // skip first line
        .filter(|line| line.chars().any(|c| c != ' ')) // non-blank lines only
        .map(|line| line.chars().take_while(|c| *c == ' ').count())
        .min()
        .unwrap_or(0);

    let mut cleaned = String::with_capacity(doc.len());
    // Strip all leading spaces from the first line
    if let Some(first_line) = doc.split('\n').next() {
        let trimmed = first_line.trim_start();
        // Early exit: no leading spaces on first line AND margin == 0
        if trimmed.len() == first_line.len() && margin == 0 {
            return doc.to_owned();
        }
        cleaned.push_str(trimmed);
    }
    // Subsequent lines: skip up to `margin` leading spaces
    for line in doc.split('\n').skip(1) {
        cleaned.push('\n');
        let skip = line.chars().take(margin).take_while(|c| *c == ' ').count();
        cleaned.push_str(&line[skip..]);
    }

    cleaned
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

fn split_doc<'a>(body: &'a [ast::Stmt], opts: &CompileOpts) -> (Option<String>, &'a [ast::Stmt]) {
    if let Some((ast::Stmt::Expr(expr), body_rest)) = body.split_first() {
        let doc_comment = match &*expr.value {
            ast::Expr::StringLiteral(value) => Some(&value.value),
            // f-strings are not allowed in Python doc comments.
            ast::Expr::FString(_) => None,
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

pub fn ruff_int_to_bigint(int: &ast::Int) -> Result<BigInt, CodegenErrorType> {
    if let Some(small) = int.as_u64() {
        Ok(BigInt::from(small))
    } else {
        parse_big_integer(int)
    }
}

/// Converts a `ruff` ast integer into a `BigInt`.
/// Unlike small integers, big integers may be stored in one of four possible radix representations.
fn parse_big_integer(int: &ast::Int) -> Result<BigInt, CodegenErrorType> {
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

/// Strip Python comments from f-string debug text (leading/trailing around `=`).
/// A comment starts with `#` and extends to the end of the line.
/// The newline character itself is preserved.
fn strip_fstring_debug_comments(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut in_comment = false;
    for ch in text.chars() {
        if in_comment {
            if ch == '\n' {
                in_comment = false;
                result.push(ch);
            }
        } else if ch == '#' {
            in_comment = true;
        } else {
            result.push(ch);
        }
    }
    result
}

#[cfg(test)]
mod ruff_tests {
    use super::*;
    use ast::name::Name;

    /// Test if the compiler can correctly identify fstrings containing an `await` expression.
    #[test]
    fn test_fstring_contains_await() {
        let range = TextRange::default();
        let flags = ast::FStringFlags::empty();

        // f'{x}'
        let expr_x = ast::Expr::Name(ast::ExprName {
            node_index: ast::AtomicNodeIndex::NONE,
            range,
            id: Name::new("x"),
            ctx: ast::ExprContext::Load,
        });
        let not_present = &ast::Expr::FString(ast::ExprFString {
            node_index: ast::AtomicNodeIndex::NONE,
            range,
            value: ast::FStringValue::single(ast::FString {
                node_index: ast::AtomicNodeIndex::NONE,
                range,
                elements: vec![ast::InterpolatedStringElement::Interpolation(
                    ast::InterpolatedElement {
                        node_index: ast::AtomicNodeIndex::NONE,
                        range,
                        expression: Box::new(expr_x),
                        debug_text: None,
                        conversion: ast::ConversionFlag::None,
                        format_spec: None,
                    },
                )]
                .into(),
                flags,
            }),
        });
        assert!(!Compiler::contains_await(not_present));

        // f'{await x}'
        let expr_await_x = ast::Expr::Await(ast::ExprAwait {
            node_index: ast::AtomicNodeIndex::NONE,
            range,
            value: Box::new(ast::Expr::Name(ast::ExprName {
                node_index: ast::AtomicNodeIndex::NONE,
                range,
                id: Name::new("x"),
                ctx: ast::ExprContext::Load,
            })),
        });
        let present = &ast::Expr::FString(ast::ExprFString {
            node_index: ast::AtomicNodeIndex::NONE,
            range,
            value: ast::FStringValue::single(ast::FString {
                node_index: ast::AtomicNodeIndex::NONE,
                range,
                elements: vec![ast::InterpolatedStringElement::Interpolation(
                    ast::InterpolatedElement {
                        node_index: ast::AtomicNodeIndex::NONE,
                        range,
                        expression: Box::new(expr_await_x),
                        debug_text: None,
                        conversion: ast::ConversionFlag::None,
                        format_spec: None,
                    },
                )]
                .into(),
                flags,
            }),
        });
        assert!(Compiler::contains_await(present));

        // f'{x:{await y}}'
        let expr_x = ast::Expr::Name(ast::ExprName {
            node_index: ast::AtomicNodeIndex::NONE,
            range,
            id: Name::new("x"),
            ctx: ast::ExprContext::Load,
        });
        let expr_await_y = ast::Expr::Await(ast::ExprAwait {
            node_index: ast::AtomicNodeIndex::NONE,
            range,
            value: Box::new(ast::Expr::Name(ast::ExprName {
                node_index: ast::AtomicNodeIndex::NONE,
                range,
                id: Name::new("y"),
                ctx: ast::ExprContext::Load,
            })),
        });
        let present = &ast::Expr::FString(ast::ExprFString {
            node_index: ast::AtomicNodeIndex::NONE,
            range,
            value: ast::FStringValue::single(ast::FString {
                node_index: ast::AtomicNodeIndex::NONE,
                range,
                elements: vec![ast::InterpolatedStringElement::Interpolation(
                    ast::InterpolatedElement {
                        node_index: ast::AtomicNodeIndex::NONE,
                        range,
                        expression: Box::new(expr_x),
                        debug_text: None,
                        conversion: ast::ConversionFlag::None,
                        format_spec: Some(Box::new(ast::InterpolatedStringFormatSpec {
                            node_index: ast::AtomicNodeIndex::NONE,
                            range,
                            elements: vec![ast::InterpolatedStringElement::Interpolation(
                                ast::InterpolatedElement {
                                    node_index: ast::AtomicNodeIndex::NONE,
                                    range,
                                    expression: Box::new(expr_await_y),
                                    debug_text: None,
                                    conversion: ast::ConversionFlag::None,
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
    use rustpython_compiler_core::{SourceFileBuilder, bytecode::OpArg};

    fn assert_scope_exit_locations(code: &CodeObject) {
        for (instr, (location, _)) in code.instructions.iter().zip(code.locations.iter()) {
            if matches!(
                instr.op,
                Instruction::ReturnValue
                    | Instruction::RaiseVarargs { .. }
                    | Instruction::Reraise { .. }
            ) {
                assert!(
                    location.line.get() > 0,
                    "scope-exit instruction {instr:?} is missing a line number"
                );
            }
        }
        for constant in code.constants.iter() {
            if let ConstantData::Code { code } = constant {
                assert_scope_exit_locations(code);
            }
        }
    }

    fn compile_exec(source: &str) -> CodeObject {
        let opts = CompileOpts::default();
        compile_exec_with_options(source, opts)
    }

    fn compile_single(source: &str) -> CodeObject {
        let opts = CompileOpts::default();
        let source_file = SourceFileBuilder::new("source_path", source).finish();
        let parsed = ruff_python_parser::parse(
            source_file.source_text(),
            ruff_python_parser::Mode::Module.into(),
        )
        .unwrap()
        .into_syntax();
        compile_top(parsed, source_file, Mode::Single, opts).unwrap()
    }

    fn compile_exec_optimized(source: &str) -> CodeObject {
        let opts = CompileOpts {
            optimize: 1,
            ..CompileOpts::default()
        };
        compile_exec_with_options(source, opts)
    }

    fn compile_exec_with_options(source: &str, opts: CompileOpts) -> CodeObject {
        let source_file = SourceFileBuilder::new("source_path", source).finish();
        let parsed = ruff_python_parser::parse(
            source_file.source_text(),
            ruff_python_parser::Mode::Module.into(),
        )
        .unwrap();
        let mut ast = parsed.into_syntax();
        preprocess::preprocess_mod(&mut ast);
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

    fn scan_program_symbol_table(source: &str) -> SymbolTable {
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
        SymbolTable::scan_program(&ast, source_file)
            .map_err(|e| e.into_codegen_error("source_path".to_owned()))
            .unwrap()
    }

    fn compile_exec_late_cfg_trace(source: &str) -> Vec<(String, String)> {
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
        let _table = compiler.pop_symbol_table();
        let stack_top = compiler.code_stack.pop().unwrap();
        stack_top.debug_late_cfg_trace().unwrap()
    }

    fn compile_single_function_late_cfg_trace(
        source: &str,
        function_name: &str,
    ) -> Vec<(String, String)> {
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
        let function = ast
            .body
            .iter()
            .find_map(|stmt| match stmt {
                ast::Stmt::FunctionDef(f) if f.name.as_str() == function_name => Some(f),
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing function {function_name}"));

        let name = &function.name;
        let parameters = &function.parameters;
        let body = &function.body;
        let is_async = function.is_async;
        let range = function.range();

        let mut compiler = Compiler::new(opts, source_file, "<module>".to_owned());
        compiler.future_annotations = symbol_table.future_annotations;
        compiler.symbol_table_stack.push(symbol_table);
        compiler.set_source_range(range);
        compiler.enter_function(name.as_str(), parameters).unwrap();
        compiler
            .current_code_info()
            .flags
            .set(bytecode::CodeFlags::COROUTINE, is_async);

        let prev_ctx = compiler.ctx;
        compiler.ctx = CompileContext {
            loop_data: None,
            in_class: prev_ctx.in_class,
            func: if is_async {
                FunctionContext::AsyncFunction
            } else {
                FunctionContext::Function
            },
            in_async_scope: is_async,
        };
        compiler.set_qualname();
        compiler.compile_statements(body).unwrap();
        match body.last() {
            Some(ast::Stmt::Return(_)) => {}
            _ => compiler.emit_return_const_no_location(ConstantData::None),
        }
        if compiler.current_code_info().metadata.consts.is_empty() {
            compiler.arg_constant(ConstantData::None);
        }

        let _table = compiler.pop_symbol_table();
        let stack_top = compiler.code_stack.pop().unwrap();
        stack_top.debug_late_cfg_trace().unwrap()
    }

    #[test]
    #[ignore = "debug helper"]
    fn debug_trace_make_dataclass_borrow_tail() {
        let trace = compile_single_function_late_cfg_trace(
            r#"
def f(module, cls, decorator, init, repr, eq, order, unsafe_hash, frozen, match_args, kw_only, slots, weakref_slot):
    if module is None:
        try:
            module = sys._getframemodulename(1) or '__main__'
        except AttributeError:
            try:
                module = sys._getframe(1).f_globals.get('__name__', '__main__')
            except (AttributeError, ValueError):
                pass
    if module is not None:
        cls.__module__ = module
    cls = decorator(cls, init=init, repr=repr, eq=eq, order=order,
                    unsafe_hash=unsafe_hash, frozen=frozen,
                    match_args=match_args, kw_only=kw_only, slots=slots,
                    weakref_slot=weakref_slot)
    return cls
"#,
            "f",
        );
        for (label, dump) in trace {
            if label.starts_with("after_") {
                eprintln!("=== {label} ===\n{dump}");
            }
        }
    }

    #[test]
    #[ignore = "debug helper"]
    fn debug_trace_protected_attr_subscript_tail() {
        let trace = compile_single_function_late_cfg_trace(
            r#"
def f(f, oldcls, newcls):
    try:
        idx = f.__code__.co_freevars.index("__class__")
    except ValueError:
        return False
    closure = f.__closure__[idx]
    if closure.cell_contents is oldcls:
        closure.cell_contents = newcls
        return True
    return False
"#,
            "f",
        );
        for (label, dump) in trace {
            if label.starts_with("after_") {
                eprintln!("=== {label} ===\n{dump}");
            }
        }
    }

    #[test]
    #[ignore = "debug helper"]
    fn debug_trace_dtrace_tail() {
        let trace = compile_single_function_late_cfg_trace(
            r#"
def f(proc, unittest):
    try:
        with proc:
            version, stderr = proc.communicate()
        if proc.returncode:
            raise Exception(version, stderr)
    except OSError:
        raise unittest.SkipTest("x")
    match = re.search("pat", version)
    if match is None:
        raise unittest.SkipTest(f"Unable to parse readelf version: {version}")
    return int(match.group(1)), int(match.group(2))
"#,
            "f",
        );
        for (label, dump) in trace {
            if label == "after_raw_optimize_load_fast_borrow"
                || label.contains("deoptimize_borrow_in_protected_conditional_tail")
                || label.contains("deoptimize_borrow_after_terminal_except_tail")
            {
                eprintln!("=== {label} ===\n{dump}");
            }
        }
    }

    #[test]
    #[ignore = "debug helper"]
    fn debug_trace_colorize_tail() {
        let trace = compile_single_function_late_cfg_trace(
            r#"
def f(sys, os, file):
    if sys.platform == "win32":
        try:
            import nt
            if not nt._supports_virtual_terminal():
                return False
        except (ImportError, AttributeError):
            return False

    try:
        return os.isatty(file.fileno())
    except OSError:
        return hasattr(file, "isatty") and file.isatty()
"#,
            "f",
        );
        for (label, dump) in trace {
            if label == "after_raw_optimize_load_fast_borrow"
                || label == "after_deoptimize_borrow_after_protected_import"
                || label == "after_optimize_load_fast_borrow"
                || label == "after_borrow_deopts"
            {
                eprintln!("=== {label} ===\n{dump}");
            }
        }
    }

    fn find_code<'a>(code: &'a CodeObject, name: &str) -> Option<&'a CodeObject> {
        if code.obj_name == name {
            return Some(code);
        }
        code.constants.iter().find_map(|constant| {
            if let ConstantData::Code { code } = constant {
                find_code(code, name)
            } else {
                None
            }
        })
    }

    fn has_common_constant(code: &CodeObject, expected: bytecode::CommonConstant) -> bool {
        code.instructions.iter().any(|unit| match unit.op {
            Instruction::LoadCommonConstant { idx } => {
                idx.get(OpArg::new(u32::from(u8::from(unit.arg)))) == expected
            }
            _ => false,
        })
    }

    fn has_intrinsic_1(code: &CodeObject, expected: IntrinsicFunction1) -> bool {
        code.instructions.iter().any(|unit| match unit.op {
            Instruction::CallIntrinsic1 { func } => {
                func.get(OpArg::new(u32::from(u8::from(unit.arg)))) == expected
            }
            _ => false,
        })
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
    fn test_trace_assert_true_try_pair() {
        let trace = compile_exec_late_cfg_trace(
            "\
try:
    assert True
except AssertionError as e:
    fail()
try:
    assert True, 'msg'
except AssertionError as e:
    fail()
",
        );
        for (stage, dump) in trace {
            eprintln!("=== {stage} ===\n{dump}");
        }
    }

    #[test]
    fn test_trace_for_unpack_list_literal() {
        let trace = compile_exec_late_cfg_trace(
            "\
result = []
for x, in [(1,), (2,), (3,)]:
    result.append(x)
",
        );
        for (stage, dump) in trace {
            eprintln!("=== {stage} ===\n{dump}");
        }
    }

    #[test]
    fn test_trace_break_in_finally_function() {
        let trace = compile_single_function_late_cfg_trace(
            "\
def f(self):
    count = 0
    while count < 2:
        count += 1
        try:
            pass
        finally:
            break
    self.assertEqual(count, 1)
",
            "f",
        );
        for (stage, dump) in trace {
            eprintln!("=== {stage} ===\n{dump}");
        }
    }

    #[test]
    fn test_import_originated_name_disables_method_call_optimization_even_with_local_import() {
        let code = compile_exec(
            "\
import warnings

def f(ch):
    import warnings
    warnings.warn(
        '\"\\\\%c\" is an invalid escape sequence' % ch
        if 0x20 <= ch < 0x7F
        else '\"\\\\x%02x\" is an invalid escape sequence' % ch,
        DeprecationWarning,
        stacklevel=2,
    )
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f.instructions.iter().map(|unit| unit.op).collect();
        let warn_attr = ops
            .iter()
            .position(|op| matches!(op, Instruction::LoadAttr { .. }))
            .expect("missing LOAD_ATTR for warnings.warn");
        let push_null = ops[warn_attr + 10..]
            .iter()
            .position(|op| matches!(op, Instruction::PushNull))
            .map(|idx| warn_attr + 10 + idx)
            .expect("expected PUSH_NULL after plain LOAD_ATTR");

        let load_attr = match f.instructions[warn_attr].op {
            Instruction::LoadAttr { namei } => namei.get(OpArg::new(u32::from(u8::from(
                f.instructions[warn_attr].arg,
            )))),
            _ => unreachable!(),
        };
        assert!(
            !load_attr.is_method(),
            "import-originated names should use plain LOAD_ATTR"
        );
        assert!(
            matches!(ops[push_null + 1], Instruction::LoadSmallInt { .. }),
            "expected warning message expression to start after PUSH_NULL, got ops={ops:?}"
        );
    }

    #[test]
    fn test_trace_constant_false_elif_chain() {
        let trace = compile_exec_late_cfg_trace(
            "\
if 0: pass
elif 0: pass
elif 0: pass
elif 0: pass
else: pass
",
        );
        for (stage, dump) in trace {
            eprintln!("=== {stage} ===\n{dump}");
        }
    }

    #[test]
    fn test_trace_multi_pass_suite() {
        let trace = compile_exec_late_cfg_trace(
            "\
if 1:
    #
    #
    #
    pass
    pass
    #
    pass
    #
",
        );
        for (stage, dump) in trace {
            eprintln!("=== {stage} ===\n{dump}");
        }
    }

    #[test]
    fn test_trace_single_compare_if() {
        let trace = compile_exec_late_cfg_trace(
            "\
if 1 == 1:
    pass
",
        );
        for (stage, dump) in trace {
            eprintln!("=== {stage} ===\n{dump}");
        }
    }

    #[test]
    fn test_trace_comparison_suite() {
        let trace = compile_exec_late_cfg_trace(
            "\
if 1: pass
x = (1 == 1)
if 1 == 1: pass
if 1 != 1: pass
if 1 < 1: pass
if 1 > 1: pass
if 1 <= 1: pass
if 1 >= 1: pass
if x is x: pass
if x is not x: pass
if 1 in (): pass
if 1 not in (): pass
",
        );
        for (stage, dump) in trace {
            eprintln!("=== {stage} ===\n{dump}");
        }
    }

    #[test]
    fn test_trace_if_for_except_layout() {
        let trace = compile_exec_late_cfg_trace(
            "\
from sys import maxsize
if maxsize == 2147483647:
    for s in ('2147483648', '0o40000000000', '0x100000000', '0b10000000000000000000000000000000'):
        try:
            x = eval(s)
        except OverflowError:
            fail(\"OverflowError on huge integer literal %r\" % s)
elif maxsize == 9223372036854775807:
    pass
",
        );
        for (stage, dump) in trace {
            eprintln!("=== {stage} ===\n{dump}");
        }
    }

    #[test]
    fn test_break_in_finally_tail_loads_borrow_through_empty_fallthrough_block() {
        let code = compile_exec(
            "\
def f(self):
    count = 0
    while count < 2:
        count += 1
        try:
            pass
        finally:
            break
    self.assertEqual(count, 1)
",
        );
        let code = find_code(&code, "f").unwrap();
        let ops: Vec<_> = code
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();
        assert!(
            ops.windows(5).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::LoadFastBorrow { .. },
                        Instruction::LoadAttr { .. },
                        Instruction::LoadFastBorrow { .. },
                        Instruction::LoadSmallInt { .. },
                        Instruction::Call { .. }
                    ]
                )
            }),
            "{:?}",
            code.instructions
                .iter()
                .map(|unit| unit.op)
                .collect::<Vec<_>>()
        );
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
    fn test_nested_bool_op() {
        assert_dis_snapshot!(compile_exec(
            "\
x = Test() and False or False
"
        ));
    }

    #[test]
    fn test_const_bool_not_op() {
        assert_dis_snapshot!(compile_exec_optimized(
            "\
x = not True
"
        ));
    }

    #[test]
    fn test_plain_constant_bool_op_folds_to_selected_operand() {
        let code = compile_exec(
            "\
x = 1 or 2 or 3
",
        );
        let ops: Vec<_> = code
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();
        let folded_small_int = code.instructions.iter().any(|unit| {
            matches!(
                unit.op,
                Instruction::LoadSmallInt { i }
                    if i.get(OpArg::new(u32::from(u8::from(unit.arg)))) == 1
            )
        });
        let folded_const_one = code
            .instructions
            .iter()
            .find_map(|unit| match unit.op {
                Instruction::LoadConst { .. } => code.constants.get(usize::from(u8::from(unit.arg))),
                _ => None,
            })
            .is_some_and(|constant| {
                matches!(constant, ConstantData::Integer { value } if *value == BigInt::from(1))
            });

        assert!(
            folded_small_int || folded_const_one,
            "expected folded constant 1, got ops={ops:?}"
        );
        assert!(
            !ops.iter().any(|op| {
                matches!(
                    op,
                    Instruction::Copy { .. }
                        | Instruction::ToBool
                        | Instruction::PopJumpIfTrue { .. }
                        | Instruction::PopJumpIfFalse { .. }
                )
            }),
            "plain constant BoolOp should not leave short-circuit scaffolding, got ops={ops:?}"
        );
    }

    #[test]
    fn test_starred_call_preserves_bool_op_short_circuit_shape() {
        let code = compile_exec(
            "\
def f(g):
    return g(*(() or (1,)))
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.iter().any(|op| matches!(op, Instruction::Copy { .. })),
            "starred BoolOp should keep short-circuit COPY, got ops={ops:?}"
        );
        assert!(
            ops.iter().any(|op| matches!(op, Instruction::ToBool)),
            "starred BoolOp should keep TO_BOOL, got ops={ops:?}"
        );
        assert!(
            ops.iter()
                .any(|op| matches!(op, Instruction::PopJumpIfTrue { .. })),
            "starred BoolOp should keep POP_JUMP_IF_TRUE, got ops={ops:?}"
        );
    }

    #[test]
    fn test_partial_constant_bool_op_folds_prefix_in_value_context() {
        let code = compile_exec(
            "\
def outer(null):
    @False or null
    def f(x):
        pass
",
        );
        let outer = find_code(&code, "outer").expect("missing outer code");
        let ops: Vec<_> = outer
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.iter().any(|op| {
                matches!(
                    op,
                    Instruction::LoadFast { .. } | Instruction::LoadFastBorrow { .. }
                )
            }),
            "expected surviving decorator expression to load null directly, got ops={ops:?}"
        );
        assert!(
            !ops.iter().any(|op| {
                matches!(
                    op,
                    Instruction::Copy { .. }
                        | Instruction::ToBool
                        | Instruction::PopJumpIfTrue { .. }
                        | Instruction::PopJumpIfFalse { .. }
                )
            }),
            "partial constant BoolOp should not leave short-circuit scaffolding, got ops={ops:?}"
        );
    }

    #[test]
    fn test_nonliteral_constant_bool_op_preserves_short_circuit_shape() {
        let code = compile_exec(
            "\
x = (\"a\"[0]) or 2
",
        );
        let ops: Vec<_> = code
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            !code.instructions.iter().any(|unit| matches!(
                unit.op,
                Instruction::BinaryOp { op }
                    if op.get(OpArg::new(u32::from(u8::from(unit.arg))))
                        == oparg::BinaryOperator::Subscr
            )),
            "constant subscript should fold before bool-op lowering, got ops={ops:?}"
        );
        assert!(
            ops.iter().any(|op| matches!(op, Instruction::Copy { .. })),
            "folded non-literal BoolOp operand should keep COPY, got ops={ops:?}"
        );
        assert!(
            ops.iter().any(|op| matches!(op, Instruction::ToBool)),
            "folded non-literal BoolOp operand should keep TO_BOOL, got ops={ops:?}"
        );
        assert!(
            ops.iter()
                .any(|op| matches!(op, Instruction::PopJumpIfTrue { .. })),
            "folded non-literal BoolOp operand should keep POP_JUMP_IF_TRUE, got ops={ops:?}"
        );
    }

    #[test]
    fn test_unary_positive_complex_constant_folds_to_load_const() {
        let code = compile_exec(
            "\
x = +0.0j
",
        );
        let ops: Vec<_> = code
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            !ops.iter()
                .any(|op| matches!(op, Instruction::CallIntrinsic1 { .. })),
            "unary positive complex constant should not leave CALL_INTRINSIC_1, got ops={ops:?}"
        );
        assert!(
            matches!(
                ops.as_slice(),
                [
                    Instruction::Resume { .. },
                    Instruction::LoadConst { .. },
                    Instruction::StoreName { .. },
                    Instruction::LoadConst { .. },
                    Instruction::ReturnValue
                ]
            ),
            "expected module assignment to fold +0.0j into LOAD_CONST, got ops={ops:?}"
        );
    }

    #[test]
    fn test_folded_nonliteral_bool_op_tail_keeps_plain_load_fast() {
        let code = compile_exec(
            "\
def and_true(x):
    return True and x

def or_false(x):
    return False or x
",
        );

        for name in ["and_true", "or_false"] {
            let function = find_code(&code, name).unwrap_or_else(|| panic!("missing {name} code"));
            let ops: Vec<_> = function
                .instructions
                .iter()
                .map(|unit| unit.op)
                .filter(|op| !matches!(op, Instruction::Cache))
                .collect();

            assert!(
                ops.iter()
                    .any(|op| matches!(op, Instruction::LoadFast { .. })),
                "expected folded bool-op tail to keep LOAD_FAST in {name}, got ops={ops:?}"
            );
            assert!(
                !ops.iter().any(|op| {
                    matches!(
                        op,
                        Instruction::LoadFastBorrow { .. }
                            | Instruction::LoadFastBorrowLoadFastBorrow { .. }
                    )
                }),
                "folded bool-op tail should not introduce borrow loads in {name}, got ops={ops:?}"
            );
        }
    }

    #[test]
    fn test_folded_nonliteral_tuple_unpack_tail_keeps_plain_load_fast() {
        let code = compile_exec(
            "\
def f(self, mod):
    optimize, opt = (1, 1) if __debug__ else (0, '')
    mod.call(self.path, optimize=optimize)
    cached = mod.cache(self.source_path, optimization=opt)
    self.assertTrue(cached)
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();
        let tail_start = ops
            .iter()
            .position(|op| matches!(op, Instruction::LoadFast { .. }))
            .expect("missing folded assignment tail load");
        let tail = &ops[tail_start..];

        assert!(
            tail.iter()
                .any(|op| matches!(op, Instruction::LoadFast { .. })),
            "expected folded nonliteral tuple-unpack tail to use strong LOAD_FAST, got tail={tail:?}"
        );
        assert!(
            !tail.iter().any(|op| {
                matches!(
                    op,
                    Instruction::LoadFastBorrow { .. }
                        | Instruction::LoadFastBorrowLoadFastBorrow { .. }
                )
            }),
            "folded nonliteral tuple-unpack tail should not borrow local loads, got tail={tail:?}"
        );
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

    #[test]
    fn test_scope_exit_instructions_keep_line_numbers() {
        let code = compile_exec(
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
",
        );
        assert_scope_exit_locations(&code);
    }

    #[test]
    fn test_attribute_ex_call_uses_plain_load_attr() {
        let code = compile_exec(
            "\
def f(cls, args, kwargs):
    cls.__new__(cls, *args)
    cls.__new__(cls, *args, **kwargs)
",
        );
        let f = find_code(&code, "f").expect("missing function code");

        let ex_call_count = f
            .instructions
            .iter()
            .filter(|unit| matches!(unit.op, Instruction::CallFunctionEx))
            .count();
        let load_attr_count = f
            .instructions
            .iter()
            .filter(|unit| matches!(unit.op, Instruction::LoadAttr { .. }))
            .count();

        assert_eq!(ex_call_count, 2);
        assert_eq!(load_attr_count, 2);

        for unit in f.instructions.iter() {
            if let Instruction::LoadAttr { namei } = unit.op {
                let load_attr = namei.get(OpArg::new(u32::from(u8::from(unit.arg))));
                assert!(
                    !load_attr.is_method(),
                    "CALL_FUNCTION_EX should use plain LOAD_ATTR"
                );
            }
        }
    }

    #[test]
    fn test_large_plain_call_uses_direct_call_until_stack_guideline() {
        let code = compile_exec(
            "\
def f(g):
    return g(a0, a1, a2, a3, a4, a5, a6, a7, a8,
             a9, a10, a11, a12, a13, a14, a15, a16, a17)
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let direct_call_18 = f.instructions.iter().any(|unit| match unit.op {
            Instruction::Call { argc } => argc.get(OpArg::new(u32::from(u8::from(unit.arg)))) == 18,
            _ => false,
        });
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            direct_call_18,
            "18 positional arguments should stay on CPython's direct CALL path, got ops={ops:?}"
        );
        assert!(
            !ops.iter()
                .any(|op| matches!(op, Instruction::CallFunctionEx)),
            "18 positional arguments should not use CALL_FUNCTION_EX, got ops={ops:?}"
        );
    }

    #[test]
    fn test_simple_attribute_call_keeps_method_load() {
        let code = compile_exec(
            "\
def f(obj, arg):
    return obj.method(arg)
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let load_attr = f
            .instructions
            .iter()
            .find_map(|unit| match unit.op {
                Instruction::LoadAttr { namei } => {
                    Some(namei.get(OpArg::new(u32::from(u8::from(unit.arg)))))
                }
                _ => None,
            })
            .expect("missing LOAD_ATTR");

        assert!(
            load_attr.is_method(),
            "simple method calls should stay optimized"
        );
    }

    #[test]
    fn test_builtin_any_genexpr_call_is_optimized() {
        let code = compile_exec(
            "\
def f(xs):
    return any(x for x in xs)
",
        );
        let f = find_code(&code, "f").expect("missing function code");

        assert!(has_common_constant(f, bytecode::CommonConstant::BuiltinAny));
        assert!(
            f.instructions
                .iter()
                .any(|unit| matches!(unit.op, Instruction::PopJumpIfTrue { .. }))
        );
        assert!(
            f.instructions
                .iter()
                .any(|unit| matches!(unit.op, Instruction::NotTaken))
        );
        assert_eq!(
            f.instructions
                .iter()
                .filter(|unit| matches!(unit.op, Instruction::PushNull))
                .count(),
            1,
            "fallback call path should remain for shadowed any()"
        );
    }

    #[test]
    fn test_builtin_tuple_genexpr_call_is_optimized_but_list_set_are_not() {
        let code = compile_exec(
            "\
def tuple_f(xs):
    return tuple(x for x in xs)

def list_f(xs):
    return list(x for x in xs)

def set_f(xs):
    return set(x for x in xs)
",
        );

        let tuple_f = find_code(&code, "tuple_f").expect("missing tuple_f code");
        assert!(has_common_constant(
            tuple_f,
            bytecode::CommonConstant::BuiltinTuple
        ));
        assert!(has_intrinsic_1(tuple_f, IntrinsicFunction1::ListToTuple));
        let tuple_list_append = tuple_f
            .instructions
            .iter()
            .find_map(|unit| match unit.op {
                Instruction::ListAppend { .. } => Some(u32::from(u8::from(unit.arg))),
                _ => None,
            })
            .expect("tuple(genexpr) fast path should emit LIST_APPEND");
        assert_eq!(tuple_list_append, 2);

        let list_f = find_code(&code, "list_f").expect("missing list_f code");
        assert!(
            list_f
                .instructions
                .iter()
                .any(|unit| matches!(unit.op, Instruction::Call { .. })),
            "list(genexpr) should stay on the normal call path"
        );
        assert!(
            !has_common_constant(list_f, bytecode::CommonConstant::BuiltinList),
            "CPython 3.14.2 does not optimize list(genexpr)"
        );

        let set_f = find_code(&code, "set_f").expect("missing set_f code");
        assert!(
            set_f
                .instructions
                .iter()
                .any(|unit| matches!(unit.op, Instruction::Call { .. })),
            "set(genexpr) should stay on the normal call path"
        );
        assert!(
            !has_common_constant(set_f, bytecode::CommonConstant::BuiltinSet),
            "CPython 3.14.2 does not optimize set(genexpr)"
        );
    }

    #[test]
    fn test_builtin_tuple_genexpr_try_assignment_uses_shared_tail() {
        let code = compile_exec(
            "\
def f(xs):
    global y
    try:
        y = tuple(int(i) for i in xs.split('.'))
    except ValueError:
        y = ()
    return y
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();
        let intrinsic = ops
            .iter()
            .position(|op| matches!(op, Instruction::CallIntrinsic1 { .. }))
            .expect("tuple(genexpr) fast path should emit LIST_TO_TUPLE");
        let first_fallback = ops[intrinsic + 1..]
            .iter()
            .position(|op| matches!(op, Instruction::PushNull))
            .map(|offset| intrinsic + 1 + offset)
            .expect("shadowed tuple fallback call should remain after fast path");
        let first_store = ops[intrinsic + 1..]
            .iter()
            .position(|op| matches!(op, Instruction::StoreGlobal { .. }))
            .map(|offset| intrinsic + 1 + offset)
            .expect("tuple(genexpr) result should be stored after fast or fallback call");

        assert!(
            matches!(ops[intrinsic + 1], Instruction::JumpForward { .. })
                && first_fallback < first_store,
            "tuple(genexpr) fast path should jump over fallback to CPython-style shared store tail, got ops={ops:?}"
        );
    }

    #[test]
    fn test_module_store_uses_store_global_when_nested_scope_declares_global() {
        let code = compile_exec(
            "\
_address_fmt_re = None

class C:
    def f(self):
        global _address_fmt_re
        if _address_fmt_re is None:
            _address_fmt_re = 1
",
        );

        assert!(code.instructions.iter().any(|unit| match unit.op {
            Instruction::StoreGlobal { namei } => {
                let idx = namei.get(OpArg::new(u32::from(u8::from(unit.arg))));
                code.names[usize::try_from(idx).unwrap()].as_str() == "_address_fmt_re"
            }
            _ => false,
        }));
    }

    #[test]
    fn test_conditional_return_epilogue_is_duplicated() {
        let code = compile_exec(
            "\
def f(base, cls, state):
    if base is object:
        obj = object.__new__(cls)
    else:
        obj = base.__new__(cls, state)
    return obj
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let return_count = f
            .instructions
            .iter()
            .filter(|unit| matches!(unit.op, Instruction::ReturnValue))
            .count();

        assert_eq!(return_count, 2);
    }

    #[test]
    fn test_loop_store_subscr_threads_direct_backedge() {
        let code = compile_exec(
            "\
def f(kwonlyargs, kw_only_defaults, arg2value):
    missing = 0
    for kwarg in kwonlyargs:
        if kwarg not in arg2value:
            if kw_only_defaults and kwarg in kw_only_defaults:
                arg2value[kwarg] = kw_only_defaults[kwarg]
            else:
                missing += 1
    return missing
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        let store_subscr = ops
            .iter()
            .position(|op| matches!(op, Instruction::StoreSubscr))
            .expect("missing STORE_SUBSCR");
        let next_op = ops
            .get(store_subscr + 1)
            .expect("missing jump after STORE_SUBSCR");
        let window_start = store_subscr.saturating_sub(3);
        let window_end = (store_subscr + 5).min(ops.len());
        let window = &ops[window_start..window_end];

        assert!(
            matches!(next_op, Instruction::JumpBackward { .. }),
            "expected direct loop backedge after STORE_SUBSCR, got {next_op:?}; ops={window:?}"
        );
    }

    #[test]
    fn test_protected_store_subscr_tail_uses_strong_loads() {
        let code = compile_exec(
            "\
def f(cache, lock, format):
    with lock:
        format_regex = cache.get(format)
        if not format_regex:
            try:
                format_regex = cache.compile(format)
            except KeyError as err:
                bad_directive = err.args[0]
                del err
                raise ValueError(bad_directive) from None
            cache[format] = format_regex
    return format_regex.match('x')
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(4).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::LoadFastLoadFast { .. },
                        Instruction::LoadFast { .. },
                        Instruction::StoreSubscr,
                        Instruction::LoadConst { .. },
                    ]
                )
            }),
            "expected CPython-style strong loads before protected STORE_SUBSCR tail, got ops={ops:?}"
        );

        let code = compile_exec(
            "\
cache = {}
def g(lock, format):
    with lock:
        format_regex = cache.get(format)
        if not format_regex:
            try:
                format_regex = compile(format)
            except KeyError as err:
                bad_directive = err.args[0]
                del err
                raise ValueError(bad_directive) from None
            cache[format] = format_regex
    return format_regex.match('x')
",
        );
        let g = find_code(&code, "g").expect("missing function code");
        let ops: Vec<_> = g
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(4).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::LoadFast { .. },
                        Instruction::LoadGlobal { .. },
                        Instruction::LoadFast { .. },
                        Instruction::StoreSubscr,
                    ]
                )
            }),
            "expected CPython-style strong value/key loads around global STORE_SUBSCR tail, got ops={ops:?}"
        );
    }

    #[test]
    fn test_augassign_two_part_slice_uses_slice_opcodes() {
        let code = compile_exec(
            "\
def aug(x, a, b, y):
    x[a:b] += y
",
        );
        let aug = find_code(&code, "aug").expect("missing aug code");
        let ops: Vec<_> = aug
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert_eq!(
            ops.iter()
                .filter(|op| matches!(op, Instruction::BinarySlice))
                .count(),
            1,
            "expected one BINARY_SLICE in augassign slice path, got ops={ops:?}"
        );
        assert_eq!(
            ops.iter()
                .filter(|op| matches!(op, Instruction::StoreSlice))
                .count(),
            1,
            "expected one STORE_SLICE in augassign slice path, got ops={ops:?}"
        );
        assert!(
            !ops.iter().any(|op| {
                matches!(
                    op,
                    Instruction::BuildSlice { .. } | Instruction::StoreSubscr
                )
            }),
            "two-part augassign slice should avoid BUILD_SLICE/STORE_SUBSCR, got ops={ops:?}"
        );
        assert!(
            ops.windows(10).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::Copy { .. },
                        Instruction::Copy { .. },
                        Instruction::Copy { .. },
                        Instruction::BinarySlice,
                        Instruction::LoadFastBorrow { .. } | Instruction::LoadFast { .. },
                        Instruction::BinaryOp { .. },
                        Instruction::Swap { .. },
                        Instruction::Swap { .. },
                        Instruction::Swap { .. },
                        Instruction::StoreSlice,
                    ]
                )
            }),
            "expected CPython-style augassign slice window, got ops={ops:?}"
        );
    }

    #[test]
    fn test_loop_return_reorders_backedge_before_exit_cleanup() {
        let code = compile_exec(
            "\
def f(obj):
    for base in obj.__mro__:
        if base is not object:
            doc = base.__doc__
            if doc is not None:
                return doc
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        let has_cpython_shape = ops.windows(7).any(|window| {
            matches!(
                window,
                [
                    Instruction::PopJumpIfNotNone { .. },
                    Instruction::NotTaken,
                    Instruction::JumpBackward { .. },
                    Instruction::LoadFastBorrow { .. } | Instruction::LoadFast { .. },
                    Instruction::Swap { .. },
                    Instruction::PopTop,
                    Instruction::ReturnValue,
                ]
            )
        });
        let has_conservative_shape = ops.windows(9).any(|window| {
            matches!(
                window,
                [
                    Instruction::PopJumpIfNone { .. },
                    Instruction::NotTaken,
                    Instruction::LoadFastBorrow { .. } | Instruction::LoadFast { .. },
                    Instruction::Swap { .. },
                    Instruction::PopTop,
                    Instruction::ReturnValue,
                    Instruction::Nop,
                    Instruction::JumpBackward { .. },
                    Instruction::EndFor,
                ]
            )
        });
        assert!(
            has_cpython_shape || has_conservative_shape,
            "expected loop return null-check to keep the backedge adjacent to the return cleanup, got ops={ops:?}"
        );

        let end_for_idx = ops
            .iter()
            .position(|op| matches!(op, Instruction::EndFor))
            .expect("missing END_FOR");
        let return_before_end = ops[..end_for_idx]
            .iter()
            .rposition(|op| matches!(op, Instruction::ReturnValue))
            .expect("missing loop-body RETURN_VALUE");
        assert!(
            matches!(ops.get(return_before_end - 1), Some(Instruction::PopTop)),
            "expected POP_TOP before loop-body RETURN_VALUE, got {:?}; ops={ops:?}",
            ops.get(return_before_end.saturating_sub(1))
        );
    }

    #[test]
    fn test_nested_try_finally_cleanup_reorder_does_not_invert_forward_jumps() {
        compile_exec(include_str!("../../../Lib/poplib.py"));
    }

    #[test]
    fn test_conditional_body_is_preserved_before_final_return() {
        let code = compile_exec(
            "\
def f(x, y):
    if x == y:
        print('then', flush=True)
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        let cond_idx = ops
            .iter()
            .position(|op| matches!(op, Instruction::PopJumpIfFalse { .. }))
            .expect("missing POP_JUMP_IF_FALSE");
        let first_return_idx = ops
            .iter()
            .position(|op| matches!(op, Instruction::ReturnValue))
            .expect("missing RETURN_VALUE");

        assert!(
            ops[cond_idx..first_return_idx]
                .iter()
                .any(|op| matches!(op, Instruction::CallKw { .. })),
            "expected conditional body call before final return, got ops={ops:?}"
        );
    }

    #[test]
    fn test_nested_conditional_body_is_preserved_before_final_return() {
        let code = compile_exec(
            "\
def outer():
    def side():
        print('side', flush=True)
    def cb():
        flag = True
        if flag:
            side()
    return cb
",
        );
        let cb = find_code(&code, "cb").expect("missing nested cb code");
        let ops: Vec<_> = cb
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        let cond_idx = ops
            .iter()
            .position(|op| matches!(op, Instruction::PopJumpIfFalse { .. }))
            .expect("missing POP_JUMP_IF_FALSE");
        let first_return_idx = ops
            .iter()
            .position(|op| matches!(op, Instruction::ReturnValue))
            .expect("missing RETURN_VALUE");

        assert!(
            ops[cond_idx..first_return_idx]
                .iter()
                .any(|op| matches!(op, Instruction::Call { .. })),
            "expected nested conditional body call before final return, got ops={ops:?}"
        );
    }

    #[test]
    fn test_try_line_nop_is_preserved_before_setup_finally() {
        let code = compile_exec(
            "\
def f(msg):
    try:
        fw = _wm.formatwarning
    except AttributeError:
        pass
    else:
        if fw is not _formatwarning_orig:
            return fw(msg.message, msg.category, msg.filename, msg.lineno, msg.line)
    return _wm._formatwarnmsg_impl(msg)
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            matches!(
                ops.as_slice(),
                [Instruction::Resume { .. }, Instruction::Nop, ..]
            ),
            "expected CPython try-line NOP before setup/fetch, got ops={ops:?}"
        );
    }

    #[test]
    fn test_nested_try_line_nops_after_for_cleanup_are_preserved() {
        let code = compile_exec(
            "\
def f(xs, env):
    for x in xs:
        pass
    try:
        try:
            if env is not None:
                env_list = []
            else:
                env_list = None
        finally:
            pass
    finally:
        pass
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(6).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::EndFor,
                        Instruction::PopIter,
                        Instruction::Nop,
                        Instruction::Nop,
                        Instruction::LoadFastBorrow { .. } | Instruction::LoadFast { .. },
                        Instruction::PopJumpIfNone { .. },
                    ]
                )
            }),
            "expected CPython-style outer and inner try-line NOPs after for cleanup, got ops={ops:?}"
        );
    }

    #[test]
    fn test_try_finally_if_break_false_edge_keeps_finalbody_entry_nop() {
        let code = compile_exec(
            "\
def f(self, pid):
    while True:
        try:
            if pid == self.pid:
                self.h()
                break
        finally:
            self.r()
        self.g()
    return self.x
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(6).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::ReturnValue,
                        Instruction::Nop,
                        Instruction::LoadFastBorrow { .. } | Instruction::LoadFast { .. },
                        Instruction::LoadAttr { .. },
                        Instruction::Call { .. },
                        Instruction::PopTop,
                    ]
                )
            }),
            "expected CPython-style if-line NOP before fallthrough finally body, got ops={ops:?}"
        );
    }

    #[test]
    fn test_try_percent_format_preprocess_removes_redundant_try_nop() {
        let code = compile_exec(
            "\
def f(self, signal):
    if self.returncode and self.returncode < 0:
        try:
            return \"Command '%s' died with %r.\" % (
                self.cmd, signal.Signals(-self.returncode))
        except ValueError:
            return \"Command '%s' died with unknown signal %d.\" % (
                self.cmd, -self.returncode)
    return \"Command '%s' returned non-zero exit status %d.\" % (
        self.cmd, self.returncode)
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(3).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::NotTaken,
                        Instruction::LoadConst { .. },
                        Instruction::LoadFastBorrow { .. } | Instruction::LoadFast { .. },
                    ]
                )
            }),
            "expected preprocessed percent-format body immediately after condition, got ops={ops:?}"
        );
        assert!(
            !ops.windows(4).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::NotTaken,
                        Instruction::Nop,
                        Instruction::LoadConst { .. },
                        Instruction::LoadFastBorrow { .. } | Instruction::LoadFast { .. },
                    ]
                )
            }),
            "percent-format preprocessing should let CFG remove the try-line NOP, got ops={ops:?}"
        );
    }

    #[test]
    fn test_nested_try_except_in_finally_exception_path_shares_continuation() {
        let code = compile_exec(
            "\
def f(self, exc_type, KeyboardInterrupt, TimeoutExpired):
    try:
        if self.stdin:
            self.stdin.close()
    finally:
        if exc_type == KeyboardInterrupt:
            if self._sigint_wait_secs > 0:
                try:
                    self._wait(timeout=self._sigint_wait_secs)
                except TimeoutExpired:
                    pass
            self._sigint_wait_secs = 0
        else:
            self.wait()
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();
        let store_reraise_tails = ops
            .windows(2)
            .filter(|window| {
                matches!(
                    window,
                    [Instruction::StoreAttr { .. }, Instruction::Reraise { .. },]
                )
            })
            .count();

        assert_eq!(
            store_reraise_tails, 1,
            "nested try/except inside an exceptional finally body should share the remaining finalbody tail before RERAISE, got ops={ops:?}"
        );
        assert!(
            ops.windows(5).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::LoadSmallInt { .. },
                        Instruction::LoadFastBorrow { .. },
                        Instruction::StoreAttr { .. },
                        Instruction::LoadConst { .. },
                        Instruction::ReturnValue,
                    ]
                )
            }),
            "normal finally body should keep CPython-style borrowed load before STORE_ATTR, got ops={ops:?}"
        );
    }

    #[test]
    fn test_try_else_return_keeps_nop_before_final_call_return() {
        let code = compile_exec(
            "\
def f(msg):
    try:
        fw = _wm.formatwarning
    except AttributeError:
        pass
    else:
        if fw is not _formatwarning_orig:
            return fw(msg.message, msg.category, msg.filename, msg.lineno, msg.line)
    return _wm._formatwarnmsg_impl(msg)
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(7).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::ReturnValue,
                        Instruction::Nop,
                        Instruction::LoadGlobal { .. },
                        Instruction::LoadAttr { .. },
                        Instruction::LoadFastBorrow { .. } | Instruction::LoadFast { .. },
                        Instruction::Call { .. },
                        Instruction::ReturnValue,
                    ]
                )
            }),
            "expected CPython-style NOP between conditional return and final call return, got ops={ops:?}"
        );
    }

    #[test]
    fn test_conditional_compare_uses_bool_compare_oparg() {
        let code = compile_exec(
            "\
def f(x, y):
    if x == y:
        return 1
    return 0
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let compare = f
            .instructions
            .iter()
            .find(|unit| matches!(unit.op, Instruction::CompareOp { .. }))
            .expect("missing COMPARE_OP");

        assert_eq!(u8::from(compare.arg), 88);
    }

    #[test]
    fn test_multiline_is_none_conditional_keeps_comparator_nop() {
        let code = compile_exec(
            "\
def f(x):
    if x.find(
            'a') is not None:
        return 1
    return 0
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(3).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::Call { .. },
                        Instruction::Nop,
                        Instruction::PopJumpIfNone { .. },
                    ]
                )
            }),
            "expected CPython-style comparator NOP before folded POP_JUMP_IF_NONE, got ops={ops:?}"
        );
    }

    #[test]
    fn test_chained_conditional_compares_use_bool_compare_oparg() {
        let code = compile_exec(
            "\
def f(a, b, c):
    if a < b < c:
        return 1
    return 0
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let compare_args: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| matches!(unit.op, Instruction::CompareOp { .. }))
            .map(|unit| u8::from(unit.arg))
            .collect();

        assert_eq!(compare_args, vec![18, 18]);
    }

    #[test]
    fn test_shared_final_return_is_cloned_for_jump_target() {
        let code = compile_exec(
            "\
def f(node):
    if not isinstance(
        node, (AsyncFunctionDef, FunctionDef, ClassDef, Module)
    ) or len(node.body) < 1:
        return None
    node = node.body[0]
    if not isinstance(node, Expr):
        return None
    node = node.value
    if isinstance(node, Constant) and isinstance(node.value, str):
        return node
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        let return_count = ops
            .iter()
            .filter(|op| matches!(op, Instruction::ReturnValue))
            .count();
        assert_eq!(
            return_count, 5,
            "expected cloned return sites for each shared return edge, got ops={ops:?}"
        );
    }

    #[test]
    fn test_for_break_uses_poptop_cleanup() {
        let code = compile_exec(
            "\
def f(parts):
    for value in parts:
        if value:
            break
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        let pop_iter_count = ops
            .iter()
            .filter(|op| matches!(op, Instruction::PopIter))
            .count();
        assert_eq!(
            pop_iter_count, 1,
            "expected only the loop-exhaustion POP_ITER, got ops={ops:?}"
        );

        let break_cleanup_idx = ops
            .windows(3)
            .position(|window| {
                matches!(
                    window,
                    [
                        Instruction::PopTop,
                        Instruction::LoadConst { .. },
                        Instruction::ReturnValue
                    ]
                )
            })
            .expect("missing POP_TOP/LOAD_CONST/RETURN_VALUE break cleanup");
        let end_for_idx = ops
            .iter()
            .position(|op| matches!(op, Instruction::EndFor))
            .expect("missing END_FOR");
        assert!(
            break_cleanup_idx < end_for_idx,
            "expected break cleanup before END_FOR, got ops={ops:?}"
        );
    }

    #[test]
    fn test_for_exit_before_elif_does_not_leave_line_anchor_nop() {
        let code = compile_exec(
            "\
from sys import maxsize
if maxsize == 2147483647:
    for s in ('2147483648', '0o40000000000', '0x100000000', '0b10000000000000000000000000000000'):
        try:
            x = eval(s)
        except OverflowError:
            fail('OverflowError on huge integer literal %r' % s)
elif maxsize == 9223372036854775807:
    pass
",
        );
        let ops: Vec<_> = code
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(4).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::EndFor,
                        Instruction::PopIter,
                        Instruction::LoadConst { .. },
                        Instruction::ReturnValue,
                    ]
                )
            }),
            "expected for-exit epilogue without extra NOP, got ops={ops:?}"
        );
        assert!(
            !ops.windows(4).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::EndFor,
                        Instruction::PopIter,
                        Instruction::Nop,
                        Instruction::LoadConst { .. },
                    ]
                )
            }),
            "unexpected line-anchor NOP before for-exit epilogue, got ops={ops:?}"
        );
    }

    #[test]
    fn test_for_tuple_target_does_not_leave_loop_header_nop() {
        let code = compile_exec(
            "\
def f(pairs):
    for left, right in pairs:
        pass
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(2).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::ForIter { .. },
                        Instruction::UnpackSequence { .. }
                    ]
                )
            }),
            "expected FOR_ITER to flow directly into UNPACK_SEQUENCE, got ops={ops:?}"
        );
        assert!(
            !ops.windows(3).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::ForIter { .. },
                        Instruction::Nop,
                        Instruction::UnpackSequence { .. },
                    ]
                )
            }),
            "unexpected loop-header NOP before tuple unpack, got ops={ops:?}"
        );
    }

    #[test]
    fn test_tstring_build_template_matches_cpython_stack_order() {
        let code = compile_exec("t = t\"{0}\"");
        let units: Vec<_> = code
            .instructions
            .iter()
            .copied()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();

        assert!(
            units.windows(6).any(|window| {
                matches!(
                    window,
                    [
                        a,
                        b,
                        c,
                        d,
                        e,
                        f,
                    ]
                    if matches!(a.op, Instruction::LoadConst { .. })
                        && matches!(b.op, Instruction::LoadSmallInt { .. })
                        && matches!(c.op, Instruction::LoadConst { .. })
                        && matches!(d.op, Instruction::BuildInterpolation { .. })
                        && u8::from(d.arg) == 2
                        && matches!(e.op, Instruction::BuildTuple { .. })
                        && u8::from(e.arg) == 1
                        && matches!(f.op, Instruction::BuildTemplate)
                )
            }),
            "expected CPython-style t-string lowering, got units={units:?}"
        );
        assert!(
            !units
                .iter()
                .any(|unit| matches!(unit.op, Instruction::Swap { .. })),
            "unexpected SWAP in t-string lowering, got units={units:?}"
        );
    }

    #[test]
    fn test_tstring_debug_specifier_uses_debug_literal_and_repr_default() {
        let code = compile_exec(
            "\
value = 42
t = t\"Value: {value=}\"
",
        );

        let string_consts = code
            .instructions
            .iter()
            .filter_map(|unit| match unit.op {
                Instruction::LoadConst { consti } => {
                    Some(&code.constants[consti.get(OpArg::new(u32::from(u8::from(unit.arg))))])
                }
                _ => None,
            })
            .collect::<Vec<_>>();

        assert!(
            string_consts.iter().any(|constant| matches!(
                constant,
                ConstantData::Tuple { elements }
                    if matches!(
                        &elements[..],
                        [
                            ConstantData::Str { value: first },
                            ConstantData::Str { value: second },
                        ] if first.to_string() == "Value: value=" && second.is_empty()
                    )
            )),
            "expected debug literal prefix in t-string constants, got {string_consts:?}"
        );
        assert!(
            code.instructions.iter().any(|unit| matches!(
                unit.op,
                Instruction::BuildInterpolation { .. }
            ) && u8::from(unit.arg) == 10),
            "expected default repr conversion for debug t-string"
        );
    }

    #[test]
    fn test_tstring_literal_preserves_surrogate_wtf8() {
        let code = compile_exec("t = t\"\\ud800\"");

        assert!(code.constants.iter().any(|constant| matches!(
            constant,
            ConstantData::Str { value } if value.clone().into_bytes() == [0xED, 0xA0, 0x80]
        )));
    }

    #[test]
    fn test_break_in_finally_after_return_keeps_load_fast_check_for_loop_locals() {
        let code = compile_exec(
            "\
def g2(x):
    for count in [0, 1]:
        for count2 in [10, 20]:
            try:
                return count + count2
            finally:
                if x:
                    break
    return 'end', count, count2
",
        );
        let g2 = find_code(&code, "g2").expect("missing g2 code");
        let ops: Vec<_> = g2
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(4).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::LoadConst { .. },
                        Instruction::LoadFastCheck { .. },
                        Instruction::LoadFastCheck { .. },
                        Instruction::BuildTuple { .. },
                    ]
                )
            }),
            "expected LOAD_FAST_CHECK pair for after-return loop locals, got ops={ops:?}"
        );
    }

    #[test]
    fn test_high_index_parameter_stays_initialized_in_fast_scan() {
        let params = (0..65)
            .map(|idx| format!("p{idx}"))
            .collect::<Vec<_>>()
            .join(", ");
        let code = compile_exec(&format!(
            "\
def f({params}):
    return p64
"
        ));
        let f = find_code(&code, "f").expect("missing f code");

        assert!(
            f.instructions.iter().any(|unit| matches!(
                unit.op,
                Instruction::LoadFast { var_num } | Instruction::LoadFastBorrow { var_num }
                    if f.varnames
                        [usize::from(var_num.get(OpArg::new(u32::from(u8::from(unit.arg)))))]
                        == "p64"
            )),
            "expected high-index parameter p64 to use LOAD_FAST, got ops={:?}",
            f.instructions
                .iter()
                .map(|unit| unit.op)
                .collect::<Vec<_>>()
        );
        assert!(
            !f.instructions.iter().any(|unit| matches!(
                unit.op,
                Instruction::LoadFastCheck { var_num }
                    if f.varnames
                        [usize::from(var_num.get(OpArg::new(u32::from(u8::from(unit.arg)))))]
                        == "p64"
            )),
            "high-index parameter p64 should not use LOAD_FAST_CHECK before deletion"
        );
    }

    #[test]
    fn test_deleted_high_index_parameter_uses_load_fast_check() {
        let params = (0..65)
            .map(|idx| format!("p{idx}"))
            .collect::<Vec<_>>()
            .join(", ");
        let code = compile_exec(&format!(
            "\
def f({params}):
    del p64
    return p64
"
        ));
        let f = find_code(&code, "f").expect("missing f code");

        assert!(
            f.instructions.iter().any(|unit| matches!(
                unit.op,
                Instruction::LoadFastCheck { var_num }
                    if f.varnames
                        [usize::from(var_num.get(OpArg::new(u32::from(u8::from(unit.arg)))))]
                        == "p64"
            )),
            "expected deleted high-index parameter p64 to use LOAD_FAST_CHECK, got ops={:?}",
            f.instructions
                .iter()
                .map(|unit| unit.op)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_assert_without_message_raises_class_directly() {
        let code = compile_exec(
            "\
def f(x):
    assert x
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let call_count = f
            .instructions
            .iter()
            .filter(|unit| matches!(unit.op, Instruction::Call { .. }))
            .count();
        let push_null_count = f
            .instructions
            .iter()
            .filter(|unit| matches!(unit.op, Instruction::PushNull))
            .count();

        assert_eq!(call_count, 0);
        assert_eq!(push_null_count, 0);
    }

    #[test]
    fn test_assert_with_message_uses_common_constant_direct_call() {
        let code = compile_exec(
            "\
def f(x, y):
    assert x, y
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let load_assertion = f
            .instructions
            .iter()
            .position(|unit| {
                matches!(unit.op, Instruction::LoadCommonConstant { .. })
                    && matches!(
                        unit.op,
                        Instruction::LoadCommonConstant { idx }
                            if idx.get(OpArg::new(u32::from(u8::from(unit.arg))))
                                == bytecode::CommonConstant::AssertionError
                    )
            })
            .expect("missing LOAD_COMMON_CONSTANT AssertionError");

        assert!(
            !matches!(
                f.instructions.get(load_assertion + 1).map(|unit| unit.op),
                Some(Instruction::PushNull)
            ),
            "assert message path should not use PUSH_NULL, got ops={:?}",
            f.instructions
                .iter()
                .map(|unit| unit.op)
                .collect::<Vec<_>>()
        );
        assert!(
            matches!(
                f.instructions.get(load_assertion + 2).map(|unit| unit.op),
                Some(Instruction::Call { .. })
            ),
            "expected direct CALL after loading assert message, got ops={:?}",
            f.instructions
                .iter()
                .map(|unit| unit.op)
                .collect::<Vec<_>>()
        );

        let call_arg = f.instructions[load_assertion + 2].arg;
        assert_eq!(u8::from(call_arg), 0);
    }

    #[test]
    fn test_conditional_assert_message_target_uses_strong_load_fast() {
        let code = compile_exec(
            "\
def f(fname):
    if fname == 'a':
        return 1
    assert False, 'Unknown attrname %s' % fname
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();
        let assertion_error = ops
            .iter()
            .position(|op| matches!(op, Instruction::LoadCommonConstant { .. }))
            .expect("missing LOAD_COMMON_CONSTANT AssertionError");
        let window = &ops[assertion_error..(assertion_error + 5).min(ops.len())];
        assert!(
            matches!(
                window,
                [
                    Instruction::LoadCommonConstant { .. },
                    Instruction::LoadConst { .. },
                    Instruction::LoadFast { .. },
                    Instruction::BinaryOp { .. },
                    Instruction::Call { .. },
                    ..
                ]
            ),
            "expected CPython-style strong LOAD_FAST in targeted assert message block, got {window:?}"
        );
    }

    #[test]
    fn test_assert_message_after_condition_in_same_block_keeps_borrowed_loads() {
        let code = compile_exec(
            "\
def f(expected_ns, namespace):
    try:
        assert expected_ns == namespace, ('expected %s, got %s' % (expected_ns, namespace))
    except AssertionError as e:
        raise RuntimeError(e)
    setattr(namespace, 'spam', expected_ns)
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();
        let assertion_error = ops
            .iter()
            .position(|unit| matches!(unit.op, Instruction::LoadCommonConstant { .. }))
            .expect("missing LOAD_COMMON_CONSTANT AssertionError");
        let raise = ops[assertion_error..]
            .iter()
            .position(|unit| matches!(unit.op, Instruction::RaiseVarargs { .. }))
            .map(|idx| assertion_error + idx)
            .expect("missing assert raise");
        let message_path = &ops[assertion_error..raise];

        let load_fast_name = |unit: &&bytecode::CodeUnit| match unit.op {
            Instruction::LoadFast { var_num } => {
                let arg = OpArg::new(u32::from(u8::from(unit.arg)));
                Some(f.varnames[usize::from(var_num.get(arg))].as_str())
            }
            _ => None,
        };
        let borrow_name = |unit: &&bytecode::CodeUnit| match unit.op {
            Instruction::LoadFastBorrow { var_num } => {
                let arg = OpArg::new(u32::from(u8::from(unit.arg)));
                Some(f.varnames[usize::from(var_num.get(arg))].as_str())
            }
            _ => None,
        };

        assert!(
            message_path
                .iter()
                .filter_map(load_fast_name)
                .all(|name| name != "expected_ns" && name != "namespace"),
            "assert message after same-block condition should keep borrowed loads, got {message_path:?}"
        );
        for name in ["expected_ns", "namespace"] {
            assert!(
                message_path
                    .iter()
                    .filter_map(borrow_name)
                    .any(|var| var == name),
                "expected borrowed {name} load in assert message path, got {message_path:?}"
            );
        }

        let setattr = ops
            .iter()
            .position(|unit| matches!(unit.op, Instruction::LoadGlobal { .. }))
            .expect("missing final setattr load");
        let tail = &ops[setattr..];
        for name in ["expected_ns", "namespace"] {
            assert!(
                tail.iter()
                    .filter_map(load_fast_name)
                    .any(|var| var == name),
                "expected strong {name} load in post-try tail, got {tail:?}"
            );
        }
    }

    #[test]
    fn test_bare_function_annotations_check_attribute_and_subscript_expressions() {
        assert_dis_snapshot!(compile_exec(
            "\
def f(one: int):
    int.new_attr: int
    [list][0].new_attr: [int, str]
    my_lst = [1]
    my_lst[one]: int
    return my_lst
"
        ));
    }

    #[test]
    fn test_non_simple_bare_name_annotation_does_not_create_local_binding() {
        let code = compile_exec(
            "\
def f2bad():
    (no_such_global): int
    print(no_such_global)
",
        );
        let f = find_code(&code, "f2bad").expect("missing f2bad code");
        assert!(
            f.instructions
                .iter()
                .any(|unit| matches!(unit.op, Instruction::LoadGlobal { .. })),
            "expected LOAD_GLOBAL for non-simple bare annotated name, got ops={:?}",
            f.instructions
                .iter()
                .map(|unit| unit.op)
                .collect::<Vec<_>>()
        );
        assert!(
            !f.instructions
                .iter()
                .any(|unit| matches!(unit.op, Instruction::LoadFastCheck { .. })),
            "non-simple bare annotated name should not become a local binding, got ops={:?}",
            f.instructions
                .iter()
                .map(|unit| unit.op)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_constant_true_if_pass_keeps_line_anchor_nop() {
        assert_dis_snapshot!(compile_exec(
            "\
if 1:
    pass
"
        ));
    }

    #[test]
    fn test_negative_constant_binop_folds_after_unary_folding() {
        let code = compile_exec(
            "\
def f():
    return -2147483647 - 1
",
        );
        let f = find_code(&code, "f").expect("missing function code");

        assert!(
            !f.instructions
                .iter()
                .any(|unit| matches!(unit.op, Instruction::BinaryOp { .. })),
            "negative constant expression should fold to a single constant, got ops={:?}",
            f.instructions
                .iter()
                .map(|unit| unit.op)
                .collect::<Vec<_>>()
        );
        assert!(
            f.instructions
                .iter()
                .any(|unit| matches!(unit.op, Instruction::LoadConst { .. })),
            "expected folded constant load, got ops={:?}",
            f.instructions
                .iter()
                .map(|unit| unit.op)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_genexpr_filter_header_uses_store_fast_load_fast() {
        let code = compile_exec(
            "\
def f(it):
    return (x for x in it if x)
",
        );
        let genexpr = find_code(&code, "<genexpr>").expect("missing <genexpr> code");
        let store_fast_load_fast_idx = genexpr
            .instructions
            .iter()
            .position(|unit| matches!(unit.op, Instruction::StoreFastLoadFast { .. }))
            .expect("missing STORE_FAST_LOAD_FAST in genexpr header");

        assert!(
            matches!(
                genexpr
                    .instructions
                    .get(store_fast_load_fast_idx + 1)
                    .map(|unit| unit.op),
                Some(Instruction::ToBool)
            ),
            "expected TO_BOOL immediately after STORE_FAST_LOAD_FAST, got ops={:?}",
            genexpr
                .instructions
                .iter()
                .map(|unit| unit.op)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_generator_filter_keeps_cpython_style_forward_yield_body_entry() {
        let code = compile_exec(
            "\
def gen(it):
    for f in it:
        if f.name:
            yield f.name
",
        );
        let gen_code = find_code(&code, "gen").expect("missing gen code");
        let ops: Vec<_> = gen_code
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(7).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::ToBool,
                        Instruction::PopJumpIfTrue { .. },
                        Instruction::NotTaken,
                        Instruction::JumpBackward { .. }
                            | Instruction::JumpBackwardNoInterrupt { .. },
                        Instruction::LoadFastBorrow { .. } | Instruction::LoadFast { .. },
                        Instruction::LoadAttr { .. },
                        Instruction::YieldValue { .. },
                    ]
                )
            }),
            "expected CPython-style generator filter to jump on true into the yield body and fall through into the loop backedge on false, got ops={ops:?}"
        );
    }

    #[test]
    fn test_generator_negated_filter_keeps_cpython_style_false_edge_into_yield_body() {
        let code = compile_exec(
            "\
def gen(fields):
    for f in fields:
        if f.init and not f.kw_only:
            yield f
",
        );
        let gen_code = find_code(&code, "gen").expect("missing gen code");
        let ops: Vec<_> = gen_code
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(7).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::ToBool,
                        Instruction::PopJumpIfFalse { .. },
                        Instruction::NotTaken,
                        Instruction::JumpBackward { .. }
                            | Instruction::JumpBackwardNoInterrupt { .. },
                        Instruction::LoadFastBorrow { .. } | Instruction::LoadFast { .. },
                        Instruction::YieldValue { .. },
                        Instruction::Resume { .. },
                    ]
                )
            }),
            "expected CPython-style negated generator filter to jump on false into the yield body and fall through into the loop backedge on true, got ops={ops:?}"
        );
    }

    #[test]
    fn test_multi_with_header_uses_store_fast_load_fast() {
        let code = compile_exec(
            "\
def f(manager):
    with manager() as x, manager():
        pass
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        assert!(
            f.instructions
                .iter()
                .any(|unit| matches!(unit.op, Instruction::StoreFastLoadFast { .. })),
            "expected STORE_FAST_LOAD_FAST in multi-with header, got ops={:?}",
            f.instructions
                .iter()
                .map(|unit| unit.op)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_sequential_store_then_load_uses_store_fast_load_fast() {
        let code = compile_exec(
            "\
def f(self):
    x = ''; y = \"\"; self.assertTrue(len(x) == 0 and x == y)
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        assert!(
            f.instructions
                .iter()
                .any(|unit| matches!(unit.op, Instruction::StoreFastLoadFast { .. })),
            "expected STORE_FAST_LOAD_FAST in sequential statement body, got ops={:?}",
            f.instructions
                .iter()
                .map(|unit| unit.op)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_match_guard_capture_uses_store_fast_load_fast() {
        let code = compile_exec(
            "\
def f():
    match 0:
        case x if x:
            z = 0
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        assert!(
            f.instructions
                .iter()
                .any(|unit| matches!(unit.op, Instruction::StoreFastLoadFast { .. })),
            "expected STORE_FAST_LOAD_FAST in match guard capture path, got ops={:?}",
            f.instructions
                .iter()
                .map(|unit| unit.op)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_match_nested_capture_uses_store_fast_store_fast() {
        let code = compile_exec(
            "\
def f(x):
    match x:
        case ((0 as w) as z):
            return w, z
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        assert!(
            f.instructions
                .iter()
                .any(|unit| matches!(unit.op, Instruction::StoreFastStoreFast { .. })),
            "expected STORE_FAST_STORE_FAST in nested match capture path, got ops={:?}",
            f.instructions
                .iter()
                .map(|unit| unit.op)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_match_value_real_zero_minus_zero_complex_folds_to_negative_zero_imag() {
        let code = compile_exec(
            "\
def f(x):
    match x:
        case 0 - 0j:
            return 0
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        assert!(
            f.constants.iter().any(|constant| matches!(
                constant,
                ConstantData::Complex { value }
                    if value.re == 0.0 && value.im == 0.0 && value.im.is_sign_negative()
            )),
            "expected folded -0j constant in match value"
        );
    }

    #[test]
    fn test_match_or_uses_shared_success_block() {
        let code = compile_exec(
            "\
def http_error(status):
    match status:
        case 400:
            return 'Bad request'
        case 401 | 403 | 404:
            return 'Not allowed'
        case 418:
            return 'I am a teapot'
",
        );
        let f = find_code(&code, "http_error").expect("missing http_error code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        let jump_positions: Vec<_> = ops
            .iter()
            .enumerate()
            .filter_map(|(i, op)| matches!(op, Instruction::JumpForward { .. }).then_some(i))
            .collect();

        assert!(
            jump_positions.len() >= 4,
            "expected shared-success JumpForward ops in OR pattern, got ops={ops:?}"
        );

        let first_pop_top_pair = ops
            .windows(2)
            .position(|window| matches!(window, [Instruction::PopTop, Instruction::PopTop]))
            .expect("missing POP_TOP/POP_TOP success cleanup");

        assert!(
            jump_positions
                .iter()
                .take(3)
                .all(|&idx| idx < first_pop_top_pair),
            "expected OR-alternative jumps before shared success cleanup, got ops={ops:?}"
        );
    }

    #[test]
    fn test_match_mapping_attribute_key_keeps_plain_load_fast() {
        let code = compile_exec(
            "\
def f(self):
    class Keys:
        KEY = 'a'
    x = {'a': 0, 'b': 1}
    with self.assertRaises(ValueError):
        match x:
            case {Keys.KEY: y, 'a': z}:
                w = 0
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let key_load_idx = f
            .instructions
            .iter()
            .position(|unit| match unit.op {
                Instruction::LoadAttr { namei } => {
                    let load_attr = namei.get(OpArg::new(u32::from(u8::from(unit.arg))));
                    f.names[usize::try_from(load_attr.name_idx()).unwrap()].as_str() == "KEY"
                }
                _ => false,
            })
            .expect("missing Keys.KEY attribute load");
        let prev = f.instructions[key_load_idx - 1].op;
        assert!(
            matches!(prev, Instruction::LoadFast { .. }),
            "expected plain LOAD_FAST before Keys.KEY mapping key, got ops={:?}",
            f.instructions
                .iter()
                .map(|unit| unit.op)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    #[ignore = "debug trace for sequence star-wildcard pattern layout"]
    fn test_debug_trace_match_sequence_star_wildcard_layout() {
        let trace = compile_single_function_late_cfg_trace(
            "\
def f(w):
    match w:
        case [x, *_, y]:
            z = 0
    return x, y, z
",
            "f",
        );
        for (stage, dump) in trace {
            eprintln!("=== {stage} ===\n{dump}");
        }
    }

    #[test]
    #[ignore = "debug trace for loop bool-chain jump-back layout"]
    fn test_debug_trace_loop_break_bool_chain_layout() {
        let trace = compile_single_function_late_cfg_trace(
            "\
def f(filters, text, category, module, lineno, defaultaction):
    for item in filters:
        action, msg, cat, mod, ln = item
        if ((msg is None or msg.match(text)) and
            issubclass(category, cat) and
            (mod is None or mod.match(module)) and
            (ln == 0 or lineno == ln)):
            break
    else:
        action = defaultaction
    return action
",
            "f",
        );
        for (stage, dump) in trace {
            eprintln!("=== {stage} ===\n{dump}");
        }
    }

    #[test]
    #[ignore = "debug trace for loop conditional body jump-back layout"]
    fn test_debug_trace_loop_conditional_body_layout() {
        let trace = compile_single_function_late_cfg_trace(
            "\
def f(new, old):
    for replace in ['__module__', '__name__', '__qualname__', '__doc__']:
        if hasattr(old, replace):
            setattr(new, replace, getattr(old, replace))
    return new
",
            "f",
        );
        for (stage, dump) in trace {
            eprintln!("=== {stage} ===\n{dump}");
        }
    }

    #[test]
    #[ignore = "debug trace for minimized utf7 encode nested-if layout"]
    fn test_debug_trace_utf7_min_encode_layout() {
        let trace = compile_single_function_late_cfg_trace(
            "\
def f(s, size, encodeSetO, encodeWhiteSpace):
    inShift = True
    base64bits = 0
    out = []
    for i, ch in enumerate(s):
        if base64bits == 0:
            if i + 1 < size:
                ch2 = s[i + 1]
                if E(ch2, encodeSetO, encodeWhiteSpace):
                    if B(ch2) or ch2 == '-':
                        out.append(b'-')
                    inShift = False
            else:
                out.append(b'-')
                inShift = False
    return out
",
            "f",
        );
        for (stage, dump) in trace {
            eprintln!("=== {stage} ===\n{dump}");
        }
    }

    #[test]
    #[ignore = "debug trace for with-protected loop bool-chain layout"]
    fn test_debug_trace_with_loop_break_bool_chain_layout() {
        let trace = compile_single_function_late_cfg_trace(
            "\
def f(filters, text, category, module, lineno, defaultaction, _wm):
    with _wm._lock:
        for item in filters:
            action, msg, cat, mod, ln = item
            if ((msg is None or msg.match(text)) and
                issubclass(category, cat) and
                (mod is None or mod.match(module)) and
                (ln == 0 or lineno == ln)):
                break
        else:
            action = defaultaction
    return action
",
            "f",
        );
        for (stage, dump) in trace {
            eprintln!("=== {stage} ===\n{dump}");
        }
    }

    #[test]
    fn test_try_except_else_with_finally_keeps_with_handler_before_outer_except() {
        let code = compile_exec(
            "\
def f(i):
    try:
        1 / 0
    except ZeroDivisionError:
        print('e')
    else:
        with i as dodgy:
            print('w')
    finally:
        print('d')
",
        );
        let jumpy = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = jumpy
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        let with_except_idx = ops
            .iter()
            .position(|op| matches!(op, Instruction::WithExceptStart))
            .expect("missing WITH_EXCEPT_START");
        let check_exc_idx = ops
            .iter()
            .position(|op| matches!(op, Instruction::CheckExcMatch))
            .expect("missing CHECK_EXC_MATCH");
        assert!(
            with_except_idx < check_exc_idx,
            "expected with-except cleanup to be emitted before outer except matching like CPython, got ops={ops:?}",
        );

        let with_cleanup_end = ops
            .windows(5)
            .position(|window| {
                matches!(
                    window,
                    [
                        Instruction::LoadConst { .. },
                        Instruction::LoadConst { .. },
                        Instruction::LoadConst { .. },
                        Instruction::Call { .. },
                        Instruction::PopTop,
                    ]
                )
            })
            .expect("missing with success cleanup")
            + 5;
        assert!(
            !matches!(ops.get(with_cleanup_end), Some(Instruction::Nop)),
            "expected with success cleanup to fall straight into the surrounding continuation without a synthetic NOP target, got ops={ops:?}",
        );
    }

    #[test]
    fn test_nested_try_finally_keeps_inner_finally_cleanup_nop() {
        let code = compile_exec(
            "\
def f(a, b, d):
    try:
        try:
            a()
        finally:
            b()
    finally:
        d()
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops_lines: Vec<_> = f
            .instructions
            .iter()
            .zip(&f.locations)
            .filter_map(|(unit, (location, _))| {
                (!matches!(unit.op, Instruction::Cache)).then_some((unit.op, location.line.get()))
            })
            .collect();

        assert!(
            ops_lines.windows(3).any(|window| {
                matches!(
                    window,
                    [
                        (Instruction::PopTop, 6),
                        (Instruction::Nop, 6),
                        (
                            Instruction::LoadFastBorrow { .. } | Instruction::LoadFast { .. },
                            8
                        ),
                    ]
                )
            }),
            "expected CPython-style inner finally cleanup NOP before outer finalbody, got ops_lines={ops_lines:?}",
        );
    }

    #[test]
    fn test_try_except_finally_normal_cleanup_keeps_body_exit_nop() {
        let code = compile_exec(
            "\
def f(self, x):
    if x and self.sock:
        saved = self.sock.gettimeout()
        self.sock.settimeout(x)
        try:
            resp = self._get_line()
        except TimeoutError as err:
            raise self._timeout from err
        finally:
            self.sock.settimeout(saved)
    else:
        resp = self._get_line()
    return resp
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();
        let resp_store = ops
            .iter()
            .position(|unit| match unit.op {
                Instruction::StoreFast { var_num } => {
                    let arg = OpArg::new(u32::from(u8::from(unit.arg)));
                    f.varnames[usize::from(var_num.get(arg))] == "resp"
                }
                _ => false,
            })
            .expect("missing resp store");

        assert!(
            matches!(
                ops.get(resp_store + 1).map(|unit| unit.op),
                Some(Instruction::Nop)
            ),
            "expected CPython-style NOP between try/except normal body exit and finally cleanup, got ops={ops:?}",
        );
    }

    #[test]
    fn test_try_except_finally_suppressing_handler_drops_body_exit_nop() {
        let code = compile_exec(
            "\
def f(self):
    try:
        self.sock.shutdown(socket.SHUT_RDWR)
    except OSError as exc:
        if exc.errno != errno.ENOTCONN:
            raise
    finally:
        self.sock.close()
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();
        let shutdown_pop = ops
            .iter()
            .position(|op| matches!(op, Instruction::PopTop))
            .expect("missing shutdown POP_TOP");

        assert!(
            !matches!(ops.get(shutdown_pop + 1), Some(Instruction::Nop)),
            "suppressing except handler should fall directly into finally cleanup without a CPython body-exit NOP, got ops={ops:?}",
        );
        assert!(
            matches!(
                ops.get(shutdown_pop + 1),
                Some(Instruction::LoadFastBorrow { .. })
            ),
            "suppressing except handler should keep CPython-style borrowed finally cleanup receiver, got ops={ops:?}",
        );
    }

    #[test]
    fn test_conditional_break_finally_does_not_keep_break_cleanup_nop() {
        let code = compile_exec(
            "\
def f(tar1, x):
    try:
        while True:
            if x:
                break
            x = 1
    finally:
        tar1.close()
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops_lines: Vec<_> = f
            .instructions
            .iter()
            .zip(&f.locations)
            .filter_map(|(unit, (location, _))| {
                (!matches!(unit.op, Instruction::Cache)).then_some((unit.op, location.line.get()))
            })
            .collect();

        assert!(
            !ops_lines.windows(2).any(|window| {
                matches!(
                    window,
                    [
                        (Instruction::Nop, 5),
                        (
                            Instruction::LoadFastBorrow { .. } | Instruction::LoadFast { .. },
                            8
                        ),
                    ]
                )
            }),
            "expected CPython-style break cleanup to jump directly into finally body, got ops_lines={ops_lines:?}",
        );
    }

    #[test]
    fn test_with_break_cleanup_makes_following_jump_artificial() {
        let code = compile_exec(
            "\
def f(self):
    while self.returncode is None:
        with self._waitpid_lock:
            if self.returncode is not None:
                break
            self.work()
    return self.returncode
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops_lines: Vec<_> = f
            .instructions
            .iter()
            .zip(&f.locations)
            .filter_map(|(unit, (location, _))| {
                (!matches!(unit.op, Instruction::Cache)).then_some((unit.op, location.line.get()))
            })
            .collect();

        assert!(
            !ops_lines.windows(2).any(|window| {
                matches!(
                    window,
                    [
                        (Instruction::Nop, 5),
                        (
                            Instruction::LoadFastBorrow { .. } | Instruction::LoadFast { .. },
                            7
                        ),
                    ]
                )
            }),
            "expected CPython-style artificial jump after with-break cleanup, got ops_lines={ops_lines:?}",
        );
    }

    #[test]
    fn test_while_exit_before_with_cleanup_materializes_anchor_nop() {
        let code = compile_exec(
            "\
def f(selector, self):
    with selector:
        while selector.get_map():
            pass
    try:
        self.wait()
    except Exception:
        pass
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops_lines: Vec<_> = f
            .instructions
            .iter()
            .zip(&f.locations)
            .filter_map(|(unit, (location, _))| {
                (!matches!(unit.op, Instruction::Cache)).then_some((unit.op, location.line.get()))
            })
            .collect();

        assert!(
            ops_lines.windows(6).any(|window| {
                matches!(
                    window,
                    [
                        (Instruction::JumpBackward { .. }, 4),
                        (Instruction::Nop, 3),
                        (Instruction::LoadConst { .. }, 2),
                        (Instruction::LoadConst { .. }, 2),
                        (Instruction::LoadConst { .. }, 2),
                        (Instruction::Call { .. }, 2),
                    ]
                )
            }),
            "expected CPython-style while-exit anchor NOP before with cleanup, got ops_lines={ops_lines:?}",
        );
    }

    #[test]
    fn test_nested_boolop_same_or_prefixes_compile_without_extra_boolop_block() {
        let code = compile_exec(
            "\
def f(c, encodeO, encodeWS):
    return (
        (c > 127 or utf7_special[c] == 1)
        or (encodeWS and (utf7_special[c] == 2))
        or (encodeO and (utf7_special[c] == 3))
    )
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let pop_jump_if_true_count = f
            .instructions
            .iter()
            .filter(|unit| matches!(unit.op, Instruction::PopJumpIfTrue { .. }))
            .count();

        assert!(
            pop_jump_if_true_count >= 3,
            "expected nested boolop prefix path to compile short-circuit jumps, got ops={:?}",
            f.instructions
                .iter()
                .map(|unit| unit.op)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_nested_opposite_boolop_threads_to_fallthrough_like_cpython() {
        for source in [
            "\
def f(a, b, c):
    return ((a and b)
            or c)
",
            "\
def f(a, b, c):
    return ((a or b)
            and c)
",
        ] {
            let code = compile_exec(source);
            let f = find_code(&code, "f").expect("missing f code");
            let jumps: Vec<_> = f
                .instructions
                .iter()
                .filter(|unit| {
                    matches!(
                        unit.op,
                        Instruction::PopJumpIfFalse { .. } | Instruction::PopJumpIfTrue { .. }
                    )
                })
                .collect();

            assert_eq!(
                jumps.len(),
                2,
                "expected two conditional jumps, got {jumps:?}"
            );
            assert!(
                u8::from(jumps[0].arg) > u8::from(jumps[1].arg),
                "expected CPython-style first jump to bypass the opposite short-circuit test, got jumps={jumps:?}"
            );
        }
    }

    #[test]
    fn test_loop_or_continue_keeps_boolop_true_edge_to_continue() {
        let code = compile_exec(
            "\
def f(numpy_array, lshape, rshape, litems, fmt, tl):
    for _ in range(3):
        if numpy_array:
            if 0 in lshape or 0 in rshape:
                continue
            zl = numpy_array_from_structure(litems, fmt, tl)
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(8).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::ContainsOp { .. },
                        Instruction::PopJumpIfTrue { .. },
                        Instruction::NotTaken,
                        Instruction::LoadSmallInt { .. },
                        Instruction::LoadFastBorrow { .. } | Instruction::LoadFast { .. },
                        Instruction::ContainsOp { .. },
                        Instruction::PopJumpIfFalse { .. },
                        Instruction::NotTaken,
                    ]
                )
            }),
            "expected CPython-style `or` continue test to keep first true edge to continue, got ops={ops:?}"
        );
        assert!(
            !ops.windows(5).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::ContainsOp { .. },
                        Instruction::PopJumpIfFalse { .. },
                        Instruction::NotTaken,
                        Instruction::JumpBackward { .. }
                            | Instruction::JumpBackwardNoInterrupt { .. },
                        Instruction::LoadSmallInt { .. },
                    ]
                )
            }),
            "unexpected inverted first `or` continue condition before second operand, got ops={ops:?}"
        );
    }

    #[test]
    fn test_nested_and_or_expression_threads_same_false_short_circuit() {
        let code = compile_exec(
            "\
def f(fmt, MEMORYVIEW):
    x = len(fmt)
    return ((x == 1 or (x == 2 and fmt[0] == '@')) and
            fmt[x - 1] in MEMORYVIEW)
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let false_jumps: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| matches!(unit.op, Instruction::PopJumpIfFalse { .. }))
            .collect();

        assert!(
            false_jumps.len() >= 2,
            "expected nested boolop false jumps, got ops={:?}",
            f.instructions
                .iter()
                .map(|unit| unit.op)
                .collect::<Vec<_>>()
        );
        assert!(
            u8::from(false_jumps[0].arg) > u8::from(false_jumps[1].arg),
            "expected CPython-style same-false short-circuit threading to outer end, got false_jumps={false_jumps:?}"
        );
    }

    #[test]
    fn test_broad_exception_import_keeps_borrow_in_common_tail() {
        let code = compile_exec(
            "\
def f(msg):
    if msg.source is not None:
        try:
            import tracemalloc
        except Exception:
            suggest_tracemalloc = False
            tb = None
        suggest_tracemalloc = not tracemalloc.is_tracing()
        tb = tracemalloc.get_object_traceback(msg.source)
        if tb is not None:
            for frame in tb:
                pass
    return 0
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let import_idx = f
            .instructions
            .iter()
            .position(|unit| matches!(unit.op, Instruction::ImportName { .. }))
            .expect("missing IMPORT_NAME");

        assert!(
            f.instructions[import_idx + 1..]
                .iter()
                .any(|unit| matches!(unit.op, Instruction::LoadFastBorrow { .. })),
            "expected common tail after broad-exception import to keep LOAD_FAST_BORROW, got ops={:?}",
            f.instructions
                .iter()
                .map(|unit| unit.op)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_try_import_return_handler_deopts_common_tail_borrow() {
        let code = compile_exec(
            "\
def f():
    try:
        import pwd, grp
    except ImportError:
        return False
    if pwd.getpwuid(0)[0] != 'root':
        return False
    if grp.getgrgid(0)[0] != 'root':
        return False
    return True
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            !ops.iter()
                .any(|op| matches!(op, Instruction::LoadFastBorrow { .. })),
            "expected CPython-style LOAD_FAST after protected import common tail, got ops={ops:?}",
        );
    }

    #[test]
    fn test_try_import_pass_else_keeps_borrow() {
        let code = compile_exec(
            "\
def f(self):
    try:
        from _ctypes import set_conversion_mode
    except ImportError:
        pass
    else:
        self.prev_conv_mode = set_conversion_mode('ascii', 'strict')
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();
        let handler_start = ops
            .iter()
            .position(|op| matches!(op, Instruction::PushExcInfo))
            .expect("missing handler entry");
        let normal_tail = &ops[..handler_start];

        assert!(
            normal_tail.iter().any(|op| {
                matches!(
                    op,
                    Instruction::LoadFastBorrow { .. }
                        | Instruction::LoadFastBorrowLoadFastBorrow { .. }
                )
            }),
            "try-import pass/else normal path should keep CPython-style borrows, got tail={normal_tail:?}"
        );
    }

    #[test]
    fn test_protected_attr_direct_return_keeps_borrow() {
        let code = compile_exec(
            "\
def f(obj):
    try:
        x = 1
    except ValueError:
        return False
    return obj.values()
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();
        let handler_start = ops
            .iter()
            .position(|op| matches!(op, Instruction::PushExcInfo))
            .expect("missing handler entry");
        let protected_tail = &ops[..handler_start];

        assert!(
            protected_tail.windows(4).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::LoadFastBorrow { .. },
                        Instruction::LoadAttr { .. },
                        Instruction::Call { .. },
                        Instruction::ReturnValue,
                    ]
                )
            }),
            "expected protected direct attr-call return to keep LOAD_FAST_BORROW, got tail={protected_tail:?}"
        );
    }

    #[test]
    fn test_protected_store_normal_tail_uses_strong_loads() {
        let code = compile_exec(
            "\
def f(tarfile, tarinfo, self):
    try:
        filtered = tarfile.tar_filter(tarinfo, '')
    except UnicodeEncodeError:
        return None
    self.assertIs(filtered.name, tarinfo.name)
    return filtered
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();
        let filtered_store = ops
            .iter()
            .position(|op| matches!(op, Instruction::StoreFast { .. }))
            .expect("missing filtered store");
        let handler_start = ops
            .iter()
            .position(|op| matches!(op, Instruction::PushExcInfo))
            .expect("missing handler entry");
        let normal_tail = &ops[filtered_store + 1..handler_start];

        assert!(
            !normal_tail.iter().any(|op| matches!(
                op,
                Instruction::LoadFastBorrow { .. }
                    | Instruction::LoadFastBorrowLoadFastBorrow { .. }
            )),
            "expected CPython-style strong LOAD_FAST in protected store normal tail, got tail={normal_tail:?}",
        );
    }

    #[test]
    fn test_protected_store_finally_cleanup_keeps_borrow_tail() {
        let code = compile_exec(
            "\
def f(re, f):
    try:
        try:
            m = re.search('x', f.read())
        finally:
            f.close()
        if m is not None:
            return m.group(1)
    except OSError:
        pass
    return None
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();
        let is_m_borrow = |unit: &bytecode::CodeUnit| match unit.op {
            Instruction::LoadFastBorrow { var_num } => {
                let arg = OpArg::new(u32::from(u8::from(unit.arg)));
                f.varnames[usize::from(var_num.get(arg))].as_str() == "m"
            }
            _ => false,
        };

        assert!(
            ops.windows(3).any(|window| {
                is_m_borrow(window[0])
                    && matches!(window[1].op, Instruction::PopJumpIfNone { .. })
                    && matches!(window[2].op, Instruction::NotTaken)
            }) && ops.windows(2).any(|window| {
                is_m_borrow(window[0]) && matches!(window[1].op, Instruction::LoadAttr { .. })
            }),
            "finally cleanup RERAISE should not make the outer except deopt the normal m tail, got ops={ops:?}"
        );
    }

    #[test]
    fn test_try_else_finally_cleanup_keeps_borrow_tail() {
        let code = compile_exec(
            "\
def f(re, open):
    global _SYSTEM_VERSION
    if _SYSTEM_VERSION is None:
        _SYSTEM_VERSION = ''
        try:
            f = open('/System/Library/CoreServices/SystemVersion.plist', encoding='utf-8')
        except OSError:
            pass
        else:
            try:
                m = re.search('x', f.read())
            finally:
                f.close()
            if m is not None:
                _SYSTEM_VERSION = '.'.join(m.group(1).split('.')[:2])
    return _SYSTEM_VERSION
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();
        let is_m_borrow = |unit: &bytecode::CodeUnit| match unit.op {
            Instruction::LoadFastBorrow { var_num } => {
                let arg = OpArg::new(u32::from(u8::from(unit.arg)));
                f.varnames[usize::from(var_num.get(arg))].as_str() == "m"
            }
            _ => false,
        };

        assert!(
            ops.windows(3).any(|window| {
                is_m_borrow(window[0])
                    && matches!(window[1].op, Instruction::PopJumpIfNone { .. })
                    && matches!(window[2].op, Instruction::NotTaken)
            }) && ops.windows(2).any(|window| {
                is_m_borrow(window[0]) && matches!(window[1].op, Instruction::LoadAttr { .. })
            }),
            "try/else finally cleanup should keep CPython-style borrowed m tail, got ops={ops:?}"
        );
    }

    #[test]
    fn test_generator_protected_store_subscr_tail_uses_strong_loads() {
        let code = compile_exec(
            "\
def f(names, modules):
    for name in names:
        try:
            mod = __import__(name)
        except ImportError:
            continue
        modules[name] = mod
        yield mod
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();
        let store_subscr = ops
            .iter()
            .position(|op| matches!(op, Instruction::StoreSubscr))
            .expect("missing STORE_SUBSCR");
        let window = &ops[store_subscr.saturating_sub(2)..(store_subscr + 3).min(ops.len())];

        assert!(
            matches!(
                window,
                [
                    Instruction::LoadFastLoadFast { .. },
                    Instruction::LoadFast { .. },
                    Instruction::StoreSubscr,
                    Instruction::LoadFast { .. },
                    Instruction::YieldValue { .. },
                    ..
                ]
            ),
            "expected CPython-style strong LOAD_FAST around protected STORE_SUBSCR generator tail, got {window:?}"
        );
    }

    #[test]
    fn test_protected_call_function_ex_store_tail_uses_strong_loads() {
        let code = compile_exec(
            "\
def f(func, *args):
    try:
        result = func(*args)
    except Exception:
        return None
    return type(result)
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();
        let tail_call = ops
            .iter()
            .rposition(|op| matches!(op, Instruction::Call { .. }))
            .expect("missing tail CALL");
        let result_store = ops[..tail_call]
            .iter()
            .rposition(|op| matches!(op, Instruction::StoreFast { .. }))
            .expect("missing protected result STORE_FAST");
        let tail = &ops[result_store + 1..tail_call];

        assert!(
            tail.iter()
                .any(|op| matches!(op, Instruction::LoadFast { .. })),
            "expected CPython-style strong LOAD_FAST after protected CALL_FUNCTION_EX store, got ops={ops:?}",
        );
        assert!(
            !tail
                .iter()
                .any(|op| matches!(op, Instruction::LoadFastBorrow { .. })),
            "protected CALL_FUNCTION_EX store tail should not borrow result, got ops={ops:?}",
        );
    }

    #[test]
    fn test_protected_attr_subscript_tail_uses_strong_load_fast() {
        let code = compile_exec(
            "\
def f(obj, idx):
    try:
        x = 1
    except ValueError:
        return False
    return obj.__closure__[idx]
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();
        let handler_start = ops
            .iter()
            .position(|op| matches!(op, Instruction::PushExcInfo))
            .expect("missing handler entry");
        let protected_tail = &ops[..handler_start];

        assert!(
            !protected_tail.iter().any(|op| {
                matches!(
                    op,
                    Instruction::LoadFastBorrow { .. }
                        | Instruction::LoadFastBorrowLoadFastBorrow { .. }
                )
            }),
            "expected protected attr-subscript tail to keep strong LOAD_FAST ops, got tail={protected_tail:?}"
        );
    }

    #[test]
    fn test_protected_direct_subscript_tail_uses_strong_load_fast() {
        let code = compile_exec(
            "\
def f(seq):
    try:
        items = [int(item) for item in seq]
    except ValueError:
        return None
    return items[0] + items[1]
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();
        let handler_start = ops
            .iter()
            .position(|op| matches!(op, Instruction::PushExcInfo))
            .expect("missing handler entry");
        let protected_store = ops[..handler_start]
            .iter()
            .rposition(|op| matches!(op, Instruction::StoreFast { .. }))
            .expect("missing protected local store");
        let tail = &ops[protected_store + 1..handler_start];

        assert!(
            !tail.iter().any(|op| {
                matches!(
                    op,
                    Instruction::LoadFastBorrow { .. }
                        | Instruction::LoadFastBorrowLoadFastBorrow { .. }
                )
            }),
            "expected protected direct-subscript tail to keep strong LOAD_FAST ops, got tail={tail:?}"
        );
    }

    #[test]
    fn test_protected_attr_iter_chain_uses_strong_load_fast() {
        let code = compile_exec(
            "\
def f(fields):
    try:
        x = 1
    except ValueError:
        return False
    return tuple(v for v in fields.values())
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();
        let handler_start = ops
            .iter()
            .position(|op| matches!(op, Instruction::PushExcInfo))
            .expect("missing handler entry");
        let protected_tail = &ops[..handler_start];

        assert!(
            !protected_tail.iter().any(|op| {
                matches!(
                    op,
                    Instruction::LoadFastBorrow { .. }
                        | Instruction::LoadFastBorrowLoadFastBorrow { .. }
                )
            }),
            "expected protected attr-iter chain to keep strong LOAD_FAST ops, got tail={protected_tail:?}"
        );
    }

    #[test]
    fn test_generator_except_return_handler_deopts_normal_tail_borrows() {
        let code = compile_exec(
            "\
def f(fields):
    try:
        x = 1
    except ValueError:
        return
    for fielddesc in fields:
        yield fielddesc
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();
        let handler_start = ops
            .iter()
            .position(|op| matches!(op, Instruction::PushExcInfo))
            .expect("missing handler entry");
        let normal_tail = &ops[..handler_start];

        assert!(
            normal_tail
                .iter()
                .any(|op| matches!(op, Instruction::LoadFast { .. })),
            "generator tail after non-yielding except return should keep CPython-style strong LOAD_FAST, got tail={normal_tail:?}"
        );
        assert!(
            !normal_tail.iter().any(|op| {
                matches!(
                    op,
                    Instruction::LoadFastBorrow { .. }
                        | Instruction::LoadFastBorrowLoadFastBorrow { .. }
                )
            }),
            "generator tail after non-yielding except return should not borrow, got tail={normal_tail:?}"
        );
    }

    #[test]
    fn test_generator_except_yielding_handler_keeps_normal_tail_borrows() {
        let code = compile_exec(
            "\
def f(tp, parent=None):
    try:
        fields = tp._fields_
    except AttributeError:
        yield parent
    else:
        for fielddesc in fields:
            yield fielddesc
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();
        let handler_start = ops
            .iter()
            .position(|op| matches!(op, Instruction::PushExcInfo))
            .expect("missing handler entry");
        let normal_tail = &ops[..handler_start];

        assert!(
            normal_tail.iter().any(|op| {
                matches!(
                    op,
                    Instruction::LoadFastBorrow { .. }
                        | Instruction::LoadFastBorrowLoadFastBorrow { .. }
                )
            }),
            "generator tail after yielding except handler should keep CPython-style borrows, got tail={normal_tail:?}"
        );
    }

    #[test]
    fn test_generator_except_pass_resume_tail_keeps_borrows() {
        let code = compile_exec(
            "\
def f(self, msg):
    if self.log_queue is not None:
        yield
        output = []
        try:
            while True:
                output.append(self.log_queue.get_nowait().getMessage())
        except queue.Empty:
            pass
    else:
        with self.assertLogs('concurrent.futures', 'CRITICAL') as cm:
            yield
        output = cm.output
    self.assertTrue(any(msg in line for line in output), output)
",
        );
        let f = find_code(&code, "f").expect("missing f code");

        let has_strong_load = |name: &str| {
            f.instructions.iter().any(|unit| match unit.op {
                Instruction::LoadFast { var_num } => {
                    let arg = OpArg::new(u32::from(u8::from(unit.arg)));
                    f.varnames[usize::from(var_num.get(arg))] == name
                }
                _ => false,
            })
        };
        let has_borrow_load = |name: &str| {
            f.instructions.iter().any(|unit| match unit.op {
                Instruction::LoadFastBorrow { var_num } => {
                    let arg = OpArg::new(u32::from(u8::from(unit.arg)));
                    f.varnames[usize::from(var_num.get(arg))] == name
                }
                _ => false,
            })
        };

        for name in ["msg", "output"] {
            assert!(
                has_borrow_load(name),
                "generator except-pass resume tail should borrow {name}, got instructions={:?}",
                f.instructions
            );
            assert!(
                !has_strong_load(name),
                "generator except-pass resume tail should not force strong LOAD_FAST for {name}, got instructions={:?}",
                f.instructions
            );
        }
    }

    #[test]
    fn test_async_for_cleanup_resume_tail_uses_strong_loads() {
        let code = compile_exec(
            "\
async def f(g, self, x):
    async for val in g:
        break
    self.x(x)
    await g.aclose()
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let instructions: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();
        let ops: Vec<_> = instructions.iter().map(|unit| unit.op).collect();
        let aclose_idx = instructions
            .iter()
            .position(|unit| match unit.op {
                Instruction::LoadAttr { namei } => {
                    let load_attr = namei.get(OpArg::new(u32::from(u8::from(unit.arg))));
                    f.names[usize::try_from(load_attr.name_idx()).unwrap()].as_str() == "aclose"
                }
                _ => false,
            })
            .expect("missing aclose load");

        assert!(
            ops.windows(4).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::LoadFast { .. },
                        Instruction::LoadAttr { .. },
                        Instruction::LoadFast { .. },
                        Instruction::Call { .. },
                    ]
                )
            }),
            "async-for cleanup resume tail should use strong LOAD_FAST ops before the await, got ops={ops:?}"
        );
        assert!(
            matches!(
                instructions
                    .get(aclose_idx.saturating_sub(1))
                    .map(|unit| unit.op),
                Some(Instruction::LoadFast { .. })
            ),
            "async-for cleanup resume tail should keep g strong before aclose, got ops={ops:?}"
        );
    }

    #[test]
    fn test_async_generator_async_with_yield_keeps_borrow() {
        let code = compile_exec(
            "\
async def f(self, my_cm):
    async with self.exit_stack() as stack:
        await stack.enter_async_context(my_cm())
        yield stack
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let instructions: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();
        let ops: Vec<_> = instructions.iter().map(|unit| unit.op).collect();
        let wrap_idx = instructions
            .iter()
            .position(|unit| match unit.op {
                Instruction::CallIntrinsic1 { func } => {
                    func.get(OpArg::new(u32::from(u8::from(unit.arg))))
                        == IntrinsicFunction1::AsyncGenWrap
                }
                _ => false,
            })
            .expect("missing async generator wrap");

        assert!(
            matches!(
                ops.get(wrap_idx.saturating_sub(1)),
                Some(Instruction::LoadFastBorrow { .. })
            ),
            "async generator yield inside async-with should borrow stack like CPython, got ops={ops:?}"
        );
    }

    #[test]
    fn test_deoptimized_async_with_enter_continuation_uses_strong_loads() {
        let code = compile_exec(
            "\
async def f():
    async def cm():
        pass
    try:
        async with cm():
            1 / 0
    except ZeroDivisionError as e:
        frames = e
    class E(RuntimeError):
        pass
    try:
        async with cm():
            raise E(42)
    except E as e:
        frames = e
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(5).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::LoadFast { .. },
                        Instruction::PushNull,
                        Instruction::LoadSmallInt { .. },
                        Instruction::Call { .. },
                        Instruction::RaiseVarargs { .. },
                    ]
                )
            }),
            "async-with enter continuation after a deoptimized setup block should keep raised class strong, got ops={ops:?}"
        );
    }

    #[test]
    fn test_async_with_bare_raise_continuation_keeps_borrow() {
        let code = compile_exec(
            "\
async def f(tg):
    class E(Exception):
        pass
    try:
        async with tg:
            raise E
    except ExceptionGroup:
        pass
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();
        let raise_idx = ops
            .iter()
            .position(|op| matches!(op, Instruction::RaiseVarargs { .. }))
            .expect("missing raise");

        assert!(
            matches!(
                ops.get(raise_idx.saturating_sub(1)),
                Some(Instruction::LoadFastBorrow { .. })
            ),
            "bare async-with raise continuation should keep the raised class borrowed like CPython, got ops={ops:?}"
        );
    }

    #[test]
    fn test_except_star_tail_uses_strong_loads() {
        let code = compile_exec(
            "\
def f(self):
    try:
        pass
    except* ValueError:
        pass
    self.fail('x')
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(4).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::LoadFast { .. },
                        Instruction::LoadAttr { .. },
                        Instruction::LoadConst { .. },
                        Instruction::Call { .. },
                    ]
                )
            }),
            "except* tail should use strong LOAD_FAST like CPython, got ops={ops:?}"
        );
        assert!(
            !ops.windows(4).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::LoadFastBorrow { .. },
                        Instruction::LoadAttr { .. },
                        Instruction::LoadConst { .. },
                        Instruction::Call { .. },
                    ]
                )
            }),
            "except* tail should not borrow the receiver after the handler region, got ops={ops:?}"
        );
    }

    #[test]
    fn test_protected_attr_subscript_store_tail_uses_strong_load_fast() {
        let code = compile_exec(
            "\
def f(f, oldcls, newcls):
    try:
        idx = f.__code__.co_freevars.index('__class__')
    except ValueError:
        return False
    closure = f.__closure__[idx]
    if closure.cell_contents is oldcls:
        closure.cell_contents = newcls
        return True
    return False
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();
        let handler_start = ops
            .iter()
            .position(|op| matches!(op, Instruction::PushExcInfo))
            .expect("missing handler entry");
        let protected_tail = &ops[..handler_start];
        let store_closure_idx = protected_tail
            .windows(2)
            .position(|window| {
                matches!(
                    window,
                    [Instruction::BinaryOp { .. }, Instruction::StoreFast { .. }]
                )
            })
            .map(|idx| idx + 1)
            .expect("missing STORE_FAST for closure");
        let post_store_tail = &protected_tail[store_closure_idx + 1..];

        assert!(
            !post_store_tail.iter().any(|op| {
                matches!(
                    op,
                    Instruction::LoadFastBorrow { .. }
                        | Instruction::LoadFastBorrowLoadFastBorrow { .. }
                )
            }),
            "expected protected attr-subscript store tail to keep strong LOAD_FAST ops, got tail={post_store_tail:?}"
        );
    }

    #[test]
    fn test_plain_attr_subscript_tail_keeps_borrow() {
        let code = compile_exec(
            "\
def f(self, name):
    annotations = self.method_annotations[name]
    return annotations
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(4).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::LoadFastBorrow { .. },
                        Instruction::LoadAttr { .. },
                        Instruction::LoadFastBorrow { .. },
                        Instruction::BinaryOp { .. },
                    ]
                )
            }),
            "expected plain attr-subscript tail to keep borrowed receiver/index loads, got ops={ops:?}"
        );
    }

    #[test]
    fn test_plain_attr_iter_chain_keeps_borrow() {
        let code = compile_exec(
            "\
def f(fields):
    return tuple(v for v in fields.values())
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(4).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::LoadFastBorrow { .. },
                        Instruction::LoadAttr { .. },
                        Instruction::Call { .. },
                        Instruction::GetIter,
                    ]
                )
            }),
            "expected plain attr-iter chain to keep borrowed receiver, got ops={ops:?}"
        );
    }

    #[test]
    fn test_genexpr_true_filter_omits_bool_scaffolding() {
        let code = compile_exec(
            "\
def f(it):
    return (x for x in it if True)
",
        );
        let genexpr = find_code(&code, "<genexpr>").expect("missing <genexpr> code");
        assert!(
            !genexpr.instructions.iter().any(|unit| {
                matches!(unit.op, Instruction::LoadConst { .. })
                    && matches!(
                        genexpr.constants.get(usize::from(u8::from(unit.arg))),
                        Some(ConstantData::Boolean { value: true })
                    )
            }),
            "constant-true filter should not load True, got ops={:?}",
            genexpr
                .instructions
                .iter()
                .map(|unit| unit.op)
                .collect::<Vec<_>>()
        );
        assert!(
            !genexpr
                .instructions
                .iter()
                .any(|unit| matches!(unit.op, Instruction::PopJumpIfTrue { .. })),
            "constant-true filter should not leave POP_JUMP_IF_TRUE scaffolding, got ops={:?}",
            genexpr
                .instructions
                .iter()
                .map(|unit| unit.op)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_classdictcell_uses_load_closure_path_and_borrows_after_optimize() {
        let code = compile_exec(
            "\
class C:
    def method(self):
        return 1
",
        );
        let class_code = find_code(&code, "C").expect("missing class code");
        let store_classdictcell = class_code
            .instructions
            .iter()
            .position(|unit| {
                matches!(
                    unit.op,
                    Instruction::StoreName { namei }
                        if class_code.names
                            [namei.get(OpArg::new(u32::from(u8::from(unit.arg)))) as usize]
                            .as_str()
                            == "__classdictcell__"
                )
            })
            .expect("missing STORE_NAME __classdictcell__");

        assert!(
            matches!(
                class_code
                    .instructions
                    .get(store_classdictcell.saturating_sub(1))
                    .map(|unit| unit.op),
                Some(Instruction::LoadFastBorrow { .. })
            ),
            "expected LOAD_FAST_BORROW before __classdictcell__ store, got ops={:?}",
            class_code
                .instructions
                .iter()
                .map(|unit| unit.op)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_conditional_class_body_duplicates_no_location_exit_tail() {
        let code = compile_exec(
            "\
flag = False
class C:
    if flag:
        value = 1
",
        );
        let class_code = find_code(&code, "C").expect("missing class code");
        let ops: Vec<_> = class_code
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();
        let return_count = ops
            .iter()
            .filter(|op| matches!(op, Instruction::ReturnValue))
            .count();
        let static_attrs_count = class_code
            .instructions
            .iter()
            .filter(|unit| {
                matches!(
                    unit.op,
                    Instruction::StoreName { namei }
                        if class_code.names
                            [namei.get(OpArg::new(u32::from(u8::from(unit.arg)))) as usize]
                            .as_str()
                            == "__static_attributes__"
                )
            })
            .count();

        assert_eq!(
            return_count, 2,
            "conditional class body should duplicate CPython no-location return tail, got ops={ops:?}"
        );
        assert_eq!(
            static_attrs_count, 2,
            "conditional class body should duplicate __static_attributes__ tail, got ops={ops:?}"
        );
    }

    #[test]
    fn test_class_lambda_assignment_does_not_create_classdictcell() {
        let code = compile_exec(
            "\
class C:
    data = start = end = lambda *a: None
",
        );
        let class_code = find_code(&code, "C").expect("missing class code");

        assert!(
            !class_code.instructions.iter().any(|unit| {
                matches!(
                    unit.op,
                    Instruction::StoreName { namei }
                        if class_code.names
                            [namei.get(OpArg::new(u32::from(u8::from(unit.arg)))) as usize]
                            .as_str()
                            == "__classdictcell__"
                )
            }),
            "lambda-only class should not create __classdictcell__, got ops={:?}",
            class_code
                .instructions
                .iter()
                .map(|unit| unit.op)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_nested_function_static_attributes_are_collected() {
        let code = compile_exec(
            "\
class C:
    def f(self):
        self.x = 1
        self.y = 2
        self.x = 3

    def g(self, obj):
        self.y = 4
        self.z = 5

        def h(self, a):
            self.u = 6
            self.v = 7

        obj.self = 8
",
        );
        let class_code = find_code(&code, "C").expect("missing class code");

        assert!(
            class_code.constants.iter().any(|constant| matches!(
                constant,
                ConstantData::Tuple { elements }
                    if elements
                        == &[
                            ConstantData::Str { value: "u".into() },
                            ConstantData::Str { value: "v".into() },
                            ConstantData::Str { value: "x".into() },
                            ConstantData::Str { value: "y".into() },
                            ConstantData::Str { value: "z".into() },
                        ]
            )),
            "expected nested function static attributes in class consts"
        );
    }

    #[test]
    fn test_static_attributes_match_cpython_store_rule() {
        let code = compile_exec(
            "\
class C:
    @staticmethod
    def f():
        self.x = 1

    @classmethod
    def g(cls):
        self.y = 2

    def h(obj):
        obj.z = 3
        tarinfo.uid = 4

    def i(self):
        self.a: int
        self.b: int = 1
        self.c += 1
        del self.d
",
        );
        let class_code = find_code(&code, "C").expect("missing class code");

        assert!(
            class_code.constants.iter().any(|constant| matches!(
                constant,
                ConstantData::Tuple { elements }
                    if elements
                        == &[
                            ConstantData::Str { value: "b".into() },
                            ConstantData::Str { value: "x".into() },
                            ConstantData::Str { value: "y".into() },
                        ]
            )),
            "expected only CPython-collected static attributes in class consts"
        );
    }

    #[test]
    fn test_decorated_class_uses_first_decorator_for_firstlineno() {
        let code = compile_exec(
            "\
@dec1
@dec2
class C:
    pass
",
        );
        let class_code = find_code(&code, "C").expect("missing class code");
        let store_firstlineno = class_code
            .instructions
            .iter()
            .position(|unit| {
                matches!(
                    unit.op,
                    Instruction::StoreName { namei }
                        if class_code.names
                            [namei.get(OpArg::new(u32::from(u8::from(unit.arg)))) as usize]
                            .as_str()
                            == "__firstlineno__"
                )
            })
            .expect("missing STORE_NAME __firstlineno__");
        let load_firstlineno = class_code
            .instructions
            .get(store_firstlineno.saturating_sub(1))
            .expect("missing LOAD_CONST for __firstlineno__");

        let expected = ConstantData::Integer {
            value: BigInt::from(1),
        };
        assert!(
            matches!(
                load_firstlineno.op,
                Instruction::LoadSmallInt { .. } | Instruction::LoadConst { .. }
            ),
            "expected LOAD_SMALL_INT/LOAD_CONST before __firstlineno__, got {:?}",
            load_firstlineno.op
        );
        if let Instruction::LoadConst { consti } = load_firstlineno.op {
            let value = &class_code.constants
                [consti.get(OpArg::new(u32::from(u8::from(load_firstlineno.arg))))];
            assert_eq!(value, &expected);
        } else {
            assert_eq!(u32::from(u8::from(load_firstlineno.arg)), 1);
        }
    }

    #[test]
    fn test_future_annotations_class_keeps_conditional_annotations_cell() {
        let code = compile_exec(
            "\
from __future__ import annotations
class C:
    x: int
",
        );
        let class_code = find_code(&code, "C").expect("missing class code");

        assert!(
            class_code
                .cellvars
                .iter()
                .any(|name| name.as_str() == "__conditional_annotations__"),
            "expected __conditional_annotations__ cellvar, got cellvars={:?}",
            class_code.cellvars
        );
    }

    #[test]
    fn test_plain_super_call_keeps_class_freevar() {
        let code = compile_exec(
            "\
class A:
    pass

class B(A):
    def method(self):
        return super()
",
        );
        let method = find_code(&code, "method").expect("missing method code");
        assert!(
            method.freevars.iter().any(|name| name == "__class__"),
            "plain super() must keep __class__ freevar, got freevars={:?}",
            method.freevars
        );
        assert!(
            method
                .instructions
                .iter()
                .any(|unit| matches!(unit.op, Instruction::CopyFreeVars { .. })),
            "plain super() must keep COPY_FREE_VARS prelude, got ops={:?}",
            method
                .instructions
                .iter()
                .map(|unit| unit.op)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_nested_class_super_does_not_create_outer_class_closure() {
        let code = compile_exec(
            "\
class C:
    def outer(self):
        class D:
            def __init__(self):
                super().__init__()
",
        );
        let outer_class = find_code(&code, "C").expect("missing outer class code");
        let nested_class = find_code(&code, "D").expect("missing nested class code");
        let init = find_code(&code, "__init__").expect("missing nested __init__ code");

        assert!(
            !outer_class.cellvars.iter().any(|name| name == "__class__"),
            "nested super() must not force __class__ on outer class, got cellvars={:?}",
            outer_class.cellvars
        );
        assert!(
            nested_class.cellvars.iter().any(|name| name == "__class__"),
            "nested class should own __class__ cell, got cellvars={:?}",
            nested_class.cellvars
        );
        assert!(
            init.freevars.iter().any(|name| name == "__class__"),
            "method using super() should close over nested class, got freevars={:?}",
            init.freevars
        );
    }

    #[test]
    fn test_nested_closure_parameter_class_does_not_create_outer_class_closure() {
        let code = compile_exec(
            "\
class C:
    def m(self):
        def create_closure(__class__):
            return (lambda: __class__).__closure__
",
        );
        let outer_class = find_code(&code, "C").expect("missing class code");
        let create_closure =
            find_code(&code, "create_closure").expect("missing create_closure code");
        let lambda = find_code(&code, "<lambda>").expect("missing lambda code");

        assert!(
            !outer_class.cellvars.iter().any(|name| name == "__class__"),
            "nested __class__ parameter must not force outer class cell, got cellvars={:?}",
            outer_class.cellvars
        );
        assert!(
            create_closure
                .cellvars
                .iter()
                .any(|name| name == "__class__"),
            "create_closure should own __class__ parameter cell, got cellvars={:?}",
            create_closure.cellvars
        );
        assert!(
            lambda.freevars.iter().any(|name| name == "__class__"),
            "lambda should close over create_closure parameter, got freevars={:?}",
            lambda.freevars
        );
    }

    #[test]
    fn test_chained_compare_jump_uses_single_cleanup_copy() {
        let code = compile_exec(
            "\
def f(code):
    if not 1 <= code <= 2147483647:
        raise ValueError('x')
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let copy_count = f
            .instructions
            .iter()
            .filter(|unit| matches!(unit.op, Instruction::Copy { .. }))
            .count();
        let pop_top_count = f
            .instructions
            .iter()
            .filter(|unit| matches!(unit.op, Instruction::PopTop))
            .count();

        assert_eq!(copy_count, 1);
        assert_eq!(pop_top_count, 1);
    }

    #[test]
    fn test_yield_from_cleanup_jumps_to_shared_end_send() {
        let code = compile_exec(
            "\
def outer():
    def inner():
        yield from outer_gen
    return inner
",
        );
        let inner = find_code(&code, "inner").expect("missing inner code");
        let ops: Vec<_> = inner
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        let cleanup_idx = ops
            .iter()
            .position(|op| matches!(op, Instruction::CleanupThrow))
            .expect("missing CLEANUP_THROW");
        assert!(
            matches!(
                ops.get(cleanup_idx + 1),
                Some(Instruction::JumpBackwardNoInterrupt { .. } | Instruction::JumpForward { .. })
            ),
            "expected CLEANUP_THROW to jump to shared END_SEND block, got ops={ops:?}"
        );
        assert!(
            !matches!(ops.get(cleanup_idx + 1), Some(Instruction::EndSend)),
            "CLEANUP_THROW should not inline END_SEND directly, got ops={ops:?}"
        );
    }

    #[test]
    fn test_try_except_falls_through_to_post_handler_code() {
        let code = compile_exec(
            "\
def f():
    try:
        line = 2
        raise KeyError
    except:
        line = 5
    line = 6
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        let first_pop_except = ops
            .iter()
            .position(|op| matches!(op, Instruction::PopExcept))
            .expect("missing POP_EXCEPT");
        assert!(
            !matches!(
                ops.get(first_pop_except + 1),
                Some(Instruction::JumpForward { .. })
            ),
            "expected except body to fall through to post-handler code, got ops={ops:?}"
        );
        assert!(
            matches!(
                ops.get(first_pop_except + 1),
                Some(Instruction::LoadSmallInt { .. } | Instruction::LoadConst { .. })
            ),
            "expected line-after-except code immediately after POP_EXCEPT, got ops={ops:?}"
        );
    }

    #[test]
    fn test_named_except_cleanup_keeps_jump_over_cleanup_and_next_try() {
        let code = compile_exec(
            r#"
def f(self):
    try:
        assert 0, 'msg'
    except AssertionError as e:
        self.assertEqual(e.args[0], 'msg')
    else:
        self.fail("AssertionError not raised by assert 0")

    try:
        assert False
    except AssertionError as e:
        self.assertEqual(len(e.args), 0)
    else:
        self.fail("AssertionError not raised by 'assert False'")
"#,
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        let first_pop_except = ops
            .iter()
            .position(|op| matches!(op, Instruction::PopExcept))
            .expect("missing POP_EXCEPT");
        let window = &ops[first_pop_except..(first_pop_except + 6).min(ops.len())];
        assert!(
            matches!(
                window,
                [
                    Instruction::PopExcept,
                    Instruction::LoadConst { .. },
                    Instruction::StoreName { .. } | Instruction::StoreFast { .. },
                    Instruction::DeleteName { .. } | Instruction::DeleteFast { .. },
                    Instruction::JumpForward { .. },
                    ..
                ]
            ),
            "expected named except cleanup to jump over cleanup reraise block, got ops={window:?}"
        );
    }

    #[test]
    fn test_bare_except_deopts_post_handler_load_fast_borrow() {
        let code = compile_exec(
            "\
def f(self):
    try:
        1 / 0
    except:
        pass
    with self.assertRaises(SyntaxError):
        pass
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        let attr_idx = ops
            .iter()
            .position(|op| matches!(op, Instruction::LoadAttr { .. }))
            .expect("missing LOAD_ATTR for assertRaises");
        assert!(
            matches!(ops.get(attr_idx - 1), Some(Instruction::LoadFast { .. })),
            "bare except tail should deopt self to LOAD_FAST, got ops={ops:?}"
        );
    }

    #[test]
    fn test_typed_except_keeps_post_handler_load_fast_borrow() {
        let code = compile_exec(
            "\
def f(self):
    try:
        1 / 0
    except ZeroDivisionError:
        pass
    with self.assertRaises(SyntaxError):
        pass
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        let attr_idx = ops
            .iter()
            .position(|op| matches!(op, Instruction::LoadAttr { .. }))
            .expect("missing LOAD_ATTR for assertRaises");
        assert!(
            matches!(
                ops.get(attr_idx - 1),
                Some(Instruction::LoadFastBorrow { .. })
            ),
            "typed except tail should keep LOAD_FAST_BORROW, got ops={ops:?}"
        );
    }

    #[test]
    fn test_reraising_typed_except_deopts_post_handler_loads() {
        let code = compile_exec(
            "\
def f(x, os, self, pid, exitcode):
    try:
        y = 1
    except RuntimeError:
        raise
    if x:
        os._exit(exitcode)
    self.wait_impl(pid, exitcode=exitcode)
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        let guard_idx = ops
            .iter()
            .position(|op| matches!(op, Instruction::ToBool))
            .and_then(|idx| idx.checked_sub(1))
            .expect("missing post-handler bool guard");
        assert!(
            matches!(ops.get(guard_idx), Some(Instruction::LoadFast { .. })),
            "reraising typed except tail should deopt guard load, got ops={ops:?}"
        );

        let wait_idx = ops
            .iter()
            .position(|op| matches!(op, Instruction::CallKw { .. }))
            .expect("missing wait_impl CALL_KW");
        let call_args = &ops[wait_idx.saturating_sub(3)..wait_idx];
        assert!(
            call_args.iter().any(|op| matches!(
                op,
                Instruction::LoadFastLoadFast { .. } | Instruction::LoadFast { .. }
            )),
            "reraising typed except tail should keep strong fast loads for call args, got ops={ops:?}"
        );
        assert!(
            !call_args.iter().any(|op| matches!(
                op,
                Instruction::LoadFastBorrowLoadFastBorrow { .. }
                    | Instruction::LoadFastBorrow { .. }
            )),
            "reraising typed except tail should not borrow call args, got ops={ops:?}"
        );
    }

    #[test]
    fn test_reraising_except_loop_backedge_keeps_loop_header_borrow() {
        let code = compile_exec(
            "\
def f(self, tag, expect_bye):
    while 1:
        result = self.tagged_commands[tag]
        if result is not None:
            del self.tagged_commands[tag]
            return result
        if expect_bye:
            typ = 'BYE'
            bye = self.untagged_responses.pop(typ, None)
            if bye is not None:
                return (typ, bye)
        self._check_bye()
        try:
            self._get_response()
        except self.abort as val:
            if __debug__:
                if self.debug >= 1:
                    self.print_log()
            raise
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let instructions: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();
        let handler_start = instructions
            .iter()
            .position(|unit| matches!(unit.op, Instruction::PushExcInfo))
            .expect("missing handler entry");
        let warm_ops: Vec<_> = instructions[..handler_start]
            .iter()
            .map(|unit| unit.op)
            .collect();

        assert!(
            warm_ops.iter().any(|op| matches!(
                op,
                Instruction::LoadFastBorrow { .. }
                    | Instruction::LoadFastBorrowLoadFastBorrow { .. }
            )),
            "expected loop body before reraising handler to keep borrowed loads, got ops={warm_ops:?}"
        );
        assert!(
            warm_ops
                .iter()
                .all(|op| !matches!(op, Instruction::LoadFast { .. })),
            "loop backedge into reraising handler should not deopt warm loop loads, got ops={warm_ops:?}"
        );
    }

    #[test]
    fn test_protected_store_break_handler_deopts_bool_guard_tail() {
        let code = compile_exec(
            "\
def f(self, size):
    parts = []
    while size > 0:
        try:
            buf = self.sock.recv(DEFAULT_BUFFER_SIZE)
        except ConnectionError:
            break
        if not buf:
            break
        self._readbuf.append(buf)
        size -= len(buf)
    return b''.join(parts)
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let instructions: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();
        let guard_bool = instructions
            .iter()
            .position(|unit| matches!(unit.op, Instruction::ToBool))
            .expect("missing bool guard");
        let store_buf = instructions[..guard_bool]
            .iter()
            .rposition(|unit| matches!(unit.op, Instruction::StoreFast { .. }))
            .expect("missing protected STORE_FAST before bool guard");
        let guard_load = instructions[store_buf + 1].op;
        let append_call = instructions[store_buf + 1..]
            .iter()
            .position(|unit| matches!(unit.op, Instruction::Call { .. }))
            .map(|idx| idx + store_buf + 1)
            .expect("missing append call");
        let append_arg = instructions[append_call - 1].op;

        assert!(
            matches!(guard_load, Instruction::LoadFast { .. }),
            "CPython uses strong LOAD_FAST for protected-store break guard, got ops={:?}",
            instructions.iter().map(|unit| unit.op).collect::<Vec<_>>()
        );
        assert!(
            matches!(append_arg, Instruction::LoadFast { .. }),
            "CPython uses strong LOAD_FAST for protected-store append arg, got ops={:?}",
            instructions.iter().map(|unit| unit.op).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_assertion_success_join_keeps_following_debug_tail_borrowed() {
        let code = compile_exec(
            "\
def f(self, typ, dat):
    if self._idle_capture:
        if self._idle_responses:
            response = self._idle_responses[-1]
            assert response[0] == typ
            response[1].append(dat)
        else:
            self._idle_responses.append((typ, [dat]))
        if self.debug >= 5:
            self._mesg(f'idle: queue untagged {typ} {dat!r}')
        return
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let instructions: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();
        let debug_attr = instructions
            .iter()
            .position(|unit| match unit.op {
                Instruction::LoadAttr { namei } => {
                    let load_attr = namei.get(OpArg::new(u32::from(u8::from(unit.arg))));
                    f.names[usize::try_from(load_attr.name_idx()).unwrap()].as_str() == "debug"
                }
                _ => false,
            })
            .expect("missing debug LOAD_ATTR");
        let mesg_attr = instructions
            .iter()
            .position(|unit| match unit.op {
                Instruction::LoadAttr { namei } => {
                    let load_attr = namei.get(OpArg::new(u32::from(u8::from(unit.arg))));
                    f.names[usize::try_from(load_attr.name_idx()).unwrap()].as_str() == "_mesg"
                }
                _ => false,
            })
            .expect("missing _mesg LOAD_ATTR");

        assert!(
            matches!(
                instructions[debug_attr - 1].op,
                Instruction::LoadFastBorrow { .. }
            ),
            "CPython keeps LOAD_FAST_BORROW after assertion success join, got ops={:?}",
            instructions.iter().map(|unit| unit.op).collect::<Vec<_>>()
        );
        assert!(
            matches!(
                instructions[mesg_attr - 1].op,
                Instruction::LoadFastBorrow { .. }
            ),
            "CPython keeps LOAD_FAST_BORROW in assertion-success debug body, got ops={:?}",
            instructions.iter().map(|unit| unit.op).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_multi_protected_method_call_terminal_handler_deopts_block() {
        let code = compile_exec(
            "\
def f(self, literal):
    try:
        self.send(literal)
        self.send(CRLF)
    except OSError as val:
        raise self.abort('socket error: %s' % val)
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let instructions: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();
        let first_send = instructions
            .iter()
            .position(|unit| match unit.op {
                Instruction::LoadAttr { namei } => {
                    let load_attr = namei.get(OpArg::new(u32::from(u8::from(unit.arg))));
                    f.names[usize::try_from(load_attr.name_idx()).unwrap()].as_str() == "send"
                }
                _ => false,
            })
            .expect("missing send LOAD_ATTR");
        let first_literal = instructions[first_send + 1].op;
        let second_send = instructions[first_send + 1..]
            .iter()
            .position(|unit| match unit.op {
                Instruction::LoadAttr { namei } => {
                    let load_attr = namei.get(OpArg::new(u32::from(u8::from(unit.arg))));
                    f.names[usize::try_from(load_attr.name_idx()).unwrap()].as_str() == "send"
                }
                _ => false,
            })
            .map(|idx| idx + first_send + 1)
            .expect("missing second send LOAD_ATTR");

        assert!(
            matches!(
                instructions[first_send - 1].op,
                Instruction::LoadFast { .. }
            ),
            "CPython uses strong LOAD_FAST for first protected send receiver, got ops={:?}",
            instructions.iter().map(|unit| unit.op).collect::<Vec<_>>()
        );
        assert!(
            matches!(first_literal, Instruction::LoadFast { .. }),
            "CPython uses strong LOAD_FAST for first protected send arg, got ops={:?}",
            instructions.iter().map(|unit| unit.op).collect::<Vec<_>>()
        );
        assert!(
            matches!(
                instructions[second_send - 1].op,
                Instruction::LoadFast { .. }
            ),
            "CPython uses strong LOAD_FAST for second protected send receiver, got ops={:?}",
            instructions.iter().map(|unit| unit.op).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_dunder_debug_constant_false_if_deopts_tail_borrow() {
        let code = compile_exec(
            "\
def f(self):
    if not __debug__:
        self.skipTest('need asserts, run without -O')
    self.do_disassembly_test()
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let instructions: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();
        let attr_idx = instructions
            .iter()
            .position(|unit| match unit.op {
                Instruction::LoadAttr { namei } => {
                    let load_attr = namei.get(OpArg::new(u32::from(u8::from(unit.arg))));
                    f.names[usize::try_from(load_attr.name_idx()).unwrap()].as_str()
                        == "do_disassembly_test"
                }
                _ => false,
            })
            .expect("missing LOAD_ATTR for do_disassembly_test");
        let ops: Vec<_> = instructions.iter().map(|unit| unit.op).collect();
        assert!(
            matches!(ops.get(attr_idx - 1), Some(Instruction::LoadFast { .. })),
            "constant-false __debug__ tail should deopt self to LOAD_FAST, got ops={ops:?}"
        );
    }

    #[test]
    fn test_constant_slice_folds_constant_bounds() {
        let code = compile_exec(
            "\
def f(obj):
    return obj['a':123456789012345678901234567890]
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();
        let folded_slice = f
            .constants
            .iter()
            .find_map(|constant| match constant {
                ConstantData::Slice { elements } => Some(elements),
                _ => None,
            })
            .expect("missing folded slice constant");
        assert!(
            matches!(
                folded_slice.as_ref(),
                [
                    ConstantData::Str { .. },
                    ConstantData::Integer { .. },
                    ConstantData::None,
                ]
            ),
            "expected folded slice('a', 123456789012345678901234567890, None), got {folded_slice:?}"
        );
        assert!(
            matches!(
                ops.as_slice(),
                [
                    Instruction::Resume { .. },
                    Instruction::LoadFastBorrow { .. },
                    Instruction::LoadConst { .. },
                    Instruction::BinaryOp { .. },
                    Instruction::ReturnValue,
                ]
            ),
            "expected CPython-style LOAD_CONST(slice(...)) path for constant bounds, got ops={ops:?}"
        );
    }

    #[test]
    fn test_negative_step_slice_uses_build_slice() {
        let code = compile_exec(
            "\
def f(obj):
    return obj[::-1]
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            matches!(
                ops.as_slice(),
                [
                    Instruction::Resume { .. },
                    Instruction::LoadFastBorrow { .. },
                    Instruction::LoadConst { .. },
                    Instruction::LoadConst { .. },
                    Instruction::LoadConst { .. },
                    Instruction::BuildSlice { .. },
                    Instruction::BinaryOp { .. },
                    Instruction::ReturnValue,
                ]
            ),
            "expected CPython-style BUILD_SLICE path for non-literal negative step, got ops={ops:?}"
        );
    }

    #[test]
    fn test_bool_int_binop_constants_fold() {
        let code = compile_exec(
            "\
def f():
    return False + 2, True + 2, False + False, True / 1, True & False

def g():
    return False + 2
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            !ops.iter()
                .any(|op| matches!(op, Instruction::BinaryOp { .. })),
            "expected CPython-style folded bool/int binops, got ops={ops:?}"
        );
        assert!(
            matches!(
                ops.as_slice(),
                [
                    Instruction::Resume { .. },
                    Instruction::LoadConst { .. },
                    Instruction::ReturnValue
                ]
            ),
            "expected folded constants for bool/int binops, got ops={ops:?}"
        );

        let g = find_code(&code, "g").expect("missing function code");
        let g_ops: Vec<_> = g
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();
        assert!(
            !g_ops
                .iter()
                .any(|op| matches!(op, Instruction::BinaryOp { .. })),
            "expected top-level bool/int binop to fold, got ops={g_ops:?}"
        );
    }

    #[test]
    fn test_double_not_expression_folds_to_bool_conversion() {
        let code = compile_exec(
            "\
def f(x):
    return not not x
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            matches!(
                ops.as_slice(),
                [
                    Instruction::Resume { .. },
                    Instruction::LoadFastBorrow { .. },
                    Instruction::ToBool,
                    Instruction::ReturnValue,
                ]
            ),
            "expected CPython-style double-not bool conversion, got ops={ops:?}"
        );
    }

    #[test]
    fn test_tuple_bound_slice_uses_two_part_slice_path() {
        let code = compile_exec(
            "\
def f(obj):
    return obj[(1, 2):]
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            matches!(
                ops.as_slice(),
                [
                    Instruction::Resume { .. },
                    Instruction::LoadFastBorrow { .. },
                    Instruction::LoadConst { .. },
                    Instruction::LoadConst { .. },
                    Instruction::BinarySlice,
                    Instruction::ReturnValue,
                ]
            ),
            "expected CPython-style BINARY_SLICE path for tuple lower bound, got ops={ops:?}"
        );
    }

    #[test]
    fn test_exception_cleanup_jump_to_return_is_inlined() {
        let code = compile_exec(
            "\
def f(names, cls):
    try:
        cls.attr = names
    except:
        pass
    return names
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let return_count = f
            .instructions
            .iter()
            .filter(|unit| matches!(unit.op, Instruction::ReturnValue))
            .count();

        assert_eq!(
            return_count, 2,
            "expected CPython-style distinct return sites for normal and except paths"
        );
    }

    #[test]
    fn test_except_break_preserves_plain_jump_when_inlining_no_lineno_tail() {
        let code = compile_exec(
            "\
def f(compiler_so, cc_args):
    strip_sysroot = True
    if '-arch' in cc_args:
        while True:
            try:
                index = compiler_so.index('-arch')
                del compiler_so[index:index + 2]
            except ValueError:
                break
    if strip_sysroot:
        while True:
            indices = [i for i, x in enumerate(compiler_so) if x.startswith('-isysroot')]
            if not indices:
                break
            index = indices[0]
            del compiler_so[index:index + 1]
    return compiler_so
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(2).any(|window| {
                matches!(
                    window,
                    [Instruction::PopExcept, Instruction::JumpBackward { .. }]
                )
            }) && !ops.windows(2).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::PopExcept,
                        Instruction::JumpBackwardNoInterrupt { .. }
                    ]
                )
            }),
            "except-break cleanup should preserve CPython's plain JUMP when a no-lineno tail is inlined, got ops={ops:?}"
        );
    }

    #[test]
    fn test_nested_with_bare_except_keeps_handler_cleanup_before_following_code() {
        let code = compile_exec(
            "\
def f(cm, self):
    try:
        with cm:
            raise Exception
    except:
        pass
    self.g()
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        let outer_handler = ops
            .iter()
            .enumerate()
            .filter_map(|(idx, op)| matches!(op, Instruction::PushExcInfo).then_some(idx))
            .next_back()
            .expect("missing outer handler");
        assert!(
            ops[outer_handler..].windows(6).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::PopExcept,
                        Instruction::JumpForward { .. },
                        Instruction::Copy { .. },
                        Instruction::PopExcept,
                        Instruction::Reraise { .. },
                        Instruction::LoadFast { .. },
                    ]
                )
            }),
            "expected CPython-style handler cleanup before following code, got ops={ops:?}"
        );
    }

    #[test]
    fn test_try_else_for_cleanup_drops_redundant_jump_nop() {
        let code = compile_exec(
            "\
def f(self, xs, ys, cm1, cm2):
    for x in xs:
        with self.subTest(x=x):
            try:
                with cm1:
                    self.a()
            except Exception:
                if x:
                    pass
                else:
                    raise
            else:
                for y in ys:
                    with self.subTest(y=y):
                        with cm2:
                            self.b()
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(7).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::EndFor,
                        Instruction::PopIter,
                        Instruction::LoadConst { .. },
                        Instruction::LoadConst { .. },
                        Instruction::LoadConst { .. },
                        Instruction::Call { .. },
                        Instruction::PopTop,
                    ]
                )
            }),
            "expected inner for cleanup to fall directly into surrounding with cleanup, got ops={ops:?}",
        );
        assert!(
            !ops.windows(8).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::EndFor,
                        Instruction::PopIter,
                        Instruction::Nop,
                        Instruction::LoadConst { .. },
                        Instruction::LoadConst { .. },
                        Instruction::LoadConst { .. },
                        Instruction::Call { .. },
                        Instruction::PopTop,
                    ]
                )
            }),
            "expected CPython-style removal of the redundant jump NOP after for cleanup, got ops={ops:?}",
        );
    }

    #[test]
    fn test_non_none_final_return_is_not_duplicated() {
        let code = compile_exec(
            "\
def f(p, s):
    if p == '':
        if s == '':
            return 0
    return -1
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let minus_one_loads = f
            .instructions
            .iter()
            .filter(|unit| {
                matches!(
                    unit.op,
                    Instruction::LoadConst { consti }
                        if matches!(
                            f.constants.get(
                                consti
                                    .get(OpArg::new(u32::from(u8::from(unit.arg))))
                                    .as_usize()
                            ),
                            Some(ConstantData::Integer { value }) if value == &BigInt::from(-1)
                        )
                )
            })
            .count();

        assert_eq!(
            minus_one_loads,
            1,
            "expected a single final return -1 epilogue, got ops={:?}",
            f.instructions
                .iter()
                .map(|unit| unit.op)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_for_return_unary_constant_preserves_value_over_iterator_cleanup() {
        let code = compile_exec(
            "\
def f(xs):
    for x in xs:
        return -1
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let units: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();

        assert!(
            units.windows(4).any(|window| {
                matches!(
                    window[0].op,
                    Instruction::LoadConst { .. } | Instruction::LoadSmallInt { .. }
                ) && matches!(
                    window[1].op,
                    Instruction::Swap { i }
                        if i.get(OpArg::new(u32::from(u8::from(window[1].arg)))) == 2
                ) && matches!(window[2].op, Instruction::PopTop)
                    && matches!(window[3].op, Instruction::ReturnValue)
            }),
            "expected CPython-style LOAD_CONST/SWAP/POP_TOP/RETURN_VALUE cleanup, got units={units:?}"
        );
    }

    #[test]
    fn test_try_else_if_return_keeps_conditional_target_nop() {
        let code = compile_exec(
            "\
def f(cond):
    try:
        x = cond
    except E:
        pass
    else:
        if x:
            return 1
    return 2
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        let has_cpython_nop_target = ops.windows(5).any(|window| {
            matches!(
                window,
                [
                    Instruction::LoadSmallInt { .. } | Instruction::LoadConst { .. },
                    Instruction::ReturnValue,
                    Instruction::Nop,
                    Instruction::LoadSmallInt { .. } | Instruction::LoadConst { .. },
                    Instruction::ReturnValue,
                ]
            )
        });
        let has_direct_fallthrough = ops.windows(4).any(|window| {
            matches!(
                window,
                [
                    Instruction::LoadSmallInt { .. } | Instruction::LoadConst { .. },
                    Instruction::ReturnValue,
                    Instruction::LoadSmallInt { .. } | Instruction::LoadConst { .. },
                    Instruction::ReturnValue,
                ]
            )
        });
        assert!(
            has_cpython_nop_target || has_direct_fallthrough,
            "expected adjacent try-else return and final return targets, got ops={ops:?}"
        );
    }

    #[test]
    fn test_named_except_conditional_branch_duplicates_cleanup_return() {
        let code = compile_exec(
            "\
def f(self):
    try:
        raise TypeError('x')
    except TypeError as e:
        if '+' not in str(e):
            self.fail('join() ate exception message')
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        let cleanup_return_count = ops
            .windows(6)
            .filter(|window| {
                matches!(
                    window,
                    [
                        Instruction::PopExcept,
                        Instruction::LoadConst { .. },
                        Instruction::StoreFast { .. } | Instruction::StoreName { .. },
                        Instruction::DeleteFast { .. } | Instruction::DeleteName { .. },
                        Instruction::LoadConst { .. },
                        Instruction::ReturnValue,
                    ]
                )
            })
            .count();

        assert_eq!(
            cleanup_return_count, 2,
            "expected duplicated named-except cleanup return blocks, got ops={ops:?}"
        );
    }

    #[test]
    fn test_listcomp_cleanup_tail_keeps_split_store_fast_pair() {
        let code = compile_exec(
            "\
def f(escaped_string, quote_types):
    possible_quotes = [q for q in quote_types if q not in escaped_string]
    return possible_quotes
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        let pop_iter_idx = ops
            .iter()
            .position(|op| matches!(op, Instruction::PopIter))
            .expect("missing POP_ITER");
        let tail = &ops[pop_iter_idx + 1..];

        assert!(
            matches!(
                tail,
                [
                    Instruction::StoreFast { .. },
                    Instruction::StoreFast { .. },
                    Instruction::LoadFastBorrow { .. },
                    Instruction::ReturnValue,
                    ..
                ]
            ),
            "expected split STORE_FAST pair after listcomp cleanup, got ops={ops:?}"
        );
    }

    #[test]
    fn test_dictcomp_cleanup_tail_keeps_split_store_fast_pair() {
        let code = compile_exec(
            "\
def f(obj, g):
    return {g(k): g(v) for k, v in obj.items()}
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        let pop_iter_idx = ops
            .iter()
            .position(|op| matches!(op, Instruction::PopIter))
            .expect("missing POP_ITER");
        let tail = &ops[pop_iter_idx + 1..];

        assert!(
            matches!(
                tail,
                [
                    Instruction::Swap { .. },
                    Instruction::StoreFast { .. },
                    Instruction::StoreFast { .. },
                    Instruction::ReturnValue,
                    ..
                ]
            ),
            "expected split STORE_FAST pair after dictcomp cleanup, got ops={ops:?}"
        );
    }

    #[test]
    fn test_static_swap_triple_assign_keeps_store_fast_store_fast() {
        let code = compile_exec(
            "\
def f(x, y, z):
    a, b, a = x, y, z
    return a
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(3).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::Swap { .. },
                        Instruction::StoreFastStoreFast { .. },
                        Instruction::StoreFast { .. }
                    ]
                )
            }),
            "expected CPython-style SWAP/STORE_FAST_STORE_FAST/STORE_FAST sequence, got ops={ops:?}"
        );
    }

    #[test]
    fn test_static_swap_duplicate_pair_eliminates_swap() {
        let code = compile_exec(
            "\
def f(x, y):
    a, a = x, y
    return a
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            !ops.iter().any(|op| matches!(op, Instruction::Swap { .. })),
            "duplicate pair assignment should statically eliminate SWAP, got ops={ops:?}"
        );
        assert!(
            ops.windows(2).any(|window| {
                matches!(window, [Instruction::StoreFast { .. }, Instruction::PopTop])
            }),
            "expected CPython-style STORE_FAST/POP_TOP duplicate assignment, got ops={ops:?}"
        );
    }

    #[test]
    fn test_static_swap_duplicate_prefix_eliminates_swap() {
        let code = compile_exec(
            "\
def f(x, y, z):
    a, a, b = x, y, z
    return a
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            !ops.iter().any(|op| matches!(op, Instruction::Swap { .. })),
            "duplicate-prefix assignment should statically eliminate SWAP, got ops={ops:?}"
        );
        assert!(
            ops.windows(2).any(|window| {
                matches!(
                    window,
                    [Instruction::StoreFastStoreFast { .. }, Instruction::PopTop]
                )
            }),
            "expected CPython-style STORE_FAST_STORE_FAST/POP_TOP duplicate prefix, got ops={ops:?}"
        );
    }

    #[test]
    fn test_constant_if_expression_stmt_in_loop_removes_empty_body() {
        let code = compile_exec(
            "\
def f(x):
    while x:
        0 if 1 else 0
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            !ops.iter()
                .any(|op| matches!(op, Instruction::LoadSmallInt { .. })),
            "expected constant if-expression statement to compile away inside loop, got ops={ops:?}"
        );
    }

    #[test]
    fn test_if_expression_in_jump_context_skips_constant_true_arm_load() {
        let code = compile_exec(
            "\
def f():
    a if (1 if b else c) else d
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            !ops.iter()
                .any(|op| matches!(op, Instruction::LoadSmallInt { .. })),
            "expected jump-context if-expression to avoid materializing constant truthy arm, got ops={ops:?}"
        );
    }

    #[test]
    fn test_with_suppress_tail_duplicates_final_return_none() {
        let code = compile_exec(
            "\
def f(cm, cond):
    if cond:
        with cm():
            pass
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        let return_count = ops
            .iter()
            .filter(|op| matches!(op, Instruction::ReturnValue))
            .count();

        assert_eq!(
            return_count, 3,
            "expected duplicated return-none epilogues, got ops={ops:?}"
        );
        assert!(
            !ops.iter()
                .any(|op| matches!(op, Instruction::JumpBackwardNoInterrupt { .. })),
            "with suppress tail should not jump back to shared return block, got ops={ops:?}"
        );
    }

    #[test]
    fn test_with_conditional_bare_return_keeps_return_line_nop_before_exit_cleanup() {
        let code = compile_exec(
            "\
def f(cm, registry, altkey):
    with cm:
        if registry.get(altkey):
            return
        registry[altkey] = 1
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(8).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::Nop,
                        Instruction::LoadConst { .. },
                        Instruction::LoadConst { .. },
                        Instruction::LoadConst { .. },
                        Instruction::Call { .. },
                        Instruction::PopTop,
                        Instruction::LoadConst { .. },
                        Instruction::ReturnValue,
                    ]
                )
            }),
            "expected CPython-style return-line NOP before with-exit cleanup return, got ops={ops:?}"
        );
    }

    #[test]
    fn test_try_finally_conditional_return_duplicates_finally_exit_return() {
        let code = compile_exec(
            "\
def f(flag, data, callback):
    try:
        if flag:
            return
        value = 1
    finally:
        if data:
            callback(data)
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        let return_count = ops
            .iter()
            .filter(|op| matches!(op, Instruction::ReturnValue))
            .count();
        assert_eq!(
            return_count, 4,
            "try-finally return unwind should keep CPython-style distinct true/false finalbody exits, got ops={ops:?}"
        );
    }

    #[test]
    fn test_named_except_conditional_cleanup_is_inlined_per_branch() {
        let code = compile_exec(
            "\
def f(self, logger):
    try:
        work()
    except A as exc:
        if not self.closing:
            self.fatal(exc, 'msg')
        elif self.loop.get_debug():
            logger.debug('closing', exc_info=True)
    finally:
        if self.length > -1:
            self.recv()
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        let cleanup_after_branch_count = ops
            .windows(6)
            .filter(|window| {
                matches!(
                    window,
                    [
                        Instruction::PopTop,
                        Instruction::PopExcept,
                        Instruction::LoadConst { .. },
                        Instruction::StoreFast { .. },
                        Instruction::DeleteFast { .. },
                        Instruction::JumpBackwardNoInterrupt { .. },
                    ]
                )
            })
            .count();
        assert_eq!(
            cleanup_after_branch_count, 2,
            "named except branch exits should inline cleanup like CPython, got ops={ops:?}"
        );
    }

    #[test]
    fn test_try_finally_exception_path_duplicates_conditional_reraise() {
        let code = compile_exec(
            "\
def f(flag, callback):
    try:
        work()
    finally:
        if flag:
            callback()
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        let reraise_count = ops
            .iter()
            .filter(|op| matches!(op, Instruction::Reraise { .. }))
            .count();
        assert_eq!(
            reraise_count, 3,
            "try-finally exception finalbody should duplicate CPython no-location RERAISE exits, got ops={ops:?}"
        );
    }

    #[test]
    fn test_genexpr_compare_header_uses_store_fast_load_fast_like_cpython() {
        let code = compile_exec(
            "\
def f(it):
    return (offset == (4, 10) for offset in it)
",
        );
        let genexpr = find_code(&code, "<genexpr>").expect("missing <genexpr> code");
        let ops: Vec<_> = genexpr
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(3).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::StoreFastLoadFast { .. },
                        Instruction::LoadConst { .. },
                        Instruction::CompareOp { .. },
                    ]
                )
            }),
            "expected CPython-style STORE_FAST_LOAD_FAST compare header, got ops={ops:?}"
        );
    }

    #[test]
    fn test_fstring_adjacent_literals_are_merged() {
        let code = compile_exec(
            "\
def f(cls, proto):
    raise TypeError(
        f\"cannot pickle {cls.__name__!r} object: \"
        f\"a class that defines __slots__ without \"
        f\"defining __getstate__ cannot be pickled \"
        f\"with protocol {proto}\"
    )
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let string_consts = f
            .instructions
            .iter()
            .filter_map(|unit| match unit.op {
                Instruction::LoadConst { consti } => {
                    Some(&f.constants[consti.get(OpArg::new(u32::from(u8::from(unit.arg))))])
                }
                _ => None,
            })
            .filter_map(|constant| match constant {
                ConstantData::Str { value } => Some(value.to_string()),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert!(
            string_consts.iter().any(|value| {
                value
                    == " object: a class that defines __slots__ without defining __getstate__ cannot be pickled with protocol "
            }),
            "expected merged trailing f-string literal, got {string_consts:?}"
        );
        assert!(
            !string_consts.iter().any(|value| value == " object: "),
            "did not expect split trailing literal, got {string_consts:?}"
        );
    }

    #[test]
    fn test_literal_only_fstring_statement_is_optimized_away() {
        let code = compile_exec(
            "\
def f():
    f'''Not a docstring'''
",
        );
        let f = find_code(&code, "f").expect("missing function code");

        assert!(
            !f.instructions
                .iter()
                .any(|unit| matches!(unit.op, Instruction::PopTop)),
            "literal-only f-string statement should be removed"
        );
        assert!(
            !f.constants.iter().any(|constant| matches!(
                constant,
                ConstantData::Str { value } if value.to_string() == "Not a docstring"
            )),
            "literal-only f-string should not survive in constants"
        );
    }

    #[test]
    fn test_empty_fstring_literals_are_elided_around_interpolation() {
        let code = compile_exec(
            "\
def f(x):
    if '' f'{x}':
        return 1
    return 2
",
        );
        let f = find_code(&code, "f").expect("missing function code");

        let empty_string_loads = f
            .instructions
            .iter()
            .filter_map(|unit| match unit.op {
                Instruction::LoadConst { consti } => {
                    Some(&f.constants[consti.get(OpArg::new(u32::from(u8::from(unit.arg))))])
                }
                _ => None,
            })
            .filter(|constant| {
                matches!(
                    constant,
                    ConstantData::Str { value } if value.is_empty()
                )
            })
            .count();
        let build_string_count = f
            .instructions
            .iter()
            .filter(|unit| matches!(unit.op, Instruction::BuildString { .. }))
            .count();

        assert_eq!(empty_string_loads, 0);
        assert_eq!(build_string_count, 0);
    }

    #[test]
    fn test_large_fstring_uses_join_list_like_cpython() {
        let mut source = String::from("def f(x):\n    return f\"");
        for _ in 0..=STACK_USE_GUIDELINE {
            source.push_str("{x}");
        }
        source.push_str("\"\n");

        let code = compile_exec(&source);
        let f = find_code(&code, "f").expect("missing function code");
        let build_string_count = f
            .instructions
            .iter()
            .filter(|unit| matches!(unit.op, Instruction::BuildString { .. }))
            .count();
        let list_append_count = f
            .instructions
            .iter()
            .filter(|unit| matches!(unit.op, Instruction::ListAppend { .. }))
            .count();
        let join_attr_count = f
            .instructions
            .iter()
            .filter(|unit| match unit.op {
                Instruction::LoadAttr { namei } => {
                    let load_attr = namei.get(OpArg::new(u32::from(u8::from(unit.arg))));
                    load_attr.is_method()
                        && f.names[usize::try_from(load_attr.name_idx()).unwrap()].as_str()
                            == "join"
                }
                _ => false,
            })
            .count();

        assert_eq!(build_string_count, 0);
        assert_eq!(
            list_append_count,
            usize::try_from(STACK_USE_GUIDELINE + 1).unwrap()
        );
        assert_eq!(join_attr_count, 1);
    }

    #[test]
    fn test_large_power_is_not_constant_folded() {
        let code = compile_exec("x = 2**100\n");

        assert!(code.instructions.iter().any(|unit| match unit.op {
            Instruction::BinaryOp { op } => {
                op.get(OpArg::new(u32::from(u8::from(unit.arg)))) == oparg::BinaryOperator::Power
            }
            _ => false,
        }));
    }

    #[test]
    fn test_string_and_bytes_binops_constant_fold_like_cpython() {
        let code = compile_exec(
            "\
x = b'\\\\' + b'u1881'\n\
y = 103 * 'a' + 'x'\n",
        );

        assert!(
            !code
                .instructions
                .iter()
                .any(|unit| matches!(unit.op, Instruction::BinaryOp { .. })),
            "unexpected runtime BINARY_OP in folded string/bytes constants: {:?}",
            code.instructions
        );
        assert!(code.constants.iter().any(|constant| matches!(
            constant,
            ConstantData::Bytes { value } if value == b"\\u1881"
        )));
        let expected = format!("{}x", "a".repeat(103));
        assert!(code.constants.iter().any(|constant| matches!(
            constant,
            ConstantData::Str { value }
                if value.to_string() == expected
        )));
    }

    #[test]
    fn test_float_floor_division_constant_folds_like_cpython() {
        let code = compile_exec(
            "\
x = 1.0 // 0.1\n\
y = 1.0 % 0.1\n\
z = 1e300 * 1e300 * 0\n",
        );

        assert!(
            !code
                .instructions
                .iter()
                .any(|unit| matches!(unit.op, Instruction::BinaryOp { .. })),
            "float constant floor-div/mod should fold away, got instructions={:?}",
            code.instructions
        );
        assert!(code.constants.iter().any(|constant| matches!(
            constant,
            ConstantData::Float { value } if value.to_bits() == 9.0f64.to_bits()
        )));
        assert!(code.constants.iter().any(|constant| matches!(
            constant,
            ConstantData::Float { value }
                if value.to_bits() == 0.09999999999999995f64.to_bits()
        )));
        assert!(code.constants.iter().any(|constant| matches!(
            constant,
            ConstantData::Float { value } if value.is_nan()
        )));
    }

    #[test]
    fn test_float_power_overflow_constant_does_not_fold() {
        let code = compile_exec("x = 1e300 ** 2\n");

        assert!(
            code.instructions.iter().any(|unit| matches!(
                unit.op,
                Instruction::BinaryOp { op }
                    if op.get(OpArg::new(u32::from(u8::from(unit.arg))))
                        == oparg::BinaryOperator::Power
            )),
            "overflowing float power should stay runtime like CPython, got instructions={:?}",
            code.instructions
        );
    }

    #[test]
    fn test_large_string_and_bytes_binops_constant_fold_like_cpython() {
        let code = compile_exec(
            r#"
encoded = b'\xff\xfe\x00\x00' + b'\x00\x00\x01\x00' * 1024
text = '\U00010000' * 1024
"#,
        );

        assert!(
            !code
                .instructions
                .iter()
                .any(|unit| matches!(unit.op, Instruction::BinaryOp { .. })),
            "large safe string/bytes constants should fold away, got instructions={:?}",
            code.instructions
        );
        assert!(code.constants.iter().any(|constant| matches!(
            constant,
            ConstantData::Bytes { value } if value.len() == 4100
        )));
        assert!(code.constants.iter().any(|constant| matches!(
            constant,
            ConstantData::Str { value } if value.code_points().count() == 1024
        )));
    }

    #[test]
    fn test_constant_string_subscript_folds_inside_collection() {
        let code = compile_exec(
            "\
values = [item for item in [r\"\\\\'a\\\\'\", r\"\\t3\", r\"\\\\\"[0]]]\n",
        );

        assert!(
            !code
                .instructions
                .iter()
                .any(|unit| matches!(unit.op, Instruction::BinaryOp { .. })),
            "unexpected runtime BINARY_OP after constant subscript folding: {:?}",
            code.instructions
        );
        assert!(code.constants.iter().any(|constant| matches!(
            constant,
            ConstantData::Tuple { elements }
                if elements.len() == 3
                    && matches!(&elements[2], ConstantData::Str { value } if value.to_string() == "\\")
        )));
    }

    #[test]
    fn test_constant_string_subscript_with_surrogate_skips_lossy_fold() {
        let code = compile_exec("value = \"\\ud800\"[0]\n");

        assert!(
            code.instructions.iter().any(|unit| match unit.op {
                Instruction::BinaryOp { op } => {
                    op.get(OpArg::new(u32::from(u8::from(unit.arg))))
                        == oparg::BinaryOperator::Subscr
                }
                _ => false,
            }),
            "expected runtime subscript for surrogate literal, got instructions={:?}",
            code.instructions
        );
    }

    #[test]
    fn test_constant_subscript_folds_in_load_context() {
        let cases = [
            ("value = (1, 2, 3)[0]\n", Some(BigInt::from(1)), None),
            ("value = b\"abc\"[0]\n", Some(BigInt::from(97)), None),
            ("value = \"abc\"[0]\n", None, Some("a")),
        ];

        for (source, expected_int, expected_str) in cases {
            let code = compile_exec(source);
            assert!(
                !code.instructions.iter().any(|unit| matches!(
                    unit.op,
                    Instruction::BinaryOp { op }
                        if op.get(OpArg::new(u32::from(u8::from(unit.arg))))
                            == oparg::BinaryOperator::Subscr
                )),
                "expected folded constant subscript for {source:?}, got instructions={:?}",
                code.instructions
            );

            if let Some(expected_int) = expected_int.as_ref() {
                let has_small_int = code.instructions.iter().any(|unit| {
                    matches!(
                        unit.op,
                        Instruction::LoadSmallInt { i }
                            if BigInt::from(i.get(OpArg::new(u32::from(u8::from(unit.arg)))))
                                == *expected_int
                    )
                });
                let has_const_int = code.constants.iter().any(|constant| {
                    matches!(constant, ConstantData::Integer { value } if value == expected_int)
                });
                assert!(
                    has_small_int || has_const_int,
                    "missing folded integer constant {expected_int} for {source:?}, instructions={:?}",
                    code.instructions
                );
            }

            if let Some(expected_str) = expected_str {
                assert!(
                    code.constants.iter().any(|constant| {
                        matches!(constant, ConstantData::Str { value } if value.to_string() == expected_str)
                    }),
                    "missing folded string constant {expected_str:?} for {source:?}",
                );
            }
        }
    }

    #[test]
    fn test_constant_slice_subscript_folds_in_load_context() {
        let code = compile_exec(
            "\
a = 'hello'[:4]\n\
b = b'abcd'[1:3]\n\
c = (1, 2, 3)[:2]\n",
        );

        assert!(
            !code.instructions.iter().any(|unit| matches!(
                unit.op,
                Instruction::BinaryOp { op }
                    if op.get(OpArg::new(u32::from(u8::from(unit.arg))))
                        == oparg::BinaryOperator::Subscr
            )),
            "expected folded constant slice subscripts, got instructions={:?}",
            code.instructions
        );
        assert!(code.constants.iter().any(|constant| matches!(
            constant,
            ConstantData::Str { value } if value.to_string() == "hell"
        )));
        assert!(code.constants.iter().any(|constant| matches!(
            constant,
            ConstantData::Bytes { value } if value == b"bc"
        )));
        assert!(code.constants.iter().any(|constant| matches!(
            constant,
            ConstantData::Tuple { elements }
                if matches!(
                    elements.as_slice(),
                    [
                        ConstantData::Integer { value: a },
                        ConstantData::Integer { value: b },
                    ] if *a == BigInt::from(1)
                        && *b == BigInt::from(2)
                )
        )));
    }

    #[test]
    fn test_list_of_constant_tuples_uses_list_extend() {
        let code = compile_exec(
            "\
deprecated_cases = [('a', 'b'), ('c', 'd'), ('e', 'f'), ('g', 'h'), ('i', 'j')]
",
        );

        assert!(
            code.instructions
                .iter()
                .any(|unit| matches!(unit.op, Instruction::ListExtend { .. })),
            "expected constant tuple list folding"
        );
    }

    #[test]
    fn test_large_list_of_unary_constants_uses_list_extend() {
        let code = compile_exec(
            "\
values = [-1, not True, ~0, +True, 5]
",
        );

        assert!(
            code.instructions
                .iter()
                .any(|unit| matches!(unit.op, Instruction::ListExtend { .. })),
            "expected unary-folded constants to participate in list folding, got instructions={:?}",
            code.instructions
        );
        assert!(code.constants.iter().any(|constant| matches!(
            constant,
            ConstantData::Tuple { elements }
                if elements.len() == 5
                    && matches!(&elements[0], ConstantData::Integer { value } if *value == BigInt::from(-1))
                    && matches!(&elements[1], ConstantData::Boolean { value } if !value)
                    && matches!(&elements[2], ConstantData::Integer { value } if *value == BigInt::from(-1))
                    && matches!(&elements[3], ConstantData::Integer { value } if *value == BigInt::from(1))
                    && matches!(&elements[4], ConstantData::Integer { value } if *value == BigInt::from(5))
        )));
    }

    #[test]
    fn test_outer_unary_after_binop_folds_before_list_folding() {
        let code = compile_exec(
            "\
values = [2.0**53, -0.5, -2.0**-54]
",
        );

        assert!(
            code.instructions
                .iter()
                .any(|unit| matches!(unit.op, Instruction::ListExtend { .. })),
            "expected binop-folded constants to participate in list folding, got instructions={:?}",
            code.instructions
        );
        assert!(
            !code.instructions.iter().any(|unit| matches!(
                unit.op,
                Instruction::BinaryOp { .. } | Instruction::UnaryNegative
            )),
            "constant expression list should not leave runtime ops, got instructions={:?}",
            code.instructions
        );
        assert!(code.constants.iter().any(|constant| matches!(
            constant,
            ConstantData::Tuple { elements }
                if elements.len() == 3
                    && matches!(&elements[0], ConstantData::Float { value } if *value == 9007199254740992.0)
                    && matches!(&elements[1], ConstantData::Float { value } if *value == -0.5)
                    && matches!(&elements[2], ConstantData::Float { value } if value.is_sign_negative())
        )));
    }

    #[test]
    fn test_negative_integer_power_folds_to_float_constant() {
        let code = compile_exec("value = -3.0 * 2**(-333)\n");

        assert!(
            !code
                .instructions
                .iter()
                .any(|unit| matches!(unit.op, Instruction::BinaryOp { .. })),
            "negative integer power should fold through the enclosing multiply, got instructions={:?}",
            code.instructions
        );
        assert!(code.constants.iter().any(|constant| matches!(
            constant,
            ConstantData::Float { value }
                if value.is_sign_negative() && *value < 0.0 && value.abs() < 1.0e-90
        )));
    }

    #[test]
    fn test_complex_power_constants_fold_like_cpython() {
        let code = compile_exec(
            "\
one = 3j ** 0j
zero = 0j ** 2
",
        );

        assert!(
            !code
                .instructions
                .iter()
                .any(|unit| matches!(unit.op, Instruction::BinaryOp { .. })),
            "safe complex power constants should fold away, got instructions={:?}",
            code.instructions
        );
        assert!(code.constants.iter().any(|constant| matches!(
            constant,
            ConstantData::Complex { value } if value.re == 1.0 && value.im == 0.0
        )));
        assert!(code.constants.iter().any(|constant| matches!(
            constant,
            ConstantData::Complex { value } if value.re == 0.0 && value.im == 0.0
        )));
    }

    #[test]
    fn test_zero_complex_power_exception_constants_do_not_fold() {
        let code = compile_exec("value = 0j ** (3 - 2j)\n");

        assert!(
            code.instructions
                .iter()
                .any(|unit| matches!(unit.op, Instruction::BinaryOp { .. })),
            "zero complex to complex power should stay runtime so ZeroDivisionError is preserved, got instructions={:?}",
            code.instructions
        );
    }

    #[test]
    fn test_large_constant_list_keeps_streaming_build() {
        let source = format!(
            "values = [{}]\n",
            (0..31)
                .map(|i| format!("'v{i}'"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        let code = compile_exec(&source);

        assert!(
            code.instructions
                .iter()
                .any(|unit| matches!(unit.op, Instruction::ListAppend { .. })),
            "large constant lists should keep LIST_APPEND streaming form, got instructions={:?}",
            code.instructions
        );
        assert!(
            !code
                .instructions
                .iter()
                .any(|unit| matches!(unit.op, Instruction::ListExtend { .. })),
            "large constant lists should not fold to LIST_EXTEND, got instructions={:?}",
            code.instructions
        );
    }

    #[test]
    fn test_large_constant_tuple_stream_folds_to_tuple_const() {
        let source = format!(
            "values = ({},)\n",
            (0..31)
                .map(|i| format!("'v{i}'"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        let code = compile_exec(&source);

        assert!(
            !code.instructions.iter().any(|unit| matches!(
                unit.op,
                Instruction::BuildList { .. }
                    | Instruction::ListAppend { .. }
                    | Instruction::CallIntrinsic1 { .. }
            )),
            "large constant tuple should fold the LIST_TO_TUPLE stream, got instructions={:?}",
            code.instructions
        );
        assert!(code.constants.iter().any(|constant| matches!(
            constant,
            ConstantData::Tuple { elements } if elements.len() == 31
        )));
    }

    #[test]
    fn test_annotation_closure_uses_format_varname() {
        let code = compile_exec(
            "\
class C:
    x: int
",
        );
        let annotate = find_code(&code, "__annotate__").expect("missing __annotate__ code");
        let varnames = annotate
            .varnames
            .iter()
            .map(|name| name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(varnames, vec!["format"]);
    }

    #[test]
    fn test_type_param_evaluator_uses_dot_format_varname() {
        let code = compile_exec(
            "\
class C[T: int]:
    pass
",
        );
        let evaluator = find_code(&code, "T").expect("missing type parameter evaluator");
        let varnames = evaluator
            .varnames
            .iter()
            .map(|name| name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(varnames, vec![".format"]);
    }

    #[test]
    fn test_class_annotation_global_resolution_matches_cpython() {
        let class_global = compile_exec(
            "\
X = 'global'
class C:
    locals()['X'] = 'class'
    global X
    y: X
",
        );
        let annotate =
            find_code(&class_global, "__annotate__").expect("missing class __annotate__ code");
        assert!(
            annotate
                .instructions
                .iter()
                .any(|unit| matches!(unit.op, Instruction::LoadGlobal { .. })),
            "expected explicit class global to use LOAD_GLOBAL, got instructions={:?}",
            annotate.instructions
        );
        assert!(
            !annotate
                .instructions
                .iter()
                .any(|unit| matches!(unit.op, Instruction::LoadFromDictOrGlobals { .. })),
            "did not expect class explicit global to use LOAD_FROM_DICT_OR_GLOBALS, got instructions={:?}",
            annotate.instructions
        );

        let outer_global = compile_exec(
            "\
def f():
    global X
    class C:
        locals()['X'] = 'class'
        y: X
",
        );
        let annotate = find_code(&outer_global, "__annotate__")
            .expect("missing nested class __annotate__ code");
        assert!(
            annotate
                .instructions
                .iter()
                .any(|unit| matches!(unit.op, Instruction::LoadFromDictOrGlobals { .. })),
            "expected outer explicit global in class annotation to use LOAD_FROM_DICT_OR_GLOBALS, got instructions={:?}",
            annotate.instructions
        );
    }

    #[test]
    fn test_constant_tuple_binops_fold_like_cpython() {
        let code = compile_exec("value = (1,) * 17 + ('spam',)\n");

        assert!(
            !code
                .instructions
                .iter()
                .any(|unit| matches!(unit.op, Instruction::BinaryOp { .. })),
            "tuple constant binops should fold away, got instructions={:?}",
            code.instructions
        );
        assert!(code.constants.iter().any(|constant| matches!(
            constant,
            ConstantData::Tuple { elements }
                if elements.len() == 18
                    && elements[..17]
                        .iter()
                        .all(|elt| matches!(elt, ConstantData::Integer { value } if *value == BigInt::from(1)))
                    && matches!(&elements[17], ConstantData::Str { value } if value.to_string() == "spam")
        )));
    }

    #[test]
    fn test_constant_list_iterable_uses_tuple() {
        let code = compile_exec(
            "\
def f():
    return {x: y for x, y in [(1, 2), ]}
",
        );
        let f = find_code(&code, "f").expect("missing function code");

        assert!(
            !f.instructions
                .iter()
                .any(|unit| matches!(unit.op, Instruction::BuildList { .. })),
            "constant list iterable should avoid BUILD_LIST before GET_ITER"
        );
        assert!(f.constants.iter().any(|constant| matches!(
            constant,
            ConstantData::Tuple { elements }
                if matches!(
                    elements.as_slice(),
                    [ConstantData::Tuple { elements: inner }]
                        if matches!(
                            inner.as_slice(),
                            [
                                ConstantData::Integer { .. },
                                ConstantData::Integer { .. }
                            ]
                        )
                )
        )));
    }

    #[test]
    fn test_constant_set_iterable_uses_frozenset_const() {
        let code = compile_exec(
            "\
def f():
    return [x for x in {1, 2, 3}]
",
        );
        let f = find_code(&code, "f").expect("missing function code");

        assert!(
            !f.instructions
                .iter()
                .any(|unit| matches!(unit.op, Instruction::BuildSet { .. })),
            "constant set iterable should avoid BUILD_SET before GET_ITER"
        );
        assert!(f.constants.iter().any(|constant| matches!(
            constant,
            ConstantData::Frozenset { elements }
                if matches!(
                    elements.as_slice(),
                    [
                        ConstantData::Integer { .. },
                        ConstantData::Integer { .. },
                        ConstantData::Integer { .. }
                    ]
                )
        )));
    }

    #[test]
    fn test_constant_list_membership_uses_tuple_const() {
        let code = compile_exec(
            "\
f = lambda x: x in [1, 2, 3]
",
        );
        let lambda = find_code(&code, "<lambda>").expect("missing lambda code");

        assert!(
            !lambda
                .instructions
                .iter()
                .any(|unit| matches!(unit.op, Instruction::BuildList { .. })),
            "constant list membership should avoid BUILD_LIST before CONTAINS_OP"
        );
        assert!(lambda.constants.iter().any(|constant| matches!(
            constant,
            ConstantData::Tuple { elements }
                if matches!(
                    elements.as_slice(),
                    [
                        ConstantData::Integer { .. },
                        ConstantData::Integer { .. },
                        ConstantData::Integer { .. }
                    ]
                )
        )));
    }

    #[test]
    fn test_small_constant_set_membership_uses_frozenset_const() {
        let code = compile_exec(
            "\
f = lambda x: x in {0}
",
        );
        let lambda = find_code(&code, "<lambda>").expect("missing lambda code");

        assert!(
            !lambda
                .instructions
                .iter()
                .any(|unit| matches!(unit.op, Instruction::BuildSet { .. })),
            "constant set membership should avoid BUILD_SET before CONTAINS_OP"
        );
        assert!(lambda.constants.iter().any(|constant| matches!(
            constant,
            ConstantData::Frozenset { elements }
                if matches!(elements.as_slice(), [ConstantData::Integer { value }] if *value == BigInt::from(0))
        )));
    }

    #[test]
    fn test_nonconstant_list_membership_uses_tuple() {
        let code = compile_exec(
            "\
def f(a, b, c, x):
    return x in [a, b, c]
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(2).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::BuildTuple { .. },
                        Instruction::ContainsOp { .. }
                    ]
                )
            }),
            "expected BUILD_TUPLE before CONTAINS_OP for non-constant list membership, got ops={ops:?}"
        );
    }

    #[test]
    fn test_starred_tuple_iterable_drops_list_to_tuple_before_get_iter() {
        let code = compile_exec(
            "\
def f(a, b, c):
    for x in *a, *b, *c:
        pass
",
        );
        let f = find_code(&code, "f").expect("missing function code");

        assert!(
            !has_intrinsic_1(f, IntrinsicFunction1::ListToTuple),
            "LIST_TO_TUPLE should be removed before GET_ITER in for-iterable context"
        );
        assert!(
            f.instructions
                .iter()
                .any(|unit| matches!(unit.op, Instruction::GetIter)),
            "expected GET_ITER in for loop"
        );
    }

    #[test]
    fn test_comprehension_single_list_iterable_uses_tuple() {
        let code = compile_exec(
            "\
def g():
    [x for x in [(yield 1)]]
",
        );
        let g = find_code(&code, "g").expect("missing g code");
        let ops: Vec<_> = g
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(2).any(|window| {
                matches!(
                    window,
                    [Instruction::BuildTuple { .. }, Instruction::GetIter]
                )
            }),
            "expected BUILD_TUPLE before GET_ITER for single-item list iterable in comprehension, got ops={ops:?}"
        );
    }

    #[test]
    fn test_nested_comprehension_list_iterable_uses_tuple() {
        let code = compile_exec(
            "\
def f():
    return [[y for y in [x, x + 1]] for x in [1, 3, 5]]
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(2).any(|window| {
                matches!(
                    window,
                    [Instruction::BuildTuple { .. }, Instruction::GetIter]
                )
            }),
            "expected BUILD_TUPLE before GET_ITER for nested list iterable in comprehension, got ops={ops:?}"
        );
    }

    #[test]
    fn test_comprehension_singleton_sub_iter_uses_assignment_idiom() {
        let code = compile_exec(
            "\
def f():
    return {j: j * j for i in range(4) for j in [i + 1]}
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let for_iter_count = f
            .instructions
            .iter()
            .filter(|unit| matches!(unit.op, Instruction::ForIter { .. }))
            .count();
        let has_map_add_depth_2 = f.instructions.iter().any(|unit| {
            matches!(
                unit.op,
                Instruction::MapAdd { i }
                    if i.get(OpArg::new(u32::from(u8::from(unit.arg)))) == 2
            )
        });

        assert_eq!(
            for_iter_count, 1,
            "singleton sub-iter should not emit its own FOR_ITER, got instructions={:?}",
            f.instructions
        );
        assert!(
            has_map_add_depth_2,
            "assignment-idiom dictcomp should use MAP_ADD depth 2, got instructions={:?}",
            f.instructions
        );
        assert!(
            !f.instructions
                .iter()
                .any(|unit| matches!(unit.op, Instruction::BuildTuple { .. })),
            "singleton sub-iter should not materialize an iterator tuple, got instructions={:?}",
            f.instructions
        );
    }

    #[test]
    fn test_constant_comprehension_iterable_with_unary_int_uses_tuple_const() {
        let code = compile_exec(
            "\
l = lambda : [2 < x for x in [-1, 3, 0]]
",
        );
        let lambda = find_code(&code, "<lambda>").expect("missing lambda code");

        assert!(
            lambda.constants.iter().any(|constant| matches!(
                constant,
                ConstantData::Tuple { elements }
                    if matches!(
                        elements.as_slice(),
                        [
                            ConstantData::Integer { .. },
                            ConstantData::Integer { .. },
                            ConstantData::Integer { .. }
                        ]
                    )
            )),
            "expected folded tuple constant for comprehension iterable"
        );
    }

    #[test]
    fn test_module_scope_listcomp_is_inlined() {
        let code = compile_exec("values = [i for i in range(3)]\n");

        assert!(
            find_code(&code, "<listcomp>").is_none(),
            "module-scope list comprehension should be inlined"
        );
        assert!(
            code.instructions
                .iter()
                .any(|unit| matches!(unit.op, Instruction::LoadFastAndClear { .. })),
            "inlined module-scope list comprehension should use LOAD_FAST_AND_CLEAR, got instructions={:?}",
            code.instructions
        );
    }

    #[test]
    fn test_module_scope_dictcomp_is_inlined() {
        let code = compile_exec("mapping = {i: i for i in range(3)}\n");

        assert!(
            find_code(&code, "<dictcomp>").is_none(),
            "module-scope dict comprehension should be inlined"
        );
        assert!(
            code.instructions
                .iter()
                .any(|unit| matches!(unit.op, Instruction::LoadFastAndClear { .. })),
            "inlined module-scope dict comprehension should use LOAD_FAST_AND_CLEAR, got instructions={:?}",
            code.instructions
        );
    }

    #[test]
    fn test_async_dictcomp_in_async_function_is_inlined() {
        let code = compile_exec(
            "\
async def f(items):
    return {item: item async for item in items}
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            find_code(&code, "<dictcomp>").is_none(),
            "async dict comprehension should be inlined"
        );
        assert!(
            ops.iter().any(|op| matches!(op, Instruction::GetAIter)),
            "inlined async dict comprehension should keep GET_AITER in outer code, got ops={ops:?}"
        );
        assert!(
            ops.iter()
                .any(|op| matches!(op, Instruction::LoadFastAndClear { .. })),
            "inlined async dict comprehension should use LOAD_FAST_AND_CLEAR, got ops={ops:?}"
        );
        assert!(
            !ops.iter().any(|op| matches!(op, Instruction::MakeFunction)),
            "inlined async dict comprehension should not materialize MAKE_FUNCTION, got ops={ops:?}"
        );
    }

    #[test]
    fn test_async_inlined_comprehension_inlines_restore_return_into_end_async_for() {
        let code = compile_exec(
            "\
async def f():
    return [i + 1 async for i in g([10, 20])]
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(8).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::EndAsyncFor,
                        Instruction::Swap { .. },
                        Instruction::StoreFast { .. },
                        Instruction::ReturnValue,
                        Instruction::Swap { .. },
                        Instruction::PopTop,
                        Instruction::Swap { .. },
                        Instruction::StoreFast { .. },
                    ]
                )
            }),
            "expected CPython-style restore+return inlined into END_ASYNC_FOR before cleanup, got ops={ops:?}"
        );
        assert!(
            !ops.windows(2).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::EndAsyncFor,
                        Instruction::JumpForward { .. } | Instruction::JumpBackward { .. },
                    ]
                )
            }),
            "unexpected jump from END_ASYNC_FOR to the normal restore tail, got ops={ops:?}"
        );
    }

    #[test]
    fn test_await_cleanup_throw_falls_through_until_cold_reorder() {
        let code = compile_exec(
            "\
async def f():
    await 1
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(3).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::CleanupThrow,
                        Instruction::JumpBackwardNoInterrupt { .. },
                        Instruction::CallIntrinsic1 { .. },
                    ]
                )
            }),
            "expected CPython-style cold CLEANUP_THROW jump before StopIteration handler, got ops={ops:?}"
        );
        assert!(
            !ops.windows(2).any(|window| {
                matches!(window, [Instruction::CleanupThrow, Instruction::EndSend])
            }),
            "CLEANUP_THROW should not inline the normal END_SEND return tail, got ops={ops:?}"
        );
    }

    #[test]
    fn test_match_async_inlined_comprehension_success_jump_no_interrupt() {
        let code = compile_exec(
            "\
async def f(name_3, name_5):
    match b'':
        case True:
            pass
        case name_5 if f'e':
            {name_3: f async for name_2 in name_5}
        case []:
            pass
    [[]]
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(3).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::PopTop,
                        Instruction::StoreFast { .. },
                        Instruction::JumpBackwardNoInterrupt { .. },
                    ]
                )
            }),
            "expected CPython-style no-interrupt match success backedge after async comprehension cleanup, got ops={ops:?}"
        );
        assert!(
            !ops.windows(3).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::PopTop,
                        Instruction::StoreFast { .. },
                        Instruction::JumpBackward { .. },
                    ]
                )
            }),
            "match success cleanup backedge should not be a regular interrupting jump, got ops={ops:?}"
        );
    }

    #[test]
    fn test_for_loop_if_return_reorders_continue_backedge_before_exit_body() {
        let code = compile_exec(
            "\
def f(items, occurrence):
    for item in items:
        if item:
            occurrence -= 1
            if not occurrence:
                return item
    return None
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(3).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::PopJumpIfFalse { .. },
                        Instruction::NotTaken,
                        Instruction::JumpBackward { .. },
                    ]
                )
            }),
            "expected CPython-style inverted return guard followed by loop backedge, got ops={ops:?}"
        );
        assert!(
            !ops.windows(3).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::PopJumpIfTrue { .. },
                        Instruction::NotTaken,
                        Instruction::LoadFast { .. } | Instruction::LoadFastBorrow { .. },
                    ]
                )
            }),
            "return guard should not fall through into the return body before the loop backedge, got ops={ops:?}"
        );
    }

    #[test]
    fn test_sync_with_after_async_for_keeps_end_async_for_line_marker() {
        let code = compile_exec(
            "\
async def f(cm, source, tgt):
    with cm:
        async for tgt[0] in source():
            pass
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(6).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::EndAsyncFor,
                        Instruction::Nop,
                        Instruction::LoadConst { .. },
                        Instruction::LoadConst { .. },
                        Instruction::LoadConst { .. },
                        Instruction::Call { .. },
                    ]
                )
            }),
            "expected CPython-style line-marker NOP between END_ASYNC_FOR and with cleanup, got ops={ops:?}"
        );
    }

    #[test]
    fn test_genexpr_with_async_comprehension_element_is_async_generator() {
        let code = compile_exec(
            "\
async def f():
    gen = ([i async for i in asynciter([1, 2])] for j in [10, 20])
    return [x async for x in gen]
",
        );
        let genexpr = find_code(&code, "<genexpr>").expect("missing genexpr code");
        let units: Vec<_> = genexpr
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();

        assert!(
            units.windows(2).any(|window| {
                let [wrap, yield_value] = window else {
                    return false;
                };
                matches!(yield_value.op, Instruction::YieldValue { .. })
                    && match wrap.op {
                        Instruction::CallIntrinsic1 { func } => {
                            func.get(OpArg::new(u32::from(u8::from(wrap.arg))))
                                == bytecode::IntrinsicFunction1::AsyncGenWrap
                        }
                        _ => false,
                    }
            }),
            "expected CPython-style ASYNC_GEN_WRAP before genexpr yield, got units={units:?}"
        );
    }

    #[test]
    fn test_nested_module_scope_dictcomp_symbols_are_local() {
        let symbol_table = scan_program_symbol_table(
            "\
deoptmap = {
    specialized: base
    for base, family in _specializations.items()
    for specialized in family
}
",
        );

        for name in ["base", "family", "specialized"] {
            let symbol = symbol_table
                .lookup(name)
                .unwrap_or_else(|| panic!("missing module symbol {name}"));
            assert_eq!(
                symbol.scope,
                SymbolScope::Local,
                "expected module-scope inlined comprehension symbol {name} to be Local, got {symbol:?}"
            );
        }

        let comp = symbol_table
            .sub_tables
            .first()
            .expect("missing comprehension symbol table");
        assert!(comp.comp_inlined, "expected comprehension to be inlined");
        for name in ["base", "family", "specialized"] {
            let symbol = comp
                .lookup(name)
                .unwrap_or_else(|| panic!("missing comprehension symbol {name}"));
            assert_eq!(
                symbol.scope,
                SymbolScope::Local,
                "expected comprehension symbol {name} to be Local, got {symbol:?}"
            );
        }
    }

    #[test]
    fn test_nested_module_scope_dictcomp_uses_fast_locals() {
        let code = compile_exec(
            "\
deoptmap = {
    specialized: base
    for base, family in _specializations.items()
    for specialized in family
}
",
        );
        let ops: Vec<_> = code
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.iter()
                .any(|op| matches!(op, Instruction::StoreFastStoreFast { .. })),
            "expected outer target unpack to use STORE_FAST_STORE_FAST, got ops={ops:?}"
        );
        assert!(
            ops.iter().any(|op| matches!(
                op,
                Instruction::StoreFastLoadFast { .. }
                    | Instruction::LoadFastBorrowLoadFastBorrow { .. }
            )),
            "expected inner target/store-use path to use fast locals, got ops={ops:?}"
        );
        assert!(
            ops.iter()
                .filter(|op| matches!(op, Instruction::LoadName { .. }))
                .count()
                <= 1,
            "unexpected extra LOAD_NAME ops in nested inlined comprehension, got ops={ops:?}"
        );
        assert!(
            ops.iter()
                .filter(|op| matches!(op, Instruction::StoreName { .. }))
                .count()
                <= 1,
            "unexpected extra STORE_NAME ops in nested inlined comprehension, got ops={ops:?}"
        );
    }

    #[test]
    fn test_module_scope_inlined_comprehension_keeps_outer_iter_as_name_lookup() {
        let code = compile_exec(
            "\
path_separators = ['/']
_pathseps_with_colon = {f':{s}' for s in path_separators}
",
        );
        let ops: Vec<_> = code
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        let load_name_path = ops
            .windows(2)
            .any(|window| matches!(window, [Instruction::LoadName { .. }, Instruction::GetIter]));
        assert!(
            load_name_path,
            "expected outer iterable to stay a NAME lookup before GET_ITER, got ops={ops:?}"
        );
        assert!(
            !ops.windows(2).any(|window| matches!(
                window,
                [
                    Instruction::LoadFast { .. } | Instruction::LoadFastCheck { .. },
                    Instruction::GetIter
                ]
            )),
            "module local outer iterable should not become a fast local, got ops={ops:?}"
        );
        assert!(
            ops.iter().any(|op| matches!(
                op,
                Instruction::StoreFastLoadFast { .. } | Instruction::StoreFast { .. }
            )),
            "comprehension target should still use fast locals, got ops={ops:?}"
        );
    }

    #[test]
    fn test_function_scope_inlined_comprehension_restore_keeps_swap_before_duplicate_store() {
        let code = compile_exec(
            "\
def f():
    a = [1 for a in [0]]
    return 1
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(4).any(|window| matches!(
                window,
                [
                    Instruction::PopIter,
                    Instruction::Swap { .. },
                    Instruction::StoreFast { .. },
                    Instruction::StoreFast { .. }
                ]
            )),
            "expected PopIter/SWAP 2/STORE_FAST/STORE_FAST restore tail, got ops={ops:?}"
        );
    }

    #[test]
    fn test_inlined_comprehension_restore_does_not_form_store_fast_load_fast() {
        let code = compile_exec(
            "\
def f(e):
    e[1:3] = [g(i) for i in range(2)]

def g(datadir):
    files = [filename[:-4] for filename in sorted(os.listdir(datadir)) if filename.endswith('.xml')]
    input_files = [filename for filename in files if filename.startswith('in')]
    return files, input_files
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(7).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::EndFor,
                        Instruction::PopIter,
                        Instruction::Swap { .. },
                        Instruction::StoreFast { .. },
                        Instruction::LoadFastBorrow { .. },
                        Instruction::LoadConst { .. },
                        Instruction::StoreSubscr,
                    ]
                )
            }),
            "expected CPython-style inlined comprehension restore before slice store, got ops={ops:?}"
        );
        assert!(
            !ops.windows(3).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::StoreFastLoadFast { .. },
                        Instruction::LoadConst { .. },
                        Instruction::StoreSubscr,
                    ]
                )
            }),
            "inlined comprehension restore should not be folded into STORE_FAST_LOAD_FAST, got ops={ops:?}"
        );

        let g = find_code(&code, "g").expect("missing g code");
        let g_ops: Vec<_> = g
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();
        assert!(
            g_ops.windows(4).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::EndFor,
                        Instruction::PopIter,
                        Instruction::StoreFast { .. },
                        Instruction::StoreFast { .. },
                    ]
                )
            }),
            "expected CPython-style static swap over STORE_FAST_MAYBE_NULL restore, got ops={g_ops:?}"
        );
        assert!(
            !g_ops.windows(5).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::EndFor,
                        Instruction::PopIter,
                        Instruction::Swap { .. },
                        Instruction::StoreFast { .. },
                        Instruction::StoreFast { .. },
                    ]
                )
            }),
            "inlined comprehension restore should statically remove SWAP before adjacent stores, got ops={g_ops:?}"
        );
    }

    #[test]
    fn test_single_mode_folded_multiline_constant_does_not_leave_nops() {
        let code = compile_single(
            "\
(-
 -
 -
 1)
",
        );

        assert!(
            !code
                .instructions
                .iter()
                .any(|unit| matches!(unit.op, Instruction::Nop)),
            "expected folded single-mode multiline constant to drop NOP anchors, got instructions={:?}",
            code.instructions
        );
    }

    #[test]
    fn test_folded_multiline_tuple_constant_does_not_leave_operand_nops() {
        let code = compile_exec(
            "\
values = (
    (1 + 1j, 0 + 0j),
    (1 + 1j, 0.0),
    (1 + 1j, 0),
)
",
        );

        assert!(
            !code
                .instructions
                .iter()
                .any(|unit| matches!(unit.op, Instruction::Nop)),
            "expected CPython nop_out-style folded tuple operands to have no surviving NOPs, got instructions={:?}",
            code.instructions
        );
    }

    #[test]
    fn test_folded_multiline_bytes_binop_does_not_leave_operand_nops() {
        let code = compile_exec(
            "\
def f(self, out):
    self.assertIn(
        b'gnu' + (b'/123' * 125) + b'/longlink' + (b'/123' * 125) + b'/longname',
        out)
",
        );
        let f = find_code(&code, "f").expect("missing f code");

        assert!(
            !f.instructions
                .iter()
                .any(|unit| matches!(unit.op, Instruction::Nop)),
            "expected CPython nop_out-style folded operands to have no surviving NOPs, got instructions={:?}",
            f.instructions
        );
    }

    #[test]
    fn test_folded_binop_at_branch_body_start_does_not_leave_nop() {
        let code = compile_exec(
            "\
def f(sys):
    if sys.platform == 'win32':
        component = 'd' * 25
    return component
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            !ops.windows(3).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::NotTaken,
                        Instruction::Nop,
                        Instruction::LoadConst { .. }
                    ]
                )
            }),
            "expected CPython nop_out-style folded branch body to drop operand NOP, got ops={ops:?}",
        );
    }

    #[test]
    fn test_folded_iterable_at_assert_target_does_not_leave_nop() {
        let code = compile_exec(
            r#"
def f(caches, non_caches):
    assert 1 / 3 <= caches / non_caches, "this test needs more caches!"
    for show_caches in (False, True):
        pass
"#,
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            !ops.windows(2).any(|window| {
                matches!(window, [Instruction::Nop, Instruction::LoadConst { .. }])
            }),
            "expected folded for-iterable at assert target to drop operand NOP, got ops={ops:?}",
        );
    }

    #[test]
    fn test_multiline_unpack_target_uses_element_locations() {
        let code = compile_exec(
            "\
def f(cm):
    with cm as (_,
                filename_2):
        return filename_2
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            !ops.iter()
                .any(|op| matches!(op, Instruction::StoreFastStoreFast { .. })),
            "expected multiline target elements to keep separate STORE_FAST instructions, got ops={ops:?}",
        );
    }

    #[test]
    fn test_or_condition_in_jump_context_uses_shared_true_fallthrough() {
        let code = compile_exec(
            "\
def f(lines):
    for line in lines:
        if line.startswith('--') or not line.strip():
            continue
        return line
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        let first_pop_jump = ops
            .iter()
            .find(|op| {
                matches!(
                    op,
                    Instruction::PopJumpIfTrue { .. } | Instruction::PopJumpIfFalse { .. }
                )
            })
            .copied()
            .expect("missing conditional jump");
        assert!(
            matches!(first_pop_jump, Instruction::PopJumpIfTrue { .. }),
            "expected first OR branch to jump on true into shared fallthrough, got ops={ops:?}"
        );
    }

    #[test]
    fn test_loop_break_bool_chain_reorders_false_path_to_jump_back() {
        let code = compile_exec(
            "\
def f(filters, text, category, module, lineno, defaultaction):
    for item in filters:
        action, msg, cat, mod, ln = item
        if ((msg is None or msg.match(text)) and
            issubclass(category, cat) and
            (mod is None or mod.match(module)) and
            (ln == 0 or lineno == ln)):
            break
    else:
        action = defaultaction
    return action
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(5).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::ToBool,
                        Instruction::PopJumpIfTrue { .. },
                        Instruction::NotTaken,
                        Instruction::JumpBackward { .. }
                            | Instruction::JumpBackwardNoInterrupt { .. },
                        Instruction::LoadGlobal { .. },
                    ]
                )
            }),
            "expected CPython-style false path to fall through into loop jump-back, got ops={ops:?}"
        );
    }

    #[test]
    fn test_loop_conditional_body_keeps_duplicate_jump_back_paths() {
        let code = compile_exec(
            "\
def f(new, old):
    for replace in ['__module__', '__name__', '__qualname__', '__doc__']:
        if hasattr(old, replace):
            setattr(new, replace, getattr(old, replace))
    return new
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        let jump_back_count = ops
            .iter()
            .filter(|op| {
                matches!(
                    op,
                    Instruction::JumpBackward { .. } | Instruction::JumpBackwardNoInterrupt { .. }
                )
            })
            .count();
        assert!(
            jump_back_count >= 2,
            "expected separate false-path and body jump-back blocks, got ops={ops:?}"
        );
        assert!(
            ops.windows(5).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::ToBool,
                        Instruction::PopJumpIfTrue { .. },
                        Instruction::NotTaken,
                        Instruction::JumpBackward { .. }
                            | Instruction::JumpBackwardNoInterrupt { .. },
                        Instruction::LoadGlobal { .. },
                    ]
                )
            }),
            "expected false path to jump back before body, got ops={ops:?}"
        );
    }

    #[test]
    fn test_line_bearing_loop_if_false_backedge_keeps_body_before_jump_back() {
        let code = compile_exec(
            "\
def f(self, replacement_pairs):
    for n, d in [(19, '%OC'), (2, '%Ow')]:
        if self.LC_alt_digits is None:
            s = str(n)
            replacement_pairs.append((s, d))
            if n < 10:
                replacement_pairs.append((s[1], d))
        elif len(self.LC_alt_digits) > n:
            replacement_pairs.append((self.LC_alt_digits[n], d))
        else:
            replacement_pairs.append((d, d))
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(5).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::CompareOp { .. },
                        Instruction::PopJumpIfFalse { .. },
                        Instruction::NotTaken,
                        Instruction::LoadFastBorrow { .. } | Instruction::LoadFast { .. },
                        Instruction::LoadAttr { .. },
                    ]
                )
            }),
            "expected CPython-style line-bearing false target to keep body before backedge, got ops={ops:?}"
        );
        assert!(
            !ops.windows(6).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::CompareOp { .. },
                        Instruction::PopJumpIfTrue { .. },
                        Instruction::NotTaken,
                        Instruction::JumpBackward { .. }
                            | Instruction::JumpBackwardNoInterrupt { .. },
                        Instruction::LoadFastBorrow { .. } | Instruction::LoadFast { .. },
                        Instruction::LoadAttr { .. },
                    ]
                )
            }),
            "unexpected no-lineno-style inverted loop-if body, got ops={ops:?}"
        );
    }

    #[test]
    fn test_loop_elif_nested_if_false_backedge_keeps_body_before_jump_back() {
        let code = compile_exec(
            "\
def f(keys, parse_int, d, ampm, AM, PM):
    hour = minute = 0
    for group_key in keys:
        if group_key == 'I':
            hour = parse_int(d['I'])
            if ampm in ('', AM):
                if hour == 12:
                    hour = 0
            elif ampm == PM:
                if hour != 12:
                    hour += 12
        elif group_key == 'M':
            minute = parse_int(d['M'])
    return hour, minute
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(5).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::CompareOp { .. },
                        Instruction::PopJumpIfFalse { .. },
                        Instruction::NotTaken,
                        Instruction::LoadFastBorrow { .. } | Instruction::LoadFast { .. },
                        Instruction::LoadSmallInt { .. },
                    ]
                )
            }),
            "expected CPython-style nested elif body before false backedge, got ops={ops:?}"
        );
        assert!(
            !ops.windows(6).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::CompareOp { .. },
                        Instruction::PopJumpIfTrue { .. },
                        Instruction::NotTaken,
                        Instruction::JumpBackward { .. }
                            | Instruction::JumpBackwardNoInterrupt { .. },
                        Instruction::LoadFastBorrow { .. } | Instruction::LoadFast { .. },
                        Instruction::LoadSmallInt { .. },
                    ]
                )
            }),
            "unexpected inverted nested elif false path before body, got ops={ops:?}"
        );
        assert!(
            ops.windows(15).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::CompareOp { .. },
                        Instruction::PopJumpIfFalse { .. },
                        Instruction::NotTaken,
                        Instruction::LoadFastBorrow { .. } | Instruction::LoadFast { .. },
                        Instruction::LoadSmallInt { .. },
                        Instruction::CompareOp { .. },
                        Instruction::PopJumpIfFalse { .. },
                        Instruction::NotTaken,
                        Instruction::LoadFastBorrow { .. } | Instruction::LoadFast { .. },
                        Instruction::LoadSmallInt { .. },
                        Instruction::BinaryOp { .. },
                        Instruction::StoreFast { .. },
                        Instruction::JumpBackward { .. }
                            | Instruction::JumpBackwardNoInterrupt { .. },
                        Instruction::JumpBackward { .. }
                            | Instruction::JumpBackwardNoInterrupt { .. },
                        Instruction::JumpBackward { .. }
                            | Instruction::JumpBackwardNoInterrupt { .. },
                    ]
                )
            }),
            "expected CPython-style duplicated body/false loop exits for nested elif, got ops={ops:?}"
        );
    }

    #[test]
    fn test_loop_multiblock_conditional_body_keeps_body_before_jump_back() {
        let code = compile_exec(
            "\
def f(random, d, f):
    for dummy in range(100):
        k = random.choice('abc')
        if random.random() < 0.2:
            if k in d:
                del d[k]
                del f[k]
        else:
            v = random.choice((1, 2))
            d[k] = v
            f[k] = v
            check(f[k], v)
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(5).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::ContainsOp { .. },
                        Instruction::PopJumpIfFalse { .. },
                        Instruction::NotTaken,
                        Instruction::LoadFastBorrowLoadFastBorrow { .. },
                        Instruction::DeleteSubscr,
                    ]
                )
            }),
            "expected CPython-style multi-block body before false jump-back, got ops={ops:?}"
        );
    }

    #[test]
    fn test_loop_not_conditional_body_threads_true_path_to_jump_back() {
        let code = compile_exec(
            "\
def f(xs):
    for x in xs:
        if not x:
            g(x)
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(5).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::ToBool,
                        Instruction::PopJumpIfFalse { .. },
                        Instruction::NotTaken,
                        Instruction::JumpBackward { .. }
                            | Instruction::JumpBackwardNoInterrupt { .. },
                        Instruction::LoadGlobal { .. },
                    ]
                )
            }),
            "expected CPython-style true path to jump back before not-body, got ops={ops:?}"
        );
    }

    #[test]
    fn test_loop_not_in_conditional_body_threads_true_path_to_jump_back() {
        let code = compile_exec(
            "\
def f(native, array):
    for k in native:
        if not k in 'bBhHiIlLfd':
            del array[k]
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(5).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::ContainsOp { .. },
                        Instruction::PopJumpIfFalse { .. },
                        Instruction::NotTaken,
                        Instruction::JumpBackward { .. }
                            | Instruction::JumpBackwardNoInterrupt { .. },
                        Instruction::LoadFastBorrowLoadFastBorrow { .. }
                            | Instruction::LoadFastLoadFast { .. },
                    ]
                )
            }),
            "expected CPython-style true path to jump back before not-in body, got ops={ops:?}"
        );
    }

    #[test]
    fn test_loop_if_pass_uses_line_bearing_jump_back_instead_of_nop() {
        let code = compile_exec(
            "\
def f(x, y):
    for i in x:
        if y:
            pass
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(5).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::ToBool,
                        Instruction::PopJumpIfTrue { .. },
                        Instruction::NotTaken,
                        Instruction::JumpBackward { .. }
                            | Instruction::JumpBackwardNoInterrupt { .. },
                        Instruction::JumpBackward { .. }
                            | Instruction::JumpBackwardNoInterrupt { .. },
                    ]
                )
            }),
            "expected CPython-style synthetic false-path jump-back plus body jump-back, got ops={ops:?}"
        );
        assert!(
            !ops.iter().any(|op| matches!(op, Instruction::Nop)),
            "expected pass body line to attach to loop backedge instead of leaving a NOP, got ops={ops:?}"
        );
    }

    #[test]
    fn test_constant_true_while_pass_keeps_loop_header_nop() {
        let code = compile_exec(
            "\
def f():
    while 1:
        pass
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(2).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::Nop,
                        Instruction::JumpBackward { .. }
                            | Instruction::JumpBackwardNoInterrupt { .. },
                    ]
                )
            }),
            "expected CPython-style loop-header NOP before self backedge, got ops={ops:?}"
        );
    }

    #[test]
    fn test_nested_if_shared_jump_back_target_is_duplicated() {
        let code = compile_exec(
            "\
def f(s, size, encodeSetO, encodeWhiteSpace):
    inShift = True
    base64bits = 0
    out = []
    for i, ch in enumerate(s):
        if base64bits == 0:
            if i + 1 < size:
                ch2 = s[i + 1]
                if E(ch2, encodeSetO, encodeWhiteSpace):
                    if B(ch2) or ch2 == '-':
                        out.append(b'-')
                    inShift = False
            else:
                out.append(b'-')
                inShift = False
    return out
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(6).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::PopTop,
                        Instruction::LoadConst { .. },
                        Instruction::StoreFast { .. },
                        Instruction::JumpBackward { .. }
                            | Instruction::JumpBackwardNoInterrupt { .. },
                        Instruction::JumpBackward { .. }
                            | Instruction::JumpBackwardNoInterrupt { .. },
                        Instruction::LoadFast { .. } | Instruction::LoadFastBorrow { .. },
                    ]
                )
            }),
            "expected separate nested-if and outer-if jump-back tails, got ops={ops:?}"
        );
    }

    #[test]
    fn test_protected_loop_conditional_keeps_forward_body_entry() {
        let code = compile_exec(
            "\
def outer(it, C1):
    def f():
        for x in it:
            try:
                if C1:
                    yield 2
            except OSError:
                pass
    return f
",
        );
        let outer = find_code(&code, "outer").expect("missing outer code");
        let f = find_code(outer, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(7).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::ToBool,
                        Instruction::PopJumpIfFalse { .. },
                        Instruction::NotTaken,
                        Instruction::LoadSmallInt { .. },
                        Instruction::YieldValue { .. },
                        Instruction::Resume { .. },
                        Instruction::PopTop,
                    ]
                )
            }),
            "expected protected conditional to keep CPython-style forward body entry, got ops={ops:?}"
        );
    }

    #[test]
    fn test_nested_except_false_path_duplicates_pop_except_jump_back_tail() {
        let code = compile_exec(
            "\
def f(it, C3):
    for x in it:
        try:
            X = 3
        except OSError:
            try:
                if C3:
                    X = 4
            except OSError:
                pass
    return 42
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(6).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::LoadSmallInt { .. },
                        Instruction::StoreFast { .. },
                        Instruction::PopExcept,
                        Instruction::JumpBackward { .. }
                            | Instruction::JumpBackwardNoInterrupt { .. },
                        Instruction::PopExcept,
                        Instruction::JumpBackward { .. }
                            | Instruction::JumpBackwardNoInterrupt { .. },
                    ]
                )
            }),
            "expected CPython-style duplicated false-path exit tail, got ops={ops:?}"
        );
    }

    #[test]
    fn test_more_nested_except_false_paths_duplicate_all_jump_back_tails() {
        let code = compile_exec(
            "\
def f(it, C3, C4):
    for x in it:
        try:
            X = 3
        except OSError:
            try:
                if C3:
                    if C4:
                        X = 4
            except OSError:
                try:
                    if C3:
                        if C4:
                            X = 5
                except OSError:
                    pass
    return 42
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(8).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::LoadSmallInt { .. },
                        Instruction::StoreFast { .. },
                        Instruction::PopExcept,
                        Instruction::JumpBackward { .. }
                            | Instruction::JumpBackwardNoInterrupt { .. },
                        Instruction::PopExcept,
                        Instruction::JumpBackward { .. }
                            | Instruction::JumpBackwardNoInterrupt { .. },
                        Instruction::PopExcept,
                        Instruction::JumpBackward { .. }
                            | Instruction::JumpBackwardNoInterrupt { .. },
                    ]
                )
            }),
            "expected CPython-style duplicated nested false-path exit tails, got ops={ops:?}"
        );
    }

    #[test]
    fn test_no_wraparound_jump_keeps_forward_hop_before_loop_backedge() {
        let code = compile_exec(
            "\
def while_not_chained(a, b, c):
    while not (a < b < c):
        pass
",
        );
        let f = find_code(&code, "while_not_chained").expect("missing while_not_chained code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(5).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::PopJumpIfTrue { .. },
                        Instruction::NotTaken,
                        Instruction::JumpForward { .. },
                        Instruction::PopTop,
                        Instruction::JumpBackward { .. }
                            | Instruction::JumpBackwardNoInterrupt { .. },
                    ]
                )
            }),
            "expected CPython-style no-wraparound forward hop before the loop backedge, got ops={ops:?}"
        );
    }

    #[test]
    fn test_while_break_else_keeps_true_edge_into_forward_break_body() {
        let code = compile_exec(
            "\
def f(i):
    while i:
        i -= 1
        if i < 4:
            break
    else:
        print('x')
    print('y')
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(4).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::PopJumpIfTrue { .. },
                        Instruction::NotTaken,
                        Instruction::JumpBackward { .. }
                            | Instruction::JumpBackwardNoInterrupt { .. },
                        Instruction::JumpForward { .. },
                    ]
                )
            }),
            "expected CPython-style true edge into forward break body with false path falling into the loop backedge, got ops={ops:?}"
        );
    }

    #[test]
    fn test_nested_if_continue_reorders_false_path_to_loop_backedge() {
        let code = compile_exec(
            "\
def f(items, changes):
    for x in items:
        if not x:
            if x in changes:
                raise TypeError
            continue
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(7).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::ToBool,
                        Instruction::PopJumpIfFalse { .. },
                        Instruction::NotTaken,
                        Instruction::JumpBackward { .. }
                            | Instruction::JumpBackwardNoInterrupt { .. },
                        Instruction::LoadFastBorrowLoadFastBorrow { .. }
                            | Instruction::LoadFastLoadFast { .. },
                        Instruction::ContainsOp { .. },
                        Instruction::PopJumpIfFalse { .. },
                    ]
                )
            }),
            "expected nested if/continue to keep CPython-style false-edge jump-back tails, got ops={ops:?}"
        );
    }

    #[test]
    fn test_loop_assert_keeps_false_edge_into_raise_body() {
        let code = compile_exec(
            "\
def f(bytecode):
    for instr, positions in zip(bytecode, bytecode.codeobj.co_positions()):
        assert instr.positions == positions
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(6).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::CompareOp { .. },
                        Instruction::PopJumpIfFalse { .. },
                        Instruction::NotTaken,
                        Instruction::JumpBackward { .. }
                            | Instruction::JumpBackwardNoInterrupt { .. },
                        Instruction::LoadCommonConstant { .. },
                        Instruction::RaiseVarargs { .. },
                    ]
                )
            }),
            "expected loop assert to keep CPython-style false-edge into the raise body, got ops={ops:?}"
        );
    }

    #[test]
    fn test_and_is_not_none_loop_guard_uses_direct_jump_back_false_path() {
        let code = compile_exec(
            "\
def f(code):
    last_line = -2
    for _, _, line in code.co_lines():
        if line is not None and line != last_line:
            last_line = line
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(6).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::LoadFastBorrow { .. } | Instruction::LoadFast { .. },
                        Instruction::PopJumpIfNotNone { .. },
                        Instruction::NotTaken,
                        Instruction::JumpBackward { .. }
                            | Instruction::JumpBackwardNoInterrupt { .. },
                        Instruction::LoadFastBorrowLoadFastBorrow { .. }
                            | Instruction::LoadFastLoadFast { .. },
                        Instruction::CompareOp { .. },
                    ]
                )
            }),
            "expected CPython-style direct jump-back false path for 'is not None and ...', got ops={ops:?}"
        );
    }

    #[test]
    fn test_large_is_not_none_loop_guard_uses_direct_jump_back_false_path() {
        let code = compile_exec(
            "\
def f(cls, _FIELDS, _PARAMS):
    all_frozen_bases = None
    any_frozen_base = False
    has_dataclass_bases = False
    for b in cls.__mro__[-1:0:-1]:
        base_fields = getattr(b, _FIELDS, None)
        if base_fields is not None:
            has_dataclass_bases = True
            for field in base_fields.values():
                name = field.name
            if all_frozen_bases is None:
                all_frozen_bases = True
            current_frozen = getattr(b, _PARAMS).frozen
            all_frozen_bases = all_frozen_bases and current_frozen
            any_frozen_base = any_frozen_base or current_frozen
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(6).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::LoadFastBorrow { .. } | Instruction::LoadFast { .. },
                        Instruction::PopJumpIfNotNone { .. },
                        Instruction::NotTaken,
                        Instruction::JumpBackward { .. }
                            | Instruction::JumpBackwardNoInterrupt { .. },
                        Instruction::LoadConst { .. },
                        Instruction::StoreFast { .. },
                    ]
                )
            }),
            "expected CPython-style direct jump-back false path for large 'is not None' loop body, got ops={ops:?}"
        );
    }

    #[test]
    fn test_continue_inside_with_keeps_line_marker_nop_before_exit_cleanup() {
        let code = compile_exec(
            "\
def f(it):
    for func in it:
        with cm():
            if cond():
                continue
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(9).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::PopJumpIfFalse { .. },
                        Instruction::NotTaken,
                        Instruction::Nop,
                        Instruction::LoadConst { .. },
                        Instruction::LoadConst { .. },
                        Instruction::LoadConst { .. },
                        Instruction::Call { .. },
                        Instruction::PopTop,
                        Instruction::JumpBackward { .. }
                            | Instruction::JumpBackwardNoInterrupt { .. },
                    ]
                )
            }),
            "expected CPython-style line-marker NOP before with-exit cleanup on continue, got ops={ops:?}"
        );
    }

    #[test]
    fn test_nested_async_with_normal_cleanup_drops_pop_block_nop() {
        let code = compile_exec(
            "\
async def foo():
    async with CM():
        async with CM():
            raise RuntimeError
",
        );
        let f = find_code(&code, "foo").expect("missing foo code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(5).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::LoadConst { .. },
                        Instruction::LoadConst { .. },
                        Instruction::LoadConst { .. },
                        Instruction::Call { .. },
                        Instruction::GetAwaitable { .. },
                    ]
                )
            }),
            "expected CPython-style async-with normal cleanup without a POP_BLOCK NOP, got ops={ops:?}"
        );
        assert!(
            !ops.windows(6).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::Nop,
                        Instruction::LoadConst { .. },
                        Instruction::LoadConst { .. },
                        Instruction::LoadConst { .. },
                        Instruction::Call { .. },
                        Instruction::GetAwaitable { .. },
                    ]
                )
            }),
            "unexpected POP_BLOCK NOP before async-with normal cleanup, got ops={ops:?}"
        );
    }

    #[test]
    fn test_nested_terminal_with_keeps_outer_cleanup_target_nop() {
        let code = compile_exec(
            "\
def f():
    with a():
        with b():
            raise E()
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(6).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::Copy { .. },
                        Instruction::PopExcept,
                        Instruction::Reraise { .. },
                        Instruction::Nop,
                        Instruction::LoadConst { .. },
                        Instruction::LoadConst { .. },
                    ]
                )
            }),
            "expected CPython-style outer with-exit target NOP after terminal nested with cleanup, got ops={ops:?}"
        );
    }

    #[test]
    fn test_nested_nonterminal_with_drops_outer_cleanup_target_nop() {
        let code = compile_exec(
            "\
def f():
    with a():
        with b():
            x()
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            !ops.windows(6).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::Copy { .. },
                        Instruction::PopExcept,
                        Instruction::Reraise { .. },
                        Instruction::Nop,
                        Instruction::LoadConst { .. },
                        Instruction::LoadConst { .. },
                    ]
                )
            }),
            "unexpected outer with-exit target NOP for nested with with normal fallthrough, got ops={ops:?}"
        );
    }

    #[test]
    fn test_try_loop_elif_places_return_before_orelse_tail() {
        let code = compile_exec(
            "\
def f(source, suggest, tb, s):
    if source is not None:
        try:
            tb = tb
        except Exception:
            suggest = False
            tb = None
        if tb is not None:
            for frame in tb:
                s += frame
        elif suggest:
            s += 'x'
    return s
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        let has_direct_return = ops.windows(8).any(|window| {
            matches!(
                window,
                [
                    Instruction::EndFor,
                    Instruction::PopIter,
                    Instruction::LoadFastBorrow { .. } | Instruction::LoadFast { .. },
                    Instruction::ReturnValue,
                    Instruction::LoadFastBorrow { .. } | Instruction::LoadFast { .. },
                    Instruction::ToBool,
                    Instruction::PopJumpIfFalse { .. },
                    Instruction::NotTaken,
                ]
            )
        });
        let has_nop_anchored_return = ops.windows(9).any(|window| {
            matches!(
                window,
                [
                    Instruction::EndFor,
                    Instruction::PopIter,
                    Instruction::Nop,
                    Instruction::LoadFastBorrow { .. } | Instruction::LoadFast { .. },
                    Instruction::ReturnValue,
                    Instruction::LoadFastBorrow { .. } | Instruction::LoadFast { .. },
                    Instruction::ToBool,
                    Instruction::PopJumpIfFalse { .. },
                    Instruction::NotTaken,
                ]
            )
        });
        assert!(
            has_direct_return || has_nop_anchored_return,
            "expected CPython-style duplicated return between loop exit and elif tail, got ops={ops:?}"
        );
    }

    #[test]
    fn test_constant_false_while_else_deopts_post_else_borrows() {
        let code = compile_exec(
            "\
def f(self):
    x = 0
    while 0:
        x = 1
    else:
        x = 2
    self.assertEqual(x, 2)
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();
        let assert_idx = ops
            .iter()
            .position(|op| matches!(op, Instruction::LoadAttr { .. }))
            .expect("missing assertEqual call");
        let window = &ops[assert_idx.saturating_sub(1)..(assert_idx + 3).min(ops.len())];
        assert!(
            matches!(
                window,
                [
                    Instruction::LoadFast { .. },
                    Instruction::LoadAttr { .. },
                    Instruction::LoadFast { .. },
                    ..
                ]
            ),
            "expected post-else assertEqual call to use plain LOAD_FAST, got ops={window:?}"
        );
    }

    #[test]
    fn test_single_unpack_assignment_disables_constant_collection_folding() {
        let code = compile_exec("a, b, c = 1, 2, 3\n");

        assert!(
            !code.instructions.iter().any(|unit| {
                matches!(unit.op, Instruction::UnpackSequence { .. })
                    || matches!(unit.op, Instruction::LoadConst { .. })
                        && matches!(
                            code.constants.get(usize::from(u8::from(unit.arg))),
                            Some(ConstantData::Tuple { .. })
                        )
            }),
            "single unpack assignment should keep builder form for later lowering, got ops={:?}",
            code.instructions
                .iter()
                .map(|unit| unit.op)
                .collect::<Vec<_>>()
        );
        assert!(
            code.instructions
                .iter()
                .filter(|unit| matches!(unit.op, Instruction::LoadSmallInt { .. }))
                .count()
                >= 3,
            "expected individual constant loads before unpack-target stores, got ops={:?}",
            code.instructions
                .iter()
                .map(|unit| unit.op)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_chained_unpack_assignment_keeps_constant_collection_folding() {
        let code = compile_exec("(a, b) = c = d = (1, 2)\n");

        assert!(
            code.instructions
                .iter()
                .any(|unit| matches!(unit.op, Instruction::LoadConst { .. })),
            "chained unpack assignment should keep tuple constant, got ops={:?}",
            code.instructions
                .iter()
                .map(|unit| unit.op)
                .collect::<Vec<_>>()
        );
        assert!(
            code.instructions
                .iter()
                .any(|unit| matches!(unit.op, Instruction::UnpackSequence { .. })),
            "chained unpack assignment should still unpack the copied tuple, got ops={:?}",
            code.instructions
                .iter()
                .map(|unit| unit.op)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_constant_true_assert_skips_message_nested_scope() {
        let code = compile_exec("assert 1, (lambda x: x + 1)\n");

        assert_eq!(
            code.constants
                .iter()
                .filter(|constant| matches!(constant, ConstantData::Code { .. }))
                .count(),
            0,
            "constant-true assert should not compile the skipped message lambda"
        );
        assert!(
            !code
                .instructions
                .iter()
                .any(|unit| matches!(unit.op, Instruction::RaiseVarargs { .. })),
            "constant-true assert should be elided, got ops={:?}",
            code.instructions
                .iter()
                .map(|unit| unit.op)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_constant_false_assert_uses_direct_raise_shape() {
        let code = compile_exec("assert 0, (lambda x: x + 1)\n");

        assert!(
            !code.instructions.iter().any(|unit| {
                matches!(
                    unit.op,
                    Instruction::ToBool
                        | Instruction::PopJumpIfTrue { .. }
                        | Instruction::PopJumpIfFalse { .. }
                )
            }),
            "constant-false assert should use direct raise shape, got ops={:?}",
            code.instructions
                .iter()
                .map(|unit| unit.op)
                .collect::<Vec<_>>()
        );
        assert!(
            code.instructions
                .iter()
                .any(|unit| matches!(unit.op, Instruction::RaiseVarargs { .. })),
            "constant-false assert should still raise, got ops={:?}",
            code.instructions
                .iter()
                .map(|unit| unit.op)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            code.constants
                .iter()
                .filter(|constant| matches!(constant, ConstantData::Code { .. }))
                .count(),
            1,
            "constant-false assert should still compile the message lambda"
        );
    }

    #[test]
    fn test_constant_unary_positive_and_invert_fold() {
        let code = compile_exec("x = +1\nx = ~1\n");

        assert!(
            !code.instructions.iter().any(|unit| {
                matches!(
                    unit.op,
                    Instruction::CallIntrinsic1 { .. } | Instruction::UnaryInvert
                )
            }),
            "constant unary ops should fold away, got ops={:?}",
            code.instructions
                .iter()
                .map(|unit| unit.op)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_bool_invert_is_not_const_folded() {
        let code = compile_exec("x = ~True\n");

        assert!(
            code.instructions
                .iter()
                .any(|unit| matches!(unit.op, Instruction::UnaryInvert)),
            "~bool should remain unfurled to match CPython, got ops={:?}",
            code.instructions
                .iter()
                .map(|unit| unit.op)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_optimized_assert_preserves_nested_scope_order() {
        compile_exec_optimized(
            "\
class S:
    def f(self, sequence):
        _formats = [self._types_mapping[type(item)] for item in sequence]
        _list_len = len(_formats)
        assert sum(len(fmt) <= 8 for fmt in _formats) == _list_len
        _recreation_codes = [self._extract_recreation_code(item) for item in sequence]
",
        );
    }

    #[test]
    fn test_optimized_assert_with_nested_scope_in_first_iter() {
        // First iterator of a comprehension is evaluated in the enclosing
        // scope, so nested scopes inside it (the generator here) must also
        // be consumed when the assert is optimized away.
        compile_exec_optimized(
            "\
def f(items):
    assert [x for x in (y for y in items)]
    return [x for x in items]
",
        );
    }

    #[test]
    fn test_optimized_assert_with_lambda_defaults() {
        // Lambda default values are evaluated in the enclosing scope,
        // so nested scopes inside defaults must be consumed.
        compile_exec_optimized(
            "\
def f(items):
    assert (lambda x=[i for i in items]: x)()
    return [x for x in items]
",
        );
    }

    #[test]
    fn test_try_else_nested_scopes_keep_subtable_cursor_aligned() {
        let code = compile_exec(
            "\
try:
    import missing_mod
except ImportError:
    def fallback():
        return 0
else:
    def impl():
        return reversed('abc')
",
        );

        assert!(
            find_code(&code, "fallback").is_some(),
            "missing fallback code"
        );
        let impl_code = find_code(&code, "impl").expect("missing impl code");
        assert!(
            impl_code.instructions.iter().any(|unit| {
                matches!(
                    unit.op,
                    Instruction::LoadGlobal { .. } | Instruction::LoadName { .. }
                )
            }),
            "expected impl to compile global name access, got ops={:?}",
            impl_code
                .instructions
                .iter()
                .map(|unit| unit.op)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_nested_try_else_multi_resume_join_keeps_strong_load_fast_tail() {
        let code = compile_exec(
            "\
def f(msg):
    s = ''
    try:
        import a
    except Exception:
        suggest = False
        tb = None
    else:
        try:
            suggest = not t()
            tb = g(msg)
        except Exception:
            suggest = False
            tb = None
    if tb is not None:
        for frame in tb:
            s += frame
    elif suggest:
        s += 'y'
    return s
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        let tail_start = ops
            .iter()
            .position(|op| matches!(op, Instruction::PopJumpIfNone { .. }))
            .expect("missing tail POP_JUMP_IF_NONE")
            .saturating_sub(1);
        let handler_start = ops
            .iter()
            .position(|op| matches!(op, Instruction::PushExcInfo))
            .expect("missing handler entry");
        let tail = &ops[tail_start..handler_start];

        assert!(
            !tail.iter().any(|op| {
                matches!(
                    op,
                    Instruction::LoadFastBorrow { .. }
                        | Instruction::LoadFastBorrowLoadFastBorrow { .. }
                )
            }),
            "expected nested try/except else-resume tail to keep strong LOAD_FAST ops, got tail={tail:?}"
        );

        assert!(
            tail.iter()
                .any(|op| matches!(op, Instruction::LoadFastLoadFast { .. })),
            "expected loop body to keep LOAD_FAST_LOAD_FAST in the resume tail, got tail={tail:?}"
        );
    }

    #[test]
    fn test_protected_conditional_tail_keeps_strong_load_fast() {
        let code = compile_exec(
            "\
def f(m, class_name, category, warning_base):
    try:
        cat = getattr(m, class_name)
    except AttributeError:
        raise ValueError(category)
    if not issubclass(cat, warning_base):
        raise TypeError(category)
    return cat
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        let tail_start = ops
            .iter()
            .position(|op| matches!(op, Instruction::StoreFast { .. }))
            .expect("missing STORE_FAST cat");
        let handler_start = ops
            .iter()
            .position(|op| matches!(op, Instruction::PushExcInfo))
            .expect("missing handler entry");
        let tail = &ops[tail_start + 1..handler_start];

        assert!(
            !tail.iter().any(|op| {
                matches!(
                    op,
                    Instruction::LoadFastBorrow { .. }
                        | Instruction::LoadFastBorrowLoadFastBorrow { .. }
                )
            }),
            "expected protected conditional tail to keep strong LOAD_FAST ops, got tail={tail:?}"
        );

        assert!(
            tail.iter()
                .any(|op| matches!(op, Instruction::LoadFastLoadFast { .. })),
            "expected protected tail to keep LOAD_FAST_LOAD_FAST for issubclass args, got tail={tail:?}"
        );
    }

    #[test]
    fn test_nonresuming_protected_conditional_tail_keeps_strong_load_fast() {
        let code = compile_exec(
            "\
def f(href, parse='xml'):
    try:
        data = XINCLUDE[href]
    except KeyError:
        raise OSError('resource not found')
    if parse == 'xml':
        data = ET.XML(data)
    return data
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        let tail_start = ops
            .iter()
            .position(|op| matches!(op, Instruction::StoreFast { .. }))
            .expect("missing protected STORE_FAST data");
        let handler_start = ops
            .iter()
            .position(|op| matches!(op, Instruction::PushExcInfo))
            .expect("missing handler entry");
        let tail = &ops[tail_start + 1..handler_start];

        assert!(
            !tail.iter().any(|op| {
                matches!(
                    op,
                    Instruction::LoadFastBorrow { .. }
                        | Instruction::LoadFastBorrowLoadFastBorrow { .. }
                )
            }),
            "expected non-resuming protected conditional tail to keep strong LOAD_FAST ops, got tail={tail:?}"
        );
    }

    #[test]
    fn test_optional_nonresuming_protected_tail_keeps_borrow() {
        let code = compile_exec(
            "\
def f(b):
    if type(b) is not bytes:
        try:
            b = bytes(memoryview(b))
        except TypeError:
            raise TypeError(f'bad {type(b).__name__}') from None
    if b:
        sink(b)
    return len(b)
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let instructions: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();
        let b_index = f
            .varnames
            .iter()
            .position(|name| name.as_str() == "b")
            .expect("missing b varname");
        let store_b = instructions
            .iter()
            .position(|unit| match unit.op {
                Instruction::StoreFast { var_num } => {
                    usize::from(var_num.get(OpArg::new(u32::from(u8::from(unit.arg))))) == b_index
                }
                _ => false,
            })
            .expect("missing protected STORE_FAST b");
        let handler_start = instructions
            .iter()
            .position(|unit| matches!(unit.op, Instruction::PushExcInfo))
            .expect("missing handler entry");
        let tail = &instructions[store_b + 1..handler_start];

        assert!(
            tail.iter()
                .filter(|unit| match unit.op {
                    Instruction::LoadFast { var_num } | Instruction::LoadFastBorrow { var_num } => {
                        usize::from(var_num.get(OpArg::new(u32::from(u8::from(unit.arg)))))
                            == b_index
                    }
                    _ => false,
                })
                .all(|unit| matches!(unit.op, Instruction::LoadFastBorrow { .. })),
            "optional protected tail should keep CPython-style borrowed b loads, got tail={tail:?}"
        );
    }

    #[test]
    fn test_handled_except_conditional_tail_keeps_borrow() {
        let code = compile_exec(
            "\
def f(self):
    try:
        if self.active:
            self.step()
        if self.waiter is not None and self.pending is None:
            self.waiter.set_result(None)
    except ConnectionResetError as exc:
        self.close(exc)
    except OSError as exc:
        self.fail(exc, 'x')
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let instructions: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();
        let waiter_idx = instructions
            .iter()
            .position(|unit| match unit.op {
                Instruction::LoadAttr { namei } => {
                    let load_attr = namei.get(OpArg::new(u32::from(u8::from(unit.arg))));
                    f.names[usize::try_from(load_attr.name_idx()).unwrap()].as_str() == "waiter"
                }
                _ => false,
            })
            .expect("missing waiter LOAD_ATTR");
        let handler_start = instructions
            .iter()
            .position(|unit| matches!(unit.op, Instruction::PushExcInfo))
            .expect("missing handler entry");
        let tail = &instructions[waiter_idx.saturating_sub(1)..handler_start];

        assert!(
            tail.iter().any(|unit| {
                matches!(
                    unit.op,
                    Instruction::LoadFastBorrow { .. }
                        | Instruction::LoadFastBorrowLoadFastBorrow { .. }
                )
            }),
            "handled-except conditional tail should keep borrowed loads, got tail={tail:?}"
        );
        assert!(
            !tail
                .iter()
                .any(|unit| matches!(unit.op, Instruction::LoadFast { .. })),
            "handled-except conditional tail should not force strong LOAD_FAST, got tail={tail:?}"
        );
    }

    #[test]
    fn test_handled_except_else_tail_keeps_borrow() {
        let code = compile_exec(
            "\
def f(self, fut=None):
    try:
        if self.closed:
            return
        item = self.queue.popleft()
        self.size -= len(item)
        if self.addr is not None:
            self.future = self.loop.send(self.sock, item)
        else:
            self.future = self.loop.sendto(self.sock, item, addr=item)
    except OSError as exc:
        self.protocol.error_received(exc)
    except Exception as exc:
        self.fatal(exc, 'x')
    else:
        self.future.add_done_callback(self.loop_writing)
        self.resume()
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let instructions: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();
        let done_callback_idx = instructions
            .iter()
            .position(|unit| match unit.op {
                Instruction::LoadAttr { namei } => {
                    let load_attr = namei.get(OpArg::new(u32::from(u8::from(unit.arg))));
                    f.names[usize::try_from(load_attr.name_idx()).unwrap()].as_str()
                        == "add_done_callback"
                }
                _ => false,
            })
            .expect("missing add_done_callback LOAD_ATTR");
        let handler_start = instructions
            .iter()
            .position(|unit| matches!(unit.op, Instruction::PushExcInfo))
            .expect("missing handler entry");
        let tail = &instructions[done_callback_idx.saturating_sub(3)..handler_start];

        assert!(
            tail.iter().any(|unit| {
                matches!(
                    unit.op,
                    Instruction::LoadFastBorrow { .. }
                        | Instruction::LoadFastBorrowLoadFastBorrow { .. }
                )
            }),
            "handled-except else tail should keep borrowed loads, got tail={tail:?}"
        );
        assert!(
            !tail
                .iter()
                .any(|unit| matches!(unit.op, Instruction::LoadFast { .. })),
            "handled-except else tail should not force strong LOAD_FAST, got tail={tail:?}"
        );
    }

    #[test]
    fn test_reraising_handler_with_handled_returns_keeps_borrow() {
        let code = compile_exec(
            "\
def f(self, fut=None):
    try:
        if fut is not None:
            fut.result()
        if self.future is not fut:
            return
        fut = self.reader.recv(self.sock, 4096)
    except CancelledError:
        return
    except (SystemExit, KeyboardInterrupt):
        raise
    except BaseException as exc:
        self.handle({'exception': exc, 'loop': self})
    else:
        self.future = fut
        fut.add_done_callback(self.loop_reading)
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let instructions: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();
        let recv_idx = instructions
            .iter()
            .position(|unit| match unit.op {
                Instruction::LoadAttr { namei } => {
                    let load_attr = namei.get(OpArg::new(u32::from(u8::from(unit.arg))));
                    f.names[usize::try_from(load_attr.name_idx()).unwrap()].as_str() == "recv"
                }
                _ => false,
            })
            .expect("missing recv LOAD_ATTR");
        let done_callback_idx = instructions
            .iter()
            .position(|unit| match unit.op {
                Instruction::LoadAttr { namei } => {
                    let load_attr = namei.get(OpArg::new(u32::from(u8::from(unit.arg))));
                    f.names[usize::try_from(load_attr.name_idx()).unwrap()].as_str()
                        == "add_done_callback"
                }
                _ => false,
            })
            .expect("missing add_done_callback LOAD_ATTR");
        let handler_start = instructions
            .iter()
            .position(|unit| matches!(unit.op, Instruction::PushExcInfo))
            .expect("missing handler entry");
        let tail =
            &instructions[recv_idx.saturating_sub(3)..handler_start.min(done_callback_idx + 3)];

        assert!(
            tail.iter().any(|unit| {
                matches!(
                    unit.op,
                    Instruction::LoadFastBorrow { .. }
                        | Instruction::LoadFastBorrowLoadFastBorrow { .. }
                )
            }),
            "handler chain with handled returns should keep borrowed warm/else loads, got tail={tail:?}"
        );
        assert!(
            !tail.iter().any(|unit| {
                matches!(
                    unit.op,
                    Instruction::LoadFast { .. } | Instruction::LoadFastLoadFast { .. }
                )
            }),
            "handler chain with handled returns should not force strong warm/else loads, got tail={tail:?}"
        );
    }

    #[test]
    fn test_with_protected_conditional_tail_without_exception_match_keeps_borrow() {
        let code = compile_exec(
            "\
def f(self, cm, p, platform):
    with cm:
        if p.returncode != 0:
            if platform.machine() == 'x86_64':
                p.check_returncode()
            else:
                self.skipTest(f'could not compile indirect function: {p}')
        done()
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let instructions: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();

        let attr_load_uses_borrow = |name: &str| {
            let attr_idx = instructions
                .iter()
                .position(|unit| match unit.op {
                    Instruction::LoadAttr { namei } => {
                        let load_attr = namei.get(OpArg::new(u32::from(u8::from(unit.arg))));
                        f.names[usize::try_from(load_attr.name_idx()).unwrap()].as_str() == name
                    }
                    _ => false,
                })
                .unwrap_or_else(|| panic!("missing {name} attr load"));
            matches!(
                instructions
                    .get(attr_idx.saturating_sub(1))
                    .map(|unit| unit.op),
                Some(Instruction::LoadFastBorrow { .. })
            )
        };

        assert!(
            attr_load_uses_borrow("check_returncode"),
            "plain with-protected conditional tail should keep borrowed p load, got instructions={instructions:?}"
        );
        assert!(
            attr_load_uses_borrow("skipTest"),
            "plain with-protected conditional tail should keep borrowed self load, got instructions={instructions:?}"
        );
    }

    #[test]
    fn test_listcomp_cleanup_predecessor_does_not_deopt_following_conditional_tail() {
        let code = compile_exec(
            "\
def f(self, compile_snippet):
    sizes = [compile_snippet(i).co_stacksize for i in range(2, 5)]
    if len(set(sizes)) != 1:
        import dis, io
        out = io.StringIO()
        dis.dis(compile_snippet(1), file=out)
        self.fail('%s\\n%s' % (sizes, out.getvalue()))
",
        );
        let f = find_code(&code, "f").expect("missing f code");

        let has_strong_load = |name: &str| {
            f.instructions.iter().any(|unit| match unit.op {
                Instruction::LoadFast { var_num } => {
                    let arg = OpArg::new(u32::from(u8::from(unit.arg)));
                    f.varnames[usize::from(var_num.get(arg))] == name
                }
                _ => false,
            })
        };
        let has_borrow_load = |name: &str| {
            f.instructions.iter().any(|unit| match unit.op {
                Instruction::LoadFastBorrow { var_num } => {
                    let arg = OpArg::new(u32::from(u8::from(unit.arg)));
                    f.varnames[usize::from(var_num.get(arg))] == name
                }
                _ => false,
            })
        };

        for name in ["sizes", "io", "dis", "compile_snippet", "out", "self"] {
            assert!(
                has_borrow_load(name),
                "expected listcomp-following conditional tail to borrow {name}, got instructions={:?}",
                f.instructions
            );
        }
        for name in ["sizes", "io", "dis", "compile_snippet", "out", "self"] {
            assert!(
                !has_strong_load(name),
                "listcomp cleanup predecessor should not force strong LOAD_FAST for {name}, got instructions={:?}",
                f.instructions
            );
        }
    }

    #[test]
    fn test_handler_resume_loop_conditional_tail_keeps_strong_load_fast() {
        let code = compile_exec(
            "\
def f(self):
    is_utf8 = (self.ENCODING == 'utf-8')
    encode_errors = 'surrogateescape' if is_utf8 else 'strict'
    strings = list(self.BYTES_STRINGS)
    for text in self.STRINGS:
        try:
            encoded = text.encode(self.ENCODING, encode_errors)
            if encoded not in strings:
                strings.append(encoded)
        except UnicodeEncodeError:
            encoded = None
        if is_utf8:
            encoded2 = text.encode(self.ENCODING, 'surrogatepass')
            if encoded2 != encoded:
                strings.append(encoded2)
    for encoded in strings:
        self.consume(encoded)
",
        );
        let f = find_code(&code, "f").expect("missing f code");

        let has_strong_load = |name: &str| {
            f.instructions.iter().any(|unit| match unit.op {
                Instruction::LoadFast { var_num } => {
                    let arg = OpArg::new(u32::from(u8::from(unit.arg)));
                    f.varnames[usize::from(var_num.get(arg))] == name
                }
                _ => false,
            })
        };
        let has_borrow_load = |name: &str| {
            f.instructions.iter().any(|unit| match unit.op {
                Instruction::LoadFastBorrow { var_num } => {
                    let arg = OpArg::new(u32::from(u8::from(unit.arg)));
                    f.varnames[usize::from(var_num.get(arg))] == name
                }
                _ => false,
            })
        };

        for name in ["is_utf8", "text", "self", "strings", "encoded2"] {
            assert!(
                has_strong_load(name),
                "expected handler-resume loop tail to use strong LOAD_FAST for {name}, got instructions={:?}",
                f.instructions
            );
        }
        assert!(
            f.instructions.iter().any(|unit| match unit.op {
                Instruction::LoadFastLoadFast { var_nums } => {
                    let arg = OpArg::new(u32::from(u8::from(unit.arg)));
                    let (left, right) = var_nums.get(arg).indexes();
                    f.varnames[usize::from(left)] == "encoded2"
                        && f.varnames[usize::from(right)] == "encoded"
                }
                _ => false,
            }),
            "expected encoded2/encoded comparison to use strong LOAD_FAST_LOAD_FAST, got instructions={:?}",
            f.instructions
        );
        assert!(
            !f.instructions.iter().any(|unit| match unit.op {
                Instruction::LoadFastBorrowLoadFastBorrow { var_nums } => {
                    let arg = OpArg::new(u32::from(u8::from(unit.arg)));
                    let (left, right) = var_nums.get(arg).indexes();
                    f.varnames[usize::from(left)] == "encoded2"
                        && f.varnames[usize::from(right)] == "encoded"
                }
                _ => false,
            }),
            "handler-resume loop tail should not borrow encoded2/encoded comparison, got instructions={:?}",
            f.instructions
        );
        assert!(
            has_borrow_load("strings"),
            "expected later loop/list uses outside the deopt tail to keep borrowing strings"
        );
    }

    #[test]
    fn test_async_early_return_send_tail_uses_strong_load_fast_after_entry() {
        let code = compile_exec(
            "\
class C:
    async def _sock_sendfile_native(self, sock, file, offset, count):
        try:
            fileno = file.fileno()
        except (AttributeError, io.UnsupportedOperation) as err:
            raise exceptions.SendfileNotAvailableError('not a regular file')
        try:
            fsize = os.fstat(fileno).st_size
        except OSError:
            raise exceptions.SendfileNotAvailableError('not a regular file')
        blocksize = count if count else fsize
        if not blocksize:
            return 0
        blocksize = min(blocksize, 0xffff_ffff)
        end_pos = min(offset + count, fsize) if count else fsize
        offset = min(offset, fsize)
        total_sent = 0
        try:
            while True:
                blocksize = min(end_pos - offset, blocksize)
                if blocksize <= 0:
                    return total_sent
                await self._proactor.sendfile(sock, file, offset, blocksize)
                offset += blocksize
                total_sent += blocksize
        finally:
            if total_sent > 0:
                file.seek(offset)
",
        );
        let f = find_code(&code, "_sock_sendfile_native").expect("missing method code");

        let names_for_unit = |unit: &bytecode::CodeUnit| -> Vec<String> {
            let arg = OpArg::new(u32::from(u8::from(unit.arg)));
            match unit.op {
                Instruction::LoadFast { var_num } | Instruction::LoadFastBorrow { var_num } => {
                    vec![f.varnames[usize::from(var_num.get(arg))].to_string()]
                }
                Instruction::LoadFastLoadFast { var_nums }
                | Instruction::LoadFastBorrowLoadFastBorrow { var_nums } => {
                    let (left, right) = var_nums.get(arg).indexes();
                    vec![
                        f.varnames[usize::from(left)].to_string(),
                        f.varnames[usize::from(right)].to_string(),
                    ]
                }
                _ => Vec::new(),
            }
        };

        let borrowed_names: Vec<_> = f
            .instructions
            .iter()
            .filter_map(|unit| match unit.op {
                Instruction::LoadFastBorrow { .. }
                | Instruction::LoadFastBorrowLoadFastBorrow { .. } => Some(names_for_unit(unit)),
                _ => None,
            })
            .collect();
        assert_eq!(
            borrowed_names,
            vec![vec!["file".to_owned()]],
            "only the initial file.fileno() receiver should borrow, got instructions={:?}",
            f.instructions
        );

        let has_strong_name = |name: &str| {
            f.instructions.iter().any(|unit| {
                matches!(
                    unit.op,
                    Instruction::LoadFast { .. } | Instruction::LoadFastLoadFast { .. }
                ) && names_for_unit(unit).iter().any(|loaded| loaded == name)
            })
        };

        for name in [
            "fileno",
            "count",
            "blocksize",
            "offset",
            "fsize",
            "end_pos",
            "total_sent",
            "file",
            "self",
            "sock",
        ] {
            assert!(
                has_strong_name(name),
                "async early-return send tail should use strong LOAD_FAST for {name}, got instructions={:?}",
                f.instructions
            );
        }
    }

    #[test]
    fn test_protected_import_tail_keeps_strong_load_fast() {
        let code = compile_exec(
            "\
def f(s, size, pos, errors):
    message = 'x'
    look = pos
    try:
        import unicodedata
    except ImportError:
        return None
    if look < size and chr(s[look]) == '{':
        while look < size and chr(s[look]) != '}':
            look += 1
        if look > pos + 1 and look < size and chr(s[look]) == '}':
            message = 'y'
    return message
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        let import_idx = ops
            .iter()
            .position(|op| matches!(op, Instruction::ImportName { .. }))
            .expect("missing IMPORT_NAME");
        let protected_tail = &ops[import_idx + 1..];

        assert!(
            !protected_tail.iter().any(|op| {
                matches!(
                    op,
                    Instruction::LoadFastBorrow { .. }
                        | Instruction::LoadFastBorrowLoadFastBorrow { .. }
                )
            }),
            "expected protected import tail to keep strong LOAD_FAST ops, got tail={protected_tail:?}"
        );

        assert!(
            protected_tail
                .iter()
                .any(|op| matches!(op, Instruction::LoadFastLoadFast { .. })),
            "expected protected import tail to keep LOAD_FAST_LOAD_FAST ops, got tail={protected_tail:?}"
        );
    }

    #[test]
    fn test_unprotected_import_before_with_keeps_borrow() {
        let code = compile_exec(
            "\
def f(self, document):
    from xml.etree import ElementInclude
    document = self.xinclude_loader('C1.xml')
    with self.assertRaises(OSError) as cm:
        ElementInclude.include(document, self.xinclude_loader)
    self.assertEqual(str(cm.exception), 'resource not found')
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        let import_idx = ops
            .iter()
            .position(|op| matches!(op, Instruction::ImportName { .. }))
            .expect("missing IMPORT_NAME");
        let handler_start = ops
            .iter()
            .position(|op| matches!(op, Instruction::PushExcInfo))
            .expect("missing handler entry");
        let warm_path = &ops[import_idx + 1..handler_start];

        assert!(
            warm_path.iter().any(|op| {
                matches!(
                    op,
                    Instruction::LoadFastBorrow { .. }
                        | Instruction::LoadFastBorrowLoadFastBorrow { .. }
                )
            }),
            "expected unprotected import before with-block to keep LOAD_FAST_BORROW ops, got warm_path={warm_path:?}"
        );
        assert!(
            warm_path
                .iter()
                .any(|op| matches!(op, Instruction::LoadFastBorrowLoadFastBorrow { .. })),
            "expected with body arguments to keep LOAD_FAST_BORROW_LOAD_FAST_BORROW, got warm_path={warm_path:?}"
        );
    }

    #[test]
    fn test_unprotected_prefix_before_try_keeps_attr_subscript_borrow() {
        let code = compile_exec(
            "\
def f():
    import sys, getopt
    usage = f'usage: {sys.argv[0]}'
    try:
        opts, args = getopt.getopt(sys.argv[1:], 'h')
    except getopt.error as msg:
        sys.stdout = sys.stderr
        print(msg)
        print(usage)
        sys.exit(2)
    return usage
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let first_argv_idx = f
            .instructions
            .iter()
            .position(|unit| match unit.op {
                Instruction::LoadAttr { namei } => {
                    let load_attr = namei.get(OpArg::new(u32::from(u8::from(unit.arg))));
                    f.names[usize::try_from(load_attr.name_idx()).unwrap()].as_str() == "argv"
                }
                _ => false,
            })
            .expect("missing argv attr load");
        let receiver = f.instructions[..first_argv_idx]
            .iter()
            .rev()
            .find(|unit| !matches!(unit.op, Instruction::Cache))
            .expect("missing argv receiver")
            .op;

        assert!(
            matches!(receiver, Instruction::LoadFastBorrow { .. }),
            "unprotected prefix before try should keep CPython-style LOAD_FAST_BORROW receiver, got {receiver:?}"
        );
    }

    #[test]
    fn test_terminal_except_inlined_comprehension_keeps_borrowed_warm_loads() {
        let code = compile_exec(
            r##"
def f(output):
    output = re.sub(r"\[[0-9]+ refs\]", "", output)
    try:
        result = [
            row.split("\t")
            for row in output.splitlines()
            if row and not row.startswith('#')
        ]
        result.sort(key=lambda row: int(row[0]))
        result = [row[1] for row in result]
        return "\n".join(result)
    except (IndexError, ValueError):
        raise AssertionError(
            "tracer produced unparsable output:\n{}".format(output)
        )
"##,
        );
        let f = find_code(&code, "f").expect("missing f code");
        let handler_start = f
            .instructions
            .iter()
            .position(|unit| matches!(unit.op, Instruction::PushExcInfo))
            .expect("missing handler entry");
        let warm_path = &f.instructions[..handler_start];
        let load_fast_name = |unit: &bytecode::CodeUnit| match unit.op {
            Instruction::LoadFast { var_num } => {
                let arg = OpArg::new(u32::from(u8::from(unit.arg)));
                Some(f.varnames[usize::from(var_num.get(arg))].as_str())
            }
            _ => None,
        };
        let borrow_name = |unit: &bytecode::CodeUnit| match unit.op {
            Instruction::LoadFastBorrow { var_num } => {
                let arg = OpArg::new(u32::from(u8::from(unit.arg)));
                Some(f.varnames[usize::from(var_num.get(arg))].as_str())
            }
            _ => None,
        };

        assert!(
            warm_path
                .iter()
                .filter_map(load_fast_name)
                .all(|name| name != "row" && name != "result"),
            "terminal-except inlined comprehension warm path should keep CPython-style borrowed row/result loads, got warm_path={warm_path:?}"
        );
        for name in ["row", "result"] {
            assert!(
                warm_path
                    .iter()
                    .filter_map(borrow_name)
                    .any(|var| var == name),
                "expected borrowed {name} load in terminal-except inlined comprehension warm path, got warm_path={warm_path:?}"
            );
        }
    }

    #[test]
    fn test_outer_guarded_protected_import_keeps_borrow_tail() {
        let code = compile_exec(
            "\
def f(sys, os, file):
    if sys.platform == 'win32':
        try:
            import nt
            if not nt._supports_virtual_terminal():
                return False
        except (ImportError, AttributeError):
            return False
    try:
        return os.isatty(file.fileno())
    except OSError:
        return hasattr(file, 'isatty') and file.isatty()
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let borrows_name = |name: &str| {
            f.instructions.iter().any(|unit| match unit.op {
                Instruction::LoadFastBorrow { var_num } => {
                    let arg = OpArg::new(u32::from(u8::from(unit.arg)));
                    f.varnames[usize::from(var_num.get(arg))] == name
                }
                Instruction::LoadFastBorrowLoadFastBorrow { var_nums } => {
                    let arg = OpArg::new(u32::from(u8::from(unit.arg)));
                    let (left, right) = var_nums.get(arg).indexes();
                    f.varnames[usize::from(left)] == name || f.varnames[usize::from(right)] == name
                }
                _ => false,
            })
        };

        for name in ["nt", "os", "file"] {
            assert!(
                borrows_name(name),
                "outer-guarded protected import should keep CPython-style borrow for {name}, got instructions={:?}",
                f.instructions
            );
        }
    }

    #[test]
    fn test_loop_or_break_continue_orders_break_before_backedge() {
        let code = compile_exec(
            "\
def f(self, quoted):
    while True:
        if self.state == 'x':
            if self.token or (self.posix and quoted):
                break
            else:
                continue
        elif self.state == 'y':
            self.consume()
    x = self.a + self.b + self.c
    return x
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();
        let quoted_load = ops
            .iter()
            .position(|unit| match unit.op {
                Instruction::LoadFastBorrow { var_num } => {
                    let arg = OpArg::new(u32::from(u8::from(unit.arg)));
                    f.varnames[usize::from(var_num.get(arg))] == "quoted"
                }
                _ => false,
            })
            .expect("missing quoted LOAD_FAST_BORROW");
        let final_cond = ops[quoted_load + 1..]
            .iter()
            .position(|unit| {
                matches!(
                    unit.op,
                    Instruction::PopJumpIfFalse { .. } | Instruction::PopJumpIfTrue { .. }
                )
            })
            .map(|idx| quoted_load + 1 + idx)
            .expect("missing final conditional jump");
        assert!(
            matches!(ops[final_cond].op, Instruction::PopJumpIfFalse { .. }),
            "expected CPython-style inverted final condition, got ops={ops:?}"
        );
        let break_jump_idx = ops[final_cond + 1..]
            .iter()
            .position(|unit| matches!(unit.op, Instruction::JumpForward { .. }))
            .map(|idx| final_cond + 1 + idx)
            .expect("missing break jump after condition");
        let jump_back_idx = ops[final_cond + 1..]
            .iter()
            .position(|unit| matches!(unit.op, Instruction::JumpBackward { .. }))
            .map(|idx| final_cond + 1 + idx)
            .expect("missing continue backedge");
        assert!(
            break_jump_idx < jump_back_idx,
            "expected break jump before continue backedge, got ops={ops:?}"
        );
    }

    #[test]
    fn test_for_continue_before_return_orders_backedge_before_return_body() {
        let code = compile_exec(
            "\
def f(self):
    for version in AllowedVersions:
        if not version in self.capabilities:
            continue
        self.PROTOCOL_VERSION = version
        return
    raise self.error('x')
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();
        let contains_idx = ops
            .iter()
            .position(|unit| matches!(unit.op, Instruction::ContainsOp { .. }))
            .expect("missing containment test");
        let cond_idx = ops[contains_idx + 1..]
            .iter()
            .position(|unit| {
                matches!(
                    unit.op,
                    Instruction::PopJumpIfFalse { .. } | Instruction::PopJumpIfTrue { .. }
                )
            })
            .map(|idx| contains_idx + 1 + idx)
            .expect("missing conditional jump");
        assert!(
            matches!(ops[cond_idx].op, Instruction::PopJumpIfTrue { .. }),
            "expected CPython-style condition targeting the return body, got ops={ops:?}"
        );

        let backedge_idx = ops[cond_idx + 1..]
            .iter()
            .position(|unit| matches!(unit.op, Instruction::JumpBackward { .. }))
            .map(|idx| cond_idx + 1 + idx)
            .expect("missing continue backedge");
        let store_attr_idx = ops[cond_idx + 1..]
            .iter()
            .position(|unit| match unit.op {
                Instruction::StoreAttr { namei } => {
                    let namei = namei.get(OpArg::new(u32::from(u8::from(unit.arg))));
                    f.names[usize::try_from(namei).unwrap()].as_str() == "PROTOCOL_VERSION"
                }
                _ => false,
            })
            .map(|idx| cond_idx + 1 + idx)
            .expect("missing PROTOCOL_VERSION store");
        assert!(
            backedge_idx < store_attr_idx,
            "expected continue backedge before return body, got ops={ops:?}"
        );
    }

    #[test]
    fn test_while_conditional_return_orders_backedge_before_return_body() {
        let code = compile_exec(
            "\
def f(self, tag):
    while self._get_response():
        if self.tagged_commands[tag]:
            return tag
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();
        let subscript_idx = ops
            .iter()
            .position(|unit| matches!(unit.op, Instruction::BinaryOp { .. }))
            .expect("missing tagged_commands subscript");
        let cond_idx = ops[subscript_idx + 1..]
            .iter()
            .position(|unit| {
                matches!(
                    unit.op,
                    Instruction::PopJumpIfFalse { .. } | Instruction::PopJumpIfTrue { .. }
                )
            })
            .map(|idx| subscript_idx + 1 + idx)
            .expect("missing conditional jump");
        assert!(
            matches!(ops[cond_idx].op, Instruction::PopJumpIfTrue { .. }),
            "expected CPython-style condition targeting return body, got ops={ops:?}"
        );
        let backedge_idx = ops[cond_idx + 1..]
            .iter()
            .position(|unit| matches!(unit.op, Instruction::JumpBackward { .. }))
            .map(|idx| cond_idx + 1 + idx)
            .expect("missing loop backedge");
        let return_idx = ops[cond_idx + 1..]
            .iter()
            .position(|unit| matches!(unit.op, Instruction::ReturnValue))
            .map(|idx| cond_idx + 1 + idx)
            .expect("missing return");
        assert!(
            backedge_idx < return_idx,
            "expected loop backedge before return body, got ops={ops:?}"
        );
    }

    #[test]
    fn test_for_break_to_return_orders_backedge_before_return() {
        let code = compile_exec(
            "\
def f(it):
    best = 10
    body = None
    for prio, part in it:
        if prio < best:
            best = prio
            body = part
            if prio == 0:
                break
    return body
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();
        let compare_idx = ops
            .iter()
            .enumerate()
            .filter(|(_, unit)| matches!(unit.op, Instruction::CompareOp { .. }))
            .nth(1)
            .map(|(idx, _)| idx)
            .expect("missing break comparison");
        let cond_idx = ops[compare_idx + 1..]
            .iter()
            .position(|unit| {
                matches!(
                    unit.op,
                    Instruction::PopJumpIfFalse { .. } | Instruction::PopJumpIfTrue { .. }
                )
            })
            .map(|idx| compare_idx + 1 + idx)
            .expect("missing break conditional jump");
        assert!(
            matches!(ops[cond_idx].op, Instruction::PopJumpIfTrue { .. }),
            "expected CPython-style true jump to break return path, got ops={ops:?}"
        );
        let jump_back_idx = ops[cond_idx + 1..]
            .iter()
            .position(|unit| matches!(unit.op, Instruction::JumpBackward { .. }))
            .map(|idx| cond_idx + 1 + idx)
            .expect("missing loop backedge before break return");
        let return_idx = ops[cond_idx + 1..]
            .iter()
            .position(|unit| matches!(unit.op, Instruction::ReturnValue))
            .map(|idx| cond_idx + 1 + idx)
            .expect("missing break return path");
        assert!(
            jump_back_idx < return_idx,
            "expected loop backedge before break return block, got ops={ops:?}"
        );
    }

    #[test]
    fn test_for_conditional_raise_orders_backedge_before_raise() {
        let code = compile_exec(
            "\
def f(items, limit):
    found = 0
    for item in items:
        if item:
            found += 1
            if found >= limit:
                raise ValueError(found)
    return found
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();
        let compare_idx = ops
            .iter()
            .enumerate()
            .find(|(_, unit)| matches!(unit.op, Instruction::CompareOp { .. }))
            .map(|(idx, _)| idx)
            .expect("missing raise comparison");
        let cond_idx = ops[compare_idx + 1..]
            .iter()
            .position(|unit| {
                matches!(
                    unit.op,
                    Instruction::PopJumpIfFalse { .. } | Instruction::PopJumpIfTrue { .. }
                )
            })
            .map(|idx| compare_idx + 1 + idx)
            .expect("missing raise conditional jump");
        assert!(
            matches!(ops[cond_idx].op, Instruction::PopJumpIfTrue { .. }),
            "expected CPython-style true jump to raise path, got ops={ops:?}"
        );
        let jump_back_idx = ops[cond_idx + 1..]
            .iter()
            .position(|unit| matches!(unit.op, Instruction::JumpBackward { .. }))
            .map(|idx| cond_idx + 1 + idx)
            .expect("missing loop backedge before raise");
        let raise_idx = ops[cond_idx + 1..]
            .iter()
            .position(|unit| matches!(unit.op, Instruction::RaiseVarargs { .. }))
            .map(|idx| cond_idx + 1 + idx)
            .expect("missing raise path");
        assert!(
            jump_back_idx < raise_idx,
            "expected loop backedge before conditional raise block, got ops={ops:?}"
        );
    }

    #[test]
    fn test_simple_for_conditional_raise_orders_backedge_before_raise() {
        let code = compile_exec(
            "\
def f(kw):
    for k in ('stdout', 'check'):
        if k in kw:
            raise ValueError(f'{k} argument not allowed, it will be overridden.')
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(5).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::ContainsOp { .. },
                        Instruction::PopJumpIfTrue { .. },
                        Instruction::NotTaken,
                        Instruction::JumpBackward { .. },
                        Instruction::LoadGlobal { .. },
                    ]
                )
            }),
            "expected CPython-style true jump to raise path after loop backedge, got ops={ops:?}"
        );
        assert!(
            !ops.windows(4).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::ContainsOp { .. },
                        Instruction::PopJumpIfFalse { .. },
                        Instruction::NotTaken,
                        Instruction::LoadGlobal { .. },
                    ]
                )
            }),
            "unexpected conditional raise body before loop backedge, got ops={ops:?}"
        );
    }

    #[test]
    fn test_protected_for_is_none_raise_threads_backedge_before_raise() {
        let code = compile_exec(
            "\
def f(stacklevel, frame, skip_file_prefixes):
    try:
        for x in range(stacklevel - 1):
            frame = _next_external_frame(frame, skip_file_prefixes)
            if frame is None:
                raise ValueError
    except ValueError:
        frame = None
    return frame
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(5).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::LoadFastBorrow { .. } | Instruction::LoadFast { .. },
                        Instruction::PopJumpIfNone { .. },
                        Instruction::NotTaken,
                        Instruction::JumpBackward { .. },
                        Instruction::LoadGlobal { .. },
                    ]
                )
            }),
            "expected protected is-None raise path to match CPython's backedge-before-raise layout, got ops={ops:?}"
        );
    }

    #[test]
    fn test_exception_handler_loop_conditional_raise_orders_backedge_before_raise() {
        let code = compile_exec(
            "\
def f(chunk, dec, i):
    try:
        for c in chunk:
            acc = dec[c]
    except TypeError:
        for j, c in enumerate(chunk):
            if dec[c] is None:
                raise ValueError('%d' % (i + j)) from None
        raise
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(5).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::BinaryOp { .. },
                        Instruction::PopJumpIfNone { .. },
                        Instruction::NotTaken,
                        Instruction::JumpBackward { .. }
                            | Instruction::JumpBackwardNoInterrupt { .. },
                        Instruction::LoadGlobal { .. },
                    ]
                )
            }),
            "expected exception-handler loop false path to jump back before raise body, got ops={ops:?}"
        );
        assert!(
            !ops.windows(4).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::BinaryOp { .. },
                        Instruction::PopJumpIfNotNone { .. },
                        Instruction::NotTaken,
                        Instruction::LoadGlobal { .. },
                    ]
                )
            }),
            "unexpected exception-handler loop raise body before backedge, got ops={ops:?}"
        );
    }

    #[test]
    fn test_loop_if_body_keeps_fallthrough_before_implicit_continue_backedge() {
        let code = compile_exec(
            "\
def f(b, curr, curr_append, decoded_append, packI, curr_clear):
    for x in b:
        if 33 <= x <= 117:
            curr_append(x)
            if len(curr) == 5:
                acc = 0
                for x in curr:
                    acc = 85 * acc + (x - 33)
                decoded_append(packI(acc))
                curr_clear()
        elif x == 122:
            decoded_append(0)
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(5).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::CompareOp { .. },
                        Instruction::PopJumpIfFalse { .. },
                        Instruction::NotTaken,
                        Instruction::LoadSmallInt { .. },
                        Instruction::StoreFast { .. },
                    ]
                )
            }),
            "expected CPython-style conditional body fallthrough before implicit continue backedge, got ops={ops:?}"
        );
        assert!(
            !ops.windows(6).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::CompareOp { .. },
                        Instruction::PopJumpIfTrue { .. },
                        Instruction::NotTaken,
                        Instruction::JumpBackward { .. }
                            | Instruction::JumpBackwardNoInterrupt { .. },
                        Instruction::LoadSmallInt { .. },
                        Instruction::StoreFast { .. },
                    ]
                )
            }),
            "unexpected inverted conditional with implicit continue backedge before body, got ops={ops:?}"
        );
    }

    #[test]
    fn test_try_else_loop_if_body_keeps_cpython_fallthrough_before_backedge() {
        let code = compile_exec(
            "\
def f(self, ready, selector, key, input_view, os, BrokenPipeError):
    for key, events in ready:
        if key.fileobj is self.stdin:
            chunk = input_view[self._input_offset:self._input_offset + 1]
            try:
                self._input_offset += os.write(key.fd, chunk)
            except BrokenPipeError:
                selector.unregister(key.fileobj)
                key.fileobj.close()
            else:
                if self._input_offset >= len(input_view):
                    selector.unregister(key.fileobj)
                    key.fileobj.close()
        elif key.fileobj in (self.stdout, self.stderr):
            self.read(key)
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(5).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::CompareOp { .. },
                        Instruction::PopJumpIfFalse { .. },
                        Instruction::NotTaken,
                        Instruction::LoadFastBorrow { .. } | Instruction::LoadFast { .. },
                        Instruction::LoadAttr { .. },
                    ]
                )
            }),
            "expected CPython-style try-else if body fallthrough before loop backedge, got ops={ops:?}"
        );
        assert!(
            !ops.windows(6).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::CompareOp { .. },
                        Instruction::PopJumpIfTrue { .. },
                        Instruction::NotTaken,
                        Instruction::JumpBackward { .. }
                            | Instruction::JumpBackwardNoInterrupt { .. },
                        Instruction::LoadFastBorrow { .. } | Instruction::LoadFast { .. },
                        Instruction::LoadAttr { .. },
                    ]
                )
            }),
            "unexpected inverted try-else conditional with loop backedge before body, got ops={ops:?}"
        );
    }

    #[test]
    fn test_explicit_continue_after_return_orders_return_before_backedge() {
        let code = compile_exec(
            "\
def f(j, n):
    while j < n:
        if j < 0:
            return j
        continue
    return -1
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();
        let compare_idx = ops
            .iter()
            .enumerate()
            .filter(|(_, unit)| matches!(unit.op, Instruction::CompareOp { .. }))
            .nth(1)
            .map(|(idx, _)| idx)
            .expect("missing inner comparison");
        let cond_idx = ops[compare_idx + 1..]
            .iter()
            .position(|unit| {
                matches!(
                    unit.op,
                    Instruction::PopJumpIfFalse { .. } | Instruction::PopJumpIfTrue { .. }
                )
            })
            .map(|idx| compare_idx + 1 + idx)
            .expect("missing conditional jump");
        assert!(
            matches!(ops[cond_idx].op, Instruction::PopJumpIfFalse { .. }),
            "expected CPython-style false jump to explicit continue, got ops={ops:?}"
        );
        let return_idx = ops[cond_idx + 1..]
            .iter()
            .position(|unit| matches!(unit.op, Instruction::ReturnValue))
            .map(|idx| cond_idx + 1 + idx)
            .expect("missing return path");
        let jump_back_idx = ops[cond_idx + 1..]
            .iter()
            .position(|unit| matches!(unit.op, Instruction::JumpBackward { .. }))
            .map(|idx| cond_idx + 1 + idx)
            .expect("missing explicit continue backedge");
        assert!(
            return_idx < jump_back_idx,
            "expected return block before explicit continue backedge, got ops={ops:?}"
        );
    }

    #[test]
    fn test_implicit_while_tail_return_orders_backedge_before_return() {
        let code = compile_exec(
            "\
def f(self, j, n):
    while j < n:
        name, j = self.scan(j)
        if j < 0:
            return j
    return -1
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();
        let compare_idx = ops
            .iter()
            .enumerate()
            .filter(|(_, unit)| matches!(unit.op, Instruction::CompareOp { .. }))
            .nth(1)
            .map(|(idx, _)| idx)
            .expect("missing inner comparison");
        let cond_idx = ops[compare_idx + 1..]
            .iter()
            .position(|unit| {
                matches!(
                    unit.op,
                    Instruction::PopJumpIfFalse { .. } | Instruction::PopJumpIfTrue { .. }
                )
            })
            .map(|idx| compare_idx + 1 + idx)
            .expect("missing conditional jump");
        assert!(
            matches!(ops[cond_idx].op, Instruction::PopJumpIfTrue { .. }),
            "expected CPython-style true jump to return, got ops={ops:?}"
        );
        let jump_back_idx = ops[cond_idx + 1..]
            .iter()
            .position(|unit| matches!(unit.op, Instruction::JumpBackward { .. }))
            .map(|idx| cond_idx + 1 + idx)
            .expect("missing implicit backedge");
        let return_idx = ops[cond_idx + 1..]
            .iter()
            .position(|unit| matches!(unit.op, Instruction::ReturnValue))
            .map(|idx| cond_idx + 1 + idx)
            .expect("missing return path");
        assert!(
            jump_back_idx < return_idx,
            "expected implicit loop backedge before return block, got ops={ops:?}"
        );
    }

    #[test]
    fn test_branch_arm_implicit_continue_keeps_return_before_backedge() {
        let code = compile_exec(
            "\
def f(self, j, n, c):
    while j < n:
        if c == 'x':
            j = self.step(j)
            if j < 0:
                return j
        elif c == 'y':
            j = j + 1
    return -1
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();
        let compare_idx = ops
            .iter()
            .enumerate()
            .filter(|(_, unit)| matches!(unit.op, Instruction::CompareOp { .. }))
            .nth(2)
            .map(|(idx, _)| idx)
            .expect("missing branch-arm return comparison");
        let cond_idx = ops[compare_idx + 1..]
            .iter()
            .position(|unit| {
                matches!(
                    unit.op,
                    Instruction::PopJumpIfFalse { .. } | Instruction::PopJumpIfTrue { .. }
                )
            })
            .map(|idx| compare_idx + 1 + idx)
            .expect("missing branch-arm conditional jump");
        assert!(
            matches!(ops[cond_idx].op, Instruction::PopJumpIfFalse { .. }),
            "expected CPython-style false jump to branch-arm continuation, got ops={ops:?}"
        );
        let return_idx = ops[cond_idx + 1..]
            .iter()
            .position(|unit| matches!(unit.op, Instruction::ReturnValue))
            .map(|idx| cond_idx + 1 + idx)
            .expect("missing branch-arm return path");
        let jump_back_idx = ops[cond_idx + 1..]
            .iter()
            .position(|unit| matches!(unit.op, Instruction::JumpBackward { .. }))
            .map(|idx| cond_idx + 1 + idx)
            .expect("missing branch-arm loop backedge");
        assert!(
            return_idx < jump_back_idx,
            "expected branch-arm return before loop backedge, got ops={ops:?}"
        );
    }

    #[test]
    fn test_nested_implicit_while_tail_return_orders_backedge_before_return() {
        let code = compile_exec(
            "\
def f(self, rawdata, j, match):
    while 1:
        c = rawdata[j:j + 1]
        if c in \"'\\\"\":
            m = match(rawdata, j)
            if not m:
                return -1
            j = m.end()
        else:
            name, j = self.scan(j)
            if j < 0:
                return j
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();
        let compare_idx = ops
            .iter()
            .enumerate()
            .rfind(|(_, unit)| matches!(unit.op, Instruction::CompareOp { .. }))
            .map(|(idx, _)| idx)
            .expect("missing nested tail comparison");
        let cond_idx = ops[compare_idx + 1..]
            .iter()
            .position(|unit| {
                matches!(
                    unit.op,
                    Instruction::PopJumpIfFalse { .. } | Instruction::PopJumpIfTrue { .. }
                )
            })
            .map(|idx| compare_idx + 1 + idx)
            .expect("missing nested tail conditional jump");
        assert!(
            matches!(ops[cond_idx].op, Instruction::PopJumpIfTrue { .. }),
            "expected CPython-style true jump to nested return path, got ops={ops:?}"
        );
        let jump_back_idx = ops[cond_idx + 1..]
            .iter()
            .position(|unit| matches!(unit.op, Instruction::JumpBackward { .. }))
            .map(|idx| cond_idx + 1 + idx)
            .expect("missing nested tail loop backedge");
        let return_idx = ops[cond_idx + 1..]
            .iter()
            .position(|unit| matches!(unit.op, Instruction::ReturnValue))
            .map(|idx| cond_idx + 1 + idx)
            .expect("missing nested tail return path");
        assert!(
            jump_back_idx < return_idx,
            "expected nested implicit loop backedge before return block, got ops={ops:?}"
        );
    }

    #[test]
    fn test_join_store_global_before_import_keeps_strong_load_fast() {
        let code = compile_exec(
            "\
def f(module=None):
    global ET
    if module is None:
        module = pyET
    ET = module
    from xml.etree import ElementPath
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(2).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::LoadFast { .. },
                        Instruction::StoreGlobal { .. },
                    ]
                )
            }),
            "expected CPython-style strong LOAD_FAST before join STORE_GLOBAL followed by import, got ops={ops:?}"
        );
    }

    #[test]
    fn test_handler_resume_join_keeps_borrow_in_common_tail() {
        let code = compile_exec(
            "\
def f(p, errors, s, pos, look, final, escape_start, st):
    try:
        chr_codec = unicodedata.lookup('%s' % st)
    except LookupError as e:
        x = unicode_call_errorhandler(
            errors, 'unicodeescape', 'unknown Unicode character name', s, pos - 1, look + 1
        )
    else:
        x = chr_codec, look + 1
    p.append(x[0])
    pos = x[1]
    if not final:
        pos = escape_start
        return p, pos
    return unicode_call_errorhandler(
        errors, 'unicodeescape', 'unknown Unicode character name', s, pos - 1, look + 1
    )
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let append_idx = f
            .instructions
            .iter()
            .position(|unit| match unit.op {
                Instruction::LoadAttr { namei } => {
                    let load_attr = namei.get(OpArg::new(u32::from(u8::from(unit.arg))));
                    f.names[usize::try_from(load_attr.name_idx()).unwrap()].as_str() == "append"
                }
                _ => false,
            })
            .expect("missing append tail");
        let tail: Vec<_> = f.instructions[append_idx.saturating_sub(1)..]
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            matches!(
                tail.as_slice(),
                [
                    Instruction::LoadFastBorrow { .. },
                    Instruction::LoadAttr { .. },
                    Instruction::LoadFastBorrow { .. },
                    ..,
                ]
            ),
            "expected handler resume common tail to start with borrowed append receiver/arg loads, got tail={tail:?}"
        );
        assert!(
            tail.iter().any(|op| {
                matches!(
                    op,
                    Instruction::LoadFastBorrowLoadFastBorrow { .. }
                        | Instruction::LoadFastBorrow { .. }
                )
            }),
            "expected handler resume common tail to keep borrowed LOAD_FAST ops, got tail={tail:?}"
        );
    }

    #[test]
    fn test_multi_handler_guarded_resume_tail_keeps_borrow() {
        let code = compile_exec(
            "\
def f(a):
    try:
        g()
    except ValueError:
        pass
    except TypeError:
        pass
    if a:
        return a.x
    return 0
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(5).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::LoadFastBorrow { .. },
                        Instruction::ToBool,
                        Instruction::PopJumpIfFalse { .. },
                        Instruction::NotTaken,
                        Instruction::LoadFastBorrow { .. },
                    ]
                )
            }),
            "expected guarded resume tail to keep borrowed guard/body loads, got ops={ops:?}"
        );
        assert!(
            ops.windows(2).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::LoadFastBorrow { .. },
                        Instruction::LoadAttr { .. }
                    ]
                )
            }),
            "expected guarded resume tail attr access to keep borrowed receiver, got ops={ops:?}"
        );
    }

    #[test]
    fn test_multi_handler_method_tail_keeps_borrow() {
        let code = compile_exec(
            "\
def f(self, xs):
    for vals, expected in xs:
        try:
            actual = g(vals)
        except OverflowError:
            self.fail(expected)
        except ValueError:
            self.fail(expected)
        self.assertEqual(actual, expected)
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        let assert_equal_idx = ops
            .iter()
            .position(|op| matches!(op, Instruction::LoadAttr { .. }))
            .expect("missing assertEqual LOAD_ATTR");
        let tail = &ops[assert_equal_idx.saturating_sub(1)..];

        assert!(
            matches!(tail.first(), Some(Instruction::LoadFastBorrow { .. })),
            "expected multi-handler method-call tail receiver to keep LOAD_FAST_BORROW, got tail={tail:?}"
        );
        assert!(
            tail.iter()
                .any(|op| matches!(op, Instruction::LoadFastBorrow { .. })),
            "expected multi-handler method-call tail args to keep borrowed loads, got tail={tail:?}"
        );
    }

    #[test]
    fn test_named_except_cleanup_loop_header_keeps_borrow_in_for_loop() {
        let code = compile_exec(
            "\
def f(args):
    for arg in args:
        try:
            _wm._setoption(arg)
        except _wm._OptionError as msg:
            print('Invalid -W option ignored:', msg, file=sys.stderr)
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let attr_idx = f
            .instructions
            .iter()
            .position(|unit| match unit.op {
                Instruction::LoadAttr { namei } => {
                    let load_attr = namei.get(OpArg::new(u32::from(u8::from(unit.arg))));
                    f.names[usize::try_from(load_attr.name_idx()).unwrap()].as_str() == "_setoption"
                }
                _ => false,
            })
            .expect("missing _setoption attr load");
        let window: Vec<_> = f.instructions[attr_idx + 1..]
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .take(3)
            .collect();
        assert!(
            matches!(
                window.as_slice(),
                [
                    Instruction::LoadFastBorrow { .. },
                    Instruction::Call { .. },
                    Instruction::PopTop
                ]
            ),
            "expected loop body call to keep borrowed arg load after named-except cleanup, got window={window:?}"
        );
    }

    #[test]
    fn test_multi_named_except_loop_header_keeps_borrow_for_normal_path() {
        let code = compile_exec(
            "\
def f(self):
    for badval in ['illegal', -1, 1 << 32]:
        class A:
            def __len__(self):
                return badval
        try:
            bool(A())
        except (Exception) as e_bool:
            try:
                len(A())
            except (Exception) as e_len:
                self.assertEqual(str(e_bool), str(e_len))
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(4).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::LoadBuildClass,
                        Instruction::PushNull,
                        Instruction::LoadFastBorrow { .. },
                        Instruction::BuildTuple { .. },
                    ]
                )
            }),
            "expected class closure setup in loop header to borrow badval, got ops={ops:?}"
        );
        assert!(
            ops.windows(5).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::LoadFastBorrow { .. },
                        Instruction::PushNull,
                        Instruction::Call { .. },
                        Instruction::Call { .. },
                        Instruction::PopTop,
                    ]
                )
            }),
            "expected normal bool(A()) path in loop header to borrow A, got ops={ops:?}"
        );
    }

    #[test]
    fn test_named_except_cleanup_simple_resume_tail_keeps_borrow() {
        let code = compile_exec(
            "\
def f(self):
    try:
        1 / 0
    except Exception as e:
        tb = e.__traceback__
    self.get_disassemble_as_string(tb.tb_frame.f_code, tb.tb_lasti)
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let instructions: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();
        let attr_idx = instructions
            .iter()
            .position(|unit| match unit.op {
                Instruction::LoadAttr { namei } => {
                    let load_attr = namei.get(OpArg::new(u32::from(u8::from(unit.arg))));
                    f.names[usize::try_from(load_attr.name_idx()).unwrap()].as_str()
                        == "get_disassemble_as_string"
                }
                _ => false,
            })
            .expect("missing LOAD_ATTR for get_disassemble_as_string");
        let ops: Vec<_> = instructions.iter().map(|unit| unit.op).collect();
        assert!(
            matches!(
                ops.get(attr_idx - 1),
                Some(Instruction::LoadFastBorrow { .. })
            ),
            "expected named-except resume tail to keep borrowed self load, got ops={ops:?}"
        );
        assert!(
            matches!(
                ops.get(attr_idx + 4),
                Some(Instruction::LoadFastBorrow { .. })
            ),
            "expected named-except resume tail to keep borrowed tb load, got ops={ops:?}"
        );
    }

    #[test]
    fn test_named_except_cleanup_conditional_raise_tail_keeps_borrow() {
        let code = compile_exec(
            "\
def f(self):
    try:
        output = self.trace()
        output = output.strip()
    except (A, B, C) as fnfe:
        output = str(fnfe)
    if output != 'probe: success':
        raise E('{} {}'.format(self.command[0], output))
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let instructions: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();
        let raise_idx = instructions
            .iter()
            .position(|unit| matches!(unit.op, Instruction::RaiseVarargs { .. }))
            .expect("missing conditional raise");
        let handler_start = instructions
            .iter()
            .position(|unit| matches!(unit.op, Instruction::PushExcInfo))
            .expect("missing handler entry");
        let tail = &instructions[..handler_start.min(raise_idx)];

        assert!(
            tail.iter().any(|unit| {
                matches!(
                    unit.op,
                    Instruction::LoadFastBorrow { .. }
                        | Instruction::LoadFastBorrowLoadFastBorrow { .. }
                )
            }),
            "named-except cleanup conditional raise tail should keep borrowed loads, got tail={tail:?}"
        );
        assert!(
            !tail
                .iter()
                .any(|unit| matches!(unit.op, Instruction::LoadFast { .. })),
            "named-except cleanup conditional raise tail should not force strong LOAD_FAST, got tail={tail:?}"
        );
    }

    #[test]
    fn test_with_suppress_named_except_resume_tail_uses_strong_loads() {
        let code = compile_exec(
            "\
def f(self, cm, E):
    try:
        with cm:
            pass
    except E as e:
        frames = e
    self.x(frames)
    self.y(frames)
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let instructions: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();
        let first_tail_attr = instructions
            .iter()
            .position(|unit| match unit.op {
                Instruction::LoadAttr { namei } => {
                    let load_attr = namei.get(OpArg::new(u32::from(u8::from(unit.arg))));
                    f.names[usize::try_from(load_attr.name_idx()).unwrap()].as_str() == "x"
                }
                _ => false,
            })
            .expect("missing x attr load");
        let handler_start = instructions
            .iter()
            .position(|unit| matches!(unit.op, Instruction::PushExcInfo))
            .expect("missing handler entry");
        let tail = &instructions[first_tail_attr.saturating_sub(1)..handler_start];

        assert!(
            tail.iter()
                .any(|unit| matches!(unit.op, Instruction::LoadFast { .. })),
            "expected with-suppress/named-except resume tail to use strong LOAD_FAST, got tail={tail:?}"
        );
        assert!(
            tail.iter().all(|unit| {
                !matches!(
                    unit.op,
                    Instruction::LoadFastBorrow { .. }
                        | Instruction::LoadFastBorrowLoadFastBorrow { .. }
                )
            }),
            "expected with-suppress/named-except resume tail not to borrow, got tail={tail:?}"
        );
    }

    #[test]
    fn test_with_except_else_with_resume_loop_tail_uses_strong_loads() {
        let code = compile_exec(
            "\
def f(self, cm, E):
    with cm:
        try:
            g()
        except E:
            pass
        else:
            with self.z(E):
                h()
        for _ in support.sleeping_retry(support.SHORT_TIMEOUT, 'not ready'):
            if self.x:
                break
        self.y()
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let instructions: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();
        let get_iter = instructions
            .iter()
            .position(|unit| matches!(unit.op, Instruction::GetIter))
            .expect("missing loop iterator");
        let handler_start = instructions
            .iter()
            .position(|unit| matches!(unit.op, Instruction::PushExcInfo))
            .expect("missing handler entry");
        let tail = &instructions[get_iter.saturating_sub(1)..handler_start];

        assert!(
            tail.iter()
                .any(|unit| matches!(unit.op, Instruction::LoadFast { .. })),
            "expected with except/else-with resume loop tail to use strong LOAD_FAST ops, got tail={tail:?}"
        );
        assert!(
            tail.iter().all(|unit| {
                !matches!(
                    unit.op,
                    Instruction::LoadFastBorrow { .. }
                        | Instruction::LoadFastBorrowLoadFastBorrow { .. }
                )
            }),
            "with except/else-with resume loop tail should not borrow LOAD_FAST ops, got tail={tail:?}"
        );
    }

    #[test]
    fn test_plain_with_then_global_loop_tail_keeps_borrow() {
        let code = compile_exec(
            "\
def f(self, cm):
    with cm:
        self.x()
    for value in ITEMS:
        self.y(value)
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let instructions: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();
        let y_attr = instructions
            .iter()
            .position(|unit| match unit.op {
                Instruction::LoadAttr { namei } => {
                    let load_attr = namei.get(OpArg::new(u32::from(u8::from(unit.arg))));
                    f.names[usize::try_from(load_attr.name_idx()).unwrap()].as_str() == "y"
                }
                _ => false,
            })
            .expect("missing y attr load");

        assert!(
            matches!(
                instructions
                    .get(y_attr.saturating_sub(1))
                    .map(|unit| unit.op),
                Some(Instruction::LoadFastBorrow { .. })
            ),
            "plain with/global-loop tail should keep CPython-style borrowed self load, got instructions={instructions:?}"
        );
    }

    #[test]
    fn test_context_manager_for_join_tail_keeps_borrow() {
        let code = compile_exec(
            "\
def f(self, factory):
    with factory() as e:
        executor = e
        self.x(e)
    for t in executor._threads:
        t.join()
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let instructions: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();
        let join_attr = instructions
            .iter()
            .position(|unit| match unit.op {
                Instruction::LoadAttr { namei } => {
                    let load_attr = namei.get(OpArg::new(u32::from(u8::from(unit.arg))));
                    f.names[usize::try_from(load_attr.name_idx()).unwrap()].as_str() == "join"
                }
                _ => false,
            })
            .expect("missing join attr load");

        assert!(
            matches!(
                instructions
                    .get(join_attr.saturating_sub(1))
                    .map(|unit| unit.op),
                Some(Instruction::LoadFastBorrow { .. })
            ),
            "context-manager for-join tail should keep CPython-style borrowed t load, got instructions={instructions:?}"
        );
    }

    #[test]
    fn test_with_except_resume_normal_tail_uses_strong_loads() {
        let code = compile_exec(
            "\
def f(self, cm, E):
    try:
        with self.assertRaises(E):
            with cm:
                h()
    except TimeoutError:
        self._fail_on_deadlock(cm)
    cm.shutdown(wait=True)
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let instructions: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();
        let shutdown_attr = instructions
            .iter()
            .position(|unit| match unit.op {
                Instruction::LoadAttr { namei } => {
                    let load_attr = namei.get(OpArg::new(u32::from(u8::from(unit.arg))));
                    f.names[usize::try_from(load_attr.name_idx()).unwrap()].as_str() == "shutdown"
                }
                _ => false,
            })
            .expect("missing shutdown attr load");

        assert!(
            matches!(
                instructions
                    .get(shutdown_attr.saturating_sub(1))
                    .map(|unit| unit.op),
                Some(Instruction::LoadFast { .. })
            ),
            "with/except resume normal tail should keep CPython-style strong cm load, got instructions={instructions:?}"
        );
    }

    #[test]
    fn test_with_except_else_attr_subscript_tail_keeps_borrow() {
        let code = compile_exec(
            "\
def f(self, cm, E, obj):
    try:
        with cm:
            pass
    except E as exc:
        self.x(exc)
    else:
        self.fail('Expected')
    inner = obj.saved_details[1]
    self.x(inner)
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let instructions: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();
        let saved_details = instructions
            .iter()
            .position(|unit| match unit.op {
                Instruction::LoadAttr { namei } => {
                    let load_attr = namei.get(OpArg::new(u32::from(u8::from(unit.arg))));
                    f.names[usize::try_from(load_attr.name_idx()).unwrap()].as_str()
                        == "saved_details"
                }
                _ => false,
            })
            .expect("missing saved_details attr load");
        let handler_start = instructions
            .iter()
            .position(|unit| matches!(unit.op, Instruction::PushExcInfo))
            .expect("missing handler entry");
        let tail = &instructions[saved_details.saturating_sub(1)..handler_start];

        assert!(
            tail.iter().any(|unit| matches!(
                unit.op,
                Instruction::LoadFastBorrow { .. }
                    | Instruction::LoadFastBorrowLoadFastBorrow { .. }
            )),
            "expected except-else attr-subscript tail to keep borrowed LOAD_FAST ops, got tail={tail:?}"
        );
        assert!(
            tail.iter()
                .all(|unit| !matches!(unit.op, Instruction::LoadFast { .. })),
            "except-else attr-subscript tail should not be deoptimized to strong LOAD_FAST, got tail={tail:?}"
        );
    }

    #[test]
    fn test_with_suppress_attr_subscript_tail_keeps_borrow() {
        let code = compile_exec(
            "\
def f(self, cm):
    stack = self.exit_stack()
    with self.assertRaisesRegex(TypeError, 'the context manager'):
        stack.enter_context(cm)
    stack.push(cm)
    self.assertIs(stack._exit_callbacks[-1][1], cm)
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let instructions: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();
        let exit_callbacks = instructions
            .iter()
            .position(|unit| match unit.op {
                Instruction::LoadAttr { namei } => {
                    let load_attr = namei.get(OpArg::new(u32::from(u8::from(unit.arg))));
                    f.names[usize::try_from(load_attr.name_idx()).unwrap()].as_str()
                        == "_exit_callbacks"
                }
                _ => false,
            })
            .expect("missing _exit_callbacks attr load");

        assert!(
            matches!(
                instructions
                    .get(exit_callbacks.saturating_sub(1))
                    .map(|unit| unit.op),
                Some(Instruction::LoadFastBorrow { .. })
            ),
            "with-suppress attr-subscript tail should keep CPython-style borrowed stack load, got instructions={instructions:?}"
        );
    }

    #[test]
    fn test_named_except_conditional_reraise_deopts_with_chain_tail() {
        let code = compile_exec(
            "\
def f(self, arc, tmp_filename, new_mode):
    try:
        os.chmod(tmp_filename, new_mode)
    except OSError as exc:
        if exc.errno == ERR:
            self.skipTest()
        else:
            raise
    with self.check_context(arc.open(), 'fully_trusted'):
        self.expect_file('a')
    with self.check_context(arc.open(), 'tar'):
        self.expect_file('b')
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let instructions: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();
        let first_check_context = instructions
            .iter()
            .position(|unit| match unit.op {
                Instruction::LoadAttr { namei } => {
                    let load_attr = namei.get(OpArg::new(u32::from(u8::from(unit.arg))));
                    f.names[usize::try_from(load_attr.name_idx()).unwrap()].as_str()
                        == "check_context"
                }
                _ => false,
            })
            .expect("missing check_context load");
        let first_handler = instructions
            .iter()
            .position(|unit| matches!(unit.op, Instruction::PushExcInfo))
            .unwrap_or(instructions.len());
        let warm_tail = &instructions[first_check_context.saturating_sub(1)..first_handler];

        assert!(
            warm_tail
                .iter()
                .any(|unit| matches!(unit.op, Instruction::LoadFast { .. })),
            "expected conditional named-except reraise tail to use strong LOAD_FAST ops, got tail={warm_tail:?}"
        );
        assert!(
            warm_tail.iter().all(|unit| {
                !matches!(
                    unit.op,
                    Instruction::LoadFastBorrow { .. }
                        | Instruction::LoadFastBorrowLoadFastBorrow { .. }
                )
            }),
            "expected all warm with-chain tail loads to stay strong after named-except reraise, got tail={warm_tail:?}"
        );
    }

    #[test]
    fn test_terminal_except_before_with_deopts_with_body_borrows() {
        let code = compile_exec(
            "\
def f(self, cm):
    try:
        g()
    except OSError:
        raise Exception('skip')
    with cm:
        self.x()
        self.y()
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let instructions: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();
        let first_tail_attr = instructions
            .iter()
            .position(|unit| match unit.op {
                Instruction::LoadAttr { namei } => {
                    let load_attr = namei.get(OpArg::new(u32::from(u8::from(unit.arg))));
                    f.names[usize::try_from(load_attr.name_idx()).unwrap()].as_str() == "x"
                }
                _ => false,
            })
            .expect("missing x attr load");
        let handler_start = instructions
            .iter()
            .position(|unit| matches!(unit.op, Instruction::PushExcInfo))
            .expect("missing handler entry");
        let with_tail = &instructions[first_tail_attr.saturating_sub(1)..handler_start];

        assert!(
            with_tail
                .iter()
                .any(|unit| matches!(unit.op, Instruction::LoadFast { .. })),
            "expected terminal-except before with to use strong LOAD_FAST ops, got tail={with_tail:?}"
        );
        assert!(
            with_tail.iter().all(|unit| {
                !matches!(
                    unit.op,
                    Instruction::LoadFastBorrow { .. }
                        | Instruction::LoadFastBorrowLoadFastBorrow { .. }
                )
            }),
            "terminal-except before with should not borrow protected with body loads, got tail={with_tail:?}"
        );
    }

    #[test]
    fn test_terminal_except_resume_tail_uses_strong_loads() {
        let code = compile_exec(
            "\
def f(re, proc, unittest):
    try:
        version = proc.communicate()
    except OSError:
        raise unittest.SkipTest('x')
    match = re.search('pat', version)
    if match is None:
        raise unittest.SkipTest(f'Unable to parse readelf version: {version}')
    return int(match.group(1)), int(match.group(2))
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let instructions: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();
        let search_attr = instructions
            .iter()
            .position(|unit| match unit.op {
                Instruction::LoadAttr { namei } => {
                    let load_attr = namei.get(OpArg::new(u32::from(u8::from(unit.arg))));
                    f.names[usize::try_from(load_attr.name_idx()).unwrap()].as_str() == "search"
                }
                _ => false,
            })
            .expect("missing re.search attr load");
        let handler_start = instructions
            .iter()
            .position(|unit| matches!(unit.op, Instruction::PushExcInfo))
            .expect("missing handler entry");
        let tail = &instructions[search_attr.saturating_sub(1)..handler_start];

        let strong_loads_name = |name: &str| {
            tail.iter().any(|unit| match unit.op {
                Instruction::LoadFast { var_num } => {
                    let arg = OpArg::new(u32::from(u8::from(unit.arg)));
                    f.varnames[usize::from(var_num.get(arg))] == name
                }
                Instruction::LoadFastLoadFast { var_nums } => {
                    let arg = OpArg::new(u32::from(u8::from(unit.arg)));
                    let (left, right) = var_nums.get(arg).indexes();
                    f.varnames[usize::from(left)] == name || f.varnames[usize::from(right)] == name
                }
                _ => false,
            })
        };
        let borrows_name = |name: &str| {
            tail.iter().any(|unit| match unit.op {
                Instruction::LoadFastBorrow { var_num } => {
                    let arg = OpArg::new(u32::from(u8::from(unit.arg)));
                    f.varnames[usize::from(var_num.get(arg))] == name
                }
                Instruction::LoadFastBorrowLoadFastBorrow { var_nums } => {
                    let arg = OpArg::new(u32::from(u8::from(unit.arg)));
                    let (left, right) = var_nums.get(arg).indexes();
                    f.varnames[usize::from(left)] == name || f.varnames[usize::from(right)] == name
                }
                _ => false,
            })
        };

        for name in ["re", "version", "match"] {
            assert!(
                strong_loads_name(name),
                "terminal-except resume tail should use strong LOAD_FAST for {name}, got tail={tail:?}"
            );
            assert!(
                !borrows_name(name),
                "terminal-except resume tail should not borrow {name}, got tail={tail:?}"
            );
        }
    }

    #[test]
    fn test_terminal_except_conditional_return_tail_uses_strong_loads() {
        let code = compile_exec(
            "\
def f(param, value, quote):
    try:
        value.encode('ascii')
    except UnicodeEncodeError:
        return param
    if quote:
        return param
    return value
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let instructions: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();
        let handler_start = instructions
            .iter()
            .position(|unit| matches!(unit.op, Instruction::PushExcInfo))
            .expect("missing handler entry");
        let quote_idx = instructions[..handler_start]
            .iter()
            .position(|unit| match unit.op {
                Instruction::LoadFast { var_num } | Instruction::LoadFastBorrow { var_num } => {
                    let arg = OpArg::new(u32::from(u8::from(unit.arg)));
                    f.varnames[usize::from(var_num.get(arg))] == "quote"
                }
                _ => false,
            })
            .expect("missing quote guard load");
        let tail = &instructions[quote_idx..handler_start];

        let strong_loads_name = |name: &str| {
            tail.iter().any(|unit| match unit.op {
                Instruction::LoadFast { var_num } => {
                    let arg = OpArg::new(u32::from(u8::from(unit.arg)));
                    f.varnames[usize::from(var_num.get(arg))] == name
                }
                _ => false,
            })
        };
        let borrows_name = |name: &str| {
            tail.iter().any(|unit| match unit.op {
                Instruction::LoadFastBorrow { var_num } => {
                    let arg = OpArg::new(u32::from(u8::from(unit.arg)));
                    f.varnames[usize::from(var_num.get(arg))] == name
                }
                _ => false,
            })
        };

        for name in ["quote", "param", "value"] {
            assert!(
                strong_loads_name(name),
                "terminal-except conditional tail should use strong LOAD_FAST for {name}, got tail={tail:?}"
            );
            assert!(
                !borrows_name(name),
                "terminal-except conditional tail should not borrow {name}, got tail={tail:?}"
            );
        }
    }

    #[test]
    fn test_terminal_except_successor_call_tail_uses_strong_load() {
        let code = compile_exec(
            "\
def f(curr, decoded_append, packI, curr_clear, Error):
    if len(curr) == 5:
        acc = 0
        for x in curr:
            acc = 85 * acc + (x - 33)
        try:
            decoded_append(packI(acc))
        except Error:
            raise ValueError('overflow') from None
        curr_clear()
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let instructions: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();
        let handler_start = instructions
            .iter()
            .position(|unit| matches!(unit.op, Instruction::PushExcInfo))
            .expect("missing handler entry");
        let curr_clear_load = instructions[..handler_start]
            .iter()
            .rev()
            .find(|unit| match unit.op {
                Instruction::LoadFast { var_num } | Instruction::LoadFastBorrow { var_num } => {
                    let arg = OpArg::new(u32::from(u8::from(unit.arg)));
                    f.varnames[usize::from(var_num.get(arg))] == "curr_clear"
                }
                _ => false,
            })
            .expect("missing curr_clear load");

        assert!(
            matches!(curr_clear_load.op, Instruction::LoadFast { .. }),
            "terminal except successor call tail should use strong LOAD_FAST for curr_clear, got instructions={instructions:?}"
        );
    }

    #[test]
    fn test_protected_method_call_after_terminal_except_tail_uses_strong_loads() {
        let code = compile_exec(
            "\
def f(items, chunk, out, packI, Error):
    for i in items:
        acc = 0
        try:
            for c in chunk:
                acc = acc * 85 + c
        except TypeError:
            raise
        try:
            out.append(packI(acc))
        except Error:
            raise ValueError from None
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let instructions: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();
        let handler_start = instructions
            .iter()
            .position(|unit| matches!(unit.op, Instruction::PushExcInfo))
            .expect("missing handler entry");
        let append_attr = instructions[..handler_start]
            .iter()
            .position(|unit| match unit.op {
                Instruction::LoadAttr { namei } => {
                    let load_attr = namei.get(OpArg::new(u32::from(u8::from(unit.arg))));
                    f.names[usize::try_from(load_attr.name_idx()).unwrap()].as_str() == "append"
                }
                _ => false,
            })
            .expect("missing append LOAD_ATTR");
        let tail = &instructions[append_attr.saturating_sub(1)..handler_start];

        let strong_loads_name = |name: &str| {
            tail.iter().any(|unit| match unit.op {
                Instruction::LoadFast { var_num } => {
                    let arg = OpArg::new(u32::from(u8::from(unit.arg)));
                    f.varnames[usize::from(var_num.get(arg))] == name
                }
                _ => false,
            })
        };
        let borrows_name = |name: &str| {
            tail.iter().any(|unit| match unit.op {
                Instruction::LoadFastBorrow { var_num } => {
                    let arg = OpArg::new(u32::from(u8::from(unit.arg)));
                    f.varnames[usize::from(var_num.get(arg))] == name
                }
                _ => false,
            })
        };

        for name in ["out", "packI", "acc"] {
            assert!(
                strong_loads_name(name),
                "protected method-call after terminal except tail should use strong LOAD_FAST for {name}, got tail={tail:?}"
            );
            assert!(
                !borrows_name(name),
                "protected method-call after terminal except tail should not borrow {name}, got tail={tail:?}"
            );
        }
    }

    #[test]
    fn test_terminal_except_loop_successor_augassign_uses_strong_load_pair() {
        let code = compile_exec(
            "\
def f(items, decoded, b32rev):
    for i in range(0, len(items), 8):
        quanta = items[i:i + 8]
        acc = 0
        try:
            for c in quanta:
                acc = (acc << 5) + b32rev[c]
        except KeyError:
            raise ValueError from None
        decoded += acc.to_bytes(5)
    return decoded
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let instructions: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();
        let to_bytes_attr = instructions
            .iter()
            .position(|unit| match unit.op {
                Instruction::LoadAttr { namei } => {
                    let load_attr = namei.get(OpArg::new(u32::from(u8::from(unit.arg))));
                    f.names[usize::try_from(load_attr.name_idx()).unwrap()].as_str() == "to_bytes"
                }
                _ => false,
            })
            .expect("missing to_bytes LOAD_ATTR");
        let pair = instructions[to_bytes_attr.saturating_sub(1)].op;

        assert!(
            matches!(pair, Instruction::LoadFastLoadFast { .. }),
            "terminal-except loop successor augassign should use strong LOAD_FAST_LOAD_FAST, got instructions={instructions:?}"
        );
    }

    #[test]
    fn test_terminal_except_loop_backedge_keeps_header_borrows() {
        let code = compile_exec(
            "\
def f(self, value, start=0, stop=None):
    i = start
    while stop is None or i < stop:
        try:
            v = self[i]
        except IndexError:
            break
        if v is value or v == value:
            return i
        i += 1
    raise ValueError
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let instructions: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();

        assert!(
            instructions.windows(7).any(|window| {
                matches!(window[0].op, Instruction::StoreFast { .. })
                    && matches!(window[1].op, Instruction::LoadFastBorrow { .. })
                    && matches!(window[2].op, Instruction::PopJumpIfNone { .. })
                    && matches!(window[3].op, Instruction::NotTaken)
                    && matches!(
                        window[4].op,
                        Instruction::LoadFastBorrowLoadFastBorrow { .. }
                    )
                    && matches!(window[5].op, Instruction::CompareOp { .. })
                    && matches!(window[6].op, Instruction::PopJumpIfFalse { .. })
            }),
            "terminal-except loop backedge deopt should not cross into the loop header, got instructions={instructions:?}"
        );
    }

    #[test]
    fn test_loop_if_implicit_continue_places_body_after_jumpback() {
        let code = compile_exec(
            "\
def f(_config_vars, _INITPRE):
    for k in list(_config_vars):
        if k.startswith(_INITPRE):
            del _config_vars[k]
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(7).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::PopJumpIfTrue { .. },
                        Instruction::NotTaken,
                        Instruction::JumpBackward { .. }
                            | Instruction::JumpBackwardNoInterrupt { .. },
                        Instruction::LoadFastBorrowLoadFastBorrow { .. }
                            | Instruction::LoadFastLoadFast { .. },
                        Instruction::DeleteSubscr,
                        Instruction::JumpBackward { .. }
                            | Instruction::JumpBackwardNoInterrupt { .. },
                        Instruction::EndFor,
                    ]
                )
            }),
            "loop if with implicit continue should use CPython body-after-jumpback layout, got ops={ops:?}"
        );
    }

    #[test]
    fn test_loop_nested_if_delete_slice_places_body_after_jumpback() {
        let code = compile_exec(
            "\
def f(compiler_so):
    for idx in reversed(range(len(compiler_so))):
        if compiler_so[idx] == '-arch' and compiler_so[idx + 1] == 'arm64':
            del compiler_so[idx:idx + 2]
",
        );
        let f = find_code(&code, "f").expect("missing function code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            ops.windows(15).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::CompareOp { .. },
                        Instruction::PopJumpIfTrue { .. },
                        Instruction::NotTaken,
                        Instruction::JumpBackward { .. }
                            | Instruction::JumpBackwardNoInterrupt { .. },
                        Instruction::LoadFastBorrowLoadFastBorrow { .. }
                            | Instruction::LoadFastLoadFast { .. },
                        Instruction::LoadSmallInt { .. },
                        Instruction::BinaryOp { .. },
                        Instruction::BinaryOp { .. },
                        Instruction::LoadConst { .. },
                        Instruction::CompareOp { .. },
                        Instruction::PopJumpIfTrue { .. },
                        Instruction::NotTaken,
                        Instruction::JumpBackward { .. }
                            | Instruction::JumpBackwardNoInterrupt { .. },
                        Instruction::LoadFastBorrowLoadFastBorrow { .. }
                            | Instruction::LoadFastLoadFast { .. },
                        Instruction::LoadFastBorrow { .. } | Instruction::LoadFast { .. },
                    ]
                )
            }),
            "nested loop delete-slice condition should put false jump-back before body, got ops={ops:?}"
        );
    }

    #[test]
    fn test_except_handler_with_conditional_raise_and_resume_keeps_borrow() {
        let code = compile_exec(
            "\
def f(formatstr, args, output, overflowok):
    try:
        result = formatstr % args
    except OverflowError:
        if not overflowok:
            raise
        print('overflow')
    else:
        if output and result != output:
            raise AssertionError(result, output)
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let instructions: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();
        let assertion_error = instructions
            .iter()
            .position(|unit| match unit.op {
                Instruction::LoadGlobal { namei } => {
                    let load_global = namei.get(OpArg::new(u32::from(u8::from(unit.arg))));
                    f.names[usize::try_from(load_global >> 1).unwrap()].as_str() == "AssertionError"
                }
                _ => false,
            })
            .expect("missing AssertionError raise");
        let handler_start = instructions
            .iter()
            .position(|unit| matches!(unit.op, Instruction::PushExcInfo))
            .expect("missing handler entry");
        let tail = &instructions[..handler_start.min(assertion_error)];

        assert!(
            tail.iter().any(|unit| {
                matches!(
                    unit.op,
                    Instruction::LoadFastBorrow { .. }
                        | Instruction::LoadFastBorrowLoadFastBorrow { .. }
                )
            }),
            "conditional-raise handler with a resume path should keep borrowed loads, got tail={tail:?}"
        );
        assert!(
            tail.iter()
                .all(|unit| !matches!(unit.op, Instruction::LoadFast { .. })),
            "conditional-raise handler with a resume path should not force strong LOAD_FAST, got tail={tail:?}"
        );
    }

    #[test]
    fn test_reraising_except_else_tail_keeps_borrow() {
        let code = compile_exec(
            "\
def f(self, data, length):
    if self._paused:
        self._pending_data_length = length
        return
    if length == 0:
        self._eof_received()
        return
    if isinstance(self._protocol, protocols.BufferedProtocol):
        try:
            protocols._feed_data_to_buffered_proto(self._protocol, data)
        except (SystemExit, KeyboardInterrupt):
            raise
        except BaseException as exc:
            self._fatal_error(exc, 'x')
            return
    else:
        self._protocol.data_received(data)
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let instructions: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();
        let data_received_attr = instructions
            .iter()
            .position(|unit| match unit.op {
                Instruction::LoadAttr { namei } => {
                    let load_attr = namei.get(OpArg::new(u32::from(u8::from(unit.arg))));
                    f.names[usize::try_from(load_attr.name_idx()).unwrap()].as_str()
                        == "data_received"
                }
                _ => false,
            })
            .expect("missing data_received LOAD_ATTR");
        let ops: Vec<_> = instructions.iter().map(|unit| unit.op).collect();

        assert!(
            matches!(
                ops.get(data_received_attr - 2),
                Some(Instruction::LoadFastBorrow { .. })
            ),
            "normal else tail after reraising handler should keep borrowed self load, got ops={ops:?}"
        );
        assert!(
            matches!(
                ops.get(data_received_attr + 1),
                Some(Instruction::LoadFastBorrow { .. })
            ),
            "normal else tail after reraising handler should keep borrowed data load, got ops={ops:?}"
        );
    }

    #[test]
    fn test_try_else_finally_with_keeps_context_manager_borrow() {
        let code = compile_exec(
            "\
def f(i):
    try:
        1 / 0
    except ZeroDivisionError:
        pass
    else:
        with i as dodgy:
            pass
    finally:
        pass
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let instructions: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();
        let first_with_exit = instructions
            .iter()
            .position(|unit| match unit.op {
                Instruction::LoadSpecial { method } => {
                    method.get(OpArg::new(u32::from(u8::from(unit.arg)))) == SpecialMethod::Exit
                }
                _ => false,
            })
            .expect("missing __exit__ load");

        assert!(
            matches!(
                instructions
                    .get(first_with_exit.saturating_sub(2))
                    .map(|unit| unit.op),
                Some(Instruction::LoadFastBorrow { .. })
            ),
            "try/except/else/finally with setup should keep CPython-style borrowed context manager load, got instructions={instructions:?}"
        );
    }

    #[test]
    fn test_except_star_handler_pop_block_does_not_leave_nop_before_with_exit() {
        let code = compile_exec(
            "\
def f(self):
    with self.assertRaises(TypeError):
        try:
            raise OSError('blah')
        except* ExceptionGroup as e:
            pass
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let ops: Vec<_> = f
            .instructions
            .iter()
            .map(|unit| unit.op)
            .filter(|op| !matches!(op, Instruction::Cache))
            .collect();

        assert!(
            !ops.windows(5).any(|window| {
                matches!(
                    window,
                    [
                        Instruction::Reraise { .. },
                        Instruction::Nop,
                        Instruction::LoadConst { .. },
                        Instruction::LoadConst { .. },
                        Instruction::LoadConst { .. },
                    ]
                )
            }),
            "except* handler cleanup should not leave an extra NOP before with-exit None loads, got ops={ops:?}"
        );
    }

    #[test]
    fn test_except_star_body_to_else_jump_drops_without_line_nop() {
        let code = compile_exec(
            "\
async def f(self, cm):
    try:
        async with cm:
            pass
    except* Exception:
        pass
    else:
        self.fail()
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let instructions: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();
        let fail_attr = instructions
            .iter()
            .position(|unit| match unit.op {
                Instruction::LoadAttr { namei } => {
                    let load_attr = namei.get(OpArg::new(u32::from(u8::from(unit.arg))));
                    f.names[usize::try_from(load_attr.name_idx()).unwrap()].as_str() == "fail"
                }
                _ => false,
            })
            .expect("missing self.fail load");

        assert!(
            !matches!(
                instructions
                    .get(fail_attr.saturating_sub(2))
                    .map(|unit| unit.op),
                Some(Instruction::Nop)
            ),
            "body-to-else jump should be no-location and disappear, got instructions={instructions:?}"
        );
    }

    #[test]
    fn test_resuming_except_before_with_keeps_with_body_borrows() {
        let code = compile_exec(
            "\
def f(self, cm):
    try:
        g()
    except OSError:
        pass
    with cm:
        self.x()
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let instructions: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();
        let first_tail_attr = instructions
            .iter()
            .position(|unit| match unit.op {
                Instruction::LoadAttr { namei } => {
                    let load_attr = namei.get(OpArg::new(u32::from(u8::from(unit.arg))));
                    f.names[usize::try_from(load_attr.name_idx()).unwrap()].as_str() == "x"
                }
                _ => false,
            })
            .expect("missing x attr load");
        let handler_start = instructions
            .iter()
            .position(|unit| matches!(unit.op, Instruction::PushExcInfo))
            .expect("missing handler entry");
        let with_tail = &instructions[first_tail_attr.saturating_sub(1)..handler_start];

        assert!(
            with_tail.iter().any(|unit| {
                matches!(
                    unit.op,
                    Instruction::LoadFastBorrow { .. }
                        | Instruction::LoadFastBorrowLoadFastBorrow { .. }
                )
            }),
            "resuming except before with should keep borrowed LOAD_FAST ops, got tail={with_tail:?}"
        );
    }

    #[test]
    fn test_nested_finally_except_resume_loop_uses_strong_loads() {
        let code = compile_exec(
            "\
def f(self, xs):
    try:
        try:
            g()
        finally:
            h()
    except OSError:
        self.skipTest('x')
    for x in xs:
        self.x(x)
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let instructions: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();
        let for_iter = instructions
            .iter()
            .position(|unit| matches!(unit.op, Instruction::ForIter { .. }))
            .expect("missing FOR_ITER");
        let handler_start = instructions
            .iter()
            .position(|unit| matches!(unit.op, Instruction::PushExcInfo))
            .expect("missing handler entry");
        let loop_tail = &instructions[for_iter.saturating_sub(2)..handler_start];

        assert!(
            loop_tail
                .iter()
                .any(|unit| matches!(unit.op, Instruction::LoadFast { .. })),
            "expected nested finally/except resume loop to use strong LOAD_FAST ops, got tail={loop_tail:?}"
        );
        assert!(
            loop_tail.iter().all(|unit| {
                !matches!(
                    unit.op,
                    Instruction::LoadFastBorrow { .. }
                        | Instruction::LoadFastBorrowLoadFastBorrow { .. }
                )
            }),
            "nested finally/except resume loop should not borrow LOAD_FAST ops, got tail={loop_tail:?}"
        );
    }

    #[test]
    fn test_finally_protected_loop_without_except_resume_keeps_borrows() {
        let code = compile_exec(
            "\
def f(self, obj, expected, buf):
    try:
        lines = obj.readlines()
    except ValueError:
        self.fail('x')
    if lines != expected:
        self.fail('bad')
    obj.close()
    obj = self.open()
    try:
        for line in obj:
            pass
        try:
            obj.readline()
            obj.readinto(buf)
        except ValueError:
            self.fail('inner')
    finally:
        obj.close()
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let instructions: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();
        let get_iter = instructions
            .iter()
            .position(|unit| matches!(unit.op, Instruction::GetIter))
            .expect("missing GET_ITER");
        assert!(
            matches!(
                instructions
                    .get(get_iter.saturating_sub(1))
                    .map(|unit| unit.op),
                Some(Instruction::LoadFastBorrow { .. })
            ),
            "finally-protected loop without except resume should keep borrowed iterable load, got instructions={instructions:?}"
        );

        for attr_name in ["close", "open"] {
            let attr_idx = instructions[..get_iter]
                .iter()
                .rposition(|unit| match unit.op {
                    Instruction::LoadAttr { namei } => {
                        let load_attr = namei.get(OpArg::new(u32::from(u8::from(unit.arg))));
                        f.names[usize::try_from(load_attr.name_idx()).unwrap()].as_str()
                            == attr_name
                    }
                    _ => false,
                })
                .unwrap_or_else(|| panic!("missing {attr_name} attr load before loop"));
            assert!(
                matches!(
                    instructions
                        .get(attr_idx.saturating_sub(1))
                        .map(|unit| unit.op),
                    Some(Instruction::LoadFastBorrow { .. })
                ),
                "pre-loop {attr_name} call should keep borrowed receiver load, got instructions={instructions:?}"
            );
        }
    }

    #[test]
    fn test_plain_except_resume_loop_keeps_borrows() {
        let code = compile_exec(
            "\
def f(self, xs):
    try:
        g()
    except OSError:
        self.skipTest('x')
    for x in xs:
        self.x(x)
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let instructions: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();
        let for_iter = instructions
            .iter()
            .position(|unit| matches!(unit.op, Instruction::ForIter { .. }))
            .expect("missing FOR_ITER");
        let handler_start = instructions
            .iter()
            .position(|unit| matches!(unit.op, Instruction::PushExcInfo))
            .expect("missing handler entry");
        let loop_tail = &instructions[for_iter.saturating_sub(2)..handler_start];

        assert!(
            loop_tail.iter().any(|unit| {
                matches!(
                    unit.op,
                    Instruction::LoadFastBorrow { .. }
                        | Instruction::LoadFastBorrowLoadFastBorrow { .. }
                )
            }),
            "plain except resume loop should keep borrowed LOAD_FAST ops, got tail={loop_tail:?}"
        );
    }

    #[test]
    fn test_named_except_cleanup_deopts_same_guard_fallbacks_not_outer_tail() {
        let code = compile_exec(
            r#"
def f(s, size, errors, final):
    found_invalid_escape = False
    p = []
    pos = 0
    while pos < size:
        ch = chr(s[pos])
        pos += 1
        if ch == "N":
            message = "malformed \\N character escape"
            look = pos
            try:
                import unicodedata
            except ImportError:
                message = "\\N escapes not supported (can't load unicodedata module)"
                unicode_call_errorhandler(
                    errors, "unicodeescape", message, s, pos - 1, size
                )
                continue
            if look < size and chr(s[look]) == "{":
                while look < size and chr(s[look]) != "}":
                    look += 1
                if look > pos + 1 and look < size and chr(s[look]) == "}":
                    message = "unknown Unicode character name"
                    st = s[pos + 1 : look]
                    try:
                        chr_codec = unicodedata.lookup("%s" % st)
                    except LookupError as e:
                        x = unicode_call_errorhandler(
                            errors, "unicodeescape", message, s, pos - 1, look + 1
                        )
                    else:
                        x = chr_codec, look + 1
                    p.append(x[0])
                    pos = x[1]
                else:
                    if not final:
                        pos = 0
                        break
                    x = unicode_call_errorhandler(
                        errors, "unicodeescape", message, s, pos - 1, look + 1
                    )
                    p.append(x[0])
                    pos = x[1]
            else:
                if not final:
                    pos = 0
                    break
                x = unicode_call_errorhandler(
                    errors, "unicodeescape", message, s, pos - 1, look + 1
                )
                p.append(x[0])
                pos = x[1]
        else:
            if not found_invalid_escape:
                found_invalid_escape = True
                warnings.warn(
                    "invalid escape sequence '\\%c'" % ch, DeprecationWarning, 2
                )
            p.append("\\")
            p.append(ch)
    return p, pos
"#,
        );
        let f = find_code(&code, "f").expect("missing f code");

        let mut saw_strong_final = false;
        let mut saw_borrow_p_after_warn = false;
        let mut saw_borrow_ch_after_warn = false;
        let mut after_warn_attr = false;

        for unit in f.instructions.iter() {
            match unit.op {
                Instruction::LoadAttr { namei } => {
                    let load_attr = namei.get(OpArg::new(u32::from(u8::from(unit.arg))));
                    let name = f.names[usize::try_from(load_attr.name_idx()).unwrap()].as_str();
                    if name == "warn" {
                        after_warn_attr = true;
                    }
                }
                Instruction::LoadFast { var_num } => {
                    let idx = usize::from(var_num.get(OpArg::new(u32::from(u8::from(unit.arg)))));
                    let name = f.varnames[idx].as_str();
                    if name == "final" {
                        saw_strong_final = true;
                    }
                }
                Instruction::LoadFastBorrow { var_num } => {
                    let idx = usize::from(var_num.get(OpArg::new(u32::from(u8::from(unit.arg)))));
                    let name = f.varnames[idx].as_str();
                    if after_warn_attr && name == "p" {
                        saw_borrow_p_after_warn = true;
                    }
                    if after_warn_attr && name == "ch" {
                        saw_borrow_ch_after_warn = true;
                    }
                }
                _ => {}
            }
        }

        assert!(
            saw_strong_final,
            "expected named-except fallback guards to deopt final to strong LOAD_FAST"
        );
        assert!(
            saw_borrow_p_after_warn && saw_borrow_ch_after_warn,
            "expected outer invalid-escape tail to keep borrowed p/ch loads"
        );
    }

    #[test]
    fn test_imap_idle_status_debug_tail_keeps_borrow() {
        let code = compile_exec(
            "\
def f(self, exc_type, CRLF, OSError):
    imap = self._imap
    try:
        imap.send(b'DONE' + CRLF)
        status, [msg] = imap._command_complete('IDLE', self._tag)
        if __debug__ and imap.debug >= 4:
            imap._mesg(f'idle status: {status} {msg!r}')
    except OSError:
        if not exc_type:
            raise
    return False
",
        );
        let f = find_code(&code, "f").expect("missing f code");
        let instructions: Vec<_> = f
            .instructions
            .iter()
            .filter(|unit| !matches!(unit.op, Instruction::Cache))
            .collect();
        let mesg_attr = instructions
            .iter()
            .position(|unit| match unit.op {
                Instruction::LoadAttr { namei } => {
                    let load_attr = namei.get(OpArg::new(u32::from(u8::from(unit.arg))));
                    f.names[usize::try_from(load_attr.name_idx()).unwrap()].as_str() == "_mesg"
                }
                _ => false,
            })
            .expect("missing _mesg attr load");
        let tail = &instructions[mesg_attr.saturating_sub(1)..];

        assert!(
            tail.iter()
                .any(|unit| matches!(unit.op, Instruction::LoadFastBorrow { .. })),
            "expected idle status debug tail to keep borrowed loads, got tail={tail:?}"
        );
    }

    #[test]
    fn test_match_async_comprehension_iter_keeps_capture_borrow() {
        let code = compile_exec(
            r#"
async def name_4():
    match b'':
        case True:
            pass
        case name_5 if f'e':
            {name_3: name_4 async for name_2 in name_5}
        case []:
            pass
    [[]]
"#,
        );
        let name_4 = find_code(&code, "name_4").expect("missing name_4 code");
        let Some(get_aiter_pos) = name_4
            .instructions
            .iter()
            .position(|unit| matches!(unit.op, Instruction::GetAIter))
        else {
            panic!("missing GET_AITER in name_4");
        };
        let prev = &name_4.instructions[get_aiter_pos - 1];
        assert!(
            matches!(
                prev.op,
                Instruction::LoadFastBorrow { var_num }
                    if name_4.varnames[usize::from(var_num.get(OpArg::new(u32::from(u8::from(prev.arg)))))] == "name_5"
            ),
            "expected async comprehension iterator capture to borrow name_5 before GET_AITER, got {prev:?}"
        );
    }
}
