/*
 * Take an AST and transform it into py_code_object compatiable bytecode
 */
extern crate rustpython_parser;
extern crate py_code_object;
use rustpython_parser::compiler::ast;
use self::py_code_object::{PyCodeObject};

struct Compiler {
    code_object: PyCodeObject,
    nxt_label: usize,
}

pub fn compile(p: ast::Program) -> PyCodeObject {
    let mut compiler = Compiler::new();
    compiler.compile_program(p);
    compiler.code_object
}

type Label = usize;

impl Compiler {
    fn new() -> Self {
        Compiler {
            code_object: PyCodeObject::new(),
            nxt_label: 0,
        }
    }

    fn compile_program(&mut self, program: ast::Program) {
        self.compile_statements(program.statements);
        return;
    }

    fn compile_statements(&mut self, statements: Vec<ast::Statement>) {
        for statement in statements {
            self.compile_statement(statement)
        }
    }

    fn compile_statement(&mut self, statement: ast::Statement) {
        trace!("Compiling {:?}", statement);
        match statement {
            ast::Statement::Expression { expression } => {
                self.compile_expression(expression);

                // Pop result of stack, since we not use it:
                // self.emit(Instruction::Pop);
            }
            _ => {
                panic!("Not impl");
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
                // TODO: what to emit?
                // self.emit(Instruction::CallFunction { count: count });
            }
            ast::Expression::Identifier { name } => {
                panic!("What to emit?");
                // self.emit(Instruction::LoadName { name });
            }
            _ => {
                panic!("Not impl {:?}", expression);
            }
        }
    }
}
