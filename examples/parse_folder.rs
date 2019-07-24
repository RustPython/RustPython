/// This an example usage of the rustpython_parser crate.
/// This program crawls over a directory of python files and
/// tries to parse them into an abstract syntax tree (AST)
///
/// example usage:
/// $ RUST_LOG=info cargo run --release parse_folder /usr/lib/python3.7

#[macro_use]
extern crate clap;
extern crate env_logger;
#[macro_use]
extern crate log;

use clap::{App, Arg};

use rustpython_parser::{ast, parser};
use std::path::Path;

fn main() {
    env_logger::init();
    let app = App::new("RustPython")
        .version(crate_version!())
        .author(crate_authors!())
        .about("Walks over all .py files in a folder, and parses them.")
        .arg(
            Arg::with_name("folder")
                .help("Folder to scan")
                .required(true),
        );
    let matches = app.get_matches();

    let folder = Path::new(matches.value_of("folder").unwrap());
    if folder.exists() && folder.is_dir() {
        println!("Parsing folder of python code: {:?}", folder);
        let res = parse_folder(&folder).unwrap();
        println!("Processed {:?} files", res.len());
    } else {
        println!("{:?} is not a folder.", folder);
    }
}

fn parse_folder(path: &Path) -> std::io::Result<Vec<ast::Program>> {
    let mut res = vec![];
    info!("Parsing folder of python code: {:?}", path);
    for entry in path.read_dir()? {
        debug!("Entry: {:?}", entry);
        let entry = entry?;
        let metadata = entry.metadata()?;

        let path = entry.path();
        if metadata.is_dir() {
            let x = parse_folder(&path)?;
            res.extend(x);
        }

        if metadata.is_file() && path.extension().map(|s| s.to_str().unwrap()) == Some("py") {
            match parse_python_file(&path) {
                Ok(x) => res.push(x),
                Err(y) => error!("Erreur in file {:?} {:?}", path, y),
            }
        }
    }
    Ok(res)
}

fn parse_python_file(filename: &Path) -> Result<ast::Program, String> {
    info!("Parsing file {:?}", filename);
    let source = std::fs::read_to_string(filename).map_err(|e| e.to_string())?;
    parser::parse_program(&source).map_err(|e| e.to_string())
}
