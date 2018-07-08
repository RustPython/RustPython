/*
 * Take an AST and transform it into py_code_object compatiable bytecode
 * TODO: this file is obsoleted. It might be better to translate internal
 * bytecode into cpython compatible bytecode.
 *
 */
extern crate rustpython_parser;
extern crate py_code_object;
use rustpython_parser::ast;
use self::py_code_object::{PyCodeObject, NativeType};

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
                // panic!("What to emit?");
                let i = self.emit_name(name);
                self.emit("LOAD_NAME".to_string(), Some(i));
            }
            ast::Expression::Number { value } => {
                // panic!("What to emit?");
                let i = self.emit_const(value);
                self.emit("LOAD_CONST".to_string(), Some(i));
            }
            _ => {
                panic!("Not impl {:?}", expression);
            }
        }
    }

    fn emit_name(&mut self, name: String) -> usize {
        self.code_object.co_names.push(name);
        // TODO: is this index in vector?
        0
    }

    fn emit_const(&mut self, value: i32) -> usize {
        self.code_object.co_consts.push(NativeType::Int ( value ));
        0
    }

    fn emit(&mut self, instruction: String, arg: Option<usize>) {
        self.code_object.co_code.push((0, instruction, arg));
    }
}
