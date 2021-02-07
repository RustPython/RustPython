/* Python code is pre-scanned for symbols in the ast.

This ensures that global and nonlocal keywords are picked up.
Then the compiler can use the symbol table to generate proper
load and store instructions for names.

Inspirational file: https://github.com/python/cpython/blob/master/Python/symtable.c
*/

use crate::error::{CompileError, CompileErrorType};
use crate::IndexMap;
use rustpython_ast::{self as ast, Location};
use std::borrow::Cow;
use std::fmt;

pub fn make_symbol_table(program: &[ast::Stmt]) -> SymbolTableResult<SymbolTable> {
    let mut builder = SymbolTableBuilder::new();
    builder.scan_statements(program)?;
    builder.finish()
}

pub fn make_symbol_table_expr(expr: &ast::Expr) -> SymbolTableResult<SymbolTable> {
    let mut builder = SymbolTableBuilder::new();
    builder.scan_expression(expr, ExpressionContext::Load)?;
    builder.finish()
}

/// Captures all symbols in the current scope, and has a list of subscopes in this scope.
#[derive(Clone)]
pub struct SymbolTable {
    /// The name of this symbol table. Often the name of the class or function.
    pub name: String,

    /// The type of symbol table
    pub typ: SymbolTableType,

    /// The line number in the sourcecode where this symboltable begins.
    pub line_number: usize,

    // Return True if the block is a nested class or function
    pub is_nested: bool,

    /// A set of symbols present on this scope level.
    pub symbols: IndexMap<String, Symbol>,

    /// A list of subscopes in the order as found in the
    /// AST nodes.
    pub sub_tables: Vec<SymbolTable>,
}

impl SymbolTable {
    fn new(name: String, typ: SymbolTableType, line_number: usize, is_nested: bool) -> Self {
        SymbolTable {
            name,
            typ,
            line_number,
            is_nested,
            symbols: IndexMap::default(),
            sub_tables: vec![],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SymbolTableType {
    Module,
    Class,
    Function,
    Comprehension,
}

impl fmt::Display for SymbolTableType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            SymbolTableType::Module => write!(f, "module"),
            SymbolTableType::Class => write!(f, "class"),
            SymbolTableType::Function => write!(f, "function"),
            SymbolTableType::Comprehension => write!(f, "comprehension"),
        }
    }
}

/// Indicator for a single symbol what the scope of this symbol is.
/// The scope can be unknown, which is unfortunate, but not impossible.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SymbolScope {
    Unknown,
    Local,
    GlobalExplicit,
    GlobalImplicit,
    Free,
    Cell,
}

/// A single symbol in a table. Has various properties such as the scope
/// of the symbol, and also the various uses of the symbol.
#[derive(Debug, Clone)]
pub struct Symbol {
    pub name: String,
    // pub table: SymbolTableRef,
    pub scope: SymbolScope,
    // TODO: Use bitflags replace
    pub is_referenced: bool,
    pub is_assigned: bool,
    pub is_parameter: bool,
    pub is_annotated: bool,
    pub is_imported: bool,
    pub is_nonlocal: bool,

    // indicates if the symbol gets a value assigned by a named expression in a comprehension
    // this is required to correct the scope in the analysis.
    pub is_assign_namedexpr_in_comprehension: bool,

    // inidicates that the symbol is used a bound iterator variable. We distinguish this case
    // from normal assignment to detect unallowed re-assignment to iterator variables.
    pub is_iter: bool,

    /// indicates that the symbol is a free variable in a class method from the scope that the
    /// class is defined in, e.g.:
    /// ```python
    /// def foo(x):
    ///     class A:
    ///         def method(self):
    ///             return x // is_free_class
    /// ```
    pub is_free_class: bool,
}

