
/*
 * Implement virtual machine to run instructions.
 */

use std::rc::Rc;
use std::collections::HashMap;
use std::cell::RefMut;
use std::ops::Deref;

use super::bytecode;
use super::builtins;
use super::pyobject::{PyObject, PyObjectRef};

// use objects::objects;

// Container of the virtual machine state:
pub fn evaluate(code: bytecode::CodeObject) {
    let mut vm = VirtualMachine::new();

    // Register built in function:
    // vm.scope.insert(String::from("print"), PyObject::RustFunction { function: builtins::print }.into_ref());

    // { stack: Vec::new() };
    vm.run(Rc::new(code));
}

// Objects are live when they are on stack, or referenced by a name (for now)

#[derive(Clone)]
struct Block {
    block_type: String, //Enum?
    handler: usize // The destination we should jump to if the block finishes
    // level?
}

struct Frame {
    // TODO: We are using Option<i32> in stack for handline None return value
    code: Rc<bytecode::CodeObject>,
    // We need 1 stack per frame
    stack: Vec<PyObjectRef>,   // The main data frame of the stack machine
    blocks: Vec<Block>,  // Block frames, for controling loops and exceptions
    globals: HashMap<String, PyObjectRef>, // Variables
    locals: HashMap<String, PyObjectRef>, // Variables
    labels: HashMap<usize, usize>, // Maps label id to line number, just for speedup
    lasti: usize, // index of last instruction ran
    // return_value: NativeType,
    why: String, //Not sure why we need this //Maybe use a enum if we have fininte options
    // cmp_op: Vec<&'a Fn(NativeType, NativeType) -> bool>, // TODO: change compare to a function list
}

struct VirtualMachine {
    frames: Vec<Frame>,
}

impl Frame {
    pub fn new(code: Rc<bytecode::CodeObject>, callargs: HashMap<String, PyObjectRef>, globals: Option<HashMap<String, PyObjectRef>>) -> Frame {
        //populate the globals and locals
        let labels = HashMap::new();
        //TODO: This is wrong, check https://github.com/nedbat/byterun/blob/31e6c4a8212c35b5157919abff43a7daa0f377c6/byterun/pyvm2.py#L95
        let globals = match globals {
            Some(g) => g,
            None => HashMap::new(),
        };
        let mut locals = globals;
        locals.extend(callargs);

        //TODO: move this into the __builtin__ module when we have a module type
        locals.insert(String::from("print"), PyObject::RustFunction { function: builtins::print }.into_ref());
        // locals.insert("print".to_string(), Rc::new(NativeType::NativeFunction(builtins::print)));
        // locals.insert("len".to_string(), Rc::new(NativeType::NativeFunction(builtins::len)));
        Frame {
            code: code,
            stack: vec![],
            blocks: vec![],
            // save the callargs as locals
            globals: locals.clone(),
            locals: locals,
            labels: labels,
            lasti: 0,
            // return_value: NativeType::NoneType,
            why: "none".to_string(),
        }
    }

    fn pop_multiple(&mut self, count: usize) -> Vec<PyObjectRef> {
        let mut objs: Vec<PyObjectRef> = Vec::new();
        for _x in 0..count {
            objs.push(self.stack.pop().unwrap());
        }
        objs.reverse();
        objs
    }

}

impl VirtualMachine {
    pub fn new() -> VirtualMachine {
        VirtualMachine {
            frames: vec![],
        }
    }

    fn current_frame(&mut self) -> &mut Frame {
        self.frames.last_mut().unwrap()
    }

    fn pop_frame(&mut self) {
        self.frames.pop().unwrap();
    }

    fn run(&mut self, code: Rc<bytecode::CodeObject>) {
        let frame = Frame::new(code, HashMap::new(), None);
        self.run_frame(frame);
    }

    // The Option<i32> is the return value of the frame, remove when we have implemented frame
    // TODO: read the op codes directly from the internal code object
    fn run_frame(&mut self, mut frame: Frame) -> PyObjectRef {
        self.frames.push(frame);

        //let mut why = None;
        // Change this to a loop for jump
        loop {
            {
                let curr_frame = self.current_frame();
                if curr_frame.lasti >= curr_frame.code.instructions.len() {
                    break;
                }
            }

            //while curr_frame.lasti < curr_frame.code.co_code.len() {
            self.execute_instruction();
            /*if curr_frame.blocks.len() > 0 {
              self.manage_block_stack(&why);
              }
              */
            //if let Some(_) = why {
            //    break;
            //}
        }
        let return_value = {
            //let curr_frame = self.frames.last_mut().unwrap();
            // self.curr_frame().return_value.clone()
            // TODO
            PyObject::Integer { value: 1 }
        };
        self.pop_frame();
        return_value.into_ref().clone()
    }

    fn execute_binop(&mut self, frame: &mut Frame, op: &bytecode::BinaryOperator) {
    }

