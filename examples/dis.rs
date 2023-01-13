//! This an example usage of the rustpython_compiler crate.
//! This program reads, parses, and compiles a file you provide
//! to RustPython bytecode, and then displays the output in the
//! `dis.dis` format.
//!
//! example usage:
//! $ cargo run --release --example dis demo*.py
extern crate env_logger;
#[macro_use]
extern crate log;

use clap::{crate_authors, crate_version, Arg, ArgAction, Command};
use rustpython_compiler as compiler;
use std::error::Error;
use std::fs;
use std::path::Path;

fn main() {
    env_logger::init();
    let app = Command::new("dis")
        .version(crate_version!())
        .author(crate_authors!())
        .about("Compiles and disassembles python script files for viewing their bytecode.")
        .arg(
            Arg::new("scripts")
                .help("Scripts to scan")
                .action(ArgAction::Append)
                .required(true),
        )
        .arg(
            Arg::new("mode")
                .help("The mode to compile the scripts in")
                .long("mode")
                .short('m')
                .action(ArgAction::Set)
                .default_value("exec")
                .value_parser(["exec", "single", "eval"]),
        )
        .arg(
            Arg::new("no_expand")
                .help(
                    "Don't expand CodeObject LoadConst instructions to show \
                     the instructions inside",
                )
                .long("no-expand")
                .short('x')
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("optimize")
                .help("The amount of optimization to apply to the compiled bytecode")
                .short('O')
                .action(ArgAction::Count),
        );
    let matches = app.get_matches();

    let mode = matches.get_one::<String>("mode").unwrap().parse().unwrap();
    let expand_codeobjects = !matches.get_flag("no_expand");
    let optimize = matches.get_count("optimize");
    let scripts = matches.get_raw("scripts").unwrap();

    let opts = compiler::CompileOpts {
        optimize,
        ..Default::default()
    };

    for script in scripts.map(Path::new) {
        if script.exists() && script.is_file() {
            let res = display_script(script, mode, opts.clone(), expand_codeobjects);
            if let Err(e) = res {
                error!("Error while compiling {:?}: {}", script, e);
            }
        } else {
            eprintln!("{script:?} is not a file.");
        }
    }
}

fn display_script(
    path: &Path,
    mode: compiler::Mode,
    opts: compiler::CompileOpts,
    expand_codeobjects: bool,
) -> Result<(), Box<dyn Error>> {
    let source = fs::read_to_string(path)?;
    let code = compiler::compile(&source, mode, path.to_string_lossy().into_owned(), opts)?;
    println!("{}:", path.display());
    if expand_codeobjects {
        println!("{}", code.display_expand_codeobjects());
    } else {
        println!("{code}");
    }
    Ok(())
}
