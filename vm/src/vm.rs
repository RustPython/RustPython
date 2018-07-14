/*
 * Implement virtual machine to run instructions.
 * See also:
 *   https://github.com/ProgVal/pythonvm-rust/blob/master/src/processor/mod.rs
 */

extern crate rustpython_parser;

use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
// use std::cell::RefMut;
// use std::ops::Deref;

use self::rustpython_parser::parser;
use super::builtins;
use super::bytecode;
use super::compile;
use super::pyobject::{PyObject, PyObjectKind, PyObjectRef, PyResult, PyContext, Executor};
use super::objstr;

// use objects::objects;

// Objects are live when they are on stack, or referenced by a name (for now)

#[derive(Clone)]
struct Block {
    block_type: String, //Enum?
    handler: usize,     // The destination we should jump to if the block finishes
                        // level?
}

struct Frame {
    // TODO: We are using Option<i32> in stack for handline None return value
    code: Rc<bytecode::CodeObject>,
    // We need 1 stack per frame
    stack: Vec<PyObjectRef>, // The main data frame of the stack machine
    blocks: Vec<Block>,      // Block frames, for controling loops and exceptions
    globals: HashMap<String, PyObjectRef>, // Variables
    locals: HashMap<String, PyObjectRef>, // Variables
    lasti: usize,            // index of last instruction ran
                             // cmp_op: Vec<&'a Fn(NativeType, NativeType) -> bool>, // TODO: change compare to a function list
}

pub struct VirtualMachine {
    frames: Vec<Frame>,
    builtins: PyObjectRef,
    ctx: PyContext,
}

