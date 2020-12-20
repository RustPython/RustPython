use std::collections::HashMap;

use rustpython_bytecode::{CodeObject, ConstantData, Instruction};
use rustpython_jit::{CompiledCode, JitType};

#[derive(Debug, Clone)]
pub struct Function {
    code: Box<CodeObject>,
    name: String,
    annotations: HashMap<String, StackValue>,
}

impl Function {
    pub fn compile(self) -> CompiledCode {
        let mut arg_types = Vec::new();
        for arg in self.code.arg_names().args {
            let arg_type = match self.annotations.get(arg) {
                Some(StackValue::String(annotation)) => match annotation.as_str() {
                    "int" => JitType::Int,
                    "float" => JitType::Float,
                    "bool" => JitType::Bool,
                    _ => panic!("Unrecognised jit type"),
                },
                _ => panic!("Argument have annotation"),
            };
            arg_types.push(arg_type);
        }

        rustpython_jit::compile(&self.code, &arg_types).expect("Compile failure")
    }
}

#[derive(Debug, Clone)]
enum StackValue {
    String(String),
    None,
    Map(HashMap<String, StackValue>),
    Code(Box<CodeObject>),
    Function(Function),
}

impl From<ConstantData> for StackValue {
    fn from(value: ConstantData) -> Self {
        match value {
            ConstantData::Str { value } => StackValue::String(value),
            ConstantData::None => StackValue::None,
            ConstantData::Code { code } => StackValue::Code(code),
            c => unimplemented!("constant {:?} isn't yet supported in py_function!", c),
        }
    }
}

pub struct StackMachine {
    stack: Vec<StackValue>,
    locals: HashMap<String, StackValue>,
}

impl StackMachine {
    pub fn new() -> StackMachine {
        StackMachine {
            stack: Vec::new(),
            locals: HashMap::new(),
        }
    }

    pub fn run(&mut self, code: CodeObject) {
        for instruction in code.instructions.into_vec() {
            if self.process_instruction(instruction, &code.constants, &code.names) {
                break;
            }
        }
    }

    fn process_instruction(
        &mut self,
        instruction: Instruction,
        constants: &[ConstantData],
        names: &[String],
    ) -> bool {
        match instruction {
            Instruction::LoadConst { idx } => {
                self.stack.push(constants[idx as usize].clone().into())
            }
            Instruction::LoadNameAny(idx) => self
                .stack
                .push(StackValue::String(names[idx as usize].clone())),
            Instruction::StoreLocal(idx) => {
                self.locals
                    .insert(names[idx as usize].clone(), self.stack.pop().unwrap());
            }
            Instruction::StoreAttr { .. } => {
                // Do nothing except throw away the stack values
                self.stack.pop().unwrap();
                self.stack.pop().unwrap();
            }
            Instruction::BuildMap { size, .. } => {
                let mut map = HashMap::new();
                for _ in 0..size {
                    let value = self.stack.pop().unwrap();
                    let name = if let Some(StackValue::String(name)) = self.stack.pop() {
                        name
                    } else {
                        unimplemented!("no string keys isn't yet supported in py_function!")
                    };
                    map.insert(name, value);
                }
                self.stack.push(StackValue::Map(map));
            }
            Instruction::MakeFunction(_flags) => {
                let name = if let Some(StackValue::String(name)) = self.stack.pop() {
                    name
                } else {
                    panic!("Expected function name")
                };
                let code = if let Some(StackValue::Code(code)) = self.stack.pop() {
                    code
                } else {
                    panic!("Expected function code")
                };
                let annotations = if let Some(StackValue::Map(map)) = self.stack.pop() {
                    map
                } else {
                    panic!("Expected function annotations")
                };
                self.stack.push(StackValue::Function(Function {
                    name,
                    code,
                    annotations,
                }));
            }
            Instruction::Duplicate => {
                let value = self.stack.last().unwrap().clone();
                self.stack.push(value);
            }
            Instruction::Rotate { amount } => {
                let mut values = Vec::new();

                // Pop all values from stack:
                values.extend(self.stack.drain(self.stack.len() - amount as usize..));

                // Push top of stack back first:
                self.stack.push(values.pop().unwrap());

                // Push other value back in order:
                self.stack.extend(values);
            }
            Instruction::ReturnValue => return true,
            _ => unimplemented!(
                "instruction {:?} isn't yet supported in py_function!",
                instruction
            ),
        }
        return false;
    }

    pub fn get_function(&self, name: &str) -> Function {
        if let Some(StackValue::Function(function)) = self.locals.get(name) {
            function.clone()
        } else {
            panic!("There was no function named {}", name)
        }
    }
}

macro_rules! jit_function {
    ($func_name:ident => $($t:tt)*) => {
        {
            let code = rustpython_derive::py_compile!(
                crate_name = "rustpython_bytecode",
                source = $($t)*
            );
            let mut machine = $crate::common::StackMachine::new();
            machine.run(code);
            machine.get_function(stringify!($func_name)).compile()
        }
    };
    ($func_name:ident($($arg_name:ident:$arg_type:ty),*) -> $ret_type:ty => $($t:tt)*) => {
        {
            use std::convert::TryInto;

            let jit_code = jit_function!($func_name => $($t)*);

            move |$($arg_name:$arg_type),*| -> Result<$ret_type, rustpython_jit::JitArgumentError> {
                jit_code
                    .invoke(&[$($arg_name.into()),*])
                    .map(|ret| match ret {
                        Some(ret) => ret.try_into().expect("jit function returned unexpected type"),
                        None => panic!("jit function unexpectedly returned None")
                    })
            }
        }
    };
    ($func_name:ident($($arg_name:ident:$arg_type:ty),*) => $($t:tt)*) => {
        {
            let jit_code = jit_function!($func_name => $($t)*);

            move |$($arg_name:$arg_type),*| -> Result<(), rustpython_jit::JitArgumentError> {
                jit_code
                    .invoke(&[$($arg_name.into()),*])
                    .map(|ret| match ret {
                        Some(ret) => panic!("jit function unexpectedly returned a value {:?}", ret),
                        None => ()
                    })
            }
        }
    };
}
