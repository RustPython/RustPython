/* Python code is pre-scanned for symbols in the ast.

This ensures that global and nonlocal keywords are picked up.
Then the compiler can use the symbol table to generate proper
load and store instructions for names.

Inspirational file: https://github.com/python/cpython/blob/main/Python/symtable.c
*/

use crate::{
    IndexMap, IndexSet,
    error::{CodegenError, CodegenErrorType},
};
use alloc::{borrow::Cow, fmt};
use bitflags::bitflags;
use ruff_python_ast as ast;
use ruff_text_size::{Ranged, TextRange};
use rustpython_compiler_core::{PositionEncoding, SourceFile, SourceLocation};

const DEFAULT_RECURSION_LIMIT: usize = 1000;
const RECURSION_ERROR: &str = "maximum recursion depth exceeded during compilation";

/// Captures all symbols in the current scope, and has a list of sub-scopes in this scope.
#[derive(Clone)]
pub struct SymbolTable {
    /// The name of this symbol table. Often the name of the class or function.
    pub name: String,

    /// The type of symbol table
    pub typ: CompilerScope,

    /// The line number in the source code where this symboltable begins.
    pub line_number: u32,

    // Return True if the block is a nested class or function
    pub is_nested: bool,

    /// Whether this function-like scope was created directly in a class block.
    pub is_method: bool,

    /// A set of symbols present on this scope level.
    pub symbols: IndexMap<String, Symbol>,

    /// A list of sub-scopes in the order as found in the
    /// AST nodes.
    pub sub_tables: Vec<Self>,

    /// Annotation scopes registered in st_blocks but not added
    /// to ste_children, e.g. future-annotation function signatures.
    pub hidden_annotation_blocks: Vec<Self>,

    /// Cursor pointing to the next hidden annotation block to consume.
    pub next_hidden_annotation_block: usize,

    /// Inlined comprehension scopes removed from ste_children but
    /// can still find through st_blocks keyed by the comprehension expression.
    pub inlined_comprehension_blocks: Vec<Self>,

    /// Cursor pointing to the next inlined comprehension block to consume.
    pub next_inlined_comprehension_block: usize,

    /// Cursor pointing to the next sub-table to consume during compilation.
    pub next_sub_table: usize,

    /// Variable names in definition order (parameters first, then locals)
    pub varnames: Vec<String>,

    /// Whether this class scope needs an implicit __class__ cell
    pub needs_class_closure: bool,

    /// Whether this class scope needs an implicit __classdict__ cell
    pub needs_classdict: bool,

    /// Whether this type param scope can see the parent class scope
    pub can_see_class_scope: bool,

    /// Whether this scope contains yield/yield from (is a generator function)
    pub is_generator: bool,

    /// Whether this scope contains await or async comprehension machinery.
    pub is_coroutine: bool,

    /// Whether this scope contains a return statement with a value.
    pub returns_value: bool,

    /// Whether this block visited at least one annotation expression.
    pub annotations_used: bool,

    /// Optional description of the current type-variable evaluator context.
    pub scope_info: Option<&'static str>,

    /// Whether this annotation block is currently visiting an unevaluated
    /// function-local annotation.
    pub in_unevaluated_annotation: bool,

    /// Whether this comprehension scope should be inlined (PEP 709)
    /// True for list/set/dict comprehensions in non-generator expressions
    pub comp_inlined: bool,

    /// PEP 649: Reference to annotation scope for this block
    /// Annotations are compiled as a separate `__annotate__` function
    pub annotation_block: Option<Box<Self>>,

    /// True only for deferred function/class/module annotation scopes that
    /// should resolve outer names as if they were siblings of the owning
    /// function body, matching PEP 649 lookup rules.
    pub skip_enclosing_function_scope: bool,

    /// PEP 649: Whether this scope has conditional annotations
    /// (annotations inside if/for/while/etc. blocks or at module level)
    pub has_conditional_annotations: bool,

    /// Whether `from __future__ import annotations` is active
    pub future_annotations: bool,

    /// Names of type parameters that should still be mangled in type param scopes.
    /// When Some, only names in this set are mangled; other names are left unmangled.
    /// Set on type param blocks for generic classes; inherited by non-class child scopes.
    pub mangled_names: Option<IndexSet<String>>,
}

impl SymbolTable {
    fn new(name: String, typ: CompilerScope, line_number: u32, is_nested: bool) -> Self {
        Self {
            name,
            typ,
            line_number,
            is_nested,
            is_method: false,
            symbols: IndexMap::default(),
            sub_tables: vec![],
            hidden_annotation_blocks: vec![],
            next_hidden_annotation_block: 0,
            inlined_comprehension_blocks: vec![],
            next_inlined_comprehension_block: 0,
            next_sub_table: 0,
            varnames: Vec::new(),
            needs_class_closure: false,
            needs_classdict: false,
            can_see_class_scope: false,
            is_generator: false,
            is_coroutine: false,
            returns_value: false,
            annotations_used: false,
            scope_info: None,
            in_unevaluated_annotation: false,
            comp_inlined: false,
            annotation_block: None,
            skip_enclosing_function_scope: false,
            has_conditional_annotations: false,
            future_annotations: false,
            mangled_names: None,
        }
    }

    fn add_format_parameter(&mut self) {
        let name = ".format";
        let symbol = self
            .symbols
            .entry(name.to_owned())
            .or_insert_with(|| Symbol::new(name));
        symbol
            .flags
            .insert(SymbolFlags::PARAMETER | SymbolFlags::REFERENCED);
        if !self.varnames.iter().any(|varname| varname == name) {
            self.varnames.push(name.to_owned());
        }
    }

    pub fn scan_program(
        program: &ast::ModModule,
        source_file: SourceFile,
    ) -> SymbolTableResult<Self> {
        Self::scan_program_with_options(program, source_file, false, false, DEFAULT_RECURSION_LIMIT)
    }

    pub fn scan_program_with_options(
        program: &ast::ModModule,
        source_file: SourceFile,
        allow_top_level_await: bool,
        future_annotations: bool,
        recursion_limit: usize,
    ) -> SymbolTableResult<Self> {
        let mut builder = SymbolTableBuilder::new(source_file);
        builder.allow_top_level_await = allow_top_level_await;
        builder.recursion_limit = recursion_limit;
        builder.future_annotations = future_annotations
            || SymbolTableBuilder::future_annotations_from_module_body(program.body.as_ref());
        builder.scan_statements(program.body.as_ref())?;
        builder.finish()
    }

    pub fn scan_expr(
        expr: &ast::ModExpression,
        source_file: SourceFile,
    ) -> SymbolTableResult<Self> {
        Self::scan_expr_with_options(expr, source_file, false, false, DEFAULT_RECURSION_LIMIT)
    }

    pub fn scan_expr_with_options(
        expr: &ast::ModExpression,
        source_file: SourceFile,
        allow_top_level_await: bool,
        future_annotations: bool,
        recursion_limit: usize,
    ) -> SymbolTableResult<Self> {
        let mut builder = SymbolTableBuilder::new(source_file);
        builder.allow_top_level_await = allow_top_level_await;
        builder.recursion_limit = recursion_limit;
        builder.future_annotations = future_annotations;
        builder.scan_expression(expr.body.as_ref(), ExpressionContext::Load)?;
        builder.finish()
    }

    #[must_use]
    pub fn lookup(&self, name: &str) -> Option<&Symbol> {
        self.symbols.get(name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompilerScope {
    Module,
    Class,
    Function,
    AsyncFunction,
    Lambda,
    Comprehension,
    TypeParams,
    /// PEP 649: Annotation scope for deferred evaluation
    Annotation,
    TypeAlias,
    TypeVariable,
}

impl fmt::Display for CompilerScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Module => write!(f, "module"),
            Self::Class => write!(f, "class"),
            Self::Function => write!(f, "function"),
            Self::AsyncFunction => write!(f, "async function"),
            Self::Lambda => write!(f, "lambda"),
            Self::Comprehension => write!(f, "comprehension"),
            Self::TypeParams => write!(f, "type parameter"),
            Self::Annotation => write!(f, "annotation"),
            Self::TypeAlias => write!(f, "type alias"),
            Self::TypeVariable => write!(f, "TypeVar bound"),
        }
    }
}

/// Indicator for a single symbol what the scope of this symbol is.
/// The scope can be unknown, which is unfortunate, but not impossible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolScope {
    Unknown,
    Local,
    GlobalExplicit,
    GlobalImplicit,
    Free,
    Cell,
}

impl SymbolScope {
    /// Returns the [`i32`] representation of this symbol scope.
    ///
    /// # See also
    /// [CPython's definition](https://github.com/python/cpython/blob/v3.14.6/Include/internal/pycore_symtable.h#L180-L184)
    #[must_use]
    pub const fn as_i32(&self) -> i32 {
        match self {
            Self::Unknown => 0,
            Self::Local => 1,
            Self::GlobalExplicit => 2,
            Self::GlobalImplicit => 3,
            Self::Free => 4,
            Self::Cell => 5,
        }
    }
}

impl From<SymbolScope> for i32 {
    fn from(scope: SymbolScope) -> Self {
        scope.as_i32()
    }
}

bitflags! {
    #[derive(Copy, Clone, Debug, PartialEq, Eq)]
    pub struct SymbolFlags: u16 {
        const REFERENCED = 0x001;  // USE
        const ASSIGNED = 0x002;    // DEF_LOCAL
        const PARAMETER = 0x004;   // DEF_PARAM
        const ANNOTATED = 0x008;   // DEF_ANNOT
        const IMPORTED = 0x010;    // DEF_IMPORT
        const NONLOCAL = 0x020;    // DEF_NONLOCAL
        // indicates if the symbol gets a value assigned by a named expression in a comprehension
        // this is required to correct the scope in the analysis.
        const ASSIGNED_IN_COMPREHENSION = 0x040;
        // indicates that the symbol is used a bound iterator variable. We distinguish this case
        // from normal assignment to detect disallowed re-assignment to iterator variables.
        const ITER = 0x080;
        /// indicates that the symbol is a free variable in a class method from the scope that the
        /// class is defined in, e.g.:
        /// ```python
        /// def foo(x):
        ///     class A:
        ///         def method(self):
        ///             return x // is_free_class
        /// ```
        const FREE_CLASS = 0x100;  // DEF_FREE_CLASS
        const GLOBAL = 0x200;      // DEF_GLOBAL
        const COMP_ITER = 0x400;   // DEF_COMP_ITER
        const COMP_CELL = 0x800;   // DEF_COMP_CELL
        const TYPE_PARAM = 0x1000; // DEF_TYPE_PARAM
        const BOUND = Self::ASSIGNED.bits() | Self::PARAMETER.bits() | Self::IMPORTED.bits() | Self::ITER.bits() | Self::TYPE_PARAM.bits();
    }
}

/// A single symbol in a table. Has various properties such as the scope
/// of the symbol, and also the various uses of the symbol.
#[derive(Debug, Clone)]
pub struct Symbol {
    pub name: String,
    pub scope: SymbolScope,
    pub flags: SymbolFlags,
    pub location: Option<SourceLocation>,
}

impl Symbol {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            // table,
            scope: SymbolScope::Unknown,
            flags: SymbolFlags::empty(),
            location: None,
        }
    }

    #[must_use]
    pub const fn is_global(&self) -> bool {
        matches!(
            self.scope,
            SymbolScope::GlobalExplicit | SymbolScope::GlobalImplicit
        )
    }

    #[must_use]
    pub const fn is_local(&self) -> bool {
        matches!(self.scope, SymbolScope::Local | SymbolScope::Cell)
    }

    #[must_use]
    pub const fn is_bound(&self) -> bool {
        self.flags.intersects(SymbolFlags::BOUND)
    }
}

#[derive(Debug)]
pub struct SymbolTableError {
    error: String,
    location: Option<SourceLocation>,
}

impl SymbolTableError {
    #[must_use]
    pub fn into_codegen_error(self, source_path: String) -> CodegenError {
        let error = if self.error == RECURSION_ERROR {
            CodegenErrorType::RecursionError
        } else {
            CodegenErrorType::SyntaxError(self.error)
        };
        CodegenError {
            location: self.location,
            error,
            source_path,
        }
    }
}

type SymbolTableResult<T = ()> = Result<T, SymbolTableError>;

impl core::fmt::Debug for SymbolTable {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "SymbolTable({:?} symbols, {:?} sub scopes)",
            self.symbols.len(),
            self.sub_tables.len()
        )
    }
}

/* Perform some sort of analysis on nonlocals, globals etc..
  See also: https://github.com/python/cpython/blob/main/Python/symtable.c#L410
*/
fn analyze_symbol_table(symbol_table: &mut SymbolTable) -> SymbolTableResult {
    let mut analyzer = SymbolTableAnalyzer::default();
    // Discard the newfree set at the top level - it's only needed for propagation
    // Pass None for class_entry at top level
    let _newfree = analyzer.analyze_symbol_table(symbol_table, None)?;
    Ok(())
}

/* Drop __class__ and __classdict__ from free variables in class scope
   and set the appropriate flags. Equivalent to drop_class_free().
   See: https://github.com/python/cpython/blob/main/Python/symtable.c#L884

   This function removes __class__ and __classdict__ from the
   `newfree` set (which contains free variables collected from all child scopes)
   and sets the corresponding flags on the class's symbol table entry.
*/
fn drop_class_free(symbol_table: &mut SymbolTable, newfree: &mut IndexSet<String>) {
    // Check if __class__ is in the free variables collected from children
    // If found, it means a child scope (method) references __class__
    if newfree.shift_remove("__class__") {
        symbol_table.needs_class_closure = true;
    }

    // Check if __classdict__ is in the free variables collected from children
    if newfree.shift_remove("__classdict__") {
        symbol_table.needs_classdict = true;
    }

    // Check if __conditional_annotations__ is in the free variables collected from children
    // Remove it from free set - it's handled specially in class scope
    if newfree.shift_remove("__conditional_annotations__") {
        symbol_table.has_conditional_annotations = true;
    }
}

/// PEP 709: Merge symbols from an inlined comprehension into the parent scope.
/// Matches symtable.c inline_comprehension().
fn inline_comprehension(
    parent_symbols: &mut SymbolMap,
    comp: &SymbolTable,
    comp_free: &mut IndexSet<String>,
    inlined_cells: &mut IndexSet<String>,
    parent_type: CompilerScope,
) -> IndexSet<String> {
    let mut removed_class_implicits = IndexSet::default();
    for (name, sub_symbol) in &comp.symbols {
        // Skip the .0 parameter
        if sub_symbol.flags.contains(SymbolFlags::PARAMETER) {
            continue;
        }

        // Track inlined cells
        if sub_symbol.scope == SymbolScope::Cell
            || sub_symbol.flags.contains(SymbolFlags::COMP_CELL)
        {
            inlined_cells.insert(name.clone());
        }

        // __class__, __classdict__ and __conditional_annotations__ are never
        // allowed to be free through a class scope.
        let scope = if sub_symbol.scope == SymbolScope::Free
            && parent_type == CompilerScope::Class
            && matches!(
                name.as_str(),
                "__class__" | "__classdict__" | "__conditional_annotations__"
            ) {
            let is_free_in_child = comp.sub_tables.iter().any(|child| {
                child
                    .symbols
                    .get(name)
                    .is_some_and(|s| s.scope == SymbolScope::Free)
            });
            if !is_free_in_child {
                comp_free.swap_remove(name);
            }
            removed_class_implicits.insert(name.clone());
            SymbolScope::GlobalImplicit
        } else {
            sub_symbol.scope
        };

        if let Some(existing) = parent_symbols.get(name) {
            // Name exists in parent
            if existing.is_bound() && parent_type != CompilerScope::Class {
                // Check if the name is free in any child of the comprehension
                let is_free_in_child = comp.sub_tables.iter().any(|child| {
                    child
                        .symbols
                        .get(name)
                        .is_some_and(|s| s.scope == SymbolScope::Free)
                });
                if !is_free_in_child {
                    comp_free.swap_remove(name);
                }
            }
        } else {
            // Name doesn't exist in parent, copy the comprehension binding.
            // Matches inline_comprehension(): newly introduced
            // comprehension locals stay locals in the parent scope.
            let mut symbol = sub_symbol.clone();
            symbol.scope = scope;
            parent_symbols.insert(name.clone(), symbol);
        }
    }
    removed_class_implicits
}

