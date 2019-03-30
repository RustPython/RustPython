extern crate proc_macro;

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, AttributeArgs, DeriveInput, Item};

mod from_args;
mod pyclass;

fn rustpython_path(inside_vm: bool) -> syn::Path {
    let path = if inside_vm {
        quote!(crate)
    } else {
        quote!(::rustpython_vm)
    };
    syn::parse2(path).unwrap()
}

/// Does the item have the #[__inside_vm] attribute on it, signifying that the derive target is
/// being derived from inside the `rustpython_vm` crate.
fn rustpython_path_derive(input: &DeriveInput) -> syn::Path {
    rustpython_path(
        input
            .attrs
            .iter()
            .any(|attr| attr.path.is_ident("__inside_vm")),
    )
}

fn rustpython_path_attr(attr: &AttributeArgs) -> syn::Path {
    rustpython_path(attr.iter().any(|meta| {
        if let syn::NestedMeta::Meta(meta) = meta {
            if let syn::Meta::Word(ident) = meta {
                ident == "__inside_vm"
            } else {
                false
            }
        } else {
            false
        }
    }))
}

#[proc_macro_derive(FromArgs, attributes(__inside_vm, pyarg))]
pub fn derive_from_args(input: TokenStream) -> TokenStream {
    let ast: DeriveInput = syn::parse(input).unwrap();

    from_args::impl_from_args(ast).into()
}

#[proc_macro_attribute]
pub fn pyclass(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr = parse_macro_input!(attr as AttributeArgs);
    let item = parse_macro_input!(item as Item);
    pyclass::impl_py_class(attr, item).into()
}