impl Symbol {
    fn new(name: &str) -> Self {
        Symbol {
            name: name.to_owned(),
            // table,
            scope: SymbolScope::Unknown,
            is_referenced: false,
            is_assigned: false,
            is_parameter: false,
            is_annotated: false,
            is_imported: false,
            is_nonlocal: false,
            is_assign_namedexpr_in_comprehension: false,
            is_iter: false,
            is_free_class: false,
        }
    }

    pub fn is_global(&self) -> bool {
        matches!(
            self.scope,
            SymbolScope::GlobalExplicit | SymbolScope::GlobalImplicit
        )
    }

    pub fn is_local(&self) -> bool {
        self.scope == SymbolScope::Local
    }

    pub fn is_bound(&self) -> bool {
        self.is_assigned || self.is_parameter || self.is_imported || self.is_iter
    }
}

#[derive(Debug)]
pub struct SymbolTableError {
    error: String,
    location: Location,
}

impl SymbolTableError {
    pub fn into_compile_error(self, source_path: String) -> CompileError {
        CompileError {
            error: CompileErrorType::SyntaxError(self.error),
            location: self.location,
            source_path,
        }
    }
}

type SymbolTableResult<T = ()> = Result<T, SymbolTableError>;

impl SymbolTable {
    pub fn lookup(&self, name: &str) -> Option<&Symbol> {
        self.symbols.get(name)
    }
}

impl std::fmt::Debug for SymbolTable {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            f,
            "SymbolTable({:?} symbols, {:?} sub scopes)",
            self.symbols.len(),
            self.sub_tables.len()
        )
    }
}

/* Perform some sort of analysis on nonlocals, globals etc..
  See also: https://github.com/python/cpython/blob/master/Python/symtable.c#L410
*/
fn analyze_symbol_table(symbol_table: &mut SymbolTable) -> SymbolTableResult {
    let mut analyzer = SymbolTableAnalyzer::default();
    analyzer.analyze_symbol_table(symbol_table)
}

type SymbolMap = IndexMap<String, Symbol>;

mod stack {
    use std::panic;
    use std::ptr::NonNull;
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

        pub fn iter(&self) -> impl Iterator<Item = &T> + DoubleEndedIterator + '_ {
            self.as_ref().iter().copied()
        }
        pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut T> + DoubleEndedIterator + '_ {
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
    tables: StackStack<(SymbolMap, SymbolTableType)>,
}

impl SymbolTableAnalyzer {
    fn analyze_symbol_table(&mut self, symbol_table: &mut SymbolTable) -> SymbolTableResult {
        let symbols = std::mem::take(&mut symbol_table.symbols);
        let sub_tables = &mut *symbol_table.sub_tables;

        let mut info = (symbols, symbol_table.typ);
        self.tables.with_append(&mut info, |list| {
            let inner_scope = unsafe { &mut *(list as *mut _ as *mut SymbolTableAnalyzer) };
            // Analyze sub scopes:
            for sub_table in sub_tables.iter_mut() {
                inner_scope.analyze_symbol_table(sub_table)?;
            }
            Ok(())
        })?;

        symbol_table.symbols = info.0;

        // Analyze symbols:
        for symbol in symbol_table.symbols.values_mut() {
            self.analyze_symbol(symbol, symbol_table.typ, sub_tables)?;
        }
        Ok(())
    }

