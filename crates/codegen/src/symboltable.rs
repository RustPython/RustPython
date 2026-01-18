/* Python code is pre-scanned for symbols in the ast.

This ensures that global and nonlocal keywords are picked up.
Then the compiler can use the symbol table to generate proper
load and store instructions for names.

Inspirational file: https://github.com/python/cpython/blob/main/Python/symtable.c
*/

use crate::{
    IndexMap,
    error::{CodegenError, CodegenErrorType},
};
use alloc::{borrow::Cow, fmt};
use bitflags::bitflags;
use ruff_python_ast as ast;
use ruff_text_size::{Ranged, TextRange};
use rustpython_compiler_core::{PositionEncoding, SourceFile, SourceLocation};
use std::collections::HashSet;

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

    /// A set of symbols present on this scope level.
    pub symbols: IndexMap<String, Symbol>,

    /// A list of sub-scopes in the order as found in the
    /// AST nodes.
    pub sub_tables: Vec<SymbolTable>,

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

    /// Whether this comprehension scope should be inlined (PEP 709)
    /// True for list/set/dict comprehensions in non-generator expressions
    pub comp_inlined: bool,

    /// PEP 649: Reference to annotation scope for this block
    /// Annotations are compiled as a separate `__annotate__` function
    pub annotation_block: Option<Box<SymbolTable>>,

    /// PEP 649: Whether this scope has conditional annotations
    /// (annotations inside if/for/while/etc. blocks or at module level)
    pub has_conditional_annotations: bool,

    /// Whether `from __future__ import annotations` is active
    pub future_annotations: bool,
}

impl SymbolTable {
    fn new(name: String, typ: CompilerScope, line_number: u32, is_nested: bool) -> Self {
        Self {
            name,
            typ,
            line_number,
            is_nested,
            symbols: IndexMap::default(),
            sub_tables: vec![],
            next_sub_table: 0,
            varnames: Vec::new(),
            needs_class_closure: false,
            needs_classdict: false,
            can_see_class_scope: false,
            comp_inlined: false,
            annotation_block: None,
            has_conditional_annotations: false,
            future_annotations: false,
        }
    }

    pub fn scan_program(
        program: &ast::ModModule,
        source_file: SourceFile,
    ) -> SymbolTableResult<Self> {
        let mut builder = SymbolTableBuilder::new(source_file);
        builder.scan_statements(program.body.as_ref())?;
        builder.finish()
    }

