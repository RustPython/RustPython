/* Python code is pre-scanned for symbols in the ast.

This ensures that global and nonlocal keywords are picked up.
Then the compiler can use the symbol table to generate proper
load and store instructions for names.
*/

use rustpython_parser::ast;
use std::collections::HashMap;

pub enum SymbolRole {
    Global,
    Nonlocal,
    Used,
    Assigned,
}

pub struct SymbolTable {
    // TODO: split-up into nested scopes.
    symbols: HashMap<String, SymbolRole>,
}

impl SymbolTable {
    pub fn new() -> Self {
        SymbolTable {
            symbols: HashMap::new(),
        }
    }

    pub fn lookup(&self, name: &str) -> Option<&SymbolRole> {
        return self.symbols.get(name);
    }

    pub fn scan_program(&mut self, program: &ast::Program) {
        self.scan_statements(&program.statements);
    }

    pub fn scan_statements(&mut self, statements: &[ast::LocatedStatement]) {
        for statement in statements {
            self.scan_statement(statement)
        }
    }

    fn scan_statement(&mut self, statement: &ast::LocatedStatement) {
        match &statement.node {
            ast::Statement::Global { names } => {
                for name in names {
                    self.register_name(name, SymbolRole::Global);
                }
            }
            ast::Statement::Nonlocal { names } => {
                for name in names {
                    self.register_name(name, SymbolRole::Nonlocal);
                }
            }
            ast::Statement::FunctionDef {
                name,
                body,
                args,
                decorator_list,
                returns,
            } => {
                self.scan_expressions(decorator_list);
                self.register_name(name, SymbolRole::Assigned);
                for parameter in &args.args {}
                for parameter in &args.kwonlyargs {}

                self.scan_expressions(&args.defaults);
                for kw_default in &args.kw_defaults {
                    if let Some(expression) = kw_default {
                        self.scan_expression(&expression);
                    }
                }

                self.scan_statements(body);
                if let Some(expression) = returns {
                    self.scan_expression(expression);
                }
            }
            ast::Statement::ClassDef {
                name,
                body,
                bases,
                keywords,
                decorator_list,
            } => {
                self.register_name(name, SymbolRole::Assigned);
                self.scan_statements(body);
                self.scan_expressions(bases);
                for keyword in keywords {
                    self.scan_expression(&keyword.value);
                }
                self.scan_expressions(decorator_list);
            }
            ast::Statement::Expression { expression } => self.scan_expression(expression),
            ast::Statement::If { test, body, orelse } => {
                self.scan_expression(test);
                self.scan_statements(body);
                if let Some(code) = orelse {
                    self.scan_statements(code);
                }
            }
            ast::Statement::For {
                target,
                iter,
                body,
                orelse,
            } => {
                self.scan_expression(target);
                self.scan_expression(iter);
                self.scan_statements(body);
                if let Some(code) = orelse {
                    self.scan_statements(code);
                }
            }
            ast::Statement::While { test, body, orelse } => {
                self.scan_expression(test);
                self.scan_statements(body);
                if let Some(code) = orelse {
                    self.scan_statements(code);
                }
            }
            ast::Statement::Break | ast::Statement::Continue | ast::Statement::Pass => {
                // No symbols here.
            }
            ast::Statement::Import { import_parts } => for part in import_parts {},
            ast::Statement::Return { value } => {
                if let Some(expression) = value {
                    self.scan_expression(expression);
                }
            }
            ast::Statement::Assert { test, msg } => {
                self.scan_expression(test);
                if let Some(expression) = msg {
                    self.scan_expression(expression);
                }
            }
            ast::Statement::Delete { targets } => {
                self.scan_expressions(targets);
            }
            ast::Statement::Assign { targets, value } => {
                self.scan_expressions(targets);
                self.scan_expression(value);
            }
            ast::Statement::AugAssign {
                target,
                op: _,
                value,
            } => {
                self.scan_expression(target);
                self.scan_expression(value);
            }
            ast::Statement::With { items, body } => {
                for item in items {
                    self.scan_expression(&item.context_expr);
                    if let Some(expression) = &item.optional_vars {
                        self.scan_expression(expression);
                    }
                }
                self.scan_statements(body);
            }
            ast::Statement::Try {
                body,
                handlers,
                orelse,
                finalbody,
            } => {
                self.scan_statements(body);
                if let Some(code) = orelse {
                    self.scan_statements(code);
                }
                if let Some(code) = finalbody {
                    self.scan_statements(code);
                }
            }
            ast::Statement::Raise { exception, cause } => {
                if let Some(expression) = exception {
                    self.scan_expression(expression);
                }
                if let Some(expression) = cause {
                    self.scan_expression(expression);
                }
            }
        }
    }

    fn scan_expressions(&mut self, expressions: &[ast::Expression]) {
        for expression in expressions {
            self.scan_expression(expression);
        }
    }

    fn scan_expression(&mut self, expression: &ast::Expression) {
        match expression {
            ast::Expression::Binop { a, op: _, b } => {
                self.scan_expression(a);
                self.scan_expression(b);
            }
            ast::Expression::BoolOp { a, op: _, b } => {
                self.scan_expression(a);
                self.scan_expression(b);
            }
            ast::Expression::Compare { vals, ops: _ } => {
                self.scan_expressions(vals);
            }
            ast::Expression::Subscript { a, b } => {
                self.scan_expression(a);
                self.scan_expression(b);
            }
            ast::Expression::Attribute { value, name: _ } => {
                self.scan_expression(value);
            }
            ast::Expression::Dict { elements } => {
                for (key, value) in elements {
                    self.scan_expression(key);
                    self.scan_expression(value);
                }
            }
            ast::Expression::Yield { value } => {
                if let Some(expression) = value {
                    self.scan_expression(expression);
                }
            }
            ast::Expression::YieldFrom { value } => {
                self.scan_expression(value);
            }
            ast::Expression::Unop { op: _, a } => {
                self.scan_expression(a);
            }
            ast::Expression::True
            | ast::Expression::False
            | ast::Expression::None
            | ast::Expression::Ellipsis => {}
            ast::Expression::Number { .. } => {}
            ast::Expression::Starred { value } => {
                self.scan_expression(value);
            }
            ast::Expression::Bytes { .. } => {}
            ast::Expression::Tuple { elements }
            | ast::Expression::Set { elements }
            | ast::Expression::List { elements }
            | ast::Expression::Slice { elements } => {
                self.scan_expressions(elements);
            }
            ast::Expression::Comprehension { kind, generators } => {}
            ast::Expression::Call {
                function,
                args,
                keywords,
            } => {
                self.scan_expression(function);
                self.scan_expressions(args);
            }
            ast::Expression::String { value } => {}
            ast::Expression::Identifier { name } => {
                self.register_name(name, SymbolRole::Used);
            }
            ast::Expression::Lambda { args, body } => {
                self.scan_expression(body);
            }
            ast::Expression::IfExpression { test, body, orelse } => {
                self.scan_expression(test);
                self.scan_expression(body);
                self.scan_expression(orelse);
            }
        }
    }

    fn register_name(&mut self, name: &String, role: SymbolRole) {
        self.symbols.insert(name.clone(), role);
    }
}
