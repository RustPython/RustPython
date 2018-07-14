/*
 * Take an AST and transform it into bytecode
 */

extern crate rustpython_parser;

use self::rustpython_parser::ast;
use super::bytecode::{self, CodeObject, Instruction};

struct Compiler {
    code_object_stack: Vec<CodeObject>,
    nxt_label: usize,
}

pub fn compile(p: ast::Program, mode: Mode) -> CodeObject {
    let mut compiler = Compiler::new();
    compiler.compile_program(p)
}

pub enum Mode {
    Exec,
    Eval,
}

type Label = usize;

impl Compiler {
    fn new() -> Self {
        Compiler {
            code_object_stack: Vec::new(),
            nxt_label: 0,
        }
    }

    fn compile_program(&mut self, program: ast::Program) -> CodeObject {
        self.code_object_stack.push(CodeObject::new());
        self.compile_statements(program.statements);
        assert!(self.code_object_stack.len() == 1);

        // Emit None at end:
        self.emit(Instruction::LoadConst {
            value: bytecode::Constant::None,
        });
        self.emit(Instruction::ReturnValue);

        self.code_object_stack.pop().unwrap()
    }

    fn compile_statements(&mut self, statements: Vec<ast::Statement>) {
        for statement in statements {
            self.compile_statement(statement)
        }
    }

    fn compile_statement(&mut self, statement: ast::Statement) {
        trace!("Compiling {:?}", statement);
        match statement {
            ast::Statement::Import { name } => {
                self.emit(Instruction::Import { name: name.clone() });
                self.emit(Instruction::StoreName { name: name.clone() });
            }
            ast::Statement::Expression { expression } => {
                self.compile_expression(expression);

                // Pop result of stack, since we not use it:
                self.emit(Instruction::Pop);
            }
            ast::Statement::If { test, body } => {
                self.compile_expression(test);
                let else_label = self.new_label();
                self.emit(Instruction::JumpIf { target: else_label });
                self.compile_statements(body);
                self.set_label(else_label);
            }
            ast::Statement::While { test, body } => {
                let start_label = self.new_label();
                let end_label = self.new_label();
                self.set_label(start_label);

                self.compile_expression(test);
                self.emit(Instruction::UnaryOperation {
                    op: bytecode::UnaryOperator::Not,
                });
                self.emit(Instruction::JumpIf { target: end_label });
                self.compile_statements(body);
                self.emit(Instruction::Jump {
                    target: start_label,
                });
                self.set_label(end_label);
            }
            ast::Statement::With { items, body } => {
                // TODO
            }
            ast::Statement::For {
                target,
                iter,
                body,
                or_else,
            } => {
                // The thing iterated:
                for i in iter {
                    self.compile_expression(i);
                }

                // Retrieve iterator
                self.emit(Instruction::GetIter);

                // Start loop
                let start_label = self.new_label();
                let end_label = self.new_label();
                self.emit(Instruction::PushBlock {
                    start: start_label,
                    end: end_label,
                });
                self.set_label(start_label);
                self.emit(Instruction::ForIter);

                // Start of loop iteration, set targets:
                for t in target {
                    match t {
                        ast::Expression::Identifier { name } => {
                            self.emit(Instruction::StoreName { name: name });
                        }
                        _ => panic!("Not impl"),
                    }
                }

                // Body of loop:
                self.compile_statements(body);
                self.set_label(end_label);
                self.emit(Instruction::PopBlock);
            }
            ast::Statement::FunctionDef { name, body } => {
                // Create bytecode for this function:
                self.code_object_stack.push(CodeObject::new());
                self.compile_statements(body);
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
                self.emit(Instruction::MakeFunction);
                self.emit(Instruction::StoreName { name: name });
            }
            ast::Statement::ClassDef { name } => {
                // TODO?
            }
            ast::Statement::Assert { test, msg } => {
                // TODO: if some flag, ignore all assert statements!

                self.compile_expression(test);

                // if true, jump over raise:
                let end_label = self.new_label();
                self.emit(Instruction::JumpIf { target: end_label });

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
                    match target {
                        ast::Expression::Identifier { name } => {
                            self.emit(Instruction::StoreName { name: name });
                        }
                        _ => {
                            panic!("WTF");
                        }
                    }
                }
            }
            ast::Statement::Delete { targets } => {
                // Remove the given names from the scope
                // self.emit(Instruction::DeleteName);
            }
            ast::Statement::Pass => {
                self.emit(Instruction::Pass);
            }
        }
    }

    fn compile_expression(&mut self, expression: ast::Expression) {
        trace!("Compiling {:?}", expression);
        match expression {
            ast::Expression::Call { function, args } => {
                self.compile_expression(*function);
                let count = args.len();
                for arg in args {
                    self.compile_expression(arg)
                }
                self.emit(Instruction::CallFunction { count: count });
            }
            ast::Expression::Binop { a, op, b } => {
                self.compile_expression(*a);
                self.compile_expression(*b);

                // Perform operation:
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
                    ast::Operator::Subscript => bytecode::BinaryOperator::Subscript,
                };
                let i = Instruction::BinaryOperation { op: i };
                self.emit(i);
            }
            ast::Expression::Unop { op, a } => {
                self.compile_expression(*a);

                // Perform operation:
                let i = match op {
                    ast::UnaryOperator::Neg => bytecode::UnaryOperator::Minus,
                };
                let i = Instruction::UnaryOperation { op: i };
                self.emit(i);
            }
            ast::Expression::Attribute { value, name } => {
                self.compile_expression(*value);
                self.emit(Instruction::LoadAttr { name: name });
            }
            ast::Expression::Compare { a, op, b } => {
                self.compile_expression(*a);
                self.compile_expression(*b);

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
                self.emit(Instruction::LoadConst {
                    value: bytecode::Constant::Integer { value: value },
                });
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
            ast::Expression::Slice { elements } => {
                let size = elements.len();
                for element in elements {
                    self.compile_expression(element);
                }
                self.emit(Instruction::BuildSlice { size: size });
            }
            ast::Expression::True => {
                self.emit(Instruction::LoadConst {
                    value: bytecode::Constant::Integer { value: 1 },
                });
            }
            ast::Expression::False => {
                self.emit(Instruction::LoadConst {
                    value: bytecode::Constant::Integer { value: 0 },
                });
            }
            ast::Expression::None => {
                self.emit(Instruction::LoadConst {
                    value: bytecode::Constant::None,
                });
            }
            ast::Expression::String { value } => {
                self.emit(Instruction::LoadConst {
                    value: bytecode::Constant::String { value: value },
                });
            }
            ast::Expression::Identifier { name } => {
                self.emit(Instruction::LoadName { name });
            }
        }
    }

    // Low level helper functions:
    fn emit(&mut self, instruction: Instruction) {
        self.current_code_object().instructions.push(instruction);
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
}
