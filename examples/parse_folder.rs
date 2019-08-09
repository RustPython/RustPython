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
use std::path::{Path, PathBuf};
use std::time::Instant;

fn main() {
    env_logger::init();
    let app = App::new("parse_folders")
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
        let t1 = Instant::now();
        let parsed_files = parse_folder(&folder).unwrap();
        let t2 = Instant::now();
        let results = ScanResult {
            t1,
            t2,
            parsed_files,
        };
        statistics(results);
    } else {
        println!("{:?} is not a folder.", folder);
    }
}

fn parse_folder(path: &Path) -> std::io::Result<Vec<ParsedFile>> {
    let mut res = vec![];
    info!("Parsing folder of python code: {:?}", path);
    for entry in path.read_dir()? {
        debug!("Entry: {:?}", entry);
        let entry = entry?;
        let metadata = entry.metadata()?;

        let path = entry.path();
        if metadata.is_dir() {
            res.extend(parse_folder(&path)?);
        }

        if metadata.is_file() && path.extension().and_then(|s| s.to_str()) == Some("py") {
            let result = parse_python_file(&path);
            match &result {
                Ok(_) => {}
                Err(y) => error!("Erreur in file {:?} {:?}", path, y),
            }
            res.push(ParsedFile {
                filename: Box::new(path),
                result,
            });
        }
    }
    Ok(res)
}

fn parse_python_file(filename: &Path) -> ParseResult {
    info!("Parsing file {:?}", filename);
    let source = std::fs::read_to_string(filename).map_err(|e| e.to_string())?;
    parser::parse_program(&source).map_err(|e| e.to_string())
}

fn statistics(results: ScanResult) {
    // println!("Processed {:?} files", res.len());
    println!("Scanned a total of {} files", results.parsed_files.len());
    let total = results.parsed_files.len();
    let failed = results
        .parsed_files
        .iter()
        .filter(|p| p.result.is_err())
        .count();
    let passed = results
        .parsed_files
        .iter()
        .filter(|p| p.result.is_ok())
        .count();
    println!("Passed: {} Failed: {} Total: {}", passed, failed, total);
    println!(
        "That is {} % success rate.",
        (passed as f64 * 100.0) / total as f64
    );
    let duration = results.t2 - results.t1;
    println!("Total time spend: {:?}", duration);
    println!(
        "File processing rate: {} files/second",
        (total * 1_000_000) as f64 / duration.as_micros() as f64
    );
}

struct ScanResult {
    t1: Instant,
    t2: Instant,
    parsed_files: Vec<ParsedFile>,
}

struct ParsedFile {
    filename: Box<PathBuf>,
    result: ParseResult,
}

type ParseResult = Result<ast::Program, String>;
