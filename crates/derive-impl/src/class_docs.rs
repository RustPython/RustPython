//! Attribute docs resolved while expanding `#[pyclass]`.

use alloc::collections::BTreeSet;

use proc_macro2::TokenStream;
use quote::quote;

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

/// `get_attr`, then `class_attr_doc` when that entry is missing or empty.
/// An explicit empty entry with no fallback is `Some("")`.
fn resolved_attr_doc(module_name: Option<&str>, class: &str, attr: &str) -> Option<&'static str> {
    let module_key = module_name.unwrap_or("builtins");
    let exact = rustpython_doc::get_attr(module_key, class, attr);
    let qualified = rustpython_doc::class_attr_doc(module_name, class, attr);
    match exact {
        Some(doc) if !doc.is_empty() => Some(doc),
        Some(_) => qualified.or(Some("")),
        None => qualified,
    }
}

pub(crate) fn attr_docs_tokens(module_name: Option<&str>, class: &str) -> TokenStream {
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
    let entries = names.into_iter().filter_map(|attr| {
        let doc = resolved_attr_doc(module_name, class, attr)?;
        Some(quote! { (#attr, #doc) })
    });
    quote! { &[#(#entries),*] }
}
