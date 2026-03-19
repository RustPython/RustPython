//! This crate implements internal macros for the `rustpython` library.

use proc_macro::TokenStream;
use syn::{Item, parse_macro_input};

mod newtype_oparg;

#[proc_macro_attribute]
pub fn newtype_oparg(_metadata: TokenStream, input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as Item);

    let output = match input {
        Item::Enum(data) => {
            newtype_oparg::handle_enum(data).unwrap_or_else(|e| e.to_compile_error())
        }
        Item::Struct(data) => {
            newtype_oparg::handle_struct(data).unwrap_or_else(|e| e.to_compile_error())
        }
        _ => syn::Error::new_spanned(input, "newtype_oparg only supports structs and enums")
            .to_compile_error(),
    };

    output.into()
}
