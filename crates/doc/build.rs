extern crate alloc;

use alloc::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=src/data.inc.rs");
    let src = fs::read_to_string("src/data.inc.rs").expect("read data.inc.rs");
    let entries = parse_db(&src);
    let mut text = String::new();
    let mut seen: BTreeMap<&str, (u32, u32)> = BTreeMap::new();
    let mut spans = Vec::with_capacity(entries.len());
    for (key, doc) in &entries {
        let span = if doc.is_empty() {
            (0, 0)
        } else if let Some(span) = seen.get(doc.as_str()) {
            *span
        } else {
            let offset = u32::try_from(text.len()).expect("doc offset");
            text.push_str(doc);
            let len = u32::try_from(doc.len()).expect("doc len");
            seen.insert(doc.as_str(), (offset, len));
            (offset, len)
        };
        spans.push((key.as_str(), span));
    }
    let mut index = String::from(
        "#[derive(Clone, Copy, Debug)]\n\
         pub struct DocRef {\n\
         \x20   pub offset: u32,\n\
         \x20   pub len: u32,\n\
         }\n\
         \n\
         pub static DB: &[(&str, DocRef)] = &[\n",
    );
    for (key, (offset, len)) in spans {
        index.push_str("    (");
        index.push_str(&rust_string(key));
        index.push_str(", DocRef { offset: ");
        index.push_str(&offset.to_string());
        index.push_str(", len: ");
        index.push_str(&len.to_string());
        index.push_str(" }),\n");
    }
    index.push_str("];\n");
    let blob = xz::encode_all(text.as_bytes(), 9).expect("xz encode");
    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    fs::write(out.join("docs.xz"), blob).expect("write blob");
    fs::write(out.join("index.rs"), index).expect("write index");
}

fn rust_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{{{:x}}}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn parse_db(src: &str) -> Vec<(String, String)> {
    let mut entries = Vec::new();
    for line in src.lines() {
        let line = line.trim();
        if !line.starts_with('(') {
            continue;
        }
        let rest = &line[1..];
        let (key, rest) = parse_rust_string(rest).expect("doc key");
        let rest = rest
            .trim_start()
            .strip_prefix(',')
            .expect("comma")
            .trim_start();
        let (doc, rest) = parse_rust_string(rest).expect("doc text");
        let rest = rest.trim_start();
        assert!(rest.starts_with(')'), "{rest}");
        entries.push((key, doc));
    }
    assert!(!entries.is_empty());
    entries
}

fn parse_rust_string(input: &str) -> Option<(String, &str)> {
    let bytes = input.as_bytes();
    if bytes.first() != Some(&b'"') {
        return None;
    }
    let mut i = 1;
    let mut out = Vec::new();
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => {
                i += 1;
                match *bytes.get(i)? {
                    b'n' => out.push(b'\n'),
                    b'r' => out.push(b'\r'),
                    b't' => out.push(b'\t'),
                    b'\\' => out.push(b'\\'),
                    b'"' => out.push(b'"'),
                    b'\'' => out.push(b'\''),
                    b'0' => out.push(0),
                    b'x' => {
                        let hex = core::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
                        out.push(u8::from_str_radix(hex, 16).ok()?);
                        i += 2;
                    }
                    b'u' => {
                        if bytes.get(i + 1) != Some(&b'{') {
                            return None;
                        }
                        let start = i + 2;
                        let end = bytes[start..].iter().position(|b| *b == b'}')? + start;
                        let hex = core::str::from_utf8(&bytes[start..end]).ok()?;
                        let code = u32::from_str_radix(hex, 16).ok()?;
                        let ch = char::from_u32(code)?;
                        out.extend_from_slice(ch.encode_utf8(&mut [0; 4]).as_bytes());
                        i = end;
                    }
                    _ => return None,
                }
                i += 1;
            }
            b'"' => {
                let text = String::from_utf8(out).ok()?;
                return Some((text, &input[i + 1..]));
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    None
}
