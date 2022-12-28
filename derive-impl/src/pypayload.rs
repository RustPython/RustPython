use proc_macro2::TokenStream;
use quote::quote;
use syn::{DeriveInput, Result};

pub(crate) fn impl_pypayload(input: DeriveInput) -> Result<TokenStream> {
    let ty = &input.ident;

    let ret = quote! {
        impl ::rustpython_vm::PyPayload for #ty {
            fn class(_vm: &::rustpython_vm::VirtualMachine) -> &'static rustpython_vm::Py<::rustpython_vm::builtins::PyType> {
                <Self as ::rustpython_vm::class::StaticType>::static_type()
            }
        }
    };
    Ok(ret)
}
