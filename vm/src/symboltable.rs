/* Python code is pre-scanned for symbols in the ast.

This ensures that global and nonlocal keywords are picked up.
Then the compiler can use the symbol table to generate proper
load and store instructions for names.
*/

use rustpython_parser::ast;
use std::collections::HashMap;

pub fn make_symbol_table(program: &ast::Program) -> SymbolScope {
    let mut builder = SymbolTableBuilder::new();
    builder.enter_scope();
    builder.scan_program(program).unwrap();
    assert!(builder.scopes.len() == 1);
    builder.scopes.pop().unwrap()
}

pub fn statements_to_symbol_table(statements: &[ast::LocatedStatement]) -> SymbolScope {
    let mut builder = SymbolTableBuilder::new();
    builder.enter_scope();
    builder.scan_statements(statements).unwrap();
    assert!(builder.scopes.len() == 1);
    builder.scopes.pop().unwrap()
}

#[derive(Debug)]
pub enum SymbolRole {
    Global,
    Nonlocal,
    Used,
    Assigned,
}

/// Symbolscope captures all symbols in the current scope, and
/// has a list of subscopes in this scope.
pub struct SymbolScope {
    /// A set of symbols present on this scope level.
    pub symbols: HashMap<String, SymbolRole>,

    /// A list of subscopes in the order as found in the
    /// AST nodes.
    pub sub_scopes: Vec<SymbolScope>,
}

type SymbolTableResult = Result<(), String>;

impl SymbolScope {
    pub fn new() -> Self {
        SymbolScope {
            symbols: HashMap::new(),
            sub_scopes: vec![],
        }
    }

    pub fn lookup(&self, name: &str) -> Option<&SymbolRole> {
        self.symbols.get(name)
    }
}

impl std::fmt::Debug for SymbolScope {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            f,
            "SymbolScope({:?} symbols, {:?} sub scopes)",
            self.symbols.len(),
            self.sub_scopes.len()
        )
    }
}

pub struct SymbolTableBuilder {
    // Scope stack.
    pub scopes: Vec<SymbolScope>,
}

impl SymbolTableBuilder {
    pub fn new() -> Self {
        SymbolTableBuilder { scopes: vec![] }
    }

    pub fn enter_scope(&mut self) {
        // Create new scope and push into scope stack.
        let scope = SymbolScope::new();
        self.scopes.push(scope);
    }

    pub fn leave_scope(&mut self) {
        // Pop scope and add to subscopes of parent scope.
        let scope = self.scopes.pop().unwrap();
        self.scopes.last_mut().unwrap().sub_scopes.push(scope);
    }

    pub fn scan_program(&mut self, program: &ast::Program) -> SymbolTableResult {
        self.scan_statements(&program.statements)?;
        Ok(())
    }

    pub fn scan_statements(&mut self, statements: &[ast::LocatedStatement]) -> SymbolTableResult {
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
        self.register_name(&parameter.arg, SymbolRole::Assigned)
    }

    fn scan_parameters_annotations(&mut self, parameters: &[ast::Parameter]) -> SymbolTableResult {
        for parameter in parameters {
            self.scan_parameter_annotation(parameter)?;
        }
        Ok(())
    }

    fn scan_parameter_annotation(&mut self, parameter: &ast::Parameter) -> SymbolTableResult {
        if let Some(annotation) = &parameter.annotation {
            self.scan_expression(&annotation)?;
        }
        Ok(())
    }

