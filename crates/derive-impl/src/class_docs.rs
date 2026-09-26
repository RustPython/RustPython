//! Attribute docs resolved while expanding `#[pyclass]`.

use alloc::collections::BTreeSet;

use proc_macro2::TokenStream;
use quote::quote;
use rustpython_doc::DocRef;

/// Modules `get_qualified` consults for `module`.
fn module_aliases(module: &str) -> Vec<String> {
    let mut aliases = vec![module.to_owned()];
    if module == "os" || module == "_os" {
        aliases.push("posix".to_owned());
        aliases.push("nt".to_owned());
    }
    if let Some(rest) = module.strip_prefix('_')
        && !rest.is_empty()
    {
        aliases.push(rest.to_owned());
    }
    if !module.is_empty() && !module.starts_with('_') {
        aliases.push(format!("_{module}"));
    }
    if module != "builtins" && !module.starts_with('_') {
        aliases.push("builtins".to_owned());
    }
    aliases
}

pub(crate) fn item_doc_tokens(doc: Option<DocRef>, rust_doc: Option<String>) -> TokenStream {
    let doc = match crate::doc_use::accept(doc) {
        Ok(doc) => doc,
        Err(error) => return error,
    };
    if let Some(doc) = doc.filter(|doc| doc.len != 0) {
        let offset = doc.offset;
        let len = doc.len;
        quote! {
            {
                #[cfg(feature = "doc")]
                {
                    ::rustpython_vm::function::ItemDoc {
                        text: None,
                        offset: #offset,
                        len: #len,
                    }
                }
                #[cfg(not(feature = "doc"))]
                {
                    ::rustpython_vm::function::ItemDoc::NONE
                }
            }
        }
    } else if let Some(rust_doc) = rust_doc {
        quote!(::rustpython_vm::function::ItemDoc::static_text(#rust_doc))
    } else {
        quote!(::rustpython_vm::function::ItemDoc::NONE)
    }
}

/// `get_attr`, then `class_attr_doc` when that entry is missing or empty.
/// An explicit empty entry with no fallback is `Some` with `len == 0`.
fn resolved_attr_doc(module_name: Option<&str>, class: &str, attr: &str) -> Option<DocRef> {
    let module_key = module_name.unwrap_or("builtins");
    let exact = rustpython_doc::get_attr(module_key, class, attr);
    let qualified = rustpython_doc::class_attr_doc(module_name, class, attr);
    match exact {
        Some(doc) if !doc.listed || doc.len != 0 => Some(doc),
        Some(_) => qualified.or(Some(DocRef {
            offset: 0,
            len: 0,
            listed: true,
            key: "",
        })),
        None => qualified,
    }
}

pub(crate) fn attr_docs_tokens(
    module_name: Option<&str>,
    class: &str,
) -> (TokenStream, TokenStream) {
    let module_key = module_name.unwrap_or("builtins");
    let mut names = BTreeSet::new();
    for module in module_aliases(module_key) {
        let prefix = format!("{module}.{class}.");
        for (key, _) in rustpython_doc::DB {
            if let Some(attr) = key.strip_prefix(&prefix)
                && !attr.is_empty()
                && !attr.contains('.')
            {
                names.insert(attr);
            }
        }
    }
    let mut spans = Vec::new();
    let mut name_lits = Vec::new();
    for attr in names {
        let Some(doc) = resolved_attr_doc(module_name, class, attr) else {
            continue;
        };
        let doc = match crate::doc_use::accept(Some(doc)) {
            Ok(Some(doc)) => doc,
            Ok(None) => continue,
            Err(error) => return (error.clone(), error),
        };
        name_lits.push(quote!(#attr));
        let offset = if doc.len == 0 { u32::MAX } else { doc.offset };
        let len = doc.len;
        spans.push(quote! { (#attr, #offset, #len) });
    }
    (quote! { &[#(#spans),*] }, quote! { &[#(#name_lits),*] })
}
