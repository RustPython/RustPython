/// This an example usage of the rustpython_compiler crate.
/// This program reads, parses, and compiles a file you provide
/// to RustPython bytecode, and then displays the output in the
/// `dis.dis` format.
///
/// example usage:
/// $ cargo run --release --example dis demo*.py

#[macro_use]
extern crate clap;
extern crate env_logger;
#[macro_use]
extern crate log;

use clap::{App, Arg};

use rustpython_compiler as compile;
use std::error::Error;
use std::fs;
use std::path::Path;

fn main() {
    env_logger::init();
    let app = App::new("dis")
        .version(crate_version!())
        .author(crate_authors!())
        .about("Compiles and disassembles python script files for viewing their bytecode.")
        .arg(
            Arg::with_name("scripts")
                .help("Scripts to scan")
                .multiple(true)
                .required(true),
        )
        .arg(
            Arg::with_name("mode")
                .help("The mode to compile the scripts in")
                .long("mode")
                .short("m")
                .default_value("exec")
                .possible_values(&["exec", "single", "eval"])
                .takes_value(true),
        )
        .arg(
            Arg::with_name("no_expand")
                .help(
                    "Don't expand CodeObject LoadConst instructions to show \
                     the instructions inside",
                )
                .long("no-expand")
                .short("x"),
        )
        .arg(
            Arg::with_name("optimize")
                .help("The amount of optimization to apply to the compiled bytecode")
                .short("O")
                .multiple(true),
        );
    let matches = app.get_matches();

    let mode = matches.value_of_lossy("mode").unwrap().parse().unwrap();
    let expand_codeobjects = !matches.is_present("no_expand");
    let optimize = matches.occurrences_of("optimize") as u8;
    let scripts = matches.values_of_os("scripts").unwrap();

    let opts = compile::CompileOpts {
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
            eprintln!("{:?} is not a file.", script);
        }
    }
}

fn display_script(
    path: &Path,
    mode: compile::Mode,
    opts: compile::CompileOpts,
    expand_codeobjects: bool,
) -> Result<(), Box<dyn Error>> {
    let source = fs::read_to_string(path)?;
    let code = compile::compile(&source, mode, path.to_string_lossy().into_owned(), opts)?;
    println!("{}:", path.display());
    if expand_codeobjects {
        println!("{}", code.display_expand_codeobjects());
    } else {
        println!("{}", code);
    }
    Ok(())
}
