use super::Diagnostic;
use proc_macro2::TokenStream;
use quote::quote;
use syn::DeriveInput;

pub(crate) fn impl_pyvalue(input: DeriveInput) -> std::result::Result<TokenStream, Diagnostic> {
    let ty = &input.ident;

    let ret = quote! {
        impl ::rustpython_vm::PyValue for #ty {
            fn class(_vm: &::rustpython_vm::VirtualMachine) -> &rustpython_vm::builtins::PyTypeRef {
                use ::rustpython_vm::StaticType;
                Self::static_type()
            }
        }
    };
    Ok(ret)
}