    pub fn scan_expr(
        expr: &ast::ModExpression,
        source_file: SourceFile,
    ) -> SymbolTableResult<Self> {
        let mut builder = SymbolTableBuilder::new(source_file);
        builder.scan_expression(expr.body.as_ref(), ExpressionContext::Load)?;
        builder.finish()
    }

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
            // TODO missing types from the C implementation
            // if self._table.type == _symtable.TYPE_TYPE_VAR_BOUND:
            //     return "TypeVar bound"
            // if self._table.type == _symtable.TYPE_TYPE_ALIAS:
            //     return "type alias"
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

bitflags! {
    #[derive(Copy, Clone, Debug, PartialEq)]
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
}

impl Symbol {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            // table,
            scope: SymbolScope::Unknown,
            flags: SymbolFlags::empty(),
        }
    }

    pub const fn is_global(&self) -> bool {
        matches!(
            self.scope,
            SymbolScope::GlobalExplicit | SymbolScope::GlobalImplicit
        )
    }

    pub const fn is_local(&self) -> bool {
        matches!(self.scope, SymbolScope::Local | SymbolScope::Cell)
    }

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
    pub fn into_codegen_error(self, source_path: String) -> CodegenError {
        CodegenError {
            location: self.location,
            error: CodegenErrorType::SyntaxError(self.error),
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
   and set the appropriate flags. Equivalent to CPython's drop_class_free().
   See: https://github.com/python/cpython/blob/main/Python/symtable.c#L884

   This function removes __class__ and __classdict__ from the
   `newfree` set (which contains free variables collected from all child scopes)
   and sets the corresponding flags on the class's symbol table entry.
*/
fn drop_class_free(symbol_table: &mut SymbolTable, newfree: &mut HashSet<String>) {
    // Check if __class__ is in the free variables collected from children
    // If found, it means a child scope (method) references __class__
    if newfree.remove("__class__") {
        symbol_table.needs_class_closure = true;
    }

    // Check if __classdict__ is in the free variables collected from children
    if newfree.remove("__classdict__") {
        symbol_table.needs_classdict = true;
    }

    // Check if __conditional_annotations__ is in the free variables collected from children
    // Remove it from free set - it's handled specially in class scope
    if newfree.remove("__conditional_annotations__") {
        symbol_table.has_conditional_annotations = true;
    }
}

type SymbolMap = IndexMap<String, Symbol>;

mod stack {
    use core::ptr::NonNull;
    use std::panic;
    pub struct StackStack<T> {
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
        pub fn with_append<F, R>(&mut self, x: &mut T, f: F) -> R
        where
            F: FnOnce(&mut Self) -> R,
        {
            self.v.push(x.into());
            let res = panic::catch_unwind(panic::AssertUnwindSafe(|| f(self)));
            self.v.pop();
            res.unwrap_or_else(|x| panic::resume_unwind(x))
        }

        pub fn iter(&self) -> impl DoubleEndedIterator<Item = &T> + '_ {
            self.as_ref().iter().copied()
        }
        pub fn iter_mut(&mut self) -> impl DoubleEndedIterator<Item = &mut T> + '_ {
            self.as_mut().iter_mut().map(|x| &mut **x)
        }
        // pub fn top(&self) -> Option<&T> {
        //     self.as_ref().last().copied()
        // }
        // pub fn top_mut(&mut self) -> Option<&mut T> {
        //     self.as_mut().last_mut().map(|x| &mut **x)
        // }
        pub fn len(&self) -> usize {
            self.v.len()
        }
        pub fn is_empty(&self) -> bool {
            self.len() == 0
        }

        pub fn as_ref(&self) -> &[&T] {
            unsafe { &*(self.v.as_slice() as *const [NonNull<T>] as *const [&T]) }
        }

        pub fn as_mut(&mut self) -> &mut [&mut T] {
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
    tables: StackStack<(SymbolMap, CompilerScope)>,
}

impl SymbolTableAnalyzer {
    /// Analyze a symbol table and return the set of free variables.
    /// See symtable.c analyze_block().
    /// class_entry: PEP 649 - enclosing class symbols for annotation scopes
    fn analyze_symbol_table(
        &mut self,
        symbol_table: &mut SymbolTable,
        class_entry: Option<&SymbolMap>,
    ) -> SymbolTableResult<HashSet<String>> {
        let symbols = core::mem::take(&mut symbol_table.symbols);
        let sub_tables = &mut *symbol_table.sub_tables;

        // Collect free variables from all child scopes
        let mut newfree = HashSet::new();

        let annotation_block = &mut symbol_table.annotation_block;

        // PEP 649: Determine class_entry to pass to children
        // If current scope is a class with annotation block that can_see_class_scope,
        // we need to pass class symbols to the annotation scope
        let is_class = symbol_table.typ == CompilerScope::Class;

        // Clone class symbols if needed for annotation scope (to avoid borrow conflict)
        let class_symbols_for_ann = if is_class
            && annotation_block
                .as_ref()
                .is_some_and(|b| b.can_see_class_scope)
        {
            Some(symbols.clone())
        } else {
            None
        };

        let mut info = (symbols, symbol_table.typ);
        self.tables.with_append(&mut info, |list| {
            let inner_scope = unsafe { &mut *(list as *mut _ as *mut Self) };
            // Analyze sub scopes and collect their free variables
            for sub_table in sub_tables.iter_mut() {
                // Sub-scopes (functions, nested classes) don't inherit class_entry
                let child_free = inner_scope.analyze_symbol_table(sub_table, None)?;
                // Propagate child's free variables to this scope
                newfree.extend(child_free);
            }
            // PEP 649: Analyze annotation block if present
            if let Some(annotation_table) = annotation_block {
                // Pass class symbols to annotation scope if can_see_class_scope
                let ann_class_entry = class_symbols_for_ann.as_ref().or(class_entry);
                let child_free =
                    inner_scope.analyze_symbol_table(annotation_table, ann_class_entry)?;
                // Propagate annotation's free variables to this scope
                newfree.extend(child_free);
            }
            Ok(())
        })?;

        symbol_table.symbols = info.0;

        // PEP 709: Merge symbols from inlined comprehensions into parent scope
        // Only merge symbols that are actually bound in the comprehension,
        // not references to outer scope variables (Free symbols).
        const BOUND_FLAGS: SymbolFlags = SymbolFlags::ASSIGNED
            .union(SymbolFlags::PARAMETER)
            .union(SymbolFlags::ITER)
            .union(SymbolFlags::ASSIGNED_IN_COMPREHENSION);

        for sub_table in sub_tables.iter() {
            if sub_table.comp_inlined {
                for (name, sub_symbol) in &sub_table.symbols {
                    // Skip the .0 parameter - it's internal to the comprehension
                    if name == ".0" {
                        continue;
                    }
                    // Only merge symbols that are bound in the comprehension
                    // Skip Free references to outer scope variables
                    if !sub_symbol.flags.intersects(BOUND_FLAGS) {
                        continue;
                    }
                    // If the symbol doesn't exist in parent, add it
                    if !symbol_table.symbols.contains_key(name) {
                        let mut symbol = sub_symbol.clone();
                        // Mark as local in parent scope
                        symbol.scope = SymbolScope::Local;
                        symbol_table.symbols.insert(name.clone(), symbol);
                    }
                }
            }
        }

        // Analyze symbols in current scope
        for symbol in symbol_table.symbols.values_mut() {
            self.analyze_symbol(symbol, symbol_table.typ, sub_tables, class_entry)?;

            // Collect free variables from this scope
            // These will be propagated to the parent scope
            if symbol.scope == SymbolScope::Free || symbol.flags.contains(SymbolFlags::FREE_CLASS) {
                newfree.insert(symbol.name.clone());
            }
        }

        // Handle class-specific implicit cells
        // This removes __class__ and __classdict__ from newfree if present
        // and sets the corresponding flags on the symbol table
        if symbol_table.typ == CompilerScope::Class {
            drop_class_free(symbol_table, &mut newfree);
        }

        Ok(newfree)
    }

    fn analyze_symbol(
        &mut self,
        symbol: &mut Symbol,
        st_typ: CompilerScope,
        sub_tables: &[SymbolTable],
        class_entry: Option<&SymbolMap>,
    ) -> SymbolTableResult {
        if symbol
            .flags
            .contains(SymbolFlags::ASSIGNED_IN_COMPREHENSION)
            && st_typ == CompilerScope::Comprehension
        {
            // propagate symbol to next higher level that can hold it,
            // i.e., function or module. Comprehension is skipped and
            // Class is not allowed and detected as error.
            //symbol.scope = SymbolScope::Nonlocal;
            self.analyze_symbol_comprehension(symbol, 0)?
        } else {
            match symbol.scope {
                SymbolScope::Free => {
                    if !self.tables.as_ref().is_empty() {
                        let scope_depth = self.tables.as_ref().len();
                        // check if the name is already defined in any outer scope
                        // therefore
                        if scope_depth < 2
                            || self.found_in_outer_scope(&symbol.name, st_typ)
                                != Some(SymbolScope::Free)
                        {
                            return Err(SymbolTableError {
                                error: format!("no binding for nonlocal '{}' found", symbol.name),
                                // TODO: accurate location info, somehow
                                location: None,
                            });
                        }
                    } else {
                        return Err(SymbolTableError {
                            error: format!(
                                "nonlocal {} defined at place without an enclosing scope",
                                symbol.name
                            ),
                            // TODO: accurate location info, somehow
                            location: None,
                        });
                    }
                }
                SymbolScope::GlobalExplicit | SymbolScope::GlobalImplicit => {
                    // TODO: add more checks for globals?
                }
                SymbolScope::Local | SymbolScope::Cell => {
                    // all is well
                }
                SymbolScope::Unknown => {
                    // PEP 649: Check class_entry first (like analyze_name)
                    // If name is bound in enclosing class, mark as GlobalImplicit
                    if let Some(class_symbols) = class_entry
                        && let Some(class_sym) = class_symbols.get(&symbol.name)
                    {
                        // DEF_BOUND && !DEF_NONLOCAL -> GLOBAL_IMPLICIT
                        if class_sym.is_bound() && class_sym.scope != SymbolScope::Free {
                            symbol.scope = SymbolScope::GlobalImplicit;
                            return Ok(());
                        }
                    }

                    // Try hard to figure out what the scope of this symbol is.
                    let scope = if symbol.is_bound() {
                        self.found_in_inner_scope(sub_tables, &symbol.name, st_typ)
                            .unwrap_or(SymbolScope::Local)
                    } else if let Some(scope) = self.found_in_outer_scope(&symbol.name, st_typ) {
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
        }
        Ok(())
    }

    fn found_in_outer_scope(&mut self, name: &str, st_typ: CompilerScope) -> Option<SymbolScope> {
        let mut decl_depth = None;
        for (i, (symbols, typ)) in self.tables.iter().rev().enumerate() {
            if matches!(typ, CompilerScope::Module)
                || matches!(typ, CompilerScope::Class if name != "__class__" && name != "__classdict__" && name != "__conditional_annotations__")
            {
                continue;
            }

            // PEP 649: Annotation scope is conceptually a sibling of the function,
            // not a child. Skip the immediate parent function scope when looking
            // for outer variables from annotation scope.
            if st_typ == CompilerScope::Annotation
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
            for (table, typ) in self.tables.iter_mut().rev().take(decl_depth) {
                if let CompilerScope::Class = typ {
                    if let Some(free_class) = table.get_mut(name) {
                        free_class.flags.insert(SymbolFlags::FREE_CLASS)
                    } else {
                        let mut symbol = Symbol::new(name);
                        symbol.flags.insert(SymbolFlags::FREE_CLASS);
                        symbol.scope = SymbolScope::Free;
                        table.insert(name.to_owned(), symbol);
                    }
                } else if !table.contains_key(name) {
                    let mut symbol = Symbol::new(name);
                    symbol.scope = SymbolScope::Free;
                    // symbol.is_referenced = true;
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
            let sym = st.symbols.get(name)?;
            if sym.scope == SymbolScope::Free || sym.flags.contains(SymbolFlags::FREE_CLASS) {
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

    // Implements the symbol analysis and scope extension for names
    // assigned by a named expression in a comprehension. See:
    // https://github.com/python/cpython/blob/7b78e7f9fd77bb3280ee39fb74b86772a7d46a70/Python/symtable.c#L1435
    fn analyze_symbol_comprehension(
        &mut self,
        symbol: &mut Symbol,
        parent_offset: usize,
    ) -> SymbolTableResult {
        // when this is called, we expect to be in the direct parent scope of the scope that contains 'symbol'
        let last = self.tables.iter_mut().rev().nth(parent_offset).unwrap();
        let symbols = &mut last.0;
        let table_type = last.1;

        // it is not allowed to use an iterator variable as assignee in a named expression
        if symbol.flags.contains(SymbolFlags::ITER) {
            return Err(SymbolTableError {
                error: format!(
                    "assignment expression cannot rebind comprehension iteration variable {}",
                    symbol.name
                ),
                // TODO: accurate location info, somehow
                location: None,
            });
        }

        match table_type {
            CompilerScope::Module => {
                symbol.scope = SymbolScope::GlobalImplicit;
            }
            CompilerScope::Class => {
                // named expressions are forbidden in comprehensions on class scope
                return Err(SymbolTableError {
                    error: "assignment expression within a comprehension cannot be used in a class body".to_string(),
                    // TODO: accurate location info, somehow
                    location: None,
                });
            }
            CompilerScope::Function | CompilerScope::AsyncFunction | CompilerScope::Lambda => {
                if let Some(parent_symbol) = symbols.get_mut(&symbol.name) {
                    if let SymbolScope::Unknown = parent_symbol.scope {
                        // this information is new, as the assignment is done in inner scope
                        parent_symbol.flags.insert(SymbolFlags::ASSIGNED);
                    }

                    symbol.scope = if parent_symbol.is_global() {
                        parent_symbol.scope
                    } else {
                        SymbolScope::Free
                    };
                } else {
                    let mut cloned_sym = symbol.clone();
                    cloned_sym.scope = SymbolScope::Cell;
                    last.0.insert(cloned_sym.name.to_owned(), cloned_sym);
                }
            }
            CompilerScope::Comprehension => {
                // TODO check for conflicts - requires more context information about variables
                match symbols.get_mut(&symbol.name) {
                    Some(parent_symbol) => {
                        // check if assignee is an iterator in top scope
                        if parent_symbol.flags.contains(SymbolFlags::ITER) {
                            return Err(SymbolTableError {
                                error: format!(
                                    "assignment expression cannot rebind comprehension iteration variable {}",
                                    symbol.name
                                ),
                                location: None,
                            });
                        }

                        // we synthesize the assignment to the symbol from inner scope
                        parent_symbol.flags.insert(SymbolFlags::ASSIGNED); // more checks are required
                    }
                    None => {
                        // extend the scope of the inner symbol
                        // as we are in a nested comprehension, we expect that the symbol is needed
                        // outside, too, and set it therefore to non-local scope. I.e., we expect to
                        // find a definition on a higher level
                        let mut cloned_sym = symbol.clone();
                        cloned_sym.scope = SymbolScope::Free;
                        last.0.insert(cloned_sym.name.to_owned(), cloned_sym);
                    }
                }

                self.analyze_symbol_comprehension(symbol, parent_offset + 1)?;
            }
            CompilerScope::TypeParams => {
                // Named expression in comprehension cannot be used in type params
                return Err(SymbolTableError {
                    error: "assignment expression within a comprehension cannot be used within the definition of a generic".to_string(),
                    location: None,
                });
            }
            CompilerScope::Annotation => {
                // Named expression is not allowed in annotation scope
                return Err(SymbolTableError {
                    error: "named expression cannot be used within an annotation".to_string(),
                    location: None,
                });
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
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
    source_file: SourceFile,
    // Current scope's varnames being collected (temporary storage)
    current_varnames: Vec<String>,
    // Stack to preserve parent varnames when entering nested scopes
    varnames_stack: Vec<Vec<String>>,
    // Track if we're inside an iterable definition expression (for nested comprehensions)
    in_iter_def_exp: bool,
    // Track if we're inside an annotation (yield/await/named expr not allowed)
    in_annotation: bool,
    // Track if we're inside a type alias (yield/await/named expr not allowed)
    in_type_alias: bool,
    // Track if we're scanning an inner loop iteration target (not the first generator)
    in_comp_inner_loop_target: bool,
    // Scope info for error messages (e.g., "a TypeVar bound")
    scope_info: Option<&'static str>,
    // PEP 649: Track if we're inside a conditional block (if/for/while/etc.)
    in_conditional_block: bool,
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
            source_file,
            current_varnames: Vec::new(),
            varnames_stack: Vec::new(),
            in_iter_def_exp: false,
            in_annotation: false,
            in_type_alias: false,
            in_comp_inner_loop_target: false,
            scope_info: None,
            in_conditional_block: false,
        };
        this.enter_scope("top", CompilerScope::Module, 0);
        this
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
        let is_nested = self
            .tables
            .last()
            .map(|table| table.is_nested || table.typ == CompilerScope::Function)
            .unwrap_or(false);
        let table = SymbolTable::new(name.to_owned(), typ, line_number, is_nested);
        self.tables.push(table);
        // Save parent's varnames and start fresh for the new scope
        self.varnames_stack
            .push(core::mem::take(&mut self.current_varnames));
    }

    fn enter_type_param_block(&mut self, name: &str, line_number: u32) -> SymbolTableResult {
        // Check if we're in a class scope
        let in_class = self
            .tables
            .last()
            .is_some_and(|t| t.typ == CompilerScope::Class);

        self.enter_scope(name, CompilerScope::TypeParams, line_number);

        // If we're in a class, mark that this type param scope can see the class scope
        if let Some(table) = self.tables.last_mut() {
            table.can_see_class_scope = in_class;

            // Add __classdict__ as a USE symbol in type param scope if in class
            if in_class {
                self.register_name("__classdict__", SymbolUsage::Used, TextRange::default())?;
            }
        }

        // Register .type_params as a SET symbol (it will be converted to cell variable later)
        self.register_name(".type_params", SymbolUsage::Assigned, TextRange::default())?;

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

    /// Enter annotation scope (PEP 649)
    /// Creates or reuses the annotation block for the current scope
    fn enter_annotation_scope(&mut self, line_number: u32) {
        let current = self.tables.last_mut().unwrap();
        let can_see_class_scope = current.typ == CompilerScope::Class;
        let has_conditional = current.has_conditional_annotations;

        // Create annotation block if not exists
        if current.annotation_block.is_none() {
            let mut annotation_table = SymbolTable::new(
                "__annotate__".to_owned(),
                CompilerScope::Annotation,
                line_number,
                true, // is_nested
            );
            // Annotation scope in class can see class scope
            annotation_table.can_see_class_scope = can_see_class_scope;
            // Add 'format' parameter
            annotation_table.varnames.push("format".to_owned());
            current.annotation_block = Some(Box::new(annotation_table));
        }

        // Take the annotation block and push to stack for processing
        let annotation_table = current.annotation_block.take().unwrap();
        self.tables.push(*annotation_table);
        // Save parent's varnames and seed with existing annotation varnames (e.g., "format")
        self.varnames_stack
            .push(core::mem::take(&mut self.current_varnames));
        self.current_varnames = self.tables.last().unwrap().varnames.clone();

        if can_see_class_scope && !self.future_annotations {
            self.add_classdict_freevar();
            // Also add __conditional_annotations__ as free var if parent has conditional annotations
            if has_conditional {
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
        self.check_name(
            parameter.name.as_str(),
            ExpressionContext::Store,
            parameter.name.range,
        )?;

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
                    "duplicate parameter '{}' in function definition",
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

    fn scan_annotation(&mut self, annotation: &ast::Expr) -> SymbolTableResult {
        let current_scope = self.tables.last().map(|t| t.typ);

        // PEP 649: Check if this is a conditional annotation
        // Module-level: always conditional (module may be partially executed)
        // Class-level: conditional only when inside if/for/while/etc.
        if !self.future_annotations {
            let is_conditional = matches!(current_scope, Some(CompilerScope::Module))
                || (matches!(current_scope, Some(CompilerScope::Class))
                    && self.in_conditional_block);

            if is_conditional && !self.tables.last().unwrap().has_conditional_annotations {
                self.tables.last_mut().unwrap().has_conditional_annotations = true;
                // Register __conditional_annotations__ symbol in the scope (USE flag, not DEF)
                self.register_name(
                    "__conditional_annotations__",
                    SymbolUsage::Used,
                    annotation.range(),
                )?;
            }
        }

        // Create annotation scope for deferred evaluation
        let line_number = self.line_index_start(annotation.range());
        self.enter_annotation_scope(line_number);

        if self.future_annotations {
            // PEP 563: annotations are stringified at compile time
            // Don't scan expression - symbols would fail to resolve
            // Just create the annotation_block structure
            self.leave_annotation_scope();
            return Ok(());
        }

        // PEP 649: scan expression for symbol references
        // Class annotations are evaluated in class locals (not module globals)
        let was_in_annotation = self.in_annotation;
        self.in_annotation = true;
        let result = self.scan_expression(annotation, ExpressionContext::Load);
        self.in_annotation = was_in_annotation;

        self.leave_annotation_scope();

        // Module scope: re-scan to register symbols (builtins like str, int)
        // Class scope: do NOT re-scan to preserve class-local symbol resolution
        if matches!(current_scope, Some(CompilerScope::Module)) {
            let was_in_annotation = self.in_annotation;
            self.in_annotation = true;
            let _ = self.scan_expression(annotation, ExpressionContext::Load);
            self.in_annotation = was_in_annotation;
        }

        result
    }

    fn scan_statement(&mut self, statement: &ast::Stmt) -> SymbolTableResult {
        use ast::*;
        if let Stmt::ImportFrom(StmtImportFrom { module, names, .. }) = &statement
            && module.as_ref().map(|id| id.as_str()) == Some("__future__")
        {
            self.future_annotations =
                self.future_annotations || names.iter().any(|future| &future.name == "annotations");
        }

        match &statement {
            Stmt::Global(StmtGlobal { names, .. }) => {
                for name in names {
                    self.register_ident(name, SymbolUsage::Global)?;
                }
            }
            Stmt::Nonlocal(StmtNonlocal { names, .. }) => {
                for name in names {
                    self.register_ident(name, SymbolUsage::Nonlocal)?;
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
                ..
            }) => {
                self.scan_decorators(decorator_list, ExpressionContext::Load)?;
                self.register_ident(name, SymbolUsage::Assigned)?;

                // Save the parent's annotation_block before scanning function annotations,
                // so function annotations don't interfere with parent scope annotations.
                // This applies to both class scope (methods) and module scope (top-level functions).
                let parent_scope_typ = self.tables.last().map(|t| t.typ);
                let should_save_annotation_block = matches!(
                    parent_scope_typ,
                    Some(CompilerScope::Class) | Some(CompilerScope::Module)
                );
                let saved_annotation_block = if should_save_annotation_block {
                    self.tables.last_mut().unwrap().annotation_block.take()
                } else {
                    None
                };

                // For generic functions, enter type_param block FIRST so that
                // annotation scopes are nested inside and can see type parameters.
                if let Some(type_params) = type_params {
                    self.enter_type_param_block(
                        &format!("<generic parameters of {}>", name.as_str()),
                        self.line_index_start(type_params.range),
                    )?;
                    self.scan_type_params(type_params)?;
                }
                let has_return_annotation = if let Some(expression) = returns {
                    self.scan_annotation(expression)?;
                    true
                } else {
                    false
                };
                self.enter_scope_with_parameters(
                    name.as_str(),
                    parameters,
                    self.line_index_start(*range),
                    has_return_annotation,
                )?;
                self.scan_statements(body)?;
                self.leave_scope();
                if type_params.is_some() {
                    self.leave_scope();
                }

                // Restore parent's annotation_block after processing the function
                if let Some(block) = saved_annotation_block {
                    self.tables.last_mut().unwrap().annotation_block = Some(block);
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
            }) => {
                if let Some(type_params) = type_params {
                    self.enter_type_param_block(
                        &format!("<generic parameters of {}>", name.as_str()),
                        self.line_index_start(type_params.range),
                    )?;
                    self.scan_type_params(type_params)?;
                }
                self.enter_scope(
                    name.as_str(),
                    CompilerScope::Class,
                    self.line_index_start(*range),
                );
                // Reset in_conditional_block for new class scope
                // (each scope has its own conditional context)
                let saved_in_conditional = self.in_conditional_block;
                self.in_conditional_block = false;
                let prev_class = self.class_name.replace(name.to_string());
                self.register_name("__module__", SymbolUsage::Assigned, *range)?;
                self.register_name("__qualname__", SymbolUsage::Assigned, *range)?;
                self.register_name("__doc__", SymbolUsage::Assigned, *range)?;
                self.register_name("__class__", SymbolUsage::Assigned, *range)?;
                self.scan_statements(body)?;
                self.leave_scope();
                self.in_conditional_block = saved_in_conditional;
                self.class_name = prev_class;
                if let Some(arguments) = arguments {
                    self.scan_expressions(&arguments.args, ExpressionContext::Load)?;
                    for keyword in &arguments.keywords {
                        self.scan_expression(&keyword.value, ExpressionContext::Load)?;
                    }
                }
                if type_params.is_some() {
                    self.leave_scope();
                }
                self.scan_decorators(decorator_list, ExpressionContext::Load)?;
                self.register_ident(name, SymbolUsage::Assigned)?;
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
                ..
            }) => {
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
                        self.check_name(alias.as_str(), ExpressionContext::Store, alias.range)?;
                        self.register_ident(alias, SymbolUsage::Imported)?;
                    } else if name.name.as_str() == "*" {
                        // Star imports are only allowed at module level
                        if self.tables.last().unwrap().typ != CompilerScope::Module {
                            return Err(SymbolTableError {
                                error: "'import *' only allowed at module level".to_string(),
                                location: Some(self.source_file.to_source_code().source_location(
                                    name.name.range.start(),
                                    PositionEncoding::Utf8,
                                )),
                            });
                        }
                        // Don't register star imports as symbols
                    } else {
                        // `import module` or `from x import name`
                        let imported_name = name.name.split('.').next().unwrap();
                        self.check_name(imported_name, ExpressionContext::Store, name.name.range)?;
                        self.register_name(imported_name, SymbolUsage::Imported, name.name.range)?;
                    }
                }
            }
            Stmt::Return(StmtReturn { value, .. }) => {
                if let Some(expression) = value {
                    self.scan_expression(expression, ExpressionContext::Load)?;
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
            }) => {
                // https://github.com/python/cpython/blob/main/Python/symtable.c#L1233
                match &**target {
                    Expr::Name(ast::ExprName { id, .. }) if *simple => {
                        let id_str = id.as_str();

                        self.check_name(id_str, ExpressionContext::Store, *range)?;

                        self.register_name(id_str, SymbolUsage::AnnotationAssigned, *range)?;
                        // PEP 649: Register annotate function in module/class scope
                        let current_scope = self.tables.last().map(|t| t.typ);
                        match current_scope {
                            Some(CompilerScope::Module) => {
                                self.register_name("__annotate__", SymbolUsage::Assigned, *range)?;
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
                    }
                    _ => {
                        self.scan_expression(target, ExpressionContext::Store)?;
                    }
                }
                self.scan_annotation(annotation)?;
                if let Some(value) = value {
                    self.scan_expression(value, ExpressionContext::Load)?;
                }
            }
            Stmt::With(StmtWith { items, body, .. }) => {
                for item in items {
                    self.scan_expression(&item.context_expr, ExpressionContext::Load)?;
                    if let Some(expression) = &item.optional_vars {
                        self.scan_expression(expression, ExpressionContext::Store)?;
                    }
                }
                // PEP 649: Track conditional block for annotations
                let saved_in_conditional_block = self.in_conditional_block;
                self.in_conditional_block = true;
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
                        self.register_ident(name, SymbolUsage::Assigned)?;
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
                }
                if let Some(expression) = cause {
                    self.scan_expression(expression, ExpressionContext::Load)?;
                }
            }
            Stmt::TypeAlias(StmtTypeAlias {
                name,
                value,
                type_params,
                ..
            }) => {
                let was_in_type_alias = self.in_type_alias;
                self.in_type_alias = true;
                if let Some(type_params) = type_params {
                    self.enter_type_param_block(
                        "TypeAlias",
                        self.line_index_start(type_params.range),
                    )?;
                    self.scan_type_params(type_params)?;
                    self.scan_expression(value, ExpressionContext::Load)?;
                    self.leave_scope();
                } else {
                    self.scan_expression(value, ExpressionContext::Load)?;
                }
                self.in_type_alias = was_in_type_alias;
                self.scan_expression(name, ExpressionContext::Store)?;
            }
            Stmt::IpyEscapeCommand(_) => todo!(),
        }
        Ok(())
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
        use ast::*;

        // Check for expressions not allowed in certain contexts
        // (type parameters, annotations, type aliases, TypeVar bounds/defaults)
        if let Some(keyword) = match expression {
            Expr::Yield(_) | Expr::YieldFrom(_) => Some("yield"),
            Expr::Await(_) => Some("await"),
            Expr::Named(_) => Some("named"),
            _ => None,
        } {
            // Determine the context name for the error message
            // scope_info takes precedence (e.g., "a TypeVar bound")
            let context_name = if let Some(scope_info) = self.scope_info {
                Some(scope_info)
            } else if let Some(table) = self.tables.last()
                && table.typ == CompilerScope::TypeParams
            {
                Some("a type parameter")
            } else if self.in_annotation {
                Some("an annotation")
            } else if self.in_type_alias {
                Some("a type alias")
            } else {
                None
            };

            if let Some(context_name) = context_name {
                return Err(SymbolTableError {
                    error: format!("{keyword} expression cannot be used within {context_name}"),
                    location: Some(
                        self.source_file
                            .to_source_code()
                            .source_location(expression.range().start(), PositionEncoding::Utf8),
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
            }) => {
                for item in items {
                    if let Some(key) = &item.key {
                        self.scan_expression(key, context)?;
                    }
                    self.scan_expression(&item.value, context)?;
                }
            }
            Expr::Await(ExprAwait {
                value,
                node_index: _,
                range: _,
            }) => {
                self.scan_expression(value, context)?;
            }
            Expr::Yield(ExprYield {
                value,
                node_index: _,
                range: _,
            }) => {
                if let Some(expression) = value {
                    self.scan_expression(expression, context)?;
                }
            }
            Expr::YieldFrom(ExprYieldFrom {
                value,
                node_index: _,
                range: _,
            }) => {
                self.scan_expression(value, context)?;
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
            }) => {
                let was_in_iter_def_exp = self.in_iter_def_exp;
                if context == ExpressionContext::IterDefinitionExp {
                    self.in_iter_def_exp = true;
                }
                // Dict comprehension - is_generator = false (can be inlined)
                self.scan_comprehension("<dictcomp>", key, Some(value), generators, *range, false)?;
                self.in_iter_def_exp = was_in_iter_def_exp;
            }
            Expr::Call(ExprCall {
                func,
                arguments,
                node_index: _,
                range: _,
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
                    self.scan_expression(&keyword.value, ExpressionContext::Load)?;
                }
            }
            Expr::Name(ExprName { id, range, .. }) => {
                let id = id.as_str();

                self.check_name(id, context, *range)?;

                // Determine the contextual usage of this symbol:
                match context {
                    ExpressionContext::Delete => {
                        self.register_name(id, SymbolUsage::Assigned, *range)?;
                        self.register_name(id, SymbolUsage::Used, *range)?;
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
                    && self.tables.last().unwrap().typ == CompilerScope::Function
                    && id == "super"
                {
                    self.register_name("__class__", SymbolUsage::Used, *range)?;
                }
            }
            Expr::Lambda(ExprLambda {
                body,
                parameters,
                node_index: _,
                range: _,
            }) => {
                if let Some(parameters) = parameters {
                    self.enter_scope_with_parameters(
                        "lambda",
                        parameters,
                        self.line_index_start(expression.range()),
                        false, // lambdas have no return annotation
                    )?;
                } else {
                    self.enter_scope(
                        "lambda",
                        CompilerScope::Lambda,
                        self.line_index_start(expression.range()),
                    );
                }
                match context {
                    ExpressionContext::IterDefinitionExp => {
                        self.scan_expression(body, ExpressionContext::IterDefinitionExp)?;
                    }
                    _ => {
                        self.scan_expression(body, ExpressionContext::Load)?;
                    }
                }
                self.leave_scope();
            }
            Expr::FString(ExprFString { value, .. }) => {
                for expr in value.elements().filter_map(|x| x.as_interpolation()) {
                    self.scan_expression(&expr.expression, ExpressionContext::Load)?;
                    if let Some(format_spec) = &expr.format_spec {
                        for element in format_spec.elements.interpolations() {
                            self.scan_expression(&element.expression, ExpressionContext::Load)?
                        }
                    }
                }
            }
            Expr::TString(tstring) => {
                // Scan t-string interpolation expressions (similar to f-strings)
                for expr in tstring
                    .value
                    .elements()
                    .filter_map(|x| x.as_interpolation())
                {
                    self.scan_expression(&expr.expression, ExpressionContext::Load)?;
                    if let Some(format_spec) = &expr.format_spec {
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
            | Expr::BooleanLiteral(_)
            | Expr::NoneLiteral(_)
            | Expr::EllipsisLiteral(_) => {}
            Expr::IpyEscapeCommand(_) => todo!(),
            Expr::If(ExprIf {
                test,
                body,
                orelse,
                node_index: _,
                range: _,
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
            }) => {
                // named expressions are not allowed in the definition of
                // comprehension iterator definitions (including nested comprehensions)
                if context == ExpressionContext::IterDefinitionExp || self.in_iter_def_exp {
                    return Err(SymbolTableError {
                          error: "assignment expression cannot be used in a comprehension iterable expression".to_string(),
                          location: Some(self.source_file.to_source_code().source_location(target.range().start(), PositionEncoding::Utf8)),
                      });
                }

                self.scan_expression(value, ExpressionContext::Load)?;

                // special handling for assigned identifier in named expressions
                // that are used in comprehensions. This required to correctly
                // propagate the scope of the named assigned named and not to
                // propagate inner names.
                if let Expr::Name(ExprName { id, .. }) = &**target {
                    let id = id.as_str();
                    self.check_name(id, ExpressionContext::Store, *range)?;
                    let table = self.tables.last().unwrap();
                    if table.typ == CompilerScope::Comprehension {
                        self.register_name(
                            id,
                            SymbolUsage::AssignedNamedExprInComprehension,
                            *range,
                        )?;
                    } else {
                        // omit one recursion. When the handling of an store changes for
                        // Identifiers this needs adapted - more forward safe would be
                        // calling scan_expression directly.
                        self.register_name(id, SymbolUsage::Assigned, *range)?;
                    }
                } else {
                    self.scan_expression(target, ExpressionContext::Store)?;
                }
            }
        }
        Ok(())
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
        // Comprehensions are compiled as functions, so create a scope for them:
        self.enter_scope(
            scope_name,
            CompilerScope::Comprehension,
            self.line_index_start(range),
        );

        // Mark non-generator comprehensions as inlined (PEP 709)
        // inline_comp = entry->ste_comprehension && !entry->ste_generator && !ste->ste_can_see_class_scope
        // We check is_generator and can_see_class_scope of parent
        let parent_can_see_class = self
            .tables
            .get(self.tables.len().saturating_sub(2))
            .map(|t| t.can_see_class_scope)
            .unwrap_or(false);
        if !is_generator
            && !parent_can_see_class
            && let Some(table) = self.tables.last_mut()
        {
            table.comp_inlined = true;
        }

        // Register the passed argument to the generator function as the name ".0"
        self.register_name(".0", SymbolUsage::Parameter, range)?;

        self.scan_expression(elt1, ExpressionContext::Load)?;
        if let Some(elt2) = elt2 {
            self.scan_expression(elt2, ExpressionContext::Load)?;
        }

        let mut is_first_generator = true;
        for generator in generators {
            // Set flag for INNER_LOOP_CONFLICT check (only for inner loops, not the first)
            if !is_first_generator {
                self.in_comp_inner_loop_target = true;
            }
            self.scan_expression(&generator.target, ExpressionContext::Iter)?;
            self.in_comp_inner_loop_target = false;

            if is_first_generator {
                is_first_generator = false;
            } else {
                self.scan_expression(&generator.iter, ExpressionContext::IterDefinitionExp)?;
            }

            for if_expr in &generator.ifs {
                self.scan_expression(if_expr, ExpressionContext::Load)?;
            }
        }

        self.leave_scope();

        // The first iterable is passed as an argument into the created function:
        assert!(!generators.is_empty());
        self.scan_expression(&generators[0].iter, ExpressionContext::IterDefinitionExp)?;

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
        // Enter a new TypeParams scope for the bound/default expression
        // This allows the expression to access outer scope symbols
        let line_number = self.line_index_start(expr.range());
        self.enter_scope(scope_name, CompilerScope::TypeParams, line_number);

        // Note: In CPython, can_see_class_scope is preserved in the new scope
        // In RustPython, this is handled through the scope hierarchy

        // Set scope_info for better error messages
        let old_scope_info = self.scope_info;
        self.scope_info = Some(scope_info);

        // Scan the expression in this new scope
        let result = self.scan_expression(expr, ExpressionContext::Load);

        // Restore scope_info and exit the scope
        self.scope_info = old_scope_info;
        self.leave_scope();

        result
    }

    fn scan_type_params(&mut self, type_params: &ast::TypeParams) -> SymbolTableResult {
        // Check for duplicate type parameter names
        let mut seen_names: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for type_param in &type_params.type_params {
            let (name, range) = match type_param {
                ast::TypeParam::TypeVar(tv) => (tv.name.as_str(), tv.range),
                ast::TypeParam::ParamSpec(ps) => (ps.name.as_str(), ps.range),
                ast::TypeParam::TypeVarTuple(tvt) => (tvt.name.as_str(), tvt.range),
            };
            if !seen_names.insert(name) {
                return Err(SymbolTableError {
                    error: format!("duplicate type parameter '{}'", name),
                    location: Some(
                        self.source_file
                            .to_source_code()
                            .source_location(range.start(), PositionEncoding::Utf8),
                    ),
                });
            }
        }

        // Register .type_params as a type parameter (automatically becomes cell variable)
        self.register_name(".type_params", SymbolUsage::TypeParam, type_params.range)?;

        // First register all type parameters
        for type_param in &type_params.type_params {
            match type_param {
                ast::TypeParam::TypeVar(ast::TypeParamTypeVar {
                    name,
                    bound,
                    range: type_var_range,
                    default,
                    node_index: _,
                }) => {
                    self.register_name(name.as_str(), SymbolUsage::TypeParam, *type_var_range)?;

                    // Process bound in a separate scope
                    if let Some(binding) = bound {
                        let (scope_name, scope_info) = if binding.is_tuple_expr() {
                            (
                                format!("<TypeVar constraint of {name}>"),
                                "a TypeVar constraint",
                            )
                        } else {
                            (format!("<TypeVar bound of {name}>"), "a TypeVar bound")
                        };
                        self.scan_type_param_bound_or_default(binding, &scope_name, scope_info)?;
                    }

                    // Process default in a separate scope
                    if let Some(default_value) = default {
                        let scope_name = format!("<TypeVar default of {name}>");
                        self.scan_type_param_bound_or_default(
                            default_value,
                            &scope_name,
                            "a TypeVar default",
                        )?;
                    }
                }
                ast::TypeParam::ParamSpec(ast::TypeParamParamSpec {
                    name,
                    range: param_spec_range,
                    default,
                    node_index: _,
                }) => {
                    self.register_name(name, SymbolUsage::TypeParam, *param_spec_range)?;

                    // Process default in a separate scope
                    if let Some(default_value) = default {
                        let scope_name = format!("<ParamSpec default of {name}>");
                        self.scan_type_param_bound_or_default(
                            default_value,
                            &scope_name,
                            "a ParamSpec default",
                        )?;
                    }
                }
                ast::TypeParam::TypeVarTuple(ast::TypeParamTypeVarTuple {
                    name,
                    range: type_var_tuple_range,
                    default,
                    node_index: _,
                }) => {
                    self.register_name(name, SymbolUsage::TypeParam, *type_var_tuple_range)?;

                    // Process default in a separate scope
                    if let Some(default_value) = default {
                        let scope_name = format!("<TypeVarTuple default of {name}>");
                        self.scan_type_param_bound_or_default(
                            default_value,
                            &scope_name,
                            "a TypeVarTuple default",
                        )?;
                    }
                }
            }
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
        use ast::Pattern::*;
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
                    self.register_ident(rest, SymbolUsage::Assigned)?;
                }
            }
            MatchClass(ast::PatternMatchClass { cls, arguments, .. }) => {
                self.scan_expression(cls, ExpressionContext::Load)?;
                self.scan_patterns(&arguments.patterns)?;
                for kw in &arguments.keywords {
                    self.scan_pattern(&kw.pattern)?;
                }
            }
            MatchStar(ast::PatternMatchStar { name, .. }) => {
                if let Some(name) = name {
                    self.register_ident(name, SymbolUsage::Assigned)?;
                }
            }
            MatchAs(ast::PatternMatchAs { pattern, name, .. }) => {
                if let Some(pattern) = pattern {
                    self.scan_pattern(pattern)?;
                }
                if let Some(name) = name {
                    self.register_ident(name, SymbolUsage::Assigned)?;
                }
            }
            MatchOr(ast::PatternMatchOr { patterns, .. }) => self.scan_patterns(patterns)?,
        }
        Ok(())
    }

    fn enter_scope_with_parameters(
        &mut self,
        name: &str,
        parameters: &ast::Parameters,
        line_number: u32,
        has_return_annotation: bool,
    ) -> SymbolTableResult {
        // Evaluate eventual default parameters:
        for default in parameters
            .posonlyargs
            .iter()
            .chain(parameters.args.iter())
            .chain(parameters.kwonlyargs.iter())
            .filter_map(|arg| arg.default.as_ref())
        {
            self.scan_expression(default, ExpressionContext::Load)?; // not ExprContext?
        }

        // Annotations are scanned in outer scope:
        for annotation in parameters
            .posonlyargs
            .iter()
            .chain(parameters.args.iter())
            .chain(parameters.kwonlyargs.iter())
            .filter_map(|arg| arg.parameter.annotation.as_ref())
        {
            self.scan_annotation(annotation)?;
        }
        if let Some(annotation) = parameters
            .vararg
            .as_ref()
            .and_then(|arg| arg.annotation.as_ref())
        {
            self.scan_annotation(annotation)?;
        }
        if let Some(annotation) = parameters
            .kwarg
            .as_ref()
            .and_then(|arg| arg.annotation.as_ref())
        {
            self.scan_annotation(annotation)?;
        }

        // Check if this function has any annotations (parameter or return)
        let has_param_annotations = parameters
            .posonlyargs
            .iter()
            .chain(parameters.args.iter())
            .chain(parameters.kwonlyargs.iter())
            .any(|p| p.parameter.annotation.is_some())
            || parameters
                .vararg
                .as_ref()
                .is_some_and(|p| p.annotation.is_some())
            || parameters
                .kwarg
                .as_ref()
                .is_some_and(|p| p.annotation.is_some());

        let has_any_annotations = has_param_annotations || has_return_annotation;

        // Take annotation_block if this function has any annotations.
        // When in class scope, the class's annotation_block was saved before scanning
        // function annotations, so the current annotation_block belongs to this function.
        let annotation_block = if has_any_annotations {
            self.tables.last_mut().unwrap().annotation_block.take()
        } else {
            None
        };

        self.enter_scope(name, CompilerScope::Function, line_number);

        // Move annotation_block to function scope only if we have one
        if let Some(block) = annotation_block {
            self.tables.last_mut().unwrap().annotation_block = Some(block);
        }

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

        // Note: __debug__ checks are handled by check_name function, so no check needed here.

        let scope_depth = self.tables.len();
        let table = self.tables.last_mut().unwrap();

        let name = mangle_name(self.class_name.as_deref(), name);
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
                        "comprehension inner loop cannot rebind assignment expression target '{}'",
                        name
                    ),
                    location,
                });
            }

            // Role already set..
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
                        error: format!("cannot define nonlocal '{name}' at top level."),
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
                // Local variables (assigned) are added to varnames if they are local scope
                // and not already in varnames
                if symbol.scope == SymbolScope::Local {
                    let name_str = symbol.name.clone();
                    if !self.current_varnames.contains(&name_str) {
                        self.current_varnames.push(name_str);
                    }
                }
            }
            SymbolUsage::AssignedNamedExprInComprehension => {
                flags.insert(SymbolFlags::ASSIGNED | SymbolFlags::ASSIGNED_IN_COMPREHENSION);
                // Named expressions in comprehensions might also be locals
                if symbol.scope == SymbolScope::Local {
                    let name_str = symbol.name.clone();
                    if !self.current_varnames.contains(&name_str) {
                        self.current_varnames.push(name_str);
                    }
                }
            }
            SymbolUsage::Global => {
                symbol.scope = SymbolScope::GlobalExplicit;
                flags.insert(SymbolFlags::GLOBAL);
            }
            SymbolUsage::Used => {
                flags.insert(SymbolFlags::REFERENCED);
            }
            SymbolUsage::Iter => {
                flags.insert(SymbolFlags::ITER);
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

pub(crate) fn mangle_name<'a>(class_name: Option<&str>, name: &'a str) -> Cow<'a, str> {
    let class_name = match class_name {
        Some(n) => n,
        None => return name.into(),
    };
    if !name.starts_with("__") || name.ends_with("__") || name.contains('.') {
        return name.into();
    }
    // strip leading underscore
    let class_name = class_name.strip_prefix(|c| c == '_').unwrap_or(class_name);
    let mut ret = String::with_capacity(1 + class_name.len() + name.len());
    ret.push('_');
    ret.push_str(class_name);
    ret.push_str(name);
    ret.into()
}