    fn analyze_symbol(
        &mut self,
        symbol: &mut Symbol,
        st_typ: SymbolTableType,
        sub_tables: &mut [SymbolTable],
    ) -> SymbolTableResult {
        if symbol.is_assign_namedexpr_in_comprehension && st_typ == SymbolTableType::Comprehension {
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
                            || self.found_in_outer_scope(&symbol.name) != Some(SymbolScope::Free)
                        {
                            return Err(SymbolTableError {
                                error: format!("no binding for nonlocal '{}' found", symbol.name),
                                // TODO: accurate location info, somehow
                                location: Location::default(),
                            });
                        }
                    } else {
                        return Err(SymbolTableError {
                            error: format!(
                                "nonlocal {} defined at place without an enclosing scope",
                                symbol.name
                            ),
                            // TODO: accurate location info, somehow
                            location: Location::default(),
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
                    // Try hard to figure out what the scope of this symbol is.
                    let scope = if symbol.is_bound() {
                        self.found_in_inner_scope(sub_tables, &symbol.name, st_typ)
                            .unwrap_or(SymbolScope::Local)
                    } else if let Some(scope) = self.found_in_outer_scope(&symbol.name) {
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

    fn found_in_outer_scope(&mut self, name: &str) -> Option<SymbolScope> {
        let mut decl_depth = None;
        for (i, (symbols, typ)) in self.tables.iter().rev().enumerate() {
            if matches!(typ, SymbolTableType::Module)
                || matches!(typ, SymbolTableType::Class if name != "__class__")
            {
                continue;
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
                if let SymbolTableType::Class = typ {
                    if let Some(free_class) = table.get_mut(name) {
                        free_class.is_free_class = true;
                    } else {
                        let mut symbol = Symbol::new(name);
                        symbol.is_free_class = true;
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
        st_typ: SymbolTableType,
    ) -> Option<SymbolScope> {
        sub_tables.iter().find_map(|st| {
            st.symbols.get(name).and_then(|sym| {
                if sym.scope == SymbolScope::Free || sym.is_free_class {
                    if st_typ == SymbolTableType::Class && name != "__class__" {
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
        if symbol.is_iter {
            return Err(SymbolTableError {
                error: format!(
                    "assignment expression cannot rebind comprehension iteration variable {}",
                    symbol.name
                ),
                // TODO: accurate location info, somehow
                location: Location::default(),
            });
        }

        match table_type {
            SymbolTableType::Module => {
                symbol.scope = SymbolScope::GlobalImplicit;
            }
            SymbolTableType::Class => {
                // named expressions are forbidden in comprehensions on class scope
                return Err(SymbolTableError {
                    error: "assignment expression within a comprehension cannot be used in a class body".to_string(),
                    // TODO: accurate location info, somehow
                    location: Location::default(),
                });
            }
            SymbolTableType::Function => {
                if let Some(parent_symbol) = symbols.get_mut(&symbol.name) {
                    if let SymbolScope::Unknown = parent_symbol.scope {
                        // this information is new, as the asignment is done in inner scope
                        parent_symbol.is_assigned = true;
                    }

                    symbol.scope = if parent_symbol.is_global() {
                        parent_symbol.scope
                    } else {
                        SymbolScope::Free
                    };
                } else {
                    let mut cloned_sym = symbol.clone();
                    cloned_sym.scope = SymbolScope::Local;
                    last.0.insert(cloned_sym.name.to_owned(), cloned_sym);
                }
            }
            SymbolTableType::Comprehension => {
                // TODO check for conflicts - requires more context information about variables
                match symbols.get_mut(&symbol.name) {
                    Some(parent_symbol) => {
                        // check if assignee is an iterator in top scope
                        if parent_symbol.is_iter {
                            return Err(SymbolTableError {
                                error: format!("assignment expression cannot rebind comprehension iteration variable {}", symbol.name),
                                // TODO: accurate location info, somehow
                                location: Location::default(),
                            });
                        }

                        // we synthesize the assignment to the symbol from inner scope
                        parent_symbol.is_assigned = true; // more checks are required
                    }
                    None => {
                        // extend the scope of the inner symbol
                        // as we are in a nested comprehension, we expect that the symbol is needed
                        // ouside, too, and set it therefore to non-local scope. I.e., we expect to
                        // find a definition on a higher level
                        let mut cloned_sym = symbol.clone();
                        cloned_sym.scope = SymbolScope::Free;
                        last.0.insert(cloned_sym.name.to_owned(), cloned_sym);
                    }
                }

                self.analyze_symbol_comprehension(symbol, parent_offset + 1)?;
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
    AssignedNamedExprInCompr,
    Iter,
}

struct SymbolTableBuilder {
    class_name: Option<String>,
    // Scope stack.
    tables: Vec<SymbolTable>,
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
    fn new() -> Self {
        let mut this = Self {
            class_name: None,
            tables: vec![],
        };
        this.enter_scope("top", SymbolTableType::Module, 0);
        this
    }

    fn finish(mut self) -> Result<SymbolTable, SymbolTableError> {
        assert_eq!(self.tables.len(), 1);
        let mut symbol_table = self.tables.pop().unwrap();
        analyze_symbol_table(&mut symbol_table)?;
        Ok(symbol_table)
    }

    fn enter_scope(&mut self, name: &str, typ: SymbolTableType, line_number: usize) {
        let is_nested = self
            .tables
            .last()
            .map(|table| table.is_nested || table.typ == SymbolTableType::Function)
            .unwrap_or(false);
        let table = SymbolTable::new(name.to_owned(), typ, line_number, is_nested);
        self.tables.push(table);
    }

    /// Pop symbol table and add to sub table of parent table.
    fn leave_scope(&mut self) {
        let table = self.tables.pop().unwrap();
        self.tables.last_mut().unwrap().sub_tables.push(table);
    }

    fn scan_statements(&mut self, statements: &[ast::Stmt]) -> SymbolTableResult {
        for statement in statements {
            self.scan_statement(statement)?;
        }
        Ok(())
    }

    fn scan_parameters(&mut self, parameters: &[ast::Arg]) -> SymbolTableResult {
        for parameter in parameters {
            self.scan_parameter(parameter)?;
        }
        Ok(())
    }

    fn scan_parameter(&mut self, parameter: &ast::Arg) -> SymbolTableResult {
        let usage = if parameter.node.annotation.is_some() {
            SymbolUsage::AnnotationParameter
        } else {
            SymbolUsage::Parameter
        };
        self.register_name(&parameter.node.arg, usage, parameter.location)
    }

    fn scan_parameters_annotations(&mut self, parameters: &[ast::Arg]) -> SymbolTableResult {
        for parameter in parameters {
            self.scan_parameter_annotation(parameter)?;
        }
        Ok(())
    }

    fn scan_parameter_annotation(&mut self, parameter: &ast::Arg) -> SymbolTableResult {
        if let Some(annotation) = &parameter.node.annotation {
            self.scan_expression(&annotation, ExpressionContext::Load)?;
        }
        Ok(())
    }

    fn scan_statement(&mut self, statement: &ast::Stmt) -> SymbolTableResult {
        use ast::StmtKind::*;
        let location = statement.location;
        match &statement.node {
            Global { names } => {
                for name in names {
                    self.register_name(name, SymbolUsage::Global, location)?;
                }
            }
            Nonlocal { names } => {
                for name in names {
                    self.register_name(name, SymbolUsage::Nonlocal, location)?;
                }
            }
            FunctionDef {
                name,
                body,
                args,
                decorator_list,
                returns,
                ..
            }
            | AsyncFunctionDef {
                name,
                body,
                args,
                decorator_list,
                returns,
                ..
            } => {
                self.scan_expressions(decorator_list, ExpressionContext::Load)?;
                self.register_name(name, SymbolUsage::Assigned, location)?;
                if let Some(expression) = returns {
                    self.scan_expression(expression, ExpressionContext::Load)?;
                }
                self.enter_function(name, args, location.row())?;
                self.scan_statements(body)?;
                self.leave_scope();
            }
            ClassDef {
                name,
                body,
                bases,
                keywords,
                decorator_list,
            } => {
                self.enter_scope(name, SymbolTableType::Class, location.row());
                let prev_class = std::mem::replace(&mut self.class_name, Some(name.to_owned()));
                self.register_name("__module__", SymbolUsage::Assigned, location)?;
                self.register_name("__qualname__", SymbolUsage::Assigned, location)?;
                self.register_name("__doc__", SymbolUsage::Assigned, location)?;
                self.register_name("__class__", SymbolUsage::Assigned, location)?;
                self.scan_statements(body)?;
                self.leave_scope();
                self.class_name = prev_class;
                self.scan_expressions(bases, ExpressionContext::Load)?;
                for keyword in keywords {
                    self.scan_expression(&keyword.node.value, ExpressionContext::Load)?;
                }
                self.scan_expressions(decorator_list, ExpressionContext::Load)?;
                self.register_name(name, SymbolUsage::Assigned, location)?;
            }
            Expr { value } => self.scan_expression(value, ExpressionContext::Load)?,
            If { test, body, orelse } => {
                self.scan_expression(test, ExpressionContext::Load)?;
                self.scan_statements(body)?;
                self.scan_statements(orelse)?;
            }
            For {
                target,
                iter,
                body,
                orelse,
                ..
            }
            | AsyncFor {
                target,
                iter,
                body,
                orelse,
                ..
            } => {
                self.scan_expression(target, ExpressionContext::Store)?;
                self.scan_expression(iter, ExpressionContext::Load)?;
                self.scan_statements(body)?;
                self.scan_statements(orelse)?;
            }
            While { test, body, orelse } => {
                self.scan_expression(test, ExpressionContext::Load)?;
                self.scan_statements(body)?;
                self.scan_statements(orelse)?;
            }
            Break | Continue | Pass => {
                // No symbols here.
            }
            Import { names } | ImportFrom { names, .. } => {
                for name in names {
                    if let Some(alias) = &name.asname {
                        // `import mymodule as myalias`
                        self.register_name(alias, SymbolUsage::Imported, location)?;
                    } else {
                        // `import module`
                        self.register_name(
                            name.name.split('.').next().unwrap(),
                            SymbolUsage::Imported,
                            location,
                        )?;
                    }
                }
            }
            Return { value } => {
                if let Some(expression) = value {
                    self.scan_expression(expression, ExpressionContext::Load)?;
                }
            }
            Assert { test, msg } => {
                self.scan_expression(test, ExpressionContext::Load)?;
                if let Some(expression) = msg {
                    self.scan_expression(expression, ExpressionContext::Load)?;
                }
            }
            Delete { targets } => {
                self.scan_expressions(targets, ExpressionContext::Delete)?;
            }
            Assign { targets, value, .. } => {
                self.scan_expressions(targets, ExpressionContext::Store)?;
                self.scan_expression(value, ExpressionContext::Load)?;
            }
            AugAssign { target, value, .. } => {
                self.scan_expression(target, ExpressionContext::Store)?;
                self.scan_expression(value, ExpressionContext::Load)?;
            }
            AnnAssign {
                target,
                annotation,
                value,
                simple,
            } => {
                // https://github.com/python/cpython/blob/master/Python/symtable.c#L1233
                match &target.node {
                    ast::ExprKind::Name { id, .. } if *simple => {
                        self.register_name(id, SymbolUsage::AnnotationAssigned, location)?;
                    }
                    _ => {
                        self.scan_expression(target, ExpressionContext::Store)?;
                    }
                }
                self.scan_expression(annotation, ExpressionContext::Load)?;
                if let Some(value) = value {
                    self.scan_expression(value, ExpressionContext::Load)?;
                }
            }
            With { items, body, .. } | AsyncWith { items, body, .. } => {
                for item in items {
                    self.scan_expression(&item.context_expr, ExpressionContext::Load)?;
                    if let Some(expression) = &item.optional_vars {
                        self.scan_expression(expression, ExpressionContext::Store)?;
                    }
                }
                self.scan_statements(body)?;
            }
            Try {
                body,
                handlers,
                orelse,
                finalbody,
            } => {
                self.scan_statements(body)?;
                for handler in handlers {
                    let ast::ExcepthandlerKind::ExceptHandler { type_, name, body } = &handler.node;
                    if let Some(expression) = type_ {
                        self.scan_expression(expression, ExpressionContext::Load)?;
                    }
                    if let Some(name) = name {
                        self.register_name(name, SymbolUsage::Assigned, location)?;
                    }
                    self.scan_statements(body)?;
                }
                self.scan_statements(orelse)?;
                self.scan_statements(finalbody)?;
            }
            Raise { exc, cause } => {
                if let Some(expression) = exc {
                    self.scan_expression(expression, ExpressionContext::Load)?;
                }
                if let Some(expression) = cause {
                    self.scan_expression(expression, ExpressionContext::Load)?;
                }
            }
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
        use ast::ExprKind::*;
        let location = expression.location;
        match &expression.node {
            BinOp { left, right, .. } => {
                self.scan_expression(left, context)?;
                self.scan_expression(right, context)?;
            }
            BoolOp { values, .. } => {
                self.scan_expressions(values, context)?;
            }
            Compare {
                left, comparators, ..
            } => {
                self.scan_expression(left, context)?;
                self.scan_expressions(comparators, context)?;
            }
            Subscript { value, slice, .. } => {
                self.scan_expression(value, ExpressionContext::Load)?;
                self.scan_expression(slice, ExpressionContext::Load)?;
            }
            Attribute { value, .. } => {
                self.scan_expression(value, ExpressionContext::Load)?;
            }
            Dict { keys, values } => {
                for (key, value) in keys.iter().zip(values) {
                    if let Some(key) = key {
                        self.scan_expression(key, context)?;
                    } else {
                        // dict unpacking marker
                    }
                    self.scan_expression(value, context)?;
                }
            }
            Await { value } => {
                self.scan_expression(value, context)?;
            }
            Yield { value } => {
                if let Some(expression) = value {
                    self.scan_expression(expression, context)?;
                }
            }
            YieldFrom { value } => {
                self.scan_expression(value, context)?;
            }
            UnaryOp { operand, .. } => {
                self.scan_expression(operand, context)?;
            }
            Constant { .. } => {}
            Starred { value, .. } => {
                self.scan_expression(value, context)?;
            }
            Tuple { elts, .. } | Set { elts, .. } | List { elts, .. } => {
                self.scan_expressions(elts, context)?;
            }
            Slice { lower, upper, step } => {
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
            GeneratorExp { elt, generators } => {
                self.scan_comprehension("genexpr", elt, None, generators, location)?;
            }
            ListComp { elt, generators } => {
                self.scan_comprehension("genexpr", elt, None, generators, location)?;
            }
            SetComp { elt, generators } => {
                self.scan_comprehension("genexpr", elt, None, generators, location)?;
            }
            DictComp {
                key,
                value,
                generators,
            } => {
                self.scan_comprehension("genexpr", key, Some(value), generators, location)?;
            }
            Call {
                func,
                args,
                keywords,
            } => {
                match context {
                    ExpressionContext::IterDefinitionExp => {
                        self.scan_expression(func, ExpressionContext::IterDefinitionExp)?;
                    }
                    _ => {
                        self.scan_expression(func, ExpressionContext::Load)?;
                    }
                }

                self.scan_expressions(args, ExpressionContext::Load)?;
                for keyword in keywords {
                    self.scan_expression(&keyword.node.value, ExpressionContext::Load)?;
                }
            }
            FormattedValue {
                value, format_spec, ..
            } => {
                self.scan_expression(value, ExpressionContext::Load)?;
                if let Some(spec) = format_spec {
                    self.scan_expression(spec, ExpressionContext::Load)?;
                }
            }
            JoinedStr { values } => {
                for value in values {
                    self.scan_expression(value, ExpressionContext::Load)?;
                }
            }
            Name { id, .. } => {
                // Determine the contextual usage of this symbol:
                match context {
                    ExpressionContext::Delete => {
                        self.register_name(id, SymbolUsage::Assigned, location)?;
                        self.register_name(id, SymbolUsage::Used, location)?;
                    }
                    ExpressionContext::Load | ExpressionContext::IterDefinitionExp => {
                        self.register_name(id, SymbolUsage::Used, location)?;
                    }
                    ExpressionContext::Store => {
                        self.register_name(id, SymbolUsage::Assigned, location)?;
                    }
                    ExpressionContext::Iter => {
                        self.register_name(id, SymbolUsage::Iter, location)?;
                    }
                }
                // Interesting stuff about the __class__ variable:
                // https://docs.python.org/3/reference/datamodel.html?highlight=__class__#creating-the-class-object
                if context == ExpressionContext::Load
                    && self.tables.last().unwrap().typ == SymbolTableType::Function
                    && id == "super"
                {
                    self.register_name("__class__", SymbolUsage::Used, location)?;
                }
            }
            Lambda { args, body } => {
                self.enter_function("lambda", args, expression.location.row())?;
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
            IfExp { test, body, orelse } => {
                self.scan_expression(test, ExpressionContext::Load)?;
                self.scan_expression(body, ExpressionContext::Load)?;
                self.scan_expression(orelse, ExpressionContext::Load)?;
            }

            NamedExpr { target, value } => {
                // named expressions are not allowed in the definiton of
                // comprehension iterator definitions
                if let ExpressionContext::IterDefinitionExp = context {
                    return Err(SymbolTableError {
                        error: "assignment expression cannot be used in a comprehension iterable expression".to_string(),
                        // TODO: accurate location info, somehow
                        location: Location::default(),
                    });
                }

                self.scan_expression(value, ExpressionContext::Load)?;

                // special handling for assigned identifier in named expressions
                // that are used in comprehensions. This required to correctly
                // propagate the scope of the named assigned named and not to
                // propagate inner names.
                if let Name { id, .. } = &target.node {
                    let table = self.tables.last().unwrap();
                    if table.typ == SymbolTableType::Comprehension {
                        self.register_name(id, SymbolUsage::AssignedNamedExprInCompr, location)?;
                    } else {
                        // omit one recursion. When the handling of an store changes for
                        // Identifiers this needs adapted - more forward safe would be
                        // calling scan_expression directly.
                        self.register_name(id, SymbolUsage::Assigned, location)?;
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
        location: Location,
    ) -> SymbolTableResult {
        // Comprehensions are compiled as functions, so create a scope for them:

        self.enter_scope(scope_name, SymbolTableType::Comprehension, location.row());

        // Register the passed argument to the generator function as the name ".0"
        self.register_name(".0", SymbolUsage::Parameter, location)?;

        self.scan_expression(elt1, ExpressionContext::Load)?;
        if let Some(elt2) = elt2 {
            self.scan_expression(elt2, ExpressionContext::Load)?;
        }

        let mut is_first_generator = true;
        for generator in generators {
            self.scan_expression(&generator.target, ExpressionContext::Iter)?;
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

    fn enter_function(
        &mut self,
        name: &str,
        args: &ast::Arguments,
        line_number: usize,
    ) -> SymbolTableResult {
        // Evaluate eventual default parameters:
        self.scan_expressions(&args.defaults, ExpressionContext::Load)?;
        for kw_default in &args.kw_defaults {
            if let Some(expression) = kw_default {
                self.scan_expression(&expression, ExpressionContext::Load)?;
            }
        }

        // Annotations are scanned in outer scope:
        self.scan_parameters_annotations(&args.posonlyargs)?;
        self.scan_parameters_annotations(&args.args)?;
        self.scan_parameters_annotations(&args.kwonlyargs)?;
        if let Some(name) = &args.vararg {
            self.scan_parameter_annotation(name)?;
        }
        if let Some(name) = &args.kwarg {
            self.scan_parameter_annotation(name)?;
        }

        self.enter_scope(name, SymbolTableType::Function, line_number);

        // Fill scope with parameter names:
        self.scan_parameters(&args.posonlyargs)?;
        self.scan_parameters(&args.args)?;
        self.scan_parameters(&args.kwonlyargs)?;
        if let Some(name) = &args.vararg {
            self.scan_parameter(name)?;
        }
        if let Some(name) = &args.kwarg {
            self.scan_parameter(name)?;
        }
        Ok(())
    }

    fn register_name(
        &mut self,
        name: &str,
        role: SymbolUsage,
        location: Location,
    ) -> SymbolTableResult {
        let scope_depth = self.tables.len();
        let table = self.tables.last_mut().unwrap();

        let name = mangle_name(self.class_name.as_deref(), name);

        // Some checks for the symbol that present on this scope level:
        let symbol = if let Some(symbol) = table.symbols.get_mut(name.as_ref()) {
            // Role already set..
            match role {
                SymbolUsage::Global => {
                    if !symbol.is_global() {
                        return Err(SymbolTableError {
                            error: format!("name '{}' is used prior to global declaration", name),
                            location,
                        });
                    }
                }
                SymbolUsage::Nonlocal => {
                    if symbol.is_parameter {
                        return Err(SymbolTableError {
                            error: format!("name '{}' is parameter and nonlocal", name),
                            location,
                        });
                    }
                    if symbol.is_referenced {
                        return Err(SymbolTableError {
                            error: format!("name '{}' is used prior to nonlocal declaration", name),
                            location,
                        });
                    }
                    if symbol.is_annotated {
                        return Err(SymbolTableError {
                            error: format!("annotated name '{}' can't be nonlocal", name),
                            location,
                        });
                    }
                    if symbol.is_assigned {
                        return Err(SymbolTableError {
                            error: format!(
                                "name '{}' is assigned to before nonlocal declaration",
                                name
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
                        error: format!("cannot define nonlocal '{}' at top level.", name),
                        location,
                    })
                }
                _ => {
                    // Ok!
                }
            }
            // Insert symbol when required:
            let symbol = Symbol::new(name.as_ref());
            table.symbols.entry(name.into_owned()).or_insert(symbol)
        };

        // Set proper flags on symbol:
        match role {
            SymbolUsage::Nonlocal => {
                symbol.scope = SymbolScope::Free;
                symbol.is_nonlocal = true;
            }
            SymbolUsage::Imported => {
                symbol.is_assigned = true;
                symbol.is_imported = true;
            }
            SymbolUsage::Parameter => {
                symbol.is_parameter = true;
            }
            SymbolUsage::AnnotationParameter => {
                symbol.is_parameter = true;
                symbol.is_annotated = true;
            }
            SymbolUsage::AnnotationAssigned => {
                symbol.is_assigned = true;
                symbol.is_annotated = true;
            }
            SymbolUsage::Assigned => {
                symbol.is_assigned = true;
            }
            SymbolUsage::AssignedNamedExprInCompr => {
                symbol.is_assigned = true;
                symbol.is_assign_namedexpr_in_comprehension = true;
            }
            SymbolUsage::Global => {
                symbol.scope = SymbolScope::GlobalExplicit;
            }
            SymbolUsage::Used => {
                symbol.is_referenced = true;
            }
            SymbolUsage::Iter => {
                symbol.is_iter = true;
            }
        }

        // and even more checking
        // it is not allowed to assign to iterator variables (by named expressions)
        if symbol.is_iter && symbol.is_assigned
        /*&& symbol.is_assign_namedexpr_in_comprehension*/
        {
            return Err(SymbolTableError {
                error:
                    "assignment expression cannot be used in a comprehension iterable expression"
                        .to_string(),
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
