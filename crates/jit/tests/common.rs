use core::ops::ControlFlow;
use rustpython_compiler_core::bytecode::{
    CodeObject, ConstantData, Instruction, OpArg, OpArgState,
};
use rustpython_jit::{CompiledCode, JitType};
use rustpython_wtf8::{Wtf8, Wtf8Buf};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct Function {
    code: Box<CodeObject>,
    annotations: HashMap<Wtf8Buf, StackValue>,
}

impl Function {
    pub fn compile(self) -> CompiledCode {
        let mut arg_types = Vec::new();
        for arg in self.code.arg_names().args {
            let arg_type = match self.annotations.get(AsRef::<Wtf8>::as_ref(arg.as_str())) {
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

        let ret_type = match self.annotations.get(AsRef::<Wtf8>::as_ref("return")) {
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
    Map(HashMap<Wtf8Buf, StackValue>),
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

/// Extract annotations from an annotate function's bytecode.
/// The annotate function uses BUILD_MAP with key-value pairs loaded before it.
/// Keys are parameter names (from LOAD_CONST), values are type names (from LOAD_NAME/LOAD_GLOBAL).
fn extract_annotations_from_annotate_code(code: &CodeObject) -> HashMap<Wtf8Buf, StackValue> {
    let mut annotations = HashMap::new();
    let mut stack: Vec<(bool, usize)> = Vec::new(); // (is_const, index)
    let mut op_arg_state = OpArgState::default();

    for &word in code.instructions.iter() {
        let (instruction, arg) = op_arg_state.get(word);

        match instruction {
            Instruction::LoadConst { idx } => {
                stack.push((true, idx.get(arg) as usize));
            }
            Instruction::LoadName(idx) | Instruction::LoadGlobal(idx) => {
                stack.push((false, idx.get(arg) as usize));
            }
            Instruction::BuildMap { size, .. } => {
                let count = size.get(arg) as usize;
                // Stack has key-value pairs in order: k1, v1, k2, v2, ...
                // So we need count * 2 items from the stack
                let start = stack.len().saturating_sub(count * 2);
                let pairs: Vec<_> = stack.drain(start..).collect();

                for chunk in pairs.chunks(2) {
                    if chunk.len() == 2 {
                        let (key_is_const, key_idx) = chunk[0];
                        let (val_is_const, val_idx) = chunk[1];

                        // Key should be a const string (parameter name)
                        if key_is_const
                            && let ConstantData::Str { value } = &code.constants[key_idx]
                        {
                            let param_name = value;
                            // Value can be a name (type ref) or a const string (forward ref)
                            let type_name = if val_is_const {
                                match code.constants.get(val_idx) {
                                    Some(ConstantData::Str { value }) => value
                                        .as_str()
                                        .map(|s| s.to_owned())
                                        .unwrap_or_else(|_| value.to_string_lossy().into_owned()),
                                    Some(other) => panic!(
                                        "Unsupported annotation const for '{:?}' at idx {}: {:?}",
                                        param_name, val_idx, other
                                    ),
                                    None => panic!(
                                        "Annotation const idx out of bounds for '{:?}': {} (len={})",
                                        param_name,
                                        val_idx,
                                        code.constants.len()
                                    ),
                                }
                            } else {
                                match code.names.get(val_idx) {
                                    Some(name) => name.clone(),
                                    None => panic!(
                                        "Annotation name idx out of bounds for '{:?}': {} (len={})",
                                        param_name,
                                        val_idx,
                                        code.names.len()
                                    ),
                                }
                            };
                            annotations.insert(param_name.clone(), StackValue::String(type_name));
                        }
                    }
                }
                // Return after processing BUILD_MAP - we got our annotations
                return annotations;
            }
            Instruction::Resume { .. }
            | Instruction::LoadFast(_)
            | Instruction::CompareOp { .. }
            | Instruction::ExtendedArg => {
                // Ignore these instructions for annotation extraction
            }
            Instruction::ReturnValue | Instruction::ReturnConst { .. } => {
                // End of function - return what we have
                return annotations;
            }
            _ => {
                // For other instructions, clear the stack tracking as we don't understand the effect
                stack.clear();
            }
        }
    }

    annotations
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
            Instruction::Resume { .. } => {
                // No-op for JIT tests - just marks function entry point
            }
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
                        Wtf8Buf::from(name)
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

                let flags = attr.get(arg);

                // Handle ANNOTATE flag (PEP 649 style - Python 3.14+)
                // The attr_value is a function that returns annotations when called
                if flags.contains(rustpython_compiler_core::bytecode::MakeFunctionFlags::ANNOTATE) {
                    if let StackValue::Function(annotate_func) = attr_value {
                        // Parse the annotate function's bytecode to extract annotations
                        // The pattern is: LOAD_CONST (key), LOAD_NAME (value), ... BUILD_MAP
                        let annotate_code = &annotate_func.code;
                        let annotations = extract_annotations_from_annotate_code(annotate_code);

                        let updated_func = Function {
                            code: func.code,
                            annotations,
                        };
                        self.stack.push(StackValue::Function(updated_func));
                    } else {
                        panic!("Expected annotate function for ANNOTATE flag");
                    }
                }
                // Handle old ANNOTATIONS flag (Python 3.12 style)
                else if flags
                    .contains(rustpython_compiler_core::bytecode::MakeFunctionFlags::ANNOTATIONS)
                {
                    if let StackValue::Map(annotations) = attr_value {
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
