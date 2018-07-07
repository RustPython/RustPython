
// A function which takes CPython bytecode (from json) and transforms
// this into RustPython bytecode. This to decouple RustPython from CPython
// internal bytecode representations.

use rustpython_vm::bytecode;
use py_code_object::{PyCodeObject, NativeType};

pub fn convert(cpython_bytecode: PyCodeObject) -> bytecode::CodeObject {
    let mut c = Converter::new();
    c.convert(cpython_bytecode);
    c.code2
}


// TODO: think of an appropriate name for this thing:
pub struct Converter {
    code: Option<PyCodeObject>,
    code2: bytecode::CodeObject,
}

impl Converter {
    pub fn new() -> Converter {
        Converter {
            code: None,
            code2: bytecode::CodeObject::new(),
        }
    }

    pub fn convert(&mut self, code: PyCodeObject) {
        self.code = Some(code);
        for op_code in self.code.unwrap().co_code {
           self.dispatch(op_code);
        }
    }

    fn dispatch(&mut self, op_code: (usize, String, Option<usize>)) {
        debug!("Converting op code: {:?}", op_code);
        match (op_code.1.as_ref(), op_code.2) {
            ("LOAD_CONST", Some(consti)) => {
                // println!("Loading const at index: {}", consti);
                let value = match self.code.unwrap().co_consts[consti] {
                    // NativeType::Boolean { value } => { bytecode::Constant::Boolean { true } },
                    // NativeType::Int { value } => { bytecode::Constant::Integer { value: value } },
                    _ => { panic!("Not impl "); }
                };
                self.emit(bytecode::Instruction::LoadConst { value: value });
            },

            // TODO: universal stack element type
            ("LOAD_CONST", None) => {
                self.emit(bytecode::Instruction::LoadConst { value: bytecode::Constant::None });
            },
            ("POP_TOP", None) => {
                self.emit(bytecode::Instruction::Pop);
            },
            ("STORE_NAME", Some(namei)) => {
                // println!("Loading const at index: {}", consti);
                let name = self.code.unwrap().co_names[namei].clone();
                self.emit(bytecode::Instruction::StoreName { name });
            },
            ("LOAD_NAME", Some(namei)) => {
                // println!("Loading const at index: {}", consti);
                let name = self.code.unwrap().co_names[namei].clone();
                self.emit(bytecode::Instruction::LoadName { name });
            },
            ("LOAD_GLOBAL", Some(namei)) => {
                // We need to load the underlying value the name points to, but stuff like
                // AssertionError is in the names right after compile, so we load the string
                // instead for now
                // let curr_frame = self.curr_frame();
                // let name = &curr_frame.code.co_names[namei];
            },

            ("BUILD_LIST", Some(count)) => {
                self.emit(bytecode::Instruction::BuildList { size: count });
            },

/*
            ("BUILD_SLICE", Some(count)) => {
                let curr_frame = self.curr_frame();
                assert!(count == 2 || count == 3);
            },
            */

            ("GET_ITER", None) => {
                self.emit(bytecode::Instruction::GetIter);
            },

            ("FOR_ITER", Some(delta)) => {
                self.emit(bytecode::Instruction::ForIter);
            },

/*
            ("COMPARE_OP", Some(cmp_op_i)) => {
                let curr_frame = self.curr_frame();
            },
            ("POP_JUMP_IF_TRUE", Some(ref target)) => {
                let curr_frame = self.curr_frame();
                let v = curr_frame.stack.pop().unwrap();
                if *v == NativeType::Boolean(true) {
                    curr_frame.lasti = curr_frame.labels.get(target).unwrap().clone();
                }
                None

            }
            ("POP_JUMP_IF_FALSE", Some(ref target)) => {
                let curr_frame = self.curr_frame();
                let v = curr_frame.stack.pop().unwrap();
                if *v == NativeType::Boolean(false) {
                    curr_frame.lasti = curr_frame.labels.get(target).unwrap().clone();
                }
                None
                
            }
            ("JUMP_FORWARD", Some(ref delta)) => {
                let curr_frame = self.curr_frame();
                let last_offset = curr_frame.get_bytecode_offset().unwrap();
                curr_frame.lasti = curr_frame.labels.get(&(last_offset + delta)).unwrap().clone();
                None
            },
            ("JUMP_ABSOLUTE", Some(ref target)) => {
                let curr_frame = self.curr_frame();
                curr_frame.lasti = curr_frame.labels.get(target).unwrap().clone();
                None
            },
            ("BREAK_LOOP", None) => {
                // Do we still need to return the why if we use unwind from jsapy?
                self.unwind("break".to_string());
                None //?
            },
            ("RAISE_VARARGS", Some(argc)) => {
                let curr_frame = self.curr_frame();
                // let (exception, params, traceback) = match argc {
                let exception = match argc {
                    1 => curr_frame.stack.pop().unwrap(),
                    0 | 2 | 3 => panic!("Not implemented!"),
                    _ => panic!("Invalid paramter for RAISE_VARARGS, must be between 0 to 3")
                };
                panic!("{:?}", exception);
            }
            ("INPLACE_ADD", None) => {
                self.emit(bytecode::Instruction::BinaryOperation { op: BinaryOperator::Add });
            },
            
            ("STORE_SUBSCR", None) => {
                let curr_frame = self.curr_frame();
                let tos = curr_frame.stack.pop().unwrap();
                let tos1 = curr_frame.stack.pop().unwrap();
                let tos2 = curr_frame.stack.pop().unwrap();
                match (tos1.deref(), tos.deref()) {
                    (&NativeType::List(ref refl), &NativeType::Int(index)) => {
                        refl.borrow_mut()[index as usize] = (*tos2).clone();
                    },
                    (&NativeType::Str(_), &NativeType::Int(_)) => {
                        // TODO: raise TypeError: 'str' object does not support item assignment
                        panic!("TypeError: 'str' object does not support item assignment")
                    },
                    _ => panic!("TypeError in STORE_SUBSCR")
                }
                curr_frame.stack.push(tos1);
            },
*/
            ("BINARY_ADD", None) => {
                self.emit(bytecode::Instruction::BinaryOperation { op: bytecode::BinaryOperator::Add });
            },
            ("BINARY_POWER", None) => {
                self.emit(bytecode::Instruction::BinaryOperation { op: bytecode::BinaryOperator::Power });
            },
            ("BINARY_MULTIPLY", None) => {
                self.emit(bytecode::Instruction::BinaryOperation { op: bytecode::BinaryOperator::Multiply });
            },
            ("BINARY_TRUE_DIVIDE", None) => {
                self.emit(bytecode::Instruction::BinaryOperation { op: bytecode::BinaryOperator::Divide });
            },
            ("BINARY_MODULO", None) => {
                self.emit(bytecode::Instruction::BinaryOperation { op: bytecode::BinaryOperator::Modulo });
            },
            ("BINARY_SUBTRACT", None) => {
                self.emit(bytecode::Instruction::BinaryOperation { op: bytecode::BinaryOperator::Subtract });
            },

/*
            ("ROT_TWO", None) => {
                // TODO: self.emit(Instruction::BinaryOperation { op: bytecode::BinaryOperator::RotTwo });
            }
            */
            ("UNARY_NEGATIVE", None) => {
                self.emit(bytecode::Instruction::UnaryOperation { op: bytecode::UnaryOperator::Minus });
            },
            ("UNARY_POSITIVE", None) => {
                self.emit(bytecode::Instruction::UnaryOperation { op: bytecode::UnaryOperator::Plus });
            },
            /*
            ("PRINT_ITEM", None) => {
                // TODO: Print without the (...)
                println!("{:?}", curr_frame.stack.pop().unwrap());
            },
            ("PRINT_NEWLINE", None) => {
                print!("\n");
            },*/
            /*
            ("MAKE_FUNCTION", Some(argc)) => {
                // https://docs.python.org/3.4/library/dis.html#opcode-MAKE_FUNCTION
                self.emit(bytecode::Instruction::MakeFunction { });
            },
            */
            ("CALL_FUNCTION", Some(argc)) => {
                let kw_count = (argc >> 8) as u8;
                let pos_count = (argc & 0xFF) as usize;
                self.emit(bytecode::Instruction::CallFunction { count: pos_count });
            },
            /*
            ("RETURN_VALUE", None) => {
                // Hmmm... what is this used?
                // I believe we need to push this to the next frame
                self.curr_frame().return_value = (*self.curr_frame().stack.pop().unwrap()).clone();
                Some("return".to_string())
                self.emit(bytecode::Instruction::CallFunction { });
            },
            ("SETUP_LOOP", Some(delta)) => {
                let curr_frame = self.curr_frame();
                let curr_offset = curr_frame.get_bytecode_offset().unwrap();
                curr_frame.blocks.push(Block {
                    block_type: "loop".to_string(),
                    handler: *curr_frame.labels.get(&(curr_offset + delta)).unwrap(),
                });
            },
            */
            ("POP_BLOCK", None) => {
                self.emit(bytecode::Instruction::PopBlock);
            }
            ("SetLineno", _) | ("LABEL", _)=> {
                // Skip
            },
            (name, _) => {
                panic!("Unrecongnizable op code: {}", name);
            }
        } // end match
    } // end dispatch function

    fn emit(&mut self, instruction: bytecode::Instruction) {
        self.code2.instructions.push(instruction);
    }
}
