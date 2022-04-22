use std::fmt::Write as _;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::PathBuf;
use tiny_keccak::{Hasher, Sha3};

fn main() {
    check_lalrpop();
    gen_phf();
}

fn check_lalrpop() {
    println!("cargo:rerun-if-changed=src/python.lalrpop");
    let sha3_line = BufReader::with_capacity(128, File::open("src/python.rs").unwrap())
        .lines()
        .nth(1)
        .unwrap()
        .unwrap();
    let expected_sha3_str = sha3_line.strip_prefix("// sha3: ").unwrap();

    let mut hasher = Sha3::v256();
    hasher.update(&std::fs::read("src/python.lalrpop").unwrap());
    let mut actual_sha3 = [0u8; 32];
    hasher.finalize(&mut actual_sha3);

    // stupid stupid stupid hack. lalrpop outputs each byte as "{:x}" instead of "{:02x}"
    let sha3_equal = if expected_sha3_str.len() == 64 {
        let mut expected_sha3 = [0u8; 32];
        for (i, b) in expected_sha3.iter_mut().enumerate() {
            *b = u8::from_str_radix(&expected_sha3_str[i * 2..][..2], 16).unwrap();
        }
        actual_sha3 == expected_sha3
    } else {
        let mut actual_sha3_str = String::new();
        for byte in actual_sha3 {
            write!(actual_sha3_str, "{byte:x}").unwrap();
        }
        actual_sha3_str == expected_sha3_str
    };

    if !sha3_equal {
        eprintln!("you need to recompile lalrpop!");
        std::process::exit(1);
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