type SymbolMap = IndexMap<String, Symbol>;

mod stack {
    use alloc::vec::Vec;
    use core::ptr::NonNull;
    pub(super) struct StackStack<T> {
        v: Vec<NonNull<T>>,
    }
    impl<T> Default for StackStack<T> {
        fn default() -> Self {
            Self { v: Vec::new() }
        }
    }
    impl<T> StackStack<T> {
        /// Appends a reference to this stack for the duration of the function `f`. When `f`
        /// returns, the reference will be popped off the stack.
        #[cfg(feature = "std")]
        pub(super) fn with_append<F, R>(&mut self, x: &mut T, f: F) -> R
        where
            F: FnOnce(&mut Self) -> R,
        {
            self.v.push(x.into());
            let res = std::panic::catch_unwind(core::panic::AssertUnwindSafe(|| f(self)));
            self.v.pop();
            res.unwrap_or_else(|x| std::panic::resume_unwind(x))
        }

        /// Appends a reference to this stack for the duration of the function `f`. When `f`
        /// returns, the reference will be popped off the stack.
        ///
        /// Without std, panic cleanup is not guaranteed (no catch_unwind).
        #[cfg(not(feature = "std"))]
        pub fn with_append<F, R>(&mut self, x: &mut T, f: F) -> R
        where
            F: FnOnce(&mut Self) -> R,
        {
            self.v.push(x.into());
            let result = f(self);
            self.v.pop();
            result
        }

        pub(super) fn iter(&self) -> impl DoubleEndedIterator<Item = &T> + '_ {
            self.as_ref().iter().copied()
        }
        pub(super) fn iter_mut(&mut self) -> impl DoubleEndedIterator<Item = &mut T> + '_ {
            self.as_mut().iter_mut().map(|x| &mut **x)
        }
        pub(super) fn len(&self) -> usize {
            self.v.len()
        }
        pub(super) fn is_empty(&self) -> bool {
            self.len() == 0
        }

        pub(super) fn as_ref(&self) -> &[&T] {
            unsafe { &*(self.v.as_slice() as *const [NonNull<T>] as *const [&T]) }
        }

        pub(super) fn as_mut(&mut self) -> &mut [&mut T] {
            unsafe { &mut *(self.v.as_mut_slice() as *mut [NonNull<T>] as *mut [&mut T]) }
        }
    }
}
use stack::StackStack;

/// Symbol table analysis. Can be used to analyze a fully
/// build symbol table structure. It will mark variables
/// as local variables for example.
#[derive(Default)]
#[repr(transparent)]
struct SymbolTableAnalyzer {
    tables: StackStack<(SymbolMap, CompilerScope, bool)>,
}

impl SymbolTableAnalyzer {
    /// Analyze a symbol table and return the set of free variables.
    /// See symtable.c analyze_block().
    /// class_entry: PEP 649 - enclosing class symbols for annotation scopes
    fn analyze_symbol_table(
        &mut self,
        symbol_table: &mut SymbolTable,
        class_entry: Option<&SymbolMap>,
    ) -> SymbolTableResult<IndexSet<String>> {
        let symbols = core::mem::take(&mut symbol_table.symbols);
        let sub_tables = &mut *symbol_table.sub_tables;

        let annotation_block = &mut symbol_table.annotation_block;

        // PEP 649: Determine class_entry to pass to children
        let is_class = symbol_table.typ == CompilerScope::Class;

        // Clone class symbols if needed for child scopes with can_see_class_scope
        let needs_class_symbols = (is_class
            && (sub_tables.iter().any(|st| st.can_see_class_scope)
                || annotation_block
                    .as_ref()
                    .is_some_and(|b| b.can_see_class_scope)))
            || (!is_class
                && class_entry.is_some()
                && sub_tables.iter().any(|st| st.can_see_class_scope));

        let class_symbols_clone = if is_class && needs_class_symbols {
            Some(symbols.clone())
        } else {
            None
        };

        // Collect (child_free, is_inlined) pairs from child scopes.
        // We need to process inlined comprehensions after the closure
        // when we have access to symbol_table.symbols.
        let mut child_frees: Vec<(IndexSet<String>, bool)> = Vec::new();
        let mut annotation_free: Option<IndexSet<String>> = None;

        let mut info = (
            symbols,
            symbol_table.typ,
            symbol_table.skip_enclosing_function_scope,
        );
        let class_scope_entry = if is_class {
            class_symbols_clone.as_ref()
        } else {
            class_entry
        };
        self.tables.with_append(&mut info, |list| {
            let inner_scope = unsafe { &mut *(list as *mut _ as *mut Self) };
            for sub_table in sub_tables.iter_mut() {
                let child_class_entry = sub_table
                    .can_see_class_scope
                    .then_some(class_scope_entry)
                    .flatten();
                let child_free = inner_scope.analyze_symbol_table(sub_table, child_class_entry)?;
                child_frees.push((child_free, sub_table.comp_inlined));
            }
            // PEP 649: Analyze annotation block if present
            if let Some(annotation_table) = annotation_block {
                let ann_class_entry = annotation_table
                    .can_see_class_scope
                    .then_some(class_scope_entry)
                    .flatten();
                let child_free =
                    inner_scope.analyze_symbol_table(annotation_table, ann_class_entry)?;
                annotation_free = Some(child_free);
            }
            Ok(())
        })?;

        symbol_table.symbols = info.0;

        // PEP 709: Process inlined comprehensions.
        // Merge symbols from inlined comps into parent scope without bail-out.
        let mut inlined_cells: IndexSet<String> = IndexSet::default();
        let mut newfree = IndexSet::default();
        for (idx, (mut child_free, is_inlined)) in child_frees.into_iter().enumerate() {
            if is_inlined {
                let removed_class_implicit = inline_comprehension(
                    &mut symbol_table.symbols,
                    &symbol_table.sub_tables[idx],
                    &mut child_free,
                    &mut inlined_cells,
                    symbol_table.typ,
                );
                for name in removed_class_implicit {
                    symbol_table.sub_tables[idx]
                        .symbols
                        .shift_remove(name.as_str());
                }
            }
            newfree.extend(child_free);
        }
        if let Some(ann_free) = annotation_free
            && symbol_table.typ == CompilerScope::Class
        {
            // Annotation-only free variables should not leak into function
            // bodies. We only need to propagate them through class scopes so
            // drop_class_free() can materialize implicit class cells when
            // annotation scopes reference them.
            newfree.extend(ann_free);
        }

        let mut inlined_blocks = Vec::new();
        let mut idx = 0;
        while idx < symbol_table.sub_tables.len() {
            if symbol_table.sub_tables[idx].comp_inlined {
                let comp = symbol_table.sub_tables.remove(idx);
                let nested_inlined_blocks = comp.inlined_comprehension_blocks.clone();
                let children = comp.sub_tables.clone();
                let inserted = children.len();
                inlined_blocks.push(comp);
                inlined_blocks.extend(nested_inlined_blocks);
                symbol_table.sub_tables.splice(idx..idx, children);
                idx += inserted;
            } else {
                idx += 1;
            }
        }
        symbol_table
            .inlined_comprehension_blocks
            .extend(inlined_blocks);

        let sub_tables = &*symbol_table.sub_tables;

        for symbol in symbol_table.symbols.values_mut() {
            if inlined_cells.contains(&symbol.name) {
                symbol.flags.insert(SymbolFlags::COMP_CELL);
            }
        }

        // Analyze symbols in current scope
        let function_like_scope = SymbolTableBuilder::is_function_like_scope(symbol_table.typ);
        for symbol in symbol_table.symbols.values_mut() {
            self.analyze_symbol(
                symbol,
                symbol_table.typ,
                symbol_table.skip_enclosing_function_scope,
                sub_tables,
                class_entry,
            )?;

            // analyze_cells(): once a function-like scope owns a
            // child-requested name as a cell, that name is no longer free in
            // the enclosing scope.
            if function_like_scope && symbol.scope == SymbolScope::Cell {
                newfree.shift_remove(symbol.name.as_str());
            }

            // Collect free variables from this scope
            if symbol.scope == SymbolScope::Free || symbol.flags.contains(SymbolFlags::FREE_CLASS) {
                newfree.insert(symbol.name.clone());
            }
        }

        // PEP 709 / symtable.c:
        // - only promote LOCAL -> CELL in function-like scopes, where
        //   analyze_cells() runs. Module and class scopes keep their normal
        //   scope and rely on DEF_COMP_CELL for comprehension-only cells.
        for symbol in symbol_table.symbols.values_mut() {
            if inlined_cells.contains(&symbol.name)
                && function_like_scope
                && symbol.scope == SymbolScope::Local
            {
                symbol.scope = SymbolScope::Cell;
            }
        }

        // Handle class-specific implicit cells
        if symbol_table.typ == CompilerScope::Class {
            drop_class_free(symbol_table, &mut newfree);
        }

        // update_symbols(..., classflag): after class implicit frees
        // are dropped, a class block, or an annotation/type-params block that
        // can see a class scope, records existing child-free names with
        // DEF_FREE_CLASS. This preserves the current scope's own lookup kind
        // (for example GLOBAL_IMPLICIT via __classdict__) while still making
        // the name available as a closure cell for nested children such as
        // generator expressions.
        if symbol_table.typ == CompilerScope::Class || symbol_table.can_see_class_scope {
            for name in &newfree {
                if let Some(symbol) = symbol_table.symbols.get_mut(name) {
                    symbol.flags.insert(SymbolFlags::FREE_CLASS);
                }
            }
        }

        Ok(newfree)
    }

    fn analyze_symbol(
        &mut self,
        symbol: &mut Symbol,
        st_typ: CompilerScope,
        skip_enclosing_function_scope: bool,
        sub_tables: &[SymbolTable],
        class_entry: Option<&SymbolMap>,
    ) -> SymbolTableResult {
        match symbol.scope {
            SymbolScope::Free => {
                if !self.tables.as_ref().is_empty() {
                    let scope_depth = self.tables.as_ref().len();
                    // check if the name is already defined in any outer scope
                    if scope_depth < 2
                        || self.found_in_outer_scope(
                            &symbol.name,
                            st_typ,
                            skip_enclosing_function_scope,
                        ) != Some(SymbolScope::Free)
                    {
                        return Err(SymbolTableError {
                            error: format!("no binding for nonlocal '{}' found", symbol.name),
                            location: symbol.location,
                        });
                    }
                    // Check if the nonlocal binding refers to a type parameter
                    if symbol.flags.contains(SymbolFlags::NONLOCAL) {
                        for (symbols, _typ, _skip) in self.tables.iter().rev() {
                            if let Some(sym) = symbols.get(&symbol.name) {
                                if sym.flags.contains(SymbolFlags::TYPE_PARAM) {
                                    return Err(SymbolTableError {
                                        error: format!(
                                            "nonlocal binding not allowed for type parameter '{}'",
                                            symbol.name
                                        ),
                                        location: symbol.location,
                                    });
                                }
                                if sym.is_bound() {
                                    break;
                                }
                            }
                        }
                    }
                } else {
                    return Err(SymbolTableError {
                        error: format!(
                            "nonlocal {} defined at place without an enclosing scope",
                            symbol.name
                        ),
                        location: symbol.location,
                    });
                }
            }
            SymbolScope::GlobalExplicit | SymbolScope::GlobalImplicit => {}
            SymbolScope::Local | SymbolScope::Cell => {}
            SymbolScope::Unknown => {
                // Try hard to figure out what the scope of this symbol is.
                let scope = if symbol.is_bound() {
                    if symbol.flags.contains(SymbolFlags::COMP_CELL)
                        && matches!(st_typ, CompilerScope::Module | CompilerScope::Class)
                    {
                        // CPython keeps comprehension-only cells in
                        // module/class scopes as normal local/name
                        // bindings and uses DEF_COMP_CELL to allocate the
                        // synthetic cell slot. The spliced comp child
                        // should not force the outer name itself to CELL.
                        SymbolScope::Local
                    } else {
                        self.found_in_inner_scope(sub_tables, &symbol.name, st_typ)
                            .unwrap_or(SymbolScope::Local)
                    }
                } else if let Some(scope) = class_entry
                    .and_then(|class_symbols| class_symbols.get(&symbol.name))
                    .and_then(|class_sym| {
                        if class_sym.flags.contains(SymbolFlags::GLOBAL) {
                            Some(SymbolScope::GlobalExplicit)
                        } else if class_sym.is_bound() && class_sym.scope != SymbolScope::Free {
                            // If name is bound in enclosing class, use GlobalImplicit
                            // so it can be accessed via __classdict__
                            Some(SymbolScope::GlobalImplicit)
                        } else {
                            None
                        }
                    })
                {
                    scope
                } else if let Some(scope) =
                    self.found_in_outer_scope(&symbol.name, st_typ, skip_enclosing_function_scope)
                {
                    // If found in enclosing scope (function/TypeParams), use that
                    scope
                } else if self.tables.is_empty() {
                    // Don't make assumptions when we don't know.
                    SymbolScope::Unknown
                } else {
                    // If there are scopes above we assume global.
                    SymbolScope::GlobalImplicit
                };
                symbol.scope = scope;
            }
        }
        Ok(())
    }

    fn found_in_outer_scope(
        &mut self,
        name: &str,
        st_typ: CompilerScope,
        skip_enclosing_function_scope: bool,
    ) -> Option<SymbolScope> {
        let mut decl_depth = None;
        for (i, (symbols, typ, _skip)) in self.tables.iter().rev().enumerate() {
            if matches!(typ, CompilerScope::Module)
                || matches!(typ, CompilerScope::Class if name != "__class__" && name != "__classdict__" && name != "__conditional_annotations__")
            {
                continue;
            }

            // Real PEP 649 annotation blocks resolve names as siblings of the
            // owning function body. Other annotation-like scopes such as type
            // aliases and TypeVar bound/default evaluators keep normal lexical
            // lookup and therefore leave this path disabled.
            if st_typ == CompilerScope::Annotation
                && skip_enclosing_function_scope
                && i == 0
                && matches!(
                    typ,
                    CompilerScope::Function | CompilerScope::AsyncFunction | CompilerScope::Lambda
                )
            {
                continue;
            }

            // __class__ and __classdict__ are implicitly declared in class scope
            // This handles the case where nested scopes reference them
            if (name == "__class__" || name == "__classdict__")
                && matches!(typ, CompilerScope::Class)
            {
                decl_depth = Some(i);
                break;
            }

            // __conditional_annotations__ is implicitly declared in class scope
            // for classes with conditional annotations
            if name == "__conditional_annotations__" && matches!(typ, CompilerScope::Class) {
                decl_depth = Some(i);
                break;
            }

            if let Some(sym) = symbols.get(name) {
                match sym.scope {
                    SymbolScope::GlobalExplicit => return Some(SymbolScope::GlobalExplicit),
                    SymbolScope::GlobalImplicit => {}
                    _ => {
                        if sym.is_bound() {
                            decl_depth = Some(i);
                            break;
                        }
                    }
                }
            }
        }

        if let Some(decl_depth) = decl_depth {
            // decl_depth is the number of tables between the current one and
            // the one that declared the cell var
            // For implicit class scope variables (__classdict__, __conditional_annotations__),
            // only propagate free to annotation/type-param scopes, not regular functions.
            // Regular method functions don't need these in their freevars.
            let is_class_implicit =
                name == "__classdict__" || name == "__conditional_annotations__";

            for (table, typ, _skip) in self.tables.iter_mut().rev().take(decl_depth) {
                if let CompilerScope::Class = typ {
                    if let Some(free_class) = table.get_mut(name) {
                        free_class.flags.insert(SymbolFlags::FREE_CLASS)
                    } else {
                        let mut symbol = Symbol::new(name);
                        symbol.flags.insert(SymbolFlags::FREE_CLASS);
                        symbol.scope = SymbolScope::Free;
                        table.insert(name.to_owned(), symbol);
                    }
                } else if is_class_implicit
                    && matches!(
                        typ,
                        CompilerScope::Function
                            | CompilerScope::AsyncFunction
                            | CompilerScope::Lambda
                    )
                {
                    // Skip: don't add __classdict__/__conditional_annotations__
                    // as free vars in regular functions — only annotation/type scopes need them
                } else if !table.contains_key(name) {
                    let mut symbol = Symbol::new(name);
                    symbol.scope = SymbolScope::Free;
                    table.insert(name.to_owned(), symbol);
                }
            }
        }

        decl_depth.map(|_| SymbolScope::Free)
    }

