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
use std::time::{Duration, Instant};

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
            let parsed_file = parse_python_file(&path);
            match &parsed_file.result {
                Ok(_) => {}
                Err(y) => error!("Erreur in file {:?} {:?}", path, y),
            }

            res.push(parsed_file);
        }
    }
    Ok(res)
}

fn parse_python_file(filename: &Path) -> ParsedFile {
    info!("Parsing file {:?}", filename);
    match std::fs::read_to_string(filename) {
        Err(e) => ParsedFile {
            // filename: Box::new(filename.to_path_buf()),
            // code: "".to_owned(),
            num_lines: 0,
            result: Err(e.to_string()),
        },
        Ok(source) => {
            let num_lines = source.to_string().lines().count();
            let result = parser::parse_program(&source).map_err(|e| e.to_string());
            ParsedFile {
                // filename: Box::new(filename.to_path_buf()),
                // code: source.to_string(),
                num_lines,
                result,
            }
        }
    }
}

fn statistics(results: ScanResult) {
    // println!("Processed {:?} files", res.len());
    println!("Scanned a total of {} files", results.parsed_files.len());
    let total: usize = results.parsed_files.len();
    let total_lines: usize = results.parsed_files.iter().map(|p| p.num_lines).sum();
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
        "Processed {} files. That's {} files/second",
        total,
        rate(total, duration)
    );
    println!(
        "Processed {} lines of python code. That's {} lines/second",
        total_lines,
        rate(total_lines, duration)
    );
}

fn rate(counter: usize, duration: Duration) -> f64 {
    (counter * 1_000_000) as f64 / duration.as_micros() as f64
}

struct ScanResult {
    t1: Instant,
    t2: Instant,
    parsed_files: Vec<ParsedFile>,
}

struct ParsedFile {
    // filename: Box<PathBuf>,
    // code: String,
    num_lines: usize,
    result: ParseResult,
}

type ParseResult = Result<Vec<ast::Stmt>, String>;
