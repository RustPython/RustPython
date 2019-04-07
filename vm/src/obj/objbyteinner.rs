use crate::pyobject::PyObjectRef;

use crate::function::OptionalArg;

use crate::vm::VirtualMachine;

use crate::pyobject::{PyResult, TypeProtocol};

use crate::obj::objstr::PyString;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use super::objint;
use crate::obj::objint::PyInt;
use num_traits::ToPrimitive;

#[derive(Debug, Default, Clone)]
pub struct PyByteInner {
    pub elements: Vec<u8>,
}

impl PyByteInner {
    pub fn new(
        val_option: OptionalArg<PyObjectRef>,
        enc_option: OptionalArg<PyObjectRef>,
        vm: &VirtualMachine,
    ) -> PyResult<PyByteInner> {
        // First handle bytes(string, encoding[, errors])
        if let OptionalArg::Present(enc) = enc_option {
            if let OptionalArg::Present(eval) = val_option {
                if let Ok(input) = eval.downcast::<PyString>() {
                    if let Ok(encoding) = enc.clone().downcast::<PyString>() {
                        if encoding.value.to_lowercase() == "utf8".to_string()
                            || encoding.value.to_lowercase() == "utf-8".to_string()
                        // TODO: different encoding
                        {
                            return Ok(PyByteInner {
                                elements: input.value.as_bytes().to_vec(),
                            });
                        } else {
                            return Err(
                                vm.new_value_error(format!("unknown encoding: {}", encoding.value)), //should be lookup error
                            );
                        }
                    } else {
                        return Err(vm.new_type_error(format!(
                            "bytes() argument 2 must be str, not {}",
                            enc.class().name
                        )));
                    }
                } else {
                    return Err(vm.new_type_error("encoding without a string argument".to_string()));
                }
            } else {
                return Err(vm.new_type_error("encoding without a string argument".to_string()));
            }
        // On ly one argument
        } else {
            let value = if let OptionalArg::Present(ival) = val_option {
                match_class!(ival.clone(),
                    i @ PyInt => {
                            let size = objint::get_value(&i.into_object()).to_usize().unwrap();
                            let mut res: Vec<u8> = Vec::with_capacity(size);
                            for _ in 0..size {
                                res.push(0)
                            }
                            Ok(res)},
                    _l @ PyString=> {return Err(vm.new_type_error(format!(
                        "string argument without an encoding"
                    )));},
                    obj => {
                        let elements = vm.extract_elements(&obj).or_else(|_| {return Err(vm.new_type_error(format!(
                        "cannot convert {} object to bytes", obj.class().name)));});

                        let mut data_bytes = vec![];
                        for elem in elements.unwrap(){
                            let v = objint::to_int(vm, &elem, 10)?;
                            if let Some(i) = v.to_u8() {
                                data_bytes.push(i);
                            } else {
                                return Err(vm.new_value_error("bytes must be in range(0, 256)".to_string()));
                                }

                            }
                        Ok(data_bytes)
                        }
                )
            } else {
                Ok(vec![])
            };
            match value {
                Ok(val) => Ok(PyByteInner { elements: val }),
                Err(err) => Err(err),
            }
        }
    }

    pub fn repr(&self) -> PyResult<String> {
        let mut res = String::with_capacity(self.elements.len());
        for i in self.elements.iter() {
            match i {
                0..=8 => res.push_str(&format!("\\x0{}", i)),
                9 => res.push_str("\\t"),
                10 => res.push_str("\\n"),
                13 => res.push_str("\\r"),
                32..=126 => res.push(*(i) as char),
                _ => res.push_str(&format!("\\x{:x}", i)),
            }
        }
        Ok(res)
    }

    pub fn len(&self) -> usize {
        self.elements.len()
    }

    pub fn eq(&self, other: &PyByteInner, vm: &VirtualMachine) -> PyResult {
        if self.elements == other.elements {
            Ok(vm.new_bool(true))
        } else {
            Ok(vm.new_bool(false))
        }
    }

    pub fn ge(&self, other: &PyByteInner, vm: &VirtualMachine) -> PyResult {
        if self.elements >= other.elements {
            Ok(vm.new_bool(true))
        } else {
            Ok(vm.new_bool(false))
        }
    }

    pub fn le(&self, other: &PyByteInner, vm: &VirtualMachine) -> PyResult {
        if self.elements <= other.elements {
            Ok(vm.new_bool(true))
        } else {
            Ok(vm.new_bool(false))
        }
    }

    pub fn gt(&self, other: &PyByteInner, vm: &VirtualMachine) -> PyResult {
        if self.elements > other.elements {
            Ok(vm.new_bool(true))
        } else {
            Ok(vm.new_bool(false))
        }
    }

    pub fn lt(&self, other: &PyByteInner, vm: &VirtualMachine) -> PyResult {
        if self.elements < other.elements {
            Ok(vm.new_bool(true))
        } else {
            Ok(vm.new_bool(false))
        }
    }

    pub fn hash(&self) -> usize {
        let mut hasher = DefaultHasher::new();
        self.elements.hash(&mut hasher);
        hasher.finish() as usize
    }
}

// TODO
// fix b"é" not allowed should be bytes("é", "utf8")
