use alloc::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

extern crate alloc;

fn main() {
    let target = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
    let capi_enabled = std::env::var_os("CARGO_FEATURE_CAPI").is_some();

    match target.as_str() {
        "linux" if capi_enabled => {
            println!("cargo:rustc-link-arg-bin=rustpython=-Wl,--export-dynamic");
        }
        "macos" if capi_enabled => {
            println!("cargo:rustc-link-arg-bin=rustpython=-Wl,-export_dynamic");
        }
        "windows" => {
            println!("cargo:rerun-if-changed=logo.ico");
            let mut res = winresource::WindowsResource::new();
            if Path::new("logo.ico").exists() {
                res.set_icon("logo.ico");
            } else {
                println!("cargo:warning=logo.ico not found, skipping icon embedding");
                return;
            }
            res.compile()
                .map_err(|e| {
                    println!("cargo:warning=Failed to compile Windows resources: {e}");
                })
                .ok();

            if capi_enabled && let Err(e) = export_capi_symbols_for_windows_exe() {
                println!("cargo:warning=Failed to generate Windows C-API exports definition: {e}");
            }
        }
        _ => {}
    }
}

fn export_capi_symbols_for_windows_exe() -> Result<(), String> {
    let mut files = Vec::new();
    let mut symbols = BTreeSet::new();
    let capi_src = Path::new("crates").join("capi").join("src");
    collect_rs_files(&capi_src, &mut files)
        .map_err(|e| format!("failed to enumerate C-API sources: {e}"))?;

    for file in &files {
        println!("cargo:rerun-if-changed={}", file.display());
        let content = fs::read_to_string(file)
            .map_err(|e| format!("failed to read {}: {e}", file.display()))?;
        extract_define_exception_statics_symbols(&content, &mut symbols);
        extract_no_mangle_symbols(&content, &mut symbols);
    }

    if symbols.is_empty() {
        return Err("no C-API symbols discovered".to_owned());
    }

    let out_dir = std::env::var_os("OUT_DIR").ok_or_else(|| "OUT_DIR is not set".to_owned())?;
    let def_path = PathBuf::from(out_dir).join("rustpython-capi-exports.def");
    let mut def = String::from("EXPORTS\n");
    for name in &symbols {
        println!("cargo:rustc-link-arg-bin=rustpython=/INCLUDE:{name}");
        def.push_str(name);
        def.push('\n');
    }
    fs::write(&def_path, def)
        .map_err(|e| format!("failed to write {}: {e}", def_path.display()))?;

    println!(
        "cargo:warning=Windows C-API export list has {} symbols",
        symbols.len()
    );

    println!(
        "cargo:rustc-link-arg-bin=rustpython=/DEF:{}",
        def_path.display()
    );
    Ok(())
}

fn collect_rs_files(dir: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, files)?;
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
        }
    }
    Ok(())
}

fn extract_define_exception_statics_symbols(content: &str, symbols: &mut BTreeSet<String>) {
    const BLOCK_START: &str = "define_exception_statics!";
    let Some(start) = content.find(BLOCK_START) else {
        return;
    };
    let Some(open) = content[start..].find('{') else {
        return;
    };
    let after_open = start + open + 1;
    let Some(close) = content[after_open..].find('}') else {
        return;
    };
    let block = &content[after_open..after_open + close];
    block.lines().for_each(|line| {
        let trimmed = line.trim_start();
        if let Some((name, _)) = trimmed.split_once("=>") {
            let symbol: String = name
                .trim()
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if !symbol.is_empty() {
                symbols.insert(symbol);
            }
        }
    });
}

fn extract_no_mangle_symbols(content: &str, symbols: &mut BTreeSet<String>) {
    let mut saw_no_mangle = false;

    content.lines().for_each(|line| {
        let trimmed = line.trim();
        if trimmed == "#[unsafe(no_mangle)]" {
            saw_no_mangle = true;
            return;
        }
        if !saw_no_mangle {
            return;
        }

        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("//") {
            return;
        }

        if let Some(rest) = trimmed.strip_prefix("pub static mut ") {
            let symbol: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if !symbol.is_empty() {
                symbols.insert(symbol);
            }
        } else if let Some(rest) = trimmed.strip_prefix("pub static ") {
            let symbol: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if !symbol.is_empty() {
                symbols.insert(symbol);
            }
        } else if let Some(rest) = trimmed.split_once("extern \"C\" fn ").map(|(_, rest)| rest) {
            let symbol: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if !symbol.is_empty() {
                symbols.insert(symbol);
            }
        }

        saw_no_mangle = false;
    });
}
