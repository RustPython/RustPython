//extern crate rustpython_parser;
#[macro_use]
extern crate clap;
extern crate env_logger;
#[macro_use]
extern crate log;
extern crate rustpython_parser;
extern crate rustpython_vm;

use clap::{Arg, App};
use std::path::Path;
use rustpython_parser::compiler;
use rustpython_vm::VirtualMachine;


fn main() {
  env_logger::init();
  let matches = App::new("RustPython")
      .version(crate_version!())
      .author(crate_authors!())
      .about("Rust implementation of the Python language")
      .arg(Arg::with_name("script")
          .required(true)
          .index(1))
      .arg(Arg::with_name("v")
          .short("v")
          .multiple(true)
          .help("Give the verbosity"))
      .get_matches();

  // Figure out the filename:
  let script_file = matches.value_of("script").unwrap_or("foo");
  debug!("Running file {}", script_file);

  // Parse an ast from it:
  let filepath = Path::new(script_file);
  match compiler::parse(filepath) {
    Ok(program) => {
      debug!("Got ast: {:?}", program);
      let bytecode = compiler::compile_py_code_object::compile(program);
      debug!("Code object: {:?}", bytecode);
      let mut vm = VirtualMachine::new();
      vm.run_code(bytecode);
    },
    Err(msg) => error!("Parsing went horribly wrong: {}", msg),
  }
}