    fn found_in_inner_scope(
        &self,
        sub_tables: &[SymbolTable],
        name: &str,
        st_typ: CompilerScope,
    ) -> Option<SymbolScope> {
        sub_tables.iter().find_map(|st| {
            // PEP 709: For inlined comprehensions, check their children
            // instead of the comp itself (its symbols are merged into parent).
            if st.comp_inlined {
                return self.found_in_inner_scope(&st.sub_tables, name, st_typ);
            }
            let sym = st.symbols.get(name)?;
            if sym.scope == SymbolScope::Free
                || (sym.flags.contains(SymbolFlags::FREE_CLASS)
                    && !matches!(st_typ, CompilerScope::Module))
            {
                if st_typ == CompilerScope::Class && name != "__class__" {
                    None
                } else {
                    Some(SymbolScope::Cell)
                }
            } else if sym.scope == SymbolScope::GlobalExplicit && self.tables.is_empty() {
                // the symbol is defined on the module level, and an inner scope declares
                // a global that points to it
                Some(SymbolScope::GlobalExplicit)
            } else {
                None
            }
        })
    }
}

#[derive(Clone, Copy, Debug)]
enum SymbolUsage {
    Global,
    Nonlocal,
    Used,
    Assigned,
    Imported,
    AnnotationAssigned,
    Parameter,
    AnnotationParameter,
    AssignedNamedExprInComprehension,
    Iter,
    TypeParam,
}

struct SymbolTableBuilder {
    class_name: Option<String>,
    // Scope stack.
    tables: Vec<SymbolTable>,
    future_annotations: bool,
    allow_top_level_await: bool,
    source_file: SourceFile,
    // Current scope's varnames being collected (temporary storage)
    current_varnames: Vec<String>,
    // Stack to preserve parent varnames when entering nested scopes
    varnames_stack: Vec<Vec<String>>,
    // Track if we're inside an iterable definition expression (for nested comprehensions)
    in_iter_def_exp: bool,
    // Track if we're scanning an inner loop iteration target (not the first generator)
    in_comp_inner_loop_target: bool,
    // yield/yield from inside comprehension scopes is rejected with a
    // message that names the comprehension kind.
    comprehension_yield_context: Option<&'static str>,
    // PEP 649: Track if we're inside a conditional block (if/for/while/etc.)
    in_conditional_block: bool,
    // Mirrors symtable ENTER_RECURSIVE guards during compilation.
    recursion_depth: usize,
    recursion_limit: usize,
}

/// Enum to indicate in what mode an expression
/// was used.
/// In cpython this is stored in the AST, but I think this
/// is not logical, since it is not context free.
#[derive(Copy, Clone, PartialEq)]
enum ExpressionContext {
    Load,
    Store,
    Delete,
    Iter,
    IterDefinitionExp,
}

impl SymbolTableBuilder {
    fn new(source_file: SourceFile) -> Self {
        let mut this = Self {
            class_name: None,
            tables: vec![],
            future_annotations: false,
            allow_top_level_await: false,
            source_file,
            current_varnames: Vec::new(),
            varnames_stack: Vec::new(),
            in_iter_def_exp: false,
            in_comp_inner_loop_target: false,
            comprehension_yield_context: None,
            in_conditional_block: false,
            recursion_depth: 0,
            recursion_limit: DEFAULT_RECURSION_LIMIT,
        };
        this.enter_scope("top", CompilerScope::Module, 0);
        this
    }

    fn is_function_like_scope(typ: CompilerScope) -> bool {
        matches!(
            typ,
            CompilerScope::Function
                | CompilerScope::AsyncFunction
                | CompilerScope::Lambda
                | CompilerScope::Comprehension
                | CompilerScope::Annotation
                | CompilerScope::TypeAlias
                | CompilerScope::TypeVariable
                | CompilerScope::TypeParams
        )
    }

    fn future_annotations_from_module_body(body: &[ast::Stmt]) -> bool {
        let mut statements = body.iter();
        if let Some(ast::Stmt::Expr(ast::StmtExpr { value, .. })) = statements.clone().next()
            && is_docstring_expr(value)
        {
            statements.next();
        }
        for statement in statements {
            match statement {
                ast::Stmt::ImportFrom(ast::StmtImportFrom {
                    module,
                    names,
                    level,
                    ..
                }) if *level == 0
                    && module.as_ref().map(|id| id.as_str()) == Some("__future__") =>
                {
                    if names
                        .iter()
                        .any(|future| future.name.as_str() == "annotations")
                    {
                        return true;
                    }
                }
                _ => return false,
            }
        }
        false
    }

    fn finish(mut self) -> Result<SymbolTable, SymbolTableError> {
        assert_eq!(self.tables.len(), 1);
        let mut symbol_table = self.tables.pop().unwrap();
        // Save varnames for the top-level module scope
        symbol_table.varnames = self.current_varnames;
        // Propagate future_annotations to the symbol table
        symbol_table.future_annotations = self.future_annotations;
        analyze_symbol_table(&mut symbol_table)?;
        Ok(symbol_table)
    }

    fn enter_scope(&mut self, name: &str, typ: CompilerScope, line_number: u32) {
        let parent = self.tables.last();
        let is_nested =
            parent.is_some_and(|table| table.is_nested || Self::is_function_like_scope(table.typ));
        let is_method = parent.is_some_and(|table| {
            table.typ == CompilerScope::Class
                && matches!(
                    typ,
                    CompilerScope::Function
                        | CompilerScope::AsyncFunction
                        | CompilerScope::Lambda
                        | CompilerScope::Comprehension
                )
        });
        // Inherit mangled_names from parent for non-class scopes
        let inherited_mangled_names = self
            .tables
            .last()
            .and_then(|t| t.mangled_names.clone())
            .filter(|_| typ != CompilerScope::Class);
        let mut table = SymbolTable::new(name.to_owned(), typ, line_number, is_nested);
        table.is_method = is_method;
        table.future_annotations = self.future_annotations;
        table.mangled_names = inherited_mangled_names;
        self.tables.push(table);
        // Save parent's varnames and start fresh for the new scope
        self.varnames_stack
            .push(core::mem::take(&mut self.current_varnames));
    }

    fn enter_type_param_block(
        &mut self,
        name: &str,
        range: TextRange,
        for_class: bool,
        has_defaults: bool,
        has_kwdefaults: bool,
    ) -> SymbolTableResult {
        // Check if we're in a class scope
        let in_class = self
            .tables
            .last()
            .is_some_and(|t| t.typ == CompilerScope::Class);

        self.enter_scope(
            name,
            CompilerScope::TypeParams,
            self.line_index_start(range),
        );

        // Set properties on the newly created type param scope
        if let Some(table) = self.tables.last_mut() {
            table.can_see_class_scope = in_class;
            // For generic classes, create mangled_names set so that only
            // type parameter names get mangled (not bases or other expressions)
            if for_class {
                table.mangled_names = Some(IndexSet::default());
            }
        }

        // Add __classdict__ as a USE symbol in type param scope if in class
        if in_class {
            self.register_name("__classdict__", SymbolUsage::Used, range)?;
        }

        if for_class {
            // It gets set when we create the type params tuple and used when
            // we build up the bases.
            self.register_name(".type_params", SymbolUsage::Assigned, range)?;
            self.register_name(".type_params", SymbolUsage::Used, range)?;
            self.register_name(".generic_base", SymbolUsage::Assigned, range)?;
            self.register_name(".generic_base", SymbolUsage::Used, range)?;
        }
        if has_defaults {
            self.register_name(".defaults", SymbolUsage::Parameter, range)?;
        }
        if has_kwdefaults {
            self.register_name(".kwdefaults", SymbolUsage::Parameter, range)?;
        }

        Ok(())
    }

    /// Pop symbol table and add to sub table of parent table.
    fn leave_scope(&mut self) {
        let mut table = self.tables.pop().unwrap();
        // Save the collected varnames to the symbol table
        table.varnames = core::mem::take(&mut self.current_varnames);
        self.tables.last_mut().unwrap().sub_tables.push(table);
        // Restore parent's varnames
        self.current_varnames = self.varnames_stack.pop().unwrap_or_default();
    }

    /// Pop symbol table without adding it to the parent children list.
    fn discard_scope(&mut self) -> SymbolTable {
        let mut table = self.tables.pop().unwrap();
        table.varnames = core::mem::take(&mut self.current_varnames);
        self.current_varnames = self.varnames_stack.pop().unwrap_or_default();
        table
    }

    /// Enter annotation scope (PEP 649)
    /// Creates or reuses the annotation block for the current scope
    fn enter_annotation_scope(
        &mut self,
        line_number: u32,
        include_classdict_with_future: bool,
        include_conditional_annotations: bool,
    ) {
        let current = self.tables.last_mut().unwrap();
        let can_see_class_scope =
            current.typ == CompilerScope::Class || current.can_see_class_scope;
        let has_conditional = current.has_conditional_annotations;
        let is_nested = current.is_nested || Self::is_function_like_scope(current.typ);

        // Create annotation block if not exists
        if current.annotation_block.is_none() {
            let mut annotation_table = SymbolTable::new(
                "__annotate__".to_owned(),
                CompilerScope::Annotation,
                line_number,
                is_nested,
            );
            // Annotation scope in class can see class scope
            annotation_table.can_see_class_scope = can_see_class_scope;
            annotation_table.skip_enclosing_function_scope = true;
            annotation_table.add_format_parameter();
            current.annotation_block = Some(Box::new(annotation_table));
        }

        // Take the annotation block and push to stack for processing
        let annotation_table = current.annotation_block.take().unwrap();
        self.tables.push(*annotation_table);
        // Save parent's varnames and seed with existing annotation varnames (e.g., "format")
        self.varnames_stack
            .push(core::mem::take(&mut self.current_varnames));
        self.current_varnames = self.tables.last().unwrap().varnames.clone();

        if can_see_class_scope && (include_classdict_with_future || !self.future_annotations) {
            self.add_classdict_freevar();
            // Also add __conditional_annotations__ as free var if parent has conditional annotations
            if include_conditional_annotations && has_conditional {
                self.add_conditional_annotations_freevar();
            }
        }
    }

    /// Leave annotation scope (PEP 649)
    /// Stores the annotation block back to parent instead of sub_tables
    fn leave_annotation_scope(&mut self) {
        let mut table = self.tables.pop().unwrap();
        // Save the collected varnames to the symbol table
        table.varnames = core::mem::take(&mut self.current_varnames);
        // Store back to parent's annotation_block (not sub_tables)
        let parent = self.tables.last_mut().unwrap();
        parent.annotation_block = Some(Box::new(table));
        // Restore parent's varnames
        self.current_varnames = self.varnames_stack.pop().unwrap_or_default();
    }

    fn add_classdict_freevar(&mut self) {
        let table = self.tables.last_mut().unwrap();
        let name = "__classdict__";
        let symbol = table
            .symbols
            .entry(name.to_owned())
            .or_insert_with(|| Symbol::new(name));
        symbol.scope = SymbolScope::Free;
        symbol
            .flags
            .insert(SymbolFlags::REFERENCED | SymbolFlags::FREE_CLASS);
    }

    fn add_conditional_annotations_freevar(&mut self) {
        let table = self.tables.last_mut().unwrap();
        let name = "__conditional_annotations__";
        let symbol = table
            .symbols
            .entry(name.to_owned())
            .or_insert_with(|| Symbol::new(name));
        symbol.scope = SymbolScope::Free;
        symbol
            .flags
            .insert(SymbolFlags::REFERENCED | SymbolFlags::FREE_CLASS);
    }

    /// Walk up the scope chain to determine if we're inside an async function.
    /// Annotation and TypeParams scopes act as async barriers (always non-async).
    /// Comprehension scopes are transparent (inherit parent's async context).
    fn is_in_async_context(&self) -> bool {
        for table in self.tables.iter().rev() {
            match table.typ {
                CompilerScope::AsyncFunction => return true,
                CompilerScope::Function
                | CompilerScope::Lambda
                | CompilerScope::Class
                | CompilerScope::Module
                | CompilerScope::Annotation
                | CompilerScope::TypeAlias
                | CompilerScope::TypeVariable
                | CompilerScope::TypeParams => return false,
                // Comprehension inherits parent's async context
                CompilerScope::Comprehension => continue,
            }
        }
        false
    }

    fn allows_top_level_await(&self) -> bool {
        self.allow_top_level_await
            && self
                .tables
                .last()
                .is_some_and(|table| table.typ == CompilerScope::Module)
    }

    fn line_index_start(&self, range: TextRange) -> u32 {
        self.source_file
            .to_source_code()
            .line_index(range.start())
            .get() as _
    }

    fn scan_statements(&mut self, statements: &[ast::Stmt]) -> SymbolTableResult {
        for statement in statements {
            self.scan_statement(statement)?;
        }
        Ok(())
    }

    fn scan_parameters(&mut self, parameters: &[ast::ParameterWithDefault]) -> SymbolTableResult {
        for parameter in parameters {
            self.scan_parameter(&parameter.parameter)?;
        }
        Ok(())
    }

    fn scan_parameter(&mut self, parameter: &ast::Parameter) -> SymbolTableResult {
        let usage = if parameter.annotation.is_some() {
            SymbolUsage::AnnotationParameter
        } else {
            SymbolUsage::Parameter
        };

        // Check for duplicate parameter names
        let table = self.tables.last().unwrap();
        if table.symbols.contains_key(parameter.name.as_str()) {
            return Err(SymbolTableError {
                error: format!(
                    "duplicate argument '{}' in function definition",
                    parameter.name
                ),
                location: Some(
                    self.source_file
                        .to_source_code()
                        .source_location(parameter.name.range.start(), PositionEncoding::Utf8),
                ),
            });
        }

        self.register_ident(&parameter.name, usage)
    }

    /// Scan an annotation from an AnnAssign statement (can be conditional)
    fn scan_ann_assign_annotation(&mut self, annotation: &ast::Expr) -> SymbolTableResult {
        self.scan_annotation_inner(annotation, true)
    }

    fn scan_function_annotations(
        &mut self,
        parameters: &ast::Parameters,
        returns: Option<&ast::Expr>,
        line_number: u32,
    ) -> SymbolTableResult {
        let current = self.tables.last().unwrap();
        let can_see_class_scope =
            current.typ == CompilerScope::Class || current.can_see_class_scope;
        self.enter_scope("__annotate__", CompilerScope::Annotation, line_number);
        self.tables.last_mut().unwrap().can_see_class_scope = can_see_class_scope;
        self.tables.last_mut().unwrap().add_format_parameter();
        if can_see_class_scope {
            self.register_name("__classdict__", SymbolUsage::Used, TextRange::default())?;
        }

        let was_in_unevaluated_annotation = self.tables.last().unwrap().in_unevaluated_annotation;
        self.tables.last_mut().unwrap().in_unevaluated_annotation = false;

        let result = (|| {
            for annotation in parameters
                .posonlyargs
                .iter()
                .chain(parameters.args.iter())
                .filter_map(|arg| arg.parameter.annotation.as_ref())
            {
                self.tables.last_mut().unwrap().annotations_used = true;
                self.scan_expression(annotation, ExpressionContext::Load)?;
            }
            if let Some(annotation) = parameters
                .vararg
                .as_ref()
                .and_then(|arg| arg.annotation.as_ref())
            {
                self.tables.last_mut().unwrap().annotations_used = true;
                self.scan_expression(annotation, ExpressionContext::Load)?;
            }
            if let Some(annotation) = parameters
                .kwarg
                .as_ref()
                .and_then(|arg| arg.annotation.as_ref())
            {
                self.tables.last_mut().unwrap().annotations_used = true;
                self.scan_expression(annotation, ExpressionContext::Load)?;
            }
            for annotation in parameters
                .kwonlyargs
                .iter()
                .filter_map(|arg| arg.parameter.annotation.as_ref())
            {
                self.tables.last_mut().unwrap().annotations_used = true;
                self.scan_expression(annotation, ExpressionContext::Load)?;
            }
            if let Some(annotation) = returns {
                self.tables.last_mut().unwrap().annotations_used = true;
                self.scan_expression(annotation, ExpressionContext::Load)?;
            }
            Ok(())
        })();

        self.tables.last_mut().unwrap().in_unevaluated_annotation = was_in_unevaluated_annotation;
        if self.future_annotations {
            let annotation_block = self.discard_scope();
            self.tables
                .last_mut()
                .unwrap()
                .hidden_annotation_blocks
                .push(annotation_block);
        } else {
            self.leave_scope();
        }
        result
    }

