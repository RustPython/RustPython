use proc_macro2::TokenStream;
use quote::quote;
use syn::{DeriveInput, Result};

pub(crate) fn impl_pyvalue(input: DeriveInput) -> Result<TokenStream> {
    let ty = &input.ident;

    let ret = quote! {
        impl ::rustpython_vm::PyValue for #ty {
            fn class(_vm: &::rustpython_vm::VirtualMachine) -> &rustpython_vm::builtins::PyTypeRef {
                <Self as ::rustpython_vm::StaticType>::static_type()
            }
        }
    };
    Ok(ret)
}
