/*
 * Take an AST and transform it into py_code_object compatiable bytecode
 */
extern crate py_code_object;

use super::ast;
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
        return;
    }
}
