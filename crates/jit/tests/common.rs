use core::ops::ControlFlow;
use rustpython_compiler_core::bytecode::{
    CodeObject, ConstantData, Instruction, OpArg, OpArgState,
};
use rustpython_jit::{CompiledCode, JitType};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct Function {
    code: Box<CodeObject>,
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

        let ret_type = match self.annotations.get("return") {
            Some(StackValue::String(annotation)) => match annotation.as_str() {
                "int" => Some(JitType::Int),
                "float" => Some(JitType::Float),
                "bool" => Some(JitType::Bool),
                _ => panic!("Unrecognised jit type"),
            },
            _ => None,
        };

        rustpython_jit::compile(&self.code, &arg_types, ret_type).expect("Compile failure")
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
            ConstantData::Str { value } => {
                StackValue::String(value.into_string().expect("surrogate in test code"))
            }
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
        let mut op_arg_state = OpArgState::default();
        let _ = code.instructions.iter().try_for_each(|&word| {
            let (instruction, arg) = op_arg_state.get(word);
            self.process_instruction(instruction, arg, &code.constants, &code.names)
        });
    }

    fn process_instruction(
        &mut self,
        instruction: Instruction,
        arg: OpArg,
        constants: &[ConstantData],
        names: &[String],
    ) -> ControlFlow<()> {
        match instruction {
            Instruction::LoadConst { idx } => {
                let idx = idx.get(arg);
                self.stack.push(constants[idx as usize].clone().into())
            }
            Instruction::LoadName(idx) => self
                .stack
                .push(StackValue::String(names[idx.get(arg) as usize].clone())),
            Instruction::StoreName(idx) => {
                let idx = idx.get(arg);
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
                for _ in 0..size.get(arg) {
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
            Instruction::MakeFunction => {
                let code = if let Some(StackValue::Code(code)) = self.stack.pop() {
                    code
                } else {
                    panic!("Expected function code")
                };
                // Other attributes will be set by SET_FUNCTION_ATTRIBUTE
                self.stack.push(StackValue::Function(Function {
                    code,
                    annotations: HashMap::new(), // empty annotations, will be set later if needed
                }));
            }
            Instruction::SetFunctionAttribute { attr } => {
                // Stack: [..., attr_value, func] -> [..., func]
                let func = if let Some(StackValue::Function(func)) = self.stack.pop() {
                    func
                } else {
                    panic!("Expected function on stack for SET_FUNCTION_ATTRIBUTE")
                };
                let attr_value = self.stack.pop().expect("Expected attribute value on stack");

                // For now, we only handle ANNOTATIONS flag in JIT tests
                if attr
                    .get(arg)
                    .contains(rustpython_compiler_core::bytecode::MakeFunctionFlags::ANNOTATIONS)
                {
                    if let StackValue::Map(annotations) = attr_value {
                        // Update function's annotations
                        let updated_func = Function {
                            code: func.code,
                            annotations,
                        };
                        self.stack.push(StackValue::Function(updated_func));
                    } else {
                        panic!("Expected annotations to be a map");
                    }
                } else {
                    // For other attributes, just push the function back unchanged
                    // (since JIT tests mainly care about type annotations)
                    self.stack.push(StackValue::Function(func));
                }
            }
            Instruction::ReturnConst { idx } => {
                let idx = idx.get(arg);
                self.stack.push(constants[idx as usize].clone().into());
                return ControlFlow::Break(());
            }
            Instruction::ReturnValue => return ControlFlow::Break(()),
            Instruction::ExtendedArg => {}
            _ => unimplemented!(
                "instruction {:?} isn't yet supported in py_function!",
                instruction
            ),
        }
        ControlFlow::Continue(())
    }

    pub fn get_function(&self, name: &str) -> Function {
        if let Some(StackValue::Function(function)) = self.locals.get(name) {
            function.clone()
        } else {
            panic!("There was no function named {name}")
        }
    }
}

macro_rules! jit_function {
    ($func_name:ident => $($t:tt)*) => {
        {
            let code = rustpython_derive::py_compile!(
                crate_name = "rustpython_compiler_core",
                source = $($t)*
            );
            let code = code.decode(rustpython_compiler_core::bytecode::BasicBag);
            let mut machine = $crate::common::StackMachine::new();
            machine.run(code);
            machine.get_function(stringify!($func_name)).compile()
        }
    };
    ($func_name:ident($($arg_name:ident:$arg_type:ty),*) -> $ret_type:ty => $($t:tt)*) => {
        {
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
