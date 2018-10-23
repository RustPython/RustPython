extern crate rustpython_parser;

use self::rustpython_parser::ast;
use std::fmt;

use super::bytecode;
use super::pyobject::{PyObjectKind, PyObjectRef};

#[derive(Clone, Debug)]
pub enum Block {
    Loop {
        start: bytecode::Label,
        end: bytecode::Label,
    },
    TryExcept {
        handler: bytecode::Label,
    },
    With {
        end: bytecode::Label,
        context_manager: PyObjectRef,
    },
}

pub struct Frame {
    // TODO: We are using Option<i32> in stack for handline None return value
    pub code: bytecode::CodeObject,
    // We need 1 stack per frame
    stack: Vec<PyObjectRef>, // The main data frame of the stack machine
    blocks: Vec<Block>,      // Block frames, for controling loops and exceptions
    pub locals: PyObjectRef, // Variables
    pub lasti: usize,        // index of last instruction ran
                             // cmp_op: Vec<&'a Fn(NativeType, NativeType) -> bool>, // TODO: change compare to a function list
}

pub fn copy_code(code_obj: PyObjectRef) -> bytecode::CodeObject {
    let code_obj = code_obj.borrow();
    if let PyObjectKind::Code { ref code } = code_obj.kind {
        code.clone()
    } else {
        panic!("Must be code obj");
    }
}

impl Frame {
    pub fn new(code: PyObjectRef, globals: PyObjectRef) -> Frame {
        //populate the globals and locals
        //TODO: This is wrong, check https://github.com/nedbat/byterun/blob/31e6c4a8212c35b5157919abff43a7daa0f377c6/byterun/pyvm2.py#L95
        /*
        let globals = match globals {
            Some(g) => g,
            None => HashMap::new(),
        };
        */
        let locals = globals;
        // locals.extend(callargs);

        Frame {
            code: copy_code(code),
            stack: vec![],
            blocks: vec![],
            // save the callargs as locals
            // globals: locals.clone(),
            locals: locals,
            lasti: 0,
        }
    }

    pub fn fetch_instruction(&mut self) -> bytecode::Instruction {
        // TODO: an immutable reference is enough, we should not
        // clone the instruction.
        let ins2 = self.code.instructions[self.lasti].clone();
        self.lasti += 1;
        ins2
    }

    pub fn get_lineno(&self) -> ast::Location {
        self.code.locations[self.lasti].clone()
    }

    pub fn push_block(&mut self, block: Block) {
        self.blocks.push(block);
    }

    pub fn pop_block(&mut self) -> Option<Block> {
        self.blocks.pop()
    }

    pub fn last_block(&self) -> &Block {
        self.blocks.last().unwrap()
    }

    pub fn push_value(&mut self, obj: PyObjectRef) {
        self.stack.push(obj);
    }

    pub fn pop_value(&mut self) -> PyObjectRef {
        self.stack.pop().unwrap()
    }

    pub fn pop_multiple(&mut self, count: usize) -> Vec<PyObjectRef> {
        let mut objs: Vec<PyObjectRef> = Vec::new();
        for _x in 0..count {
            objs.push(self.stack.pop().unwrap());
        }
        objs.reverse();
        objs
    }

    pub fn last_value(&self) -> PyObjectRef {
        self.stack.last().unwrap().clone()
    }

    pub fn nth_value(&self, depth: usize) -> PyObjectRef {
        self.stack[self.stack.len() - depth - 1].clone()
    }
}

impl fmt::Debug for Frame {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let stack_str = self
            .stack
            .iter()
            .map(|elem| format!("\n  > {}", elem.borrow().str()))
            .collect::<Vec<_>>()
            .join("");
        let block_str = self
            .blocks
            .iter()
            .map(|elem| format!("\n  > {:?}", elem))
            .collect::<Vec<_>>()
            .join("");
        let local_str = match self.locals.borrow().kind {
            PyObjectKind::Scope { ref scope } => match scope.locals.borrow().kind {
                PyObjectKind::Dict { ref elements } => elements
                    .iter()
                    .map(|elem| format!("\n  {} = {}", elem.0, elem.1.borrow().str()))
                    .collect::<Vec<_>>()
                    .join(""),
                ref unexpected => panic!(
                    "locals unexpectedly not wrapping a dict! instead: {:?}",
                    unexpected
                ),
            },
            ref unexpected => panic!("locals unexpectedly not a scope! instead: {:?}", unexpected),
        };
        write!(
            f,
            "Frame Object {{ \n Stack:{}\n Blocks:{}\n Locals:{}\n}}",
            stack_str, block_str, local_str
        )
    }
}