    fn scan_annotation_inner(
        &mut self,
        annotation: &ast::Expr,
        is_ann_assign: bool,
    ) -> SymbolTableResult {
        let current_scope = self.tables.last().map(|t| t.typ);
        let is_unevaluated = is_ann_assign
            && current_scope.is_some_and(|scope| {
                matches!(
                    scope,
                    CompilerScope::Function | CompilerScope::AsyncFunction | CompilerScope::Lambda
                )
            });
        let needs_conditional_annotations = is_ann_assign
            && (matches!(current_scope, Some(CompilerScope::Module))
                || (matches!(current_scope, Some(CompilerScope::Class))
                    && self.in_conditional_block));
        let should_register_conditional_annotations = needs_conditional_annotations
            && !self.tables.last().unwrap().has_conditional_annotations;

        // PEP 649: Only AnnAssign annotations can be conditional.
        // Function parameter/return annotations are never conditional.
        if needs_conditional_annotations {
            self.tables.last_mut().unwrap().has_conditional_annotations = true;
        }

        if should_register_conditional_annotations {
            self.register_name(
                "__conditional_annotations__",
                SymbolUsage::Used,
                annotation.range(),
            )?;
        }

        // Create annotation scope for deferred evaluation
        let line_number = self.line_index_start(annotation.range());
        self.enter_annotation_scope(line_number, false, true);

        // PEP 649: scan expression for symbol references
        // Class annotations are evaluated in class locals (not module globals)
        let was_in_unevaluated_annotation = self.tables.last().unwrap().in_unevaluated_annotation;
        self.tables.last_mut().unwrap().in_unevaluated_annotation = is_unevaluated;
        let result = self.scan_expression(annotation, ExpressionContext::Load);
        self.tables.last_mut().unwrap().in_unevaluated_annotation = was_in_unevaluated_annotation;

        self.leave_annotation_scope();

        result
    }

    fn scan_statement(&mut self, statement: &ast::Stmt) -> SymbolTableResult {
        if self.recursion_depth >= self.recursion_limit {
            return Err(SymbolTableError {
                error: RECURSION_ERROR.to_owned(),
                location: None,
            });
        }
        self.recursion_depth += 1;
        let result = (|| {
            use ast::*;
            match &statement {
                Stmt::Global(StmtGlobal { names, .. }) => {
                    for name in names {
                        self.register_name(name.as_str(), SymbolUsage::Global, statement.range())?;
                    }
                }
                Stmt::Nonlocal(StmtNonlocal { names, .. }) => {
                    for name in names {
                        self.register_name(
                            name.as_str(),
                            SymbolUsage::Nonlocal,
                            statement.range(),
                        )?;
                    }
                }
                Stmt::FunctionDef(StmtFunctionDef {
                    name,
                    body,
                    parameters,
                    decorator_list,
                    type_params,
                    returns,
                    range,
                    is_async,
                    ..
                }) => {
                    self.register_name(name.as_str(), SymbolUsage::Assigned, *range)?;

                    self.scan_parameter_defaults(parameters)?;
                    self.scan_decorators(decorator_list, ExpressionContext::Load)?;

                    // For generic functions, enter type_param block FIRST so that
                    // annotation scopes are nested inside and can see type parameters.
                    if let Some(type_params) = type_params {
                        self.enter_type_param_block(
                            name.as_str(),
                            *range,
                            false,
                            Self::has_positional_defaults(parameters),
                            Self::has_kwonlydefaults(parameters),
                        )?;
                        self.scan_type_params(type_params)?;
                    }
                    self.enter_scope_with_parameters(
                        name.as_str(),
                        parameters,
                        self.line_index_start(*range),
                        returns.as_deref(),
                        if *is_async {
                            CompilerScope::AsyncFunction
                        } else {
                            CompilerScope::Function
                        },
                        true, // skip_defaults: already scanned above
                        false,
                    )?;
                    if *is_async {
                        self.tables.last_mut().unwrap().is_coroutine = true;
                    }
                    self.scan_statements(body)?;
                    self.leave_scope();
                    if type_params.is_some() {
                        self.leave_scope();
                    }
                }
                Stmt::ClassDef(StmtClassDef {
                    name,
                    body,
                    arguments,
                    decorator_list,
                    type_params,
                    range,
                    node_index: _,
                    ..
                }) => {
                    let prev_class = self.class_name.clone();
                    self.register_name(name.as_str(), SymbolUsage::Assigned, *range)?;
                    self.scan_decorators(decorator_list, ExpressionContext::Load)?;

                    if let Some(type_params) = type_params {
                        self.enter_type_param_block(
                            name.as_str(),
                            *range,
                            true, // for_class: enable selective mangling
                            false,
                            false,
                        )?;
                        // Set class_name for mangling in type param scope
                        self.class_name = Some(name.to_string());
                        self.scan_type_params(type_params)?;
                    }

                    if type_params.is_none() {
                        self.class_name = prev_class.clone();
                    }
                    if let Some(arguments) = arguments {
                        self.scan_expressions(&arguments.args, ExpressionContext::Load)?;
                        for keyword in &arguments.keywords {
                            if let Some(arg) = &keyword.arg {
                                self.check_name(
                                    arg.as_str(),
                                    ExpressionContext::Store,
                                    keyword.range,
                                )?;
                            }
                        }
                        for keyword in &arguments.keywords {
                            self.scan_expression(&keyword.value, ExpressionContext::Load)?;
                        }
                    }

                    self.enter_scope(
                        name.as_str(),
                        CompilerScope::Class,
                        self.line_index_start(*range),
                    );
                    // Reset in_conditional_block for new class scope
                    let saved_in_conditional = self.in_conditional_block;
                    self.in_conditional_block = false;
                    self.class_name = Some(name.to_string());
                    if type_params.is_some() {
                        self.register_name(".type_params", SymbolUsage::Used, *range)?;
                        self.register_name("__type_params__", SymbolUsage::Assigned, *range)?;
                    }
                    self.scan_statements(body)?;
                    self.leave_scope();
                    self.in_conditional_block = saved_in_conditional;
                    if type_params.is_some() {
                        self.leave_scope();
                    }
                    // Restore class_name after all ClassDef processing
                    self.class_name = prev_class;
                }
                Stmt::Expr(StmtExpr { value, .. }) => {
                    self.scan_expression(value, ExpressionContext::Load)?
                }
                Stmt::If(StmtIf {
                    test,
                    body,
                    elif_else_clauses,
                    ..
                }) => {
                    self.scan_expression(test, ExpressionContext::Load)?;
                    // PEP 649: Track conditional block for annotations
                    let saved_in_conditional_block = self.in_conditional_block;
                    self.in_conditional_block = true;
                    self.scan_statements(body)?;
                    for elif in elif_else_clauses {
                        if let Some(test) = &elif.test {
                            self.scan_expression(test, ExpressionContext::Load)?;
                        }
                        self.scan_statements(&elif.body)?;
                    }
                    self.in_conditional_block = saved_in_conditional_block;
                }
                Stmt::For(StmtFor {
                    target,
                    iter,
                    body,
                    orelse,
                    is_async,
                    ..
                }) => {
                    if *is_async && self.allows_top_level_await() {
                        self.tables.last_mut().unwrap().is_coroutine = true;
                    }
                    if *is_async && !self.tables.last().unwrap().is_coroutine {
                        return Err(SymbolTableError {
                            error: "'async for' outside async function".to_owned(),
                            location: Some(self.source_file.to_source_code().source_location(
                                statement.range().start(),
                                PositionEncoding::Utf8,
                            )),
                        });
                    }
                    self.scan_expression(target, ExpressionContext::Store)?;
                    self.scan_expression(iter, ExpressionContext::Load)?;
                    // PEP 649: Track conditional block for annotations
                    let saved_in_conditional_block = self.in_conditional_block;
                    self.in_conditional_block = true;
                    self.scan_statements(body)?;
                    self.scan_statements(orelse)?;
                    self.in_conditional_block = saved_in_conditional_block;
                }
                Stmt::While(StmtWhile {
                    test, body, orelse, ..
                }) => {
                    self.scan_expression(test, ExpressionContext::Load)?;
                    // PEP 649: Track conditional block for annotations
                    let saved_in_conditional_block = self.in_conditional_block;
                    self.in_conditional_block = true;
                    self.scan_statements(body)?;
                    self.scan_statements(orelse)?;
                    self.in_conditional_block = saved_in_conditional_block;
                }
                Stmt::Break(_) | Stmt::Continue(_) | Stmt::Pass(_) => {
                    // No symbols here.
                }
                Stmt::Import(StmtImport { names, .. })
                | Stmt::ImportFrom(StmtImportFrom { names, .. }) => {
                    for name in names {
                        if let Some(alias) = &name.asname {
                            // `import my_module as my_alias`
                            self.register_name(
                                alias.as_str(),
                                SymbolUsage::Imported,
                                name.name.range,
                            )?;
                        } else if name.name.as_str() == "*" {
                            // Star imports are only allowed at module level
                            if self.tables.last().unwrap().typ != CompilerScope::Module {
                                return Err(SymbolTableError {
                                    error: "import * only allowed at module level".to_string(),
                                    location: Some(
                                        self.source_file.to_source_code().source_location(
                                            name.name.range.start(),
                                            PositionEncoding::Utf8,
                                        ),
                                    ),
                                });
                            }
                            // Don't register star imports as symbols
                        } else {
                            // `import module` or `from x import name`
                            let imported_name = name.name.split('.').next().unwrap();
                            self.check_name(
                                imported_name,
                                ExpressionContext::Store,
                                name.name.range,
                            )?;
                            self.register_name(
                                imported_name,
                                SymbolUsage::Imported,
                                name.name.range,
                            )?;
                        }
                    }
                }
                Stmt::Return(StmtReturn { value, .. }) => {
                    if let Some(expression) = value {
                        self.scan_expression(expression, ExpressionContext::Load)?;
                        self.tables.last_mut().unwrap().returns_value = true;
                    }
                }
                Stmt::Assert(StmtAssert { test, msg, .. }) => {
                    self.scan_expression(test, ExpressionContext::Load)?;
                    if let Some(expression) = msg {
                        self.scan_expression(expression, ExpressionContext::Load)?;
                    }
                }
                Stmt::Delete(StmtDelete { targets, .. }) => {
                    self.scan_expressions(targets, ExpressionContext::Delete)?;
                }
                Stmt::Assign(StmtAssign { targets, value, .. }) => {
                    self.scan_expressions(targets, ExpressionContext::Store)?;
                    self.scan_expression(value, ExpressionContext::Load)?;
                }
                Stmt::AugAssign(StmtAugAssign { target, value, .. }) => {
                    self.scan_expression(target, ExpressionContext::Store)?;
                    self.scan_expression(value, ExpressionContext::Load)?;
                }
                Stmt::AnnAssign(StmtAnnAssign {
                    target,
                    annotation,
                    value,
                    simple,
                    range,
                    node_index: _,
                    ..
                }) => {
                    self.tables.last_mut().unwrap().annotations_used = true;
                    // https://github.com/python/cpython/blob/main/Python/symtable.c#L1233
                    match &**target {
                        Expr::Name(ast::ExprName {
                            id,
                            range: target_range,
                            ..
                        }) => {
                            let id_str = id.as_str();

                            if *simple {
                                let existing_flags = self.tables.last().and_then(|table| {
                                    let name = maybe_mangle_name(
                                        self.class_name.as_deref(),
                                        table.mangled_names.as_ref(),
                                        id_str,
                                    );
                                    table.symbols.get(name.as_ref()).map(|symbol| symbol.flags)
                                });
                                if self
                                    .tables
                                    .last()
                                    .is_some_and(|table| table.typ != CompilerScope::Module)
                                    && let Some(flags) = existing_flags
                                    && flags.intersects(SymbolFlags::GLOBAL | SymbolFlags::NONLOCAL)
                                {
                                    let usage = if flags.contains(SymbolFlags::GLOBAL) {
                                        "global"
                                    } else {
                                        "nonlocal"
                                    };
                                    return Err(SymbolTableError {
                                        error: format!(
                                            "annotated name '{id_str}' can't be {usage}"
                                        ),
                                        location: Some(
                                            self.source_file.to_source_code().source_location(
                                                range.start(),
                                                PositionEncoding::Utf8,
                                            ),
                                        ),
                                    });
                                }

                                self.register_name(
                                    id_str,
                                    SymbolUsage::AnnotationAssigned,
                                    *target_range,
                                )?;
                                // PEP 649: Register annotate function in module/class scope
                                let current_scope = self.tables.last().map(|t| t.typ);
                                match current_scope {
                                    Some(CompilerScope::Module) => {
                                        self.register_name(
                                            "__annotate__",
                                            SymbolUsage::Assigned,
                                            *range,
                                        )?;
                                    }
                                    Some(CompilerScope::Class) => {
                                        self.register_name(
                                            "__annotate_func__",
                                            SymbolUsage::Assigned,
                                            *range,
                                        )?;
                                    }
                                    _ => {}
                                }
                            } else if value.is_some() {
                                self.register_name(id_str, SymbolUsage::Assigned, *target_range)?;
                            }
                        }
                        _ => {
                            self.scan_expression(target, ExpressionContext::Store)?;
                        }
                    }
                    self.scan_ann_assign_annotation(annotation)?;
                    if let Some(value) = value {
                        self.scan_expression(value, ExpressionContext::Load)?;
                    }
                }
                Stmt::With(StmtWith {
                    items,
                    body,
                    is_async,
                    ..
                }) => {
                    if *is_async && self.allows_top_level_await() {
                        self.tables.last_mut().unwrap().is_coroutine = true;
                    }
                    if *is_async && !self.tables.last().unwrap().is_coroutine {
                        return Err(SymbolTableError {
                            error: "'async with' outside async function".to_owned(),
                            location: Some(self.source_file.to_source_code().source_location(
                                statement.range().start(),
                                PositionEncoding::Utf8,
                            )),
                        });
                    }
                    // PEP 649: Track conditional block for annotations
                    let saved_in_conditional_block = self.in_conditional_block;
                    self.in_conditional_block = true;
                    for item in items {
                        self.scan_expression(&item.context_expr, ExpressionContext::Load)?;
                        if let Some(expression) = &item.optional_vars {
                            self.scan_expression(expression, ExpressionContext::Store)?;
                        }
                    }
                    self.scan_statements(body)?;
                    self.in_conditional_block = saved_in_conditional_block;
                }
                Stmt::Try(StmtTry {
                    body,
                    handlers,
                    orelse,
                    finalbody,
                    ..
                }) => {
                    // PEP 649: Track conditional block for annotations
                    let saved_in_conditional_block = self.in_conditional_block;
                    self.in_conditional_block = true;
                    self.scan_statements(body)?;
                    for handler in handlers {
                        let ExceptHandler::ExceptHandler(ast::ExceptHandlerExceptHandler {
                            type_,
                            name,
                            body,
                            ..
                        }) = &handler;
                        if let Some(expression) = type_ {
                            self.scan_expression(expression, ExpressionContext::Load)?;
                        }
                        if let Some(name) = name {
                            self.register_name(
                                name.as_str(),
                                SymbolUsage::Assigned,
                                handler.range(),
                            )?;
                        }
                        self.scan_statements(body)?;
                    }
                    self.scan_statements(orelse)?;
                    self.scan_statements(finalbody)?;
                    self.in_conditional_block = saved_in_conditional_block;
                }
                Stmt::Match(StmtMatch { subject, cases, .. }) => {
                    self.scan_expression(subject, ExpressionContext::Load)?;
                    // PEP 649: Track conditional block for annotations
                    let saved_in_conditional_block = self.in_conditional_block;
                    self.in_conditional_block = true;
                    for case in cases {
                        self.scan_pattern(&case.pattern)?;
                        if let Some(guard) = &case.guard {
                            self.scan_expression(guard, ExpressionContext::Load)?;
                        }
                        self.scan_statements(&case.body)?;
                    }
                    self.in_conditional_block = saved_in_conditional_block;
                }
                Stmt::Raise(StmtRaise { exc, cause, .. }) => {
                    if let Some(expression) = exc {
                        self.scan_expression(expression, ExpressionContext::Load)?;
                        if let Some(expression) = cause {
                            self.scan_expression(expression, ExpressionContext::Load)?;
                        }
                    }
                }
                Stmt::TypeAlias(StmtTypeAlias {
                    name,
                    value,
                    type_params,
                    range,
                    ..
                }) => {
                    let Some(name_expr) = name.as_name_expr() else {
                        return Err(SymbolTableError {
                            error: "type alias expects name".to_owned(),
                            location: Some(
                                self.source_file
                                    .to_source_code()
                                    .source_location(name.range().start(), PositionEncoding::Utf8),
                            ),
                        });
                    };
                    let alias_name = name_expr.id.to_string();
                    self.scan_expression(name, ExpressionContext::Store)?;
                    // Check before entering any sub-scopes
                    let in_class = self
                        .tables
                        .last()
                        .is_some_and(|t| t.typ == CompilerScope::Class);
                    let is_generic = type_params.is_some();
                    if let Some(type_params) = type_params {
                        self.enter_type_param_block(&alias_name, *range, false, false, false)?;
                        self.scan_type_params(type_params)?;
                    }
                    // Value scope for lazy evaluation
                    self.enter_scope(
                        &alias_name,
                        CompilerScope::TypeAlias,
                        self.line_index_start(*range),
                    );
                    // Evaluator takes a format parameter
                    self.register_name(".format", SymbolUsage::Parameter, *range)?;
                    self.register_name(".format", SymbolUsage::Used, *range)?;
                    if in_class {
                        if let Some(table) = self.tables.last_mut() {
                            table.can_see_class_scope = true;
                        }
                        self.register_name("__classdict__", SymbolUsage::Used, value.range())?;
                    }
                    self.scan_expression(value, ExpressionContext::Load)?;
                    self.leave_scope();
                    if is_generic {
                        self.leave_scope();
                    }
                }
                Stmt::IpyEscapeCommand(stmt) => {
                    return Err(SymbolTableError {
                        error: "invalid syntax".to_owned(),
                        location: Some(
                            self.source_file
                                .to_source_code()
                                .source_location(stmt.range.start(), PositionEncoding::Utf8),
                        ),
                    });
                }
            }
            Ok(())
        })();
        self.recursion_depth -= 1;
        result
    }