    fn scan_statement(&mut self, statement: &ast::LocatedStatement) -> SymbolTableResult {
        match &statement.node {
            ast::Statement::Global { names } => {
                for name in names {
                    self.register_name(name, SymbolRole::Global)?;
                }
            }
            ast::Statement::Nonlocal { names } => {
                for name in names {
                    self.register_name(name, SymbolRole::Nonlocal)?;
                }
            }
            ast::Statement::FunctionDef {
                name,
                body,
                args,
                decorator_list,
                returns,
            } => {
                self.scan_expressions(decorator_list)?;
                self.register_name(name, SymbolRole::Assigned)?;

                self.enter_function(args)?;

                self.scan_statements(body)?;
                if let Some(expression) = returns {
                    self.scan_expression(expression)?;
                }
                self.leave_scope();
            }
            ast::Statement::ClassDef {
                name,
                body,
                bases,
                keywords,
                decorator_list,
            } => {
                self.register_name(name, SymbolRole::Assigned)?;
                self.enter_scope();
                self.scan_statements(body)?;
                self.leave_scope();
                self.scan_expressions(bases)?;
                for keyword in keywords {
                    self.scan_expression(&keyword.value)?;
                }
                self.scan_expressions(decorator_list)?;
            }
            ast::Statement::Expression { expression } => self.scan_expression(expression)?,
            ast::Statement::If { test, body, orelse } => {
                self.scan_expression(test)?;
                self.scan_statements(body)?;
                if let Some(code) = orelse {
                    self.scan_statements(code)?;
                }
            }
            ast::Statement::For {
                target,
                iter,
                body,
                orelse,
            } => {
                self.scan_expression(target)?;
                self.scan_expression(iter)?;
                self.scan_statements(body)?;
                if let Some(code) = orelse {
                    self.scan_statements(code)?;
                }
            }
            ast::Statement::While { test, body, orelse } => {
                self.scan_expression(test)?;
                self.scan_statements(body)?;
                if let Some(code) = orelse {
                    self.scan_statements(code)?;
                }
            }
            ast::Statement::Break | ast::Statement::Continue | ast::Statement::Pass => {
                // No symbols here.
            }
            ast::Statement::Import { import_parts } => {
                for part in import_parts {
                    if let Some(alias) = &part.alias {
                        self.register_name(alias, SymbolRole::Assigned)?;
                    } else {
                        if let Some(symbol) = &part.symbol {
                            self.register_name(symbol, SymbolRole::Assigned)?;
                        } else {
                            self.register_name(&part.module, SymbolRole::Assigned)?;
                        }
                    }
                }
            }
            ast::Statement::Return { value } => {
                if let Some(expression) = value {
                    self.scan_expression(expression)?;
                }
            }
            ast::Statement::Assert { test, msg } => {
                self.scan_expression(test)?;
                if let Some(expression) = msg {
                    self.scan_expression(expression)?;
                }
            }
            ast::Statement::Delete { targets } => {
                self.scan_expressions(targets)?;
            }
            ast::Statement::Assign { targets, value } => {
                self.scan_expressions(targets)?;
                self.scan_expression(value)?;
            }
            ast::Statement::AugAssign { target, value, .. } => {
                self.scan_expression(target)?;
                self.scan_expression(value)?;
            }
            ast::Statement::With { items, body } => {
                for item in items {
                    self.scan_expression(&item.context_expr)?;
                    if let Some(expression) = &item.optional_vars {
                        self.scan_expression(expression)?;
                    }
                }
                self.scan_statements(body)?;
            }
            ast::Statement::Try {
                body,
                handlers,
                orelse,
                finalbody,
            } => {
                self.scan_statements(body)?;
                for handler in handlers {
                    if let Some(expression) = &handler.typ {
                        self.scan_expression(expression)?;
                    }
                    if let Some(name) = &handler.name {
                        self.register_name(name, SymbolRole::Assigned)?;
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
            ast::Statement::Raise { exception, cause } => {
                if let Some(expression) = exception {
                    self.scan_expression(expression)?;
                }
                if let Some(expression) = cause {
                    self.scan_expression(expression)?;
                }
            }
        }
        Ok(())
    }

    fn scan_expressions(&mut self, expressions: &[ast::Expression]) -> SymbolTableResult {
        for expression in expressions {
            self.scan_expression(expression)?;
        }
        Ok(())
    }

    fn scan_expression(&mut self, expression: &ast::Expression) -> SymbolTableResult {
        match expression {
            ast::Expression::Binop { a, b, .. } => {
                self.scan_expression(a)?;
                self.scan_expression(b)?;
            }
            ast::Expression::BoolOp { a, b, .. } => {
                self.scan_expression(a)?;
                self.scan_expression(b)?;
            }
            ast::Expression::Compare { vals, .. } => {
                self.scan_expressions(vals)?;
            }
            ast::Expression::Subscript { a, b } => {
                self.scan_expression(a)?;
                self.scan_expression(b)?;
            }
            ast::Expression::Attribute { value, .. } => {
                self.scan_expression(value)?;
            }
            ast::Expression::Dict { elements } => {
                for (key, value) in elements {
                    self.scan_expression(key)?;
                    self.scan_expression(value)?;
                }
            }
            ast::Expression::Yield { value } => {
                if let Some(expression) = value {
                    self.scan_expression(expression)?;
                }
            }
            ast::Expression::YieldFrom { value } => {
                self.scan_expression(value)?;
            }
            ast::Expression::Unop { a, .. } => {
                self.scan_expression(a)?;
            }
            ast::Expression::True
            | ast::Expression::False
            | ast::Expression::None
            | ast::Expression::Ellipsis => {}
            ast::Expression::Number { .. } => {}
            ast::Expression::Starred { value } => {
                self.scan_expression(value)?;
            }
            ast::Expression::Bytes { .. } => {}
            ast::Expression::Tuple { elements }
            | ast::Expression::Set { elements }
            | ast::Expression::List { elements }
            | ast::Expression::Slice { elements } => {
                self.scan_expressions(elements)?;
            }
            ast::Expression::Comprehension { kind, generators } => {
                match **kind {
                    ast::ComprehensionKind::GeneratorExpression { ref element }
                    | ast::ComprehensionKind::List { ref element }
                    | ast::ComprehensionKind::Set { ref element } => {
                        self.scan_expression(element)?;
                    }
                    ast::ComprehensionKind::Dict { ref key, ref value } => {
                        self.scan_expression(&key)?;
                        self.scan_expression(&value)?;
                    }
                }

                for generator in generators {
                    self.scan_expression(&generator.target)?;
                    self.scan_expression(&generator.iter)?;
                    for if_expr in &generator.ifs {
                        self.scan_expression(if_expr)?;
                    }
                }
            }
            ast::Expression::Call {
                function,
                args,
                keywords,
            } => {
                self.scan_expression(function)?;
                self.scan_expressions(args)?;
                for keyword in keywords {
                    self.scan_expression(&keyword.value)?;
                }
            }
            ast::Expression::String { value } => {
                self.scan_string_group(value)?;
            }
            ast::Expression::Identifier { name } => {
                self.register_name(name, SymbolRole::Used)?;
            }
            ast::Expression::Lambda { args, body } => {
                self.enter_function(args)?;
                self.scan_expression(body)?;
                self.leave_scope();
            }
            ast::Expression::IfExpression { test, body, orelse } => {
                self.scan_expression(test)?;
                self.scan_expression(body)?;
                self.scan_expression(orelse)?;
            }
        }
        Ok(())
    }

    fn enter_function(&mut self, args: &ast::Parameters) -> SymbolTableResult {
        // Evaulate eventual default parameters:
        self.scan_expressions(&args.defaults)?;
        for kw_default in &args.kw_defaults {
            if let Some(expression) = kw_default {
                self.scan_expression(&expression)?;
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

        self.enter_scope();

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
                self.scan_expression(value)?;
            }
            ast::StringGroup::Joined { values } => {
                for subgroup in values {
                    self.scan_string_group(subgroup)?;
                }
            }
        }
        Ok(())
    }

    fn register_name(&mut self, name: &str, role: SymbolRole) -> SymbolTableResult {
        let current_scope = self.scopes.last_mut().unwrap();
        if let Some(_old_role) = current_scope.symbols.get(name) {
            // Role already set..
            // debug!("TODO: {:?}", old_role);
            match role {
                SymbolRole::Global => return Err("global must appear first".to_string()),
                _ => {
                    // Ok?
                }
            }
        } else {
            current_scope.symbols.insert(name.to_string(), role);
        }
        Ok(())
    }
}
