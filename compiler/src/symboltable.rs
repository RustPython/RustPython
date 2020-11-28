/* Python code is pre-scanned for symbols in the ast.

This ensures that global and nonlocal keywords are picked up.
Then the compiler can use the symbol table to generate proper
load and store instructions for names.

Inspirational file: https://github.com/python/cpython/blob/master/Python/symtable.c
*/

use crate::error::{CompileError, CompileErrorType};
use indexmap::map::IndexMap;
use rustpython_ast::{self as ast, Location};
use std::fmt;

pub fn make_symbol_table(program: &ast::Program) -> Result<SymbolTable, SymbolTableError> {
    let mut builder = SymbolTableBuilder::default();
    builder.prepare();
    builder.scan_program(program)?;
    builder.finish()
}

pub fn statements_to_symbol_table(
    statements: &[ast::Statement],
) -> Result<SymbolTable, SymbolTableError> {
    let mut builder = SymbolTableBuilder::default();
    builder.prepare();
    builder.scan_statements(statements)?;
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
            symbols: IndexMap::new(),
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
#[derive(Debug, Clone, Copy)]
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

    // indicates if the symbol gets a value assigned by a named expression in a comprehension
    // this is required to correct the scope in the analysis.
    pub is_assign_namedexpr_in_comprehension: bool,

    // inidicates that the symbol is used a bound iterator variable. We distinguish this case
    // from normal assignment to detect unallowed re-assignment to iterator variables.
    pub is_iter: bool,
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
            is_assign_namedexpr_in_comprehension: false,
            is_iter: false,
        }
    }

    pub fn is_global(&self) -> bool {
        matches!(
            self.scope,
            SymbolScope::GlobalExplicit | SymbolScope::GlobalImplicit
        )
    }

    pub fn is_local(&self) -> bool {
        matches!(self.scope, SymbolScope::Local)
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

type SymbolTableResult = Result<(), SymbolTableError>;

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

/// Symbol table analysis. Can be used to analyze a fully
/// build symbol table structure. It will mark variables
/// as local variables for example.
#[derive(Default)]
struct SymbolTableAnalyzer<'a> {
    tables: Vec<(&'a mut IndexMap<String, Symbol>, SymbolTableType)>,
}

impl<'a> SymbolTableAnalyzer<'a> {
    fn analyze_symbol_table(&mut self, symbol_table: &'a mut SymbolTable) -> SymbolTableResult {
        let symbols = &mut symbol_table.symbols;
        let sub_tables = &mut symbol_table.sub_tables;

        self.tables.push((symbols, symbol_table.typ));
        // Analyze sub scopes:
        for sub_table in sub_tables {
            self.analyze_symbol_table(sub_table)?;
        }
        let (symbols, st_typ) = self.tables.pop().unwrap();

        // Analyze symbols:
        for symbol in symbols.values_mut() {
            self.analyze_symbol(symbol, st_typ)?;
        }
        Ok(())
    }

    fn analyze_symbol(
        &mut self,
        symbol: &mut Symbol,
        curr_st_typ: SymbolTableType,
    ) -> SymbolTableResult {
        if symbol.is_assign_namedexpr_in_comprehension
            && curr_st_typ == SymbolTableType::Comprehension
        {
            // propagate symbol to next higher level that can hold it,
            // i.e., function or module. Comprehension is skipped and
            // Class is not allowed and detected as error.
            //symbol.scope = SymbolScope::Nonlocal;
            self.analyze_symbol_comprehension(symbol, 0)?
        } else {
            match symbol.scope {
                SymbolScope::Free => {
                    let scope_depth = self.tables.len();
                    if scope_depth > 0 {
                        // check if the name is already defined in any outer scope
                        // therefore
                        if scope_depth < 2 || !self.found_in_outer_scope(symbol) {
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
                    self.analyze_unknown_symbol(symbol);
                }
            }
        }
        Ok(())
    }

    fn found_in_outer_scope(&self, symbol: &Symbol) -> bool {
        // Interesting stuff about the __class__ variable:
        // https://docs.python.org/3/reference/datamodel.html?highlight=__class__#creating-the-class-object
        symbol.name == "__class__"
            || self.tables.iter().skip(1).rev().any(|(symbols, typ)| {
                *typ != SymbolTableType::Class
                    && symbols
                        .get(&symbol.name)
                        .map_or(false, |sym| sym.is_local() && sym.is_assigned)
            })
    }

    fn analyze_unknown_symbol(&self, symbol: &mut Symbol) {
        let scope = if symbol.is_assigned || symbol.is_parameter {
            SymbolScope::Local
        } else if self.found_in_outer_scope(symbol) {
            // Symbol is in some outer scope.
            SymbolScope::Free
        } else if self.tables.is_empty() {
            // Don't make assumptions when we don't know.
            SymbolScope::Unknown
        } else {
            // If there are scopes above we assume global.
            SymbolScope::GlobalImplicit
        };
        symbol.scope = scope;
    }

    // Implements the symbol analysis and scope extension for names
    // assigned by a named expression in a comprehension. See:
    // https://github.com/python/cpython/blob/7b78e7f9fd77bb3280ee39fb74b86772a7d46a70/Python/symtable.c#L1435
    fn analyze_symbol_comprehension(
        &mut self,
        symbol: &mut Symbol,
        parent_offset: usize,
    ) -> SymbolTableResult {
        // TODO: quite C-ish way to implement the iteration
        // when this is called, we expect to be in the direct parent scope of the scope that contains 'symbol'
        let offs = self.tables.len() - 1 - parent_offset;
        let last = self.tables.get_mut(offs).unwrap();
        let symbols = &mut *last.0;
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
                        //self.analyze_unknown_symbol(symbol); // not needed, symbol is analyzed anyhow when its scope is analyzed
                    }

                    if !symbol.is_global() {
                        symbol.scope = SymbolScope::Free;
                    }
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

#[derive(Default)]
struct SymbolTableBuilder {
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
    fn prepare(&mut self) {
        self.enter_scope("top", SymbolTableType::Module, 0)
    }

    fn finish(&mut self) -> Result<SymbolTable, SymbolTableError> {
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

    fn scan_program(&mut self, program: &ast::Program) -> SymbolTableResult {
        self.scan_statements(&program.statements)?;
        Ok(())
    }

    fn scan_statements(&mut self, statements: &[ast::Statement]) -> SymbolTableResult {
        for statement in statements {
            self.scan_statement(statement)?;
        }
        Ok(())
    }

    fn scan_parameters(&mut self, parameters: &[ast::Parameter]) -> SymbolTableResult {
        for parameter in parameters {
            self.scan_parameter(parameter)?;
        }
        Ok(())
    }

    fn scan_parameter(&mut self, parameter: &ast::Parameter) -> SymbolTableResult {
        let usage = if parameter.annotation.is_some() {
            SymbolUsage::AnnotationParameter
        } else {
            SymbolUsage::Parameter
        };
        self.register_name(&parameter.arg, usage, parameter.location)
    }

    fn scan_parameters_annotations(&mut self, parameters: &[ast::Parameter]) -> SymbolTableResult {
        for parameter in parameters {
            self.scan_parameter_annotation(parameter)?;
        }
        Ok(())
    }

    fn scan_parameter_annotation(&mut self, parameter: &ast::Parameter) -> SymbolTableResult {
        if let Some(annotation) = &parameter.annotation {
            self.scan_expression(&annotation, ExpressionContext::Load)?;
        }
        Ok(())
    }

    fn scan_statement(&mut self, statement: &ast::Statement) -> SymbolTableResult {
        use ast::StatementType::*;
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
                self.register_name("__module__", SymbolUsage::Assigned, location)?;
                self.register_name("__qualname__", SymbolUsage::Assigned, location)?;
                self.register_name("__doc__", SymbolUsage::Assigned, location)?;
                self.scan_statements(body)?;
                self.leave_scope();
                self.scan_expressions(bases, ExpressionContext::Load)?;
                for keyword in keywords {
                    self.scan_expression(&keyword.value, ExpressionContext::Load)?;
                }
                self.scan_expressions(decorator_list, ExpressionContext::Load)?;
                self.register_name(name, SymbolUsage::Assigned, location)?;
            }
            Expression { expression } => {
                self.scan_expression(expression, ExpressionContext::Load)?
            }
            If { test, body, orelse } => {
                self.scan_expression(test, ExpressionContext::Load)?;
                self.scan_statements(body)?;
                if let Some(code) = orelse {
                    self.scan_statements(code)?;
                }
            }
            For {
                target,
                iter,
                body,
                orelse,
                ..
            } => {
                self.scan_expression(target, ExpressionContext::Store)?;
                self.scan_expression(iter, ExpressionContext::Load)?;
                self.scan_statements(body)?;
                if let Some(code) = orelse {
                    self.scan_statements(code)?;
                }
            }
            While { test, body, orelse } => {
                self.scan_expression(test, ExpressionContext::Load)?;
                self.scan_statements(body)?;
                if let Some(code) = orelse {
                    self.scan_statements(code)?;
                }
            }
            Break | Continue | Pass => {
                // No symbols here.
            }
            Import { names } | ImportFrom { names, .. } => {
                for name in names {
                    if let Some(alias) = &name.alias {
                        // `import mymodule as myalias`
                        self.register_name(alias, SymbolUsage::Imported, location)?;
                    } else {
                        // `import module`
                        self.register_name(
                            name.symbol.split('.').next().unwrap(),
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
            Assign { targets, value } => {
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
            } => {
                // https://github.com/python/cpython/blob/master/Python/symtable.c#L1233
                if let ast::ExpressionType::Identifier { ref name } = target.node {
                    self.register_name(name, SymbolUsage::AnnotationAssigned, location)?;
                } else {
                    self.scan_expression(target, ExpressionContext::Store)?;
                }
                self.scan_expression(annotation, ExpressionContext::Load)?;
                if let Some(value) = value {
                    self.scan_expression(value, ExpressionContext::Load)?;
                }
            }
            With { items, body, .. } => {
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
                    if let Some(expression) = &handler.typ {
                        self.scan_expression(expression, ExpressionContext::Load)?;
                    }
                    if let Some(name) = &handler.name {
                        self.register_name(name, SymbolUsage::Assigned, location)?;
                    }
                    self.scan_statements(&handler.body)?;
                }
                if let Some(code) = orelse {
                    self.scan_statements(code)?;
                }
                if let Some(code) = finalbody {
                    self.scan_statements(code)?;
                }
            }
            Raise { exception, cause } => {
                if let Some(expression) = exception {
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
        expressions: &[ast::Expression],
        context: ExpressionContext,
    ) -> SymbolTableResult {
        for expression in expressions {
            self.scan_expression(expression, context)?;
        }
        Ok(())
    }

    fn scan_expression(
        &mut self,
        expression: &ast::Expression,
        context: ExpressionContext,
    ) -> SymbolTableResult {
        use ast::ExpressionType::*;
        let location = expression.location;
        match &expression.node {
            Binop { a, b, .. } => {
                self.scan_expression(a, context)?;
                self.scan_expression(b, context)?;
            }
            BoolOp { values, .. } => {
                self.scan_expressions(values, context)?;
            }
            Compare { vals, .. } => {
                self.scan_expressions(vals, context)?;
            }
            Subscript { a, b } => {
                self.scan_expression(a, ExpressionContext::Load)?;
                self.scan_expression(b, ExpressionContext::Load)?;
            }
            Attribute { value, .. } => {
                self.scan_expression(value, ExpressionContext::Load)?;
            }
            Dict { elements } => {
                for (key, value) in elements {
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
            Unop { a, .. } => {
                self.scan_expression(a, context)?;
            }
            True | False | None | Ellipsis => {}
            Number { .. } => {}
            Starred { value } => {
                self.scan_expression(value, context)?;
            }
            Bytes { .. } => {}
            Tuple { elements } | Set { elements } | List { elements } | Slice { elements } => {
                self.scan_expressions(elements, context)?;
            }
            Comprehension { kind, generators } => {
                // Comprehensions are compiled as functions, so create a scope for them:
                let scope_name = match **kind {
                    ast::ComprehensionKind::GeneratorExpression { .. } => "genexpr",
                    ast::ComprehensionKind::List { .. } => "listcomp",
                    ast::ComprehensionKind::Set { .. } => "setcomp",
                    ast::ComprehensionKind::Dict { .. } => "dictcomp",
                };

                self.enter_scope(scope_name, SymbolTableType::Comprehension, location.row());

                // Register the passed argument to the generator function as the name ".0"
                self.register_name(".0", SymbolUsage::Parameter, location)?;

                match **kind {
                    ast::ComprehensionKind::GeneratorExpression { ref element }
                    | ast::ComprehensionKind::List { ref element }
                    | ast::ComprehensionKind::Set { ref element } => {
                        self.scan_expression(element, ExpressionContext::Load)?;
                    }
                    ast::ComprehensionKind::Dict { ref key, ref value } => {
                        self.scan_expression(&key, ExpressionContext::Load)?;
                        self.scan_expression(&value, ExpressionContext::Load)?;
                    }
                }

                let mut is_first_generator = true;
                for generator in generators {
                    self.scan_expression(&generator.target, ExpressionContext::Iter)?;
                    if is_first_generator {
                        is_first_generator = false;
                    } else {
                        self.scan_expression(
                            &generator.iter,
                            ExpressionContext::IterDefinitionExp,
                        )?;
                    }

                    for if_expr in &generator.ifs {
                        self.scan_expression(if_expr, ExpressionContext::Load)?;
                    }
                }

                self.leave_scope();

                // The first iterable is passed as an argument into the created function:
                assert!(!generators.is_empty());
                self.scan_expression(&generators[0].iter, ExpressionContext::IterDefinitionExp)?;
            }
            Call {
                function,
                args,
                keywords,
            } => {
                match context {
                    ExpressionContext::IterDefinitionExp => {
                        self.scan_expression(function, ExpressionContext::IterDefinitionExp)?;
                    }
                    _ => {
                        self.scan_expression(function, ExpressionContext::Load)?;
                    }
                }

                self.scan_expressions(args, ExpressionContext::Load)?;
                for keyword in keywords {
                    self.scan_expression(&keyword.value, ExpressionContext::Load)?;
                }
            }
            String { value } => {
                self.scan_string_group(value)?;
            }
            Identifier { name } => {
                // Determine the contextual usage of this symbol:
                match context {
                    ExpressionContext::Delete => {
                        self.register_name(name, SymbolUsage::Used, location)?;
                    }
                    ExpressionContext::Load | ExpressionContext::IterDefinitionExp => {
                        self.register_name(name, SymbolUsage::Used, location)?;
                    }
                    ExpressionContext::Store => {
                        self.register_name(name, SymbolUsage::Assigned, location)?;
                    }
                    ExpressionContext::Iter => {
                        self.register_name(name, SymbolUsage::Iter, location)?;
                    }
                }
                if context == ExpressionContext::Load
                    && self.tables.last().unwrap().typ == SymbolTableType::Function
                    && name == "super"
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
            IfExpression { test, body, orelse } => {
                self.scan_expression(test, ExpressionContext::Load)?;
                self.scan_expression(body, ExpressionContext::Load)?;
                self.scan_expression(orelse, ExpressionContext::Load)?;
            }

            NamedExpression { left, right } => {
                // named expressions are not allowed in the definiton of
                // comprehension iterator definitions
                if let ExpressionContext::IterDefinitionExp = context {
                    return Err(SymbolTableError {
                        error: "assignment expression cannot be used in a comprehension iterable expression".to_string(),
                        // TODO: accurate location info, somehow
                        location: Location::default(),
                    });
                }

                self.scan_expression(right, ExpressionContext::Load)?;

                // special handling for assigned identifier in named expressions
                // that are used in comprehensions. This required to correctly
                // propagate the scope of the named assigned named and not to
                // propagate inner names.
                if let Identifier { name } = &left.node {
                    let table = self.tables.last().unwrap();
                    if table.typ == SymbolTableType::Comprehension {
                        self.register_name(name, SymbolUsage::AssignedNamedExprInCompr, location)?;
                    } else {
                        // omit one recursion. When the handling of an store changes for
                        // Identifiers this needs adapted - more forward safe would be
                        // calling scan_expression directly.
                        self.register_name(name, SymbolUsage::Assigned, location)?;
                    }
                } else {
                    self.scan_expression(left, ExpressionContext::Store)?;
                }
            }
        }
        Ok(())
    }

    fn enter_function(
        &mut self,
        name: &str,
        args: &ast::Parameters,
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
        self.scan_parameters_annotations(&args.args)?;
        self.scan_parameters_annotations(&args.kwonlyargs)?;
        if let ast::Varargs::Named(name) = &args.vararg {
            self.scan_parameter_annotation(name)?;
        }
        if let ast::Varargs::Named(name) = &args.kwarg {
            self.scan_parameter_annotation(name)?;
        }

        self.enter_scope(name, SymbolTableType::Function, line_number);

        // Fill scope with parameter names:
        self.scan_parameters(&args.args)?;
        self.scan_parameters(&args.kwonlyargs)?;
        if let ast::Varargs::Named(name) = &args.vararg {
            self.scan_parameter(name)?;
        }
        if let ast::Varargs::Named(name) = &args.kwarg {
            self.scan_parameter(name)?;
        }
        Ok(())
    }

    fn scan_string_group(&mut self, group: &ast::StringGroup) -> SymbolTableResult {
        match group {
            ast::StringGroup::Constant { .. } => {}
            ast::StringGroup::FormattedValue { value, spec, .. } => {
                self.scan_expression(value, ExpressionContext::Load)?;
                if let Some(spec) = spec {
                    self.scan_string_group(spec)?;
                }
            }
            ast::StringGroup::Joined { values } => {
                for subgroup in values {
                    self.scan_string_group(subgroup)?;
                }
            }
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

        // Some checks for the symbol that present on this scope level:
        if let Some(symbol) = table.symbols.get(name) {
            // Role already set..
            match role {
                SymbolUsage::Global => {
                    if symbol.is_global() {
                        // Ok
                    } else {
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
            let symbol = Symbol::new(name);
            table.symbols.insert(name.to_owned(), symbol);
        }

        // Set proper flags on symbol:
        let symbol = table.symbols.get_mut(name).unwrap();
        match role {
            SymbolUsage::Nonlocal => {
                symbol.scope = SymbolScope::Free;
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
                if let SymbolScope::Unknown = symbol.scope {
                    symbol.scope = SymbolScope::GlobalImplicit;
                } else if symbol.is_global() {
                    // Global scope can be set to global
                } else {
                    return Err(SymbolTableError {
                        error: format!("Symbol {} scope cannot be set to global, since its scope was already determined otherwise.", name),
                        location,
                    });
                }
            }
            SymbolUsage::Used => {
                symbol.is_referenced = true;
            }
            SymbolUsage::Iter => {
                symbol.is_iter = true;
                symbol.scope = SymbolScope::Local;
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