    fn scan_decorators(
        &mut self,
        decorators: &[ast::Decorator],
        context: ExpressionContext,
    ) -> SymbolTableResult {
        for decorator in decorators {
            self.scan_expression(&decorator.expression, context)?;
        }
        Ok(())
    }

    fn scan_expressions(
        &mut self,
        expressions: &[ast::Expr],
        context: ExpressionContext,
    ) -> SymbolTableResult {
        for expression in expressions {
            self.scan_expression(expression, context)?;
        }
        Ok(())
    }

    fn scan_expression(
        &mut self,
        expression: &ast::Expr,
        context: ExpressionContext,
    ) -> SymbolTableResult {
        if self.recursion_depth >= self.recursion_limit {
            return Err(SymbolTableError {
                error: RECURSION_ERROR.to_owned(),
                location: None,
            });
        }
        self.recursion_depth += 1;
        let result = (|| {
            use ast::*;

            if expression.is_constant_expr() {
                return Ok(());
            }

            // Check for expressions not allowed in certain contexts
            // (type parameters, annotations, type aliases, TypeVar bounds/defaults)
            if let Some(keyword) = match expression {
                Expr::Yield(_) | Expr::YieldFrom(_) => Some("yield"),
                Expr::Await(_) => Some("await"),
                Expr::Named(_) => Some("named"),
                _ => None,
            } {
                // Determine the context name for the error message from the
                // current symbol table entry, matching ste_type checks.
                let current_is_comprehension = self
                    .tables
                    .last()
                    .is_some_and(|table| table.typ == CompilerScope::Comprehension);
                let context_name = if keyword == "named" && current_is_comprehension {
                    None
                } else if let Some(table) = self.tables.last() {
                    match table.typ {
                        CompilerScope::Annotation => Some("an annotation"),
                        CompilerScope::TypeVariable => table.scope_info,
                        CompilerScope::TypeAlias => Some("a type alias"),
                        CompilerScope::TypeParams => Some("the definition of a generic"),
                        _ => None,
                    }
                } else {
                    None
                };

                if let Some(context_name) = context_name {
                    return Err(SymbolTableError {
                        error: format!("{keyword} expression cannot be used within {context_name}"),
                        location: Some(
                            self.source_file.to_source_code().source_location(
                                expression.range().start(),
                                PositionEncoding::Utf8,
                            ),
                        ),
                    });
                }
            }

            match expression {
                Expr::BinOp(ExprBinOp {
                    left,
                    right,
                    range: _,
                    ..
                }) => {
                    self.scan_expression(left, context)?;
                    self.scan_expression(right, context)?;
                }
                Expr::BoolOp(ExprBoolOp {
                    values, range: _, ..
                }) => {
                    self.scan_expressions(values, context)?;
                }
                Expr::Compare(ExprCompare {
                    left,
                    comparators,
                    range: _,
                    ..
                }) => {
                    self.scan_expression(left, context)?;
                    self.scan_expressions(comparators, context)?;
                }
                Expr::Subscript(ExprSubscript {
                    value,
                    slice,
                    range: _,
                    ..
                }) => {
                    self.scan_expression(value, ExpressionContext::Load)?;
                    self.scan_expression(slice, ExpressionContext::Load)?;
                }
                Expr::Attribute(ExprAttribute {
                    value, attr, range, ..
                }) => {
                    self.check_name(attr.as_str(), context, *range)?;
                    self.scan_expression(value, ExpressionContext::Load)?;
                }
                Expr::Dict(ExprDict {
                    items,
                    node_index: _,
                    range: _,
                    ..
                }) => {
                    for item in items {
                        if let Some(key) = &item.key {
                            self.scan_expression(key, context)?;
                        }
                    }
                    for item in items {
                        self.scan_expression(&item.value, context)?;
                    }
                }
                Expr::Await(ExprAwait {
                    value,
                    node_index: _,
                    range: _,
                    ..
                }) => {
                    let current_scope = self.tables.last().unwrap().typ;
                    if !self.allows_top_level_await()
                        && !Self::is_function_like_scope(current_scope)
                    {
                        return Err(SymbolTableError {
                            error: "'await' outside function".to_owned(),
                            location: Some(self.source_file.to_source_code().source_location(
                                expression.range().start(),
                                PositionEncoding::Utf8,
                            )),
                        });
                    }
                    if current_scope != CompilerScope::AsyncFunction
                        && current_scope != CompilerScope::Comprehension
                        && !self.allows_top_level_await()
                    {
                        return Err(SymbolTableError {
                            error: "'await' outside async function".to_owned(),
                            location: Some(self.source_file.to_source_code().source_location(
                                expression.range().start(),
                                PositionEncoding::Utf8,
                            )),
                        });
                    }
                    self.scan_expression(value, context)?;
                    self.tables.last_mut().unwrap().is_coroutine = true;
                }
                Expr::Yield(ExprYield {
                    value,
                    node_index: _,
                    range: _,
                    ..
                }) => {
                    if let Some(expression) = value {
                        self.scan_expression(expression, context)?;
                    }
                    self.tables.last_mut().unwrap().is_generator = true;
                    if let Some(context_name) = self.comprehension_yield_context
                        && self
                            .tables
                            .last()
                            .is_some_and(|table| table.typ == CompilerScope::Comprehension)
                    {
                        return Err(SymbolTableError {
                            error: format!("'yield' inside {context_name}"),
                            location: Some(self.source_file.to_source_code().source_location(
                                expression.range().start(),
                                PositionEncoding::Utf8,
                            )),
                        });
                    }
                }
                Expr::YieldFrom(ExprYieldFrom {
                    value,
                    node_index: _,
                    range: _,
                    ..
                }) => {
                    self.scan_expression(value, context)?;
                    self.tables.last_mut().unwrap().is_generator = true;
                    if let Some(context_name) = self.comprehension_yield_context
                        && self
                            .tables
                            .last()
                            .is_some_and(|table| table.typ == CompilerScope::Comprehension)
                    {
                        return Err(SymbolTableError {
                            error: format!("'yield' inside {context_name}"),
                            location: Some(self.source_file.to_source_code().source_location(
                                expression.range().start(),
                                PositionEncoding::Utf8,
                            )),
                        });
                    }
                }
                Expr::UnaryOp(ExprUnaryOp {
                    operand, range: _, ..
                }) => {
                    self.scan_expression(operand, context)?;
                }
                Expr::Starred(ExprStarred {
                    value, range: _, ..
                }) => {
                    self.scan_expression(value, context)?;
                }
                Expr::Tuple(ExprTuple { elts, range: _, .. })
                | Expr::Set(ExprSet { elts, range: _, .. })
                | Expr::List(ExprList { elts, range: _, .. }) => {
                    self.scan_expressions(elts, context)?;
                }
                Expr::Slice(ExprSlice {
                    lower,
                    upper,
                    step,
                    node_index: _,
                    range: _,
                    ..
                }) => {
                    if let Some(lower) = lower {
                        self.scan_expression(lower, context)?;
                    }
                    if let Some(upper) = upper {
                        self.scan_expression(upper, context)?;
                    }
                    if let Some(step) = step {
                        self.scan_expression(step, context)?;
                    }
                }
                Expr::Generator(ExprGenerator {
                    elt,
                    generators,
                    range,
                    ..
                }) => {
                    let was_in_iter_def_exp = self.in_iter_def_exp;
                    if context == ExpressionContext::IterDefinitionExp {
                        self.in_iter_def_exp = true;
                    }
                    // Generator expression - is_generator = true
                    self.scan_comprehension("<genexpr>", elt, None, generators, *range, true)?;
                    self.in_iter_def_exp = was_in_iter_def_exp;
                }
                Expr::ListComp(ExprListComp {
                    elt,
                    generators,
                    range,
                    node_index: _,
                    ..
                }) => {
                    let was_in_iter_def_exp = self.in_iter_def_exp;
                    if context == ExpressionContext::IterDefinitionExp {
                        self.in_iter_def_exp = true;
                    }
                    // List comprehension - is_generator = false (can be inlined)
                    self.scan_comprehension("<listcomp>", elt, None, generators, *range, false)?;
                    self.in_iter_def_exp = was_in_iter_def_exp;
                }
                Expr::SetComp(ExprSetComp {
                    elt,
                    generators,
                    range,
                    node_index: _,
                    ..
                }) => {
                    let was_in_iter_def_exp = self.in_iter_def_exp;
                    if context == ExpressionContext::IterDefinitionExp {
                        self.in_iter_def_exp = true;
                    }
                    // Set comprehension - is_generator = false (can be inlined)
                    self.scan_comprehension("<setcomp>", elt, None, generators, *range, false)?;
                    self.in_iter_def_exp = was_in_iter_def_exp;
                }
                Expr::DictComp(ExprDictComp {
                    key,
                    value,
                    generators,
                    range,
                    node_index: _,
                    ..
                }) => {
                    let was_in_iter_def_exp = self.in_iter_def_exp;
                    if context == ExpressionContext::IterDefinitionExp {
                        self.in_iter_def_exp = true;
                    }
                    // Dict comprehension - is_generator = false (can be inlined)
                    let key = key.as_ref();
                    self.scan_comprehension(
                        "<dictcomp>",
                        key,
                        Some(value),
                        generators,
                        *range,
                        false,
                    )?;
                    self.in_iter_def_exp = was_in_iter_def_exp;
                }
                Expr::Call(ExprCall {
                    func,
                    arguments,
                    node_index: _,
                    range: _,
                    ..
                }) => {
                    match context {
                        ExpressionContext::IterDefinitionExp => {
                            self.scan_expression(func, ExpressionContext::IterDefinitionExp)?;
                        }
                        _ => {
                            self.scan_expression(func, ExpressionContext::Load)?;
                        }
                    }

                    self.scan_expressions(&arguments.args, ExpressionContext::Load)?;
                    for keyword in &arguments.keywords {
                        if let Some(arg) = &keyword.arg {
                            self.check_name(arg.as_str(), ExpressionContext::Store, keyword.range)?;
                        }
                    }
                    for keyword in &arguments.keywords {
                        self.scan_expression(&keyword.value, ExpressionContext::Load)?;
                    }
                }
                Expr::Name(ExprName { id, range, .. }) => {
                    let id = id.as_str();

                    self.check_name(id, context, *range)?;

                    if !self
                        .tables
                        .last()
                        .is_some_and(|table| table.in_unevaluated_annotation)
                    {
                        // Determine the contextual usage of this symbol:
                        match context {
                            ExpressionContext::Delete => {
                                self.register_name(id, SymbolUsage::Assigned, *range)?;
                            }
                            ExpressionContext::Load | ExpressionContext::IterDefinitionExp => {
                                self.register_name(id, SymbolUsage::Used, *range)?;
                            }
                            ExpressionContext::Store => {
                                self.register_name(id, SymbolUsage::Assigned, *range)?;
                            }
                            ExpressionContext::Iter => {
                                self.register_name(id, SymbolUsage::Iter, *range)?;
                            }
                        }
                        // Interesting stuff about the __class__ variable:
                        // https://docs.python.org/3/reference/datamodel.html?highlight=__class__#creating-the-class-object
                        if context == ExpressionContext::Load
                            && Self::is_function_like_scope(self.tables.last().unwrap().typ)
                            && id == "super"
                        {
                            self.register_name("__class__", SymbolUsage::Used, *range)?;
                        }
                    }
                }
                Expr::Lambda(ExprLambda {
                    body,
                    parameters,
                    node_index: _,
                    range: _,
                    ..
                }) => {
                    let was_in_iter_def_exp = self.in_iter_def_exp;
                    if let Some(parameters) = parameters {
                        if was_in_iter_def_exp {
                            self.scan_parameter_defaults(parameters)?;
                        }
                        self.enter_scope_with_parameters(
                            "lambda",
                            parameters,
                            self.line_index_start(expression.range()),
                            None, // lambdas have no return annotation
                            CompilerScope::Lambda,
                            was_in_iter_def_exp,
                            false,
                        )?;
                    } else {
                        self.enter_scope(
                            "lambda",
                            CompilerScope::Lambda,
                            self.line_index_start(expression.range()),
                        );
                    }
                    self.scan_expression(body, ExpressionContext::Load)?;
                    self.in_iter_def_exp = was_in_iter_def_exp;
                    self.leave_scope();
                }
                Expr::FString(fstring) => {
                    if let Some(joined_str) = &fstring.runtime_joined_str {
                        for expr in joined_str {
                            self.scan_expression(expr, ExpressionContext::Load)?;
                        }
                        return Ok(());
                    }
                    for expr in fstring
                        .value
                        .elements()
                        .filter_map(|x| x.as_interpolation())
                    {
                        self.scan_expression(&expr.expression, ExpressionContext::Load)?;
                        if let Some(format_spec) = &expr.runtime_formatted_value_format_spec {
                            self.scan_expression(format_spec, ExpressionContext::Load)?;
                        } else if let Some(format_spec) = &expr.format_spec {
                            for element in format_spec.elements.interpolations() {
                                self.scan_expression(&element.expression, ExpressionContext::Load)?
                            }
                        }
                    }
                }
                Expr::TString(tstring) => {
                    if let Some(template_str) = &tstring.runtime_template_str {
                        for expr in template_str {
                            self.scan_expression(expr, ExpressionContext::Load)?;
                        }
                        return Ok(());
                    }
                    // Scan t-string interpolation expressions (similar to f-strings)
                    for expr in tstring
                        .value
                        .elements()
                        .filter_map(|x| x.as_interpolation())
                    {
                        self.scan_expression(&expr.expression, ExpressionContext::Load)?;
                        if expr.runtime_str.is_some() {
                            if let Some(format_spec) = &expr.runtime_interpolation_format_spec {
                                self.scan_expression(format_spec, ExpressionContext::Load)?;
                            }
                        } else if let Some(format_spec) = &expr.format_spec {
                            for element in format_spec.elements.interpolations() {
                                self.scan_expression(&element.expression, ExpressionContext::Load)?
                            }
                        }
                    }
                }
                // Constants
                Expr::StringLiteral(_)
                | Expr::BytesLiteral(_)
                | Expr::NumberLiteral(_)
                | Expr::Constant(_)
                | Expr::BooleanLiteral(_)
                | Expr::NoneLiteral(_)
                | Expr::EllipsisLiteral(_) => {}
                Expr::IpyEscapeCommand(expr) => {
                    return Err(SymbolTableError {
                        error: "invalid syntax".to_owned(),
                        location: Some(
                            self.source_file
                                .to_source_code()
                                .source_location(expr.range.start(), PositionEncoding::Utf8),
                        ),
                    });
                }
                Expr::If(ExprIf {
                    test,
                    body,
                    orelse,
                    node_index: _,
                    range: _,
                    ..
                }) => {
                    self.scan_expression(test, ExpressionContext::Load)?;
                    self.scan_expression(body, ExpressionContext::Load)?;
                    self.scan_expression(orelse, ExpressionContext::Load)?;
                }

                Expr::Named(ExprNamed {
                    target,
                    value,
                    range,
                    node_index: _,
                    ..
                }) => {
                    // named expressions are not allowed in the definition of
                    // comprehension iterator definitions (including nested comprehensions)
                    if context == ExpressionContext::IterDefinitionExp || self.in_iter_def_exp {
                        return Err(SymbolTableError {
                        error:
                            "assignment expression cannot be used in a comprehension iterable expression"
                                .to_string(),
                        location: Some(
                            self.source_file
                                .to_source_code()
                                .source_location(range.start(), PositionEncoding::Utf8),
                        ),
                    });
                    }

                    let named_target = if let Expr::Name(ExprName {
                        id,
                        range: target_range,
                        ..
                    }) = &**target
                    {
                        let id = id.as_str();
                        self.check_name(id, ExpressionContext::Store, *target_range)?;
                        let table = self.tables.last().unwrap();
                        if table.typ == CompilerScope::Comprehension {
                            self.extend_namedexpr_scope(id, *target_range)?;
                        }
                        Some((id, *target_range))
                    } else {
                        None
                    };

                    self.scan_expression(value, ExpressionContext::Load)?;

                    // special handling for assigned identifier in named expressions
                    // that are used in comprehensions. This required to correctly
                    // propagate the scope of the named assigned named and not to
                    // propagate inner names.
                    if let Some((id, target_range)) = named_target {
                        let table = self.tables.last().unwrap();
                        if table.typ == CompilerScope::Comprehension {
                            self.register_name(
                                id,
                                SymbolUsage::AssignedNamedExprInComprehension,
                                target_range,
                            )?;
                        } else {
                            // omit one recursion. When the handling of an store changes for
                            // Identifiers this needs adapted - more forward safe would be
                            // calling scan_expression directly.
                            self.register_name(id, SymbolUsage::Assigned, target_range)?;
                        }
                    } else {
                        self.scan_expression(target, ExpressionContext::Store)?;
                    }
                }
            }
            Ok(())
        })();
        self.recursion_depth -= 1;
        result
    }

