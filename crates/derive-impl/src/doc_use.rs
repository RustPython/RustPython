//! Record or reject doc keys while macros expand.

use proc_macro2::TokenStream;
use quote::quote;
use rustpython_doc::DocRef;

pub(crate) fn accept(doc: Option<DocRef>) -> Result<Option<DocRef>, TokenStream> {
    let Some(doc) = doc else {
        return Ok(None);
    };
    if !doc.key.is_empty() {
        record(doc.key);
    }
    if doc.listed || collecting() {
        Ok(Some(doc))
    } else {
        let key = doc.key;
        let msg = format!(
            "doc key `{key}` is not in crates/doc/used_keys.txt; run scripts/update_doc_keys.py"
        );
        Err(quote!(compile_error!(#msg)))
    }
}

fn collecting() -> bool {
    std::env::var_os("RUSTPYTHON_DOC_KEYS_OUT").is_some()
}

fn record(key: &str) {
    let Ok(path) = std::env::var("RUSTPYTHON_DOC_KEYS_OUT") else {
        return;
    };
    let mut file = match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        Ok(file) => file,
        Err(_) => return,
    };
    use std::io::Write;
    let _ = writeln!(file, "{key}");
}
