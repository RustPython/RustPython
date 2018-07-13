//extern crate rustpython_parser;
#[macro_use]
extern crate clap;
extern crate env_logger;
#[macro_use]
extern crate log;
extern crate rustpython_parser;
extern crate rustpython_vm;

use clap::{App, Arg};
use rustpython_parser::parse;
use rustpython_parser::parser::parse_source;
use rustpython_vm::VirtualMachine;
use rustpython_vm::compile;
use rustpython_vm::eval::eval;
use rustpython_vm::pyobject::PyObjectRef;
use std::io;
use std::io::prelude::*;
use std::path::Path;

fn main() {
    env_logger::init();
    let matches = App::new("RustPython")
        .version(crate_version!())
        .author(crate_authors!())
        .about("Rust implementation of the Python language")
        .arg(Arg::with_name("script").required(false).index(1))
        .arg(
            Arg::with_name("v")
                .short("v")
                .multiple(true)
                .help("Give the verbosity"),
        )
        .get_matches();

    // Figure out if a script was passed:
    match matches.value_of("script") {
        None => run_shell(),
        Some(filename) => run_script(&filename.to_string()),
    }
}

fn run_script(script_file: &String) {
    let mut vm = VirtualMachine::new();
    debug!("Running file {}", script_file);
    // Parse an ast from it:
    let filepath = Path::new(script_file);
    match parse(filepath) {
        Ok(program) => {
            debug!("Got ast: {:?}", program);
            let bytecode = compile::compile(program);
            debug!("Code object: {:?}", bytecode);
            vm.evaluate(bytecode);
        }
        Err(msg) => {
            error!("Parsing went horribly wrong: {}", msg);
            std::process::exit(1);
        }
    }
}

fn run_shell() {
    println!("Welcome to the magnificent Rust Python interpreter");
    let mut vm = VirtualMachine::new();
    // Read a single line:
    loop {
        let mut input = String::new();
        print!(">>>");
        io::stdout().flush().ok().expect("Could not flush stdout");
        io::stdin().read_line(&mut input);
        debug!("You entered {:?}", input);
        let result = eval(&mut vm, &input);
        match result {
            Ok(value) => println!("{}", vm.to_str(value)),
            Err(value) => println!("Error: {:?}", value),
        };
    }
}
