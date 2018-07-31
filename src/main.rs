//extern crate rustpython_parser;
#[macro_use]
extern crate clap;
extern crate env_logger;
#[macro_use]
extern crate log;
extern crate rustpython_parser;
extern crate rustpython_vm;

use clap::{App, Arg};
use rustpython_parser::parser;
use rustpython_vm::{VirtualMachine, Executor};
use rustpython_vm::compile;
use rustpython_vm::eval::eval;
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
        .arg(
            Arg::with_name("c")
                .short("c")
                .takes_value(true)
                .help("run the given string as a program"),
        )
        .get_matches();

    // Figure out if a -c option was given:
    if let Some(command) = matches.value_of("c") {
        run_command(&mut command.to_string());
        return;
    }

    // Figure out if a script was passed:
    match matches.value_of("script") {
        None => run_shell(),
        Some(filename) => run_script(&filename.to_string()),
    }
}

fn _run_string(source: &String) {
    let mut vm = VirtualMachine::new();
    let code_obj = compile::compile(&mut vm, &source, compile::Mode::Exec).unwrap();
    debug!("Code object: {:?}", code_obj.borrow());
    let builtins = vm.get_builtin_scope();
    let vars = vm.new_scope(Some(builtins)); // Keep track of local variables
    match vm.run_code_obj(code_obj, vars) {
        Ok(_value) => {}
        Err(exc) => {
            panic!("Exception: {:?}", exc);
        }
    }
}

fn run_command(source: &mut String) {
    debug!("Running command {}", source);

    // This works around https://github.com/RustPython/RustPython/issues/17
    source.push_str("\n");
    _run_string(source)
}

fn run_script(script_file: &String) {
    debug!("Running file {}", script_file);
    // Parse an ast from it:
    let filepath = Path::new(script_file);
    match parser::read_file(filepath) {
        Ok(source) => _run_string(&source),
        Err(msg) => {
            error!("Parsing went horribly wrong: {}", msg);
            std::process::exit(1);
        }
    }
}

fn run_shell() {
    println!(
        "Welcome to the magnificent Rust Python {} interpreter",
        crate_version!()
    );
    let mut vm = VirtualMachine::new();
    let builtins = vm.get_builtin_scope();
    let vars = vm.new_scope(Some(builtins)); // Keep track of local variables
    // Read a single line:
    loop {
        let mut input = String::new();
        print!(">>>>> "); // Use 5 items. pypy has 4, cpython has 3.
        io::stdout().flush().ok().expect("Could not flush stdout");
        match io::stdin().read_line(&mut input) {
            Ok(0) => {
                break;
            }
            Ok(_) => {
                debug!("You entered {:?}", input);
                match eval(&mut vm, &input, vars.clone()) {
                    Ok(value) => println!("{}", vm.to_str(value)),
                    Err(value) => println!("Error: {:?}", value),
                };
            }
            Err(msg) => {
                panic!("Error: {:?}", msg)
            }
        };
    }
}
