use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

use super::bytecode;
use super::pyobject::{Executor, PyContext, PyObject, PyObjectKind, PyObjectRef, PyResult};

#[derive(Clone, Debug)]
pub enum Block {
    Loop {
        start: bytecode::Label,
        end: bytecode::Label,
    },
    TryExcept,
}

pub struct Frame {
    // TODO: We are using Option<i32> in stack for handline None return value
    pub code: Rc<bytecode::CodeObject>,
    // We need 1 stack per frame
    stack: Vec<PyObjectRef>, // The main data frame of the stack machine
    blocks: Vec<Block>,      // Block frames, for controling loops and exceptions
    // pub globals: HashMap<String, PyObjectRef>, // Variables
    pub locals: PyObjectRef, // Variables
    pub lasti: usize,        // index of last instruction ran
                             // cmp_op: Vec<&'a Fn(NativeType, NativeType) -> bool>, // TODO: change compare to a function list
}

impl Frame {
    pub fn new(
        code: Rc<bytecode::CodeObject>,
        callargs: HashMap<String, PyObjectRef>,
        globals: PyObjectRef,
    ) -> Frame {
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
            code: code,
            stack: vec![],
            blocks: vec![],
            // save the callargs as locals
            // globals: locals.clone(),
            locals: locals,
            lasti: 0,
        }
    }

    pub fn push_block(&mut self, block: Block) {
        self.blocks.push(block);
    }

    pub fn pop_block(&mut self) -> Block {
        self.blocks.pop().unwrap()
    }

    pub fn last_block(&mut self) -> &Block {
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

    pub fn last_value(&mut self) -> PyObjectRef {
        self.stack.last().unwrap().clone()
    }
}

impl fmt::Debug for Frame {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let stack_str = self.stack
            .iter()
            .map(|elem| format!("\n  > {}", elem.borrow_mut().str()))
            .collect::<Vec<_>>()
            .join("");
        let block_str = self.blocks
            .iter()
            .map(|elem| format!("\n  > {:?}", elem))
            .collect::<Vec<_>>()
            .join("");
        let local_str = "".to_string(); /* self.locals
            .iter()
            .map(|elem| format!("\n  {} = {}", elem.0, elem.1.borrow_mut().str()))
            .collect::<Vec<_>>()
            .join(""); */
        write!(
            f,
            "Frame Object {{ \n Stack:{}\n Blocks:{}\n Locals:{}\n}}",
            stack_str, block_str, local_str
        )
    }
}
