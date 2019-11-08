/* Python code is pre-scanned for symbols in the ast.

This ensures that global and nonlocal keywords are picked up.
Then the compiler can use the symbol table to generate proper
load and store instructions for names.

Inspirational file: https://github.com/python/cpython/blob/master/Python/symtable.c
*/

use crate::error::{CompileError, CompileErrorType};
use indexmap::map::IndexMap;
use rustpython_parser::ast;
use rustpython_parser::location::Location;
use std::fmt;

pub fn make_symbol_table(program: &ast::Program) -> Result<SymbolTable, SymbolTableError> {
    let mut builder: SymbolTableBuilder = Default::default();
    builder.prepare();
    builder.scan_program(program)?;
    builder.finish()
}

pub fn statements_to_symbol_table(
    statements: &[ast::Statement],
) -> Result<SymbolTable, SymbolTableError> {
    let mut builder: SymbolTableBuilder = Default::default();
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

    /// A set of symbols present on this scope level.
    pub symbols: IndexMap<String, Symbol>,

    /// A list of subscopes in the order as found in the
    /// AST nodes.
    pub sub_tables: Vec<SymbolTable>,
}

impl SymbolTable {
    fn new(name: String, typ: SymbolTableType, line_number: usize) -> Self {
        SymbolTable {
            name,
            typ,
            line_number,
            symbols: Default::default(),
            sub_tables: vec![],
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum SymbolTableType {
    Module,
    Class,
    Function,
}

impl fmt::Display for SymbolTableType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            SymbolTableType::Module => write!(f, "module"),
            SymbolTableType::Class => write!(f, "class"),
            SymbolTableType::Function => write!(f, "function"),
        }
    }
}

/// Indicator for a single symbol what the scope of this symbol is.
/// The scope can be unknown, which is unfortunate, but not impossible.
#[derive(Debug, Clone)]
pub enum SymbolScope {
    Global,
    Nonlocal,
    Local,
    Unknown,
}

/// A single symbol in a table. Has various properties such as the scope
/// of the symbol, and also the various uses of the symbol.
#[derive(Debug, Clone)]
pub struct Symbol {
    pub name: String,
    // pub table: SymbolTableRef,
    pub scope: SymbolScope,
    pub is_param: bool,
    pub is_referenced: bool,
    pub is_assigned: bool,
    pub is_parameter: bool,
    pub is_free: bool,
}

impl Symbol {
    fn new(name: &str) -> Self {
        Symbol {
            name: name.to_string(),
            // table,
            scope: SymbolScope::Unknown,
            is_param: false,
            is_referenced: false,
            is_assigned: false,
            is_parameter: false,
            is_free: false,
        }
    }

    pub fn is_global(&self) -> bool {
        if let SymbolScope::Global = self.scope {
            true
        } else {
            false
        }
    }

    pub fn is_local(&self) -> bool {
        if let SymbolScope::Local = self.scope {
            true
        } else {
            false
        }
    }
}

#[derive(Debug)]
pub struct SymbolTableError {
    error: String,
    location: Location,
}