    // Execute a single instruction:
    fn execute_instruction(&mut self) {
        let frame = self.current_frame();

        // let current_frame = self.current_frame();
        let ins2 = ((*frame.code).instructions[frame.lasti]).clone();
        let instruction = &ins2;
        frame.lasti += 1;
        {
            debug!("stack:{:?}", frame.stack);
            debug!("env  :{:?}", frame.locals);
            debug!("Executing op code: {:?}", instruction);
        }
        match instruction {
            &bytecode::Instruction::LoadConst { ref value } => {
                let obj = match value {
                    &bytecode::Constant::Integer { ref value } => { PyObject::Integer { value: *value } },
                    // &bytecode::Constant::Float
                    &bytecode::Constant::String { ref value } => { PyObject::String { value: value.clone() } },
                };
                frame.stack.push(obj.into_ref().clone());
            },
            &bytecode::Instruction::LoadName { ref name } => {
                // Lookup name in scope and put it onto the stack!
                let obj = &frame.locals[name];
                frame.stack.push(obj.clone());
            },
            &bytecode::Instruction::StoreName { ref name } => {
                // take top of stack and assign in scope:
                let obj = frame.stack.pop().unwrap();
                frame.locals.insert(name.clone(), obj);
            },
            &bytecode::Instruction::Pop => {
                // Pop value from stack and ignore.
                frame.stack.pop();
            },
            &bytecode::Instruction::BuildList { size } => {
                let elements = frame.pop_multiple(size);
                let list_obj = PyObject::List { elements: elements }.into_ref();
                frame.stack.push(list_obj);
            },
            &bytecode::Instruction::BuildTuple { size } => {
                let elements = frame.pop_multiple(size);
                let list_obj = PyObject::Tuple { elements: elements }.into_ref();
                frame.stack.push(list_obj);
            },
            &bytecode::Instruction::BuildMap { size } => {
                let mut elements = Vec::new();
                for _x in 0..size {
                    let key = frame.stack.pop().unwrap();
                    let obj = frame.stack.pop().unwrap();
                    elements.push((key,obj));
                }
                panic!("To be implemented!")
                //let list_obj = PyObject::Tuple { elements: elements }.into_ref();
                //frame.stack.push(list_obj);
            },
            &bytecode::Instruction::BinaryOperation { ref op } => {
                let b_ref = frame.stack.pop().unwrap();
                let a_ref = frame.stack.pop().unwrap();
                let b = &*b_ref.borrow();
                let a = &*a_ref.borrow();
                let result = match op {
                    &bytecode::BinaryOperator::Subtract => a - b,
                    &bytecode::BinaryOperator::Add => a + b,
                    &bytecode::BinaryOperator::Multiply => a * b,
                    // &bytecode::BinaryOperator::Div => a / b,
                    _ => panic!("NOT IMPL"),
                };
                frame.stack.push(result.into_ref());
            },
            &bytecode::Instruction::UnaryOperation { ref op } => {
                panic!("TODO");
                // self.execute_binop(op);
            },
            /*
            &bytecode::Instruction::ReturnValue => {
                current_frame.stack.pop();
            },
            &bytecode::Instruction::PushBlock { start, end } => {
                current_frame.blocks.push((start, end));
            },
            &bytecode::Instruction::PopBlock => {
                current_frame.blocks.pop();
            }
            &bytecode::Instruction::GetIter => {
                let iterated_obj = current_frame.stack.pop().unwrap();
                let iter_obj = PyObject::Iterator {
                    position: 0, iterated_obj: iterated_obj
                }.into_ref();
                current_frame.stack.push(iter_obj);
            },
            */
            /*
            &bytecode::Instruction::ForIter => {
                // The top of stack contains the iterator, lets push it forward:
                let next_obj: Option<PyObjectRef> = {
                    let top_of_stack = current_frame.stack.last().unwrap();
                    let mut ref_mut: RefMut<PyObject> = top_of_stack.deref().borrow_mut();
                    // We require a mutable pyobject here to update the iterator:
                    let mut iterator = ref_mut; // &mut PyObject = ref_mut.;
                    // let () = iterator;
                    iterator.nxt()
                };

                // Check the next object:
                match next_obj {
                    Some(v) => {
                        current_frame.stack.push(v);
                    },
                    None => {
                        // End of for loop
                        let end_label = current_frame.blocks.last().unwrap().1;
                        self.jump(end_label);
                    }
                }
            },
            */
            &bytecode::Instruction::CallFunction { count } => {
                let mut args: Vec<PyObjectRef> = frame.pop_multiple(count);
                let func_ref = frame.stack.pop().unwrap();
                let f = func_ref.borrow();// = &*func_ref.borrow();
                f.call(args);
                // call_stack.push();
                // If a builtin function, then call directly, otherwise, execute it?
                // execute(function.code);
            },
            /* TODO
            &bytecode::Instruction::Jump { target } => {
                self.jump(target);
            }
            &bytecode::Instruction::JumpIf { target } => {
                let obj = self.stack.pop().unwrap();
                // TODO: determine if this value is True-ish:
                let result: bool = true;
                if result {
                    self.jump(target);
                }
            }
            &bytecode::Instruction::Break => {
                let end_label = frame.blocks.last().unwrap().1;
                self.jump(end_label);
            },
            */
            &bytecode::Instruction::Pass => {
                // Ah, this is nice, just relax!
            },
            /* TODO
            &bytecode::Instruction::Continue => {
                let start_label = frame.blocks.last().unwrap().0;
                self.jump(start_label);
            },
            */
            _ => panic!("NOT IMPL"),
        }
    }

    fn jump(&mut self, label: bytecode::Label) {
        // let current_frame = self.call_stack.last().unwrap();
        // self.program_counter = current_frame.label_map[&label];
    }
}