    fn scan_comprehension(
        &mut self,
        scope_name: &str,
        elt1: &ast::Expr,
        elt2: Option<&ast::Expr>,
        generators: &[ast::Comprehension],
        range: TextRange,
        is_generator: bool,
    ) -> SymbolTableResult {
        assert!(!generators.is_empty());
        let outermost = &generators[0];

        // CPython evaluates the outermost iterator in the enclosing scope
        // before entering the comprehension scope.
        let was_in_iter_def_exp = self.in_iter_def_exp;
        self.in_iter_def_exp = true;
        self.scan_expression(&outermost.iter, ExpressionContext::IterDefinitionExp)?;
        self.in_iter_def_exp = was_in_iter_def_exp;

        // Comprehensions are compiled as functions, so create a scope for them:
        self.enter_scope(
            scope_name,
            CompilerScope::Comprehension,
            self.line_index_start(range),
        );
        if outermost.is_async {
            self.tables.last_mut().unwrap().is_coroutine = true;
        }

        // PEP 709: Mark non-generator comprehensions for inlining.
        // symtable marks all non-generator comprehensions for
        // inlining, except scopes nested under a parent that can see class
        // scope (for example annotation scopes inside classes).
        if !is_generator {
            let parent = self.tables.iter().rev().nth(1);
            let parent_can_see_class = parent.is_some_and(|t| t.can_see_class_scope);
            if !parent_can_see_class {
                self.tables.last_mut().unwrap().comp_inlined = true;
            }
        }

        // Register the passed argument to the generator function as the name ".0"
        self.register_name(".0", SymbolUsage::Parameter, range)?;

        let saved_comprehension_yield_context = self.comprehension_yield_context;
        self.comprehension_yield_context = Some(match scope_name {
            "<listcomp>" => "list comprehension",
            "<setcomp>" => "set comprehension",
            "<dictcomp>" => "dict comprehension",
            "<genexpr>" => "generator expression",
            _ => "comprehension",
        });

        self.scan_expression(&outermost.target, ExpressionContext::Iter)?;
        for if_expr in &outermost.ifs {
            self.scan_expression(if_expr, ExpressionContext::Load)?;
        }

        for generator in &generators[1..] {
            self.in_comp_inner_loop_target = true;
            self.scan_expression(&generator.target, ExpressionContext::Iter)?;
            self.in_comp_inner_loop_target = false;
            let was_in_iter_def_exp = self.in_iter_def_exp;
            self.in_iter_def_exp = true;
            self.scan_expression(&generator.iter, ExpressionContext::IterDefinitionExp)?;
            self.in_iter_def_exp = was_in_iter_def_exp;
            for if_expr in &generator.ifs {
                self.scan_expression(if_expr, ExpressionContext::Load)?;
            }
            if generator.is_async {
                self.tables.last_mut().unwrap().is_coroutine = true;
            }
        }

        if let Some(elt2) = elt2 {
            self.scan_expression(elt2, ExpressionContext::Load)?;
        }
        self.scan_expression(elt1, ExpressionContext::Load)?;
        self.tables.last_mut().unwrap().is_generator = is_generator;
        self.comprehension_yield_context = saved_comprehension_yield_context;

        // symtable_handle_comprehension(): non-generator async
        // comprehensions propagate ste_coroutine to the enclosing scope after
        // the comprehension block is exited.
        let propagate_coroutine = self.tables.last().unwrap().is_coroutine && !is_generator;
        self.leave_scope();
        if propagate_coroutine
            && self
                .tables
                .last()
                .is_none_or(|table| table.typ != CompilerScope::Comprehension)
            && !self.is_in_async_context()
            && !self.allows_top_level_await()
        {
            return Err(SymbolTableError {
                error: "asynchronous comprehension outside of an asynchronous function".to_owned(),
                location: Some(
                    self.source_file
                        .to_source_code()
                        .source_location(range.start(), PositionEncoding::Utf8),
                ),
            });
        }
        if propagate_coroutine {
            self.tables.last_mut().unwrap().is_coroutine = true;
        }

        Ok(())
    }

    /// Scan type parameter bound or default in a separate scope
    // = symtable_visit_type_param_bound_or_default
    fn scan_type_param_bound_or_default(
        &mut self,
        expr: &ast::Expr,
        scope_name: &str,
        scope_info: &'static str,
    ) -> SymbolTableResult {
        // Bounds/defaults are compiled as annotation scopes.
        let in_class = self.tables.last().is_some_and(|t| t.can_see_class_scope);
        let line_number = self.line_index_start(expr.range());
        self.enter_scope(scope_name, CompilerScope::TypeVariable, line_number);
        // Evaluator takes a format parameter
        self.register_name(".format", SymbolUsage::Parameter, expr.range())?;
        self.register_name(".format", SymbolUsage::Used, expr.range())?;

        if in_class {
            if let Some(table) = self.tables.last_mut() {
                table.can_see_class_scope = true;
            }
            self.register_name("__classdict__", SymbolUsage::Used, expr.range())?;
        }

        self.tables.last_mut().unwrap().scope_info = Some(scope_info);

        // Scan the expression in this new scope
        let result = self.scan_expression(expr, ExpressionContext::Load);

        self.leave_scope();

        result
    }

    fn scan_type_params(&mut self, type_params: &ast::TypeParams) -> SymbolTableResult {
        // Each type parameter is visited as: register name, scan bound, scan default.
        for type_param in &type_params.type_params {
            if self.recursion_depth >= self.recursion_limit {
                return Err(SymbolTableError {
                    error: RECURSION_ERROR.to_owned(),
                    location: None,
                });
            }
            self.recursion_depth += 1;
            let result = (|| {
                match type_param {
                    ast::TypeParam::TypeVar(ast::TypeParamTypeVar {
                        name,
                        bound,
                        range: type_var_range,
                        default,
                        node_index: _,
                        ..
                    }) => {
                        self.register_name(name.as_str(), SymbolUsage::TypeParam, *type_var_range)?;
                        if name.as_str() == "__classdict__" {
                            return Err(SymbolTableError {
                                error: format!(
                                    "reserved name '{}' cannot be used for type parameter",
                                    name.as_str()
                                ),
                                location: Some(self.source_file.to_source_code().source_location(
                                    type_var_range.start(),
                                    PositionEncoding::Utf8,
                                )),
                            });
                        }

                        // Process bound in a separate scope
                        if let Some(binding) = bound {
                            let scope_info = if binding.is_tuple_expr() {
                                "a TypeVar constraint"
                            } else {
                                "a TypeVar bound"
                            };
                            self.scan_type_param_bound_or_default(
                                binding,
                                name.as_str(),
                                scope_info,
                            )?;
                        }

                        // Process default in a separate scope
                        if let Some(default_value) = default {
                            self.scan_type_param_bound_or_default(
                                default_value,
                                name.as_str(),
                                "a TypeVar default",
                            )?;
                        }
                    }
                    ast::TypeParam::ParamSpec(ast::TypeParamParamSpec {
                        name,
                        range: param_spec_range,
                        default,
                        node_index: _,
                        ..
                    }) => {
                        self.register_name(name, SymbolUsage::TypeParam, *param_spec_range)?;
                        if name == "__classdict__" {
                            return Err(SymbolTableError {
                                error: format!(
                                    "reserved name '{name}' cannot be used for type parameter"
                                ),
                                location: Some(self.source_file.to_source_code().source_location(
                                    param_spec_range.start(),
                                    PositionEncoding::Utf8,
                                )),
                            });
                        }

                        // Process default in a separate scope
                        if let Some(default_value) = default {
                            self.scan_type_param_bound_or_default(
                                default_value,
                                name,
                                "a ParamSpec default",
                            )?;
                        }
                    }
                    ast::TypeParam::TypeVarTuple(ast::TypeParamTypeVarTuple {
                        name,
                        range: type_var_tuple_range,
                        default,
                        node_index: _,
                        ..
                    }) => {
                        self.register_name(name, SymbolUsage::TypeParam, *type_var_tuple_range)?;
                        if name == "__classdict__" {
                            return Err(SymbolTableError {
                                error: format!(
                                    "reserved name '{name}' cannot be used for type parameter"
                                ),
                                location: Some(self.source_file.to_source_code().source_location(
                                    type_var_tuple_range.start(),
                                    PositionEncoding::Utf8,
                                )),
                            });
                        }

                        // Process default in a separate scope
                        if let Some(default_value) = default {
                            self.scan_type_param_bound_or_default(
                                default_value,
                                name,
                                "a TypeVarTuple default",
                            )?;
                        }
                    }
                }
                Ok(())
            })();
            self.recursion_depth -= 1;
            result?;
        }
        Ok(())
    }

    fn scan_patterns(&mut self, patterns: &[ast::Pattern]) -> SymbolTableResult {
        for pattern in patterns {
            self.scan_pattern(pattern)?;
        }
        Ok(())
    }

    fn scan_pattern(&mut self, pattern: &ast::Pattern) -> SymbolTableResult {
        if self.recursion_depth >= self.recursion_limit {
            return Err(SymbolTableError {
                error: RECURSION_ERROR.to_owned(),
                location: None,
            });
        }
        self.recursion_depth += 1;
        let result = (|| {
            use ast::Pattern::{
                MatchAs, MatchClass, MatchMapping, MatchOr, MatchSequence, MatchSingleton,
                MatchStar, MatchValue,
            };
            match pattern {
                MatchValue(ast::PatternMatchValue { value, .. }) => {
                    self.scan_expression(value, ExpressionContext::Load)?
                }
                MatchSingleton(_) => {}
                MatchSequence(ast::PatternMatchSequence { patterns, .. }) => {
                    self.scan_patterns(patterns)?
                }
                MatchMapping(ast::PatternMatchMapping {
                    keys,
                    patterns,
                    rest,
                    ..
                }) => {
                    self.scan_expressions(keys, ExpressionContext::Load)?;
                    self.scan_patterns(patterns)?;
                    if let Some(rest) = rest {
                        if rest.as_str() == "_" {
                            return Err(SymbolTableError {
                                error: "invalid syntax".to_owned(),
                                location: Some(
                                    self.source_file.to_source_code().source_location(
                                        rest.range.start(),
                                        PositionEncoding::Utf8,
                                    ),
                                ),
                            });
                        }
                        self.register_name(rest.as_str(), SymbolUsage::Assigned, pattern.range())?;
                    }
                }
                MatchClass(ast::PatternMatchClass { cls, arguments, .. }) => {
                    self.scan_expression(cls, ExpressionContext::Load)?;
                    self.scan_patterns(&arguments.patterns)?;
                    for kw in &arguments.keywords {
                        self.check_name(
                            kw.attr.as_str(),
                            ExpressionContext::Store,
                            kw.pattern.range(),
                        )?;
                    }
                    for kw in &arguments.keywords {
                        self.scan_pattern(&kw.pattern)?;
                    }
                }
                MatchStar(ast::PatternMatchStar { name, .. }) => {
                    if let Some(name) = name {
                        self.register_name(name.as_str(), SymbolUsage::Assigned, pattern.range())?;
                    }
                }
                MatchAs(ast::PatternMatchAs {
                    pattern: as_pattern,
                    name,
                    ..
                }) => {
                    if let Some(as_pattern) = as_pattern {
                        self.scan_pattern(as_pattern)?;
                    }
                    if let Some(name) = name {
                        self.register_name(name.as_str(), SymbolUsage::Assigned, pattern.range())?;
                    }
                }
                MatchOr(ast::PatternMatchOr { patterns, .. }) => self.scan_patterns(patterns)?,
            }
            Ok(())
        })();
        self.recursion_depth -= 1;
        result
    }

    /// Scan default parameter values (evaluated in the enclosing scope)
    fn scan_parameter_defaults(&mut self, parameters: &ast::Parameters) -> SymbolTableResult {
        for default in parameters
            .posonlyargs
            .iter()
            .chain(parameters.args.iter())
            .chain(parameters.kwonlyargs.iter())
            .filter_map(|arg| arg.default.as_ref())
        {
            self.scan_expression(default, ExpressionContext::Load)?;
        }
        Ok(())
    }