impl From<SymbolTableError> for CompileError {
    fn from(error: SymbolTableError) -> Self {
        CompileError {
            error: CompileErrorType::SyntaxError(error.error),
            location: error.location,
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
        let (symbols, _) = self.tables.pop().unwrap();

        // Analyze symbols:
        for symbol in symbols.values_mut() {
            self.analyze_symbol(symbol)?;
        }

        Ok(())
    }

    fn analyze_symbol(&self, symbol: &mut Symbol) -> SymbolTableResult {
        match symbol.scope {
            SymbolScope::Nonlocal => {
                // check if name is defined in parent table!
                let parent_symbol_table = self.tables.last();
                // symbol.table.borrow().parent.clone();

                if let Some((symbols, _)) = parent_symbol_table {
                    let scope_depth = self.tables.len();
                    if !symbols.contains_key(&symbol.name) || scope_depth < 2 {
                        return Err(SymbolTableError {
                            error: format!("no binding for nonlocal '{}' found", symbol.name),
                            location: Default::default(),
                        });
                    }
                } else {
                    return Err(SymbolTableError {
                        error: format!(
                            "nonlocal {} defined at place without an enclosing scope",
                            symbol.name
                        ),
                        location: Default::default(),
                    });
                }
            }
            SymbolScope::Global => {
                // TODO: add more checks for globals?
            }
            SymbolScope::Local => {
                // all is well
            }
            SymbolScope::Unknown => {
                // Try hard to figure out what the scope of this symbol is.

                if symbol.is_assigned || symbol.is_parameter {
                    symbol.scope = SymbolScope::Local;
                } else {
                    // Interesting stuff about the __class__ variable:
                    // https://docs.python.org/3/reference/datamodel.html?highlight=__class__#creating-the-class-object
                    let found_in_outer_scope = symbol.name == "__class__"
                        || self.tables.iter().skip(1).any(|(symbols, typ)| {
                            *typ != SymbolTableType::Class && symbols.contains_key(&symbol.name)
                        });

                    if found_in_outer_scope {
                        // Symbol is in some outer scope.
                        symbol.is_free = true;
                    } else if self.tables.is_empty() {
                        // Don't make assumptions when we don't know.
                        symbol.scope = SymbolScope::Unknown;
                    } else {
                        // If there are scopes above we can assume global.
                        symbol.scope = SymbolScope::Global;
                    }
                }
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
    Parameter,
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
enum ExpressionContext {
    Load,
    Store,
    Delete,
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
        let table = SymbolTable::new(name.to_string(), typ, line_number);
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
        self.register_name(&parameter.arg, SymbolUsage::Parameter)
    }

    fn scan_parameters_annotations(&mut self, parameters: &[ast::Parameter]) -> SymbolTableResult {
        for parameter in parameters {
            self.scan_parameter_annotation(parameter)?;
        }
        Ok(())
    }

    fn scan_parameter_annotation(&mut self, parameter: &ast::Parameter) -> SymbolTableResult {
        if let Some(annotation) = &parameter.annotation {
            self.scan_expression(&annotation, &ExpressionContext::Load)?;
        }
        Ok(())
    }

    fn scan_statement(&mut self, statement: &ast::Statement) -> SymbolTableResult {
        use ast::StatementType::*;
        match &statement.node {
            Global { names } => {
                for name in names {
                    self.register_name(name, SymbolUsage::Global)?;
                }
            }
            Nonlocal { names } => {
                for name in names {
                    self.register_name(name, SymbolUsage::Nonlocal)?;
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
                self.scan_expressions(decorator_list, &ExpressionContext::Load)?;
                self.register_name(name, SymbolUsage::Assigned)?;
                if let Some(expression) = returns {
                    self.scan_expression(expression, &ExpressionContext::Load)?;
                }
                self.enter_function(name, args, statement.location.row())?;
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
                self.enter_scope(name, SymbolTableType::Class, statement.location.row());
                self.register_name("__module__", SymbolUsage::Assigned)?;
                self.register_name("__qualname__", SymbolUsage::Assigned)?;
                self.scan_statements(body)?;
                self.leave_scope();
                self.scan_expressions(bases, &ExpressionContext::Load)?;
                for keyword in keywords {
                    self.scan_expression(&keyword.value, &ExpressionContext::Load)?;
                }
                self.scan_expressions(decorator_list, &ExpressionContext::Load)?;
                self.register_name(name, SymbolUsage::Assigned)?;
            }
            Expression { expression } => {
                self.scan_expression(expression, &ExpressionContext::Load)?
            }
            If { test, body, orelse } => {
                self.scan_expression(test, &ExpressionContext::Load)?;
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
                self.scan_expression(target, &ExpressionContext::Store)?;
                self.scan_expression(iter, &ExpressionContext::Load)?;
                self.scan_statements(body)?;
                if let Some(code) = orelse {
                    self.scan_statements(code)?;
                }
            }
            While { test, body, orelse } => {
                self.scan_expression(test, &ExpressionContext::Load)?;
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
                        self.register_name(alias, SymbolUsage::Assigned)?;
                    } else {
                        // `import module`
                        self.register_name(
                            name.symbol.split('.').next().unwrap(),
                            SymbolUsage::Assigned,
                        )?;
                    }
                }
            }
            Return { value } => {
                if let Some(expression) = value {
                    self.scan_expression(expression, &ExpressionContext::Load)?;
                }
            }
            Assert { test, msg } => {
                self.scan_expression(test, &ExpressionContext::Load)?;
                if let Some(expression) = msg {
                    self.scan_expression(expression, &ExpressionContext::Load)?;
                }
            }
            Delete { targets } => {
                self.scan_expressions(targets, &ExpressionContext::Delete)?;
            }
            Assign { targets, value } => {
                self.scan_expressions(targets, &ExpressionContext::Store)?;
                self.scan_expression(value, &ExpressionContext::Load)?;
            }
            AugAssign { target, value, .. } => {
                self.scan_expression(target, &ExpressionContext::Store)?;
                self.scan_expression(value, &ExpressionContext::Load)?;
            }
            AnnAssign {
                target,
                annotation,
                value,
            } => {
                self.scan_expression(target, &ExpressionContext::Store)?;
                self.scan_expression(annotation, &ExpressionContext::Load)?;
                if let Some(value) = value {
                    self.scan_expression(value, &ExpressionContext::Load)?;
                }
            }
            With { items, body, .. } => {
                for item in items {
                    self.scan_expression(&item.context_expr, &ExpressionContext::Load)?;
                    if let Some(expression) = &item.optional_vars {
                        self.scan_expression(expression, &ExpressionContext::Store)?;
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
                        self.scan_expression(expression, &ExpressionContext::Load)?;
                    }
                    if let Some(name) = &handler.name {
                        self.register_name(name, SymbolUsage::Assigned)?;
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
                    self.scan_expression(expression, &ExpressionContext::Load)?;
                }
                if let Some(expression) = cause {
                    self.scan_expression(expression, &ExpressionContext::Load)?;
                }
            }
        }
        Ok(())
    }

    fn scan_expressions(
        &mut self,
        expressions: &[ast::Expression],
        context: &ExpressionContext,
    ) -> SymbolTableResult {
        for expression in expressions {
            self.scan_expression(expression, context)?;
        }
        Ok(())
    }

    fn scan_expression(
        &mut self,
        expression: &ast::Expression,
        context: &ExpressionContext,
    ) -> SymbolTableResult {
        use ast::ExpressionType::*;
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
                self.scan_expression(a, context)?;
                self.scan_expression(b, context)?;
            }
            Attribute { value, .. } => {
                self.scan_expression(value, context)?;
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

                self.enter_scope(
                    scope_name,
                    SymbolTableType::Function,
                    expression.location.row(),
                );

                // Register the passed argument to the generator function as the name ".0"
                self.register_name(".0", SymbolUsage::Parameter)?;

                match **kind {
                    ast::ComprehensionKind::GeneratorExpression { ref element }
                    | ast::ComprehensionKind::List { ref element }
                    | ast::ComprehensionKind::Set { ref element } => {
                        self.scan_expression(element, &ExpressionContext::Load)?;
                    }
                    ast::ComprehensionKind::Dict { ref key, ref value } => {
                        self.scan_expression(&key, &ExpressionContext::Load)?;
                        self.scan_expression(&value, &ExpressionContext::Load)?;
                    }
                }

                let mut is_first_generator = true;
                for generator in generators {
                    self.scan_expression(&generator.target, &ExpressionContext::Store)?;
                    if is_first_generator {
                        is_first_generator = false;
                    } else {
                        self.scan_expression(&generator.iter, &ExpressionContext::Load)?;
                    }

                    for if_expr in &generator.ifs {
                        self.scan_expression(if_expr, &ExpressionContext::Load)?;
                    }
                }

                self.leave_scope();

                // The first iterable is passed as an argument into the created function:
                assert!(!generators.is_empty());
                self.scan_expression(&generators[0].iter, &ExpressionContext::Load)?;
            }
            Call {
                function,
                args,
                keywords,
            } => {
                self.scan_expression(function, &ExpressionContext::Load)?;
                self.scan_expressions(args, &ExpressionContext::Load)?;
                for keyword in keywords {
                    self.scan_expression(&keyword.value, &ExpressionContext::Load)?;
                }
            }
            String { value } => {
                self.scan_string_group(value)?;
            }
            Identifier { name } => {
                // Determine the contextual usage of this symbol:
                match context {
                    ExpressionContext::Delete => {
                        self.register_name(name, SymbolUsage::Used)?;
                    }
                    ExpressionContext::Load => {
                        self.register_name(name, SymbolUsage::Used)?;
                    }
                    ExpressionContext::Store => {
                        self.register_name(name, SymbolUsage::Assigned)?;
                    }
                }
            }
            Lambda { args, body } => {
                self.enter_function("lambda", args, expression.location.row())?;
                self.scan_expression(body, &ExpressionContext::Load)?;
                self.leave_scope();
            }
            IfExpression { test, body, orelse } => {
                self.scan_expression(test, &ExpressionContext::Load)?;
                self.scan_expression(body, &ExpressionContext::Load)?;
                self.scan_expression(orelse, &ExpressionContext::Load)?;
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
        self.scan_expressions(&args.defaults, &ExpressionContext::Load)?;
        for kw_default in &args.kw_defaults {
            if let Some(expression) = kw_default {
                self.scan_expression(&expression, &ExpressionContext::Load)?;
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
            ast::StringGroup::FormattedValue { value, .. } => {
                self.scan_expression(value, &ExpressionContext::Load)?;
            }
            ast::StringGroup::Joined { values } => {
                for subgroup in values {
                    self.scan_string_group(subgroup)?;
                }
            }
        }
        Ok(())
    }

    #[allow(clippy::single_match)]
    fn register_name(&mut self, name: &str, role: SymbolUsage) -> SymbolTableResult {
        let scope_depth = self.tables.len();
        let table = self.tables.last_mut().unwrap();
        let location = Default::default();

        // Some checks:
        let containing = table.symbols.contains_key(name);
        if containing {
            // Role already set..
            match role {
                SymbolUsage::Global => {
                    let symbol = table.symbols.get(name).unwrap();
                    if let SymbolScope::Global = symbol.scope {
                        // Ok
                    } else {
                        return Err(SymbolTableError {
                            error: format!("name '{}' is used prior to global declaration", name),
                            location,
                        });
                    }
                }
                SymbolUsage::Nonlocal => {
                    return Err(SymbolTableError {
                        error: format!("name '{}' is used prior to nonlocal declaration", name),
                        location,
                    })
                }
                _ => {
                    // Ok?
                }
            }
        }

        // Some more checks:
        match role {
            SymbolUsage::Nonlocal => {
                if scope_depth < 2 {
                    return Err(SymbolTableError {
                        error: format!("cannot define nonlocal '{}' at top level.", name),
                        location,
                    });
                }
            }
            _ => {
                // Ok!
            }
        }

        // Insert symbol when required:
        if !containing {
            let symbol = Symbol::new(name);
            table.symbols.insert(name.to_string(), symbol);
        }

        // Set proper flags on symbol:
        let symbol = table.symbols.get_mut(name).unwrap();
        match role {
            SymbolUsage::Nonlocal => {
                if let SymbolScope::Unknown = symbol.scope {
                    symbol.scope = SymbolScope::Nonlocal;
                } else {
                    return Err(SymbolTableError {
                        error: format!("Symbol {} scope cannot be set to nonlocal, since its scope was already determined otherwise.", name),
                        location,
                    });
                }
            }
            SymbolUsage::Parameter => {
                symbol.is_parameter = true;
            }
            SymbolUsage::Assigned => {
                symbol.is_assigned = true;
            }
            SymbolUsage::Global => {
                if let SymbolScope::Unknown = symbol.scope {
                    symbol.scope = SymbolScope::Global;
                } else if let SymbolScope::Global = symbol.scope {
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
        }

        Ok(())
    }
}
