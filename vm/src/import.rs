/*
 * Import mechanics
 */

extern crate rustpython_parser;

use std::path::{Path, PathBuf};
use std::io;
use std::io::ErrorKind::NotFound;

use self::rustpython_parser::parser;
use super::compile;
use super::pyobject::{Executor, PyObject, PyObjectKind, PyResult};
use super::vm::VirtualMachine;

pub fn import(rt: &mut VirtualMachine, name: &String) -> PyResult {
    // Time to search for module in any place:
    // TODO: handle 'import sys' as special case?

    let filepath = find_source(name).map_err(|e| rt.new_exception(format!("Error: {:?}", e)))?;
    let source = parser::read_file(filepath.as_path()).map_err(|e| rt.new_exception(format!("Error: {:?}", e)))?;

    let code_obj = match compile::compile(rt, &source, compile::Mode::Exec) {
        Ok(bytecode) => {
            debug!("Code object: {:?}", bytecode);
            bytecode
        }
        Err(value) => {
            panic!("Error: {}", value);
        }
    };

    let builtins = rt.get_builtin_scope();
    let scope = rt.new_scope(Some(builtins));

    match rt.run_code_obj(code_obj, scope.clone()) {
        Ok(value) => {}
        Err(value) => return Err(value),
    }

    let obj = PyObject::new(
        PyObjectKind::Module {
            name: name.clone(),
            dict: scope.clone(),
        },
        rt.get_type(),
    );
    Ok(obj)
}

fn find_source(name: &String) -> io::Result<PathBuf> {
    let suffixes = [".py", "/__init__.py"];
    let filepaths = suffixes.iter().map(|suffix| format!("{}{}", name, suffix)).map(|filename| PathBuf::from(filename));

    match filepaths.filter(|p| p.exists()).next() {
        Some(path) => Ok(path.to_path_buf()),
        None => Err(io::Error::new(NotFound, format!("Module ({}) could not be found.", name)))
    }
}