    fn has_kwonlydefaults(parameters: &ast::Parameters) -> bool {
        parameters
            .kwonlyargs
            .iter()
            .any(|arg| arg.default.is_some())
    }

    fn has_positional_defaults(parameters: &ast::Parameters) -> bool {
        parameters
            .posonlyargs
            .iter()
            .chain(parameters.args.iter())
            .any(|arg| arg.default.is_some())
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "keeps parameter/default scanning options explicit at call sites"
    )]
    fn enter_scope_with_parameters(
        &mut self,
        name: &str,
        parameters: &ast::Parameters,
        line_number: u32,
        returns: Option<&ast::Expr>,
        scope_type: CompilerScope,
        skip_defaults: bool,
        skip_annotations: bool,
    ) -> SymbolTableResult {
        // Evaluate eventual default parameters (unless already scanned before type_param_block):
        if !skip_defaults {
            self.scan_parameter_defaults(parameters)?;
        }

        let is_function_scope = matches!(
            scope_type,
            CompilerScope::Function | CompilerScope::AsyncFunction
        );
        if is_function_scope && !skip_annotations {
            self.scan_function_annotations(parameters, returns, line_number)?;
        }

        self.enter_scope(name, scope_type, line_number);

        // Fill scope with parameter names:
        self.scan_parameters(&parameters.posonlyargs)?;
        self.scan_parameters(&parameters.args)?;
        self.scan_parameters(&parameters.kwonlyargs)?;
        if let Some(name) = &parameters.vararg {
            self.scan_parameter(name)?;
        }
        if let Some(name) = &parameters.kwarg {
            self.scan_parameter(name)?;
        }
        Ok(())
    }

    fn register_ident(&mut self, ident: &ast::Identifier, role: SymbolUsage) -> SymbolTableResult {
        self.register_name(ident.as_str(), role, ident.range)
    }

    fn check_name(
        &self,
        name: &str,
        context: ExpressionContext,
        range: TextRange,
    ) -> SymbolTableResult {
        if name == "__debug__" {
            let location = Some(
                self.source_file
                    .to_source_code()
                    .source_location(range.start(), PositionEncoding::Utf8),
            );
            match context {
                ExpressionContext::Store | ExpressionContext::Iter => {
                    return Err(SymbolTableError {
                        error: "cannot assign to __debug__".to_owned(),
                        location,
                    });
                }
                ExpressionContext::Delete => {
                    return Err(SymbolTableError {
                        error: "cannot delete __debug__".to_owned(),
                        location,
                    });
                }
                _ => {}
            }
        }
        Ok(())
    }

    // Mirrors symtable_extend_namedexpr_scope(): assignment expressions
    // inside comprehensions bind in the nearest function/module-like scope, not
    // in the synthetic comprehension scope itself.
    fn extend_namedexpr_scope(&mut self, name: &str, range: TextRange) -> SymbolTableResult {
        let location = Some(
            self.source_file
                .to_source_code()
                .source_location(range.start(), PositionEncoding::Utf8),
        );

        for table_idx in (0..self.tables.len()).rev() {
            let table_type = self.tables[table_idx].typ;
            let mangled = maybe_mangle_name(
                self.class_name.as_deref(),
                self.tables[table_idx].mangled_names.as_ref(),
                name,
            )
            .into_owned();

            if table_type == CompilerScope::Comprehension {
                if self.tables[table_idx]
                    .symbols
                    .get(mangled.as_str())
                    .is_some_and(|symbol| symbol.flags.contains(SymbolFlags::ITER))
                {
                    return Err(SymbolTableError {
                        error: format!(
                            "assignment expression cannot rebind comprehension iteration variable '{mangled}'"
                        ),
                        location,
                    });
                }
                continue;
            }

            match table_type {
                CompilerScope::Function | CompilerScope::AsyncFunction | CompilerScope::Lambda => {
                    let parent_is_global = self.tables[table_idx]
                        .symbols
                        .get(mangled.as_str())
                        .is_some_and(|symbol| symbol.flags.contains(SymbolFlags::GLOBAL));
                    let current = self.tables.last_mut().unwrap();
                    let current_symbol = current
                        .symbols
                        .entry(mangled.clone())
                        .or_insert_with(|| Symbol::new(mangled.as_str()));
                    if parent_is_global {
                        current_symbol.flags.insert(SymbolFlags::GLOBAL);
                        current_symbol.scope = SymbolScope::GlobalExplicit;
                    } else {
                        current_symbol.flags.insert(SymbolFlags::NONLOCAL);
                        current_symbol.scope = SymbolScope::Free;
                    }

                    let symbol = self.tables[table_idx]
                        .symbols
                        .entry(mangled.clone())
                        .or_insert_with(|| Symbol::new(mangled.as_str()));
                    symbol.flags.insert(SymbolFlags::ASSIGNED);
                    return Ok(());
                }
                CompilerScope::Module => {
                    let current = self.tables.last_mut().unwrap();
                    let current_symbol = current
                        .symbols
                        .entry(mangled.clone())
                        .or_insert_with(|| Symbol::new(mangled.as_str()));
                    current_symbol.flags.insert(SymbolFlags::GLOBAL);
                    current_symbol.scope = SymbolScope::GlobalExplicit;

                    let symbol = self.tables[table_idx]
                        .symbols
                        .entry(mangled.clone())
                        .or_insert_with(|| Symbol::new(mangled.as_str()));
                    symbol.flags.insert(SymbolFlags::GLOBAL);
                    symbol.scope = SymbolScope::GlobalExplicit;
                    return Ok(());
                }
                CompilerScope::Class => {
                    return Err(SymbolTableError {
                        error: "assignment expression within a comprehension cannot be used in a class body".to_string(),
                        location,
                    });
                }
                CompilerScope::TypeParams => {
                    return Err(SymbolTableError {
                        error: "assignment expression within a comprehension cannot be used within the definition of a generic".to_string(),
                        location,
                    });
                }
                CompilerScope::TypeAlias => {
                    return Err(SymbolTableError {
                        error:
                            "assignment expression within a comprehension cannot be used in a type alias"
                                .to_string(),
                        location,
                    });
                }
                CompilerScope::TypeVariable => {
                    return Err(SymbolTableError {
                        error:
                            "assignment expression within a comprehension cannot be used in a TypeVar bound"
                                .to_string(),
                        location,
                    });
                }
                CompilerScope::Annotation => {}
                CompilerScope::Comprehension => unreachable!(),
            }
        }

        unreachable!("named expression scope extension requires an enclosing scope")
    }

    fn register_name(
        &mut self,
        name: &str,
        role: SymbolUsage,
        range: TextRange,
    ) -> SymbolTableResult {
        let location = self
            .source_file
            .to_source_code()
            .source_location(range.start(), PositionEncoding::Utf8);
        let location = Some(location);

        // symtable_add_def_ctx() runs check_name() for definition
        // roles covered by DEF_PARAM | DEF_LOCAL | DEF_IMPORT before adding
        // the symbol. Several Rust callers reach register_name() directly
        // instead of going through scan_expression(Name), so keep the guard here.
        if matches!(
            role,
            SymbolUsage::Assigned
                | SymbolUsage::Imported
                | SymbolUsage::AnnotationAssigned
                | SymbolUsage::Parameter
                | SymbolUsage::AnnotationParameter
                | SymbolUsage::AssignedNamedExprInComprehension
                | SymbolUsage::Iter
                | SymbolUsage::TypeParam
        ) {
            self.check_name(name, ExpressionContext::Store, range)?;
        }

        let scope_depth = self.tables.len();
        let table = self.tables.last_mut().unwrap();
        let current_scope = table.typ;

        // Add type param names to mangled_names set for selective mangling
        if matches!(role, SymbolUsage::TypeParam)
            && let Some(ref mut set) = table.mangled_names
        {
            set.insert(name.to_owned());
        }

        let original_name = name;
        let name = maybe_mangle_name(
            self.class_name.as_deref(),
            table.mangled_names.as_ref(),
            name,
        );
        // Some checks for the symbol that present on this scope level:
        let symbol = if let Some(symbol) = table.symbols.get_mut(name.as_ref()) {
            let flags = &symbol.flags;

            // INNER_LOOP_CONFLICT: comprehension inner loop cannot rebind
            // a variable that was used as a named expression target
            // Example: [i for i in range(5) if (j := 0) for j in range(5)]
            // Here 'j' is used in named expr first, then as inner loop iter target
            if self.in_comp_inner_loop_target
                && flags.contains(SymbolFlags::ASSIGNED_IN_COMPREHENSION)
            {
                return Err(SymbolTableError {
                    error: format!(
                        "comprehension inner loop cannot rebind assignment expression target '{name}'"
                    ),
                    location,
                });
            }

            if matches!(
                role,
                SymbolUsage::Parameter | SymbolUsage::AnnotationParameter
            ) && flags.contains(SymbolFlags::PARAMETER)
            {
                return Err(SymbolTableError {
                    error: format!("duplicate argument '{original_name}' in function definition"),
                    location,
                });
            }

            // Role already set..
            if matches!(role, SymbolUsage::TypeParam) && flags.contains(SymbolFlags::TYPE_PARAM) {
                return Err(SymbolTableError {
                    error: format!("duplicate type parameter '{name}'"),
                    location,
                });
            }
            match role {
                SymbolUsage::Global if !symbol.is_global() => {
                    if flags.contains(SymbolFlags::PARAMETER) {
                        return Err(SymbolTableError {
                            error: format!("name '{name}' is parameter and global"),
                            location,
                        });
                    }
                    if flags.contains(SymbolFlags::REFERENCED) {
                        return Err(SymbolTableError {
                            error: format!("name '{name}' is used prior to global declaration"),
                            location,
                        });
                    }
                    if flags.contains(SymbolFlags::ANNOTATED) {
                        return Err(SymbolTableError {
                            error: format!("annotated name '{name}' can't be global"),
                            location,
                        });
                    }
                    if flags.contains(SymbolFlags::ASSIGNED) {
                        return Err(SymbolTableError {
                            error: format!(
                                "name '{name}' is assigned to before global declaration"
                            ),
                            location,
                        });
                    }
                }
                SymbolUsage::Nonlocal => {
                    if flags.contains(SymbolFlags::PARAMETER) {
                        return Err(SymbolTableError {
                            error: format!("name '{name}' is parameter and nonlocal"),
                            location,
                        });
                    }
                    if flags.contains(SymbolFlags::REFERENCED) {
                        return Err(SymbolTableError {
                            error: format!("name '{name}' is used prior to nonlocal declaration"),
                            location,
                        });
                    }
                    if flags.contains(SymbolFlags::ANNOTATED) {
                        return Err(SymbolTableError {
                            error: format!("annotated name '{name}' can't be nonlocal"),
                            location,
                        });
                    }
                    if flags.contains(SymbolFlags::ASSIGNED) {
                        return Err(SymbolTableError {
                            error: format!(
                                "name '{name}' is assigned to before nonlocal declaration"
                            ),
                            location,
                        });
                    }
                }
                SymbolUsage::AnnotationAssigned
                    if current_scope != CompilerScope::Module
                        && flags.intersects(SymbolFlags::GLOBAL | SymbolFlags::NONLOCAL) =>
                {
                    let usage = if flags.contains(SymbolFlags::GLOBAL) {
                        "global"
                    } else {
                        "nonlocal"
                    };
                    return Err(SymbolTableError {
                        error: format!("annotated name '{name}' can't be {usage}"),
                        location,
                    });
                }
                _ => {
                    // Ok?
                }
            }
            symbol
        } else {
            // The symbol does not present on this scope level.
            // Some checks to insert new symbol into symbol table:
            match role {
                SymbolUsage::Nonlocal if scope_depth < 2 => {
                    return Err(SymbolTableError {
                        error: "nonlocal declaration not allowed at module level".into(),
                        location,
                    });
                }
                _ => {
                    // Ok!
                }
            }
            // Insert symbol when required:
            let symbol = Symbol::new(name.as_ref());
            table.symbols.entry(name.into_owned()).or_insert(symbol)
        };

        if matches!(role, SymbolUsage::Global | SymbolUsage::Nonlocal) {
            symbol.location = location;
        }

        // Set proper scope and flags on symbol:
        let flags = &mut symbol.flags;
        match role {
            SymbolUsage::Nonlocal => {
                symbol.scope = SymbolScope::Free;
                flags.insert(SymbolFlags::NONLOCAL);
            }
            SymbolUsage::Imported => {
                flags.insert(SymbolFlags::ASSIGNED | SymbolFlags::IMPORTED);
            }
            SymbolUsage::Parameter => {
                flags.insert(SymbolFlags::PARAMETER);
                // Parameters are always added to varnames first
                let name_str = symbol.name.clone();
                if !self.current_varnames.contains(&name_str) {
                    self.current_varnames.push(name_str);
                }
            }
            SymbolUsage::AnnotationParameter => {
                flags.insert(SymbolFlags::PARAMETER | SymbolFlags::ANNOTATED);
                // Annotated parameters are also added to varnames
                let name_str = symbol.name.clone();
                if !self.current_varnames.contains(&name_str) {
                    self.current_varnames.push(name_str);
                }
            }
            SymbolUsage::AnnotationAssigned => {
                flags.insert(SymbolFlags::ASSIGNED | SymbolFlags::ANNOTATED);
            }
            SymbolUsage::Assigned => {
                flags.insert(SymbolFlags::ASSIGNED);
            }
            SymbolUsage::AssignedNamedExprInComprehension => {
                flags.insert(SymbolFlags::ASSIGNED | SymbolFlags::ASSIGNED_IN_COMPREHENSION);
            }
            SymbolUsage::Global => {
                symbol.scope = SymbolScope::GlobalExplicit;
                flags.insert(SymbolFlags::GLOBAL);
            }
            SymbolUsage::Used => {
                flags.insert(SymbolFlags::REFERENCED);
            }
            SymbolUsage::Iter => {
                flags.insert(SymbolFlags::ITER | SymbolFlags::COMP_ITER);
            }
            SymbolUsage::TypeParam => {
                flags.insert(SymbolFlags::ASSIGNED | SymbolFlags::TYPE_PARAM);
            }
        }

        // and even more checking
        // it is not allowed to assign to iterator variables (by named expressions)
        if flags.contains(SymbolFlags::ITER)
            && flags.contains(SymbolFlags::ASSIGNED_IN_COMPREHENSION)
        {
            return Err(SymbolTableError {
                error: format!(
                    "assignment expression cannot rebind comprehension iteration variable '{}'",
                    symbol.name
                ),
                location,
            });
        }
        Ok(())
    }
}

fn is_docstring_expr(expr: &ast::Expr) -> bool {
    matches!(
        expr,
        ast::Expr::StringLiteral(_)
            | ast::Expr::Constant(ast::ExprConstant {
                value: ast::ConstantValue::Str(_),
                ..
            })
    )
}

pub(crate) fn mangle_name<'a>(class_name: Option<&str>, name: &'a str) -> Cow<'a, str> {
    let class_name = match class_name {
        Some(n) => n,
        None => return name.into(),
    };
    if !name.starts_with("__") || name.ends_with("__") || name.contains('.') {
        return name.into();
    }
    // Strip leading underscores from class name
    let class_name = class_name.trim_start_matches('_');
    if class_name.is_empty() {
        return name.into();
    }
    let mut ret = String::with_capacity(1 + class_name.len() + name.len());
    ret.push('_');
    ret.push_str(class_name);
    ret.push_str(name);
    ret.into()
}

/// Selective mangling for type parameter scopes around generic classes.
/// If `mangled_names` is Some, only mangle names that are in the set;
/// other names are left unmangled.
pub(crate) fn maybe_mangle_name<'a>(
    class_name: Option<&str>,
    mangled_names: Option<&IndexSet<String>>,
    name: &'a str,
) -> Cow<'a, str> {
    if let Some(set) = mangled_names
        && !set.contains(name)
    {
        return name.into();
    }
    mangle_name(class_name, name)
}

