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
use rustpython_vm::compile;
use rustpython_vm::print_exception;
use rustpython_vm::VirtualMachine;
use std::io;
use std::io::prelude::*;
use std::path::Path;

use rustpython_vm::pyobject::PyObjectRef;

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
        ).arg(
            Arg::with_name("c")
                .short("c")
                .takes_value(true)
                .help("run the given string as a program"),
        ).get_matches();

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

fn _run_string(source: &str, source_path: Option<String>) {
    let mut vm = VirtualMachine::new();
    let code_obj = compile::compile(
        &mut vm,
        &source.to_string(),
        compile::Mode::Exec,
        source_path,
    ).unwrap();
    debug!("Code object: {:?}", code_obj.borrow());
    let builtins = vm.get_builtin_scope();
    let vars = vm.context().new_scope(Some(builtins)); // Keep track of local variables
    match vm.run_code_obj(code_obj, vars) {
        Ok(_value) => {}
        Err(exc) => {
            // println!("X: {:?}", exc.get_attr("__traceback__"));
            print_exception(&mut vm, &exc);
            panic!("Exception: {:?}", exc);
        }
    }
}

fn run_command(source: &mut String) {
    debug!("Running command {}", source);

    // This works around https://github.com/RustPython/RustPython/issues/17
    source.push_str("\n");
    _run_string(source, None)
}

fn run_script(script_file: &str) {
    debug!("Running file {}", script_file);
    // Parse an ast from it:
    let filepath = Path::new(script_file);
    match parser::read_file(filepath) {
        Ok(source) => _run_string(&source, Some(filepath.to_str().unwrap().to_string())),
        Err(msg) => {
            error!("Parsing went horribly wrong: {}", msg);
            std::process::exit(1);
        }
    }
}

fn shell_exec(vm: &mut VirtualMachine, source: &str, scope: PyObjectRef) -> bool {
    match compile::compile(vm, &source.to_string(), compile::Mode::Single, None) {
        Ok(code) => {
            match vm.run_code_obj(code, scope) {
                Ok(_value) => {
                    // Printed already.
                }
                Err(msg) => {
                    print_exception(vm, &msg);
                }
            }
        }
        Err(msg) => {
            // Enum rather than special string here.
            if msg == "Unexpected end of input." {
                return false;
            } else {
                println!("Error: {:?}", msg)
            }
        }
    };
    true
}

fn read_until_empty_line(input: &mut String) -> Result<i32, std::io::Error> {
    loop {
        print!("..... ");
        io::stdout().flush().ok().expect("Could not flush stdout");
        let mut line = String::new();
        match io::stdin().read_line(&mut line) {
            Ok(0) => {
                return Ok(0);
            }
            Ok(1) => {
                return Ok(1);
            }
            Ok(_) => {
                input.push_str(&line);
            }
            Err(msg) => {
                return Err(msg);
            }
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
    let vars = vm.context().new_scope(Some(builtins)); // Keep track of local variables

    // Read a single line:
    let mut input = String::new();
    loop {
        print!(">>>>> "); // Use 5 items. pypy has 4, cpython has 3.
        io::stdout().flush().ok().expect("Could not flush stdout");
        match io::stdin().read_line(&mut input) {
            Ok(0) => {
                break;
            }
            Ok(_) => {
                debug!("You entered {:?}", input);
                if shell_exec(&mut vm, &input, vars.clone()) {
                    // Line was complete.
                    input = String::new();
                } else {
                    match read_until_empty_line(&mut input) {
                        Ok(0) => {
                            break;
                        }
                        Ok(_) => {
                            shell_exec(&mut vm, &input, vars.clone());
                        }
                        Err(msg) => panic!("Error: {:?}", msg),
                    }
                }
            }
            Err(msg) => panic!("Error: {:?}", msg),
        };
    }
}
