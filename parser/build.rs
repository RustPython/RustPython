use std::fmt::Write as _;
use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::PathBuf;
use std::process::{self, Command};
use tiny_keccak::{Hasher, Sha3};

fn main() {
    check_lalrpop("src/python.lalrpop", "src/python.rs");
    gen_phf();
}

fn check_lalrpop(source: &str, generated: &str) {
    println!("cargo:rerun-if-changed={source}");

    let sha_prefix = "// sha3: ";
    let sha3_line = BufReader::with_capacity(128, File::open(generated).unwrap())
        .lines()
        .find_map(|line| {
            let line = line.unwrap();
            line.starts_with(sha_prefix).then(|| line)
        })
        .expect("no sha3 line?");
    let expected_sha3_str = sha3_line.strip_prefix(sha_prefix).unwrap();

    let actual_sha3 = {
        let mut hasher = Sha3::v256();
        let mut f = BufReader::new(File::open(source).unwrap());
        let mut line = String::new();
        while f.read_line(&mut line).unwrap() != 0 {
            if line.ends_with('\n') {
                line.pop();
                if line.ends_with('\r') {
                    line.pop();
                }
            }
            hasher.update(line.as_bytes());
            hasher.update(b"\n");
            line.clear();
        }
        let mut hash = [0u8; 32];
        hasher.finalize(&mut hash);
        hash
    };

    if sha_equal(expected_sha3_str, &actual_sha3) {
        return;
    }
    match Command::new("lalrpop").arg(source).status() {
        Ok(stat) if stat.success() => {}
        Ok(stat) => {
            eprintln!("failed to execute lalrpop; exited with {stat}");
            process::exit(stat.code().unwrap_or(1));
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            eprintln!(
                "the lalrpop executable is not installed and parser/{source} has been changed"
            );
            eprintln!("please install lalrpop with `cargo install lalrpop`");
            process::exit(1);
        }
        Err(e) => panic!("io error {e:#}"),
    }
}

fn sha_equal(expected_sha3_str: &str, actual_sha3: &[u8; 32]) -> bool {
    // stupid stupid stupid hack. lalrpop outputs each byte as "{:x}" instead of "{:02x}"
    if expected_sha3_str.len() == 64 {
        let mut expected_sha3 = [0u8; 32];
        for (i, b) in expected_sha3.iter_mut().enumerate() {
            *b = u8::from_str_radix(&expected_sha3_str[i * 2..][..2], 16).unwrap();
        }
        *actual_sha3 == expected_sha3
    } else {
        let mut actual_sha3_str = String::new();
        for byte in actual_sha3 {
            write!(actual_sha3_str, "{byte:x}").unwrap();
        }
        actual_sha3_str == expected_sha3_str
    }
}

fn gen_phf() {
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let mut kwds = phf_codegen::Map::new();
    let kwds = kwds
        // Alphabetical keywords:
        .entry("...", "Tok::Ellipsis")
        .entry("False", "Tok::False")
        .entry("None", "Tok::None")
        .entry("True", "Tok::True")
        // moreso "standard" keywords
        .entry("and", "Tok::And")
        .entry("as", "Tok::As")
        .entry("assert", "Tok::Assert")
        .entry("async", "Tok::Async")
        .entry("await", "Tok::Await")
        .entry("break", "Tok::Break")
        .entry("class", "Tok::Class")
        .entry("continue", "Tok::Continue")
        .entry("def", "Tok::Def")
        .entry("del", "Tok::Del")
        .entry("elif", "Tok::Elif")
        .entry("else", "Tok::Else")
        .entry("except", "Tok::Except")
        .entry("finally", "Tok::Finally")
        .entry("for", "Tok::For")
        .entry("from", "Tok::From")
        .entry("global", "Tok::Global")
        .entry("if", "Tok::If")
        .entry("import", "Tok::Import")
        .entry("in", "Tok::In")
        .entry("is", "Tok::Is")
        .entry("lambda", "Tok::Lambda")
        .entry("match", "Tok::Match")
        .entry("nonlocal", "Tok::Nonlocal")
        .entry("not", "Tok::Not")
        .entry("or", "Tok::Or")
        .entry("pass", "Tok::Pass")
        .entry("raise", "Tok::Raise")
        .entry("return", "Tok::Return")
        .entry("try", "Tok::Try")
        .entry("while", "Tok::While")
        .entry("with", "Tok::With")
        .entry("yield", "Tok::Yield")
        .build();
    writeln!(
        BufWriter::new(File::create(out_dir.join("keywords.rs")).unwrap()),
        "{kwds}",
    )
    .unwrap();
}
