//! This an example usage of the rustpython_compiler crate.
//! This program reads, parses, and compiles a file you provide
//! to RustPython bytecode, and then displays the output in the
//! `dis.dis` format.
//!
//! example usage:
//! $ cargo run --release --example dis demo*.py

#[macro_use]
extern crate log;

use core::error::Error;
use lexopt::ValueExt;
use rustpython_compiler as compiler;
use std::fs;
use std::path::{Path, PathBuf};

fn main() -> Result<(), lexopt::Error> {
    env_logger::init();

    let mut scripts = vec![];
    let mut mode = compiler::Mode::Exec;
    let mut expand_code_objects = true;
    let mut optimize = 0;

    let mut parser = lexopt::Parser::from_env();
    while let Some(arg) = parser.next()? {
        use lexopt::Arg::*;
        match arg {
            Long("help") | Short('h') => {
                let bin_name = parser.bin_name().unwrap_or("dis");
                println!(
                    "usage: {bin_name} <scripts...> [-m,--mode=exec|single|eval] [-x,--no-expand] [-O]"
                );
                println!(
                    "Compiles and disassembles python script files for viewing their bytecode."
                );
                return Ok(());
            }
            Value(x) => scripts.push(PathBuf::from(x)),
            Long("mode") | Short('m') => {
                mode = parser
                    .value()?
                    .parse_with(|s| s.parse::<compiler::Mode>().map_err(|e| e.to_string()))?
            }
            Long("no-expand") | Short('x') => expand_code_objects = false,
            Short('O') => optimize += 1,
            _ => return Err(arg.unexpected()),
        }
    }

    if scripts.is_empty() {
        return Err("expected at least one argument".into());
    }

    let opts = compiler::CompileOpts {
        optimize,
        debug_ranges: true,
    };

    for script in &scripts {
        if script.exists() && script.is_file() {
            let res = display_script(script, mode, opts, expand_code_objects);
            if let Err(e) = res {
                error!("Error while compiling {script:?}: {e}");
            }
        } else {
            eprintln!("{script:?} is not a file.");
        }
    }

    Ok(())
}

fn display_script(
    path: &Path,
    mode: compiler::Mode,
    opts: compiler::CompileOpts,
    expand_code_objects: bool,
) -> Result<(), Box<dyn Error>> {
    let source = fs::read_to_string(path)?;
    let code = compiler::compile(&source, mode, &path.to_string_lossy(), opts)?;
    println!("{}:", path.display());
    if expand_code_objects {
        println!("{}", code.display_expand_code_objects());
    } else {
        println!("{code}");
    }
    Ok(())
}
