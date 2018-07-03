#[macro_use]
extern crate serde_derive;
extern crate serde_json;

use std::cell::RefCell;
use std::rc::Rc;

#[derive(PartialEq, Debug, Clone, Serialize, Deserialize)]
pub enum NativeType{
    NoneType,
    Boolean(bool),
    Int(i32),
    Float(f64),
    Str(String),
    Unicode(String),
    #[serde(skip_serializing, skip_deserializing)]
    List(RefCell<Vec<NativeType>>),
    Tuple(Vec<NativeType>),
    Iter(Vec<NativeType>), // TODO: use Iterator instead
    Code(PyCodeObject),
    Function(Function),
    Slice(Option<i32>, Option<i32>, Option<i32>), // start, stop, step
    #[serde(skip_serializing, skip_deserializing)]
    NativeFunction(fn(Vec<Rc<NativeType>>) -> NativeType ),
}


#[derive(PartialEq, Debug, Clone, Serialize, Deserialize)]
pub struct PyCodeObject {
    pub co_consts: Vec<NativeType>,
    pub co_names: Vec<String>,
    // TODO: use vector of bytecode objects instead of strings?
    pub co_code: Vec<(usize, String, Option<usize>)>, //size, name, args
    pub co_varnames: Vec<String>,
}

impl PyCodeObject {
    pub fn new() -> PyCodeObject {
        PyCodeObject {
            co_consts: Vec::<NativeType>::new(),
            co_names: Vec::<String>::new(),
            co_code: Vec::<(usize, String, Option<usize>)>::new(), //size, name, args
            co_varnames: Vec::<String>::new(),
        }
    }
}

#[derive(PartialEq, Clone, Debug, Serialize, Deserialize)]
pub struct Function {
    pub code: PyCodeObject
}

impl Function {
    pub fn new(code: PyCodeObject) -> Function {
        Function {
            code: code
        }
    }
}
