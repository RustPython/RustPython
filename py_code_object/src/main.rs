extern crate env_logger;
extern crate py_code_object;
extern crate rustpython_vm;
extern crate serde_json;

#[macro_use]
extern crate log;

mod convert;

use rustpython_vm::evaluate;
use std::env;
use std::fs::File;
use std::io::prelude::*;
use py_code_object::PyCodeObject;

fn main() {
    env_logger::init().unwrap();
    // TODO: read this from args
    let args: Vec<String> = env::args().collect();
    let filename = &args[1];

    let mut f = File::open(filename).unwrap();
    // println!("Read file");
    let mut s = String::new();
    f.read_to_string(&mut s).unwrap();
    // println!("Read string");
    // TODO: Extract this so we don't depend on json
    let cpython_code: PyCodeObject = match serde_json::from_str(&s) {
        Ok(c) => c,
        Err(_) => panic!("Fail to parse the bytecode")
    };

    let code = convert::convert(cpython_code);

    evaluate(code);
}
