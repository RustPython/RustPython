#[macro_use]
extern crate log;
extern crate env_logger;
extern crate rustpython_vm;
#[macro_use]
extern crate serde_derive;
extern crate serde_json;

use rustpython_vm::*;
use std::env;
use std::fs::File;
use std::io::prelude::*;

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
    let code: PyCodeObject = match serde_json::from_str(&s) {
        Ok(c) => c,
        Err(_) => panic!("Fail to parse the bytecode")
    };

    let mut vm = VirtualMachine::new();
    vm.run_code(code);
    // println!("Done");
}
