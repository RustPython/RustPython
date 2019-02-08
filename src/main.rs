//extern crate rustpython_parser;
#[macro_use]
extern crate clap;
extern crate env_logger;
#[macro_use]
extern crate log;
extern crate rustpython_parser;
extern crate rustpython_vm;
extern crate rustyline;

use clap::{App, Arg};
use rustpython_parser::parser;
use rustpython_vm::obj::objstr;
use rustpython_vm::print_exception;
use rustpython_vm::pyobject::{AttributeProtocol, PyObjectRef, PyResult};
use rustpython_vm::VirtualMachine;
use rustpython_vm::{compile, import};
use rustyline::error::ReadlineError;
use rustyline::Editor;
use std::path::Path;
use std::path::PathBuf;

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
        .arg(
            Arg::with_name("m")
                .short("m")
                .takes_value(true)
                .help("run library module as script"),
        )
        .arg(Arg::from_usage("[pyargs] 'args for python'").multiple(true))
        .get_matches();

    // Construct vm:
    let mut vm = VirtualMachine::new();

    // Figure out if a -c option was given:
    let result = if let Some(command) = matches.value_of("c") {
        run_command(&mut vm, command.to_string())
    } else if let Some(module) = matches.value_of("m") {
        run_module(&mut vm, module)
    } else {
        // Figure out if a script was passed:
        match matches.value_of("script") {
            None => run_shell(&mut vm),
            Some(filename) => run_script(&mut vm, filename),
        }
    };

    // See if any exception leaked out:
    handle_exception(&mut vm, result);
}

fn _run_string(vm: &mut VirtualMachine, source: &str, source_path: String) -> PyResult {
    let code_obj = compile::compile(vm, source, &compile::Mode::Exec, source_path)?;
    // trace!("Code object: {:?}", code_obj.borrow());
    let builtins = vm.get_builtin_scope();
    let vars = vm.context().new_scope(Some(builtins)); // Keep track of local variables
    vm.run_code_obj(code_obj, vars)
}

fn handle_exception(vm: &mut VirtualMachine, result: PyResult) {
    match result {
        Ok(_value) => {}
        Err(err) => {
            print_exception(vm, &err);
            std::process::exit(1);
        }
    }
}

fn run_command(vm: &mut VirtualMachine, mut source: String) -> PyResult {
    debug!("Running command {}", source);

    // This works around https://github.com/RustPython/RustPython/issues/17
    source.push_str("\n");
    _run_string(vm, &source, "<stdin>".to_string())
}

fn run_module(vm: &mut VirtualMachine, module: &str) -> PyResult {
    debug!("Running module {}", module);
    let current_path = PathBuf::from(".");
    import::import_module(vm, current_path, module)
}

fn run_script(vm: &mut VirtualMachine, script_file: &str) -> PyResult {
    debug!("Running file {}", script_file);
    // Parse an ast from it:
    let filepath = Path::new(script_file);
    match parser::read_file(filepath) {
        Ok(source) => _run_string(vm, &source, filepath.to_str().unwrap().to_string()),
        Err(msg) => {
            error!("Parsing went horribly wrong: {}", msg);
            std::process::exit(1);
        }
    }
}

fn shell_exec(vm: &mut VirtualMachine, source: &str, scope: PyObjectRef) -> bool {
    match compile::compile(vm, source, &compile::Mode::Single, "<stdin>".to_string()) {
        Ok(code) => {
            match vm.run_code_obj(code, scope) {
                Ok(_value) => {
                    // Printed already.
                }
                Err(err) => {
                    print_exception(vm, &err);
                }
            }
        }
        Err(err) => {
            // Enum rather than special string here.
            let name = vm.new_str("msg".to_string());
            let msg = match vm.get_attribute(err.clone(), name) {
                Ok(value) => objstr::get_value(&value),
                Err(_) => panic!("Expected msg attribute on exception object!"),
            };
            if msg == "Unexpected end of input." {
                return false;
            } else {
                print_exception(vm, &err);
            }
        }
    };
    true
}

#[cfg(not(target_family = "unix"))]
fn get_history_path() -> PathBuf {
    //Path buffer
    PathBuf::from(".repl_history.txt")
}

#[cfg(target_family = "unix")]
fn get_history_path() -> PathBuf {
    //work around for windows dependent builds. The xdg crate is unix specific
    //so access to the BaseDirectories struct breaks builds on python.
    extern crate xdg;

    let xdg_dirs = xdg::BaseDirectories::with_prefix("rustpython").unwrap();
    xdg_dirs.place_cache_file("repl_history.txt").unwrap()
}

fn run_shell(vm: &mut VirtualMachine) -> PyResult {
    println!(
        "Welcome to the magnificent Rust Python {} interpreter",
        crate_version!()
    );
    let builtins = vm.get_builtin_scope();
    let vars = vm.context().new_scope(Some(builtins)); // Keep track of local variables

    // Read a single line:
    let mut input = String::new();
    let mut rl = Editor::<()>::new();

    //retrieve a history_path_str dependent to the os
    let repl_history_path_str = &get_history_path();
    if rl.load_history(repl_history_path_str).is_err() {
        println!("No previous history.");
    }

    loop {
        // TODO: modules dont support getattr / setattr yet
        //let prompt = match vm.get_attribute(vm.sys_module.clone(), "ps1") {
        //        Ok(value) => objstr::get_value(&value),
        //        Err(_) => ">>>>> ".to_string(),
        //};

        // We can customize the prompt:
        let ps1 = objstr::get_value(&vm.sys_module.get_attr("ps1").unwrap());
        let ps2 = objstr::get_value(&vm.sys_module.get_attr("ps2").unwrap());

        match rl.readline(&ps1) {
            Ok(line) => {
                input.push_str(&line);
                input.push_str("\n");

                debug!("You entered {:?}", input);
                if shell_exec(vm, &input, vars.clone()) {
                    // Line was complete.
                    rl.add_history_entry(input.trim_end().as_ref());
                    input = String::new();
                } else {
                    loop {
                        // until an empty line is pressed AND the code is complete
                        //let prompt = match vm.get_attribute(vm.sys_module.clone(), "ps2") {
                        //        Ok(value) => objstr::get_value(&value),
                        //        Err(_) => "..... ".to_string(),
                        //};
                        match rl.readline(&ps2) {
                            Ok(line) => {
                                if line.len() == 0 {
                                    if shell_exec(vm, &input, vars.clone()) {
                                        rl.add_history_entry(input.trim_end().as_ref());
                                        input = String::new();
                                        break;
                                    }
                                } else {
                                    input.push_str(&line);
                                    input.push_str("\n");
                                }
                            }
                            Err(msg) => panic!("Error: {:?}", msg),
                        }
                    }
                }
            }
            Err(ReadlineError::Interrupted) => {
                // TODO: Raise a real KeyboardInterrupt exception
                println!("^C");
                break;
            }
            Err(ReadlineError::Eof) => {
                break;
            }
            Err(err) => {
                println!("Error: {:?}", err);
                break;
            }
        };
    }
    rl.save_history(repl_history_path_str).unwrap();

    Ok(vm.get_none())
}
