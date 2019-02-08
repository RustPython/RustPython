//!
//!
//! Take an AST and transform it into bytecode
//!
//! Inspirational code:
//!   https://github.com/python/cpython/blob/master/Python/compile.c
//!   https://github.com/micropython/micropython/blob/master/py/compile.c

use super::bytecode::{self, CallType, CodeObject, Instruction};
use super::pyobject::PyResult;
use super::vm::VirtualMachine;
use num_complex::Complex64;
use rustpython_parser::{ast, parser};

struct Compiler {
    code_object_stack: Vec<CodeObject>,
    nxt_label: usize,
    source_path: Option<String>,
    current_source_location: ast::Location,
}

/// Compile a given sourcecode into a bytecode object.
pub fn compile(
    vm: &mut VirtualMachine,
    source: &str,
    mode: &Mode,
    source_path: Option<String>,
) -> PyResult {
    let mut compiler = Compiler::new();
    compiler.source_path = source_path.clone();
    compiler.push_new_code_object(source_path, "<module>".to_string());
    let syntax_error = vm.context().exceptions.syntax_error.clone();
    let result = match mode {
        Mode::Exec => match parser::parse_program(source) {
            Ok(ast) => compiler.compile_program(&ast),
            Err(msg) => Err(msg),
        },
        Mode::Eval => match parser::parse_statement(source) {
            Ok(statement) => compiler.compile_statement_eval(&statement),
            Err(msg) => Err(msg),
        },
        Mode::Single => match parser::parse_program(source) {
            Ok(ast) => compiler.compile_program_single(&ast),
            Err(msg) => Err(msg),
        },
    };

    if let Err(msg) = result {
        return Err(vm.new_exception(syntax_error.clone(), msg));
    }

    let code = compiler.pop_code_object();
    trace!("Compilation completed: {:?}", code);
    Ok(vm.ctx.new_code_object(code))
}

pub enum Mode {
    Exec,
    Eval,
    Single,
}

#[derive(Clone, Copy)]
enum EvalContext {
    Statement,
    Expression,
}

type Label = usize;

impl Compiler {
    fn new() -> Self {
        Compiler {
            code_object_stack: Vec::new(),
            nxt_label: 0,
            source_path: None,
            current_source_location: ast::Location::default(),
        }
    }

    fn push_new_code_object(&mut self, source_path: Option<String>, obj_name: String) {
        let line_number = self.get_source_line_number();
        self.code_object_stack.push(CodeObject::new(
            Vec::new(),
            None,
            Vec::new(),
            None,
            source_path.clone(),
            line_number,
            obj_name,
        ));
    }

    fn pop_code_object(&mut self) -> CodeObject {
        self.code_object_stack.pop().unwrap()
    }

    fn compile_program(&mut self, program: &ast::Program) -> Result<(), String> {
        let size_before = self.code_object_stack.len();
        self.compile_statements(&program.statements)?;
        assert!(self.code_object_stack.len() == size_before);

        // Emit None at end:
        self.emit(Instruction::LoadConst {
            value: bytecode::Constant::None,
        });
        self.emit(Instruction::ReturnValue);
        Ok(())
    }

    fn compile_program_single(&mut self, program: &ast::Program) -> Result<(), String> {
        for statement in &program.statements {
            if let ast::Statement::Expression { ref expression } = statement.node {
                self.compile_expression(expression)?;
                self.emit(Instruction::PrintExpr);
            } else {
                self.compile_statement(&statement)?;
            }
        }
        self.emit(Instruction::LoadConst {
            value: bytecode::Constant::None,
        });
        self.emit(Instruction::ReturnValue);
        Ok(())
    }

    // Compile statement in eval mode:
    fn compile_statement_eval(&mut self, statement: &ast::LocatedStatement) -> Result<(), String> {
        if let ast::Statement::Expression { ref expression } = statement.node {
            self.compile_expression(expression)?;
            self.emit(Instruction::ReturnValue);
            Ok(())
        } else {
            Err("Expecting expression, got statement".to_string())
        }
    }

    fn compile_statements(&mut self, statements: &[ast::LocatedStatement]) -> Result<(), String> {
        for statement in statements {
            self.compile_statement(statement)?
        }
        Ok(())
    }