impl Frame {
    pub fn new(
        code: Rc<bytecode::CodeObject>,
        callargs: HashMap<String, PyObjectRef>,
        globals: Option<HashMap<String, PyObjectRef>>,
    ) -> Frame {
        //populate the globals and locals
        //TODO: This is wrong, check https://github.com/nedbat/byterun/blob/31e6c4a8212c35b5157919abff43a7daa0f377c6/byterun/pyvm2.py#L95
        let globals = match globals {
            Some(g) => g,
            None => HashMap::new(),
        };
        let mut locals = globals;
        locals.extend(callargs);

        // locals.insert("len".to_string(), Rc::new(NativeType::NativeFunction(builtins::len)));
        Frame {
            code: code,
            stack: vec![],
            blocks: vec![],
            // save the callargs as locals
            globals: locals.clone(),
            locals: locals,
            lasti: 0,
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

impl Executor for VirtualMachine {
    fn call(&mut self, f: PyObjectRef) -> PyResult {
        self.invoke(f, Vec::new())
    }

    fn new_str(&self, s: String) -> PyObjectRef {
        self.ctx.new_str(s)
    }

    fn new_bool(&self, b: bool) -> PyObjectRef {
        self.ctx.new_bool(b)
    }

    fn get_none(&self) -> PyObjectRef {
        // TODO
        self.ctx.new_bool(false)
    }

    fn get_type(&self) -> PyObjectRef {
        self.ctx.type_type.clone()
    }

    fn context(&self) -> &PyContext {
        &self.ctx
    }
}

impl VirtualMachine {
    pub fn new() -> VirtualMachine {
        let ctx = PyContext::new();
        let builtins = builtins::make_module(&ctx);
        VirtualMachine {
            frames: vec![],
            builtins: builtins,
            ctx: ctx,
        }
    }

    // Container of the virtual machine state:
    pub fn evaluate(&mut self, code: bytecode::CodeObject) -> PyResult {
        // Register built in function:
        // vm.scope.insert(String::from("print"), PyObject::RustFunction { function: builtins::print }.into_ref());

        // { stack: Vec::new() };
        self.run(Rc::new(code))
    }

    pub fn to_str(&mut self, obj: PyObjectRef) -> String {
        obj.borrow().str()
    }

    fn current_frame(&mut self) -> &mut Frame {
        self.frames.last_mut().unwrap()
    }

    fn pop_frame(&mut self) -> Frame {
        self.frames.pop().unwrap()
    }

    fn push_value(&mut self, obj: PyObjectRef) {
        self.current_frame().stack.push(obj);
    }

    fn pop_value(&mut self) -> PyObjectRef {
        self.current_frame().stack.pop().unwrap()
    }

    fn pop_multiple(&mut self, count: usize) -> Vec<PyObjectRef> {
        self.current_frame().pop_multiple(count)
    }

    fn store_name(&mut self, name: String) -> Option<PyResult> {
        let obj = self.pop_value();
        self.current_frame().locals.insert(name, obj);
        None
    }

    fn load_name(&mut self, name: &String) -> Option<PyResult> {
        // Lookup name in scope and put it onto the stack!
        if self.current_frame().locals.contains_key(name) {
            let obj = self.current_frame().locals[name].clone();
            self.push_value(obj);
            None
        } else if self.builtins.borrow().dict.contains_key(name) {
            let obj = self.builtins.borrow().dict[name].clone();
            self.push_value(obj);
            None
        } else {
            let name_error = PyObject::new(PyObjectKind::NameError {
                name: name.to_string(),
            }, self.get_type());
            Some(Err(name_error))
        }
    }

    fn run(&mut self, code: Rc<bytecode::CodeObject>) -> PyResult {
        let frame = Frame::new(code, HashMap::new(), None);
        self.run_frame(frame).0
    }

    fn run_frame(&mut self, frame: Frame) -> (PyResult, Frame) {
        self.frames.push(frame);

        // Execute until return or exception:
        let value = loop {
            let result = self.execute_instruction();
            match result {
                None => {}
                Some(Ok(value)) => {
                    break Ok(value);
                }
                Some(Err(value)) => {
                    // TODO: unwind stack on exception and find any handlers.
                    break Err(value);
                }
            }
        };

        let frame2 = self.pop_frame();
        (value, frame2)
    }
    /*
    fn run_code(&mut self, code: i32) -> Result<PyCodeObject, PyCodeObject> {
    }*/

    fn subscript(&mut self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        // debug!("tos: {:?}, tos1: {:?}", tos, tos1);
        // Subscript implementation: a[b]
        let a2 = &*a.borrow();
        match &a2.kind {
            /*
            (&NativeType::Tuple(ref t), &NativeType::Int(ref index)) => curr_frame.stack.push(Rc::new(t[*index as usize].clone())),
            */
            PyObjectKind::String { ref value } => objstr::subscript(self, value, b.clone()),
            // TODO: implement other Slice possibilities
            _ => panic!(
                "TypeError: indexing type {:?} with index {:?} is not supported (yet?)",
                a, b
            ),
        }
    }

    fn _sub(&mut self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        let b2 = &*b.borrow();
        let a2 = &*a.borrow();
        Ok(PyObject::new(a2 - b2, self.get_type()))
    }

    fn _add(&mut self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        let b2 = &*b.borrow();
        let a2 = &*a.borrow();
        Ok(PyObject::new(a2 + b2, self.get_type()))
    }

    fn _mul(&mut self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        let b2 = &*b.borrow();
        let a2 = &*a.borrow();
        Ok(PyObject::new(a2 * b2, self.get_type()))
    }

    fn execute_binop(&mut self, op: &bytecode::BinaryOperator) -> Option<PyResult> {
        let b_ref = self.pop_value();
        let a_ref = self.pop_value();
        // TODO: if the left hand side provides __add__, invoke that function.
        //
        let result = match op {
            &bytecode::BinaryOperator::Subtract => self._sub(a_ref, b_ref),
            &bytecode::BinaryOperator::Add => self._add(a_ref, b_ref),
            &bytecode::BinaryOperator::Multiply => self._mul(a_ref, b_ref),
            // &bytecode::BinaryOperator::Div => a / b,
            &bytecode::BinaryOperator::Subscript => self.subscript(a_ref, b_ref),
            _ => panic!("NOT IMPL {:?}", op),
        };
        match result {
            Ok(value) => {
                self.push_value(value);
                None
            }
            Err(value) => Some(Err(value)),
        }
    }

    fn execute_unop(&mut self, op: &bytecode::UnaryOperator) -> Option<PyResult> {
        let a_ref = self.pop_value();
        let a = &*a_ref.borrow();
        let result = match op {
            &bytecode::UnaryOperator::Minus => {
                // TODO:
                // self.invoke('__neg__'
                match a.kind {
                    PyObjectKind::Integer { value: ref value1 } => {
                        Ok(self.ctx.new_int(-*value1))
                    }
                    _ => panic!("Not impl {:?}", a),
                }
            }
            _ => panic!("Not impl {:?}", op),
        };
        match result {
            Ok(value) => {
                self.push_value(value);
                None
            },
            Err(value) => Some(Err(value)),
        }
    }

    fn _eq(&mut self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        let b2 = &*b.borrow();
        let a2 = &*a.borrow();
        let result_bool = a2 == b2;
        let result = self.ctx.new_bool(result_bool);
        Ok(result)
    }

    fn _ne(&mut self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        let b2 = &*b.borrow();
        let a2 = &*a.borrow();
        let result_bool = a2 != b2;
        let result = self.ctx.new_bool(result_bool);
        Ok(result)
    }

    fn _is(&mut self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        // Pointer equal:
        let result_bool = true; // TODO: *a.as_ptr() == *b.as_ptr();
        let result = self.ctx.new_bool(result_bool);
        Ok(result)
    }

    fn _is_not(&mut self, a: PyObjectRef, b: PyObjectRef) -> PyResult {
        // Pointer equal:
        let result_bool = true; // TODO: *a.as_ptr() != *b.as_ptr();
        let result = self.ctx.new_bool(result_bool);
        Ok(result)
    }

    fn execute_compare(&mut self, op: &bytecode::ComparisonOperator) -> Option<PyResult> {
        let b = self.pop_value();
        let a = self.pop_value();
        let result = match op {
            &bytecode::ComparisonOperator::Equal => self._eq(a, b),
            &bytecode::ComparisonOperator::NotEqual => self._ne(a, b),
            &bytecode::ComparisonOperator::Is => self._is(a, b),
            &bytecode::ComparisonOperator::IsNot => self._is_not(a, b),
            _ => panic!("NOT IMPL {:?}", op),
        };
        match result {
            Ok(value) => {
                self.push_value(value);
                None
            },
            Err(value) => Some(Err(value)),
        }
    }

    fn new_instance(&mut self, type_ref: PyObjectRef, args: Vec<PyObjectRef>) -> PyResult {
        // more or less __new__ operator
        let obj = PyObject::new(PyObjectKind::None, type_ref.clone());
        Ok(obj)
    }

    fn invoke(&mut self, func_ref: PyObjectRef, args: Vec<PyObjectRef>) -> PyResult {
        let f = func_ref.borrow();

        match f.kind {
            PyObjectKind::RustFunction { ref function } => f.call(self, args),
            PyObjectKind::Function { ref code } => {
                let frame = Frame::new(Rc::new(code.clone()), HashMap::new(), None);
                self.run_frame(frame).0
            },
            PyObjectKind::Class { name: _ } => {
                self.new_instance(func_ref.clone(), args)
            },
            _ => {
                println!("Not impl {:?}", f);
                panic!("Not impl");
            }
        }
    }

    fn import(&mut self, name: String) -> Option<PyObjectRef> {
        // Time to search for module in any place:
        let filename = format!("{}.py", name);
        let filepath = Path::new(&filename);

        let source = match parser::read_file(filepath) {
            Err(value) => panic!("Error: {}", value),
            Ok(value) => value,
        };

        match compile::compile(&source, compile::Mode::Exec) {
            Ok(bytecode) => {
                debug!("Code object: {:?}", bytecode);
                let obj = PyObject::new(PyObjectKind::Module { name: name.clone() }, self.get_type());

                // As a sort of hack, create a frame and run code in it
                let frame = Frame::new(Rc::new(bytecode), HashMap::new(), None);
                let frame2 = self.run_frame(frame).1;

                // TODO: we might find a better solution than this:
                for (name, member_obj) in frame2.locals.iter() {
                    obj.borrow_mut()
                        .dict
                        .insert(name.to_string(), member_obj.clone());
                }

                // Push module on stack:
                self.push_value(obj);
                None
            }
            Err(value) => {
                panic!("Error: {}", value);
            }
        }
    }

    fn load_attr(&mut self, name: String) -> Option<PyResult> {
        let parent = self.pop_value();
        // Lookup name in obj
        let obj = parent.borrow().dict[&name].clone();
        self.push_value(obj);
        None
    }

    fn fetch_instruction(&mut self) -> bytecode::Instruction {
        let frame = self.current_frame();
        // TODO: an immutable reference is enough, we should not
        // clone the instruction.
        let ins2 = ((*frame.code).instructions[frame.lasti]).clone();
        frame.lasti += 1;
        ins2
    }

    // Execute a single instruction:
    fn execute_instruction(&mut self) -> Option<PyResult> {
        let instruction = self.fetch_instruction();
        {
            trace!("=======");
            trace!(
                "  stack:{:?}",
                self.current_frame().stack
                    .iter()
                    .map(|elem| elem.borrow_mut().str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            trace!("  env  :{:?}", self.current_frame().locals);
            trace!("  Executing op code: {:?}", instruction);
        }
        match &instruction {
            &bytecode::Instruction::LoadConst { ref value } => {
                let obj = match value {
                    &bytecode::Constant::Integer { ref value } => self.ctx.new_int(*value),
                    // &bytecode::Constant::Float
                    &bytecode::Constant::String { ref value } => {
                        self.new_str(value.clone())
                    }
                    &bytecode::Constant::Code { ref code } => {
                        PyObject::new(PyObjectKind::Code { code: code.clone() }, self.get_type())
                    }
                    &bytecode::Constant::None => PyObject::new(PyObjectKind::None, self.get_type()),
                };
                self.push_value(obj);
                None
            }
            &bytecode::Instruction::Import { ref name } => {
                self.import(name.to_string());
                None
            }
            &bytecode::Instruction::LoadName { ref name } => {
                self.load_name(name)
            }
            &bytecode::Instruction::StoreName { ref name } => {
                // take top of stack and assign in scope:
                self.store_name(name.clone())
            }
            &bytecode::Instruction::Pop => {
                // Pop value from stack and ignore.
                self.pop_value();
                None
            }
            &bytecode::Instruction::BuildList { size } => {
                let elements = self.pop_multiple(size);
                let list_obj = PyObject::new(PyObjectKind::List { elements: elements }, self.get_type());
                self.push_value(list_obj);
                None
            }
            &bytecode::Instruction::BuildTuple { size } => {
                let elements = self.pop_multiple(size);
                let list_obj = PyObject::new(PyObjectKind::Tuple { elements: elements }, self.get_type());
                self.push_value(list_obj);
                None
            }
            &bytecode::Instruction::BuildMap { size } => {
                let mut elements = Vec::new();
                for _x in 0..size {
                    let key = self.pop_value();
                    let obj = self.pop_value();
                    elements.push((key, obj));
                }
                panic!("To be implemented!")
                //let list_obj = PyObject::Tuple { elements: elements }.into_ref();
                //frame.stack.push(list_obj);
            }
            &bytecode::Instruction::BuildSlice { size } => {
                assert!(size == 2 || size == 3);
                let elements = self.pop_multiple(size);

                let mut out: Vec<Option<i32>> = elements
                    .into_iter()
                    .map(|x| match x.borrow().kind {
                        PyObjectKind::Integer { value } => Some(value),
                        PyObjectKind::None => None,
                        _ => panic!("Expect Int or None as BUILD_SLICE arguments, got {:?}", x),
                    })
                    .collect();

                let start = out[0];
                let stop = out[1];
                let step = if out.len() == 3 { out[2] } else { None };

                let obj = PyObject::new(PyObjectKind::Slice { start, stop, step }, self.ctx.type_type.clone());
                self.push_value(obj);
                None
            }
            &bytecode::Instruction::BinaryOperation { ref op } => {
                self.execute_binop(op)
            }
            &bytecode::Instruction::LoadAttr { ref name } => {
                self.load_attr(name.to_string())
            }
            &bytecode::Instruction::UnaryOperation { ref op } => {
                self.execute_unop(op)
            }
            &bytecode::Instruction::CompareOperation { ref op } => {
                self.execute_compare(op)
            }
            &bytecode::Instruction::ReturnValue => {
                let value = self.pop_value();
                Some(Ok(value))
            }
            /*
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
            &bytecode::Instruction::MakeFunction => {
                let qualified_name = self.pop_value();
                let code = self.pop_value();
                let code_2 = &*code.borrow();
                let code_obj = match code_2.kind {
                    PyObjectKind::Code { ref code } => code.clone(),
                    _ => panic!("Second item on the stack should be a code object"),
                };
                // pop argc arguments
                // argument: name, args, globals
                let obj = PyObject::new(PyObjectKind::Function { code: code_obj }, self.get_type());
                self.push_value(obj);
                None
            }
            &bytecode::Instruction::CallFunction { count } => {
                let args: Vec<PyObjectRef> = self.pop_multiple(count);
                let func_ref = self.pop_value();

                // Call function:
                let func_result = self.invoke(func_ref, args);

                match func_result {
                    Ok(value) => {
                        self.push_value(value);
                        None
                    }
                    Err(value) => {
                        // Ripple exception upwards:
                        Some(Err(value))
                    }
                }
            }
            /* TODO
            &bytecode::Instruction::Jump { target } => {
                self.jump(target);
            }
            */
            &bytecode::Instruction::JumpIf { target } => {
                let obj = self.pop_value();
                // TODO: determine if this value is True-ish:
                //if *v == NativeType::Boolean(true) {
                //    curr_frame.lasti = curr_frame.labels.get(target).unwrap().clone();
                //}
                let x = obj.borrow();
                let result: bool = match x.kind {
                    PyObjectKind::Boolean { ref value } => *value,
                    _ => {
                        panic!("Not impl {:?}", x);
                    }
                };
                if result {
                    self.jump(target);
                }
                None
            }

            &bytecode::Instruction::Raise { argc } => {
                let exception = match argc {
                    1 => self.pop_value(),
                    0 | 2 | 3 => panic!("Not implemented!"),
                    _ => panic!("Invalid paramter for RAISE_VARARGS, must be between 0 to 3"),
                };
                error!("{:?}", exception);
                Some(Err(exception))
            }

            /* TODO
            &bytecode::Instruction::Break => {
                let end_label = frame.blocks.last().unwrap().1;
                self.jump(end_label);
            },
            */
            &bytecode::Instruction::Pass => {
                // Ah, this is nice, just relax!
                None
            }
            /* TODO
            &bytecode::Instruction::Continue => {
                let start_label = frame.blocks.last().unwrap().0;
                self.jump(start_label);
            },
            */
            _ => panic!("NOT IMPL {:?}", instruction),
        }
    }

    fn jump(&mut self, label: bytecode::Label) {
        let current_frame = self.current_frame();
        let target_pc = current_frame.code.label_map[&label];
        trace!(
            "program counter from {:?} to {:?}",
            current_frame.lasti,
            target_pc
        );
        current_frame.lasti = target_pc;
    }
}
