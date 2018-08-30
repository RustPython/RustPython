/*
 * Take an AST and transform it into bytecode
 */

extern crate rustpython_parser;

use self::rustpython_parser::{ast, parser};
use super::bytecode::{self, CodeObject, Instruction};
use super::pyobject::{PyObject, PyObjectKind, PyObjectRef};
use super::vm::VirtualMachine;

struct Compiler {
    code_object_stack: Vec<CodeObject>,
    nxt_label: usize,
    current_source_location: ast::Location,
}

pub fn compile(
    vm: &mut VirtualMachine,
    source: &String,
    mode: Mode,
    source_path: Option<String>,
) -> Result<PyObjectRef, String> {
    let mut compiler = Compiler::new();
    compiler.push_new_code_object(source_path);
    match mode {
        Mode::Exec => match parser::parse_program(source) {
            Ok(ast) => {
                compiler.compile_program(&ast);
            }
            Err(msg) => return Err(msg),
        },
        Mode::Eval => match parser::parse_statement(source) {
            Ok(statement) => {
                if let &ast::Statement::Expression { ref expression } = &statement.node {
                    compiler.compile_expression(expression);
                    compiler.emit(Instruction::ReturnValue);
                } else {
                    return Err("Expecting expression, got statement".to_string());
                }
            }
            Err(msg) => return Err(msg),
        },
        Mode::Single => match parser::parse_program(source) {
            Ok(ast) => {
                for statement in ast.statements {
                    if let &ast::Statement::Expression { ref expression } = &statement.node {
                        compiler.compile_expression(expression);
                        compiler.emit(Instruction::PrintExpr);
                    } else {
                        compiler.compile_statement(&statement);
                    }
                }
                compiler.emit(Instruction::LoadConst {
                    value: bytecode::Constant::None,
                });
                compiler.emit(Instruction::ReturnValue);
            }
            Err(msg) => return Err(msg),
        },
    };

    let code = compiler.pop_code_object();
    trace!("Compilation completed: {:?}", code);
    Ok(PyObject::new(
        PyObjectKind::Code { code: code },
        vm.get_type(),
    ))
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
            current_source_location: ast::Location::default(),
        }
    }

    fn push_new_code_object(&mut self, source_path: Option<String>) {
        self.code_object_stack
            .push(CodeObject::new(Vec::new(), source_path.clone()));
    }

    fn pop_code_object(&mut self) -> CodeObject {
        self.code_object_stack.pop().unwrap()
    }

    fn compile_program(&mut self, program: &ast::Program) {
        let size_before = self.code_object_stack.len();
        self.compile_statements(&program.statements);
        assert!(self.code_object_stack.len() == size_before);

        // Emit None at end:
        self.emit(Instruction::LoadConst {
            value: bytecode::Constant::None,
        });
        self.emit(Instruction::ReturnValue);
    }

    fn compile_statements(&mut self, statements: &Vec<ast::LocatedStatement>) {
        for statement in statements {
            self.compile_statement(statement)
        }
    }

    fn compile_statement(&mut self, statement: &ast::LocatedStatement) {
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
            ast::Statement::Expression { expression } => {
                self.compile_expression(expression);

                // Pop result of stack, since we not use it:
                self.emit(Instruction::Pop);
            }
            ast::Statement::If { test, body, orelse } => {
                let end_label = self.new_label();
                match orelse {
                    None => {
                        // Only if:
                        self.compile_test(test, None, Some(end_label), EvalContext::Statement);
                        self.compile_statements(body);
                        self.set_label(end_label);
                    }
                    Some(statements) => {
                        // if - else:
                        let else_label = self.new_label();
                        self.compile_test(test, None, Some(else_label), EvalContext::Statement);
                        self.compile_statements(body);
                        self.emit(Instruction::Jump { target: end_label });

                        // else:
                        self.set_label(else_label);
                        self.compile_statements(statements);
                    }
                }
                self.set_label(end_label);
            }
            ast::Statement::While {
                test,
                body,
                orelse: _,
            } => {
                // TODO: Handle while-loop else clauses
                let start_label = self.new_label();
                let end_label = self.new_label();
                self.emit(Instruction::SetupLoop {
                    start: start_label,
                    end: end_label,
                });

                self.set_label(start_label);

                self.compile_test(test, None, Some(end_label), EvalContext::Statement);
                self.compile_statements(body);
                self.emit(Instruction::Jump {
                    target: start_label,
                });
                self.set_label(end_label);
            }
            ast::Statement::With { items: _, body: _ } => {
                // TODO
            }
            ast::Statement::For {
                target,
                iter,
                body,
                orelse: _,
            } => {
                // TODO: Handle for loop else clauses
                // The thing iterated:
                for i in iter {
                    self.compile_expression(i);
                }

                // Retrieve iterator
                self.emit(Instruction::GetIter);

                // Start loop
                let start_label = self.new_label();
                let end_label = self.new_label();
                self.emit(Instruction::SetupLoop {
                    start: start_label,
                    end: end_label,
                });
                self.set_label(start_label);
                self.emit(Instruction::ForIter);

                // Start of loop iteration, set targets:
                for t in target {
                    match t {
                        ast::Expression::Identifier { name } => {
                            self.emit(Instruction::StoreName {
                                name: name.to_string(),
                            });
                        }
                        _ => panic!("Not impl"),
                    }
                }

                // Body of loop:
                self.compile_statements(body);
                self.emit(Instruction::Jump {
                    target: start_label,
                });
                self.set_label(end_label);
                self.emit(Instruction::PopBlock);
            }
            ast::Statement::Raise { expression } => match expression {
                Some(value) => {
                    self.compile_expression(value);
                    self.emit(Instruction::Raise { argc: 1 });
                }
                None => {
                    unimplemented!();
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
                self.compile_statements(body);
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
                        self.compile_expression(exc_type);
                        self.emit(Instruction::CallFunction { count: 2 });

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
                    self.compile_statements(&handler.body);
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
                    self.compile_statements(statements);
                }
                self.emit(Instruction::Raise { argc: 1 });

                // We successfully ran the try block:
                // else:
                self.set_label(else_label);
                if let Some(statements) = orelse {
                    self.compile_statements(statements);
                }

                // finally:
                self.set_label(finally_label);
                if let Some(statements) = finalbody {
                    self.compile_statements(statements);
                }

                // unimplemented!();
            }
            ast::Statement::FunctionDef { name, args, body } => {
                // Create bytecode for this function:
                let mut names = vec![];
                let mut default_elements = vec![];

                for (name, default) in args {
                    names.push(name.clone());
                    if let Some(default) = default {
                        default_elements.push(default.clone());
                    } else {
                        if default_elements.len() > 0 {
                            // Once we have started with defaults, all remaining arguments must
                            // have defaults
                            panic!("non-default argument follows default argument: {}", name);
                        }
                    }
                }

                let have_kwargs = default_elements.len() > 0;
                if have_kwargs {
                    self.compile_expression(&ast::Expression::Tuple {
                        elements: default_elements,
                    });
                }

                self.code_object_stack.push(CodeObject::new(names, None));
                self.compile_statements(body);

                // Emit None at end:
                self.emit(Instruction::LoadConst {
                    value: bytecode::Constant::None,
                });
                self.emit(Instruction::ReturnValue);

                let code = self.code_object_stack.pop().unwrap();
                self.emit(Instruction::LoadConst {
                    value: bytecode::Constant::Code { code: code },
                });
                self.emit(Instruction::LoadConst {
                    value: bytecode::Constant::String {
                        value: name.clone(),
                    },
                });

                // Turn code object into function object:
                let mut flags = bytecode::FunctionOpArg::empty();
                if have_kwargs {
                    flags = flags | bytecode::FunctionOpArg::HAS_DEFAULTS;
                }
                self.emit(Instruction::MakeFunction { flags: flags });
                self.emit(Instruction::StoreName {
                    name: name.to_string(),
                });
            }
            ast::Statement::ClassDef { name, body, args } => {
                self.emit(Instruction::LoadBuildClass);
                self.code_object_stack
                    .push(CodeObject::new(vec![String::from("__locals__")], None));
                self.emit(Instruction::LoadName {
                    name: String::from("__locals__"),
                });
                self.emit(Instruction::StoreLocals);
                self.compile_statements(body);
                self.emit(Instruction::LoadConst {
                    value: bytecode::Constant::None,
                });
                self.emit(Instruction::ReturnValue);

                let code = self.code_object_stack.pop().unwrap();
                self.emit(Instruction::LoadConst {
                    value: bytecode::Constant::Code { code: code },
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

                for base in args {
                    self.emit(Instruction::LoadName {
                        name: base.0.clone(),
                    });
                }
                self.emit(Instruction::CallFunction {
                    count: 2 + args.len(),
                });

                self.emit(Instruction::StoreName {
                    name: name.to_string(),
                });
            }
            ast::Statement::Assert { test, msg } => {
                // TODO: if some flag, ignore all assert statements!

                let end_label = self.new_label();
                self.compile_test(test, Some(end_label), None, EvalContext::Statement);
                self.emit(Instruction::LoadName {
                    name: String::from("AssertionError"),
                });
                match msg {
                    Some(e) => {
                        self.compile_expression(e);
                        self.emit(Instruction::CallFunction { count: 1 });
                    }
                    None => {
                        self.emit(Instruction::CallFunction { count: 0 });
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
                            self.compile_expression(v);
                        }

                        // If we have more than 1 return value, make it a tuple:
                        if size > 1 {
                            self.emit(Instruction::BuildTuple { size });
                        }
                    }
                    None => {
                        // TODO: Put none on stack
                    }
                }

                self.emit(Instruction::ReturnValue);
            }
            ast::Statement::Assign { targets, value } => {
                self.compile_expression(value);

                for target in targets {
                    self.compile_store(target);
                }
            }
            ast::Statement::AugAssign { target, op, value } => {
                self.compile_expression(target);
                self.compile_expression(value);

                // Perform operation:
                self.compile_op(op);
                self.compile_store(target);
            }
            ast::Statement::Delete { targets: _ } => {
                // TODO: Remove the given names from the scope
                // self.emit(Instruction::DeleteName);
            }
            ast::Statement::Pass => {
                self.emit(Instruction::Pass);
            }
        }
    }

    fn compile_store(&mut self, target: &ast::Expression) {
        match target {
            ast::Expression::Identifier { name } => {
                self.emit(Instruction::StoreName {
                    name: name.to_string(),
                });
            }
            ast::Expression::Subscript { a, b } => {
                self.compile_expression(a);
                self.compile_expression(b);
                self.emit(Instruction::StoreSubscript);
            }
            ast::Expression::Attribute { value, name } => {
                self.compile_expression(value);
                self.emit(Instruction::StoreAttr {
                    name: name.to_string(),
                });
            }
            _ => {
                panic!("WTF: {:?}", target);
            }
        }
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
    ) {
        // Compile expression for test, and jump to label if false
        match expression {
            ast::Expression::BoolOp { a, op, b } => match op {
                ast::BooleanOperator::And => {
                    let f = false_label.unwrap_or_else(|| self.new_label());
                    self.compile_test(a, None, Some(f), context);
                    self.compile_test(b, true_label, false_label, context);
                    if let None = false_label {
                        self.set_label(f);
                    }
                }
                ast::BooleanOperator::Or => {
                    let t = true_label.unwrap_or_else(|| self.new_label());
                    self.compile_test(a, Some(t), None, context);
                    self.compile_test(b, true_label, false_label, context);
                    if let None = true_label {
                        self.set_label(t);
                    }
                }
            },
            _ => {
                self.compile_expression(expression);
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
    }

    fn compile_expression(&mut self, expression: &ast::Expression) {
        trace!("Compiling {:?}", expression);
        match expression {
            ast::Expression::Call { function, args } => {
                self.compile_expression(&*function);
                let count = args.len();
                for arg in args {
                    self.compile_expression(arg)
                }
                self.emit(Instruction::CallFunction { count: count });
            }
            ast::Expression::BoolOp { .. } => {
                self.compile_test(expression, None, None, EvalContext::Expression)
            }
            ast::Expression::Binop { a, op, b } => {
                self.compile_expression(&*a);
                self.compile_expression(&*b);

                // Perform operation:
                self.compile_op(op);
            }
            ast::Expression::Subscript { a, b } => {
                self.compile_expression(&*a);
                self.compile_expression(&*b);
                self.emit(Instruction::BinaryOperation {
                    op: bytecode::BinaryOperator::Subscript,
                });
            }
            ast::Expression::Unop { op, a } => {
                self.compile_expression(&*a);

                // Perform operation:
                let i = match op {
                    ast::UnaryOperator::Neg => bytecode::UnaryOperator::Minus,
                    ast::UnaryOperator::Not => bytecode::UnaryOperator::Not,
                };
                let i = Instruction::UnaryOperation { op: i };
                self.emit(i);
            }
            ast::Expression::Attribute { value, name } => {
                self.compile_expression(&*value);
                self.emit(Instruction::LoadAttr {
                    name: name.to_string(),
                });
            }
            ast::Expression::Compare { a, op, b } => {
                self.compile_expression(&*a);
                self.compile_expression(&*b);

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
                    ast::Number::Integer { value } => bytecode::Constant::Integer { value: *value },
                    ast::Number::Float { value } => bytecode::Constant::Float { value: *value },
                };
                self.emit(Instruction::LoadConst { value: const_value });
            }
            ast::Expression::List { elements } => {
                let size = elements.len();
                for element in elements {
                    self.compile_expression(element);
                }
                self.emit(Instruction::BuildList { size: size });
            }
            ast::Expression::Tuple { elements } => {
                let size = elements.len();
                for element in elements {
                    self.compile_expression(element);
                }
                self.emit(Instruction::BuildTuple { size: size });
            }
            ast::Expression::Dict { elements } => {
                let size = elements.len();
                for (key, value) in elements {
                    self.compile_expression(key);
                    self.compile_expression(value);
                }
                self.emit(Instruction::BuildMap { size: size });
            }
            ast::Expression::Slice { elements } => {
                let size = elements.len();
                for element in elements {
                    self.compile_expression(element);
                }
                self.emit(Instruction::BuildSlice { size: size });
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
            ast::Expression::Identifier { name } => {
                self.emit(Instruction::LoadName {
                    name: name.to_string(),
                });
            }
            ast::Expression::Lambda { args, body } => {
                self.code_object_stack.push(CodeObject::new(
                    args.iter().map(|(name, _default)| name.clone()).collect(),
                    None,
                ));
                self.compile_expression(body);
                self.emit(Instruction::ReturnValue);
                let code = self.code_object_stack.pop().unwrap();
                self.emit(Instruction::LoadConst {
                    value: bytecode::Constant::Code { code: code },
                });
                self.emit(Instruction::LoadConst {
                    value: bytecode::Constant::String {
                        value: String::from("<lambda>"),
                    },
                });
                // Turn code object into function object:
                self.emit(Instruction::MakeFunction {
                    flags: bytecode::FunctionOpArg::empty(),
                });
            }
        }
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
}

#[cfg(test)]
mod tests {
    use super::bytecode::CodeObject;
    use super::bytecode::Constant::*;
    use super::bytecode::Instruction::*;
    use super::rustpython_parser::parser;
    use super::Compiler;
    fn compile_exec(source: &str) -> CodeObject {
        let mut compiler = Compiler::new();
        compiler.push_new_code_object(Option::None);
        let ast = parser::parse_program(&source.to_string()).unwrap();
        compiler.compile_program(&ast);
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
