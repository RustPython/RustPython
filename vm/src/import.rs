/*
 * Import mechanics
 */

extern crate rustpython_parser;

use std::path::Path;

use self::rustpython_parser::parser;
use super::compile;
use super::pyobject::{Executor, PyObject, PyObjectKind, PyResult};

pub fn import(rt: &mut Executor, name: &String) -> PyResult {
    // Time to search for module in any place:
    // TODO: handle 'import sys' as special case?
    let filename = format!("{}.py", name);
    let filepath = Path::new(&filename);

    let source = match parser::read_file(filepath) {
        Err(value) => panic!("Error: {}", value),
        Ok(value) => value,
    };

    let code_obj = match compile::compile(rt, &source, compile::Mode::Exec) {
        Ok(bytecode) => {
            debug!("Code object: {:?}", bytecode);
            bytecode
        }
        Err(value) => {
            panic!("Error: {}", value);
        }
    };

    let dict = rt.context().new_dict();

    match rt.run_code_obj(code_obj, dict.clone()) {
        Ok(value) => {}
        Err(value) => return Err(value),
    }

    let obj = PyObject::new(
        PyObjectKind::Module {
            name: name.clone(),
            dict: dict.clone(),
        },
        rt.get_type(),
    );
    Ok(obj)
}
