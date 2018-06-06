
extern crate clap;
#[macro_use]
extern crate log;
extern crate env_logger;
use clap::{Arg, App};
use std::path::Path;
mod compiler;


fn main() {
  env_logger::init();
  let matches = App::new("RsPython")
      .version("0.0.1")
      .author("Windel Bouwman")
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
  info!("Running file {}", script_file);

  // Parse an ast from it:
  let filepath = Path::new(script_file);
  match compiler::parse(filepath) {
    Ok(program) => {
      debug!("Got ast: {:?}", program);
      let bytecode = compiler::compile(program);
      debug!("Code object: {:?}", bytecode);
      compiler::evaluate(bytecode);
      info!("Great succes!!");
    },
    Err(msg) => error!("Parsing went horribly wrong: {}", msg),
  }
}