    fn compile_statement(&mut self, statement: &ast::LocatedStatement) -> Result<(), String> {
        trace!("Compiling {:?}", statement);
        self.set_source_location(&statement.location);

        match &statement.node {
            ast::Statement::Import { import_parts } => {
                for ast::SingleImport {
                    module,
                    symbol,
                    alias,
                } in import_parts
                {
                    match symbol {
                        Some(name) if name == "*" => {
                            self.emit(Instruction::ImportStar {
                                name: module.clone(),
                            });
                        }
                        _ => {
                            self.emit(Instruction::Import {
                                name: module.clone(),
                                symbol: symbol.clone().map(|s| s.clone()),
                            });
                            self.emit(Instruction::StoreName {
                                name: match alias {
                                    Some(alias) => alias.clone(),
                                    None => match symbol {
                                        Some(symbol) => symbol.clone(),
                                        None => module.clone(),
                                    },
                                },
                            });
                        }
                    }
                }
            }
            ast::Statement::Expression { expression } => {
                self.compile_expression(expression)?;

                // Pop result of stack, since we not use it:
                self.emit(Instruction::Pop);
            }
            ast::Statement::Global { names } => {
                unimplemented!("global {:?}", names);
            }
            ast::Statement::Nonlocal { names } => {
                unimplemented!("nonlocal {:?}", names);
            }
            ast::Statement::If { test, body, orelse } => {
                let end_label = self.new_label();
                match orelse {
                    None => {
                        // Only if:
                        self.compile_test(test, None, Some(end_label), EvalContext::Statement)?;
                        self.compile_statements(body)?;
                        self.set_label(end_label);
                    }
                    Some(statements) => {
                        // if - else:
                        let else_label = self.new_label();
                        self.compile_test(test, None, Some(else_label), EvalContext::Statement)?;
                        self.compile_statements(body)?;
                        self.emit(Instruction::Jump { target: end_label });

                        // else:
                        self.set_label(else_label);
                        self.compile_statements(statements)?;
                    }
                }
                self.set_label(end_label);
            }
            ast::Statement::While { test, body, orelse } => {
                let start_label = self.new_label();
                let else_label = self.new_label();
                let end_label = self.new_label();
                self.emit(Instruction::SetupLoop {
                    start: start_label,
                    end: end_label,
                });

                self.set_label(start_label);

                self.compile_test(test, None, Some(else_label), EvalContext::Statement)?;
                self.compile_statements(body)?;
                self.emit(Instruction::Jump {
                    target: start_label,
                });
                self.set_label(else_label);
                if let Some(orelse) = orelse {
                    self.compile_statements(orelse)?;
                }
                self.set_label(end_label);
                self.emit(Instruction::PopBlock);
            }
            ast::Statement::With { items, body } => {
                let end_label = self.new_label();
                for item in items {
                    self.compile_expression(&item.context_expr)?;
                    self.emit(Instruction::SetupWith { end: end_label });
                    match &item.optional_vars {
                        Some(var) => {
                            self.compile_store(var)?;
                        }
                        None => {
                            self.emit(Instruction::Pop);
                        }
                    }
                }

                self.compile_statements(body)?;
                for _ in 0..items.len() {
                    self.emit(Instruction::CleanupWith { end: end_label });
                }
                self.set_label(end_label);
            }
            ast::Statement::For {
                target,
                iter,
                body,
                orelse,
            } => {
                // The thing iterated:
                for i in iter {
                    self.compile_expression(i)?;
                }

                // Retrieve iterator
                self.emit(Instruction::GetIter);

                // Start loop
                let start_label = self.new_label();
                let else_label = self.new_label();
                let end_label = self.new_label();
                self.emit(Instruction::SetupLoop {
                    start: start_label,
                    end: end_label,
                });
                self.set_label(start_label);
                self.emit(Instruction::ForIter { target: else_label });

                // Start of loop iteration, set targets:
                self.compile_store(target)?;

                // Body of loop:
                self.compile_statements(body)?;
                self.emit(Instruction::Jump {
                    target: start_label,
                });
                self.set_label(else_label);
                if let Some(orelse) = orelse {
                    self.compile_statements(orelse)?;
                }
                self.set_label(end_label);
                self.emit(Instruction::PopBlock);
            }
            ast::Statement::Raise { exception, cause } => match exception {
                Some(value) => {
                    self.compile_expression(value)?;
                    match cause {
                        Some(cause) => {
                            self.compile_expression(cause)?;
                            self.emit(Instruction::Raise { argc: 2 });
                        }
                        None => {
                            self.emit(Instruction::Raise { argc: 1 });
                        }
                    }
                }
                None => {
                    self.emit(Instruction::Raise { argc: 0 });
                }
            },
            ast::Statement::Try {
                body,
                handlers,
                orelse,
                finalbody,
            } => {
                let mut handler_label = self.new_label();
                let finally_label = self.new_label();
                let else_label = self.new_label();
                // try:
                self.emit(Instruction::SetupExcept {
                    handler: handler_label,
                });
                self.compile_statements(body)?;
                self.emit(Instruction::PopBlock);
                self.emit(Instruction::Jump { target: else_label });

                // except handlers:
                self.set_label(handler_label);
                // Exception is on top of stack now
                handler_label = self.new_label();
                for handler in handlers {
                    // If we gave a typ,
                    // check if this handler can handle the exception:
                    if let Some(exc_type) = &handler.typ {
                        // Duplicate exception for test:
                        self.emit(Instruction::Duplicate);

                        // Check exception type:
                        self.emit(Instruction::LoadName {
                            name: String::from("isinstance"),
                        });
                        self.emit(Instruction::Rotate { amount: 2 });
                        self.compile_expression(exc_type)?;
                        self.emit(Instruction::CallFunction {
                            typ: CallType::Positional(2),
                        });

                        // We cannot handle this exception type:
                        self.emit(Instruction::JumpIfFalse {
                            target: handler_label,
                        });

                        // We have a match, store in name (except x as y)
                        if let Some(alias) = &handler.name {
                            self.emit(Instruction::StoreName {
                                name: alias.clone(),
                            });
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
                    self.compile_statements(&handler.body)?;
                    self.emit(Instruction::Jump {
                        target: finally_label,
                    });

                    // Emit a new label for the next handler
                    self.set_label(handler_label);
                    handler_label = self.new_label();
                }
                self.emit(Instruction::Jump {
                    target: handler_label,
                });
                self.set_label(handler_label);
                // If code flows here, we have an unhandled exception,
                // emit finally code and raise again!
                // Duplicate finally code here:
                // TODO: this bytecode is now duplicate, could this be
                // improved?
                if let Some(statements) = finalbody {
                    self.compile_statements(statements)?;
                }
                self.emit(Instruction::Raise { argc: 1 });

                // We successfully ran the try block:
                // else:
                self.set_label(else_label);
                if let Some(statements) = orelse {
                    self.compile_statements(statements)?;
                }

                // finally:
                self.set_label(finally_label);
                if let Some(statements) = finalbody {
                    self.compile_statements(statements)?;
                }

                // unimplemented!();
            }
            ast::Statement::FunctionDef {
                name,
                args,
                body,
                decorator_list,
            } => {
                // Create bytecode for this function:
                let flags = self.enter_function(name, args)?;
                self.compile_statements(body)?;

                // Emit None at end:
                self.emit(Instruction::LoadConst {
                    value: bytecode::Constant::None,
                });
                self.emit(Instruction::ReturnValue);
                let code = self.pop_code_object();

                self.prepare_decorators(decorator_list)?;
                self.emit(Instruction::LoadConst {
                    value: bytecode::Constant::Code { code },
                });
                self.emit(Instruction::LoadConst {
                    value: bytecode::Constant::String {
                        value: name.clone(),
                    },
                });

                // Turn code object into function object:
                self.emit(Instruction::MakeFunction { flags });
                self.apply_decorators(decorator_list);

                self.emit(Instruction::StoreName {
                    name: name.to_string(),
                });
            }
            ast::Statement::ClassDef {
                name,
                body,
                bases,
                keywords,
                decorator_list,
            } => {
                self.prepare_decorators(decorator_list)?;
                self.emit(Instruction::LoadBuildClass);
                let line_number = self.get_source_line_number();
                self.code_object_stack.push(CodeObject::new(
                    vec![String::from("__locals__")],
                    None,
                    vec![],
                    None,
                    self.source_path.clone(),
                    line_number,
                    name.clone(),
                ));
                self.emit(Instruction::LoadName {
                    name: String::from("__locals__"),
                });
                self.emit(Instruction::StoreLocals);
                self.compile_statements(body)?;
                self.emit(Instruction::LoadConst {
                    value: bytecode::Constant::None,
                });
                self.emit(Instruction::ReturnValue);

                let code = self.pop_code_object();
                self.emit(Instruction::LoadConst {
                    value: bytecode::Constant::Code { code },
                });
                self.emit(Instruction::LoadConst {
                    value: bytecode::Constant::String {
                        value: name.clone(),
                    },
                });

                // Turn code object into function object:
                self.emit(Instruction::MakeFunction {
                    flags: bytecode::FunctionOpArg::empty(),
                });

                self.emit(Instruction::LoadConst {
                    value: bytecode::Constant::String {
                        value: name.clone(),
                    },
                });

                for base in bases {
                    self.compile_expression(base)?;
                }

                if !keywords.is_empty() {
                    let mut kwarg_names = vec![];
                    for keyword in keywords {
                        if let Some(name) = &keyword.name {
                            kwarg_names.push(bytecode::Constant::String {
                                value: name.to_string(),
                            });
                        } else {
                            // This means **kwargs!
                            panic!("name must be set");
                        }
                        self.compile_expression(&keyword.value)?;
                    }

                    self.emit(Instruction::LoadConst {
                        value: bytecode::Constant::Tuple {
                            elements: kwarg_names,
                        },
                    });
                    self.emit(Instruction::CallFunction {
                        typ: CallType::Keyword(2 + keywords.len() + bases.len()),
                    });
                } else {
                    self.emit(Instruction::CallFunction {
                        typ: CallType::Positional(2 + bases.len()),
                    });
                }

                self.apply_decorators(decorator_list);

                self.emit(Instruction::StoreName {
                    name: name.to_string(),
                });
            }
            ast::Statement::Assert { test, msg } => {
                // TODO: if some flag, ignore all assert statements!

                let end_label = self.new_label();
                self.compile_test(test, Some(end_label), None, EvalContext::Statement)?;
                self.emit(Instruction::LoadName {
                    name: String::from("AssertionError"),
                });
                match msg {
                    Some(e) => {
                        self.compile_expression(e)?;
                        self.emit(Instruction::CallFunction {
                            typ: CallType::Positional(1),
                        });
                    }
                    None => {
                        self.emit(Instruction::CallFunction {
                            typ: CallType::Positional(0),
                        });
                    }
                }
                self.emit(Instruction::Raise { argc: 1 });
                self.set_label(end_label);
            }
            ast::Statement::Break => {
                self.emit(Instruction::Break);
            }
            ast::Statement::Continue => {
                self.emit(Instruction::Continue);
            }
            ast::Statement::Return { value } => {
                match value {
                    Some(e) => {
                        let size = e.len();
                        for v in e {
                            self.compile_expression(v)?;
                        }

                        // If we have more than 1 return value, make it a tuple:
                        if size > 1 {
                            self.emit(Instruction::BuildTuple {
                                size,
                                unpack: false,
                            });
                        }
                    }
                    None => {
                        self.emit(Instruction::LoadConst {
                            value: bytecode::Constant::None,
                        });
                    }
                }

                self.emit(Instruction::ReturnValue);
            }
            ast::Statement::Assign { targets, value } => {
                self.compile_expression(value)?;

                for (i, target) in targets.iter().enumerate() {
                    if i + 1 != targets.len() {
                        self.emit(Instruction::Duplicate);
                    }
                    self.compile_store(target)?;
                }
            }
            ast::Statement::AugAssign { target, op, value } => {
                self.compile_expression(target)?;
                self.compile_expression(value)?;

                // Perform operation:
                self.compile_op(op);
                self.compile_store(target)?;
            }
            ast::Statement::Delete { targets } => {
                for target in targets {
                    match target {
                        ast::Expression::Identifier { name } => {
                            self.emit(Instruction::DeleteName {
                                name: name.to_string(),
                            });
                        }
                        ast::Expression::Attribute { value, name } => {
                            self.compile_expression(value)?;
                            self.emit(Instruction::DeleteAttr {
                                name: name.to_string(),
                            });
                        }
                        ast::Expression::Subscript { a, b } => {
                            self.compile_expression(a)?;
                            self.compile_expression(b)?;
                            self.emit(Instruction::DeleteSubscript);
                        }
                        _ => {
                            return Err("Invalid delete statement".to_string());
                        }
                    }
                }
            }
            ast::Statement::Pass => {
                self.emit(Instruction::Pass);
            }
        }
        Ok(())
    }

    fn enter_function(
        &mut self,
        name: &str,
        args: &ast::Parameters,
    ) -> Result<bytecode::FunctionOpArg, String> {
        let have_kwargs = !args.defaults.is_empty();
        if have_kwargs {
            // Construct a tuple:
            let size = args.defaults.len();
            for element in &args.defaults {
                self.compile_expression(element)?;
            }
            self.emit(Instruction::BuildTuple {
                size,
                unpack: false,
            });
        }

        let line_number = self.get_source_line_number();
        self.code_object_stack.push(CodeObject::new(
            args.args.clone(),
            args.vararg.clone(),
            args.kwonlyargs.clone(),
            args.kwarg.clone(),
            self.source_path.clone(),
            line_number,
            name.to_string(),
        ));

        let mut flags = bytecode::FunctionOpArg::empty();
        if have_kwargs {
            flags |= bytecode::FunctionOpArg::HAS_DEFAULTS;
        }

        Ok(flags)
    }

    fn prepare_decorators(&mut self, decorator_list: &[ast::Expression]) -> Result<(), String> {
        for decorator in decorator_list {
            self.compile_expression(decorator)?;
        }
        Ok(())
    }

    fn apply_decorators(&mut self, decorator_list: &[ast::Expression]) {
        // Apply decorators:
        for _ in decorator_list {
            self.emit(Instruction::CallFunction {
                typ: CallType::Positional(1),
            });
        }
    }

    fn compile_store(&mut self, target: &ast::Expression) -> Result<(), String> {
        match target {
            ast::Expression::Identifier { name } => {
                self.emit(Instruction::StoreName {
                    name: name.to_string(),
                });
            }
            ast::Expression::Subscript { a, b } => {
                self.compile_expression(a)?;
                self.compile_expression(b)?;
                self.emit(Instruction::StoreSubscript);
            }
            ast::Expression::Attribute { value, name } => {
                self.compile_expression(value)?;
                self.emit(Instruction::StoreAttr {
                    name: name.to_string(),
                });
            }
            ast::Expression::Tuple { elements } => {
                let mut seen_star = false;

                // Scan for star args:
                for (i, element) in elements.iter().enumerate() {
                    if let ast::Expression::Starred { .. } = element {
                        if seen_star {
                            return Err("two starred expressions in assignment".to_string());
                        } else {
                            seen_star = true;
                            self.emit(Instruction::UnpackEx {
                                before: i,
                                after: elements.len() - i - 1,
                            });
                        }
                    }
                }

                if !seen_star {
                    self.emit(Instruction::UnpackSequence {
                        size: elements.len(),
                    });
                }

                for element in elements {
                    if let ast::Expression::Starred { value } = element {
                        self.compile_store(value)?;
                    } else {
                        self.compile_store(element)?;
                    }
                }
            }
            _ => {
                return Err(format!("Cannot store value into: {:?}", target));
            }
        }
        Ok(())
    }

    fn compile_op(&mut self, op: &ast::Operator) {
        let i = match op {
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
        self.emit(Instruction::BinaryOperation { op: i });
    }

    fn compile_test(
        &mut self,
        expression: &ast::Expression,
        true_label: Option<Label>,
        false_label: Option<Label>,
        context: EvalContext,
    ) -> Result<(), String> {
        // Compile expression for test, and jump to label if false
        match expression {
            ast::Expression::BoolOp { a, op, b } => match op {
                ast::BooleanOperator::And => {
                    let f = false_label.unwrap_or_else(|| self.new_label());
                    self.compile_test(a, None, Some(f), context)?;
                    self.compile_test(b, true_label, false_label, context)?;
                    if false_label.is_none() {
                        self.set_label(f);
                    }
                }
                ast::BooleanOperator::Or => {
                    let t = true_label.unwrap_or_else(|| self.new_label());
                    self.compile_test(a, Some(t), None, context)?;
                    self.compile_test(b, true_label, false_label, context)?;
                    if true_label.is_none() {
                        self.set_label(t);
                    }
                }
            },
            _ => {
                self.compile_expression(expression)?;
                match context {
                    EvalContext::Statement => {
                        if let Some(true_label) = true_label {
                            self.emit(Instruction::JumpIf { target: true_label });
                        }
                        if let Some(false_label) = false_label {
                            self.emit(Instruction::JumpIfFalse {
                                target: false_label,
                            });
                        }
                    }
                    EvalContext::Expression => {
                        if let Some(true_label) = true_label {
                            self.emit(Instruction::Duplicate);
                            self.emit(Instruction::JumpIf { target: true_label });
                            self.emit(Instruction::Pop);
                        }
                        if let Some(false_label) = false_label {
                            self.emit(Instruction::Duplicate);
                            self.emit(Instruction::JumpIfFalse {
                                target: false_label,
                            });
                            self.emit(Instruction::Pop);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn compile_expression(&mut self, expression: &ast::Expression) -> Result<(), String> {
        trace!("Compiling {:?}", expression);
        match expression {
            ast::Expression::Call {
                function,
                args,
                keywords,
            } => self.compile_call(function, args, keywords)?,
            ast::Expression::BoolOp { .. } => {
                self.compile_test(expression, None, None, EvalContext::Expression)?
            }
            ast::Expression::Binop { a, op, b } => {
                self.compile_expression(a)?;
                self.compile_expression(b)?;

                // Perform operation:
                self.compile_op(op);
            }
            ast::Expression::Subscript { a, b } => {
                self.compile_expression(a)?;
                self.compile_expression(b)?;
                self.emit(Instruction::BinaryOperation {
                    op: bytecode::BinaryOperator::Subscript,
                });
            }
            ast::Expression::Unop { op, a } => {
                self.compile_expression(a)?;

                // Perform operation:
                let i = match op {
                    ast::UnaryOperator::Pos => bytecode::UnaryOperator::Plus,
                    ast::UnaryOperator::Neg => bytecode::UnaryOperator::Minus,
                    ast::UnaryOperator::Not => bytecode::UnaryOperator::Not,
                    ast::UnaryOperator::Inv => bytecode::UnaryOperator::Invert,
                };
                let i = Instruction::UnaryOperation { op: i };
                self.emit(i);
            }
            ast::Expression::Attribute { value, name } => {
                self.compile_expression(value)?;
                self.emit(Instruction::LoadAttr {
                    name: name.to_string(),
                });
            }
            ast::Expression::Compare { a, op, b } => {
                self.compile_expression(a)?;
                self.compile_expression(b)?;

                let i = match op {
                    ast::Comparison::Equal => bytecode::ComparisonOperator::Equal,
                    ast::Comparison::NotEqual => bytecode::ComparisonOperator::NotEqual,
                    ast::Comparison::Less => bytecode::ComparisonOperator::Less,
                    ast::Comparison::LessOrEqual => bytecode::ComparisonOperator::LessOrEqual,
                    ast::Comparison::Greater => bytecode::ComparisonOperator::Greater,
                    ast::Comparison::GreaterOrEqual => bytecode::ComparisonOperator::GreaterOrEqual,
                    ast::Comparison::In => bytecode::ComparisonOperator::In,
                    ast::Comparison::NotIn => bytecode::ComparisonOperator::NotIn,
                    ast::Comparison::Is => bytecode::ComparisonOperator::Is,
                    ast::Comparison::IsNot => bytecode::ComparisonOperator::IsNot,
                };
                let i = Instruction::CompareOperation { op: i };
                self.emit(i);
            }
            ast::Expression::Number { value } => {
                let const_value = match value {
                    ast::Number::Integer { value } => bytecode::Constant::Integer {
                        value: value.clone(),
                    },
                    ast::Number::Float { value } => bytecode::Constant::Float { value: *value },
                    ast::Number::Complex { real, imag } => bytecode::Constant::Complex {
                        value: Complex64::new(*real, *imag),
                    },
                };
                self.emit(Instruction::LoadConst { value: const_value });
            }
            ast::Expression::List { elements } => {
                let size = elements.len();
                let must_unpack = self.gather_elements(elements)?;
                self.emit(Instruction::BuildList {
                    size,
                    unpack: must_unpack,
                });
            }
            ast::Expression::Tuple { elements } => {
                let size = elements.len();
                let must_unpack = self.gather_elements(elements)?;
                self.emit(Instruction::BuildTuple {
                    size,
                    unpack: must_unpack,
                });
            }
            ast::Expression::Set { elements } => {
                let size = elements.len();
                let must_unpack = self.gather_elements(elements)?;
                self.emit(Instruction::BuildSet {
                    size,
                    unpack: must_unpack,
                });
            }
            ast::Expression::Dict { elements } => {
                let size = elements.len();
                for (key, value) in elements {
                    self.compile_expression(key)?;
                    self.compile_expression(value)?;
                }
                self.emit(Instruction::BuildMap {
                    size,
                    unpack: false,
                });
            }
            ast::Expression::Slice { elements } => {
                let size = elements.len();
                for element in elements {
                    self.compile_expression(element)?;
                }
                self.emit(Instruction::BuildSlice { size });
            }
            ast::Expression::Yield { value } => {
                self.mark_generator();
                match value {
                    Some(expression) => self.compile_expression(expression)?,
                    None => self.emit(Instruction::LoadConst {
                        value: bytecode::Constant::None,
                    }),
                };
                self.emit(Instruction::YieldValue);
            }
            ast::Expression::YieldFrom { value } => {
                self.mark_generator();
                self.compile_expression(value)?;
                self.emit(Instruction::GetIter);
                self.emit(Instruction::LoadConst {
                    value: bytecode::Constant::None,
                });
                self.emit(Instruction::YieldFrom);
            }
            ast::Expression::True => {
                self.emit(Instruction::LoadConst {
                    value: bytecode::Constant::Boolean { value: true },
                });
            }
            ast::Expression::False => {
                self.emit(Instruction::LoadConst {
                    value: bytecode::Constant::Boolean { value: false },
                });
            }
            ast::Expression::None => {
                self.emit(Instruction::LoadConst {
                    value: bytecode::Constant::None,
                });
            }
            ast::Expression::String { value } => {
                self.emit(Instruction::LoadConst {
                    value: bytecode::Constant::String {
                        value: value.to_string(),
                    },
                });
            }
            ast::Expression::Bytes { value } => {
                self.emit(Instruction::LoadConst {
                    value: bytecode::Constant::Bytes {
                        value: value.clone(),
                    },
                });
            }
            ast::Expression::Identifier { name } => {
                self.emit(Instruction::LoadName {
                    name: name.to_string(),
                });
            }
            ast::Expression::Lambda { args, body } => {
                let name = "<lambda>".to_string();
                let flags = self.enter_function(&name, args)?;
                self.compile_expression(body)?;
                self.emit(Instruction::ReturnValue);
                let code = self.pop_code_object();
                self.emit(Instruction::LoadConst {
                    value: bytecode::Constant::Code { code },
                });
                self.emit(Instruction::LoadConst {
                    value: bytecode::Constant::String { value: name },
                });
                // Turn code object into function object:
                self.emit(Instruction::MakeFunction { flags });
            }
            ast::Expression::Comprehension { kind, generators } => {
                self.compile_comprehension(kind, generators)?;
            }
            ast::Expression::Starred { value } => {
                self.compile_expression(value)?;
                self.emit(Instruction::Unpack);
                panic!("We should not just unpack a starred args, since the size is unknown.");
            }
            ast::Expression::IfExpression { test, body, orelse } => {
                let no_label = self.new_label();
                let end_label = self.new_label();
                self.compile_test(test, None, Some(no_label), EvalContext::Expression)?;
                self.compile_expression(body)?;
                self.emit(Instruction::Jump { target: end_label });
                self.set_label(no_label);
                self.compile_expression(orelse)?;
                self.set_label(end_label);
            }
        }
        Ok(())
    }

    fn compile_call(
        &mut self,
        function: &ast::Expression,
        args: &[ast::Expression],
        keywords: &[ast::Keyword],
    ) -> Result<(), String> {
        self.compile_expression(function)?;
        let count = args.len() + keywords.len();

        // Normal arguments:
        let must_unpack = self.gather_elements(args)?;
        let has_double_star = keywords.iter().any(|k| k.name.is_none());

        if must_unpack || has_double_star {
            // Create a tuple with positional args:
            self.emit(Instruction::BuildTuple {
                size: args.len(),
                unpack: must_unpack,
            });

            // Create an optional map with kw-args:
            if !keywords.is_empty() {
                for keyword in keywords {
                    if let Some(name) = &keyword.name {
                        self.emit(Instruction::LoadConst {
                            value: bytecode::Constant::String {
                                value: name.to_string(),
                            },
                        });
                        self.compile_expression(&keyword.value)?;
                        if has_double_star {
                            self.emit(Instruction::BuildMap {
                                size: 1,
                                unpack: false,
                            });
                        }
                    } else {
                        // This means **kwargs!
                        self.compile_expression(&keyword.value)?;
                    }
                }

                self.emit(Instruction::BuildMap {
                    size: keywords.len(),
                    unpack: has_double_star,
                });

                self.emit(Instruction::CallFunction {
                    typ: CallType::Ex(true),
                });
            } else {
                self.emit(Instruction::CallFunction {
                    typ: CallType::Ex(false),
                });
            }
        } else {
            // Keyword arguments:
            if !keywords.is_empty() {
                let mut kwarg_names = vec![];
                for keyword in keywords {
                    if let Some(name) = &keyword.name {
                        kwarg_names.push(bytecode::Constant::String {
                            value: name.to_string(),
                        });
                    } else {
                        // This means **kwargs!
                        panic!("name must be set");
                    }
                    self.compile_expression(&keyword.value)?;
                }

                self.emit(Instruction::LoadConst {
                    value: bytecode::Constant::Tuple {
                        elements: kwarg_names,
                    },
                });
                self.emit(Instruction::CallFunction {
                    typ: CallType::Keyword(count),
                });
            } else {
                self.emit(Instruction::CallFunction {
                    typ: CallType::Positional(count),
                });
            }
        }
        Ok(())
    }

    // Given a vector of expr / star expr generate code which gives either
    // a list of expressions on the stack, or a list of tuples.
    fn gather_elements(&mut self, elements: &[ast::Expression]) -> Result<bool, String> {
        // First determine if we have starred elements:
        let has_stars = elements.iter().any(|e| {
            if let ast::Expression::Starred { .. } = e {
                true
            } else {
                false
            }
        });

        for element in elements {
            if let ast::Expression::Starred { value } = element {
                self.compile_expression(value)?;
            } else {
                self.compile_expression(element)?;
                if has_stars {
                    self.emit(Instruction::BuildTuple {
                        size: 1,
                        unpack: false,
                    });
                }
            }
        }

        Ok(has_stars)
    }

    fn compile_comprehension(
        &mut self,
        kind: &ast::ComprehensionKind,
        generators: &[ast::Comprehension],
    ) -> Result<(), String> {
        // We must have at least one generator:
        assert!(!generators.is_empty());

        let name = match kind {
            ast::ComprehensionKind::GeneratorExpression { .. } => "<genexpr>",
            ast::ComprehensionKind::List { .. } => "<listcomp>",
            ast::ComprehensionKind::Set { .. } => "<setcomp>",
            ast::ComprehensionKind::Dict { .. } => "<dictcomp>",
        }
        .to_string();

        let line_number = self.get_source_line_number();
        // Create magnificent function <listcomp>:
        self.code_object_stack.push(CodeObject::new(
            vec![".0".to_string()],
            None,
            vec![],
            None,
            self.source_path.clone(),
            line_number,
            name.clone(),
        ));

        // Create empty object of proper type:
        match kind {
            ast::ComprehensionKind::GeneratorExpression { .. } => {}
            ast::ComprehensionKind::List { .. } => {
                self.emit(Instruction::BuildList {
                    size: 0,
                    unpack: false,
                });
            }
            ast::ComprehensionKind::Set { .. } => {
                self.emit(Instruction::BuildSet {
                    size: 0,
                    unpack: false,
                });
            }
            ast::ComprehensionKind::Dict { .. } => {
                self.emit(Instruction::BuildMap {
                    size: 0,
                    unpack: false,
                });
            }
        }

        let mut loop_labels = vec![];
        for generator in generators {
            if loop_labels.is_empty() {
                // Load iterator onto stack (passed as first argument):
                self.emit(Instruction::LoadName {
                    name: String::from(".0"),
                });
            } else {
                // Evaluate iterated item:
                self.compile_expression(&generator.iter)?;

                // Get iterator / turn item into an iterator
                self.emit(Instruction::GetIter);
            }

            // Setup for loop:
            let start_label = self.new_label();
            let end_label = self.new_label();
            loop_labels.push((start_label, end_label));
            self.emit(Instruction::SetupLoop {
                start: start_label,
                end: end_label,
            });
            self.set_label(start_label);
            self.emit(Instruction::ForIter { target: end_label });

            self.compile_store(&generator.target)?;

            // Now evaluate the ifs:
            for if_condition in &generator.ifs {
                self.compile_test(
                    if_condition,
                    None,
                    Some(start_label),
                    EvalContext::Statement,
                )?
            }
        }

        match kind {
            ast::ComprehensionKind::GeneratorExpression { element } => {
                self.compile_expression(element)?;
                self.mark_generator();
                self.emit(Instruction::YieldValue);
                self.emit(Instruction::Pop);
            }
            ast::ComprehensionKind::List { element } => {
                self.compile_expression(element)?;
                self.emit(Instruction::ListAppend {
                    i: 1 + generators.len(),
                });
            }
            ast::ComprehensionKind::Set { element } => {
                self.compile_expression(element)?;
                self.emit(Instruction::SetAdd {
                    i: 1 + generators.len(),
                });
            }
            ast::ComprehensionKind::Dict { key, value } => {
                self.compile_expression(value)?;
                self.compile_expression(key)?;

                self.emit(Instruction::MapAdd {
                    i: 1 + generators.len(),
                });
            }
        }

        for (start_label, end_label) in loop_labels.iter().rev() {
            // Repeat:
            self.emit(Instruction::Jump {
                target: *start_label,
            });

            // End of for loop:
            self.set_label(*end_label);
            self.emit(Instruction::PopBlock);
        }

        // Return freshly filled list:
        self.emit(Instruction::ReturnValue);

        // Fetch code for listcomp function:
        let code = self.pop_code_object();

        // List comprehension code:
        self.emit(Instruction::LoadConst {
            value: bytecode::Constant::Code { code },
        });

        // List comprehension function name:
        self.emit(Instruction::LoadConst {
            value: bytecode::Constant::String { value: name },
        });

        // Turn code object into function object:
        self.emit(Instruction::MakeFunction {
            flags: bytecode::FunctionOpArg::empty(),
        });

        // Evaluate iterated item:
        self.compile_expression(&generators[0].iter)?;

        // Get iterator / turn item into an iterator
        self.emit(Instruction::GetIter);

        // Call just created <listcomp> function:
        self.emit(Instruction::CallFunction {
            typ: CallType::Positional(1),
        });
        Ok(())
    }

    // Low level helper functions:
    fn emit(&mut self, instruction: Instruction) {
        self.current_code_object().instructions.push(instruction);
        // TODO: insert source filename
        let location = self.current_source_location.clone();
        self.current_code_object().locations.push(location);
    }

    fn current_code_object(&mut self) -> &mut CodeObject {
        self.code_object_stack.last_mut().unwrap()
    }

    // Generate a new label
    fn new_label(&mut self) -> Label {
        let l = self.nxt_label;
        self.nxt_label += 1;
        l
    }

    // Assign current position the given label
    fn set_label(&mut self, label: Label) {
        let position = self.current_code_object().instructions.len();
        // assert!(label not in self.label_map)
        self.current_code_object().label_map.insert(label, position);
    }

    fn set_source_location(&mut self, location: &ast::Location) {
        self.current_source_location = location.clone();
    }

    fn get_source_line_number(&mut self) -> usize {
        self.current_source_location.get_row()
    }

    fn mark_generator(&mut self) {
        self.current_code_object().is_generator = true;
    }
}

#[cfg(test)]
mod tests {
    use super::bytecode::CodeObject;
    use super::bytecode::Constant::*;
    use super::bytecode::Instruction::*;
    use super::Compiler;
    use rustpython_parser::parser;
    fn compile_exec(source: &str) -> CodeObject {
        let mut compiler = Compiler::new();
        compiler.push_new_code_object(Option::None, "<module>".to_string());
        let ast = parser::parse_program(&source.to_string()).unwrap();
        compiler.compile_program(&ast).unwrap();
        compiler.pop_code_object()
    }

    #[test]
    fn test_if_ors() {
        let code = compile_exec("if True or False or False:\n pass\n");
        assert_eq!(
            vec![
                LoadConst {
                    value: Boolean { value: true }
                },
                JumpIf { target: 1 },
                LoadConst {
                    value: Boolean { value: false }
                },
                JumpIf { target: 1 },
                LoadConst {
                    value: Boolean { value: false }
                },
                JumpIfFalse { target: 0 },
                Pass,
                LoadConst { value: None },
                ReturnValue
            ],
            code.instructions
        );
    }

    #[test]
    fn test_if_ands() {
        let code = compile_exec("if True and False and False:\n pass\n");
        assert_eq!(
            vec![
                LoadConst {
                    value: Boolean { value: true }
                },
                JumpIfFalse { target: 0 },
                LoadConst {
                    value: Boolean { value: false }
                },
                JumpIfFalse { target: 0 },
                LoadConst {
                    value: Boolean { value: false }
                },
                JumpIfFalse { target: 0 },
                Pass,
                LoadConst { value: None },
                ReturnValue
            ],
            code.instructions
        );
    }

    #[test]
    fn test_if_mixed() {
        let code = compile_exec("if (True and False) or (False and True):\n pass\n");
        assert_eq!(
            vec![
                LoadConst {
                    value: Boolean { value: true }
                },
                JumpIfFalse { target: 2 },
                LoadConst {
                    value: Boolean { value: false }
                },
                JumpIf { target: 1 },
                LoadConst {
                    value: Boolean { value: false }
                },
                JumpIfFalse { target: 0 },
                LoadConst {
                    value: Boolean { value: true }
                },
                JumpIfFalse { target: 0 },
                Pass,
                LoadConst { value: None },
                ReturnValue
            ],
            code.instructions
        );
    }
}