#[cfg(test)]
mod tests {
    use super::{CompilerScope, SymbolFlags, SymbolTable, mangle_name};
    use rustpython_compiler_core::SourceFileBuilder;

    fn scan_source(source: &str) -> SymbolTable {
        scan_source_result(source).unwrap()
    }

    fn scan_source_result(source: &str) -> Result<SymbolTable, super::SymbolTableError> {
        let source_file = SourceFileBuilder::new("source_path", source).finish();
        let parsed = ruff_python_parser::parse(
            source_file.source_text(),
            ruff_python_parser::Mode::Module.into(),
        )
        .unwrap()
        .into_syntax();
        let module = match parsed {
            ruff_python_ast::Mod::Module(module) => module,
            _ => unreachable!(),
        };
        SymbolTable::scan_program(&module, source_file)
    }

    #[test]
    fn mangle_name_leaves_private_name_in_underscore_only_class() {
        assert_eq!(mangle_name(Some("_"), "__a"), "__a");
        assert_eq!(mangle_name(Some("__"), "__a"), "__a");
        assert_eq!(mangle_name(Some("___"), "__a"), "__a");
    }

    #[test]
    fn mangle_name_strips_leading_class_underscores() {
        assert_eq!(mangle_name(Some("_a"), "__a"), "_a__a");
        assert_eq!(mangle_name(Some("__a"), "__a"), "_a__a");
    }

    #[test]
    fn duplicate_parameter_check_uses_mangled_name_like_cpython() {
        let err = scan_source_result("class C:\n    def f(__x, _C__x):\n        pass\n")
            .expect_err("expected duplicate argument after class-private mangling");

        assert_eq!(
            err.error,
            "duplicate argument '_C__x' in function definition"
        );
    }

    #[test]
    fn super_name_marks_class_use_in_lambda_scope_like_cpython() {
        let table = scan_source("def f():\n    return lambda: super()\n");
        let function = table
            .sub_tables
            .iter()
            .find(|table| table.name == "f")
            .expect("missing function scope");
        let lambda = function
            .sub_tables
            .iter()
            .find(|table| table.typ == CompilerScope::Lambda)
            .expect("missing lambda scope");

        assert!(
            lambda.lookup("__class__").is_some(),
            "CPython symtable Name_kind treats super as a __class__ use in any function-like scope"
        );
    }

    #[test]
    fn comprehension_iteration_target_sets_comp_iter_flag_like_cpython() {
        let table = scan_source("result = [i for i in xs]\n");
        let comprehension = table
            .inlined_comprehension_blocks
            .iter()
            .find(|table| table.typ == CompilerScope::Comprehension)
            .expect("missing comprehension scope");
        let symbol = comprehension
            .lookup("i")
            .expect("missing comprehension iteration target");

        assert!(
            symbol.flags.contains(SymbolFlags::COMP_ITER),
            "CPython symtable_add_def_helper sets DEF_COMP_ITER on comprehension iteration targets"
        );
    }

    #[test]
    fn inlined_comprehension_children_are_spliced_like_cpython() {
        let table = scan_source("result = [(lambda: i) for i in xs]\n");

        assert!(
            !table
                .sub_tables
                .iter()
                .any(|table| table.typ == CompilerScope::Comprehension),
            "CPython removes inlined comprehension entries from ste_children"
        );
        assert!(
            table
                .sub_tables
                .iter()
                .any(|table| table.typ == CompilerScope::Lambda),
            "CPython splices children of inlined comprehensions into the parent children list"
        );

        let comprehension = table
            .inlined_comprehension_blocks
            .iter()
            .find(|table| table.typ == CompilerScope::Comprehension)
            .expect("missing inlined comprehension block");
        assert!(
            comprehension.comp_inlined,
            "CPython keeps the comprehension entry addressable through st_blocks with ste_comp_inlined set"
        );
    }

    #[test]
    fn future_annotations_annassign_still_scans_annotation_symbols_like_cpython() {
        let table = scan_source("from __future__ import annotations\nx: T\n");
        let annotation_block = table
            .annotation_block
            .as_ref()
            .expect("CPython still creates an AnnotationBlock for future annotations");

        assert!(
            annotation_block.lookup("T").is_some(),
            "CPython symtable_visit_annotation still visits the annotation expression with future annotations"
        );
    }

    #[test]
    fn annotation_like_format_parameter_is_marked_used_like_cpython() {
        let table = scan_source("def f(x: T): pass\n");
        let annotation_block = table
            .sub_tables
            .iter()
            .find(|table| table.typ == CompilerScope::Annotation)
            .expect("missing function annotation block");
        let format = annotation_block
            .lookup(".format")
            .expect("missing annotation .format parameter");
        assert!(
            format
                .flags
                .contains(SymbolFlags::PARAMETER | SymbolFlags::REFERENCED),
            "CPython symtable_enter_block() adds both DEF_PARAM and USE for annotation-like .format"
        );

        let table = scan_source("type A = T\n");
        let alias = table
            .sub_tables
            .iter()
            .find(|table| table.typ == CompilerScope::TypeAlias)
            .expect("missing type alias scope");
        let format = alias
            .lookup(".format")
            .expect("missing type alias .format parameter");
        assert!(
            format
                .flags
                .contains(SymbolFlags::PARAMETER | SymbolFlags::REFERENCED),
            "CPython TypeAliasBlock .format has DEF_PARAM | USE"
        );

        let table = scan_source("def f[T: B](): pass\n");
        let type_params = table
            .sub_tables
            .iter()
            .find(|table| table.typ == CompilerScope::TypeParams)
            .expect("missing type params scope");
        let type_variable = type_params
            .sub_tables
            .iter()
            .find(|table| table.typ == CompilerScope::TypeVariable)
            .expect("missing type variable scope");
        let format = type_variable
            .lookup(".format")
            .expect("missing type variable .format parameter");
        assert!(
            format
                .flags
                .contains(SymbolFlags::PARAMETER | SymbolFlags::REFERENCED),
            "CPython TypeVariableBlock .format has DEF_PARAM | USE"
        );
    }

    #[test]
    fn function_signature_annotation_block_is_sibling_like_cpython() {
        let table = scan_source("def f(x: T): pass\n");
        assert_eq!(table.sub_tables[0].typ, CompilerScope::Annotation);
        assert!(table.sub_tables[0].annotations_used);
        assert_eq!(table.sub_tables[1].typ, CompilerScope::Function);
        assert!(
            table.sub_tables[1].annotation_block.is_none(),
            "CPython stores the function signature AnnotationBlock as a child keyed by arguments, not on the function block"
        );

        let table = scan_source("def f(x): pass\n");
        assert_eq!(table.sub_tables[0].typ, CompilerScope::Annotation);
        assert!(!table.sub_tables[0].annotations_used);
        assert_eq!(table.sub_tables[1].typ, CompilerScope::Function);
    }

    #[test]
    fn future_function_signature_annotation_block_is_hidden_like_cpython() {
        let table = scan_source("from __future__ import annotations\ndef f(x: T): pass\n");
        assert_eq!(table.sub_tables[0].typ, CompilerScope::Function);
        assert_eq!(
            table.hidden_annotation_blocks[0].typ,
            CompilerScope::Annotation
        );
        assert!(table.hidden_annotation_blocks[0].annotations_used);
        assert!(
            table.sub_tables[0].annotation_block.is_none(),
            "CPython future AnnotationBlock stays in st_blocks and is not attached to the FunctionBlock"
        );

        let table = scan_source("from __future__ import annotations\ndef f(x): pass\n");
        assert_eq!(table.sub_tables[0].typ, CompilerScope::Function);
        assert_eq!(
            table.hidden_annotation_blocks[0].typ,
            CompilerScope::Annotation
        );
        assert!(!table.hidden_annotation_blocks[0].annotations_used);
    }

    #[test]
    fn annassign_marks_current_scope_annotations_used_like_cpython() {
        let table = scan_source("x: int\n");
        assert!(
            table.annotations_used,
            "CPython AnnAssign_kind sets ste_annotations_used on the current scope"
        );

        let table = scan_source("class C:\n    x: int\n");
        let class = table
            .sub_tables
            .iter()
            .find(|table| table.typ == CompilerScope::Class)
            .expect("missing class scope");
        assert!(
            class.annotations_used,
            "CPython AnnAssign_kind sets ste_annotations_used on class scopes"
        );

        let table = scan_source("def f():\n    x: int\n");
        let function = table
            .sub_tables
            .iter()
            .find(|table| table.typ == CompilerScope::Function)
            .expect("missing function scope");
        assert!(
            function.annotations_used,
            "CPython AnnAssign_kind also marks function-local annotations"
        );
    }

    #[test]
    fn class_base_child_scope_precedes_class_scope_like_cpython() {
        let table = scan_source("class C((lambda: Base)()):\n    pass\n");
        assert_eq!(table.sub_tables[0].typ, CompilerScope::Lambda);
        assert_eq!(table.sub_tables[1].typ, CompilerScope::Class);
    }

    #[test]
    fn try_handler_child_scope_precedes_else_scope_like_cpython() {
        let table = scan_source(
            "\
def f(x):
    try:
        pass
    except Exception:
        y = 1
        def h():
            return y
    else:
        def e():
            return x
",
        );
        let function = table
            .sub_tables
            .iter()
            .find(|table| table.name == "f")
            .expect("missing function scope");

        let function_child_names = function
            .sub_tables
            .iter()
            .filter(|table| table.typ == CompilerScope::Function)
            .map(|table| table.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(function_child_names, vec!["h", "e"]);
    }

    #[test]
    fn function_default_child_scope_precedes_decorator_scope_like_cpython() {
        let table = scan_source(
            "\
@(lambda decorator_arg: decorator_arg)
def f(x=(lambda: 1)()):
    pass
",
        );
        let lambdas = table
            .sub_tables
            .iter()
            .filter(|table| table.typ == CompilerScope::Lambda)
            .collect::<Vec<_>>();

        assert_eq!(lambdas.len(), 2);
        assert!(
            lambdas[0].varnames.is_empty(),
            "CPython symtable visits function defaults before decorators"
        );
        assert_eq!(lambdas[1].varnames, vec!["decorator_arg"]);
    }

    #[test]
    fn future_annotations_still_rejects_named_expr_in_annotation_like_cpython() {
        let err =
            scan_source_result("from __future__ import annotations\nx: (y := int)\n").unwrap_err();

        assert_eq!(
            err.error,
            "named expression cannot be used within an annotation"
        );
    }

    #[test]
    fn import_star_outside_module_uses_cpython_symtable_message() {
        let err = scan_source_result("def f():\n    from m import *\n").unwrap_err();

        assert_eq!(err.error, "import * only allowed at module level");
    }

    #[test]
    fn import_as_error_location_uses_alias_location_like_cpython() {
        let source = "import module as __debug__\n";
        let err = scan_source_result(source).unwrap_err();

        assert_eq!(err.error, "cannot assign to __debug__");
        let location = err.location.unwrap();
        assert_eq!(location.line.get(), 1);
        assert_eq!(
            location.character_offset.get(),
            8,
            "CPython reports LOCATION(a) for import aliases, at the imported name"
        );
    }

    #[test]
    fn function_def_error_location_uses_statement_location_like_cpython() {
        let source = "def __debug__():\n    pass\n";
        let err = scan_source_result(source).unwrap_err();

        assert_eq!(err.error, "cannot assign to __debug__");
        let location = err.location.unwrap();
        assert_eq!(location.line.get(), 1);
        assert_eq!(
            location.character_offset.get(),
            1,
            "CPython reports LOCATION(s) for FunctionDef, at 'def'"
        );
    }

    #[test]
    fn global_after_assign_error_location_uses_statement_location_like_cpython() {
        let source = "def f():\n    x = 1\n    global x\n";
        let err = scan_source_result(source).unwrap_err();

        assert_eq!(
            err.error,
            "name 'x' is assigned to before global declaration"
        );
        let location = err.location.unwrap();
        assert_eq!(location.line.get(), 3);
        assert_eq!(
            location.character_offset.get(),
            5,
            "CPython reports LOCATION(s) for global directives, at 'global'"
        );
    }

    #[test]
    fn type_param_debug_name_is_checked_like_cpython_add_def_ctx() {
        let source = "class C[__debug__]:\n    pass\n";
        let err = scan_source_result(source).unwrap_err();

        assert_eq!(err.error, "cannot assign to __debug__");
        let location = err.location.unwrap();
        assert_eq!(location.line.get(), 1);
        assert_eq!(
            location.character_offset.get(),
            9,
            "CPython symtable_add_def_ctx checks DEF_TYPE_PARAM | DEF_LOCAL at LOCATION(tp)"
        );
    }

    #[test]
    fn except_handler_name_error_location_uses_handler_location_like_cpython() {
        let source = "try:\n    pass\nexcept Exception as __debug__:\n    pass\n";
        let err = scan_source_result(source).unwrap_err();

        assert_eq!(err.error, "cannot assign to __debug__");
        let location = err.location.unwrap();
        assert_eq!(location.line.get(), 3);
        assert_eq!(
            location.character_offset.get(),
            1,
            "CPython reports LOCATION(eh) for except-handler names, at 'except'"
        );
    }

    #[test]
    fn match_star_capture_error_location_uses_pattern_location_like_cpython() {
        let source = "match subject:\n    case [*__debug__]:\n        pass\n";
        let err = scan_source_result(source).unwrap_err();

        assert_eq!(err.error, "cannot assign to __debug__");
        let location = err.location.unwrap();
        assert_eq!(location.line.get(), 2);
        assert_eq!(
            location.character_offset.get(),
            11,
            "CPython reports LOCATION(p) for MatchStar, at the '*'"
        );
    }

    #[test]
    fn named_expr_in_lambda_inside_comprehension_iter_is_rejected_like_cpython() {
        let err = scan_source_result("[x for x in (lambda: (y := 1))()]\n").unwrap_err();

        assert_eq!(
            err.error,
            "assignment expression cannot be used in a comprehension iterable expression"
        );
    }

    #[test]
    fn yield_in_lambda_inside_comprehension_body_is_not_comprehension_yield_like_cpython() {
        scan_source_result("[(lambda: (yield x)) for x in xs]\n").expect(
            "CPython checks ste_comprehension on the current lambda block, not the enclosing comprehension",
        );
    }

    #[test]
    fn yield_in_comprehension_scans_value_before_comprehension_error_like_cpython() {
        let err = scan_source_result("[(yield (x := 1)) for x in xs]\n").unwrap_err();

        assert_eq!(
            err.error,
            "assignment expression cannot rebind comprehension iteration variable 'x'"
        );
    }

    #[test]
    fn named_expr_in_function_annotation_comprehension_is_allowed_like_cpython() {
        scan_source_result("def f(x: [(y := int) for _ in xs]): pass\n").expect(
            "CPython skips AnnotationBlock while extending namedexpr scope from a comprehension",
        );
    }

    #[test]
    fn named_expr_in_class_annotation_comprehension_uses_cpython_message() {
        let err = scan_source_result("class C:\n    x: [(y := int) for _ in xs]\n").unwrap_err();

        assert_eq!(
            err.error,
            "assignment expression within a comprehension cannot be used in a class body"
        );
    }

    #[test]
    fn named_expr_in_type_alias_comprehension_uses_cpython_message() {
        let err = scan_source_result("type A = [(y := int) for _ in xs]\n").unwrap_err();

        assert_eq!(
            err.error,
            "assignment expression within a comprehension cannot be used in a type alias"
        );
    }

    #[test]
    fn named_expr_in_type_parameters_block_uses_cpython_message() {
        let err = scan_source_result("class C[T]((base := object)): pass\n").unwrap_err();

        assert_eq!(
            err.error,
            "named expression cannot be used within the definition of a generic"
        );
    }

    #[test]
    fn named_expr_in_typevar_bound_comprehension_uses_cpython_message() {
        let err = scan_source_result("def f[T: [(y := int) for _ in xs]](): pass\n").unwrap_err();

        assert_eq!(
            err.error,
            "assignment expression within a comprehension cannot be used in a TypeVar bound"
        );
    }
}
